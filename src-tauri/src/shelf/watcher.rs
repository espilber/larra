//! Folder synchronization via `notify`, for managed Shelf folders and
//! linked sources.
//!
//! ```text
//! new file     → process
//! changed file → process when size or mtime changed
//! renamed file → old path drops derived data, new path processes
//! deleted file → remove derived document data and search records
//! ```
//!
//! Raw events are debounced and drained into the ingest queue. Paths past
//! the debounce hold go straight to ingest instead of being dropped.

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

use crate::core::Ctx;
use crate::ids;
use crate::ingest::{Ingestor, Job, ProcessJob};
use crate::types::SourceType;

const DEBOUNCE: Duration = Duration::from_millis(800);
const TICK: Duration = Duration::from_millis(400);
/// Coalesce this many paths. Extra events go to ingest immediately so a dump
/// is never dropped.
const DEBOUNCE_HOLD: usize = 32_768;

/// Remember `path` for debounce. If the hold is full, return it so the caller
/// can send it to ingest now.
fn note_pending(
    pending: &mut HashMap<PathBuf, Instant>,
    path: PathBuf,
    now: Instant,
    hold: usize,
) -> Option<PathBuf> {
    if pending.contains_key(&path) || pending.len() < hold {
        pending.insert(path, now);
        None
    } else {
        Some(path)
    }
}

pub struct WatcherHub {
    ctx: Arc<Ctx>,
    ingestor: Ingestor,
    watcher: Mutex<Option<RecommendedWatcher>>,
    tx: mpsc::UnboundedSender<PathBuf>,
}

struct SourceMatch {
    shelf_id: String,
    source_id: String,
    source_type: SourceType,
    source_label: String,
    root: PathBuf,
}

impl WatcherHub {
    pub fn start(ctx: Arc<Ctx>, ingestor: Ingestor) -> Arc<Self> {
        let (tx, mut rx) = mpsc::unbounded_channel::<PathBuf>();
        let hub = Arc::new(Self {
            ctx,
            ingestor,
            watcher: Mutex::new(None),
            tx,
        });
        hub.rebuild();

        // Debounce loop: collect changed paths, flush the quiet ones.
        let hub2 = hub.clone();
        tokio::spawn(async move {
            let mut pending: HashMap<PathBuf, Instant> = HashMap::new();
            let mut ticker = tokio::time::interval(TICK);
            loop {
                tokio::select! {
                    maybe_path = rx.recv() => {
                        match maybe_path {
                            Some(path) => {
                                if let Some(overflow) =
                                    note_pending(&mut pending, path, Instant::now(), DEBOUNCE_HOLD)
                                {
                                    hub2.handle_path(&overflow).await;
                                }
                            }
                            None => break,
                        }
                    }
                    _ = ticker.tick() => {
                        let now = Instant::now();
                        let ready: Vec<PathBuf> = pending
                            .iter()
                            .filter(|(_, t)| now.duration_since(**t) >= DEBOUNCE)
                            .map(|(p, _)| p.clone())
                            .collect();
                        for path in ready {
                            pending.remove(&path);
                            hub2.handle_path(&path).await;
                        }
                    }
                }
            }
        });
        hub
    }

    /// (Re)watch every shelf root — call after shelves or links change.
    pub fn rebuild(&self) {
        let roots: Vec<PathBuf> = {
            let library = crate::core::read_lock(&self.ctx.library);
            library
                .shelves()
                .iter()
                .flat_map(|s| {
                    std::iter::once(s.managed_path.clone())
                        .chain(s.linked_folders.iter().map(|l| l.path.clone()))
                })
                .collect()
        };
        if roots.is_empty() {
            *crate::core::mutex_lock(&self.watcher) = None;
            return;
        }
        let tx = self.tx.clone();
        let mut watcher = match notify::recommended_watcher(move |result: notify::Result<Event>| {
            let Ok(event) = result else { return };
            if !matches!(
                event.kind,
                EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
            ) {
                return;
            }
            for path in event.paths {
                if path
                    .file_name()
                    .map(|n| n.to_string_lossy().starts_with('.'))
                    .unwrap_or(true)
                {
                    continue;
                }
                let _ = tx.send(path);
            }
        }) {
            Ok(watcher) => watcher,
            Err(error) => {
                log::error!("could not create filesystem watcher: {error:#}");
                return;
            }
        };
        for root in &roots {
            if let Err(error) = watcher.watch(root, RecursiveMode::Recursive) {
                log::warn!("cannot watch {}: {error:#}", root.display());
            }
        }
        *crate::core::mutex_lock(&self.watcher) = Some(watcher);
        log::info!("watching {} folder(s)", roots.len());
    }

    fn resolve(&self, path: &Path) -> Option<SourceMatch> {
        let library = crate::core::read_lock(&self.ctx.library);
        let mut best: Option<SourceMatch> = None;
        for shelf in library.shelves() {
            if path.starts_with(&shelf.managed_path) {
                let candidate = SourceMatch {
                    shelf_id: shelf.id.clone(),
                    source_id: crate::shelf::Shelf::IMPORTED_SOURCE.to_string(),
                    source_type: SourceType::Imported,
                    source_label: "Imported".to_string(),
                    root: shelf.managed_path.clone(),
                };
                if best
                    .as_ref()
                    .map(|b| candidate.root.components().count() > b.root.components().count())
                    .unwrap_or(true)
                {
                    best = Some(candidate);
                }
            }
            for linked in &shelf.linked_folders {
                if path.starts_with(&linked.path) {
                    let label = linked
                        .path
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| linked.path.to_string_lossy().to_string());
                    let candidate = SourceMatch {
                        shelf_id: shelf.id.clone(),
                        source_id: ids::source_id(&linked.path.to_string_lossy()),
                        source_type: SourceType::Linked,
                        source_label: label,
                        root: linked.path.clone(),
                    };
                    if best
                        .as_ref()
                        .map(|b| candidate.root.components().count() > b.root.components().count())
                        .unwrap_or(true)
                    {
                        best = Some(candidate);
                    }
                }
            }
        }
        best
    }

    async fn handle_path(&self, path: &Path) {
        if std::fs::symlink_metadata(path)
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false)
        {
            return;
        }
        let Some(source) = self.resolve(path) else {
            return;
        };
        if path.is_dir() {
            if crate::shelf::rel_is_skipped(
                &path
                    .strip_prefix(&source.root)
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_default(),
            ) {
                return;
            }
            let _ = self
                .ingestor
                .queue_new_under(
                    &source.shelf_id,
                    &source.source_id,
                    source.source_type,
                    &source.source_label,
                    &source.root,
                    path,
                )
                .await;
            return;
        }
        if source.source_type == SourceType::Linked && !source.root.is_dir() {
            log::info!(
                "linked source {} is offline; not removing {}",
                source.source_id,
                path.display()
            );
            return;
        }
        if path.exists() {
            if !crate::ingest::extract::is_supported_file(path) {
                return;
            }
            let Ok(rel) = path.strip_prefix(&source.root) else {
                return;
            };
            if rel
                .components()
                .any(|c| c.as_os_str().to_string_lossy().starts_with('.'))
            {
                return;
            }
            let rel = rel.to_string_lossy().to_string();
            if crate::shelf::rel_is_skipped(&rel) {
                return;
            }
            let doc_id = crate::ids::document_id(&source.shelf_id, &source.source_id, &rel);
            {
                let library = crate::core::read_lock(&self.ctx.library);
                if !library.accepts_document(&source.shelf_id, &doc_id) {
                    return;
                }
            }
            self.ingestor
                .enqueue(Job::Process(ProcessJob {
                    shelf_id: source.shelf_id,
                    source_id: source.source_id,
                    source_type: source.source_type,
                    source_label: source.source_label,
                    abs_path: path.to_path_buf(),
                    rel_path: rel,
                    force: false,
                    epoch: 0,
                }))
                .await;
        } else {
            // Deleted or renamed away: drop derived data for this path, and
            // for anything that lived under it if it was a folder.
            let docs: Vec<(String, String)> = {
                let library = crate::core::read_lock(&self.ctx.library);
                let prefix = format!("{}{}", path.to_string_lossy(), std::path::MAIN_SEPARATOR);
                library
                    .documents(&source.shelf_id)
                    .into_iter()
                    .filter(|d| d.path == path.to_string_lossy() || d.path.starts_with(&prefix))
                    .map(|d| (d.shelf_id, d.id))
                    .collect()
            };
            for (shelf_id, doc_id) in docs {
                self.ingestor
                    .enqueue(Job::RemoveDocument { shelf_id, doc_id })
                    .await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::NoopEvents;
    use crate::ingest::extract::ExtractorSettings;
    use crate::paths::Paths;
    use crate::types::{DocStatus, SourceType};

    impl WatcherHub {
        fn inject(&self, path: PathBuf) {
            let _ = self.tx.send(path);
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn empty_library_does_not_hold_a_watcher() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path().join("appdata"));
        let ctx = Ctx::new(paths, Arc::new(NoopEvents), ExtractorSettings::default()).unwrap();
        let ingestor = Ingestor::start(ctx.clone());
        let hub = WatcherHub::start(ctx, ingestor);
        assert!(crate::core::mutex_lock(&hub.watcher).is_none());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn injected_create_is_ingested_after_debounce() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path().join("appdata"));
        let ctx = Ctx::new(paths, Arc::new(NoopEvents), ExtractorSettings::default()).unwrap();
        let root = dir.path().join("Larra");
        let shelf = {
            let mut library = crate::core::write_lock(&ctx.library);
            library.create_shelf(&ctx.paths, "Watch", &root).unwrap()
        };
        let file = shelf.managed_path.join("hello.md");
        std::fs::write(&file, "# Hello\n\nWatched file body.\n").unwrap();

        let ingestor = Ingestor::start(ctx.clone());
        let hub = WatcherHub::start(ctx.clone(), ingestor);
        hub.inject(file.clone());

        let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
        loop {
            let docs = crate::core::read_lock(&ctx.library).documents(&shelf.id);
            if docs
                .iter()
                .any(|d| d.file_name == "hello.md" && d.status != DocStatus::Reading)
            {
                break;
            }
            if tokio::time::Instant::now() >= deadline {
                panic!(
                    "debounce/ingest did not pick up hello.md; docs={:?}",
                    docs.iter()
                        .map(|d| (&d.file_name, d.status))
                        .collect::<Vec<_>>()
                );
            }
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
    }

    #[test]
    fn debounce_hold_overflows_instead_of_dropping() {
        let mut pending = HashMap::new();
        let now = Instant::now();
        for i in 0..3 {
            assert!(note_pending(&mut pending, PathBuf::from(format!("/f{i}")), now, 3).is_none());
        }
        assert_eq!(
            note_pending(&mut pending, PathBuf::from("/f3"), now, 3),
            Some(PathBuf::from("/f3"))
        );
        assert_eq!(pending.len(), 3);
        assert!(note_pending(&mut pending, PathBuf::from("/f0"), now, 3).is_none());
        assert_eq!(pending.len(), 3);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn new_folder_scan_does_not_reread_sibling_files() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path().join("appdata"));
        let ctx = Ctx::new(paths, Arc::new(NoopEvents), ExtractorSettings::default()).unwrap();
        let root = dir.path().join("Larra");
        let shelf = {
            let mut library = crate::core::write_lock(&ctx.library);
            library.create_shelf(&ctx.paths, "Watch", &root).unwrap()
        };
        let old = shelf.managed_path.join("old.md");
        std::fs::write(&old, "# Old\n\nAlready on the Shelf.\n").unwrap();
        let job = ProcessJob {
            shelf_id: shelf.id.clone(),
            source_id: crate::shelf::Shelf::IMPORTED_SOURCE.to_string(),
            source_type: SourceType::Imported,
            source_label: "Imported".into(),
            abs_path: old.clone(),
            rel_path: "old.md".into(),
            force: false,
            epoch: 0,
        };
        crate::ingest::process_file(&ctx, &job).await.unwrap();
        let old_id =
            crate::ids::document_id(&shelf.id, crate::shelf::Shelf::IMPORTED_SOURCE, "old.md");
        let first_updated = crate::core::read_lock(&ctx.library)
            .document(&shelf.id, &old_id)
            .unwrap()
            .updated_at
            .clone();

        let dump = shelf.managed_path.join("dump");
        std::fs::create_dir(&dump).unwrap();
        let nested = dump.join("hello.md");
        std::fs::write(&nested, "# Hello\n\nWatched dump file.\n").unwrap();

        let ingestor = Ingestor::start(ctx.clone());
        let hub = WatcherHub::start(ctx.clone(), ingestor);
        hub.inject(dump.clone());

        let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
        loop {
            let docs = crate::core::read_lock(&ctx.library).documents(&shelf.id);
            if docs
                .iter()
                .any(|d| d.file_name == "hello.md" && d.status != DocStatus::Reading)
            {
                let old_meta = docs.iter().find(|d| d.file_name == "old.md").unwrap();
                assert_eq!(old_meta.updated_at, first_updated);
                break;
            }
            if tokio::time::Instant::now() >= deadline {
                panic!(
                    "subtree scan did not pick up hello.md; docs={:?}",
                    docs.iter()
                        .map(|d| (&d.file_name, d.status))
                        .collect::<Vec<_>>()
                );
            }
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn offline_linked_root_does_not_purge() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path().join("appdata"));
        let ctx = Ctx::new(paths, Arc::new(NoopEvents), ExtractorSettings::default()).unwrap();
        let root = dir.path().join("Larra");
        let linked = dir.path().join("usb");
        std::fs::create_dir_all(&linked).unwrap();
        let file = linked.join("note.md");
        std::fs::write(&file, "# Hello\n\nKeep me when the volume is away.\n").unwrap();
        let (shelf_id, source_id) = {
            let mut library = crate::core::write_lock(&ctx.library);
            let shelf = library.create_shelf(&ctx.paths, "Travel", &root).unwrap();
            library.add_linked_folder(&shelf.id, &linked).unwrap();
            let source_id =
                crate::ids::source_id(&linked.canonicalize().unwrap().to_string_lossy());
            (shelf.id, source_id)
        };
        let job = ProcessJob {
            shelf_id: shelf_id.clone(),
            source_id: source_id.clone(),
            source_type: SourceType::Linked,
            source_label: "usb".into(),
            abs_path: file.clone(),
            rel_path: "note.md".into(),
            force: false,
            epoch: 0,
        };
        crate::ingest::process_file(&ctx, &job).await.unwrap();
        let doc_id = crate::ids::document_id(&shelf_id, &source_id, "note.md");
        assert!(ctx.search.has_document(&doc_id));

        std::fs::remove_dir_all(&linked).unwrap();
        let ingestor = Ingestor::start(ctx.clone());
        let hub = WatcherHub::start(ctx.clone(), ingestor);
        hub.inject(linked.clone());
        hub.inject(file.clone());
        tokio::time::sleep(DEBOUNCE + Duration::from_millis(400)).await;

        let docs = crate::core::read_lock(&ctx.library).documents(&shelf_id);
        assert_eq!(docs.len(), 1, "unmount must not look like a delete");
        assert!(ctx.search.has_document(&doc_id));
    }
}
