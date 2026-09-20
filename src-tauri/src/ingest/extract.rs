//! Xberg integration — the single document-extraction layer.
//!
//! Everything here is deterministic and fully local: native text first,
//! vendored-tesseract OCR when a file or page has no usable text layer
//! (whatever `*.traineddata` packs are in the tessdata directory).

use anyhow::{anyhow, Context, Result};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use xberg::core::config::{LanguageDetectionConfig, OutputFormat, PageConfig};
use xberg::{
    ExtractInput, ExtractionConfig, KeywordAlgorithm, KeywordConfig, OcrConfig, OcrStrategy,
};

use crate::search::normalize_lang;
use crate::types::OutlineEntry;

/// The formats Larra accepts, intersected with Xberg's format registry at
/// startup (the registry is the source of truth for what the build handles).
const REBOST_EXTENSIONS: &[&str] = &[
    "pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "odt", "ods", "odp", "md", "markdown",
    "txt", "rtf", "csv", "tsv", "eml", "msg", "epub",
];

/// Extensions Larra offers, intersected with Xberg's registry.
pub fn supported_extensions() -> &'static HashSet<String> {
    static SUPPORTED: OnceLock<HashSet<String>> = OnceLock::new();
    SUPPORTED.get_or_init(|| {
        let registry: HashSet<String> = xberg::core::mime::list_supported_formats()
            .into_iter()
            .map(|f| f.extension.to_lowercase())
            .collect();
        REBOST_EXTENSIONS
            .iter()
            .filter(|e| registry.contains(**e))
            .map(|e| e.to_string())
            .collect()
    })
}

pub(crate) const EXTRACTED_MAX: usize = 1_000_000;

pub(crate) fn limit_extracted(text: String) -> String {
    limit_extracted_to(text, EXTRACTED_MAX)
}

fn limit_extracted_to(text: String, max: usize) -> String {
    if text.len() <= max {
        return text;
    }
    let mut end = max;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    let prefix = &text[..end];
    if let Some(at) = prefix.rfind("\n\n") {
        if at >= max / 2 {
            return prefix[..at].to_string();
        }
    }
    if let Some(at) = prefix.rfind('\n') {
        if at >= max / 2 {
            return prefix[..at].to_string();
        }
    }
    prefix.to_string()
}

fn limit_blocks(blocks: Vec<Block>) -> Vec<Block> {
    let mut used = 0usize;
    let mut out = Vec::new();
    for mut block in blocks {
        if used >= EXTRACTED_MAX {
            break;
        }
        let n = block.text.len();
        if used + n > EXTRACTED_MAX {
            let remain = EXTRACTED_MAX - used;
            if remain < 32 && !out.is_empty() {
                break;
            }
            block.text = limit_extracted_to(block.text, remain);
        }
        used += block.text.len();
        out.push(block);
    }
    out
}

/// True for names that should never be read: hidden files, Word/Excel lock
/// files (`~$…`), `*.tmp`, and the Shelf config file.
pub fn skip_file_name(name: &str) -> bool {
    if name.starts_with('.') || name.starts_with("~$") || name.eq_ignore_ascii_case("shelf.yml") {
        return true;
    }
    Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("tmp"))
}

pub fn is_supported_file(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    if skip_file_name(name) {
        return false;
    }
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| supported_extensions().contains(&e.to_lowercase()))
        .unwrap_or(false)
}

/// What the rest of the pipeline consumes.
#[derive(Debug, Clone)]
pub struct Extraction {
    /// Markdown-ish full text — also the extracted-text cache content.
    pub content: String,
    pub title: String,
    /// Normalized 2-letter language code, when detected.
    pub language: Option<&'static str>,
    /// Raw detected tag for the Card (e.g. "es").
    pub language_tag: Option<String>,
    pub summary: String,
    pub keywords: Vec<String>,
    pub outline: Vec<OutlineEntry>,
    /// Structure-aware building blocks for passages.
    pub blocks: Vec<Block>,
    pub page_count: Option<u32>,
    /// True when any of the text came from local OCR.
    pub ocr_used: bool,
}

/// One structural block, in document order.
#[derive(Debug, Clone)]
pub struct Block {
    pub kind: BlockKind,
    pub text: String,
    pub page: Option<u32>,
    pub page_end: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockKind {
    Heading { level: u8 },
    Paragraph,
    Table,
    SheetStart { name: String },
    SlideStart { number: u32, title: Option<String> },
}

#[derive(Debug, Clone)]
pub struct ExtractorSettings {
    /// Directory holding `*.traineddata` packs used at extract time.
    pub tessdata_dir: Option<PathBuf>,
    /// Bundled packs copied into `tessdata_dir` on the first extract.
    pub tessdata_bundle: Option<PathBuf>,
    /// Extraction timeout per file.
    pub timeout_secs: u64,
}

impl Default for ExtractorSettings {
    fn default() -> Self {
        Self {
            tessdata_dir: None,
            tessdata_bundle: None,
            timeout_secs: 300,
        }
    }
}

/// Tesseract language codes from `*.traineddata` in `dir` (`osd` skipped).
/// English is listed first when present so mixed-script pages have a fallback.
pub(crate) fn ocr_languages(dir: Option<&Path>) -> Vec<String> {
    let mut langs = Vec::new();
    if let Some(dir) = dir {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let Some(name) = name.to_str() else {
                    continue;
                };
                let Some(code) = name.strip_suffix(".traineddata") else {
                    continue;
                };
                if code.is_empty() || code == "osd" {
                    continue;
                }
                langs.push(code.to_string());
            }
        }
    }
    langs.sort();
    if let Some(index) = langs.iter().position(|l| l == "eng") {
        langs.remove(index);
        langs.insert(0, "eng".into());
    } else if langs.is_empty() {
        langs.push("eng".into());
    }
    langs
}

static TESSDATA_COPY: Mutex<()> = Mutex::new(());

/// True when `dir` already has at least one `*.traineddata` pack.
pub(crate) fn tessdata_has_packs(dir: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    entries.flatten().any(|entry| {
        entry
            .file_name()
            .to_str()
            .map(|name| name.ends_with(".traineddata") && entry.path().is_file())
            .unwrap_or(false)
    })
}

fn copy_traineddata(bundle: &Path, dest: &Path) {
    if let Err(error) = std::fs::create_dir_all(dest) {
        log::warn!("create tessdata dir: {error}");
        return;
    }
    let Ok(entries) = std::fs::read_dir(bundle) else {
        return;
    };
    for entry in entries.flatten() {
        let source = entry.path();
        let Some(name) = source.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !(name.ends_with(".traineddata") || name == "LICENSE") {
            continue;
        }
        let dest_file = dest.join(name);
        if dest_file.exists() {
            continue;
        }
        if let Err(error) = std::fs::copy(&source, &dest_file) {
            log::warn!("copying {name}: {error}");
        }
    }
}

fn ensure_tessdata(settings: &ExtractorSettings) {
    let Some(dest) = settings.tessdata_dir.as_ref() else {
        return;
    };
    if tessdata_has_packs(dest) {
        return;
    }
    let Some(bundle) = settings.tessdata_bundle.as_ref() else {
        return;
    };
    let _guard = TESSDATA_COPY
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if tessdata_has_packs(dest) {
        return;
    }
    log::info!("copying OCR language packs for first extract");
    copy_traineddata(bundle, dest);
}

fn build_config(settings: &ExtractorSettings) -> ExtractionConfig {
    ExtractionConfig {
        // Larra keeps its own extracted-text cache keyed by content hash.
        use_cache: false,
        output_format: OutputFormat::Markdown,
        pages: Some(PageConfig {
            extract_pages: true,
            ..Default::default()
        }),
        include_document_structure: true,
        language_detection: Some(LanguageDetectionConfig::default()),
        // Keywords and summary run post-extraction with the detected
        // language, so stopwords match the document.
        keywords: None,
        ocr: Some(OcrConfig {
            language: ocr_languages(settings.tessdata_dir.as_deref()),
            tessdata_path: settings.tessdata_dir.clone(),
            ..Default::default()
        }),
        // OCR only the pages that lack a usable text layer.
        ocr_strategy: OcrStrategy::Auto,
        extraction_timeout_secs: Some(settings.timeout_secs),
        ..Default::default()
    }
}

/// Clean a filename into a presentable title: `bank-agreement_v2.pdf` →
/// `bank agreement v2`.
fn title_from_filename(path: &Path) -> String {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Document");
    stem.replace(['-', '_'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Rebuild structural blocks from extracted markdown when the index is
/// rewritten without running Xberg again (schema bump).
pub(crate) fn blocks_from_markdown(content: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    for chunk in content.split("\n\n") {
        let trimmed = chunk.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(heading) = trimmed.strip_prefix('#') {
            let level = 1 + heading.chars().take_while(|c| *c == '#').count() as u8;
            let text = heading.trim_start_matches('#').trim().to_string();
            if !text.is_empty() {
                blocks.push(Block {
                    kind: BlockKind::Heading { level },
                    text,
                    page: None,
                    page_end: None,
                });
                continue;
            }
        }
        blocks.push(Block {
            kind: BlockKind::Paragraph,
            text: trimmed.to_string(),
            page: None,
            page_end: None,
        });
    }
    blocks
}

fn tidy(text: &str, max: usize) -> String {
    let cleaned = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if cleaned.chars().count() > max {
        let cut: String = cleaned.chars().take(max).collect();
        format!("{}…", cut.trim_end())
    } else {
        cleaned
    }
}

const STRUCTURED_SUMMARY_MAX: usize = 720;
const RANK_SUMMARY_MAX: usize = 360;

fn char_len(text: &str) -> usize {
    text.chars().count()
}

fn clip_summary_piece(text: &str, max: usize) -> String {
    if char_len(text) <= max {
        return text.to_string();
    }
    let cut: String = text.chars().take(max).collect();
    if let Some(at) = cut.rfind(['.', '!', '?']) {
        if at >= max / 3 {
            return cut[..=at].trim_end().to_string();
        }
    }
    if let Some(at) = cut.rfind(char::is_whitespace) {
        if at >= max / 3 {
            return format!("{}…", cut[..at].trim_end());
        }
    }
    format!("{}…", cut.trim_end())
}

fn push_summary_piece(out: &mut String, piece: &str, max: usize) -> bool {
    let piece = piece.trim_end();
    if piece.is_empty() {
        return true;
    }
    let extra = if out.is_empty() { 0 } else { 2 };
    let next_len = char_len(out) + extra + char_len(piece);
    if !out.is_empty() && next_len > max {
        return false;
    }
    if !out.is_empty() {
        out.push_str("\n\n");
    }
    if out.is_empty() && char_len(piece) > max {
        out.push_str(&clip_summary_piece(piece, max));
        return false;
    }
    out.push_str(piece);
    char_len(out) < max
}

/// Markdown preview from headings and the paragraphs under them.
fn summary_from_blocks(blocks: &[Block], max: usize) -> Option<String> {
    let structured = blocks.iter().any(|block| {
        matches!(
            block.kind,
            BlockKind::Heading { .. } | BlockKind::SheetStart { .. } | BlockKind::SlideStart { .. }
        )
    });
    if !structured {
        return None;
    }

    let mut out = String::new();
    for block in blocks {
        let piece = match &block.kind {
            BlockKind::Heading { level } => {
                let text = block.text.trim();
                if text.is_empty() {
                    continue;
                }
                let hashes = "#".repeat((*level).clamp(1, 6) as usize);
                format!("{hashes} {text}")
            }
            BlockKind::SheetStart { name } => {
                let name = name.trim();
                if name.is_empty() {
                    continue;
                }
                format!("## {name}")
            }
            BlockKind::SlideStart { number, title } => {
                let label = title
                    .as_deref()
                    .map(str::trim)
                    .filter(|text| !text.is_empty())
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("Slide {number}"));
                format!("## {label}")
            }
            BlockKind::Paragraph => {
                let text = block.text.trim();
                if text.is_empty() {
                    continue;
                }
                text.to_string()
            }
            BlockKind::Table => continue,
        };
        if !push_summary_piece(&mut out, &piece, max) {
            break;
        }
    }

    let trimmed = out.trim().to_string();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

fn prose_for_summary(content: &str, blocks: &[Block]) -> String {
    let from_blocks = blocks
        .iter()
        .filter_map(|block| match block.kind {
            BlockKind::Paragraph | BlockKind::Table => Some(block.text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    if !from_blocks.trim().is_empty() {
        return from_blocks;
    }
    content
        .lines()
        .map(|line| line.trim_start_matches('#').trim())
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Run Xberg on one file and shape the result for the pipeline.
pub async fn extract_file(path: &Path, settings: &ExtractorSettings) -> Result<Extraction> {
    ensure_tessdata(settings);
    let config = build_config(settings);
    let input = ExtractInput::from_uri(path.to_string_lossy().as_ref());
    let output = xberg::extract(input, &config)
        .await
        .with_context(|| format!("extract {}", path.display()))?;

    let doc = output
        .results
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("no extraction result for {}", path.display()))?;

    let content = limit_extracted(doc.content.trim().to_string());
    let ocr_used =
        doc.metadata.ocr_used || doc.extraction_method.map(|m| m.used_ocr()).unwrap_or(false);

    // A scan that even OCR couldn't read is an error; a genuinely empty
    // file is not — it simply becomes Ready with nothing to index.
    if content.is_empty() && ocr_used {
        return Err(anyhow!("couldn't read any text, even with OCR"));
    }

    // Language: xberg emits ISO 639-3 tags (e.g. "spa", "fra", "eng").
    let language_tag = doc
        .detected_languages
        .as_ref()
        .and_then(|langs| langs.first())
        .cloned();
    let language = language_tag.as_deref().and_then(normalize_lang);
    let language_card = language.map(|l| l.to_string()).or(language_tag.clone());

    // Structural blocks from the document tree; pages as fallback.
    let mut blocks: Vec<Block> = Vec::new();
    let mut outline: Vec<OutlineEntry> = Vec::new();
    let mut structure_title: Option<String> = None;

    if let Some(structure) = &doc.document {
        use xberg::types::document_structure::NodeContent;
        for node in &structure.nodes {
            match &node.content {
                NodeContent::Title { text } => {
                    if structure_title.is_none() && !text.trim().is_empty() {
                        structure_title = Some(text.clone());
                    }
                    if outline.len() < 40 && !text.trim().is_empty() {
                        outline.push(OutlineEntry {
                            title: tidy(text, 90),
                            page: node.page,
                        });
                    }
                    blocks.push(Block {
                        kind: BlockKind::Heading { level: 1 },
                        text: text.clone(),
                        page: node.page,
                        page_end: node.page_end,
                    });
                }
                NodeContent::Heading { level, text } => {
                    if !text.trim().is_empty() {
                        if *level <= 3 && outline.len() < 40 {
                            outline.push(OutlineEntry {
                                title: tidy(text, 90),
                                page: node.page,
                            });
                        }
                        blocks.push(Block {
                            kind: BlockKind::Heading { level: *level },
                            text: text.clone(),
                            page: node.page,
                            page_end: node.page_end,
                        });
                    }
                }
                NodeContent::Paragraph { text }
                | NodeContent::ListItem { text }
                | NodeContent::Footnote { text } => {
                    if !text.trim().is_empty() {
                        blocks.push(Block {
                            kind: BlockKind::Paragraph,
                            text: text.clone(),
                            page: node.page,
                            page_end: node.page_end,
                        });
                    }
                }
                NodeContent::Code { text, .. } | NodeContent::Formula { text } => {
                    if !text.trim().is_empty() {
                        blocks.push(Block {
                            kind: BlockKind::Paragraph,
                            text: text.clone(),
                            page: node.page,
                            page_end: node.page_end,
                        });
                    }
                }
                NodeContent::Table { grid } => {
                    let rendered = render_table(grid);
                    if !rendered.is_empty() {
                        blocks.push(Block {
                            kind: BlockKind::Table,
                            text: rendered,
                            page: node.page,
                            page_end: node.page_end,
                        });
                    }
                }
                NodeContent::Slide { number, title } => {
                    blocks.push(Block {
                        kind: BlockKind::SlideStart {
                            number: *number,
                            title: title.clone(),
                        },
                        text: String::new(),
                        page: node.page,
                        page_end: node.page_end,
                    });
                }
                _ => {}
            }
        }
    }

    // Page-level fallback / enrichment for sheets and slides.
    let pages = doc.pages.as_deref().unwrap_or(&[]);
    let page_count = (!pages.is_empty()).then_some(pages.len() as u32);

    if blocks.iter().all(|b| b.text.trim().is_empty()) {
        blocks.clear();
        if !pages.is_empty() {
            for page in pages {
                if let Some(sheet) = &page.sheet_name {
                    blocks.push(Block {
                        kind: BlockKind::SheetStart {
                            name: sheet.clone(),
                        },
                        text: String::new(),
                        page: Some(page.page_number),
                        page_end: Some(page.page_number),
                    });
                    if outline.len() < 40 {
                        outline.push(OutlineEntry {
                            title: sheet.clone(),
                            page: Some(page.page_number),
                        });
                    }
                } else if let Some(section) = &page.section_name {
                    blocks.push(Block {
                        kind: BlockKind::SlideStart {
                            number: page.page_number,
                            title: Some(section.clone()),
                        },
                        text: String::new(),
                        page: Some(page.page_number),
                        page_end: Some(page.page_number),
                    });
                }
                for paragraph in page.content.split("\n\n") {
                    if !paragraph.trim().is_empty() {
                        blocks.push(Block {
                            kind: BlockKind::Paragraph,
                            text: paragraph.trim().to_string(),
                            page: Some(page.page_number),
                            page_end: Some(page.page_number),
                        });
                    }
                }
                if let Some(notes) = &page.speaker_notes {
                    if !notes.trim().is_empty() {
                        blocks.push(Block {
                            kind: BlockKind::Paragraph,
                            text: format!("Speaker notes: {}", notes.trim()),
                            page: Some(page.page_number),
                            page_end: Some(page.page_number),
                        });
                    }
                }
            }
        } else {
            let md = blocks_from_markdown(&content);
            for block in &md {
                if let BlockKind::Heading { level } = &block.kind {
                    if *level <= 3 && outline.len() < 40 && !block.text.is_empty() {
                        outline.push(OutlineEntry {
                            title: tidy(&block.text, 90),
                            page: None,
                        });
                    }
                }
            }
            blocks.extend(md);
        }
    }

    // Title: document metadata → top-level heading → cleaned filename.
    let title = doc
        .metadata
        .title
        .clone()
        .filter(|t| !t.trim().is_empty())
        .or(structure_title)
        .map(|t| tidy(&t, 120))
        .unwrap_or_else(|| title_from_filename(path));

    // Structured files keep headings; the rest uses TextRank on prose.
    let summary = if content.is_empty() {
        String::new()
    } else {
        summary_from_blocks(&blocks, STRUCTURED_SUMMARY_MAX)
            .filter(|text| !text.is_empty())
            .unwrap_or_else(|| {
                let prose = prose_for_summary(&content, &blocks);
                if prose.trim().is_empty() {
                    return String::new();
                }
                xberg::text::summarization::textrank::summarize(
                    &prose,
                    language.or(Some("en")),
                    Some(80),
                )
                .map(|s| tidy(&s, RANK_SUMMARY_MAX))
                .unwrap_or_default()
            })
    };

    // Keywords: Xberg YAKE with the document's own language.
    let keywords = if content.is_empty() {
        Vec::new()
    } else {
        let keyword_config = KeywordConfig {
            algorithm: KeywordAlgorithm::Yake,
            max_keywords: 8,
            language: Some(language.unwrap_or("en").to_string()),
            ..Default::default()
        };
        xberg::keywords::extract_keywords(&content, &keyword_config)
            .map(|list| {
                list.into_iter()
                    .map(|k| tidy(&k.text, 60))
                    .filter(|k| !k.is_empty())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    };

    let final_page_count =
        page_count.or_else(|| blocks.iter().filter_map(|b| b.page_end.or(b.page)).max());

    Ok(Extraction {
        content,
        title,
        language,
        language_tag: language_card,
        summary,
        keywords,
        outline,
        blocks: limit_blocks(blocks),
        page_count: final_page_count,
        ocr_used,
    })
}

fn render_table(grid: &xberg::types::document_structure::TableGrid) -> String {
    // Cells arrive in row-major order with explicit row indices.
    let max_rows = grid.rows.min(120);
    let mut rows: Vec<Vec<&str>> = vec![Vec::new(); max_rows as usize];
    for cell in &grid.cells {
        if cell.row < max_rows {
            rows[cell.row as usize].push(cell.content.as_str());
        }
    }
    let mut lines = Vec::new();
    for row in rows {
        let line = row
            .iter()
            .map(|c| c.split_whitespace().collect::<Vec<_>>().join(" "))
            .collect::<Vec<_>>()
            .join(" | ");
        if !line.trim().is_empty() {
            lines.push(line);
        }
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ocr_languages_fall_back_to_english() {
        assert_eq!(ocr_languages(None), vec!["eng".to_string()]);
    }

    #[test]
    fn bundled_tessdata_packs_exist() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("resources/tessdata");
        let langs = ocr_languages(Some(&dir));
        assert!(
            !langs.is_empty(),
            "expected at least one *.traineddata pack in {}",
            dir.display()
        );
        for language in &langs {
            let path = dir.join(format!("{language}.traineddata"));
            assert!(path.is_file(), "missing {}", path.display());
            let size = std::fs::metadata(&path).unwrap().len();
            assert!(
                size > 1_000_000,
                "{} is suspiciously small ({size} bytes)",
                path.display()
            );
        }
        assert!(
            dir.join("LICENSE").is_file(),
            "tessdata must ship the Apache-2.0 LICENSE next to the packs"
        );
    }

    #[test]
    fn tessdata_copies_into_empty_dest() {
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("resources/tessdata");
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("tessdata");
        assert!(!tessdata_has_packs(&dest));
        ensure_tessdata(&ExtractorSettings {
            tessdata_dir: Some(dest.clone()),
            tessdata_bundle: Some(src),
            timeout_secs: 30,
        });
        assert!(tessdata_has_packs(&dest));
        assert!(
            dest.join("LICENSE").is_file(),
            "Apache-2.0 LICENSE should copy with the packs"
        );
    }

    #[test]
    fn only_document_types_are_offered() {
        assert!(is_supported_file(Path::new("brief.pdf")));
        assert!(is_supported_file(Path::new("note.md")));
        assert!(is_supported_file(Path::new("sheet.xlsx")));
        assert!(!is_supported_file(Path::new("package.json")));
        assert!(!is_supported_file(Path::new("page.html")));
        assert!(!is_supported_file(Path::new("data.xml")));
        assert!(!is_supported_file(Path::new("~$lock.docx")));
        assert!(!is_supported_file(Path::new("~$Book.xlsx")));
        assert!(!is_supported_file(Path::new("scratch.tmp")));
        assert!(!is_supported_file(Path::new("notes/draft.TMP")));
        assert!(!is_supported_file(Path::new(".hidden.md")));
        assert!(!is_supported_file(Path::new("clip.mp4")));
        assert!(is_supported_file(Path::new("report.docx")));
    }

    #[test]
    fn skip_file_name_covers_locks_and_tmp() {
        assert!(skip_file_name("~$lock.docx"));
        assert!(skip_file_name("scratch.tmp"));
        assert!(skip_file_name("SCRATCH.TMP"));
        assert!(skip_file_name(".hidden.md"));
        assert!(skip_file_name("shelf.yml"));
        assert!(!skip_file_name("note.md"));
        assert!(!skip_file_name("report.docx"));
    }

    #[test]
    fn limit_extracted_keeps_short_text() {
        assert_eq!(limit_extracted("hi".into()), "hi");
    }

    #[test]
    fn limit_extracted_cuts_at_a_paragraph() {
        let chunk = "hello world paragraph\n\n";
        let input = chunk.repeat(EXTRACTED_MAX / chunk.len() + 80);
        let out = limit_extracted(input);
        assert!(out.len() <= EXTRACTED_MAX);
        assert!(out.contains("hello world paragraph"));
    }

    fn block(kind: BlockKind, text: &str) -> Block {
        Block {
            kind,
            text: text.to_string(),
            page: None,
            page_end: None,
        }
    }

    #[test]
    fn structured_summary_keeps_markdown_headings() {
        let summary = summary_from_blocks(
            &[
                block(BlockKind::Heading { level: 1 }, "Chapter one"),
                block(BlockKind::Heading { level: 2 }, "Opening"),
                block(BlockKind::Paragraph, "The clause lasts ninety days."),
                block(BlockKind::Heading { level: 2 }, "Next"),
                block(BlockKind::Paragraph, "A later section follows."),
            ],
            720,
        )
        .expect("structured summary");
        assert_eq!(
            summary,
            "# Chapter one\n\n## Opening\n\nThe clause lasts ninety days.\n\n## Next\n\nA later section follows."
        );
    }

    #[test]
    fn structured_summary_stops_at_a_block() {
        let first = "## Opening\n\nThe clause lasts ninety days. Notice is written.";
        let summary = summary_from_blocks(
            &[
                block(BlockKind::Heading { level: 2 }, "Opening"),
                block(
                    BlockKind::Paragraph,
                    "The clause lasts ninety days. Notice is written.",
                ),
                block(BlockKind::Heading { level: 2 }, "Later"),
                block(
                    BlockKind::Paragraph,
                    "This paragraph is far too long to fit in a short summary budget.",
                ),
            ],
            first.chars().count() + 4,
        )
        .expect("structured summary");
        assert_eq!(summary, first);
        assert!(!summary.contains("Later"));
    }

    #[test]
    fn prose_summary_skips_heading_lines() {
        let prose = prose_for_summary(
            "# Chapter one\n\n## Opening\n\nThe clause lasts ninety days.",
            &[
                block(BlockKind::Heading { level: 1 }, "Chapter one"),
                block(BlockKind::Heading { level: 2 }, "Opening"),
                block(BlockKind::Paragraph, "The clause lasts ninety days."),
            ],
        );
        assert_eq!(prose, "The clause lasts ninety days.");
        assert!(!prose.contains('#'));
    }
}
