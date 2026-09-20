//! Conversations — append-oriented JSONL per thread (inspectable,
//! recoverable, easy to re-index) plus a small threads.json index.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use crate::limits::{clip_chars, THINKING_MAX_CHARS};
use crate::paths::Paths;
use crate::types::SourcePassage;

use super::prompts::format_citation_legend;

/// Messages shown when a conversation opens, and each Read more click.
pub const THREAD_PAGE_SIZE: usize = 50;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadMeta {
    pub id: String,
    pub title: String,
    /// Shelf remembered for this conversation (None = No Shelf).
    #[serde(default)]
    pub shelf_id: Option<String>,
    /// Hidden upload Shelf for this conversation, if files were attached.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upload_shelf_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default)]
    pub message_count: u32,
    /// Food face for this conversation. Catalog id from `avatars`.
    #[serde(default)]
    pub avatar_id: String,
}

/// One look-through step before the answer (open a file, search, and so on).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityStep {
    pub stage: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
}

/// Keep the log short enough to reread; drop the oldest when it runs long.
pub const ACTIVITY_MAX_STEPS: usize = 24;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredMessage {
    pub id: String,
    /// "user" | "assistant"
    pub role: String,
    pub text: String,
    /// Reasoning trace, when the model produced one (shown collapsed).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking: Option<String>,
    /// What was looked through before the answer, when anything was.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub activity: Vec<ActivityStep>,
    pub ts: String,
    #[serde(default)]
    pub shelf_id: Option<String>,
    #[serde(default)]
    pub sources: Vec<SourcePassage>,
    /// "done" | "stopped" | "interrupted" | "error"
    #[serde(default = "default_status")]
    pub status: String,
}

fn default_status() -> String {
    "done".into()
}

/// Latest messages in a conversation, plus whether older ones exist.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadPage {
    pub messages: Vec<StoredMessage>,
    pub has_older: bool,
}

/// Drop passage bodies, clip thinking, and cap the look-through log before a
/// message is stored or sent to the UI.
pub(crate) fn compact_message(message: &mut StoredMessage) {
    for source in &mut message.sources {
        source.body.clear();
    }
    if let Some(thinking) = &mut message.thinking {
        *thinking = clip_chars(thinking, THINKING_MAX_CHARS);
        if thinking.is_empty() {
            message.thinking = None;
        }
    }
    if message.activity.len() > ACTIVITY_MAX_STEPS {
        let skip = message.activity.len() - ACTIVITY_MAX_STEPS;
        message.activity.drain(..skip);
    }
}

/// Record a status step. Skips the generic thinking beat; opening the same
/// file again is kept so a long file shows each window.
pub(crate) fn push_activity(log: &mut Vec<ActivityStep>, stage: &str, file: Option<String>) {
    if stage.is_empty() || stage == "thinking" {
        return;
    }
    let file = file.filter(|name| !name.is_empty());
    let repeatable = stage == "opening" || stage == "around";
    if !repeatable {
        if let Some(last) = log.last() {
            if last.stage == stage && last.file == file {
                return;
            }
        }
    }
    if log.len() >= ACTIVITY_MAX_STEPS {
        log.remove(0);
    }
    log.push(ActivityStep {
        stage: stage.to_string(),
        file,
    });
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct ThreadsFile {
    threads: Vec<ThreadMeta>,
}

pub struct Conversations;

impl Conversations {
    pub fn list(paths: &Paths) -> Vec<ThreadMeta> {
        let transaction = crate::paths::metadata_lock(&paths.threads_index());
        let _write = crate::core::mutex_lock(&transaction);
        let mut file = load_threads(paths);
        file.threads.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        file.threads
    }

    pub fn get(paths: &Paths, thread_id: &str) -> Option<ThreadMeta> {
        let transaction = crate::paths::metadata_lock(&paths.threads_index());
        let _write = crate::core::mutex_lock(&transaction);
        load_threads(paths)
            .threads
            .into_iter()
            .find(|t| t.id == thread_id)
    }

    /// Threads `search_chats` may read: this conversation, plus others on
    /// the same Shelf. No-Shelf searches this conversation only.
    pub fn searchable_thread_ids(
        paths: &Paths,
        thread_id: &str,
        shelf_id: Option<&str>,
    ) -> Vec<String> {
        let mut ids = vec![thread_id.to_string()];
        if let Some(shelf) = shelf_id.filter(|id| !id.is_empty()) {
            for other in Self::other_ids_on_shelf(paths, thread_id, shelf) {
                if !ids.contains(&other) {
                    ids.push(other);
                }
            }
        }
        ids
    }

    /// True when this thread has turns outside the prompt window, or another
    /// conversation on the same Shelf already has messages.
    pub fn chats_worth_searching(
        paths: &Paths,
        thread_id: &str,
        shelf_id: Option<&str>,
        prompt_messages: usize,
    ) -> bool {
        let total = Self::get(paths, thread_id)
            .map(|thread| thread.message_count as usize)
            .unwrap_or(0);
        if total > prompt_messages {
            return true;
        }
        shelf_id
            .filter(|id| !id.is_empty())
            .is_some_and(|shelf| Self::has_other_shelf_messages(paths, thread_id, shelf))
    }

    /// Other conversations on this Shelf that already have messages
    /// `search_chats` could return. No-Shelf threads are never listed.
    pub fn other_ids_on_shelf(paths: &Paths, thread_id: &str, shelf_id: &str) -> Vec<String> {
        if shelf_id.trim().is_empty() {
            return Vec::new();
        }
        read_threads(paths)
            .threads
            .iter()
            .filter(|thread| {
                thread.id != thread_id
                    && thread.message_count > 0
                    && thread.shelf_id.as_deref() == Some(shelf_id)
            })
            .map(|thread| thread.id.clone())
            .collect()
    }

    /// True when another conversation on this Shelf already has messages.
    pub fn has_other_shelf_messages(paths: &Paths, thread_id: &str, shelf_id: &str) -> bool {
        !Self::other_ids_on_shelf(paths, thread_id, shelf_id).is_empty()
    }

    pub fn create(paths: &Paths, shelf_id: Option<String>) -> Result<ThreadMeta> {
        let transaction = crate::paths::metadata_lock(&paths.threads_index());
        let _write = crate::core::mutex_lock(&transaction);
        let now = chrono::Utc::now().to_rfc3339();
        let mut file = read_threads(paths);
        assign_missing_avatars(&mut file.threads);
        let used: HashSet<String> = file
            .threads
            .iter()
            .map(|thread| thread.avatar_id.clone())
            .collect();
        let id = crate::ids::thread_id();
        let meta = ThreadMeta {
            id: id.clone(),
            title: "New conversation".to_string(),
            shelf_id,
            upload_shelf_id: None,
            created_at: now.clone(),
            updated_at: now,
            message_count: 0,
            avatar_id: super::avatars::pick_id(&id, &used).to_string(),
        };
        file.threads.push(meta.clone());
        write_threads(paths, &file)?;
        Ok(meta)
    }

    pub fn set_shelf(paths: &Paths, thread_id: &str, shelf_id: Option<String>) -> Result<()> {
        let transaction = crate::paths::metadata_lock(&paths.threads_index());
        let _write = crate::core::mutex_lock(&transaction);
        let mut file = read_threads(paths);
        if let Some(thread) = file.threads.iter_mut().find(|t| t.id == thread_id) {
            thread.shelf_id = shelf_id;
        }
        write_threads(paths, &file)
    }

    /// Remember the hidden upload Shelf. Does not replace the library Shelf.
    pub fn set_upload_shelf(paths: &Paths, thread_id: &str, shelf_id: String) -> Result<()> {
        let transaction = crate::paths::metadata_lock(&paths.threads_index());
        let _write = crate::core::mutex_lock(&transaction);
        let mut file = read_threads(paths);
        let thread = file
            .threads
            .iter_mut()
            .find(|t| t.id == thread_id)
            .ok_or_else(|| anyhow::anyhow!("thread not found"))?;
        thread.upload_shelf_id = Some(shelf_id);
        thread.updated_at = chrono::Utc::now().to_rfc3339();
        write_threads(paths, &file)
    }

    pub fn rename(paths: &Paths, thread_id: &str, title: &str) -> Result<()> {
        let transaction = crate::paths::metadata_lock(&paths.threads_index());
        let _write = crate::core::mutex_lock(&transaction);
        let title = clean_title(title)?;
        let mut file = read_threads(paths);
        let thread = file
            .threads
            .iter_mut()
            .find(|t| t.id == thread_id)
            .ok_or_else(|| anyhow::anyhow!("thread not found"))?;
        thread.title = title;
        thread.updated_at = chrono::Utc::now().to_rfc3339();
        write_threads(paths, &file)
    }

    pub fn delete(paths: &Paths, thread_id: &str) -> Result<()> {
        let transaction = crate::paths::metadata_lock(&paths.threads_index());
        let _write = crate::core::mutex_lock(&transaction);
        let mut file = read_threads(paths);
        file.threads.retain(|t| t.id != thread_id);
        write_threads(paths, &file)?;
        let _ = std::fs::remove_file(paths.thread_path(thread_id));
        Ok(())
    }

    /// Append a message and refresh thread metadata. An untitled thread
    /// ("New conversation") takes its title from the first user message.
    pub fn append(paths: &Paths, thread_id: &str, message: &StoredMessage) -> Result<()> {
        let transaction = crate::paths::metadata_lock(&paths.threads_index());
        let _write = crate::core::mutex_lock(&transaction);
        let mut threads = read_threads(paths);
        if !threads.threads.iter().any(|thread| thread.id == thread_id) {
            return Err(anyhow::anyhow!("thread not found"));
        }
        let path = paths.thread_path(thread_id);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .with_context(|| format!("open {}", path.display()))?;
        // A crash may leave a partial last record. Keep it separate from the new one.
        use std::io::{Read, Seek, SeekFrom};
        if let Ok(mut existing) = std::fs::File::open(&path) {
            if existing.seek(SeekFrom::End(-1)).is_ok() {
                let mut tail = [0];
                if existing.read_exact(&mut tail).is_ok() && tail[0] != b'\n' {
                    writeln!(file)?;
                }
            }
        }
        let mut stored = message.clone();
        compact_message(&mut stored);
        writeln!(file, "{}", serde_json::to_string(&stored)?)?;
        file.sync_data()?;

        if let Some(thread) = threads.threads.iter_mut().find(|t| t.id == thread_id) {
            thread.updated_at = chrono::Utc::now().to_rfc3339();
            thread.message_count += 1;
            if thread.message_count == 1
                && thread.title == "New conversation"
                && message.role == "user"
            {
                thread.title = title_from(&message.text);
            }
            if message.shelf_id.is_some() {
                thread.shelf_id = message.shelf_id.clone();
            }
        }
        write_threads(paths, &threads)?;
        Ok(())
    }

    pub fn messages(paths: &Paths, thread_id: &str) -> Vec<StoredMessage> {
        let Ok(text) = std::fs::read_to_string(paths.thread_path(thread_id)) else {
            return Vec::new();
        };
        text.lines()
            .filter_map(|line| serde_json::from_str::<StoredMessage>(line).ok())
            .map(|mut message| {
                compact_message(&mut message);
                message
            })
            .collect()
    }

    /// Newest `limit` messages, or the `limit` immediately older than `before_id`.
    pub fn page(
        paths: &Paths,
        thread_id: &str,
        before_id: Option<&str>,
        limit: usize,
    ) -> ThreadPage {
        if limit == 0 {
            return ThreadPage {
                messages: Vec::new(),
                has_older: false,
            };
        }
        let mut newest_first = Vec::new();
        let mut skipping = before_id.is_some();
        let mut found_before = before_id.is_none();
        let mut has_older = false;
        for_each_jsonl_rev(&paths.thread_path(thread_id), |message| {
            if skipping {
                if before_id == Some(message.id.as_str()) {
                    skipping = false;
                    found_before = true;
                }
                return true;
            }
            if newest_first.len() >= limit {
                has_older = true;
                return false;
            }
            newest_first.push(message);
            true
        });
        if before_id.is_some() && !found_before {
            return ThreadPage {
                messages: Vec::new(),
                has_older: false,
            };
        }
        for message in &mut newest_first {
            compact_message(message);
        }
        newest_first.reverse();
        ThreadPage {
            messages: newest_first,
            has_older,
        }
    }

    /// Recent turns for the model — newest last, bounded in size.
    pub fn recent_history(
        paths: &Paths,
        thread_id: &str,
        max_messages: usize,
        max_chars: usize,
    ) -> Vec<StoredMessage> {
        let mut selected: Vec<StoredMessage> = Vec::new();
        let mut used = 0usize;
        for_each_jsonl_rev(&paths.thread_path(thread_id), |message| {
            if message.status == "error" {
                return true;
            }
            let cost = super::history_message_text(&message).chars().count();
            if selected.len() >= max_messages || used + cost > max_chars {
                return false;
            }
            used += cost;
            selected.push(message);
            true
        });
        selected.reverse();
        selected
    }
}

pub fn thread_markdown(title: &str, messages: &[StoredMessage]) -> String {
    let mut out = format!("# {}\n", title.trim());
    for message in messages {
        let heading = if message.role == "user" {
            "You"
        } else {
            "Larra"
        };
        out.push_str(&format!("\n**{heading}**\n\n{}\n", message.text.trim()));
        if message.role != "user" && !message.sources.is_empty() {
            let legend = format_citation_legend(&message.sources);
            if !legend.is_empty() {
                out.push('\n');
                out.push_str(&legend);
                out.push('\n');
            }
        }
    }
    out
}

pub fn export_file_stem(title: &str) -> String {
    let clipped: String = title.chars().take(60).collect();
    let cleaned = clipped
        .replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "-")
        .trim()
        .trim_matches('.')
        .to_string();
    if cleaned.is_empty() {
        "conversation".into()
    } else {
        cleaned
    }
}

fn clean_title(title: &str) -> Result<String> {
    let cleaned = title.split_whitespace().collect::<Vec<_>>().join(" ");
    if cleaned.is_empty() {
        anyhow::bail!("Give this conversation a name.");
    }
    Ok(cleaned.chars().take(80).collect())
}

fn for_each_jsonl_rev(path: &Path, mut visit: impl FnMut(StoredMessage) -> bool) {
    let Ok(mut file) = File::open(path) else {
        return;
    };
    let Ok(mut pos) = file.seek(SeekFrom::End(0)) else {
        return;
    };
    let mut leftover = Vec::new();
    let mut buf = vec![0u8; 64 * 1024];
    while pos > 0 {
        let size = (buf.len() as u64).min(pos);
        pos -= size;
        if file.seek(SeekFrom::Start(pos)).is_err() {
            return;
        }
        let n = size as usize;
        if file.read_exact(&mut buf[..n]).is_err() {
            return;
        }
        let mut combined = Vec::with_capacity(n + leftover.len());
        combined.extend_from_slice(&buf[..n]);
        combined.extend_from_slice(&leftover);
        if let Some(first_nl) = combined.iter().position(|&b| b == b'\n') {
            leftover = combined[..first_nl].to_vec();
            let rest = &combined[first_nl + 1..];
            for line in rest.split(|b| *b == b'\n').rev() {
                if line.is_empty() {
                    continue;
                }
                if let Ok(message) = serde_json::from_slice::<StoredMessage>(line) {
                    if !visit(message) {
                        return;
                    }
                }
            }
        } else {
            leftover = combined;
        }
    }
    if leftover.is_empty() {
        return;
    }
    if let Ok(message) = serde_json::from_slice::<StoredMessage>(&leftover) {
        visit(message);
    }
}

fn title_from(text: &str) -> String {
    let clean = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut title: String = clean.chars().take(48).collect();
    if clean.chars().count() > 48 {
        // Cut at a word boundary.
        if let Some(pos) = title.rfind(' ') {
            title.truncate(pos);
        }
        title.push('…');
    }
    if title.is_empty() {
        "New conversation".to_string()
    } else {
        title
    }
}

fn read_threads(paths: &Paths) -> ThreadsFile {
    read_json(&paths.threads_index()).unwrap_or_default()
}

fn load_threads(paths: &Paths) -> ThreadsFile {
    let mut file = read_threads(paths);
    if assign_missing_avatars(&mut file.threads) {
        if let Err(error) = write_threads(paths, &file) {
            log::error!("saving conversation faces: {error:#}");
        }
    }
    file
}

fn assign_missing_avatars(threads: &mut [ThreadMeta]) -> bool {
    let mut used: HashSet<String> = threads
        .iter()
        .filter(|thread| super::avatars::name_for(&thread.avatar_id).is_some())
        .map(|thread| thread.avatar_id.clone())
        .collect();
    let mut changed = false;
    for thread in threads.iter_mut() {
        if super::avatars::name_for(&thread.avatar_id).is_some() {
            continue;
        }
        let id = super::avatars::pick_id(&thread.id, &used);
        used.insert(id.to_string());
        thread.avatar_id = id.to_string();
        changed = true;
    }
    changed
}

fn write_threads(paths: &Paths, file: &ThreadsFile) -> Result<()> {
    write_json(&paths.threads_index(), file)
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
    fn concurrent_creates_preserve_every_thread() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path().join("data"));
        std::thread::scope(|scope| {
            for _ in 0..12 {
                let paths = &paths;
                scope.spawn(move || {
                    Conversations::create(paths, None).unwrap();
                });
            }
        });
        assert_eq!(Conversations::list(&paths).len(), 12);
    }

    #[test]
    fn threads_roundtrip_and_title() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path());
        paths.ensure().unwrap();

        let thread = Conversations::create(&paths, None).unwrap();
        let message = StoredMessage {
            id: crate::ids::message_id(),
            role: "user".into(),
            text: "Explain EBITDA in simple terms for our monthly board meeting".into(),
            thinking: None,
            activity: Vec::new(),
            ts: chrono::Utc::now().to_rfc3339(),
            shelf_id: None,
            sources: Vec::new(),
            status: "done".into(),
        };
        Conversations::append(&paths, &thread.id, &message).unwrap();

        let listed = Conversations::list(&paths);
        assert_eq!(listed.len(), 1);
        assert!(listed[0].title.starts_with("Explain EBITDA"));
        assert_eq!(listed[0].message_count, 1);

        let messages = Conversations::messages(&paths, &thread.id);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].text, message.text);
        let other = Conversations::create(&paths, None).unwrap();
        assert!(crate::chat::avatars::name_for(&thread.avatar_id).is_some());
        assert_ne!(thread.avatar_id, other.avatar_id);
    }

    fn user_line(text: &str) -> StoredMessage {
        StoredMessage {
            id: crate::ids::message_id(),
            role: "user".into(),
            text: text.into(),
            thinking: None,
            activity: Vec::new(),
            ts: chrono::Utc::now().to_rfc3339(),
            shelf_id: None,
            sources: Vec::new(),
            status: "done".into(),
        }
    }

    #[test]
    fn shelf_memory_only_sees_the_same_shelf() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path());
        paths.ensure().unwrap();

        let kitchen = Conversations::create(&paths, Some("s_kitchen".into())).unwrap();
        Conversations::append(&paths, &kitchen.id, &user_line("Kitchen restock")).unwrap();
        let kitchen_now = Conversations::create(&paths, Some("s_kitchen".into())).unwrap();
        let office = Conversations::create(&paths, Some("s_office".into())).unwrap();
        Conversations::append(&paths, &office.id, &user_line("Office move")).unwrap();
        let loose = Conversations::create(&paths, None).unwrap();
        Conversations::append(&paths, &loose.id, &user_line("No shelf chat")).unwrap();

        assert!(Conversations::has_other_shelf_messages(
            &paths,
            &kitchen_now.id,
            "s_kitchen"
        ));
        assert_eq!(
            Conversations::other_ids_on_shelf(&paths, &kitchen_now.id, "s_kitchen"),
            vec![kitchen.id.clone()]
        );
        assert!(!Conversations::has_other_shelf_messages(
            &paths,
            &kitchen.id,
            "s_kitchen"
        ));
        assert!(Conversations::other_ids_on_shelf(&paths, &office.id, "s_office").is_empty());
        assert!(Conversations::other_ids_on_shelf(&paths, &loose.id, "").is_empty());
        assert!(
            !Conversations::other_ids_on_shelf(&paths, &office.id, "s_kitchen")
                .contains(&office.id)
        );
        assert!(Conversations::chats_worth_searching(
            &paths,
            &kitchen_now.id,
            Some("s_kitchen"),
            12
        ));
        assert!(!Conversations::chats_worth_searching(
            &paths,
            &kitchen.id,
            Some("s_kitchen"),
            1
        ));
        assert!(Conversations::chats_worth_searching(
            &paths, &loose.id, None, 0
        ));
        assert_eq!(
            Conversations::searchable_thread_ids(&paths, &kitchen_now.id, Some("s_kitchen")),
            vec![kitchen_now.id.clone(), kitchen.id.clone()]
        );
        assert_eq!(
            Conversations::searchable_thread_ids(&paths, &loose.id, None),
            vec![loose.id.clone()]
        );
    }

    #[test]
    fn list_fills_in_a_face_for_older_threads() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path());
        paths.ensure().unwrap();
        let raw = r#"{"threads":[{"id":"t_old","title":"Old","createdAt":"2026-01-01T00:00:00Z","updatedAt":"2026-01-01T00:00:00Z","messageCount":0}]}"#;
        std::fs::write(paths.threads_index(), raw).unwrap();
        let listed = Conversations::list(&paths);
        assert_eq!(listed.len(), 1);
        assert!(crate::chat::avatars::name_for(&listed[0].avatar_id).is_some());
    }

    #[test]
    fn set_shelf_does_not_clear_upload_shelf() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path());
        paths.ensure().unwrap();
        let thread = Conversations::create(&paths, None).unwrap();
        Conversations::set_upload_shelf(&paths, &thread.id, "s_up".into()).unwrap();
        Conversations::set_shelf(&paths, &thread.id, None).unwrap();
        let loaded = Conversations::get(&paths, &thread.id).unwrap();
        assert_eq!(loaded.shelf_id, None);
        assert_eq!(loaded.upload_shelf_id.as_deref(), Some("s_up"));
    }

    #[test]
    fn set_upload_shelf_keeps_the_library_shelf() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path());
        paths.ensure().unwrap();
        let thread = Conversations::create(&paths, Some("s_lib".into())).unwrap();
        Conversations::set_upload_shelf(&paths, &thread.id, "s_up".into()).unwrap();
        let loaded = Conversations::get(&paths, &thread.id).unwrap();
        assert_eq!(loaded.shelf_id.as_deref(), Some("s_lib"));
        assert_eq!(loaded.upload_shelf_id.as_deref(), Some("s_up"));
    }

    #[test]
    fn rename_keeps_the_name_after_later_messages() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path());
        paths.ensure().unwrap();

        let thread = Conversations::create(&paths, None).unwrap();
        Conversations::rename(&paths, &thread.id, "  Weekly notes  ").unwrap();
        let message = StoredMessage {
            id: crate::ids::message_id(),
            role: "user".into(),
            text: "What changed this week?".into(),
            thinking: None,
            activity: Vec::new(),
            ts: chrono::Utc::now().to_rfc3339(),
            shelf_id: None,
            sources: Vec::new(),
            status: "done".into(),
        };
        Conversations::append(&paths, &thread.id, &message).unwrap();
        assert_eq!(Conversations::list(&paths)[0].title, "Weekly notes");
    }

    #[test]
    fn rename_to_new_conversation_is_kept_after_later_messages() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path());
        paths.ensure().unwrap();

        let thread = Conversations::create(&paths, None).unwrap();
        let first = StoredMessage {
            id: crate::ids::message_id(),
            role: "user".into(),
            text: "First question about the lease".into(),
            thinking: None,
            activity: Vec::new(),
            ts: chrono::Utc::now().to_rfc3339(),
            shelf_id: None,
            sources: Vec::new(),
            status: "done".into(),
        };
        Conversations::append(&paths, &thread.id, &first).unwrap();
        Conversations::rename(&paths, &thread.id, "New conversation").unwrap();
        let second = StoredMessage {
            id: crate::ids::message_id(),
            role: "user".into(),
            text: "And the dates?".into(),
            thinking: None,
            activity: Vec::new(),
            ts: chrono::Utc::now().to_rfc3339(),
            shelf_id: None,
            sources: Vec::new(),
            status: "done".into(),
        };
        Conversations::append(&paths, &thread.id, &second).unwrap();
        assert_eq!(Conversations::list(&paths)[0].title, "New conversation");
    }

    #[test]
    fn thread_markdown_includes_answers_and_citation_titles() {
        let md = thread_markdown(
            "Lease notes",
            &[StoredMessage {
                id: "m1".into(),
                role: "assistant".into(),
                text: "Notice is 90 days. [S1]".into(),
                thinking: Some("scratch".into()),
                activity: Vec::new(),
                ts: String::new(),
                shelf_id: None,
                sources: vec![crate::types::SourcePassage {
                    anchor: None,
                    sid: "S1".into(),
                    document_id: "d1".into(),
                    shelf_id: "s1".into(),
                    title: "Lease.pdf".into(),
                    section: None,
                    page_start: Some(4),
                    page_end: None,
                    body: String::new(),
                    path: String::new(),
                    score: 1.0,
                }],
                status: "done".into(),
            }],
        );
        assert!(md.starts_with("# Lease notes"));
        assert!(md.contains("**Larra**"));
        assert!(md.contains("S1 Lease.pdf (p. 4)"));
        assert!(!md.contains("scratch"));
    }

    #[test]
    fn export_file_stem_strips_path_characters() {
        assert_eq!(export_file_stem("a/b:c"), "a-b-c");
        assert_eq!(export_file_stem("   "), "conversation");
    }

    fn stored(id: &str, text: &str) -> StoredMessage {
        StoredMessage {
            id: id.into(),
            role: "user".into(),
            text: text.into(),
            thinking: None,
            activity: Vec::new(),
            ts: chrono::Utc::now().to_rfc3339(),
            shelf_id: None,
            sources: Vec::new(),
            status: "done".into(),
        }
    }

    #[test]
    fn append_omits_source_bodies_and_clips_thinking() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path());
        paths.ensure().unwrap();
        let thread = Conversations::create(&paths, None).unwrap();
        let thinking: String = "á".repeat(THINKING_MAX_CHARS + 40);
        let mut message = stored("m_src", "See [S1].");
        message.role = "assistant".into();
        message.thinking = Some(thinking);
        message.sources = vec![crate::types::SourcePassage {
            anchor: None,
            sid: "S1".into(),
            document_id: "d1".into(),
            shelf_id: "s1".into(),
            title: "Lease.pdf".into(),
            section: None,
            page_start: Some(4),
            page_end: None,
            body: "the whole stuffed file ".repeat(200),
            path: "/tmp/lease.pdf".into(),
            score: 1.0,
        }];
        Conversations::append(&paths, &thread.id, &message).unwrap();

        let raw = std::fs::read_to_string(paths.thread_path(&thread.id)).unwrap();
        assert!(!raw.contains("the whole stuffed file"));
        let parsed: StoredMessage = serde_json::from_str(raw.trim()).unwrap();
        assert!(parsed.sources[0].body.is_empty());
        assert_eq!(
            parsed.thinking.as_ref().unwrap().chars().count(),
            THINKING_MAX_CHARS
        );
    }

    #[test]
    fn page_returns_the_latest_messages_then_older() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path());
        paths.ensure().unwrap();
        let thread = Conversations::create(&paths, None).unwrap();
        for i in 0..8 {
            Conversations::append(
                &paths,
                &thread.id,
                &stored(&format!("m{i}"), &format!("t{i}")),
            )
            .unwrap();
        }

        let first = Conversations::page(&paths, &thread.id, None, 3);
        assert_eq!(
            first
                .messages
                .iter()
                .map(|m| m.id.as_str())
                .collect::<Vec<_>>(),
            ["m5", "m6", "m7"]
        );
        assert!(first.has_older);

        let older = Conversations::page(&paths, &thread.id, Some("m5"), 3);
        assert_eq!(
            older
                .messages
                .iter()
                .map(|m| m.id.as_str())
                .collect::<Vec<_>>(),
            ["m2", "m3", "m4"]
        );
        assert!(older.has_older);

        let oldest = Conversations::page(&paths, &thread.id, Some("m2"), 3);
        assert_eq!(
            oldest
                .messages
                .iter()
                .map(|m| m.id.as_str())
                .collect::<Vec<_>>(),
            ["m0", "m1"]
        );
        assert!(!oldest.has_older);
    }

    #[test]
    fn page_reads_across_a_file_chunk() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path());
        paths.ensure().unwrap();
        let thread = Conversations::create(&paths, None).unwrap();
        let blob = "x".repeat(800);
        for i in 0..80 {
            Conversations::append(
                &paths,
                &thread.id,
                &stored(&format!("m{i}"), &format!("{blob}{i}")),
            )
            .unwrap();
        }
        let first = Conversations::page(&paths, &thread.id, None, 50);
        assert_eq!(first.messages.first().map(|m| m.id.as_str()), Some("m30"));
        assert_eq!(first.messages.last().map(|m| m.id.as_str()), Some("m79"));
        assert!(first.has_older);
        let older = Conversations::page(&paths, &thread.id, Some("m30"), 50);
        assert_eq!(older.messages.first().map(|m| m.id.as_str()), Some("m0"));
        assert_eq!(older.messages.last().map(|m| m.id.as_str()), Some("m29"));
        assert!(!older.has_older);
    }

    #[test]
    fn page_of_a_missing_thread_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path());
        paths.ensure().unwrap();
        let page = Conversations::page(&paths, "t_missing", None, 50);
        assert!(page.messages.is_empty());
        assert!(!page.has_older);
    }

    #[test]
    fn page_clears_bodies_left_in_old_files() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path());
        paths.ensure().unwrap();
        let thread = Conversations::create(&paths, None).unwrap();
        let line = serde_json::json!({
            "id": "m1",
            "role": "assistant",
            "text": "Hi",
            "ts": "now",
            "sources": [{
                "sid": "S1",
                "documentId": "d1",
                "shelfId": "s1",
                "title": "A",
                "body": "SECRET excerpt",
                "path": "",
                "score": 1.0
            }],
            "status": "done"
        });
        std::fs::write(paths.thread_path(&thread.id), format!("{line}\n")).unwrap();
        let page = Conversations::page(&paths, &thread.id, None, 50);
        assert_eq!(page.messages.len(), 1);
        assert_eq!(page.messages[0].sources[0].body, "");
        assert!(page.messages[0].activity.is_empty());
    }

    #[test]
    fn push_activity_skips_thinking_and_keeps_each_open() {
        let mut log = Vec::new();
        push_activity(&mut log, "thinking", None);
        push_activity(&mut log, "looking", None);
        push_activity(&mut log, "looking", None);
        push_activity(&mut log, "opening", Some("notes.md".into()));
        push_activity(&mut log, "opening", Some("notes.md".into()));
        push_activity(&mut log, "around", Some("notes.md".into()));
        assert_eq!(
            log.iter().map(|s| s.stage.as_str()).collect::<Vec<_>>(),
            ["looking", "opening", "opening", "around"]
        );
    }

    #[test]
    fn compact_message_keeps_the_latest_activity_steps() {
        let mut message = stored("m_act", "See [S1].");
        message.role = "assistant".into();
        for i in 0..(ACTIVITY_MAX_STEPS + 3) {
            message.activity.push(ActivityStep {
                stage: "opening".into(),
                file: Some(format!("part-{i}.md")),
            });
        }
        compact_message(&mut message);
        assert_eq!(message.activity.len(), ACTIVITY_MAX_STEPS);
        assert_eq!(message.activity[0].file.as_deref(), Some("part-3.md"));
    }
}
