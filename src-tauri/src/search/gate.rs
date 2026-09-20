//! The Retrieval Gate.
//!
//! Tantivy always returns *something*; the gate decides what is actually
//! relevant enough to hand to the model. Chat then fits those passages
//! into the machine's measured prompt-processing budget. Deterministic
//! and explainable:
//!
//! 1. relative score cut against the best hit,
//! 2. lexical-overlap check (at least one meaningful query token must
//!    literally appear in the passage — fuzzy-only matches don't clear),
//! 3. per-file cap so one document cannot fill every slot, then a hard cap.

use std::collections::{HashMap, HashSet};

use super::compact_hyphens;
use crate::types::{MemorySnippet, SourcePassage};

/// Caps the gate applies after ranking. Light and Deep change these.
#[derive(Clone, Copy)]
pub struct GateCaps {
    pub max_passages: usize,
    pub max_per_doc: usize,
    pub max_per_named: usize,
    pub relative_floor: f32,
}

impl Default for GateCaps {
    fn default() -> Self {
        Self {
            max_passages: tuning::MAX_DOC_PASSAGES,
            max_per_doc: tuning::MAX_PASSAGES_PER_DOC,
            max_per_named: tuning::MAX_PASSAGES_PER_NAMED_DOC,
            relative_floor: tuning::DOC_RELATIVE_FLOOR,
        }
    }
}

/// Gate constants — tuned against the retrieval eval suite.
pub mod tuning {
    /// Passages scoring below this fraction of the top hit are dropped.
    pub const DOC_RELATIVE_FLOOR: f32 = 0.30;
    /// Memory snippets are held to a stricter cut.
    pub const MSG_RELATIVE_FLOOR: f32 = 0.50;
    /// Hard cap on document passages sent to the model.
    pub const MAX_DOC_PASSAGES: usize = 8;
    /// Default cap per file, so one parties page cannot crowd the prompt.
    pub const MAX_PASSAGES_PER_DOC: usize = 2;
    /// When the question names a file, keep more of that file.
    pub const MAX_PASSAGES_PER_NAMED_DOC: usize = 6;
    /// Hard cap on older-conversation snippets.
    pub const MAX_MEMORY_SNIPPETS: usize = 3;
    /// Fraction of the local-context budget reserved for memory snippets.
    pub const MEMORY_BUDGET_SHARE: f32 = 0.18;
    /// Query tokens shorter than this don't count for the overlap check.
    pub const OVERLAP_MIN_TOKEN_LEN: usize = 3;
    /// Default local-context budget when no benchmark has run yet (chars).
    pub const DEFAULT_BUDGET_CHARS: usize = 9_000;
}

fn folded(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .map(|c| match c {
            'á' | 'à' | 'ä' | 'â' => 'a',
            'é' | 'è' | 'ë' | 'ê' => 'e',
            'í' | 'ì' | 'ï' | 'î' => 'i',
            'ó' | 'ò' | 'ö' | 'ô' => 'o',
            'ú' | 'ù' | 'ü' | 'û' => 'u',
            'ç' => 'c',
            'ñ' => 'n',
            _ => c,
        })
        .collect()
}

/// True when at least one meaningful query token appears literally in the
/// passage text (title, section, body, or path), accent- and case-insensitively.
fn overlaps(query_tokens: &[String], haystacks: &[&str]) -> bool {
    let meaningful: Vec<&String> = query_tokens
        .iter()
        .filter(|t| t.chars().count() >= tuning::OVERLAP_MIN_TOKEN_LEN)
        .collect();
    if meaningful.is_empty() {
        // Nothing meaningful to check against — let the score cut decide.
        return true;
    }
    let folded_haystacks: Vec<String> = haystacks.iter().map(|h| folded(h)).collect();
    let compact_haystacks: Vec<String> = folded_haystacks
        .iter()
        .map(|h| compact_hyphens(h))
        .collect();
    meaningful.iter().any(|t| {
        let compact_t = compact_hyphens(t);
        folded_haystacks.iter().any(|h| h.contains(t.as_str()))
            || compact_haystacks
                .iter()
                .any(|h| h.contains(t.as_str()) || h.contains(compact_t.as_str()))
    })
}

/// Rank, overlap-check, and cap passage hits. Empty `query_tokens` skips the
/// overlap check (attachment / "summarize this") but still applies per-file
/// caps. `preserve_order` keeps RRF ranking.
pub fn gate_passages(
    mut hits: Vec<SourcePassage>,
    query_tokens: &[String],
    named_document_ids: &[String],
    caps: GateCaps,
    preserve_order: bool,
) -> Vec<SourcePassage> {
    if !preserve_order {
        hits.sort_by(|a, b| b.score.total_cmp(&a.score));
    }
    let Some(top) = hits.iter().map(|h| h.score).max_by(|a, b| a.total_cmp(b)) else {
        return Vec::new();
    };
    if top <= 0.0 {
        return Vec::new();
    }
    let named: HashSet<&str> = named_document_ids.iter().map(String::as_str).collect();
    let mut per_doc: HashMap<String, usize> = HashMap::new();
    let mut kept: Vec<SourcePassage> = Vec::new();
    for hit in hits {
        if caps.relative_floor > 0.0 && hit.score < top * caps.relative_floor {
            continue;
        }
        let haystacks = [
            hit.title.as_str(),
            hit.section.as_deref().unwrap_or(""),
            hit.body.as_str(),
            hit.path.as_str(),
        ];
        if !overlaps(query_tokens, &haystacks) {
            continue;
        }
        let cap = if named.contains(hit.document_id.as_str()) {
            caps.max_per_named
        } else {
            caps.max_per_doc
        };
        let count = per_doc.entry(hit.document_id.clone()).or_insert(0);
        if *count >= cap {
            continue;
        }
        *count += 1;
        kept.push(hit);
        if kept.len() >= caps.max_passages {
            break;
        }
    }
    assign_sids(kept)
}

fn assign_sids(mut kept: Vec<SourcePassage>) -> Vec<SourcePassage> {
    for (i, passage) in kept.iter_mut().enumerate() {
        passage.sid = format!("S{}", i + 1);
    }
    kept
}

/// Apply the gate to older-conversation hits.
pub fn gate_messages(mut hits: Vec<MemorySnippet>, query_tokens: &[String]) -> Vec<MemorySnippet> {
    hits.sort_by(|a, b| b.score.total_cmp(&a.score));
    let Some(top) = hits.first().map(|h| h.score) else {
        return Vec::new();
    };
    if top <= 0.0 {
        return Vec::new();
    }
    let mut kept = Vec::new();
    for hit in hits {
        if hit.score < top * tuning::MSG_RELATIVE_FLOOR {
            break;
        }
        if !overlaps(query_tokens, &[hit.body.as_str()]) {
            continue;
        }
        kept.push(hit);
        if kept.len() >= tuning::MAX_MEMORY_SNIPPETS {
            break;
        }
    }
    kept
}

pub(crate) fn passage_cost(passage: &SourcePassage) -> usize {
    passage
        .body
        .chars()
        .count()
        .saturating_add(passage.title.chars().count())
        .saturating_add(64)
}

/// Keep passages that fit `budget_chars`, truncating the first if it alone is too long.
/// Does not renumber source ids.
pub fn take_passages(passages: Vec<SourcePassage>, budget_chars: usize) -> Vec<SourcePassage> {
    let mut kept = Vec::new();
    let mut used = 0usize;
    for mut passage in passages {
        let cost = passage_cost(&passage);
        if used + cost > budget_chars {
            if kept.is_empty() && budget_chars > 512 {
                let take = budget_chars.saturating_sub(passage.title.chars().count() + 64);
                passage.body = truncate_at_boundary(&passage.body, take);
                kept.push(passage);
            }
            break;
        }
        used += cost;
        kept.push(passage);
    }
    kept
}

/// Fit older-conversation snippets into a char budget.
pub fn fit_memory(memory: Vec<MemorySnippet>, budget_chars: usize) -> Vec<MemorySnippet> {
    let mut kept = Vec::new();
    let mut used = 0usize;
    for mut snippet in memory {
        let cost = snippet.body.chars().count() + 48;
        if used + cost > budget_chars {
            if kept.is_empty() && budget_chars > 256 {
                snippet.body = truncate_at_boundary(&snippet.body, budget_chars.saturating_sub(48));
                kept.push(snippet);
            }
            break;
        }
        used += cost;
        kept.push(snippet);
    }
    kept
}

/// Cut at a sentence or word boundary, never mid-word.
pub fn truncate_at_boundary(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let hard: String = text.chars().take(max_chars).collect();
    for boundary in ['\n', '.', ';'] {
        if let Some(pos) = hard.rfind(boundary) {
            if pos > max_chars / 2 {
                return format!("{}…", &hard[..=pos]);
            }
        }
    }
    match hard.rfind(' ') {
        Some(pos) if pos > max_chars / 2 => format!("{}…", &hard[..pos]),
        _ => format!("{hard}…"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn passage(score: f32, body: &str) -> SourcePassage {
        SourcePassage {
            anchor: None,
            sid: String::new(),
            document_id: "d_x".into(),
            shelf_id: "s_x".into(),
            title: "Doc".into(),
            section: None,
            page_start: None,
            page_end: None,
            body: body.into(),
            path: "/tmp/doc.pdf".into(),
            score,
        }
    }

    fn gated(hits: Vec<SourcePassage>, token: &str) -> Vec<SourcePassage> {
        gate_passages(hits, &[token.into()], &[], GateCaps::default(), false)
    }

    #[test]
    fn empty_hits_stay_empty() {
        let out = gated(Vec::new(), "termination");
        assert!(out.is_empty());
    }

    #[test]
    fn relative_floor_cuts_weak_tail() {
        let hits = vec![
            passage(10.0, "termination clause applies"),
            passage(6.0, "termination notice period"),
            passage(1.0, "unrelated catering invoice"),
        ];
        let out = gated(hits, "termination");
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].sid, "S1");
        assert_eq!(out[1].sid, "S2");
    }

    #[test]
    fn overlap_check_drops_fuzzy_only_matches() {
        let hits = vec![
            passage(5.0, "termination clause applies"),
            // scored close but shares no literal token with the query
            passage(4.0, "wholly unrelated text"),
        ];
        let out = gated(hits, "termination");
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn empty_tokens_skip_overlap_and_keep_the_per_file_cap() {
        let hits = vec![
            passage(5.0, "termination clause applies"),
            passage(4.0, "wholly unrelated text"),
        ];
        let out = gate_passages(hits, &[], &[], GateCaps::default(), false);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].sid, "S1");
    }

    #[test]
    fn overlap_is_accent_insensitive() {
        let hits = vec![passage(5.0, "La rescisión del contrato se regula aquí")];
        let out = gated(hits, "rescision");
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn overlap_treats_hyphens_as_optional() {
        let hits = vec![passage(5.0, "The non-compete clause applies here")];
        let out = gated(hits, "noncompete");
        assert_eq!(out.len(), 1);
    }

    fn passage_on(doc: &str, score: f32, body: &str) -> SourcePassage {
        let mut hit = passage(score, body);
        hit.document_id = doc.into();
        hit
    }

    #[test]
    fn per_file_cap_leaves_room_for_other_documents() {
        let mut hits = Vec::new();
        for i in 0..6 {
            hits.push(passage_on(
                "parties",
                10.0 - i as f32 * 0.1,
                "the company and the company parties clause",
            ));
        }
        hits.push(passage_on(
            "services",
            7.0,
            "the company treats a bad leaver departure before the permanence period",
        ));
        let out = gated(hits, "company");
        let from_parties = out.iter().filter(|p| p.document_id == "parties").count();
        let from_services = out.iter().filter(|p| p.document_id == "services").count();
        assert_eq!(from_parties, tuning::MAX_PASSAGES_PER_DOC);
        assert_eq!(from_services, 1);
    }

    #[test]
    fn named_file_may_keep_more_passages() {
        let mut hits = Vec::new();
        for i in 0..6 {
            hits.push(passage_on(
                "named",
                10.0 - i as f32 * 0.1,
                "services agreement termination clause",
            ));
        }
        hits.push(passage_on("other", 6.0, "services agreement parties"));
        let out = gate_passages(
            hits,
            &["services".into()],
            &["named".into()],
            GateCaps::default(),
            false,
        );
        let from_named = out.iter().filter(|p| p.document_id == "named").count();
        let from_other = out.iter().filter(|p| p.document_id == "other").count();
        assert_eq!(from_named, 6);
        assert_eq!(from_other, 1);
        assert_eq!(out.len(), 7);
    }

    #[test]
    fn deep_caps_keep_more_of_one_file() {
        let mut hits = Vec::new();
        for i in 0..8 {
            hits.push(passage_on(
                "one",
                10.0 - i as f32 * 0.1,
                "termination clause applies here",
            ));
        }
        hits.push(passage_on("two", 8.0, "termination notice"));
        let caps = GateCaps {
            max_passages: 12,
            max_per_doc: 4,
            max_per_named: 8,
            relative_floor: 0.12,
        };
        let out = gate_passages(hits, &["termination".into()], &[], caps, false);
        assert_eq!(out.iter().filter(|p| p.document_id == "one").count(), 4);
        assert_eq!(out.iter().filter(|p| p.document_id == "two").count(), 1);
    }

    #[test]
    fn budget_fitting_truncates_first_long_passage() {
        let long_body = "word ".repeat(4000);
        let hits = vec![passage(5.0, &long_body)];
        let docs = take_passages(hits, 2000);
        assert_eq!(docs.len(), 1);
        assert!(docs[0].body.chars().count() < 2000);
    }

    #[test]
    fn truncate_prefers_sentence_boundary() {
        let text = "First sentence. Second sentence that is much longer and rambles on.";
        let cut = truncate_at_boundary(text, 40);
        assert!(cut.starts_with("First sentence."));
        assert!(cut.ends_with('…'));
    }
}
