//! Structure-aware passage building.
//!
//! Preference order (per spec): headings/sections → page boundaries →
//! paragraphs → slide boundaries → sheet/table structure. Every passage
//! keeps enough metadata to return to the exact source location, and very
//! large sections split at natural text boundaries.

use super::extract::{Block, BlockKind, EXTRACTED_MAX};
use crate::types::Passage;

/// Target passage size in characters; sections larger than this split at
/// paragraph boundaries.
const TARGET_CHARS: usize = 1400;
/// Don't emit crumbs below this size unless they are all we have.
const MIN_CHARS: usize = 30;
/// Safety cap per document.
const MAX_PASSAGES: usize = 600;

struct Builder {
    passages: Vec<Passage>,
    section: Option<String>,
    buffer: Vec<String>,
    buffer_chars: usize,
    page_start: Option<u32>,
    page_end: Option<u32>,
    total_bytes: usize,
}

impl Builder {
    fn new() -> Self {
        Self {
            passages: Vec::new(),
            section: None,
            buffer: Vec::new(),
            buffer_chars: 0,
            page_start: None,
            page_end: None,
            total_bytes: 0,
        }
    }

    fn flush(&mut self) {
        if self.buffer.is_empty() {
            return;
        }
        let body = self.buffer.join("\n\n");
        self.buffer.clear();
        self.buffer_chars = 0;
        let trimmed = body.trim();
        if trimmed.is_empty() {
            self.page_start = None;
            self.page_end = None;
            return;
        }
        if self.passages.len() >= MAX_PASSAGES {
            return;
        }
        if self.total_bytes >= EXTRACTED_MAX {
            return;
        }
        let body_len = trimmed.len();
        if self.total_bytes + body_len > EXTRACTED_MAX && !self.passages.is_empty() {
            return;
        }
        self.total_bytes += body_len;
        self.passages.push(Passage {
            section: self.section.clone(),
            page_start: self.page_start,
            page_end: self.page_end,
            body: trimmed.to_string(),
        });
        self.page_start = None;
        self.page_end = None;
    }

    fn push_text(&mut self, text: &str, page: Option<u32>, page_end: Option<u32>) {
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        // Page boundary: if this block starts on a later page and the buffer
        // is already reasonably sized, cut at the page boundary.
        if let (Some(current_end), Some(next)) = (self.page_end, page) {
            if next > current_end && self.buffer_chars >= TARGET_CHARS / 2 {
                self.flush();
            }
        }
        if self.page_start.is_none() {
            self.page_start = page;
        }
        if let Some(end) = page_end.or(page) {
            self.page_end = Some(self.page_end.map_or(end, |e| e.max(end)));
        }
        // Very long single blocks split at sentence-ish boundaries.
        if text.chars().count() > TARGET_CHARS {
            for piece in split_long(text, TARGET_CHARS) {
                self.buffer.push(piece.clone());
                self.buffer_chars += piece.chars().count();
                self.flush();
                self.page_start = page;
                self.page_end = page_end.or(page);
            }
            self.page_start = None;
            self.page_end = None;
            return;
        }
        self.buffer_chars += text.chars().count();
        self.buffer.push(text.to_string());
        if self.buffer_chars >= TARGET_CHARS {
            self.flush();
        }
    }

    fn set_section(&mut self, section: Option<String>) {
        self.flush();
        self.section = section;
    }
}

/// Split a long text at paragraph/sentence boundaries into ~`target` pieces.
fn split_long(text: &str, target: usize) -> Vec<String> {
    let mut pieces = Vec::new();
    let mut current = String::new();
    for sentence in split_sentences(text) {
        if !current.is_empty() && current.chars().count() + sentence.chars().count() > target {
            pieces.push(current.trim().to_string());
            current = String::new();
        }
        current.push_str(sentence);
        current.push(' ');
    }
    if !current.trim().is_empty() {
        pieces.push(current.trim().to_string());
    }
    pieces
}

fn split_sentences(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let bytes = text.as_bytes();
    for (i, b) in bytes.iter().enumerate() {
        if matches!(b, b'.' | b'!' | b'?' | b'\n') {
            let end = i + 1;
            if end - start > 1 {
                if let Some(slice) = text.get(start..end) {
                    out.push(slice);
                    start = end;
                }
            }
        }
    }
    if let Some(rest) = text.get(start..) {
        if !rest.trim().is_empty() {
            out.push(rest);
        }
    }
    out
}

/// Build passages from structural blocks.
pub fn build_passages(blocks: &[Block]) -> Vec<Passage> {
    let mut builder = Builder::new();
    for block in blocks {
        match &block.kind {
            BlockKind::Heading { .. } => {
                let title = block.text.trim();
                if title.is_empty() {
                    continue;
                }
                builder.set_section(Some(clip(title, 120)));
            }
            BlockKind::SheetStart { name } => {
                builder.set_section(Some(clip(name, 120)));
            }
            BlockKind::SlideStart { number, title } => {
                let label = match title {
                    Some(t) if !t.trim().is_empty() => format!("Slide {number} · {}", t.trim()),
                    _ => format!("Slide {number}"),
                };
                builder.set_section(Some(clip(&label, 120)));
            }
            BlockKind::Paragraph | BlockKind::Table => {
                builder.push_text(&block.text, block.page, block.page_end);
            }
        }
    }
    builder.flush();

    // Merge stray crumbs into their predecessor.
    let mut merged: Vec<Passage> = Vec::new();
    for passage in builder.passages {
        if passage.body.chars().count() < MIN_CHARS {
            if let Some(last) = merged.last_mut() {
                if last.section == passage.section {
                    last.body.push_str("\n\n");
                    last.body.push_str(&passage.body);
                    if last.page_end < passage.page_end {
                        last.page_end = passage.page_end;
                    }
                    continue;
                }
            }
        }
        merged.push(passage);
    }
    merged
}

fn clip(text: &str, max: usize) -> String {
    let cleaned = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if cleaned.chars().count() > max {
        cleaned.chars().take(max).collect()
    } else {
        cleaned
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingest::extract::{Block, BlockKind};

    fn heading(text: &str, page: u32) -> Block {
        Block {
            kind: BlockKind::Heading { level: 2 },
            text: text.into(),
            page: Some(page),
            page_end: Some(page),
        }
    }

    fn para(text: &str, page: u32) -> Block {
        Block {
            kind: BlockKind::Paragraph,
            text: text.into(),
            page: Some(page),
            page_end: Some(page),
        }
    }

    #[test]
    fn sections_become_passages_with_metadata() {
        let blocks = vec![
            heading("Object", 1),
            para("This agreement covers services.", 1),
            heading("Termination", 14),
            para("Either party may terminate with 90 days notice.", 14),
            para("Breach allows immediate termination.", 15),
        ];
        let passages = build_passages(&blocks);
        assert_eq!(passages.len(), 2);
        assert_eq!(passages[0].section.as_deref(), Some("Object"));
        assert_eq!(passages[1].section.as_deref(), Some("Termination"));
        assert_eq!(passages[1].page_start, Some(14));
        assert_eq!(passages[1].page_end, Some(15));
    }

    #[test]
    fn long_sections_split_at_boundaries() {
        let long = "A sentence about payments. ".repeat(200);
        let blocks = vec![heading("Payment", 4), para(&long, 4)];
        let passages = build_passages(&blocks);
        assert!(passages.len() > 1);
        for passage in &passages {
            assert!(passage.body.chars().count() <= 1700);
            assert_eq!(passage.section.as_deref(), Some("Payment"));
        }
    }

    #[test]
    fn sheets_label_sections() {
        let blocks = vec![
            Block {
                kind: BlockKind::SheetStart {
                    name: "July".into(),
                },
                text: String::new(),
                page: Some(1),
                page_end: Some(1),
            },
            para("employee | iban | salary", 1),
        ];
        let passages = build_passages(&blocks);
        assert_eq!(passages.len(), 1);
        assert_eq!(passages[0].section.as_deref(), Some("July"));
    }
}
