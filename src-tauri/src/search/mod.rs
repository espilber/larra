//! Local retrieval: one Tantivy index for Shelf passages and older chat
//! messages. Lexical, deterministic, no embeddings.

pub mod gate;
mod query;
pub(crate) mod schema;
pub(crate) mod stems;

use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Mutex;
use tantivy::indexer::IndexWriterOptions;
use tantivy::schema::Field;
use tantivy::{doc, DateTime, Index, IndexReader, IndexWriter, ReloadPolicy, Term};

use crate::types::{DocumentMeta, Passage};
use schema::{build_schema, register_tokenizers, Fields, SCHEMA_VERSION, STEM_LANGS};

pub use schema::normalize_lang;

/// Filename tokens for search: drop the extension, treat punctuation as spaces.
pub(crate) fn searchable_filename(file_name: &str) -> String {
    let stem = Path::new(file_name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(file_name);
    stem.replace(['-', '_', '.'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Drop hyphens so "non-compete" and "noncompete" share a token.
pub(crate) fn compact_hyphens(text: &str) -> String {
    text.chars()
        .filter(|c| {
            !matches!(
                c,
                '-' | '_' | '\u{2010}' | '\u{2011}' | '\u{2013}' | '\u{2014}'
            )
        })
        .collect()
}

/// Lowercase and collapse whitespace so "Zebra  East" matches "zebra east".
pub(crate) fn fold_ws(text: &str) -> String {
    text.to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

pub struct SearchIndex {
    pub(crate) index: Index,
    pub(crate) reader: IndexReader,
    pub(crate) writer: Mutex<Option<IndexWriter>>,
    pub(crate) fields: Fields,
}

/// One worker and one merger. Default Tantivy is up to 8 indexers plus 4
/// merge threads, which opens too many files when tests build many indexes.
fn writer_options() -> IndexWriterOptions {
    IndexWriterOptions::builder()
        .num_worker_threads(1)
        .num_merge_threads(1)
        .memory_budget_per_thread(32 * 1024 * 1024)
        .build()
}

/// Drop a leftover index when the schema pin does not match. Callers rebuild
/// from extracted text after a wipe.
fn wipe_stale_schema(dir: &Path) {
    let existing = std::fs::read_to_string(dir.join("larra-schema-version")).ok();
    if existing.as_deref() != Some(SCHEMA_VERSION) && dir.join("meta.json").exists() {
        if let Err(error) = std::fs::remove_dir_all(dir) {
            log::warn!("wipe stale search index: {error}");
        }
    }
}

fn wipe_index_dir(dir: &Path) {
    if !dir.exists() {
        return;
    }
    if let Err(error) = std::fs::remove_dir_all(dir) {
        log::warn!("wipe unreadable search index: {error}");
    }
}

impl SearchIndex {
    /// Open (or create) the application-level index at `dir`. The index is
    /// derived data: schema mismatch or a corrupt commit wipes it. Callers
    /// rebuild from the extracted-text cache.
    pub fn open(dir: &Path) -> Result<Self> {
        Self::open_or_rebuild(dir, true)
    }

    fn open_or_rebuild(dir: &Path, rebuild_once: bool) -> Result<Self> {
        wipe_stale_schema(dir);
        match Self::try_open(dir) {
            Ok(index) => Ok(index),
            Err(error) if rebuild_once => {
                log::warn!("search index unreadable, wiping: {error:#}");
                wipe_index_dir(dir);
                Self::open_or_rebuild(dir, false)
            }
            Err(error) => Err(error),
        }
    }

    fn try_open(dir: &Path) -> Result<Self> {
        let version_file = dir.join("larra-schema-version");
        std::fs::create_dir_all(dir)?;

        let (schema, _) = build_schema();
        let index = if dir.join("meta.json").exists() {
            Index::open_in_dir(dir).context("open tantivy index")?
        } else {
            let index =
                Index::create_in_dir(dir, schema.clone()).context("create tantivy index")?;
            std::fs::write(&version_file, SCHEMA_VERSION)?;
            index
        };
        register_tokenizers(&index);

        // Re-derive typed fields from the opened index's schema.
        let schema = index.schema();
        let field = |name: &str| -> Result<Field> {
            schema
                .get_field(name)
                .with_context(|| format!("missing field {name}"))
        };
        let mut stems = BTreeMap::new();
        for lang in STEM_LANGS {
            stems.insert(*lang, field(&format!("stem_{lang}"))?);
        }
        let fields = Fields {
            record_type: field("record_type")?,
            shelf_id: field("shelf_id")?,
            document_id: field("document_id")?,
            source_id: field("source_id")?,
            source_type: field("source_type")?,
            path: field("path")?,
            filename: field("filename")?,
            title: field("title")?,
            summary: field("summary")?,
            keywords: field("keywords")?,
            section: field("section")?,
            body: field("body")?,
            stems,
            page_start: field("page_start")?,
            page_end: field("page_end")?,
            language: field("language")?,
            quality: field("quality")?,
            thread_id: field("thread_id")?,
            message_id: field("message_id")?,
            role: field("role")?,
            created_at: field("created_at")?,
        };

        let writer = index.writer_with_options(writer_options())?;
        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::Manual)
            .try_into()?;

        Ok(Self {
            index,
            reader,
            writer: Mutex::new(Some(writer)),
            fields,
        })
    }

    fn with_writer<T>(&self, f: impl FnOnce(&mut IndexWriter) -> Result<T>) -> Result<T> {
        let mut slot = crate::core::mutex_lock(&self.writer);
        let writer = slot.as_mut().context("search writer closed")?;
        f(writer)
    }

    /// Replace all passages of a document, then commit — each file becomes
    /// searchable on its own as soon as it finishes.
    pub fn index_document(
        &self,
        meta: &DocumentMeta,
        card_summary: &str,
        keywords: &[String],
        passages: &[Passage],
        language: Option<&str>,
        quality: &str,
        title: &str,
    ) -> Result<()> {
        let lang = language.and_then(normalize_lang);
        let keywords_joined = keywords.join(" ");
        let filename = searchable_filename(&meta.file_name);
        self.with_writer(|writer| {
            writer.delete_term(Term::from_field_text(self.fields.document_id, &meta.id));
            for passage in passages {
                let mut document = doc!(
                    self.fields.record_type => "passage",
                    self.fields.shelf_id => meta.shelf_id.as_str(),
                    self.fields.document_id => meta.id.as_str(),
                    self.fields.source_id => meta.source_id.as_str(),
                    self.fields.source_type => match meta.source_type {
                        crate::types::SourceType::Imported => "imported",
                        crate::types::SourceType::Linked => "linked",
                    },
                    self.fields.path => meta.path.as_str(),
                    self.fields.filename => filename.as_str(),
                    self.fields.title => title,
                    self.fields.summary => card_summary,
                    self.fields.keywords => keywords_joined.as_str(),
                    self.fields.body => passage.body.as_str(),
                    self.fields.quality => quality,
                );
                let compact_body = compact_hyphens(&passage.body);
                if compact_body != passage.body {
                    document.add_text(self.fields.body, compact_body.as_str());
                }
                if let Some(section) = &passage.section {
                    document.add_text(self.fields.section, section);
                }
                if let Some(p) = passage.page_start {
                    document.add_u64(self.fields.page_start, p as u64);
                }
                if let Some(p) = passage.page_end {
                    document.add_u64(self.fields.page_end, p as u64);
                }
                if let Some(lang) = lang {
                    document.add_text(self.fields.language, lang);
                    if let Some(stem_field) = self.fields.stems.get(lang) {
                        let stemmed_input = format!(
                            "{title}\n{filename}\n{}\n{keywords_joined}\n{}",
                            passage.section.as_deref().unwrap_or(""),
                            passage.body
                        );
                        document.add_text(*stem_field, &stemmed_input);
                        if compact_body != passage.body {
                            document.add_text(*stem_field, compact_body.as_str());
                        }
                    }
                }
                writer.add_document(document)?;
            }
            Ok(())
        })?;
        self.commit()
    }

    /// Index one conversation message for older-conversation memory.
    pub fn index_message(
        &self,
        thread_id: &str,
        message_id: &str,
        role: &str,
        text: &str,
        language: Option<&str>,
        created_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<()> {
        let lang = language.and_then(normalize_lang);
        self.with_writer(|writer| {
            writer.delete_term(Term::from_field_text(self.fields.message_id, message_id));
            let mut document = doc!(
                self.fields.record_type => "message",
                self.fields.thread_id => thread_id,
                self.fields.message_id => message_id,
                self.fields.role => role,
                self.fields.body => text,
                self.fields.created_at => DateTime::from_timestamp_secs(created_at.timestamp()),
            );
            if let Some(lang) = lang {
                document.add_text(self.fields.language, lang);
                if let Some(stem_field) = self.fields.stems.get(lang) {
                    document.add_text(*stem_field, text);
                }
            }
            writer.add_document(document)?;
            Ok(())
        })?;
        self.commit()
    }

    pub fn remove_document(&self, document_id: &str) -> Result<()> {
        self.with_writer(|writer| {
            writer.delete_term(Term::from_field_text(self.fields.document_id, document_id));
            Ok(())
        })?;
        self.commit()
    }

    pub fn remove_documents(&self, document_ids: &[String]) -> Result<()> {
        self.with_writer(|writer| {
            for id in document_ids {
                writer.delete_term(Term::from_field_text(self.fields.document_id, id));
            }
            Ok(())
        })?;
        self.commit()
    }

    pub fn remove_shelf(&self, shelf_id: &str) -> Result<()> {
        self.with_writer(|writer| {
            writer.delete_term(Term::from_field_text(self.fields.shelf_id, shelf_id));
            Ok(())
        })?;
        self.commit()
    }

    pub fn remove_thread(&self, thread_id: &str) -> Result<()> {
        self.with_writer(|writer| {
            writer.delete_term(Term::from_field_text(self.fields.thread_id, thread_id));
            Ok(())
        })?;
        self.commit()
    }

    fn commit(&self) -> Result<()> {
        self.with_writer(|writer| {
            writer.commit()?;
            Ok(())
        })?;
        self.reader.reload()?;
        Ok(())
    }
}

impl Drop for SearchIndex {
    fn drop(&mut self) {
        let writer = match self.writer.get_mut() {
            Ok(slot) => slot.take(),
            Err(poisoned) => poisoned.into_inner().take(),
        };
        if let Some(writer) = writer {
            if let Err(error) = writer.wait_merging_threads() {
                log::warn!("search writer shutdown: {error}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{DocStatus, DocumentMeta, Passage, SourceType};

    #[test]
    fn fold_ws_collapses_case_and_spaces() {
        assert_eq!(fold_ws("Zebra  East Wing"), "zebra east wing");
        assert_eq!(fold_ws("  already  folded  "), "already folded");
    }

    fn sample_meta() -> DocumentMeta {
        DocumentMeta {
            id: "d1".into(),
            shelf_id: "s1".into(),
            source_id: "imported".into(),
            source_type: SourceType::Imported,
            path: "/note.md".into(),
            rel_path: "note.md".into(),
            file_name: "note.md".into(),
            format: "md".into(),
            size_bytes: 32,
            mtime_ms: 0,
            hash: "sha256:d1".into(),
            status: DocStatus::Ready,
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

    fn add_sample(search: &SearchIndex) {
        search
            .index_document(
                &sample_meta(),
                "",
                &[],
                &[Passage {
                    section: None,
                    page_start: None,
                    page_end: None,
                    body: "unique-rebuild-token lives here".into(),
                }],
                Some("en"),
                "full",
                "Note",
            )
            .unwrap();
    }

    #[test]
    fn open_wipes_a_corrupt_index_and_accepts_writes() {
        let dir = tempfile::tempdir().unwrap();
        {
            let search = SearchIndex::open(dir.path()).unwrap();
            add_sample(&search);
            assert!(search.num_docs() > 0);
        }
        std::fs::write(dir.path().join("meta.json"), "{not a tantivy index").unwrap();

        let search = SearchIndex::open(dir.path()).unwrap();
        assert_eq!(search.num_docs(), 0);
        add_sample(&search);
        assert_eq!(
            search
                .search_passages("unique-rebuild-token", "s1", 8)
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn open_wipes_on_schema_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        {
            let search = SearchIndex::open(dir.path()).unwrap();
            add_sample(&search);
            assert!(search.num_docs() > 0);
        }
        std::fs::write(dir.path().join("larra-schema-version"), "larra-search/v0").unwrap();

        let search = SearchIndex::open(dir.path()).unwrap();
        assert_eq!(search.num_docs(), 0);
    }
}
