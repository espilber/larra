//! Surround a search hit with the text before and after it, then drop
//! windows that now cover the same stretch of a file.

use std::collections::HashMap;

use crate::core::Ctx;
use crate::types::SourcePassage;

pub(crate) const OFF_RADIUS_CHARS: usize = 400;
pub(crate) const LIGHT_RADIUS_CHARS: usize = 700;
pub(crate) const DEEP_RADIUS_CHARS: usize = 1_000;
pub(crate) const LOOK_AROUND_RADIUS_CHARS: usize = 1_600;
const SNAP_CHARS: usize = 80;

pub(crate) fn widen_hit_body(extracted: &str, body: &str, radius: usize) -> String {
    let body = body.trim();
    if body.is_empty() || extracted.is_empty() {
        return body.to_string();
    }
    let Some(pos) = extracted.find(body) else {
        return body.to_string();
    };
    let body_end = pos + body.len();
    let start = snap_start(extracted, pos.saturating_sub(radius), pos);
    let end = snap_end(
        extracted,
        body_end.saturating_add(radius).min(extracted.len()),
    );
    extracted[start..end].trim().to_string()
}

fn floor_char(s: &str, mut i: usize) -> usize {
    if i > s.len() {
        i = s.len();
    }
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

fn snap_start(s: &str, approx: usize, hit: usize) -> usize {
    let i = floor_char(s, approx.min(hit));
    if i == 0 {
        return 0;
    }
    let limit = (i + SNAP_CHARS).min(hit);
    let slice = &s[i..limit];
    if let Some(nl) = slice.find('\n') {
        floor_char(s, i + nl + 1)
    } else {
        i
    }
}

fn snap_end(s: &str, approx: usize) -> usize {
    let i = floor_char(s, approx.min(s.len()));
    if i >= s.len() {
        return s.len();
    }
    let limit = (i + SNAP_CHARS).min(s.len());
    let slice = &s[i..limit];
    if let Some(nl) = slice.find('\n') {
        floor_char(s, i + nl)
    } else {
        i
    }
}

pub(crate) fn widen_neighbor_passages(
    ctx: &Ctx,
    shelf_id: &str,
    mut passages: Vec<SourcePassage>,
    radius: usize,
) -> Vec<SourcePassage> {
    let mut extracted: HashMap<String, String> = HashMap::new();
    for passage in &mut passages {
        if passage.body.is_empty() {
            continue;
        }
        let text = extracted
            .entry(passage.document_id.clone())
            .or_insert_with(|| {
                std::fs::read_to_string(ctx.paths.extracted_path(shelf_id, &passage.document_id))
                    .unwrap_or_default()
            });
        if text.is_empty() {
            continue;
        }
        let wider = widen_hit_body(text, &passage.body, radius);
        if wider.len() > passage.body.len() {
            passage.body = wider;
        }
    }
    collapse_overlapping_passages(passages)
}

/// True when two excerpts share enough text that one window already covers
/// the other. Empty bodies do not match.
pub(crate) fn bodies_overlap(a: &str, b: &str) -> bool {
    let a = a.trim();
    let b = b.trim();
    if a.is_empty() || b.is_empty() {
        return false;
    }
    if a.contains(b) || b.contains(a) {
        return true;
    }
    substantial_shared_run(a, b)
}

fn substantial_shared_run(a: &str, b: &str) -> bool {
    let (shorter, longer) = if a.chars().count() <= b.chars().count() {
        (a, b)
    } else {
        (b, a)
    };
    let n = shorter.chars().count();
    if n < 160 {
        return false;
    }
    let needle_len = (n / 2).max(80);
    let start = (n - needle_len) / 2;
    let needle: String = shorter.chars().skip(start).take(needle_len).collect();
    longer.contains(&needle)
}

/// Keep one passage when two hits from the same file overlap. Prefers the
/// longer excerpt, then the higher score. First-seen order is otherwise kept.
pub(crate) fn collapse_overlapping_passages(passages: Vec<SourcePassage>) -> Vec<SourcePassage> {
    let mut kept: Vec<SourcePassage> = Vec::new();
    for passage in passages {
        if let Some(i) = kept.iter().position(|existing| {
            existing.document_id == passage.document_id
                && bodies_overlap(&existing.body, &passage.body)
        }) {
            if prefer_passage(&passage, &kept[i]) {
                kept[i] = passage;
            }
        } else {
            kept.push(passage);
        }
    }
    kept
}

fn prefer_passage(new: &SourcePassage, old: &SourcePassage) -> bool {
    let new_len = new.body.chars().count();
    let old_len = old.body.chars().count();
    if new_len != old_len {
        return new_len > old_len;
    }
    new.score > old.score
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn widen_includes_text_before_and_after_the_hit() {
        let extracted =
            "alpha paragraph.\n\nThe kitchen is restocked on Tuesday.\n\nomega paragraph.";
        let body = "The kitchen is restocked on Tuesday.";
        let out = widen_hit_body(extracted, body, 80);
        assert!(out.contains("alpha"));
        assert!(out.contains("Tuesday"));
        assert!(out.contains("omega"));
    }

    #[test]
    fn widen_returns_the_body_when_it_is_not_in_the_file() {
        let out = widen_hit_body("unrelated file", "missing passage", 80);
        assert_eq!(out, "missing passage");
    }

    fn passage(doc: &str, body: &str, score: f32) -> SourcePassage {
        SourcePassage {
            anchor: None,
            sid: String::new(),
            document_id: doc.into(),
            shelf_id: "s".into(),
            title: "Doc".into(),
            section: None,
            page_start: Some(87),
            page_end: Some(87),
            body: body.into(),
            path: "/tmp/doc.pdf".into(),
            score,
        }
    }

    #[test]
    fn overlap_is_containment_or_a_long_shared_run() {
        assert!(bodies_overlap("alpha beta gamma", "beta"));
        assert!(!bodies_overlap("", "beta"));
        let left = "word ".repeat(40);
        let right = format!("{} extra", &left[20..]);
        assert!(bodies_overlap(&left, &right));
        assert!(!bodies_overlap(
            "Fees are listed in this column of the table. ",
            "Dates for the written papers appear below. "
        ));
    }

    #[test]
    fn collapse_keeps_the_longer_overlapping_window() {
        let short = "The speaking test uses two candidates. ".repeat(8);
        let long = format!("Before that, {short} After that, a discussion.");
        let out = collapse_overlapping_passages(vec![
            passage("handbook", &short, 9.0),
            passage("handbook", &long, 4.0),
        ]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].body, long);
        assert_eq!(out[0].score, 4.0);
    }

    #[test]
    fn collapse_keeps_distinct_excerpts_from_the_same_page() {
        let fees = "Paper 1 fees are published each year in the centre calendar. ".repeat(6);
        let dates = "Closing dates for the written papers are printed on page two. ".repeat(6);
        let out = collapse_overlapping_passages(vec![
            passage("handbook", &fees, 5.0),
            passage("handbook", &dates, 4.0),
        ]);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn collapse_does_not_merge_different_files() {
        let body = "The speaking test uses two candidates. ".repeat(8);
        let out = collapse_overlapping_passages(vec![
            passage("one", &body, 5.0),
            passage("two", &body, 4.0),
        ]);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn widen_then_collapse_merges_nearby_hits() {
        let extracted = format!(
            "Lead-in. {} {} Tail.",
            "Fees for each paper are listed here. ".repeat(10),
            "Speaking uses two candidates in one room. ".repeat(10)
        );
        let first = "Fees for each paper are listed here. ".repeat(4);
        let second = "Speaking uses two candidates in one room. ".repeat(4);
        let widened = vec![
            passage("handbook", &widen_hit_body(&extracted, &first, 400), 6.0),
            passage("handbook", &widen_hit_body(&extracted, &second, 400), 5.0),
        ];
        assert!(bodies_overlap(&widened[0].body, &widened[1].body));
        let out = collapse_overlapping_passages(widened);
        assert_eq!(out.len(), 1);
    }
}
