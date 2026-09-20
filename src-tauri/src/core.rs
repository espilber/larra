//! The core application context — everything the pipeline, chat and
//! commands share. Deliberately independent of Tauri so the whole core is
//! testable headlessly; the app layer plugs in a real event sink.

use crate::ingest::extract::ExtractorSettings;
use crate::paths::Paths;
use crate::pii::PiiScanner;
use crate::search::SearchIndex;
use crate::settings::Settings;
use crate::shelf::Library;
use crate::types::ShelfStats;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::time::{Duration, Instant};

/// Recover a poisoned `RwLock` instead of crashing the desktop app.
pub(crate) fn read_lock<T>(lock: &RwLock<T>) -> RwLockReadGuard<'_, T> {
    lock.read().unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub(crate) fn write_lock<T>(lock: &RwLock<T>) -> RwLockWriteGuard<'_, T> {
    lock.write()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub(crate) fn mutex_lock<T>(lock: &Mutex<T>) -> MutexGuard<'_, T> {
    lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Event sink towards the UI. The Tauri layer forwards to the webview;
/// tests collect events in memory.
pub trait Events: Send + Sync + 'static {
    fn emit(&self, event: &str, payload: serde_json::Value);
}

pub struct NoopEvents;

impl Events for NoopEvents {
    fn emit(&self, _event: &str, _payload: serde_json::Value) {}
}

/// One path waiting in the ingest channel for a Shelf.
struct QueuedFile {
    source_id: String,
    counted: bool,
    epoch: u64,
}

/// Jobs accepted but not yet taken by a worker. Not persisted.
pub struct IngestQueue {
    waiting: Mutex<HashMap<String, u32>>,
    active: Mutex<HashMap<String, u32>>,
    queued: Mutex<HashMap<(String, PathBuf), QueuedFile>>,
    last_emit: Mutex<Option<Instant>>,
    generation: AtomicU64,
    shelf_cutoff: Mutex<HashMap<String, u64>>,
    source_cutoff: Mutex<HashMap<(String, String), u64>>,
}

pub struct IngestWork<'a> {
    queue: &'a IngestQueue,
    shelf_id: String,
}
impl Drop for IngestWork<'_> {
    fn drop(&mut self) {
        let mut active = mutex_lock(&self.queue.active);
        if let Some(n) = active.get_mut(&self.shelf_id) {
            *n = n.saturating_sub(1);
            if *n == 0 {
                active.remove(&self.shelf_id);
            }
        }
    }
}

const STATS_EMIT_GAP: Duration = Duration::from_millis(200);

impl Default for IngestQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl IngestQueue {
    pub fn new() -> Self {
        Self {
            waiting: Mutex::new(HashMap::new()),
            active: Mutex::new(HashMap::new()),
            queued: Mutex::new(HashMap::new()),
            last_emit: Mutex::new(None),
            generation: AtomicU64::new(0),
            shelf_cutoff: Mutex::new(HashMap::new()),
            source_cutoff: Mutex::new(HashMap::new()),
        }
    }

    /// Stamp a job so a later delete or unlink can invalidate it.
    pub fn stamp(&self) -> u64 {
        self.generation.fetch_add(1, Ordering::Relaxed) + 1
    }

    /// True when this job was queued before a cancel of its Shelf or source.
    pub fn is_stale(&self, shelf_id: &str, source_id: &str, epoch: u64) -> bool {
        let shelf = mutex_lock(&self.shelf_cutoff).get(shelf_id).copied();
        let source = mutex_lock(&self.source_cutoff)
            .get(&(shelf_id.to_string(), source_id.to_string()))
            .copied();
        shelf.is_some_and(|cut| epoch <= cut) || source.is_some_and(|cut| epoch <= cut)
    }

    /// Drop waiting jobs for this Shelf. In-flight work must check `is_stale`.
    pub fn cancel_shelf(&self, shelf_id: &str) {
        let now = self.generation.load(Ordering::Relaxed);
        mutex_lock(&self.shelf_cutoff).insert(shelf_id.to_string(), now);
        self.drop_queued(shelf_id, None);
    }

    /// Drop waiting jobs for one linked folder. In-flight work must check
    /// `is_stale`.
    pub fn cancel_source(&self, shelf_id: &str, source_id: &str) {
        let now = self.generation.load(Ordering::Relaxed);
        mutex_lock(&self.source_cutoff).insert((shelf_id.to_string(), source_id.to_string()), now);
        self.drop_queued(shelf_id, Some(source_id));
    }

    fn drop_queued(&self, shelf_id: &str, source_id: Option<&str>) {
        let mut queued = mutex_lock(&self.queued);
        let mut dropped_counted = 0u32;
        queued.retain(|(queued_shelf, _), meta| {
            if queued_shelf != shelf_id {
                return true;
            }
            if let Some(source_id) = source_id {
                if meta.source_id != source_id {
                    return true;
                }
            }
            if meta.counted {
                dropped_counted += 1;
            }
            false
        });
        drop(queued);
        let mut waiting = mutex_lock(&self.waiting);
        if source_id.is_none() {
            waiting.remove(shelf_id);
            return;
        }
        if dropped_counted == 0 {
            return;
        }
        if let Some(n) = waiting.get_mut(shelf_id) {
            *n = n.saturating_sub(dropped_counted);
            if *n == 0 {
                waiting.remove(shelf_id);
            }
        }
    }

    pub fn waiting(&self, shelf_id: &str) -> u32 {
        mutex_lock(&self.waiting)
            .get(shelf_id)
            .copied()
            .unwrap_or(0)
    }

    pub fn begin_work(&self, shelf_id: &str) -> IngestWork<'_> {
        *mutex_lock(&self.active)
            .entry(shelf_id.to_string())
            .or_default() += 1;
        IngestWork {
            queue: self,
            shelf_id: shelf_id.to_string(),
        }
    }

    pub fn pending(&self, shelf_id: &str) -> bool {
        // Workers increment active before removing the queued job.
        let queued = mutex_lock(&self.queued);
        queued.keys().any(|(id, _)| id == shelf_id)
            || mutex_lock(&self.active).get(shelf_id).copied().unwrap_or(0) > 0
    }

    /// How many new files this Shelf can still take, counting ones already
    /// waiting to be read.
    pub fn remaining_new_files(&self, shelf_id: &str, on_shelf: usize, cap: usize) -> usize {
        cap.saturating_sub(on_shelf.saturating_add(self.waiting(shelf_id) as usize))
    }

    /// Overlay the live waiting count onto registry stats.
    pub fn with_waiting(&self, shelf_id: &str, mut stats: ShelfStats) -> ShelfStats {
        stats.waiting = self.waiting(shelf_id);
        stats
    }

    /// Claim this path for the in-flight set. `count` is true for files that
    /// are not yet on the Shelf (shown as waiting).
    pub fn try_queue(
        &self,
        shelf_id: &str,
        source_id: &str,
        path: PathBuf,
        count: bool,
        epoch: u64,
    ) -> bool {
        let key = (shelf_id.to_string(), path);
        {
            let mut queued = mutex_lock(&self.queued);
            if queued.contains_key(&key) {
                return false;
            }
            queued.insert(
                key,
                QueuedFile {
                    source_id: source_id.to_string(),
                    counted: count,
                    epoch,
                },
            );
        }
        if count {
            *mutex_lock(&self.waiting)
                .entry(shelf_id.to_string())
                .or_insert(0) += 1;
        }
        true
    }

    /// Worker took the job: drop it from the in-flight set and waiting count.
    /// A newer job for the same path is left alone.
    pub fn start(&self, shelf_id: &str, path: &Path, epoch: u64) {
        let key = (shelf_id.to_string(), path.to_path_buf());
        let counted = {
            let mut queued = mutex_lock(&self.queued);
            let Some(meta) = queued.get(&key) else {
                return;
            };
            if meta.epoch != epoch {
                return;
            }
            let counted = meta.counted;
            queued.remove(&key);
            counted
        };
        if counted {
            let mut waiting = mutex_lock(&self.waiting);
            if let Some(n) = waiting.get_mut(shelf_id) {
                *n = n.saturating_sub(1);
                if *n == 0 {
                    waiting.remove(shelf_id);
                }
            }
        }
    }

    /// Throttle shelf-stats emits during an enqueue burst. `force` always fires.
    pub fn take_stats_emit(&self, force: bool) -> bool {
        let mut last = mutex_lock(&self.last_emit);
        let now = Instant::now();
        let due = last
            .map(|t| now.saturating_duration_since(t) >= STATS_EMIT_GAP)
            .unwrap_or(true);
        if force || due {
            *last = Some(now);
            true
        } else {
            false
        }
    }
}

/// Shared app state: paths, settings, index, shelves, and the UI event sink.
pub struct Ctx {
    pub paths: Paths,
    pub settings: RwLock<Settings>,
    pub pii: PiiScanner,
    pub search: SearchIndex,
    pub library: RwLock<Library>,
    pub events: Arc<dyn Events>,
    pub extractor: ExtractorSettings,
    pub ingest_queue: IngestQueue,
    pub(crate) runtime_plan: RwLock<Option<crate::engine::tune::SpawnPlan>>,
}

impl Ctx {
    pub fn new(
        paths: Paths,
        events: Arc<dyn Events>,
        extractor: ExtractorSettings,
    ) -> anyhow::Result<Arc<Self>> {
        paths.ensure()?;
        let settings = Settings::load(&paths.settings_path());
        let search = SearchIndex::open(&paths.search_dir())?;
        let library = Library::load(&paths)?;
        Ok(Arc::new(Self {
            paths,
            settings: RwLock::new(settings),
            pii: PiiScanner::new(),
            search,
            library: RwLock::new(library),
            events,
            extractor,
            ingest_queue: IngestQueue::new(),
            runtime_plan: RwLock::new(None),
        }))
    }

    pub fn save_settings(&self) {
        let settings = write_lock(&self.settings);
        if let Err(error) = settings.save(&self.paths.settings_path()) {
            log::error!("saving settings: {error:#}");
        }
    }

    /// Publish settings only after the same snapshot is safely on disk.
    pub fn update_settings(&self, update: impl FnOnce(&mut Settings)) -> anyhow::Result<()> {
        let mut settings = write_lock(&self.settings);
        let mut next = settings.clone();
        update(&mut next);
        next.save(&self.paths.settings_path())?;
        *settings = next;
        Ok(())
    }

    /// Local-context budget in characters — measured by the installation
    /// benchmark, defaulting conservatively before it runs.
    pub fn context_budget(&self) -> usize {
        read_lock(&self.settings)
            .context_budget_chars
            .unwrap_or(crate::search::gate::tuning::DEFAULT_BUDGET_CHARS)
    }

    /// Ingestion concurrency from available memory: one worker per ~6 GB
    /// free, between 1 and 4 — index work must never starve the machine.
    pub fn ingest_concurrency() -> usize {
        let mut sys = sysinfo::System::new();
        sys.refresh_memory();
        let available = sys.available_memory();
        ((available / (6 * 1024 * 1024 * 1024)) as usize).clamp(1, 4)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_includes_work_before_it_registers_a_document() {
        let queue = IngestQueue::new();
        let path = std::path::PathBuf::from("file.md");
        let epoch = queue.stamp();
        assert!(queue.try_queue("shelf", "source", path.clone(), true, epoch));
        let work = queue.begin_work("shelf");
        queue.start("shelf", &path, epoch);
        assert_eq!(queue.waiting("shelf"), 0);
        assert!(queue.pending("shelf"));
        drop(work);
        assert!(!queue.pending("shelf"));
    }

    #[test]
    fn try_queue_dedups_the_same_path() {
        let q = IngestQueue::new();
        let path = PathBuf::from("/tmp/a.md");
        assert!(q.try_queue("s1", "imported", path.clone(), true, 1));
        assert!(!q.try_queue("s1", "imported", path.clone(), true, 1));
        assert_eq!(q.waiting("s1"), 1);
        q.start("s1", &path, 1);
        assert_eq!(q.waiting("s1"), 0);
        assert!(q.try_queue("s1", "imported", path, true, 2));
        assert_eq!(q.waiting("s1"), 1);
    }

    #[test]
    fn known_files_do_not_count_as_waiting() {
        let q = IngestQueue::new();
        let path = PathBuf::from("/tmp/old.md");
        assert!(q.try_queue("s1", "imported", path.clone(), false, 1));
        assert_eq!(q.waiting("s1"), 0);
        q.start("s1", &path, 1);
        assert_eq!(q.waiting("s1"), 0);
    }

    #[test]
    fn cancel_shelf_zeros_waiting_and_allows_requeue() {
        let q = IngestQueue::new();
        let path = PathBuf::from("/tmp/a.md");
        let epoch = q.stamp();
        assert!(q.try_queue("s1", "imported", path.clone(), true, epoch));
        assert_eq!(q.waiting("s1"), 1);
        q.cancel_shelf("s1");
        assert_eq!(q.waiting("s1"), 0);
        assert!(q.is_stale("s1", "imported", epoch));
        let next = q.stamp();
        assert!(!q.is_stale("s1", "imported", next));
        assert!(q.try_queue("s1", "imported", path, true, next));
        assert_eq!(q.waiting("s1"), 1);
    }

    #[test]
    fn cancel_source_drops_only_that_folder() {
        let q = IngestQueue::new();
        let linked = PathBuf::from("/tmp/linked.md");
        let imported = PathBuf::from("/tmp/imported.md");
        let a = q.stamp();
        let b = q.stamp();
        assert!(q.try_queue("s1", "src_a", linked.clone(), true, a));
        assert!(q.try_queue("s1", "imported", imported.clone(), true, b));
        assert_eq!(q.waiting("s1"), 2);
        q.cancel_source("s1", "src_a");
        assert_eq!(q.waiting("s1"), 1);
        assert!(q.is_stale("s1", "src_a", a));
        assert!(!q.is_stale("s1", "imported", b));
        assert!(q.try_queue("s1", "src_a", linked, true, q.stamp()));
        q.start("s1", &imported, b);
        assert_eq!(q.waiting("s1"), 1);
    }

    #[test]
    fn start_does_not_drop_a_newer_job_for_the_same_path() {
        let q = IngestQueue::new();
        let path = PathBuf::from("/tmp/a.md");
        let old = q.stamp();
        assert!(q.try_queue("s1", "imported", path.clone(), true, old));
        q.cancel_shelf("s1");
        let next = q.stamp();
        assert!(q.try_queue("s1", "imported", path.clone(), true, next));
        assert_eq!(q.waiting("s1"), 1);
        q.start("s1", &path, old);
        assert_eq!(q.waiting("s1"), 1);
        q.start("s1", &path, next);
        assert_eq!(q.waiting("s1"), 0);
    }

    #[test]
    fn remaining_new_files_counts_waiting() {
        let q = IngestQueue::new();
        assert_eq!(q.remaining_new_files("s1", 0, 10), 10);
        assert!(q.try_queue("s1", "imported", PathBuf::from("/tmp/a.md"), true, 1));
        assert_eq!(q.remaining_new_files("s1", 0, 10), 9);
        assert_eq!(q.remaining_new_files("s1", 9, 10), 0);
        assert!(q.try_queue("s1", "imported", PathBuf::from("/tmp/b.md"), false, 2));
        assert_eq!(q.remaining_new_files("s1", 0, 10), 9);
    }
}
