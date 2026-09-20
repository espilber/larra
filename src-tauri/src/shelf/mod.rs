//! Shelves — knowledge spaces with a managed folder plus linked external
//! folders, and the per-shelf document registry.

pub mod scan;
pub mod watcher;

mod longpath;

pub use scan::{rel_is_skipped, scan_new_files, MAX_FILES_PER_SHELF};

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use crate::ids;
use crate::paths::Paths;
use crate::types::{DocStatus, DocumentMeta, PiiSummaryView, ShelfStats};

/// Label in the Chat shelf selector for a conversation-only upload shelf.
pub const UPLOADED_FILES_LABEL: &str = "Uploaded files";

/// How long a file may stay Reading before opening Shelves marks it failed.
/// Matches the extract timeout. Checked only on user visits — no background timer.
pub const STALE_READING_SECS: i64 = 300;

const STALE_READING_ERROR: &str = "Reading took too long. Try again.";

/// How thoroughly Chat looks through this Shelf's files.
/// Missing `shelf.yml` values deserialize as Off. New library Shelves start at Deep.
/// Conversation upload shelves are Deep and cannot be changed.
/// `think` / `think-harder` in older yaml still load as Light / Deep.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ThinkLevel {
    #[default]
    Off,
    #[serde(alias = "think")]
    Light,
    #[serde(alias = "think-harder")]
    Deep,
}

fn think_level_is_off(level: &ThinkLevel) -> bool {
    *level == ThinkLevel::Off
}

/// `shelf.yml` inside the managed folder — the Shelf's identity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShelfConfig {
    pub schema: String,
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub linked_folders: Vec<LinkedFolder>,
    /// Set when this Shelf exists only for one conversation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    #[serde(default, skip_serializing_if = "think_level_is_off")]
    pub think_level: ThinkLevel,
}

impl ShelfConfig {
    pub const SCHEMA: &'static str = "larra-shelf/v1";
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LinkedFolder {
    pub path: PathBuf,
}

/// In-memory Shelf.
#[derive(Debug, Clone, Serialize)]
pub struct Shelf {
    pub id: String,
    pub name: String,
    pub managed_path: PathBuf,
    pub linked_folders: Vec<LinkedFolder>,
    /// When set, hidden from Shelves and deleted with that conversation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    pub think_level: ThinkLevel,
}

impl Shelf {
    /// The stable source id for imported (managed-folder) files.
    pub const IMPORTED_SOURCE: &'static str = "imported";
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct RegistryFile {
    shelves: Vec<RegistryEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RegistryEntry {
    id: String,
    name: String,
    managed_path: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    thread_id: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct DocumentsFile {
    documents: Vec<DocumentMeta>,
}

/// All shelves plus their document registries.
pub struct Library {
    shelves: Vec<Shelf>,
    /// shelf_id → (document_id → meta)
    documents: BTreeMap<String, BTreeMap<String, DocumentMeta>>,
}

impl Library {
    pub fn load(paths: &Paths) -> Result<Self> {
        let registry: RegistryFile = read_json(&paths.shelf_registry()).unwrap_or_default();
        let mut shelves = Vec::new();
        let mut documents = BTreeMap::new();
        for entry in registry.shelves {
            // shelf.yml is the source of truth for name and linked folders.
            let config_path = entry.managed_path.join("shelf.yml");
            let shelf = match read_shelf_config(&config_path) {
                Ok(config) => Shelf {
                    id: config.id,
                    name: config.name,
                    managed_path: entry.managed_path.clone(),
                    linked_folders: config.linked_folders,
                    thread_id: config.thread_id.or(entry.thread_id.clone()),
                    think_level: config.think_level,
                },
                Err(error) => {
                    log::warn!(
                        "shelf.yml unreadable at {} ({error:#}); using registry entry",
                        config_path.display()
                    );
                    Shelf {
                        id: entry.id.clone(),
                        name: entry.name.clone(),
                        managed_path: entry.managed_path.clone(),
                        linked_folders: Vec::new(),
                        thread_id: entry.thread_id.clone(),
                        think_level: ThinkLevel::Off,
                    }
                }
            };
            let docs: DocumentsFile =
                read_json(&paths.documents_registry(&shelf.id)).unwrap_or_default();
            documents.insert(
                shelf.id.clone(),
                docs.documents
                    .into_iter()
                    .map(|d| (d.id.clone(), d))
                    .collect(),
            );
            let shelf = coerce_upload_think(shelf);
            shelves.push(shelf);
        }
        Ok(Self { shelves, documents })
    }

    pub fn shelves(&self) -> &[Shelf] {
        &self.shelves
    }

    /// Shelves that belong on the Shelves view (not conversation uploads).
    pub fn visible_shelves(&self) -> impl Iterator<Item = &Shelf> {
        self.shelves.iter().filter(|s| s.thread_id.is_none())
    }

    pub fn shelf(&self, id: &str) -> Option<&Shelf> {
        self.shelves.iter().find(|s| s.id == id)
    }

    pub fn conversation_shelf(&self, thread_id: &str) -> Option<&Shelf> {
        self.shelves
            .iter()
            .find(|s| s.thread_id.as_deref() == Some(thread_id))
    }

    /// True when `path` is inside a managed Shelf folder or a linked source.
    pub fn allows_open_path(&self, path: &Path) -> bool {
        let Ok(canonical) = path.canonicalize() else {
            return false;
        };
        self.shelves.iter().any(|shelf| {
            path_is_under(&canonical, &shelf.managed_path)
                || shelf
                    .linked_folders
                    .iter()
                    .any(|linked| path_is_under(&canonical, &linked.path))
        })
    }

    fn shelf_mut(&mut self, id: &str) -> Option<&mut Shelf> {
        self.shelves.iter_mut().find(|s| s.id == id)
    }

    /// Create a Shelf: managed folder (collision-safe) + shelf.yml + registry.
    pub fn create_shelf(&mut self, paths: &Paths, name: &str, root: &Path) -> Result<Shelf> {
        let name = name.trim();
        if name.is_empty() {
            return Err(anyhow!("{}", rust_i18n::t!("errors.shelfNeedsName")));
        }
        if self
            .visible_shelves()
            .any(|s| s.name.eq_ignore_ascii_case(name))
        {
            return Err(anyhow!(
                "{}",
                rust_i18n::t!("errors.shelfExists", name = name)
            ));
        }
        let folder_name = sanitize_folder_name(name);
        std::fs::create_dir_all(root)?;
        let mut managed = root.join(&folder_name);
        let mut counter = 2;
        while managed.exists() {
            managed = root.join(format!("{folder_name} {counter}"));
            counter += 1;
        }
        std::fs::create_dir_all(&managed)?;

        let shelf = Shelf {
            id: ids::shelf_id(name),
            name: name.to_string(),
            managed_path: managed,
            linked_folders: Vec::new(),
            thread_id: None,
            think_level: ThinkLevel::Deep,
        };
        write_shelf_config(&shelf)?;
        std::fs::create_dir_all(paths.cards_dir(&shelf.id))?;
        std::fs::create_dir_all(paths.extracted_dir(&shelf.id))?;
        self.documents.insert(shelf.id.clone(), BTreeMap::new());
        self.shelves.push(shelf.clone());
        self.save_registry(paths)?;
        Ok(shelf)
    }

    /// Hidden Shelf for files attached in one conversation. Copies live in
    /// app data, not the user's Shelf folder.
    pub fn ensure_conversation_shelf(&mut self, paths: &Paths, thread_id: &str) -> Result<Shelf> {
        crate::ids::require_safe_id(thread_id)?;
        if let Some(existing) = self.conversation_shelf(thread_id).cloned() {
            if existing.think_level == ThinkLevel::Deep {
                return Ok(existing);
            }
            let id = existing.id.clone();
            if let Some(shelf) = self.shelf_mut(&id) {
                shelf.think_level = ThinkLevel::Deep;
                let snap = shelf.clone();
                write_shelf_config(&snap)?;
                return Ok(snap);
            }
            return Ok(existing);
        }
        let managed = paths.conversation_uploads_dir(thread_id);
        std::fs::create_dir_all(&managed)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&managed, std::fs::Permissions::from_mode(0o700));
        }
        let shelf = Shelf {
            id: ids::shelf_id(&format!("upload:{thread_id}")),
            name: UPLOADED_FILES_LABEL.to_string(),
            managed_path: managed,
            linked_folders: Vec::new(),
            thread_id: Some(thread_id.to_string()),
            think_level: ThinkLevel::Deep,
        };
        write_shelf_config(&shelf)?;
        std::fs::create_dir_all(paths.cards_dir(&shelf.id))?;
        std::fs::create_dir_all(paths.extracted_dir(&shelf.id))?;
        self.documents.insert(shelf.id.clone(), BTreeMap::new());
        self.shelves.push(shelf.clone());
        self.save_registry(paths)?;
        Ok(shelf)
    }

    /// Remove a Shelf from Larra. Derived data goes; a library Shelf's
    /// managed folder and original files stay on disk. Conversation upload
    /// copies are deleted with the Shelf.
    pub fn remove_shelf(&mut self, paths: &Paths, shelf_id: &str) -> Result<Vec<String>> {
        let position = self
            .shelves
            .iter()
            .position(|s| s.id == shelf_id)
            .ok_or_else(|| anyhow!("Shelf not found"))?;
        let managed = self.shelves[position].managed_path.clone();
        let purge_copies = self.shelves[position].thread_id.is_some();
        self.shelves.remove(position);
        let doc_ids: Vec<String> = self
            .documents
            .remove(shelf_id)
            .map(|docs| docs.keys().cloned().collect())
            .unwrap_or_default();
        let _ = std::fs::remove_dir_all(paths.shelf_data_dir(shelf_id));
        if purge_copies {
            let _ = std::fs::remove_dir_all(managed);
        }
        self.save_registry(paths)?;
        Ok(doc_ids)
    }

    pub fn add_linked_folder(&mut self, shelf_id: &str, folder: &Path) -> Result<Shelf> {
        if !folder.is_dir() {
            return Err(anyhow!("{}", rust_i18n::t!("errors.folderUnreadable")));
        }
        let shelf = self
            .shelf_mut(shelf_id)
            .ok_or_else(|| anyhow!("Shelf not found"))?;
        let canonical = folder
            .canonicalize()
            .unwrap_or_else(|_| folder.to_path_buf());
        if shelf.managed_path == canonical
            || shelf.linked_folders.iter().any(|l| l.path == canonical)
        {
            return Err(anyhow!("{}", rust_i18n::t!("errors.folderAlreadyOnShelf")));
        }
        shelf.linked_folders.push(LinkedFolder { path: canonical });
        let shelf = shelf.clone();
        write_shelf_config(&shelf)?;
        Ok(shelf)
    }

    /// Unlink a source folder: returns the document ids whose derived data
    /// must be removed. Original files are never touched.
    pub fn remove_linked_folder(&mut self, shelf_id: &str, source_id: &str) -> Result<Vec<String>> {
        let shelf = self
            .shelf_mut(shelf_id)
            .ok_or_else(|| anyhow!("Shelf not found"))?;
        let removed_paths: Vec<PathBuf> = shelf
            .linked_folders
            .iter()
            .filter(|linked| ids::source_id(&linked.path.to_string_lossy()) == source_id)
            .map(|linked| linked.path.clone())
            .collect();
        shelf
            .linked_folders
            .retain(|l| ids::source_id(&l.path.to_string_lossy()) != source_id);
        let shelf_snapshot = shelf.clone();
        write_shelf_config(&shelf_snapshot)?;
        let docs = self.documents.entry(shelf_id.to_string()).or_default();
        let ids: Vec<String> = docs
            .values()
            .filter(|d| {
                d.source_id == source_id
                    || removed_paths
                        .iter()
                        .any(|root| std::path::Path::new(&d.path).starts_with(root))
            })
            .map(|d| d.id.clone())
            .collect();
        for id in &ids {
            docs.remove(id);
        }
        Ok(ids)
    }

    pub fn rename_shelf(&mut self, paths: &Paths, shelf_id: &str, name: &str) -> Result<Shelf> {
        let name = name.trim();
        if name.is_empty() {
            return Err(anyhow!("{}", rust_i18n::t!("errors.shelfNeedsName")));
        }
        let shelf = self
            .shelf(shelf_id)
            .ok_or_else(|| anyhow!("Shelf not found"))?;
        if shelf.thread_id.is_some() {
            return Err(anyhow!("{}", rust_i18n::t!("errors.libraryShelvesOnly")));
        }
        if self
            .visible_shelves()
            .any(|s| s.id != shelf_id && s.name.eq_ignore_ascii_case(name))
        {
            return Err(anyhow!(
                "{}",
                rust_i18n::t!("errors.shelfExists", name = name)
            ));
        }
        let shelf = self
            .shelf_mut(shelf_id)
            .ok_or_else(|| anyhow!("Shelf not found"))?;
        if shelf.name == name {
            return Ok(shelf.clone());
        }
        shelf.name = name.to_string();
        let snap = shelf.clone();
        write_shelf_config(&snap)?;
        self.save_registry(paths)?;
        Ok(snap)
    }

    pub fn set_think_level(&mut self, shelf_id: &str, level: ThinkLevel) -> Result<Shelf> {
        let shelf = self
            .shelf_mut(shelf_id)
            .ok_or_else(|| anyhow!("Shelf not found"))?;
        if shelf.thread_id.is_some() {
            return Err(anyhow!("{}", rust_i18n::t!("errors.libraryShelvesOnly")));
        }
        shelf.think_level = level;
        let snap = shelf.clone();
        write_shelf_config(&snap)?;
        Ok(snap)
    }

    pub fn documents(&self, shelf_id: &str) -> Vec<DocumentMeta> {
        self.documents
            .get(shelf_id)
            .map(|m| m.values().cloned().collect())
            .unwrap_or_default()
    }

    pub fn document_count(&self, shelf_id: &str) -> usize {
        self.documents.get(shelf_id).map(|m| m.len()).unwrap_or(0)
    }

    /// True when this file is already on the Shelf, or there is still room.
    pub fn accepts_document(&self, shelf_id: &str, doc_id: &str) -> bool {
        if self.shelf(shelf_id).is_none() {
            return false;
        }
        if self.document(shelf_id, doc_id).is_some() {
            return true;
        }
        self.document_count(shelf_id) < MAX_FILES_PER_SHELF
    }

    /// True while this source still belongs to the Shelf (imported, or a
    /// linked folder that has not been unlinked).
    pub fn source_is_live(&self, shelf_id: &str, source_id: &str) -> bool {
        let Some(shelf) = self.shelf(shelf_id) else {
            return false;
        };
        if source_id == Shelf::IMPORTED_SOURCE {
            return true;
        }
        shelf
            .linked_folders
            .iter()
            .any(|linked| ids::source_id(&linked.path.to_string_lossy()) == source_id)
    }

    pub fn document(&self, shelf_id: &str, doc_id: &str) -> Option<DocumentMeta> {
        self.documents.get(shelf_id)?.get(doc_id).cloned()
    }

    pub fn find_document_by_path(&self, shelf_id: &str, path: &Path) -> Option<DocumentMeta> {
        let path = path.to_string_lossy();
        self.documents
            .get(shelf_id)?
            .values()
            .find(|d| d.path == path)
            .cloned()
    }

    pub fn upsert_document(&mut self, meta: DocumentMeta) {
        if self.shelf(&meta.shelf_id).is_none() {
            return;
        }
        self.documents
            .entry(meta.shelf_id.clone())
            .or_default()
            .insert(meta.id.clone(), meta);
    }

    /// Turn Reading files older than [`STALE_READING_SECS`] into Error.
    /// Returns how many files changed. Caller persists when non-zero.
    pub fn expire_stale_reading(
        &mut self,
        shelf_id: &str,
        now: chrono::DateTime<chrono::Utc>,
    ) -> usize {
        let Some(docs) = self.documents.get_mut(shelf_id) else {
            return 0;
        };
        let mut changed = 0usize;
        for doc in docs.values_mut() {
            if doc.status != DocStatus::Reading {
                continue;
            }
            let Ok(updated) = chrono::DateTime::parse_from_rfc3339(&doc.updated_at) else {
                continue;
            };
            let age = now.signed_duration_since(updated.with_timezone(&chrono::Utc));
            if age.num_seconds() < STALE_READING_SECS {
                continue;
            }
            doc.status = DocStatus::Error;
            doc.error = Some(STALE_READING_ERROR.to_string());
            doc.updated_at = now.to_rfc3339();
            changed += 1;
        }
        changed
    }

    pub fn remove_document(&mut self, shelf_id: &str, doc_id: &str) -> Option<DocumentMeta> {
        self.documents.get_mut(shelf_id)?.remove(doc_id)
    }

    pub fn save_documents(&self, paths: &Paths, shelf_id: &str) -> Result<()> {
        let documents = DocumentsFile {
            documents: self.documents(shelf_id),
        };
        write_json(&paths.documents_registry(shelf_id), &documents)
    }

    pub fn save_registry(&self, paths: &Paths) -> Result<()> {
        let registry = RegistryFile {
            shelves: self
                .shelves
                .iter()
                .map(|s| RegistryEntry {
                    id: s.id.clone(),
                    name: s.name.clone(),
                    managed_path: s.managed_path.clone(),
                    thread_id: s.thread_id.clone(),
                })
                .collect(),
        };
        write_json(&paths.shelf_registry(), &registry)
    }

    /// Shelf overview numbers.
    pub fn stats(&self, shelf_id: &str) -> ShelfStats {
        let mut stats = ShelfStats::default();
        let Some(docs) = self.documents.get(shelf_id) else {
            return stats;
        };
        let mut categories: BTreeMap<String, u32> = BTreeMap::new();
        for doc in docs.values() {
            stats.files += 1;
            match doc.status {
                DocStatus::Ready => stats.searchable += 1,
                DocStatus::Reading => stats.reading += 1,
                DocStatus::Error => stats.errors += 1,
            }
            if doc.pii_total > 0 {
                stats.files_with_pii += 1;
            }
            for (category, count) in &doc.pii_categories {
                *categories.entry(category.clone()).or_insert(0) += count;
            }
        }
        let total = categories.values().sum();
        stats.pii = PiiSummaryView { total, categories };
        stats
    }
}

/// Result of copying drops into a Shelf's managed folder.
pub struct ImportCopy {
    pub dir: PathBuf,
    pub files: Vec<PathBuf>,
    pub at_limit: bool,
    pub skipped_long: u32,
}

/// Copy dropped files/folders into a fresh, duplicate-safe import subfolder.
/// Only supported files are copied. Stops at `max_files`.
pub fn import_into_shelf(
    shelf: &Shelf,
    dropped: &[PathBuf],
    max_files: usize,
) -> Result<ImportCopy> {
    if max_files == 0 {
        return Ok(ImportCopy {
            dir: shelf.managed_path.join("Imports"),
            files: Vec::new(),
            at_limit: true,
            skipped_long: 0,
        });
    }
    let imports_root = shelf.managed_path.join("Imports");
    std::fs::create_dir_all(longpath::with_long_path(&imports_root))?;
    let stamp = chrono::Local::now().format("%Y-%m-%d %H-%M").to_string();
    let mut import_dir = imports_root.join(&stamp);
    let mut counter = 2;
    while longpath::with_long_path(&import_dir).exists() {
        import_dir = imports_root.join(format!("{stamp} ({counter})"));
        counter += 1;
    }
    std::fs::create_dir_all(longpath::with_long_path(&import_dir))?;

    let mut copied = Vec::new();
    let mut at_limit = false;
    let mut skipped_long = 0u32;
    for source in dropped {
        if copied.len() >= max_files {
            at_limit = true;
            break;
        }
        let readable = longpath::with_long_path(source);
        if readable.is_dir() {
            let target = unique_target(&import_dir, source)?;
            copy_dir(
                &readable,
                &target,
                &mut copied,
                max_files,
                &mut at_limit,
                &mut skipped_long,
            )?;
        } else if readable.is_file() {
            if !crate::ingest::extract::is_supported_file(source) {
                continue;
            }
            let target = unique_target(&import_dir, source)?;
            if copy_import_file(&readable, &target, &mut skipped_long)? {
                copied.push(target);
            }
        }
    }
    Ok(ImportCopy {
        dir: import_dir,
        files: copied,
        at_limit,
        skipped_long,
    })
}

fn unique_target(dir: &Path, source: &Path) -> Result<PathBuf> {
    let name = source
        .file_name()
        .ok_or_else(|| anyhow!("invalid file name"))?;
    let mut target = dir.join(name);
    let stem = target
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("file")
        .to_string();
    let ext = target
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| format!(".{e}"))
        .unwrap_or_default();
    let mut counter = 2;
    while longpath::with_long_path(&target).exists() {
        target = dir.join(format!("{stem} ({counter}){ext}"));
        counter += 1;
    }
    Ok(target)
}

fn copy_dir(
    source: &Path,
    target: &Path,
    copied: &mut Vec<PathBuf>,
    max_files: usize,
    at_limit: &mut bool,
    skipped_long: &mut u32,
) -> Result<()> {
    let remaining = max_files.saturating_sub(copied.len());
    if remaining == 0 {
        *at_limit = true;
        return Ok(());
    }
    let outcome = scan_new_files(source, remaining, &HashSet::new());
    if outcome.hit_file_cap {
        *at_limit = true;
    }
    for file in outcome.files {
        let rel = file.strip_prefix(source).unwrap_or(file.as_path());
        let dest = target.join(rel);
        if copy_import_file(&file, &dest, skipped_long)? {
            copied.push(dest);
        }
    }
    Ok(())
}

/// Copy one file. Skips destinations that cannot fit even with long-path APIs.
fn copy_import_file(source: &Path, dest: &Path, skipped_long: &mut u32) -> Result<bool> {
    if longpath::dest_too_long(dest) {
        *skipped_long += 1;
        return Ok(false);
    }
    if let Some(parent) = dest.parent() {
        if let Err(error) = std::fs::create_dir_all(longpath::with_long_path(parent)) {
            if longpath::is_path_length_error(&error) {
                *skipped_long += 1;
                return Ok(false);
            }
            return Err(error).with_context(|| format!("copy {}", source.display()));
        }
    }
    match std::fs::copy(
        longpath::with_long_path(source),
        longpath::with_long_path(dest),
    ) {
        Ok(_) => Ok(true),
        Err(error) if longpath::is_path_length_error(&error) => {
            *skipped_long += 1;
            Ok(false)
        }
        Err(error) => Err(error).with_context(|| format!("copy {}", source.display())),
    }
}

fn path_is_under(path: &Path, root: &Path) -> bool {
    match root.canonicalize() {
        Ok(root) => path.starts_with(&root),
        Err(_) => path.starts_with(root),
    }
}

fn sanitize_folder_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => ' ',
            c => c,
        })
        .collect();
    let cleaned = cleaned.trim().trim_end_matches('.').to_string();
    if cleaned.is_empty() {
        "Shelf".to_string()
    } else {
        cleaned
    }
}

fn coerce_upload_think(mut shelf: Shelf) -> Shelf {
    if shelf.thread_id.is_some() {
        shelf.think_level = ThinkLevel::Deep;
    }
    shelf
}

fn read_shelf_config(path: &Path) -> Result<ShelfConfig> {
    let text = std::fs::read_to_string(path)?;
    Ok(serde_yaml_ng::from_str(&text)?)
}

fn write_shelf_config(shelf: &Shelf) -> Result<()> {
    let config = ShelfConfig {
        schema: ShelfConfig::SCHEMA.to_string(),
        id: shelf.id.clone(),
        name: shelf.name.clone(),
        linked_folders: shelf.linked_folders.clone(),
        thread_id: shelf.thread_id.clone(),
        think_level: shelf.think_level,
    };
    let text = serde_yaml_ng::to_string(&config)?;
    std::fs::write(shelf.managed_path.join("shelf.yml"), text)?;
    Ok(())
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Option<T> {
    crate::paths::read_json(path)
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    crate::paths::write_json(path, value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expire_stale_reading_only_touches_old_reading_files() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path().join("appdata"));
        paths.ensure().unwrap();
        let root = dir.path().join("Larra");
        let mut library = Library::load(&paths).unwrap();
        let shelf = library.create_shelf(&paths, "Docs", &root).unwrap();
        let now = chrono::DateTime::parse_from_rfc3339("2026-08-15T12:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let fresh = (now - chrono::Duration::seconds(60)).to_rfc3339();
        let stale = (now - chrono::Duration::seconds(STALE_READING_SECS + 1)).to_rfc3339();
        library.upsert_document(crate::types::DocumentMeta {
            id: "d_fresh".into(),
            shelf_id: shelf.id.clone(),
            source_id: Shelf::IMPORTED_SOURCE.into(),
            source_type: crate::types::SourceType::Imported,
            path: "/tmp/fresh.md".into(),
            rel_path: "fresh.md".into(),
            file_name: "fresh.md".into(),
            format: "md".into(),
            size_bytes: 1,
            mtime_ms: 0,
            hash: String::new(),
            status: DocStatus::Reading,
            error: None,
            passage_count: 0,
            pages: None,
            pii_total: 0,
            pii_categories: Default::default(),
            ocr: false,
            updated_at: fresh,
            source_label: "Imported".into(),
        });
        library.upsert_document(crate::types::DocumentMeta {
            id: "d_stale".into(),
            shelf_id: shelf.id.clone(),
            source_id: Shelf::IMPORTED_SOURCE.into(),
            source_type: crate::types::SourceType::Imported,
            path: "/tmp/stale.md".into(),
            rel_path: "stale.md".into(),
            file_name: "stale.md".into(),
            format: "md".into(),
            size_bytes: 1,
            mtime_ms: 0,
            hash: String::new(),
            status: DocStatus::Reading,
            error: None,
            passage_count: 0,
            pages: None,
            pii_total: 0,
            pii_categories: Default::default(),
            ocr: false,
            updated_at: stale,
            source_label: "Imported".into(),
        });
        assert_eq!(library.expire_stale_reading(&shelf.id, now), 1);
        assert_eq!(
            library.document(&shelf.id, "d_fresh").unwrap().status,
            DocStatus::Reading
        );
        let stale_doc = library.document(&shelf.id, "d_stale").unwrap();
        assert_eq!(stale_doc.status, DocStatus::Error);
        assert_eq!(stale_doc.error.as_deref(), Some(STALE_READING_ERROR));
    }

    #[test]
    fn create_shelf_is_collision_safe() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path().join("appdata"));
        paths.ensure().unwrap();
        let root = dir.path().join("Larra");
        // A folder with the target name already exists.
        std::fs::create_dir_all(root.join("Finance")).unwrap();

        let mut library = Library::load(&paths).unwrap();
        let shelf = library.create_shelf(&paths, "Finance", &root).unwrap();
        assert_eq!(shelf.name, "Finance");
        assert!(shelf.managed_path.ends_with("Finance 2"));
        assert!(shelf.managed_path.join("shelf.yml").exists());

        // Same name again is rejected.
        assert!(library.create_shelf(&paths, "finance", &root).is_err());

        // Reload round-trips.
        let library2 = Library::load(&paths).unwrap();
        assert_eq!(library2.shelves().len(), 1);
        assert_eq!(library2.shelves()[0].name, "Finance");
        assert_eq!(library2.shelves()[0].think_level, ThinkLevel::Deep);
    }

    #[test]
    fn rename_shelf_changes_the_name_not_the_folder() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path().join("appdata"));
        paths.ensure().unwrap();
        let root = dir.path().join("Larra");
        let mut library = Library::load(&paths).unwrap();
        let shelf = library.create_shelf(&paths, "Docs", &root).unwrap();
        let managed = shelf.managed_path.clone();
        library.create_shelf(&paths, "Other", &root).unwrap();

        assert!(library.rename_shelf(&paths, &shelf.id, "  ").is_err());
        assert!(library.rename_shelf(&paths, &shelf.id, "other").is_err());

        let renamed = library.rename_shelf(&paths, &shelf.id, "Research").unwrap();
        assert_eq!(renamed.name, "Research");
        assert_eq!(renamed.managed_path, managed);
        let yaml = std::fs::read_to_string(managed.join("shelf.yml")).unwrap();
        assert!(yaml.contains("Research"), "{yaml}");

        let reloaded = Library::load(&paths).unwrap();
        assert_eq!(reloaded.shelf(&shelf.id).unwrap().name, "Research");
    }

    #[test]
    fn think_level_round_trips_in_shelf_yml() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path().join("appdata"));
        paths.ensure().unwrap();
        let root = dir.path().join("Larra");
        let mut library = Library::load(&paths).unwrap();
        let shelf = library.create_shelf(&paths, "Legal", &root).unwrap();
        let yaml = std::fs::read_to_string(shelf.managed_path.join("shelf.yml")).unwrap();
        assert!(yaml.contains("deep"), "{yaml}");
        assert_eq!(shelf.think_level, ThinkLevel::Deep);

        library.set_think_level(&shelf.id, ThinkLevel::Off).unwrap();
        let yaml = std::fs::read_to_string(shelf.managed_path.join("shelf.yml")).unwrap();
        assert!(!yaml.contains("think_level"), "{yaml}");

        library
            .set_think_level(&shelf.id, ThinkLevel::Light)
            .unwrap();
        let yaml = std::fs::read_to_string(shelf.managed_path.join("shelf.yml")).unwrap();
        assert!(yaml.contains("light"), "{yaml}");

        library
            .set_think_level(&shelf.id, ThinkLevel::Deep)
            .unwrap();
        let yaml = std::fs::read_to_string(shelf.managed_path.join("shelf.yml")).unwrap();
        assert!(yaml.contains("deep"), "{yaml}");

        let reloaded = Library::load(&paths).unwrap();
        assert_eq!(
            reloaded.shelf(&shelf.id).unwrap().think_level,
            ThinkLevel::Deep
        );

        let upload = library
            .ensure_conversation_shelf(&paths, "t_upload1")
            .unwrap();
        assert_eq!(upload.think_level, ThinkLevel::Deep);
        assert!(library
            .set_think_level(&upload.id, ThinkLevel::Light)
            .is_err());
    }

    #[test]
    fn older_think_yaml_aliases_still_load() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path().join("appdata"));
        paths.ensure().unwrap();
        let root = dir.path().join("Larra");
        let mut library = Library::load(&paths).unwrap();
        let shelf = library.create_shelf(&paths, "Legal", &root).unwrap();
        let yml = shelf.managed_path.join("shelf.yml");
        let raw = std::fs::read_to_string(&yml)
            .unwrap()
            .replace("think_level: deep", "think_level: think-harder");
        std::fs::write(&yml, &raw).unwrap();
        let reloaded = Library::load(&paths).unwrap();
        assert_eq!(
            reloaded.shelf(&shelf.id).unwrap().think_level,
            ThinkLevel::Deep
        );
        let raw = raw.replace("think_level: think-harder", "think_level: think");
        std::fs::write(&yml, raw).unwrap();
        let reloaded = Library::load(&paths).unwrap();
        assert_eq!(
            reloaded.shelf(&shelf.id).unwrap().think_level,
            ThinkLevel::Light
        );
    }

    #[test]
    fn import_creates_stamped_folder_and_copies() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path().join("appdata"));
        paths.ensure().unwrap();
        let root = dir.path().join("Larra");
        let mut library = Library::load(&paths).unwrap();
        let shelf = library.create_shelf(&paths, "Docs", &root).unwrap();

        let source = dir.path().join("invoice.txt");
        std::fs::write(&source, "Invoice INV-1 total 100 EUR").unwrap();
        let first =
            import_into_shelf(&shelf, std::slice::from_ref(&source), MAX_FILES_PER_SHELF).unwrap();
        assert!(first.dir.starts_with(shelf.managed_path.join("Imports")));
        assert_eq!(first.files.len(), 1);
        // Original untouched.
        assert!(source.exists());

        // Second import of the same file gets its own folder — no overwrite.
        let second =
            import_into_shelf(&shelf, std::slice::from_ref(&source), MAX_FILES_PER_SHELF).unwrap();
        assert_ne!(first.dir, second.dir);
        assert_eq!(second.files.len(), 1);
    }

    #[test]
    fn import_skips_packages_and_unsupported_types_and_respects_the_cap() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path().join("appdata"));
        paths.ensure().unwrap();
        let root = dir.path().join("Larra");
        let mut library = Library::load(&paths).unwrap();
        let shelf = library.create_shelf(&paths, "Docs", &root).unwrap();

        let bundle = dir.path().join("drop");
        std::fs::create_dir_all(bundle.join("node_modules/pkg")).unwrap();
        std::fs::write(bundle.join("node_modules/pkg/readme.md"), "no").unwrap();
        std::fs::write(bundle.join("clip.mp4"), "video").unwrap();
        std::fs::write(bundle.join("a.md"), "one").unwrap();
        std::fs::write(bundle.join("b.md"), "two").unwrap();
        std::fs::write(bundle.join("c.md"), "three").unwrap();

        let copied = import_into_shelf(&shelf, std::slice::from_ref(&bundle), 2).unwrap();
        assert_eq!(copied.files.len(), 2);
        assert!(copied.at_limit);
        assert!(copied.files.iter().all(|p| {
            let name = p.file_name().unwrap();
            name == "a.md" || name == "b.md" || name == "c.md"
        }));
        assert!(copied
            .files
            .iter()
            .all(|p| { !p.components().any(|c| c.as_os_str() == "node_modules") }));
    }

    #[test]
    fn import_copies_only_readable_types() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path().join("appdata"));
        paths.ensure().unwrap();
        let root = dir.path().join("Larra");
        let mut library = Library::load(&paths).unwrap();
        let shelf = library.create_shelf(&paths, "Docs", &root).unwrap();

        let md = dir.path().join("note.md");
        let mp4 = dir.path().join("clip.mp4");
        let json = dir.path().join("data.json");
        std::fs::write(&md, "ok").unwrap();
        std::fs::write(&mp4, "no").unwrap();
        std::fs::write(&json, "{}").unwrap();
        let copied = import_into_shelf(&shelf, &[md, mp4, json], MAX_FILES_PER_SHELF).unwrap();
        assert_eq!(copied.files.len(), 1);
        assert_eq!(copied.skipped_long, 0);
        assert_eq!(copied.files[0].file_name().unwrap(), "note.md");
        let listed: Vec<_> = std::fs::read_dir(&copied.dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name())
            .collect();
        assert!(listed.iter().all(|n| n != "clip.mp4" && n != "data.json"));
    }

    #[test]
    fn copy_skips_a_destination_whose_name_is_too_long() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("ok.md");
        std::fs::write(&source, "hi").unwrap();
        let dest = dir.path().join(format!("{}.md", "a".repeat(256)));
        let mut skipped = 0u32;
        assert!(!copy_import_file(&source, &dest, &mut skipped).unwrap());
        assert_eq!(skipped, 1);
        assert!(!dest.exists());
    }

    #[test]
    fn linked_folder_roundtrip_and_open_allowlist() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path().join("appdata"));
        paths.ensure().unwrap();
        let root = dir.path().join("Larra");
        let mut library = Library::load(&paths).unwrap();
        let shelf = library.create_shelf(&paths, "Docs", &root).unwrap();
        let linked = dir.path().join("incoming");
        std::fs::create_dir_all(&linked).unwrap();
        std::fs::write(linked.join("note.md"), "hi").unwrap();
        library.add_linked_folder(&shelf.id, &linked).unwrap();
        let source = crate::ids::source_id(&linked.canonicalize().unwrap().to_string_lossy());
        assert!(library.source_is_live(&shelf.id, &source));
        let reloaded = Library::load(&paths).unwrap();
        assert_eq!(reloaded.shelf(&shelf.id).unwrap().linked_folders.len(), 1);
        assert!(reloaded.allows_open_path(&linked.join("note.md")));
        let outside = dir.path().join("secret.txt");
        std::fs::write(&outside, "nope").unwrap();
        assert!(!reloaded.allows_open_path(&outside));
        let ids = {
            let mut library = Library::load(&paths).unwrap();
            library.remove_linked_folder(&shelf.id, &source).unwrap()
        };
        let _ = ids;
        let after = Library::load(&paths).unwrap();
        assert!(after.shelf(&shelf.id).unwrap().linked_folders.is_empty());
        assert!(!after.source_is_live(&shelf.id, &source));
    }

    #[cfg(unix)]
    #[test]
    fn symlink_escape_is_not_opened_or_scanned() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path().join("appdata"));
        paths.ensure().unwrap();
        let root = dir.path().join("Larra");
        let mut library = Library::load(&paths).unwrap();
        let shelf = library.create_shelf(&paths, "Docs", &root).unwrap();
        let linked = dir.path().join("incoming");
        std::fs::create_dir_all(&linked).unwrap();
        std::fs::write(linked.join("note.md"), "inside").unwrap();
        let secret = dir.path().join("secret.md");
        std::fs::write(&secret, "outside").unwrap();
        std::os::unix::fs::symlink(&secret, linked.join("escape.md")).unwrap();
        library.add_linked_folder(&shelf.id, &linked).unwrap();
        let reloaded = Library::load(&paths).unwrap();
        assert!(reloaded.allows_open_path(&linked.join("note.md")));
        assert!(!reloaded.allows_open_path(&secret));
        assert!(
            !reloaded.allows_open_path(&linked.join("escape.md")),
            "symlink to a file outside the Shelf must not pass the open allowlist"
        );
        let traversal = format!("{}/../../secret.md", linked.display());
        assert!(!reloaded.allows_open_path(std::path::Path::new(&traversal)));
        let scanned = crate::shelf::scan_new_files(
            &linked,
            crate::shelf::MAX_FILES_PER_SHELF,
            &HashSet::new(),
        )
        .files;
        assert_eq!(
            scanned.len(),
            1,
            "symlink must not be ingested: {scanned:?}"
        );
        assert_eq!(scanned[0].file_name().unwrap(), "note.md");
    }

    #[test]
    fn remove_shelf_drops_registry_keeps_managed_folder() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path().join("appdata"));
        paths.ensure().unwrap();
        let root = dir.path().join("Larra");
        let mut library = Library::load(&paths).unwrap();
        let shelf = library.create_shelf(&paths, "Docs", &root).unwrap();
        let managed = shelf.managed_path.clone();
        std::fs::write(managed.join("keep.md"), "original").unwrap();
        library.remove_shelf(&paths, &shelf.id).unwrap();
        assert!(Library::load(&paths).unwrap().shelf(&shelf.id).is_none());
        assert!(managed.join("keep.md").exists());
        library.upsert_document(crate::types::DocumentMeta {
            id: "d_ghost".into(),
            shelf_id: shelf.id.clone(),
            source_id: Shelf::IMPORTED_SOURCE.into(),
            source_type: crate::types::SourceType::Imported,
            path: managed.join("keep.md").to_string_lossy().into(),
            rel_path: "keep.md".into(),
            file_name: "keep.md".into(),
            format: "md".into(),
            size_bytes: 8,
            mtime_ms: 0,
            hash: String::new(),
            status: crate::types::DocStatus::Ready,
            error: None,
            passage_count: 0,
            pages: None,
            pii_total: 0,
            pii_categories: Default::default(),
            ocr: false,
            updated_at: "2026-01-01T00:00:00Z".into(),
            source_label: "Imported".into(),
        });
        assert!(library.document(&shelf.id, "d_ghost").is_none());
        assert!(!library.accepts_document(&shelf.id, "d_new"));
        assert!(!library.source_is_live(&shelf.id, Shelf::IMPORTED_SOURCE));
    }

    #[test]
    fn conversation_shelf_is_hidden_reusable_and_purged() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path().join("appdata"));
        paths.ensure().unwrap();
        let root = dir.path().join("Larra");
        let mut library = Library::load(&paths).unwrap();
        library.create_shelf(&paths, "Legal", &root).unwrap();
        let thread_a = crate::ids::thread_id();
        let thread_b = crate::ids::thread_id();
        let upload_a = library
            .ensure_conversation_shelf(&paths, &thread_a)
            .unwrap();
        let again = library
            .ensure_conversation_shelf(&paths, &thread_a)
            .unwrap();
        assert_eq!(upload_a.id, again.id);
        let upload_b = library
            .ensure_conversation_shelf(&paths, &thread_b)
            .unwrap();
        assert_ne!(upload_a.id, upload_b.id);
        assert_eq!(upload_a.name, UPLOADED_FILES_LABEL);
        assert_eq!(upload_a.think_level, ThinkLevel::Deep);
        assert!(upload_a.managed_path.starts_with(paths.conversations_dir()));
        assert_eq!(library.visible_shelves().count(), 1);
        assert_eq!(library.shelves().len(), 3);

        std::fs::write(upload_a.managed_path.join("note.md"), "secret").unwrap();
        library.remove_shelf(&paths, &upload_a.id).unwrap();
        assert!(library.conversation_shelf(&thread_a).is_none());
        assert!(!upload_a.managed_path.exists());
        assert_eq!(library.visible_shelves().count(), 1);
        assert!(library.conversation_shelf(&thread_b).is_some());
    }

    #[test]
    fn conversation_shelf_does_not_block_a_library_name() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path().join("appdata"));
        paths.ensure().unwrap();
        let root = dir.path().join("Larra");
        let mut library = Library::load(&paths).unwrap();
        library
            .ensure_conversation_shelf(&paths, &crate::ids::thread_id())
            .unwrap();
        assert!(library
            .create_shelf(&paths, UPLOADED_FILES_LABEL, &root)
            .is_ok());
    }
}
