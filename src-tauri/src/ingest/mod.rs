//! The ingestion pipeline.
//!
//! ```text
//! file → size+mtime (hash only when those changed) → Xberg extraction
//!      → pii-vault scan → Card + extracted-text cache
//!      → structure-aware passages → Tantivy → Ready
//! ```
//!
//! Deterministic, no language model involved. Files are processed through an
//! unbounded queue whose concurrency follows available memory, and each file
//! becomes searchable on its own the moment it finishes. A dump of new files
//! waits in the queue instead of being dropped. Deleting a Shelf or unlinking
//! a folder drops matching jobs; a worker aborts before persist if the Shelf
//! or source is gone. A linked folder that is offline (USB or network volume
//! unmounted) is paused: documents stay, nothing is purged. Office lock files
//! and `*.tmp` are skipped. If a file's
//! size changes while hashing, ingest waits and retries once.

pub mod card;
pub mod excerpt;
pub mod extract;
pub mod passages;

use anyhow::Result;
use serde_json::json;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::mpsc;

use crate::core::{read_lock, write_lock, Ctx};
use crate::ids;
use crate::shelf::{rel_is_skipped, scan_new_files, MAX_FILES_PER_SHELF};
use crate::types::{DocStatus, DocumentMeta, SourceType};

/// Result of scanning one source onto a Shelf.
#[derive(Debug, Clone, Copy, Default)]
pub struct SyncOutcome {
    pub new_files: usize,
    pub at_limit: bool,
}

#[derive(Debug, Clone)]
pub struct ProcessJob {
    pub shelf_id: String,
    pub source_id: String,
    pub source_type: SourceType,
    pub source_label: String,
    pub abs_path: PathBuf,
    pub rel_path: String,
    pub force: bool,
    /// Queue generation at enqueue. `0` means unstamped (tests that call
    /// `process_file` directly).
    pub epoch: u64,
}

#[derive(Debug, Clone)]
pub enum Job {
    Process(ProcessJob),
    RemoveDocument { shelf_id: String, doc_id: String },
}

#[derive(Clone)]
pub struct Ingestor {
    ctx: Arc<Ctx>,
    tx: mpsc::UnboundedSender<Job>,
}

fn persist_docs(ctx: &Ctx, library: &crate::shelf::Library, shelf_id: &str) {
    if library.shelf(shelf_id).is_none() {
        return;
    }
    if let Err(error) = library.save_documents(&ctx.paths, shelf_id) {
        log::error!("saving document registry for {shelf_id}: {error:#}");
    }
}

fn source_is_live(ctx: &Ctx, shelf_id: &str, source_id: &str) -> bool {
    read_lock(&ctx.library).source_is_live(shelf_id, source_id)
}

/// Linked folder is still on the Shelf, but the path is not here (unmounted
/// volume). Leave registry, cards, and the index alone.
fn linked_source_offline(ctx: &Ctx, shelf_id: &str, source_id: &str) -> bool {
    let library = read_lock(&ctx.library);
    let Some(shelf) = library.shelf(shelf_id) else {
        return false;
    };
    shelf.linked_folders.iter().any(|linked| {
        ids::source_id(&linked.path.to_string_lossy()) == source_id && !linked.path.is_dir()
    })
}

fn job_still_wanted(ctx: &Ctx, job: &ProcessJob) -> bool {
    if ctx
        .ingest_queue
        .is_stale(&job.shelf_id, &job.source_id, job.epoch)
    {
        return false;
    }
    source_is_live(ctx, &job.shelf_id, &job.source_id)
}

fn abort_derived(ctx: &Ctx, shelf_id: &str, doc_id: &str) {
    let _ = std::fs::remove_file(ctx.paths.card_path(shelf_id, doc_id));
    let _ = std::fs::remove_file(ctx.paths.extracted_path(shelf_id, doc_id));
    if let Err(error) = ctx.search.remove_document(doc_id) {
        log::debug!("abort index remove {doc_id}: {error:#}");
    }
}

impl Ingestor {
    /// Start the worker pool. Concurrency follows available memory so
    /// reading files never crowds out the rest of the machine.
    pub fn start(ctx: Arc<Ctx>) -> Self {
        let (tx, rx) = mpsc::unbounded_channel::<Job>();
        let rx = Arc::new(tokio::sync::Mutex::new(rx));
        let workers = Ctx::ingest_concurrency();
        log::info!("ingestion workers: {workers}");
        for _ in 0..workers {
            let ctx = ctx.clone();
            let rx = rx.clone();
            tokio::spawn(async move {
                loop {
                    let job = { rx.lock().await.recv().await };
                    let Some(job) = job else { break };
                    match job {
                        Job::Process(job) => {
                            let _work = ctx.ingest_queue.begin_work(&job.shelf_id);
                            ctx.ingest_queue
                                .start(&job.shelf_id, &job.abs_path, job.epoch);
                            if !job_still_wanted(&ctx, &job) {
                                continue;
                            }
                            if let Err(error) = process_file(&ctx, &job).await {
                                log::error!(
                                    "pipeline error for {}: {error:#}",
                                    job.abs_path.display()
                                );
                            }
                        }
                        Job::RemoveDocument { shelf_id, doc_id } => {
                            remove_document(&ctx, &shelf_id, &doc_id);
                        }
                    }
                }
            });
        }
        Self { ctx, tx }
    }

    pub async fn enqueue(&self, job: Job) {
        self.enqueue_jobs(std::iter::once(job));
    }

    fn enqueue_jobs(&self, jobs: impl IntoIterator<Item = Job>) {
        let mut emit_for = HashSet::new();
        for job in jobs {
            match job {
                Job::Process(mut p) => {
                    if !source_is_live(&self.ctx, &p.shelf_id, &p.source_id) {
                        continue;
                    }
                    let registered = process_job_is_registered(&self.ctx, &p);
                    if !registered && !room_for_new(&self.ctx, &p.shelf_id) {
                        continue;
                    }
                    p.epoch = self.ctx.ingest_queue.stamp();
                    if self
                        .ctx
                        .ingest_queue
                        .is_stale(&p.shelf_id, &p.source_id, p.epoch)
                    {
                        continue;
                    }
                    if !self.ctx.ingest_queue.try_queue(
                        &p.shelf_id,
                        &p.source_id,
                        p.abs_path.clone(),
                        !registered,
                        p.epoch,
                    ) {
                        continue;
                    }
                    emit_for.insert(p.shelf_id.clone());
                    let shelf_id = p.shelf_id.clone();
                    let path = p.abs_path.clone();
                    let epoch = p.epoch;
                    if self.tx.send(Job::Process(p)).is_err() {
                        self.ctx.ingest_queue.start(&shelf_id, &path, epoch);
                        log::error!("ingest queue closed");
                    }
                }
                Job::RemoveDocument { .. } => {
                    if self.tx.send(job).is_err() {
                        log::error!("ingest queue closed");
                    }
                }
            }
        }
        for shelf_id in emit_for {
            if self.ctx.ingest_queue.take_stats_emit(false) {
                emit_shelf_stats(&self.ctx, &shelf_id);
            }
        }
    }

    /// Queue new supported files under `scan_root` only. Does not re-enqueue
    /// files already on the Shelf or remove stale ones.
    pub async fn queue_new_under(
        &self,
        shelf_id: &str,
        source_id: &str,
        source_type: SourceType,
        source_label: &str,
        source_root: &Path,
        scan_root: &Path,
    ) -> SyncOutcome {
        let already: HashSet<PathBuf> = {
            let library = read_lock(&self.ctx.library);
            library
                .documents(shelf_id)
                .into_iter()
                .filter(|d| d.source_id == source_id)
                .map(|d| PathBuf::from(d.path))
                .collect()
        };
        let max_new = remaining_file_slots(&self.ctx, shelf_id);
        if max_new == 0 {
            return SyncOutcome {
                new_files: 0,
                at_limit: true,
            };
        }
        let scan_root_buf = scan_root.to_path_buf();
        let outcome =
            tokio::task::spawn_blocking(move || scan_new_files(&scan_root_buf, max_new, &already))
                .await
                .unwrap_or_default();
        let mut jobs = Vec::new();
        for file in &outcome.files {
            let rel = file
                .strip_prefix(source_root)
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|_| file.to_string_lossy().to_string());
            if rel_is_skipped(&rel) {
                continue;
            }
            jobs.push(Job::Process(ProcessJob {
                shelf_id: shelf_id.to_string(),
                source_id: source_id.to_string(),
                source_type,
                source_label: source_label.to_string(),
                abs_path: file.clone(),
                rel_path: rel,
                force: false,
                epoch: 0,
            }));
        }
        let n = jobs.len();
        if n > 0 {
            self.enqueue_jobs(jobs);
            self.ctx.ingest_queue.take_stats_emit(true);
            emit_shelf_stats(&self.ctx, shelf_id);
        }
        SyncOutcome {
            new_files: n,
            at_limit: outcome.hit_file_cap,
        }
    }

    /// Queue supported files of a shelf source. Stops accepting new files
    /// once the Shelf is at `MAX_FILES_PER_SHELF`.
    pub async fn sync_source(
        &self,
        shelf_id: &str,
        source_id: &str,
        source_type: SourceType,
        source_label: &str,
        root: &Path,
        force: bool,
    ) -> SyncOutcome {
        if source_type == SourceType::Linked && !root.is_dir() {
            log::info!(
                "linked source {source_id} is offline ({}); leaving documents in place",
                root.display()
            );
            return SyncOutcome::default();
        }
        let source_docs: Vec<DocumentMeta> = {
            let library = read_lock(&self.ctx.library);
            library
                .documents(shelf_id)
                .into_iter()
                .filter(|d| d.source_id == source_id)
                .collect()
        };

        let mut stale: Vec<String> = Vec::new();
        let mut live: Vec<DocumentMeta> = Vec::new();
        for doc in source_docs {
            let path = PathBuf::from(&doc.path);
            let skipped = rel_is_skipped(&doc.rel_path);
            if skipped || !path.exists() || !crate::ingest::extract::is_supported_file(&path) {
                stale.push(doc.id);
                continue;
            }
            live.push(doc);
        }
        for doc_id in &stale {
            self.enqueue(Job::RemoveDocument {
                shelf_id: shelf_id.to_string(),
                doc_id: doc_id.clone(),
            })
            .await;
        }

        for doc in &live {
            self.enqueue(Job::Process(ProcessJob {
                shelf_id: shelf_id.to_string(),
                source_id: source_id.to_string(),
                source_type,
                source_label: source_label.to_string(),
                abs_path: PathBuf::from(&doc.path),
                rel_path: doc.rel_path.clone(),
                force,
                epoch: 0,
            }))
            .await;
        }

        self.queue_new_under(shelf_id, source_id, source_type, source_label, root, root)
            .await
    }

    /// Sync every source of every shelf (startup / manual refresh).
    pub async fn sync_all(&self, force: bool) {
        let shelves: Vec<crate::shelf::Shelf> = {
            let library = read_lock(&self.ctx.library);
            library.shelves().to_vec()
        };
        for shelf in shelves {
            let _ = self
                .sync_source(
                    &shelf.id,
                    crate::shelf::Shelf::IMPORTED_SOURCE,
                    SourceType::Imported,
                    "Imported",
                    &shelf.managed_path,
                    force,
                )
                .await;
            for linked in &shelf.linked_folders {
                let label = linked
                    .path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| linked.path.to_string_lossy().to_string());
                let _ = self
                    .sync_source(
                        &shelf.id,
                        &ids::source_id(&linked.path.to_string_lossy()),
                        SourceType::Linked,
                        &label,
                        &linked.path,
                        force,
                    )
                    .await;
            }
        }
    }
}

pub(crate) fn emit_file_event(ctx: &Ctx, meta: &DocumentMeta) {
    ctx.events.emit(
        "larra://ingest",
        json!({
            "shelfId": meta.shelf_id,
            "documentId": meta.id,
            "fileName": meta.file_name,
            "status": meta.status,
            "error": meta.error,
            "piiTotal": meta.pii_total,
            "passages": meta.passage_count,
            "document": meta,
        }),
    );
}

fn emit_shelf_stats(ctx: &Ctx, shelf_id: &str) {
    let stats = ctx
        .ingest_queue
        .with_waiting(shelf_id, read_lock(&ctx.library).stats(shelf_id));
    ctx.events.emit(
        "larra://shelf-stats",
        json!({ "shelfId": shelf_id, "stats": stats }),
    );
}

fn process_job_is_registered(ctx: &Ctx, job: &ProcessJob) -> bool {
    let doc_id = ids::document_id(&job.shelf_id, &job.source_id, &job.rel_path);
    read_lock(&ctx.library)
        .document(&job.shelf_id, &doc_id)
        .is_some()
}

pub(crate) fn remaining_file_slots(ctx: &Ctx, shelf_id: &str) -> usize {
    ctx.ingest_queue.remaining_new_files(
        shelf_id,
        read_lock(&ctx.library).document_count(shelf_id),
        MAX_FILES_PER_SHELF,
    )
}

fn room_for_new(ctx: &Ctx, shelf_id: &str) -> bool {
    remaining_file_slots(ctx, shelf_id) > 0
}

/// Turn an internal failure into product language (action-oriented, no
/// stack traces).
fn friendly_error(error: &anyhow::Error) -> String {
    if let Some(xberg_error) = error.downcast_ref::<xberg::XbergError>() {
        return match xberg_error {
            xberg::XbergError::UnsupportedFormat(_) => {
                "This file type isn't supported.".to_string()
            }
            xberg::XbergError::Ocr { .. } => {
                "Couldn't read this file. Try opening it to check it's OK, then try again."
                    .to_string()
            }
            xberg::XbergError::Timeout { .. } => "Reading took too long. Try again.".to_string(),
            xberg::XbergError::Io(_) => "Couldn't open this file. Try again.".to_string(),
            _ => "Couldn't read this file. Try again.".to_string(),
        };
    }
    let text = error.to_string();
    if text.contains("couldn't read any text") {
        "Larra couldn't read any text in this file.".to_string()
    } else {
        "Couldn't read this file. Try again.".to_string()
    }
}

struct FileFingerprint {
    size: u64,
    mtime_ms: u64,
}

fn file_fingerprint(path: &Path) -> std::io::Result<FileFingerprint> {
    let meta = std::fs::metadata(path)?;
    let mtime_ms = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    Ok(FileFingerprint {
        size: meta.len(),
        mtime_ms,
    })
}

enum Unchanged {
    Ready,
    Error,
}

fn unchanged_on_disk(existing: &DocumentMeta, fp: &FileFingerprint) -> Option<Unchanged> {
    let size_ok = existing.size_bytes == fp.size;
    let mtime_ok = existing.mtime_ms != 0 && existing.mtime_ms == fp.mtime_ms;
    match existing.status {
        DocStatus::Ready if size_ok && mtime_ok => Some(Unchanged::Ready),
        DocStatus::Error if size_ok && (mtime_ok || existing.mtime_ms == 0) => {
            Some(Unchanged::Error)
        }
        DocStatus::Ready | DocStatus::Error | DocStatus::Reading => None,
    }
}

fn persist_fingerprint(ctx: &Ctx, existing: &DocumentMeta, fp: &FileFingerprint) {
    if existing.size_bytes == fp.size && existing.mtime_ms == fp.mtime_ms {
        return;
    }
    if !source_is_live(ctx, &existing.shelf_id, &existing.source_id) {
        return;
    }
    let mut meta = existing.clone();
    meta.size_bytes = fp.size;
    meta.mtime_ms = fp.mtime_ms;
    let mut library = write_lock(&ctx.library);
    library.upsert_document(meta);
    persist_docs(ctx, &library, &existing.shelf_id);
}

/// True when extract can be skipped (index is present, or empty file).
async fn try_skip_ready(ctx: &Arc<Ctx>, existing: &DocumentMeta) -> Result<bool> {
    if !source_is_live(ctx, &existing.shelf_id, &existing.source_id) {
        return Ok(true);
    }
    if ctx.search.has_document(&existing.id) || existing.passage_count == 0 {
        return Ok(true);
    }
    let ctx2 = ctx.clone();
    let existing = existing.clone();
    match tokio::task::spawn_blocking(move || reindex_from_cache(&ctx2, &existing)).await {
        Ok(Ok(())) => Ok(true),
        Ok(Err(error)) => {
            log::warn!("reindex from cache: {error:#}");
            Ok(false)
        }
        Err(error) => {
            log::warn!("reindex from cache join: {error:#}");
            Ok(false)
        }
    }
}

/// Rebuild the search index from Card + extracted text after a schema wipe.
fn reindex_from_cache(ctx: &Ctx, meta: &DocumentMeta) -> Result<()> {
    let card = card::read_card(&ctx.paths.card_path(&meta.shelf_id, &meta.id))?;
    let extracted = extract::limit_extracted(std::fs::read_to_string(
        ctx.paths.extracted_path(&meta.shelf_id, &meta.id),
    )?);
    if extracted.trim().is_empty() {
        anyhow::bail!("empty extracted text");
    }
    let passages = passages::build_passages(&extract::blocks_from_markdown(&extracted));
    if passages.is_empty() {
        anyhow::bail!("no passages from extracted text");
    }
    ctx.search.index_document(
        meta,
        &card.summary,
        &card.keywords,
        &passages,
        card.language.as_deref(),
        &card.quality,
        &card.title,
    )?;
    Ok(())
}

pub(crate) fn fail_idle_reading(ctx: &Ctx, shelf_id: &str) {
    if ctx.ingest_queue.pending(shelf_id) {
        return;
    }
    let mut library = write_lock(&ctx.library);
    let docs = library
        .documents(shelf_id)
        .into_iter()
        .filter(|d| d.status == DocStatus::Reading)
        .collect::<Vec<_>>();
    if docs.is_empty() || ctx.ingest_queue.pending(shelf_id) {
        return;
    }
    for mut doc in docs {
        doc.status = DocStatus::Error;
        doc.error = Some(rust_i18n::t!("documents.couldntRead").to_string());
        library.upsert_document(doc.clone());
        emit_file_event(ctx, &doc);
    }
    persist_docs(ctx, &library, shelf_id);
    drop(library);
    emit_shelf_stats(ctx, shelf_id);
}

pub async fn process_file(ctx: &Arc<Ctx>, job: &ProcessJob) -> Result<()> {
    let result = process_file_inner(ctx, job).await;
    if let Err(error) = &result {
        if job_still_wanted(ctx, job) {
            let id = ids::document_id(&job.shelf_id, &job.source_id, &job.rel_path);
            let mut library = write_lock(&ctx.library);
            if library.accepts_document(&job.shelf_id, &id) {
                let mut meta =
                    library
                        .document(&job.shelf_id, &id)
                        .unwrap_or_else(|| DocumentMeta {
                            id,
                            shelf_id: job.shelf_id.clone(),
                            source_id: job.source_id.clone(),
                            source_type: job.source_type,
                            path: job.abs_path.to_string_lossy().to_string(),
                            rel_path: job.rel_path.clone(),
                            file_name: job
                                .abs_path
                                .file_name()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .to_string(),
                            format: job
                                .abs_path
                                .extension()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .to_string(),
                            size_bytes: 0,
                            mtime_ms: 0,
                            hash: String::new(),
                            status: DocStatus::Error,
                            error: None,
                            passage_count: 0,
                            pages: None,
                            pii_total: 0,
                            pii_categories: Default::default(),
                            ocr: false,
                            updated_at: chrono::Utc::now().to_rfc3339(),
                            source_label: job.source_label.clone(),
                        });
                meta.status = DocStatus::Error;
                meta.error = Some(friendly_error(error));
                library.upsert_document(meta.clone());
                persist_docs(ctx, &library, &job.shelf_id);
                emit_file_event(ctx, &meta);
            }
            drop(library);
            emit_shelf_stats(ctx, &job.shelf_id);
        }
    }
    result
}

async fn process_file_inner(ctx: &Arc<Ctx>, job: &ProcessJob) -> Result<()> {
    if std::fs::symlink_metadata(&job.abs_path)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
    {
        log::warn!("skipping symlink {}", job.abs_path.display());
        return Ok(());
    }
    if !job.abs_path.exists() {
        if linked_source_offline(ctx, &job.shelf_id, &job.source_id) {
            log::info!(
                "skip remove for {} — linked source is offline",
                job.abs_path.display()
            );
            return Ok(());
        }
        let doc = {
            let library = read_lock(&ctx.library);
            library.find_document_by_path(&job.shelf_id, &job.abs_path)
        };
        if let Some(doc) = doc {
            remove_document(ctx, &job.shelf_id, &doc.id);
        }
        return Ok(());
    }
    if crate::shelf::rel_is_skipped(&job.rel_path) || !extract::is_supported_file(&job.abs_path) {
        return Ok(());
    }
    if !job_still_wanted(ctx, job) {
        return Ok(());
    }

    let doc_id = ids::document_id(&job.shelf_id, &job.source_id, &job.rel_path);
    let existing = {
        let library = read_lock(&ctx.library);
        if !library.accepts_document(&job.shelf_id, &doc_id) {
            return Ok(());
        }
        library.document(&job.shelf_id, &doc_id)
    };
    let fp = file_fingerprint(&job.abs_path).ok();

    if !job.force {
        if let (Some(existing), Some(fp)) = (existing.as_ref(), fp.as_ref()) {
            match unchanged_on_disk(existing, fp) {
                Some(Unchanged::Error) => return Ok(()),
                Some(Unchanged::Ready) if try_skip_ready(ctx, existing).await? => {
                    return Ok(());
                }
                Some(Unchanged::Ready) | None => {}
            }
        }
    }

    let hash = {
        let path = job.abs_path.clone();
        match tokio::task::spawn_blocking(move || ids::content_hash_file_stable(&path)).await? {
            Ok(Some(hash)) => hash,
            Ok(None) => return Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error.into()),
        }
    };
    if !job_still_wanted(ctx, job) {
        return Ok(());
    }
    let fp = file_fingerprint(&job.abs_path).ok().or(fp);

    if !job.force {
        if let Some(existing) = existing.as_ref() {
            if existing.hash == hash {
                match existing.status {
                    DocStatus::Ready => {
                        if let Some(fp) = fp.as_ref() {
                            persist_fingerprint(ctx, existing, fp);
                        }
                        if try_skip_ready(ctx, existing).await? {
                            return Ok(());
                        }
                    }
                    DocStatus::Error => {
                        if let Some(fp) = fp.as_ref() {
                            persist_fingerprint(ctx, existing, fp);
                        }
                        return Ok(());
                    }
                    DocStatus::Reading => {}
                }
            }
        }
    }

    let file_name = job
        .abs_path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| job.rel_path.clone());
    let format = job
        .abs_path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();
    let size_bytes = fp.as_ref().map(|f| f.size).unwrap_or_else(|| {
        std::fs::metadata(&job.abs_path)
            .map(|m| m.len())
            .unwrap_or(0)
    });
    let mtime_ms = fp.as_ref().map(|f| f.mtime_ms).unwrap_or(0);
    let now = chrono::Utc::now().to_rfc3339();

    let mut meta = DocumentMeta {
        id: doc_id.clone(),
        shelf_id: job.shelf_id.clone(),
        source_id: job.source_id.clone(),
        source_type: job.source_type,
        path: job.abs_path.to_string_lossy().to_string(),
        rel_path: job.rel_path.clone(),
        file_name,
        format,
        size_bytes,
        mtime_ms,
        hash: hash.clone(),
        status: DocStatus::Reading,
        error: None,
        passage_count: 0,
        pages: None,
        pii_total: 0,
        pii_categories: Default::default(),
        ocr: false,
        updated_at: now,
        source_label: job.source_label.clone(),
    };

    {
        if ctx
            .ingest_queue
            .is_stale(&job.shelf_id, &job.source_id, job.epoch)
        {
            return Ok(());
        }
        let mut library = write_lock(&ctx.library);
        if !library.source_is_live(&job.shelf_id, &job.source_id)
            || !library.accepts_document(&job.shelf_id, &doc_id)
        {
            return Ok(());
        }
        library.upsert_document(meta.clone());
        persist_docs(ctx, &library, &job.shelf_id);
    }
    emit_file_event(ctx, &meta);
    emit_shelf_stats(ctx, &job.shelf_id);

    // Native text, or local OCR when the file has none.
    let extraction = match extract::extract_file(&job.abs_path, &ctx.extractor).await {
        Ok(extraction) => extraction,
        Err(error) => {
            if !job_still_wanted(ctx, job) {
                return Ok(());
            }
            meta.status = DocStatus::Error;
            meta.error = Some(friendly_error(&error));
            meta.updated_at = chrono::Utc::now().to_rfc3339();
            {
                if ctx
                    .ingest_queue
                    .is_stale(&job.shelf_id, &job.source_id, job.epoch)
                {
                    return Ok(());
                }
                let mut library = write_lock(&ctx.library);
                if !library.source_is_live(&job.shelf_id, &job.source_id) {
                    return Ok(());
                }
                library.upsert_document(meta.clone());
                persist_docs(ctx, &library, &job.shelf_id);
            }
            emit_file_event(ctx, &meta);
            emit_shelf_stats(ctx, &job.shelf_id);
            log::warn!(
                "extraction failed for {}: {error:#}",
                job.abs_path.display()
            );
            return Ok(());
        }
    };

    if !job_still_wanted(ctx, job) {
        abort_derived(ctx, &job.shelf_id, &doc_id);
        return Ok(());
    }

    // Counts only. The card keeps placeholders, never the matched values.
    let (privacy, summary, keywords) = {
        let content = extraction.content.clone();
        let summary = extraction.summary.clone();
        let keywords = extraction.keywords.clone();
        let pii = &ctx.pii;
        // pii scan is CPU-bound regex work — keep it off the async threads.
        tokio::task::block_in_place(|| {
            (
                pii.summarize(&content),
                pii.redact(&summary),
                keywords
                    .into_iter()
                    .map(|word| pii.redact(&word))
                    .collect::<Vec<_>>(),
            )
        })
    };

    // Card + extracted-text cache for the drawer.
    let card = card::build_card(card::CardInputs {
        doc_id: &doc_id,
        source_type: job.source_type,
        path: &meta.path,
        hash: &hash,
        title: &extraction.title,
        format: &meta.format,
        language: extraction.language_tag.as_deref(),
        summary: &summary,
        keywords: &keywords,
        outline: &extraction.outline,
        ocr_used: extraction.ocr_used,
        privacy: &privacy,
    });
    card::write_card(&ctx.paths.card_path(&job.shelf_id, &doc_id), &card)?;
    let extracted_path = ctx.paths.extracted_path(&job.shelf_id, &doc_id);
    if let Some(parent) = extracted_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&extracted_path, &extraction.content)?;

    if !job_still_wanted(ctx, job) {
        abort_derived(ctx, &job.shelf_id, &doc_id);
        return Ok(());
    }

    // Index passages for retrieval.
    let passages = passages::build_passages(&extraction.blocks);
    let passage_count = passages.len() as u32;
    {
        let ctx2 = ctx.clone();
        let meta2 = meta.clone();
        let summary = summary.clone();
        let keywords = keywords.clone();
        let title = extraction.title.clone();
        let language = extraction.language;
        let quality = if extraction.ocr_used { "ocr" } else { "full" };
        tokio::task::spawn_blocking(move || {
            ctx2.search.index_document(
                &meta2, &summary, &keywords, &passages, language, quality, &title,
            )
        })
        .await??;
    }

    if !job_still_wanted(ctx, job) {
        abort_derived(ctx, &job.shelf_id, &doc_id);
        return Ok(());
    }

    // Mark ready once the index write succeeds.
    meta.status = DocStatus::Ready;
    meta.error = None;
    meta.passage_count = passage_count;
    meta.pages = extraction.page_count;
    meta.pii_total = privacy.total;
    meta.pii_categories = privacy.categories.clone();
    meta.ocr = extraction.ocr_used;
    meta.updated_at = chrono::Utc::now().to_rfc3339();
    {
        if ctx
            .ingest_queue
            .is_stale(&job.shelf_id, &job.source_id, job.epoch)
        {
            abort_derived(ctx, &job.shelf_id, &doc_id);
            return Ok(());
        }
        let mut library = write_lock(&ctx.library);
        if !library.source_is_live(&job.shelf_id, &job.source_id) {
            abort_derived(ctx, &job.shelf_id, &doc_id);
            return Ok(());
        }
        library.upsert_document(meta.clone());
        persist_docs(ctx, &library, &job.shelf_id);
    }
    emit_file_event(ctx, &meta);
    emit_shelf_stats(ctx, &job.shelf_id);
    Ok(())
}

/// Remove one document's derived data (card, extracted text, index records,
/// registry entry). Never touches the original file.
pub fn remove_document(ctx: &Arc<Ctx>, shelf_id: &str, doc_id: &str) {
    {
        let mut library = write_lock(&ctx.library);
        library.remove_document(shelf_id, doc_id);
        persist_docs(ctx, &library, shelf_id);
    }
    let _ = std::fs::remove_file(ctx.paths.card_path(shelf_id, doc_id));
    let _ = std::fs::remove_file(ctx.paths.extracted_path(shelf_id, doc_id));
    if let Err(error) = ctx.search.remove_document(doc_id) {
        log::error!("removing {doc_id} from index: {error:#}");
    }
    ctx.events.emit(
        "larra://ingest",
        json!({
            "shelfId": shelf_id,
            "documentId": doc_id,
            "status": "removed",
        }),
    );
    emit_shelf_stats(ctx, shelf_id);
}

/// Remove all derived data for a list of documents (source unlink / shelf
/// removal).
pub fn remove_documents(ctx: &Arc<Ctx>, shelf_id: &str, doc_ids: &[String]) {
    for doc_id in doc_ids {
        let _ = std::fs::remove_file(ctx.paths.card_path(shelf_id, doc_id));
        let _ = std::fs::remove_file(ctx.paths.extracted_path(shelf_id, doc_id));
    }
    if let Err(error) = ctx.search.remove_documents(doc_ids) {
        log::error!("removing documents from index: {error:#}");
    }
    emit_shelf_stats(ctx, shelf_id);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::SourceType;

    fn meta(status: DocStatus, size: u64, mtime_ms: u64) -> DocumentMeta {
        DocumentMeta {
            id: "d_x".into(),
            shelf_id: "s_x".into(),
            source_id: "imported".into(),
            source_type: SourceType::Imported,
            path: "/tmp/x.md".into(),
            rel_path: "x.md".into(),
            file_name: "x.md".into(),
            format: "md".into(),
            size_bytes: size,
            mtime_ms,
            hash: String::new(),
            status,
            error: None,
            passage_count: 1,
            pages: None,
            pii_total: 0,
            pii_categories: Default::default(),
            ocr: false,
            updated_at: "2026-01-01T00:00:00Z".into(),
            source_label: "Imported".into(),
        }
    }

    #[test]
    fn ready_file_with_same_size_and_mtime_is_unchanged() {
        let fp = FileFingerprint {
            size: 10,
            mtime_ms: 1_000,
        };
        assert!(matches!(
            unchanged_on_disk(&meta(DocStatus::Ready, 10, 1_000), &fp),
            Some(Unchanged::Ready)
        ));
    }

    #[test]
    fn ready_file_with_new_mtime_is_not_skipped() {
        let fp = FileFingerprint {
            size: 10,
            mtime_ms: 2_000,
        };
        assert!(unchanged_on_disk(&meta(DocStatus::Ready, 10, 1_000), &fp).is_none());
    }

    #[test]
    fn error_file_with_same_size_is_left_alone() {
        let fp = FileFingerprint {
            size: 10,
            mtime_ms: 1_000,
        };
        assert!(matches!(
            unchanged_on_disk(&meta(DocStatus::Error, 10, 0), &fp),
            Some(Unchanged::Error)
        ));
        assert!(matches!(
            unchanged_on_disk(&meta(DocStatus::Error, 10, 1_000), &fp),
            Some(Unchanged::Error)
        ));
    }

    #[test]
    fn error_file_with_new_size_is_not_skipped() {
        let fp = FileFingerprint {
            size: 99,
            mtime_ms: 1_000,
        };
        assert!(unchanged_on_disk(&meta(DocStatus::Error, 10, 1_000), &fp).is_none());
    }

    #[test]
    fn reading_file_is_never_skipped_by_fingerprint() {
        let fp = FileFingerprint {
            size: 10,
            mtime_ms: 1_000,
        };
        assert!(unchanged_on_disk(&meta(DocStatus::Reading, 10, 1_000), &fp).is_none());
    }
}
