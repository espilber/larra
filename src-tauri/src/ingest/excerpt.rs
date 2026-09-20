//! A window of extracted text for the source panel and document drawer.
//!
//! Conversation logs keep citation ids, not passage bodies. Opening S1 must
//! not load or render a whole long file.

use serde::{Deserialize, Serialize};

/// Enough to read the cited spot. Longer text is paged with Earlier / Later.
pub const WINDOW_CHARS: usize = 8_000;

/// Labels used by `open_shelf_file` windows, not real headings.
pub(crate) const OPEN_WINDOW_START: &str = "from the start";
pub(crate) const OPEN_WINDOW_NEXT: &str = "continued";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DocumentExcerpt {
    #[serde(default)]
    pub version_changed: bool,
    pub text: String,
    pub start_char: u32,
    pub end_char: u32,
    pub total_chars: u32,
    pub window_chars: u32,
}

#[derive(Debug, Clone, Default)]
pub struct LocateHint {
    pub page: Option<u32>,
    pub pages: Option<u32>,
    pub section: Option<String>,
    pub around: Option<String>,
}

/// Slice `text` at `start_char`, or locate from `hint` when the start is unknown.
pub fn from_text(text: &str, start_char: Option<u32>, hint: &LocateHint) -> DocumentExcerpt {
    let start = match start_char {
        Some(at) => at as usize,
        None => locate(text, hint),
    };
    window(text, start, WINDOW_CHARS)
}

fn window(text: &str, start_char: usize, max_chars: usize) -> DocumentExcerpt {
    let total = text.chars().count();
    let start = start_char.min(total);
    let rest = slice_from_char(text, start);
    let taken = take_chars_snapped(rest, max_chars);
    let end = start.saturating_add(taken.chars().count());
    DocumentExcerpt {
        version_changed: false,
        text: taken,
        start_char: start as u32,
        end_char: end as u32,
        total_chars: total as u32,
        window_chars: max_chars as u32,
    }
}

fn locate(text: &str, hint: &LocateHint) -> usize {
    if let Some(around) = hint.around.as_deref() {
        if let Some(at) = find_needle(text, around) {
            return at;
        }
    }
    if let Some(section) = hint.section.as_deref() {
        if let Some(at) = find_section(text, section) {
            return at;
        }
    }
    if let (Some(page), Some(pages)) = (hint.page, hint.pages) {
        let total = text.chars().count();
        return snap_paragraph_start(text, page_offset(total, page, pages));
    }
    0
}

fn skip_section(section: &str) -> bool {
    matches!(section, OPEN_WINDOW_START | OPEN_WINDOW_NEXT)
}

fn find_needle(text: &str, needle: &str) -> Option<usize> {
    let needle = needle
        .trim()
        .strip_suffix('…')
        .unwrap_or(needle.trim())
        .trim_end();
    if needle.chars().count() < 12 {
        return None;
    }
    if let Some(byte) = text.find(needle) {
        return Some(text[..byte].chars().count());
    }
    let short: String = needle.chars().take(80).collect();
    if short.chars().count() >= 12 {
        if let Some(byte) = text.find(&short) {
            return Some(text[..byte].chars().count());
        }
    }
    None
}

fn find_section(text: &str, section: &str) -> Option<usize> {
    let section = section.trim();
    if section.is_empty() || skip_section(section) {
        return None;
    }
    let mut byte = 0usize;
    for line in text.split_inclusive('\n') {
        let heading = line.trim().trim_start_matches('#').trim();
        if heading.eq_ignore_ascii_case(section) {
            return Some(text[..byte].chars().count());
        }
        byte += line.len();
    }
    None
}

fn page_offset(total_chars: usize, page: u32, pages: u32) -> usize {
    if total_chars == 0 || pages == 0 || page == 0 {
        return 0;
    }
    let page = page.min(pages);
    let ratio = f64::from(page.saturating_sub(1)) / f64::from(pages);
    ((ratio * total_chars as f64).round() as usize).min(total_chars)
}

fn snap_paragraph_start(text: &str, char_off: usize) -> usize {
    if char_off == 0 {
        return 0;
    }
    let byte = char_to_byte(text, char_off);
    let back_char = char_off.saturating_sub(240);
    let back_byte = char_to_byte(text, back_char);
    let slice = &text[back_byte..byte];
    if let Some(rel) = slice.rfind("\n\n") {
        return back_char + slice[..=rel].chars().count();
    }
    if let Some(rel) = slice.rfind('\n') {
        return back_char + slice[..=rel].chars().count();
    }
    char_off
}

fn take_chars_snapped(text: &str, max_chars: usize) -> String {
    let count = text.chars().count();
    if count <= max_chars {
        return text.to_string();
    }
    let hard: String = text.chars().take(max_chars).collect();
    if let Some(pos) = hard.rfind("\n\n") {
        if pos > max_chars / 2 {
            return hard[..=pos].to_string();
        }
    }
    if let Some(pos) = hard.rfind('\n') {
        if pos > max_chars / 2 {
            return hard[..=pos].to_string();
        }
    }
    if let Some(pos) = hard.rfind(' ') {
        if pos > max_chars / 2 {
            return hard[..pos].to_string();
        }
    }
    hard
}

fn slice_from_char(text: &str, start: usize) -> &str {
    match text.char_indices().nth(start) {
        Some((i, _)) => &text[i..],
        None => "",
    }
}

fn char_to_byte(text: &str, char_off: usize) -> usize {
    text.char_indices()
        .nth(char_off)
        .map(|(i, _)| i)
        .unwrap_or(text.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pages(n: u32) -> String {
        (1..=n)
            .map(|p| format!("PAGE {p}\n{}\n\n", "x".repeat(400)))
            .collect()
    }

    #[test]
    fn short_file_is_returned_whole() {
        let excerpt = from_text("hello there", None, &LocateHint::default());
        assert_eq!(excerpt.text, "hello there");
        assert_eq!(excerpt.start_char, 0);
        assert_eq!(excerpt.end_char, 11);
        assert_eq!(excerpt.total_chars, 11);
    }

    #[test]
    fn long_file_is_capped() {
        let text: String = "á".repeat(WINDOW_CHARS + 4_000);
        let excerpt = from_text(&text, None, &LocateHint::default());
        assert!(excerpt.text.chars().count() <= WINDOW_CHARS);
        assert_eq!(excerpt.start_char, 0);
        assert!(excerpt.end_char < excerpt.total_chars);
        assert_eq!(excerpt.total_chars, (WINDOW_CHARS + 4_000) as u32);
    }

    #[test]
    fn later_window_does_not_rewind() {
        let text: String = "a".repeat(WINDOW_CHARS * 2);
        let first = from_text(&text, None, &LocateHint::default());
        let next = from_text(&text, Some(first.end_char), &LocateHint::default());
        assert_eq!(next.start_char, first.end_char);
        assert!(!next.text.is_empty());
        assert!(next.start_char > 0);
    }

    #[test]
    fn around_needle_lands_on_the_cited_spot() {
        let text = "aaaa prefix\nThe indemnity clause lasts ninety days.\nzzzz suffix";
        let excerpt = from_text(
            text,
            None,
            &LocateHint {
                around: Some("The indemnity clause lasts ninety days.".into()),
                ..LocateHint::default()
            },
        );
        assert!(excerpt.text.starts_with("The indemnity clause"));
        assert!(excerpt.start_char > 0);
    }

    #[test]
    fn section_heading_is_found() {
        let text = "Intro\n\n## Hours\n\nWeekdays 10:00–19:00.\n";
        let at = locate(
            text,
            &LocateHint {
                section: Some("Hours".into()),
                ..LocateHint::default()
            },
        );
        assert_eq!(&text[char_to_byte(text, at)..][..8], "## Hours");
    }

    #[test]
    fn open_window_section_is_ignored() {
        let text = "from the start\n\nreal body";
        let at = locate(
            text,
            &LocateHint {
                section: Some(OPEN_WINDOW_START.into()),
                ..LocateHint::default()
            },
        );
        assert_eq!(at, 0);
    }

    #[test]
    fn page_hint_moves_into_the_file() {
        let text = pages(10);
        let at = locate(
            &text,
            &LocateHint {
                page: Some(8),
                pages: Some(10),
                ..LocateHint::default()
            },
        );
        assert!(at > 0);
        let slice = slice_from_char(&text, at.saturating_sub(80));
        assert!(
            slice.contains("PAGE 7") || slice.contains("PAGE 8") || slice.contains("PAGE 9"),
            "landed far from page 8: {}…",
            slice.chars().take(40).collect::<String>()
        );
    }

    #[test]
    fn paging_a_huge_file_stays_small() {
        let text: String = "word ".repeat(80_000);
        let first = from_text(&text, None, &LocateHint::default());
        assert!(first.text.chars().count() <= WINDOW_CHARS);
        let mid = from_text(
            &text,
            None,
            &LocateHint {
                page: Some(500),
                pages: Some(1000),
                ..LocateHint::default()
            },
        );
        assert!(mid.start_char > 10_000);
        assert!(mid.text.chars().count() <= WINDOW_CHARS);
    }
}
