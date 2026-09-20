//! Application data layout.
//!
//! Lives next to the OS app (Application Support / `%APPDATA%`), not inside
//! the install. Replacing the app does not delete it.
//!
//! ```text
//! <app-data>/
//! ├── library/                managed Shelf folders (user files; kept on Reset)
//! ├── shelves/
//! │   └── <shelf-id>/
//! │       ├── cards/          one YAML Card per document
//! │       ├── extracted/      extracted-text cache, one .md per document
//! │       └── documents.json  per-shelf document registry
//! ├── search/tantivy/         the one application-level index
//! ├── conversations/          threads.json + one JSONL per thread
//! │                           + <thread-id>/uploads/ for chat attachments
//! ├── models/                 GGUF files
//! ├── engine/                 pinned llama.cpp build
//! ├── recipes.json            the saved-prompt library
//! ├── settings.json
//! ├── instance.lock           exclusive lock while Larra is open
//! └── logs/
//! ```

use std::path::{Path, PathBuf};

/// Managed Shelf folders (imports you drop or add). Not Documents, so macOS
/// does not prompt on New Shelf. Reset leaves this directory in place.
pub const LIBRARY_DIR: &str = "library";

#[derive(Debug, Clone)]
pub struct Paths {
    base: PathBuf,
    bundled_engine_archive: Option<PathBuf>,
}

impl Paths {
    pub fn new(base: impl Into<PathBuf>) -> Self {
        Self {
            base: base.into(),
            bundled_engine_archive: None,
        }
    }

    pub fn set_bundled_engine_archive(&mut self, path: Option<PathBuf>) {
        self.bundled_engine_archive = path;
    }

    pub fn bundled_engine_archive(&self) -> Option<&Path> {
        self.bundled_engine_archive.as_deref()
    }

    /// Ensure every directory of the layout exists (owner-only on Unix).
    pub fn ensure(&self) -> std::io::Result<()> {
        for dir in [
            self.base.clone(),
            self.library_dir(),
            self.shelves_dir(),
            self.search_dir(),
            self.conversations_dir(),
            self.models_dir(),
            self.engine_dir(),
            self.logs_dir(),
        ] {
            std::fs::create_dir_all(&dir)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
            }
        }
        Ok(())
    }

    pub fn base(&self) -> &Path {
        &self.base
    }

    /// Default root for new library Shelves (`<app-data>/library`).
    pub fn library_dir(&self) -> PathBuf {
        self.base.join(LIBRARY_DIR)
    }

    pub fn shelves_dir(&self) -> PathBuf {
        self.base.join("shelves")
    }

    pub fn shelf_registry(&self) -> PathBuf {
        self.shelves_dir().join("registry.json")
    }

    pub fn shelf_data_dir(&self, shelf_id: &str) -> PathBuf {
        self.shelves_dir().join(shelf_id)
    }

    pub fn cards_dir(&self, shelf_id: &str) -> PathBuf {
        self.shelf_data_dir(shelf_id).join("cards")
    }

    pub fn card_path(&self, shelf_id: &str, doc_id: &str) -> PathBuf {
        self.cards_dir(shelf_id).join(format!("{doc_id}.yml"))
    }

    pub fn extracted_dir(&self, shelf_id: &str) -> PathBuf {
        self.shelf_data_dir(shelf_id).join("extracted")
    }

    pub fn extracted_path(&self, shelf_id: &str, doc_id: &str) -> PathBuf {
        self.extracted_dir(shelf_id).join(format!("{doc_id}.md"))
    }

    pub fn documents_registry(&self, shelf_id: &str) -> PathBuf {
        self.shelf_data_dir(shelf_id).join("documents.json")
    }

    pub fn search_dir(&self) -> PathBuf {
        self.base.join("search").join("tantivy")
    }

    pub fn conversations_dir(&self) -> PathBuf {
        self.base.join("conversations")
    }

    pub fn threads_index(&self) -> PathBuf {
        self.conversations_dir().join("threads.json")
    }

    pub fn thread_path(&self, thread_id: &str) -> PathBuf {
        self.conversations_dir().join(format!("{thread_id}.jsonl"))
    }

    /// Copies of files attached in a conversation (deleted with the thread).
    pub fn conversation_uploads_dir(&self, thread_id: &str) -> PathBuf {
        self.conversations_dir().join(thread_id).join("uploads")
    }

    pub fn models_dir(&self) -> PathBuf {
        self.base.join("models")
    }

    pub fn engine_dir(&self) -> PathBuf {
        self.base.join("engine")
    }

    pub fn recipes_path(&self) -> PathBuf {
        self.base.join("recipes.json")
    }

    pub fn settings_path(&self) -> PathBuf {
        self.base.join("settings.json")
    }

    pub fn logs_dir(&self) -> PathBuf {
        self.base.join("logs")
    }

    pub fn engine_log_path(&self) -> PathBuf {
        self.logs_dir().join("engine.log")
    }
}

/// Serialize metadata transactions for one file without blocking independent workspaces.
pub(crate) fn metadata_lock(path: &Path) -> std::sync::Arc<std::sync::Mutex<()>> {
    use std::sync::{Arc, Mutex, OnceLock, Weak};
    static LOCKS: OnceLock<Mutex<std::collections::HashMap<PathBuf, Weak<Mutex<()>>>>> =
        OnceLock::new();
    let mut locks = crate::core::mutex_lock(LOCKS.get_or_init(|| Mutex::new(Default::default())));
    if let Some(lock) = locks.get(path).and_then(Weak::upgrade) {
        return lock;
    }
    locks.retain(|_, lock| lock.strong_count() > 0);
    let lock = Arc::new(Mutex::new(()));
    locks.insert(path.to_path_buf(), Arc::downgrade(&lock));
    lock
}

/// Replace a file using a unique, flushed sibling so concurrent writes cannot
/// rename or truncate one another's temporary file.
pub fn atomic_write(path: &Path, contents: impl AsRef<[u8]>) -> std::io::Result<()> {
    use std::io::Write;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_file_name(format!(
        ".{}.{}.tmp",
        path.file_name().unwrap_or_default().to_string_lossy(),
        uuid::Uuid::new_v4()
    ));
    let result = (|| {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&tmp)?;
        file.write_all(contents.as_ref())?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

fn backup_path(path: &Path) -> PathBuf {
    path.with_file_name(format!(
        "{}.bak",
        path.file_name().unwrap_or_default().to_string_lossy()
    ))
}

/// Keep the last valid JSON snapshot. Callers serialize read-modify-write operations.
pub fn write_json(path: &Path, value: &impl serde::Serialize) -> anyhow::Result<()> {
    let contents = serde_json::to_vec_pretty(value)?;
    if let Ok(previous) = std::fs::read(path) {
        if serde_json::from_slice::<serde_json::Value>(&previous).is_ok() {
            atomic_write(&backup_path(path), previous)?;
        } else {
            // Preserve the damaged snapshot for recovery instead of overwriting it.
            let damaged = path.with_extension(format!("corrupt-{}", uuid::Uuid::new_v4()));
            atomic_write(&damaged, previous)?;
        }
    }
    atomic_write(path, contents)?;
    Ok(())
}

pub fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Option<T> {
    if let Some(value) = std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
    {
        return Some(value);
    }
    let value = std::fs::read(backup_path(path))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok());
    if value.is_some() {
        log::warn!("using last valid metadata snapshot for {}", path.display());
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn damaged_metadata_uses_the_last_valid_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        write_json(&path, &vec!["saved"]).unwrap();
        write_json(&path, &vec!["newer"]).unwrap();
        std::fs::write(&path, "{interrupted").unwrap();
        assert_eq!(read_json::<Vec<String>>(&path).unwrap(), vec!["saved"]);
    }

    #[test]
    fn concurrent_atomic_writes_never_mix_snapshots() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("snapshot.json");
        std::thread::scope(|scope| {
            for n in 0..12 {
                let path = &path;
                scope.spawn(move || {
                    atomic_write(path, format!("\"{}\"", n.to_string().repeat(1000))).unwrap();
                });
            }
        });
        let result: String = read_json(&path).unwrap();
        assert!((0..12).any(|n| result == n.to_string().repeat(1000)));
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn library_dir_is_under_app_data() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path().join("io.larra.desktop"));
        assert_eq!(paths.library_dir(), paths.base().join(LIBRARY_DIR));
        assert_eq!(paths.engine_log_path(), paths.logs_dir().join("engine.log"));
        paths.ensure().unwrap();
        assert!(paths.library_dir().is_dir());
    }

    #[test]
    fn atomic_write_replaces_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notes.json");
        atomic_write(&path, b"one").unwrap();
        atomic_write(&path, b"two").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "two");
        assert!(!dir.path().join("notes.json.tmp").exists());
    }
}
