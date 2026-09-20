//! Extra search queries for Light and Deep: one short pass before retrieval.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::engine::{ChatMessage, Engine};
use crate::search::fold_ws;

pub(crate) async fn extra_search_queries(
    engine: &Arc<Engine>,
    question: &str,
    count: usize,
    cancel: &Arc<AtomicBool>,
) -> Vec<String> {
    if count == 0 || cancel.load(Ordering::Relaxed) {
        return Vec::new();
    }
    let messages = vec![
        ChatMessage::text(
            "system",
            "You write search queries that find answers in the user's files. Output only the queries.",
        ),
        ChatMessage::text("user", extra_query_prompt(question, count)),
    ];
    match engine.chat_once(&messages, 0.3, 200, cancel).await {
        Ok(output) => {
            if cancel.load(Ordering::Relaxed) {
                return Vec::new();
            }
            let mut parsed = parse_search_queries(&output.answer, question, count);
            if parsed.is_empty() && !output.thinking.is_empty() {
                parsed = parse_search_queries(&output.thinking, question, count);
            }
            parsed
        }
        Err(error) => {
            log::warn!("extra search queries failed: {error:#}");
            Vec::new()
        }
    }
}

fn extra_query_prompt(question: &str, count: usize) -> String {
    const JOBS: &[&str] = &[
        "names, dates, and amounts from the question. Keywords only.",
        "a paraphrase that uses different wording.",
        "a likely filename, heading, or section title.",
    ];
    let mut prompt = format!(
        "Write exactly {count} search queries. Same language as the question. Queries only. \
Do not repeat the question.\n"
    );
    for i in 0..count {
        let job = JOBS
            .get(i)
            .copied()
            .unwrap_or("another distinct keyword query.");
        prompt.push_str(&format!("Line {}: {job}\n", i + 1));
    }
    prompt.push_str("\nQuestion:\n");
    prompt.push_str(question);
    prompt
}

fn parse_search_queries(raw: &str, original: &str, count: usize) -> Vec<String> {
    let original_fold = fold_ws(original);
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for line in raw.lines() {
        let cleaned = clean_query_line(line);
        if cleaned.chars().count() < 3 {
            continue;
        }
        let fold = fold_ws(&cleaned);
        if fold == original_fold || !seen.insert(fold) {
            continue;
        }
        out.push(cleaned);
        if out.len() >= count {
            break;
        }
    }
    out
}

fn clean_query_line(line: &str) -> String {
    let mut text = line.trim();
    if let Some(stripped) = text.strip_prefix("```") {
        text = stripped.trim();
    }
    text = text.trim_start_matches(['-', '*', '•']).trim();
    if let Some((index, rest)) = text.split_once(['.', ')', ':']) {
        if !index.is_empty() && index.chars().all(|c| c.is_ascii_digit()) {
            text = rest.trim();
        }
    }
    text.trim_matches(|c| c == '"' || c == '\'' || c == '`')
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_strips_numbering_and_skips_the_original() {
        let raw = "1. bad leaver permanence period\n\
- termination of executive director\n\
3. Which penalties do I have if I leave the company as CEO in the first years?\n\
non-compete after leaving\n";
        let original =
            "Which penalties do I have if I leave the company as CEO in the first years?";
        let got = parse_search_queries(raw, original, 3);
        assert_eq!(
            got,
            vec![
                "bad leaver permanence period",
                "termination of executive director",
                "non-compete after leaving",
            ]
        );
    }

    #[test]
    fn parse_dedupes_and_caps() {
        let raw = "zebra east wing\nZebra East Wing\nkitchen restock\nmore\n";
        let got = parse_search_queries(raw, "When is the kitchen restocked?", 2);
        assert_eq!(got, vec!["zebra east wing", "kitchen restock"]);
    }

    #[test]
    fn three_query_prompt_asks_for_distinct_jobs() {
        let prompt = extra_query_prompt("When can they terminate?", 3);
        assert!(prompt.contains("Line 1:"));
        assert!(prompt.contains("Line 2:"));
        assert!(prompt.contains("Line 3:"));
        assert!(prompt.contains("filename"));
        let two = extra_query_prompt("When can they terminate?", 2);
        assert!(two.contains("Line 2:"));
        assert!(!two.contains("Line 3:"));
    }
}
