//! Core data types shared across the pipeline, storage and UI surface.

use crate::pii::PiiSummary;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Where a document came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceType {
    /// Copied into the Shelf's managed folder (drag & drop / add files).
    Imported,
    /// Processed in place inside a linked external folder.
    Linked,
}

/// Per-file processing state, as surfaced in the Shelf table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DocStatus {
    /// "Reading your files" — extraction/OCR/indexing under way.
    Reading,
    /// "Ready to use" — searchable.
    Ready,
    /// Could not be read; visible but not indexed as evidence.
    Error,
}

/// Registry entry for one document of a Shelf (`documents.json`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentMeta {
    pub id: String,
    #[serde(alias = "shelf_id")]
    pub shelf_id: String,
    #[serde(alias = "source_id")]
    pub source_id: String,
    #[serde(alias = "source_type")]
    pub source_type: SourceType,
    /// Absolute path of the original file.
    pub path: String,
    /// Path relative to its source root (managed folder or linked folder).
    #[serde(alias = "rel_path")]
    pub rel_path: String,
    #[serde(alias = "file_name")]
    pub file_name: String,
    /// Lowercased extension, e.g. "pdf".
    pub format: String,
    #[serde(alias = "size_bytes")]
    pub size_bytes: u64,
    /// Filesystem mtime in milliseconds since the Unix epoch. `0` if unknown
    /// (older registries).
    #[serde(default, alias = "mtime_ms")]
    pub mtime_ms: u64,
    /// Content hash (`sha256:…`) of the last processed version.
    pub hash: String,
    pub status: DocStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Number of searchable passages currently in the index.
    #[serde(alias = "passage_count")]
    pub passage_count: u32,
    /// Pages (or sheets/slides) if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pages: Option<u32>,
    /// Total personal-information matches (details live on the Card).
    #[serde(alias = "pii_total")]
    pub pii_total: u32,
    /// Per-category counts, for table filtering and Shelf aggregates.
    #[serde(
        default,
        skip_serializing_if = "BTreeMap::is_empty",
        alias = "pii_categories"
    )]
    pub pii_categories: BTreeMap<String, u32>,
    /// Text came from local OCR rather than a native text layer.
    #[serde(default)]
    pub ocr: bool,
    /// RFC 3339.
    #[serde(alias = "updated_at")]
    pub updated_at: String,
    /// Display label of the source ("Imported" or the linked folder name).
    #[serde(alias = "source_label")]
    pub source_label: String,
}

/// The deterministic Card kept for each document (`larra-card/v1`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Card {
    pub schema: String,
    pub id: String,
    pub source: SourceType,
    pub path: String,
    pub hash: String,
    pub title: String,
    pub format: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub summary: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keywords: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub outline: Vec<OutlineEntry>,
    /// "full" for native text, "ocr" when text came from local OCR.
    pub quality: String,
    pub privacy: PiiSummary,
}

impl Card {
    pub const SCHEMA: &'static str = "larra-card/v1";
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutlineEntry {
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<u32>,
}

/// A structure-aware passage ready for indexing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Passage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub section: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_start: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_end: Option<u32>,
    pub body: String,
}

/// Aggregated Shelf statistics for the overview header.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShelfStats {
    pub files: u32,
    pub searchable: u32,
    pub reading: u32,
    pub errors: u32,
    /// Process jobs accepted but not yet taken by a worker. Live only; not in
    /// `documents.json`.
    #[serde(default)]
    pub waiting: u32,
    #[serde(alias = "files_with_pii")]
    pub files_with_pii: u32,
    pub pii: PiiSummaryView,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PiiSummaryView {
    pub total: u32,
    pub categories: BTreeMap<String, u32>,
}

/// Stable location and a bounded copy of the evidence used by an answer.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceAnchor {
    pub hash: String,
    pub start_char: Option<u32>,
    pub end_char: Option<u32>,
    pub quote: String,
}

/// One retrieved document passage that cleared the gate.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourcePassage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor: Option<SourceAnchor>,
    /// "S1", "S2"… as cited in the answer.
    pub sid: String,
    #[serde(alias = "document_id")]
    pub document_id: String,
    #[serde(alias = "shelf_id")]
    pub shelf_id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub section: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "page_start")]
    pub page_start: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "page_end")]
    pub page_end: Option<u32>,
    /// Passage text for the prompt. Cleared when a message is stored.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub body: String,
    pub path: String,
    pub score: f32,
}

/// One retrieved older-conversation snippet.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemorySnippet {
    pub thread_id: String,
    pub message_id: String,
    pub role: String,
    pub body: String,
    pub created_at: String,
    pub score: f32,
}
