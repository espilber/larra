//! Shelf, document, and file-manager commands.

use serde::Serialize;
use serde_json::json;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

use super::{friendly, require_id, CmdResult};
use crate::core::Ctx;
use crate::ingest::{Ingestor, Job, ProcessJob};
use crate::shelf::watcher::WatcherHub;
use crate::shelf::ThinkLevel;
use crate::types::{Card, DocumentMeta, ShelfStats, SourceType};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkedView {
    pub source_id: String,
    pub path: String,
    pub label: String,
    /// False when the folder is not on this computer (unmounted volume).
    pub available: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShelfView {
    pub id: String,
    pub name: String,
    pub managed_path: String,
    pub linked_folders: Vec<LinkedView>,
    pub stats: ShelfStats,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    pub think_level: ThinkLevel,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportResult {
    pub queued: u32,
    pub names: Vec<String>,
    #[serde(default)]
    pub cancelled: bool,
    #[serde(default)]
    pub at_limit: bool,
    #[serde(default)]
    pub skipped_long: u32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AddLinkedResult {
    pub shelf: ShelfView,
    pub queued: u32,
    pub at_limit: bool,
}

/// Paths the OS just offered via a native drop or file picker.
/// The webview cannot import a path that is not in this set.
pub struct PendingImports {
    paths: Mutex<HashSet<PathBuf>>,
}

const DROP_WAIT_ATTEMPTS: u32 = 20;
const DROP_WAIT_STEP: Duration = Duration::from_millis(25);

impl PendingImports {
    pub fn new() -> Self {
        Self {
            paths: Mutex::new(HashSet::new()),
        }
    }

    pub fn admit(&self, paths: impl IntoIterator<Item = PathBuf>) {
        let mut set = crate::core::mutex_lock(&self.paths);
        for path in paths {
            set.insert(normalize_import_path(&path));
        }
    }

    /// Keep only requested paths that were admitted. A miss does not clear
    /// the rest of the set (a probe must not eat a real drop).
    pub fn take_allowed(&self, requested: Vec<PathBuf>) -> Vec<PathBuf> {
        let mut set = crate::core::mutex_lock(&self.paths);
        let mut out = Vec::new();
        for path in requested {
            let key = normalize_import_path(&path);
            if set.remove(&key) {
                out.push(path);
            }
        }
        out
    }

    pub fn take_all(&self) -> Vec<PathBuf> {
        let mut set = crate::core::mutex_lock(&self.paths);
        set.drain().collect()
    }
}

impl Default for PendingImports {
    fn default() -> Self {
        Self::new()
    }
}

fn normalize_import_path(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

async fn take_dropped(pending: &PendingImports) -> Vec<PathBuf> {
    for _ in 0..DROP_WAIT_ATTEMPTS {
        let paths = pending.take_all();
        if !paths.is_empty() {
            return paths;
        }
        tokio::time::sleep(DROP_WAIT_STEP).await;
    }
    Vec::new()
}

pub(crate) fn shelf_view(shelf: &crate::shelf::Shelf, stats: ShelfStats) -> ShelfView {
    ShelfView {
        id: shelf.id.clone(),
        name: shelf.name.clone(),
        managed_path: shelf.managed_path.to_string_lossy().to_string(),
        linked_folders: shelf
            .linked_folders
            .iter()
            .map(|l| LinkedView {
                source_id: crate::ids::source_id(&l.path.to_string_lossy()),
                path: l.path.to_string_lossy().to_string(),
                label: l
                    .path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| l.path.to_string_lossy().to_string()),
                available: l.path.is_dir(),
            })
            .collect(),
        stats,
        thread_id: shelf.thread_id.clone(),
        think_level: shelf.think_level,
    }
}

pub(crate) fn stats_for(ctx: &Ctx, shelf_id: &str) -> ShelfStats {
    ctx.ingest_queue.with_waiting(
        shelf_id,
        crate::core::read_lock(&ctx.library).stats(shelf_id),
    )
}

/// On Shelves visits: Reading files older than five minutes become Error.
fn expire_stale_reading(ctx: &Ctx, shelf_id: &str) {
    let now = chrono::Utc::now();
    let changed = {
        let mut library = crate::core::write_lock(&ctx.library);
        let reading_ids: std::collections::HashSet<_> = library
            .documents(shelf_id)
            .into_iter()
            .filter(|d| d.status == crate::types::DocStatus::Reading)
            .map(|d| d.id)
            .collect();
        let changed = library.expire_stale_reading(shelf_id, now);
        if changed > 0 {
            for doc in library.documents(shelf_id).into_iter().filter(|d| {
                reading_ids.contains(&d.id) && d.status == crate::types::DocStatus::Error
            }) {
                crate::ingest::emit_file_event(ctx, &doc);
            }
            if let Err(error) = library.save_documents(&ctx.paths, shelf_id) {
                log::error!("saving documents after stale reading expire: {error:#}");
            }
        }
        changed
    };
    if changed == 0 {
        return;
    }
    let stats = stats_for(ctx, shelf_id);
    ctx.events.emit(
        "larra://shelf-stats",
        json!({ "shelfId": shelf_id, "stats": stats }),
    );
}

pub(crate) fn remove_shelf_inner(ctx: &Arc<Ctx>, shelf_id: &str) -> CmdResult<Vec<String>> {
    let doc_ids = {
        let mut library = crate::core::write_lock(&ctx.library);
        library
            .remove_shelf(&ctx.paths, shelf_id)
            .map_err(friendly)?
    };
    ctx.search.remove_shelf(shelf_id).map_err(friendly)?;
    crate::ingest::remove_documents(ctx, shelf_id, &doc_ids);
    Ok(doc_ids)
}

/// List shelves with stats.
#[tauri::command]
pub fn shelves_list(
    ctx: State<'_, Arc<Ctx>>,
    watcher: State<'_, Arc<WatcherHub>>,
) -> Vec<ShelfView> {
    watcher.rebuild();
    let ids: Vec<String> = crate::core::read_lock(&ctx.library)
        .visible_shelves()
        .map(|s| s.id.clone())
        .collect();
    for id in &ids {
        expire_stale_reading(&ctx, id);
    }
    let library = crate::core::read_lock(&ctx.library);
    library
        .visible_shelves()
        .map(|s| {
            shelf_view(
                s,
                ctx.ingest_queue.with_waiting(&s.id, library.stats(&s.id)),
            )
        })
        .collect()
}

/// One Shelf, including conversation-only upload shelves.
#[tauri::command]
pub fn shelf_get(ctx: State<'_, Arc<Ctx>>, shelf_id: String) -> CmdResult<ShelfView> {
    require_id(&shelf_id)?;
    let library = crate::core::read_lock(&ctx.library);
    let shelf = library.shelf(&shelf_id).ok_or("Shelf not found")?;
    Ok(shelf_view(
        shelf,
        ctx.ingest_queue
            .with_waiting(&shelf_id, library.stats(&shelf_id)),
    ))
}

/// Create a shelf under the configured root.
#[tauri::command]
pub async fn shelf_create(
    ctx: State<'_, Arc<Ctx>>,
    watcher: State<'_, Arc<WatcherHub>>,
    name: String,
) -> CmdResult<ShelfView> {
    let root = ctx.paths.library_dir();
    let shelf = {
        let mut library = crate::core::write_lock(&ctx.library);
        library
            .create_shelf(&ctx.paths, &name, &root)
            .map_err(friendly)?
    };
    watcher.rebuild();
    ctx.events.emit("larra://shelves", json!({}));
    Ok(shelf_view(&shelf, stats_for(&ctx, &shelf.id)))
}

/// Remove a shelf and its indexed documents.
#[tauri::command]
pub async fn shelf_remove(
    ctx: State<'_, Arc<Ctx>>,
    watcher: State<'_, Arc<WatcherHub>>,
    shelf_id: String,
) -> CmdResult<()> {
    require_id(&shelf_id)?;
    ctx.ingest_queue.cancel_shelf(&shelf_id);
    remove_shelf_inner(&ctx, &shelf_id)?;
    watcher.rebuild();
    ctx.events.emit("larra://shelves", json!({}));
    Ok(())
}

/// Rename a library Shelf. The folder on disk stays put.
#[tauri::command]
pub fn shelf_rename(
    ctx: State<'_, Arc<Ctx>>,
    shelf_id: String,
    name: String,
) -> CmdResult<ShelfView> {
    require_id(&shelf_id)?;
    let shelf = {
        let mut library = crate::core::write_lock(&ctx.library);
        library
            .rename_shelf(&ctx.paths, &shelf_id, &name)
            .map_err(friendly)?
    };
    ctx.events.emit("larra://shelves", json!({}));
    Ok(shelf_view(&shelf, stats_for(&ctx, &shelf.id)))
}

/// How Chat looks through this Shelf.
#[tauri::command]
pub fn shelf_set_think_level(
    ctx: State<'_, Arc<Ctx>>,
    shelf_id: String,
    think_level: ThinkLevel,
) -> CmdResult<ShelfView> {
    require_id(&shelf_id)?;
    let shelf = {
        let mut library = crate::core::write_lock(&ctx.library);
        library
            .set_think_level(&shelf_id, think_level)
            .map_err(friendly)?
    };
    ctx.events.emit("larra://shelves", json!({}));
    Ok(shelf_view(&shelf, stats_for(&ctx, &shelf.id)))
}

/// "Add folder from this computer" — folder picker + link + scan.
#[tauri::command]
pub async fn shelf_add_linked(
    app: AppHandle,
    ctx: State<'_, Arc<Ctx>>,
    ingestor: State<'_, Ingestor>,
    watcher: State<'_, Arc<WatcherHub>>,
    shelf_id: String,
) -> CmdResult<Option<AddLinkedResult>> {
    require_id(&shelf_id)?;
    let Some(folder) = app.dialog().file().blocking_pick_folder() else {
        return Ok(None);
    };
    let folder: PathBuf = folder.into_path().map_err(friendly)?;
    let folder = folder.canonicalize().unwrap_or(folder);
    let shelf = {
        let mut library = crate::core::write_lock(&ctx.library);
        library
            .add_linked_folder(&shelf_id, &folder)
            .map_err(friendly)?
    };
    watcher.rebuild();
    let source_id = crate::ids::source_id(&folder.to_string_lossy());
    let label = folder
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let outcome = ingestor
        .sync_source(
            &shelf_id,
            &source_id,
            SourceType::Linked,
            &label,
            &folder,
            false,
        )
        .await;
    ctx.events.emit("larra://shelves", json!({}));
    Ok(Some(AddLinkedResult {
        shelf: shelf_view(&shelf, stats_for(&ctx, &shelf.id)),
        queued: outcome.new_files as u32,
        at_limit: outcome.at_limit,
    }))
}

/// Unlink a folder from a shelf.
#[tauri::command]
pub async fn shelf_remove_source(
    ctx: State<'_, Arc<Ctx>>,
    watcher: State<'_, Arc<WatcherHub>>,
    shelf_id: String,
    source_id: Option<String>,
    path: Option<String>,
) -> CmdResult<()> {
    require_id(&shelf_id)?;
    let source_id = resolve_linked_source_id(source_id, path)?;
    require_id(&source_id)?;
    ctx.ingest_queue.cancel_source(&shelf_id, &source_id);
    let doc_ids = {
        let mut library = crate::core::write_lock(&ctx.library);
        let ids = library
            .remove_linked_folder(&shelf_id, &source_id)
            .map_err(friendly)?;
        if let Err(error) = library.save_documents(&ctx.paths, &shelf_id) {
            log::error!("saving document registry for {shelf_id}: {error:#}");
        }
        ids
    };
    crate::ingest::remove_documents(&ctx, &shelf_id, &doc_ids);
    watcher.rebuild();
    ctx.events.emit("larra://shelves", json!({}));
    Ok(())
}

/// Copy dropped or picked files into a Shelf. `dropped` must already be
/// trusted (native drop, picker, or an allowlisted `shelf_import_paths`).
async fn import_copied_paths(
    ctx: &Ctx,
    ingestor: &Ingestor,
    shelf_id: String,
    dropped: Vec<PathBuf>,
) -> CmdResult<ImportResult> {
    if dropped.is_empty() {
        return Ok(ImportResult {
            queued: 0,
            names: Vec::new(),
            cancelled: false,
            at_limit: false,
            skipped_long: 0,
        });
    }
    let shelf = crate::core::read_lock(&ctx.library)
        .shelf(&shelf_id)
        .cloned()
        .ok_or("Shelf not found")?;
    let remaining = crate::ingest::remaining_file_slots(ctx, &shelf_id);
    let copied = {
        let shelf = shelf.clone();
        tokio::task::spawn_blocking(move || {
            crate::shelf::import_into_shelf(&shelf, &dropped, remaining)
        })
        .await
        .map_err(friendly)?
        .map_err(friendly)?
    };
    let mut queued = 0u32;
    let mut names = Vec::new();
    for file in &copied.files {
        let rel = file
            .strip_prefix(&shelf.managed_path)
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| file.to_string_lossy().to_string());
        ingestor
            .enqueue(Job::Process(ProcessJob {
                shelf_id: shelf_id.clone(),
                source_id: crate::shelf::Shelf::IMPORTED_SOURCE.to_string(),
                source_type: SourceType::Imported,
                source_label: "Imported".to_string(),
                abs_path: file.clone(),
                rel_path: rel,
                force: false,
                epoch: 0,
            }))
            .await;
        if let Some(name) = file.file_name() {
            names.push(name.to_string_lossy().to_string());
        }
        queued += 1;
    }
    Ok(ImportResult {
        queued,
        names,
        cancelled: false,
        at_limit: copied.at_limit,
        skipped_long: copied.skipped_long,
    })
}

/// Import files the OS just dropped or the picker just returned.
/// Empty `paths` consumes the latest native drop. Other paths must already
/// be on the allowlist; the webview cannot pass an arbitrary file.
#[tauri::command]
pub async fn shelf_import_paths(
    ctx: State<'_, Arc<Ctx>>,
    ingestor: State<'_, Ingestor>,
    pending: State<'_, PendingImports>,
    shelf_id: String,
    paths: Vec<String>,
) -> CmdResult<ImportResult> {
    require_id(&shelf_id)?;
    if crate::core::read_lock(&ctx.library)
        .shelf(&shelf_id)
        .is_none()
    {
        return Err("Shelf not found".into());
    }
    let dropped = if paths.is_empty() {
        take_dropped(&pending).await
    } else {
        pending.take_allowed(paths.into_iter().map(PathBuf::from).collect())
    };
    import_copied_paths(&ctx, &ingestor, shelf_id, dropped).await
}

/// File picker with no Shelf yet — used so Chat can cancel before creating
/// a conversation upload Shelf.
#[tauri::command]
pub async fn pick_files(
    app: AppHandle,
    pending: State<'_, PendingImports>,
) -> CmdResult<Option<Vec<String>>> {
    let Some(files) = app.dialog().file().blocking_pick_files() else {
        return Ok(None);
    };
    let paths: Vec<PathBuf> = files
        .into_iter()
        .filter_map(|f| f.into_path().ok())
        .collect();
    pending.admit(paths.iter().cloned());
    Ok(Some(
        paths
            .into_iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect(),
    ))
}

/// "Add files" button — file picker variant of import.
#[tauri::command]
pub async fn shelf_import_dialog(
    app: AppHandle,
    ctx: State<'_, Arc<Ctx>>,
    ingestor: State<'_, Ingestor>,
    shelf_id: String,
) -> CmdResult<ImportResult> {
    require_id(&shelf_id)?;
    let Some(files) = app.dialog().file().blocking_pick_files() else {
        return Ok(ImportResult {
            queued: 0,
            names: Vec::new(),
            cancelled: true,
            at_limit: false,
            skipped_long: 0,
        });
    };
    let paths: Vec<PathBuf> = files
        .into_iter()
        .filter_map(|f| f.into_path().ok())
        .collect();
    import_copied_paths(&ctx, &ingestor, shelf_id, paths).await
}

/// List documents in a shelf, newest first.
#[tauri::command]
pub fn shelf_documents(ctx: State<'_, Arc<Ctx>>, shelf_id: String) -> CmdResult<Vec<DocumentMeta>> {
    require_id(&shelf_id)?;
    expire_stale_reading(&ctx, &shelf_id);
    let mut docs = crate::core::read_lock(&ctx.library).documents(&shelf_id);
    docs.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    Ok(docs)
}

/// Read the cached card for a document.
#[tauri::command]
pub fn document_card(
    ctx: State<'_, Arc<Ctx>>,
    shelf_id: String,
    doc_id: String,
) -> CmdResult<Card> {
    require_id(&shelf_id)?;
    require_id(&doc_id)?;
    crate::ingest::card::read_card(&ctx.paths.card_path(&shelf_id, &doc_id)).map_err(friendly)
}

/// Read a window of extracted text for the source panel or drawer.
#[tauri::command]
pub fn document_text(
    ctx: State<'_, Arc<Ctx>>,
    shelf_id: String,
    doc_id: String,
    start_char: Option<u32>,
    page: Option<u32>,
    section: Option<String>,
    around: Option<String>,
    source_hash: Option<String>,
) -> CmdResult<crate::ingest::excerpt::DocumentExcerpt> {
    require_id(&shelf_id)?;
    require_id(&doc_id)?;
    let text = std::fs::read_to_string(ctx.paths.extracted_path(&shelf_id, &doc_id))
        .map_err(|_| "No extracted text yet.".to_string())?;
    let doc = crate::core::read_lock(&ctx.library).document(&shelf_id, &doc_id);
    let pages = doc.as_ref().and_then(|doc| doc.pages);
    let version_changed = source_hash
        .as_ref()
        .is_some_and(|hash| doc.as_ref().is_none_or(|doc| &doc.hash != hash));
    let start_char = if version_changed { None } else { start_char };
    let mut around = around.filter(|s| !s.trim().is_empty());
    if around.is_none() && start_char.is_none() {
        around = ctx.search.passage_needle(&doc_id, page, section.as_deref());
    }
    let mut excerpt = crate::ingest::excerpt::from_text(
        &text,
        start_char,
        &crate::ingest::excerpt::LocateHint {
            page,
            pages,
            section,
            around,
        },
    );
    excerpt.version_changed = version_changed;
    Ok(excerpt)
}

/// Force a document through ingest again.
#[tauri::command]
pub async fn document_reprocess(
    ctx: State<'_, Arc<Ctx>>,
    ingestor: State<'_, Ingestor>,
    shelf_id: String,
    doc_id: String,
) -> CmdResult<()> {
    require_id(&shelf_id)?;
    require_id(&doc_id)?;
    let doc = crate::core::read_lock(&ctx.library)
        .document(&shelf_id, &doc_id)
        .ok_or("File not found")?;
    ingestor
        .enqueue(Job::Process(ProcessJob {
            shelf_id: doc.shelf_id,
            source_id: doc.source_id,
            source_type: doc.source_type,
            source_label: doc.source_label,
            abs_path: PathBuf::from(doc.path),
            rel_path: doc.rel_path,
            force: true,
            epoch: 0,
        }))
        .await;
    Ok(())
}

/// Queue every failed file on a Shelf for another read (Resume on Sync error).
#[tauri::command]
pub async fn shelf_retry_failed(
    ctx: State<'_, Arc<Ctx>>,
    ingestor: State<'_, Ingestor>,
    shelf_id: String,
) -> CmdResult<u32> {
    require_id(&shelf_id)?;
    let docs: Vec<_> = crate::core::read_lock(&ctx.library)
        .documents(&shelf_id)
        .into_iter()
        .filter(|doc| doc.status == crate::types::DocStatus::Error)
        .collect();
    let queued = docs.len() as u32;
    for doc in docs {
        ingestor
            .enqueue(Job::Process(ProcessJob {
                shelf_id: doc.shelf_id,
                source_id: doc.source_id,
                source_type: doc.source_type,
                source_label: doc.source_label,
                abs_path: PathBuf::from(doc.path),
                rel_path: doc.rel_path,
                force: true,
                epoch: 0,
            }))
            .await;
    }
    Ok(queued)
}

pub(crate) fn ensure_shelf_path(ctx: &Ctx, path: &str) -> CmdResult<()> {
    if !crate::core::read_lock(&ctx.library).allows_open_path(std::path::Path::new(path)) {
        return Err("That file is not in a Shelf Larra knows.".into());
    }
    Ok(())
}

/// Open a shelf file in the system default app.
#[tauri::command]
pub fn open_original(app: AppHandle, ctx: State<'_, Arc<Ctx>>, path: String) -> CmdResult<()> {
    ensure_shelf_path(&ctx, &path)?;
    app.opener()
        .open_path(path, None::<String>)
        .map_err(friendly)
}

/// Reveal a shelf file in Finder or Explorer.
#[tauri::command]
pub fn reveal_item(app: AppHandle, ctx: State<'_, Arc<Ctx>>, path: String) -> CmdResult<()> {
    ensure_shelf_path(&ctx, &path)?;
    app.opener().reveal_item_in_dir(path).map_err(friendly)
}

fn resolve_linked_source_id(source_id: Option<String>, path: Option<String>) -> CmdResult<String> {
    if let Some(id) = source_id.filter(|id| !id.is_empty()) {
        return Ok(id);
    }
    if let Some(path) = path.filter(|path| !path.is_empty()) {
        return Ok(crate::ids::source_id(&path));
    }
    Err(rust_i18n::t!("errors.folderNotFound").into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::NoopEvents;
    use crate::ingest::extract::ExtractorSettings;
    use crate::paths::Paths;
    use std::sync::Arc;

    fn test_ctx() -> (tempfile::TempDir, Arc<Ctx>) {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path().join("appdata"));
        let ctx = Ctx::new(paths, Arc::new(NoopEvents), ExtractorSettings::default()).unwrap();
        (dir, ctx)
    }

    #[test]
    fn open_and_reveal_reject_paths_outside_a_shelf() {
        let (dir, ctx) = test_ctx();
        let root = dir.path().join("Larra");
        let shelf = {
            let mut library = crate::core::write_lock(&ctx.library);
            library.create_shelf(&ctx.paths, "Legal", &root).unwrap()
        };
        let inside = shelf.managed_path.join("contract.md");
        std::fs::write(&inside, "x").unwrap();
        assert_eq!(ensure_shelf_path(&ctx, &inside.to_string_lossy()), Ok(()));
        assert_eq!(
            ensure_shelf_path(&ctx, "/tmp/not-in-a-shelf.md"),
            Err("That file is not in a Shelf Larra knows.".into())
        );
    }

    #[test]
    fn document_commands_reject_traversal_ids() {
        assert!(require_id("../etc").is_err());
        assert!(require_id("s_ok").is_ok());
    }

    #[test]
    fn pending_imports_only_release_admitted_paths() {
        let pending = PendingImports::new();
        let dir = tempfile::tempdir().unwrap();
        let allowed = dir.path().join("ok.txt");
        let other = dir.path().join("other.txt");
        std::fs::write(&allowed, "a").unwrap();
        std::fs::write(&other, "b").unwrap();
        pending.admit(std::iter::once(allowed.clone()));
        let got = pending.take_allowed(vec![other.clone(), allowed.clone()]);
        assert_eq!(got, vec![allowed.clone()]);
        assert!(pending.take_all().is_empty());
    }

    #[test]
    fn pending_imports_miss_does_not_clear_admitted() {
        let pending = PendingImports::new();
        let dir = tempfile::tempdir().unwrap();
        let allowed = dir.path().join("ok.txt");
        let other = dir.path().join("other.txt");
        std::fs::write(&allowed, "a").unwrap();
        std::fs::write(&other, "b").unwrap();
        pending.admit(std::iter::once(allowed.clone()));
        assert!(pending.take_allowed(vec![other]).is_empty());
        let remaining = pending.take_all();
        assert_eq!(remaining, vec![normalize_import_path(&allowed)]);
    }

    #[test]
    fn linked_source_id_can_come_from_the_folder_path() {
        let from_path =
            resolve_linked_source_id(None, Some("/Users/example/Notes".into())).unwrap();
        assert_eq!(from_path, crate::ids::source_id("/Users/example/Notes"));
        assert_eq!(
            resolve_linked_source_id(Some("src_abc".into()), Some("/tmp".into())).unwrap(),
            "src_abc"
        );
        assert!(resolve_linked_source_id(None, None).is_err());
    }
}
