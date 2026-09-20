//! Ranking queries over the passage and conversation-memory indexes.

use anyhow::Result;
use std::collections::{HashMap, HashSet};
use tantivy::collector::{Count, TopDocs};
use tantivy::query::{
    BooleanQuery, BoostQuery, FuzzyTermQuery, Occur, PhraseQuery, Query, TermQuery,
};
use tantivy::schema::{Field, IndexRecordOption, Value};
use tantivy::tokenizer::TokenStream;
use tantivy::{TantivyDocument, Term};

use super::schema::{weights, STEM_LANGS};
use super::{compact_hyphens, searchable_filename, SearchIndex};
use crate::types::{MemorySnippet, SourcePassage};

/// Closed-class words dropped from search queries. Folded (unaccented) forms
/// only — the exact analyzer already ascii-folds. Keep `any` out: Catalan year.
fn is_search_stopword(token: &str) -> bool {
    matches!(
        token,
        "a" | "an"
            | "the"
            | "and"
            | "or"
            | "of"
            | "to"
            | "is"
            | "are"
            | "was"
            | "were"
            | "be"
            | "been"
            | "being"
            | "do"
            | "does"
            | "did"
            | "have"
            | "has"
            | "had"
            | "which"
            | "what"
            | "who"
            | "whom"
            | "this"
            | "that"
            | "these"
            | "those"
            | "if"
            | "as"
            | "in"
            | "on"
            | "at"
            | "for"
            | "with"
            | "from"
            | "by"
            | "it"
            | "its"
            | "i"
            | "me"
            | "my"
            | "we"
            | "our"
            | "you"
            | "your"
            | "when"
            | "can"
            | "how"
            | "el"
            | "la"
            | "los"
            | "las"
            | "del"
            | "un"
            | "una"
            | "unos"
            | "unas"
            | "y"
            | "o"
            | "que"
            | "es"
            | "son"
            | "en"
            | "por"
            | "para"
            | "con"
            | "se"
            | "su"
            | "sus"
            | "al"
            | "lo"
            | "le"
            | "els"
            | "les"
            | "amb"
            | "per"
    )
}

fn is_filename_noise(token: &str) -> bool {
    matches!(
        token,
        "pdf" | "docx" | "doc" | "md" | "txt" | "xlsx" | "pptx" | "odt" | "rtf"
    )
}

/// "non-compete" → "noncompete" so a typed hyphen still hits unhyphenated text.
fn hyphen_join_variants(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for word in text.split_whitespace() {
        if !word
            .chars()
            .any(|c| matches!(c, '-' | '_' | '\u{2010}' | '\u{2013}'))
        {
            continue;
        }
        let joined = compact_hyphens(word);
        if joined.chars().count() < 5 {
            continue;
        }
        if seen.insert(joined.clone()) {
            out.push(joined);
        }
    }
    out
}

fn hit_key(hit: &SourcePassage) -> String {
    let prefix: String = hit.body.chars().take(48).collect();
    format!(
        "{}:{}:{prefix}",
        hit.document_id,
        hit.page_start.unwrap_or(0)
    )
}

fn merge_hits(base: &mut Vec<SourcePassage>, extra: Vec<SourcePassage>) {
    let mut seen: HashSet<String> = base.iter().map(hit_key).collect();
    for hit in extra {
        if seen.insert(hit_key(&hit)) {
            base.push(hit);
        }
    }
}

/// Reciprocal rank fusion. Rank by how often a passage appears across
/// query lists; keep the strongest lexical score for the gate.
pub(crate) fn fuse_passage_lists(lists: Vec<Vec<SourcePassage>>) -> Vec<SourcePassage> {
    const K: f32 = 60.0;
    if lists.len() <= 1 {
        return lists.into_iter().next().unwrap_or_default();
    }
    let mut by_key: HashMap<String, (f32, SourcePassage)> = HashMap::new();
    for list in lists {
        for (rank, hit) in list.into_iter().enumerate() {
            let add = 1.0 / (K + rank as f32 + 1.0);
            let key = hit_key(&hit);
            match by_key.get_mut(&key) {
                Some((rrf, existing)) => {
                    *rrf += add;
                    if hit.score > existing.score {
                        *existing = hit;
                    }
                }
                None => {
                    by_key.insert(key, (add, hit));
                }
            }
        }
    }
    let mut fused: Vec<(f32, SourcePassage)> = by_key.into_values().collect();
    fused.sort_by(|a, b| b.0.total_cmp(&a.0));
    fused.into_iter().map(|(_, passage)| passage).collect()
}

impl SearchIndex {
    /// Tokens of `text` under the exact analyzer (lowercased, folded).
    fn exact_tokens(&self, text: &str) -> Vec<String> {
        let mut analyzer = self
            .index
            .tokenizers()
            .get("larra_exact")
            .expect("larra_exact registered");
        let mut tokens = Vec::new();
        let mut stream = analyzer.token_stream(text);
        while let Some(token) = stream.next() {
            tokens.push(token.text.clone());
            if tokens.len() >= 48 {
                break;
            }
        }
        tokens
    }

    /// Query tokens with function words removed so "which / the / have"
    /// cannot dominate ranking.
    fn content_tokens(&self, text: &str) -> Vec<String> {
        let mut out = Vec::new();
        for token in self.exact_tokens(text) {
            if is_search_stopword(&token) {
                continue;
            }
            out.push(token);
            if out.len() >= 24 {
                break;
            }
        }
        out
    }

    fn filename_tokens(&self, file_name: &str) -> Vec<String> {
        self.exact_tokens(&searchable_filename(file_name))
            .into_iter()
            .filter(|t| t.chars().count() >= 3 && !is_filename_noise(t))
            .collect()
    }

    fn stem_tokens(&self, lang: &str, text: &str) -> Vec<String> {
        let Some(mut analyzer) = self.index.tokenizers().get(&format!("larra_stem_{lang}")) else {
            return Vec::new();
        };
        let mut tokens = Vec::new();
        let mut stream = analyzer.token_stream(text);
        while let Some(token) = stream.next() {
            tokens.push(token.text.clone());
            if tokens.len() >= 24 {
                break;
            }
        }
        tokens
    }

    /// Build the passage query: exact terms across weighted fields, stemmed
    /// terms across every language field (documents fill exactly one),
    /// a phrase bonus, and cautious fuzzy matching for typos — never on
    /// numeric or short tokens, so business identifiers stay precise.
    fn passage_query(
        &self,
        message: &str,
        shelf_id: &str,
        document_id: Option<&str>,
    ) -> Box<dyn Query> {
        let tokens = self.content_tokens(message);
        let mut should: Vec<(Occur, Box<dyn Query>)> = Vec::new();

        let weighted_fields: [(Field, f32); 6] = [
            (self.fields.title, weights::TITLE),
            (self.fields.filename, weights::FILENAME),
            (self.fields.keywords, weights::KEYWORDS),
            (self.fields.section, weights::SECTION),
            (self.fields.body, weights::BODY),
            (self.fields.summary, weights::SUMMARY),
        ];

        for token in &tokens {
            for (field, weight) in weighted_fields {
                let term = Term::from_field_text(field, token);
                let tq: Box<dyn Query> =
                    Box::new(TermQuery::new(term, IndexRecordOption::WithFreqs));
                should.push((Occur::Should, Box::new(BoostQuery::new(tq, weight))));
            }
            // Fuzzy for reasonable typos on alphabetic tokens only.
            let alphabetic = token.chars().all(|c| c.is_alphabetic());
            if alphabetic && token.chars().count() >= 5 {
                let distance = if token.chars().count() >= 9 { 2 } else { 1 };
                for field in [self.fields.body, self.fields.title, self.fields.filename] {
                    let term = Term::from_field_text(field, token);
                    let fq: Box<dyn Query> = Box::new(FuzzyTermQuery::new(term, distance, true));
                    should.push((Occur::Should, Box::new(BoostQuery::new(fq, weights::FUZZY))));
                }
            }
        }

        for variant in hyphen_join_variants(message) {
            for token in self.exact_tokens(&variant) {
                if is_search_stopword(&token) || tokens.iter().any(|t| t == &token) {
                    continue;
                }
                for (field, weight) in weighted_fields {
                    let term = Term::from_field_text(field, &token);
                    let tq: Box<dyn Query> =
                        Box::new(TermQuery::new(term, IndexRecordOption::WithFreqs));
                    should.push((Occur::Should, Box::new(BoostQuery::new(tq, weight))));
                }
            }
        }

        // Stemmed terms — one field per language; a document only carries its
        // own language field, so cross-language noise stays out.
        let stem_text = tokens.join(" ");
        for lang in STEM_LANGS {
            let stem_field = self.fields.stems[*lang];
            for token in self.stem_tokens(lang, &stem_text) {
                if is_search_stopword(&token) {
                    continue;
                }
                let term = Term::from_field_text(stem_field, &token);
                let tq: Box<dyn Query> =
                    Box::new(TermQuery::new(term, IndexRecordOption::WithFreqs));
                should.push((
                    Occur::Should,
                    Box::new(BoostQuery::new(tq, weights::STEMMED)),
                ));
            }
        }

        // Phrase bonus keeps multi-word references precise.
        if tokens.len() >= 2 && tokens.len() <= 12 {
            let body_terms: Vec<Term> = tokens
                .iter()
                .map(|t| Term::from_field_text(self.fields.body, t))
                .collect();
            let pq: Box<dyn Query> = Box::new(PhraseQuery::new(body_terms));
            should.push((
                Occur::Should,
                Box::new(BoostQuery::new(pq, weights::PHRASE)),
            ));
            let name_terms: Vec<Term> = tokens
                .iter()
                .map(|t| Term::from_field_text(self.fields.filename, t))
                .collect();
            let npq: Box<dyn Query> = Box::new(PhraseQuery::new(name_terms));
            should.push((
                Occur::Should,
                Box::new(BoostQuery::new(npq, weights::PHRASE)),
            ));
        }

        let relevance: Box<dyn Query> = Box::new(BooleanQuery::new(should));
        let mut clauses: Vec<(Occur, Box<dyn Query>)> = vec![
            (
                Occur::Must,
                Box::new(TermQuery::new(
                    Term::from_field_text(self.fields.record_type, "passage"),
                    IndexRecordOption::Basic,
                )),
            ),
            (Occur::Must, relevance),
        ];
        clauses.push((
            Occur::Must,
            Box::new(TermQuery::new(
                Term::from_field_text(self.fields.shelf_id, shelf_id),
                IndexRecordOption::Basic,
            )),
        ));
        if let Some(doc_id) = document_id {
            clauses.push((
                Occur::Must,
                Box::new(TermQuery::new(
                    Term::from_field_text(self.fields.document_id, doc_id),
                    IndexRecordOption::Basic,
                )),
            ));
        }
        Box::new(BooleanQuery::new(clauses))
    }

    /// Search Shelf passages for the user message, unchanged.
    pub fn search_passages(
        &self,
        message: &str,
        shelf_id: &str,
        limit: usize,
    ) -> Result<Vec<SourcePassage>> {
        self.search_passages_in(message, shelf_id, None, limit)
    }

    fn search_passages_in(
        &self,
        message: &str,
        shelf_id: &str,
        document_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<SourcePassage>> {
        if self.content_tokens(message).is_empty() {
            return Ok(Vec::new());
        }
        let searcher = self.reader.searcher();
        let query = self.passage_query(message, shelf_id, document_id);
        let top = searcher.search(&query, &TopDocs::with_limit(limit).order_by_score())?;
        let mut hits = Vec::new();
        for (score, address) in top {
            let doc: TantivyDocument = searcher.doc(address)?;
            let get_str = |f: Field| -> String {
                doc.get_first(f)
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string()
            };
            let get_u64 = |f: Field| -> Option<u32> {
                doc.get_first(f).and_then(|v| v.as_u64()).map(|v| v as u32)
            };
            hits.push(SourcePassage {
                anchor: None,
                sid: String::new(),
                document_id: get_str(self.fields.document_id),
                shelf_id: get_str(self.fields.shelf_id),
                title: get_str(self.fields.title),
                section: {
                    let s = get_str(self.fields.section);
                    if s.is_empty() {
                        None
                    } else {
                        Some(s)
                    }
                },
                page_start: get_u64(self.fields.page_start),
                page_end: get_u64(self.fields.page_end),
                body: get_str(self.fields.body),
                path: get_str(self.fields.path),
                score,
            });
        }
        Ok(hits)
    }

    /// Files whose names overlap the question — used to pull extra passages
    /// from those documents and to raise their per-file cap.
    pub fn named_document_ids(&self, message: &str, files: &[(String, String)]) -> Vec<String> {
        let query: HashSet<String> = self.content_tokens(message).into_iter().collect();
        if query.is_empty() || files.is_empty() {
            return Vec::new();
        }
        let file_tokens: Vec<(&str, Vec<String>)> = files
            .iter()
            .map(|(id, name)| (id.as_str(), self.filename_tokens(name)))
            .collect();

        let mut df: HashMap<&str, usize> = HashMap::new();
        for (_, toks) in &file_tokens {
            let unique: HashSet<&str> = toks.iter().map(String::as_str).collect();
            for t in unique {
                *df.entry(t).or_insert(0) += 1;
            }
        }

        let mut scored: Vec<(f32, String)> = Vec::new();
        for (id, toks) in &file_tokens {
            let mut hits = 0u32;
            let mut score = 0.0f32;
            let mut distinctive = false;
            let unique: HashSet<&str> = toks.iter().map(String::as_str).collect();
            for t in unique {
                if !query.contains(t) {
                    continue;
                }
                hits += 1;
                let n = *df.get(t).unwrap_or(&1) as f32;
                score += 1.0 / n;
                if n <= 1.0 && t.chars().count() >= 4 {
                    distinctive = true;
                }
            }
            if hits >= 2 || distinctive {
                scored.push((score, (*id).to_string()));
            }
        }
        scored.sort_by(|a, b| b.0.total_cmp(&a.0));
        scored.truncate(3);
        scored.into_iter().map(|(_, id)| id).collect()
    }

    /// Shelf search plus extra passages from files the question names.
    pub fn search_and_merge_named(
        &self,
        message: &str,
        shelf_id: &str,
        files: &[(String, String)],
        limit: usize,
    ) -> Result<(Vec<SourcePassage>, Vec<String>)> {
        let named = self.named_document_ids(message, files);
        let mut hits = self.search_passages(message, shelf_id, limit)?;
        for id in &named {
            let extra = self.search_passages_in(message, shelf_id, Some(id), 12)?;
            merge_hits(&mut hits, extra);
        }
        Ok((hits, named))
    }

    /// Several queries fused with RRF. Named-file extras come from every query.
    pub fn search_fused(
        &self,
        queries: &[String],
        shelf_id: &str,
        files: &[(String, String)],
        limit: usize,
    ) -> Result<(Vec<SourcePassage>, Vec<String>)> {
        let Some((first, rest)) = queries.split_first() else {
            return Ok((Vec::new(), Vec::new()));
        };
        let (first_hits, mut named) = self.search_and_merge_named(first, shelf_id, files, limit)?;
        if rest.is_empty() {
            return Ok((first_hits, named));
        }
        let mut lists = vec![first_hits];
        for query in rest {
            let (hits, more) = self.search_and_merge_named(query, shelf_id, files, limit)?;
            lists.push(hits);
            for id in more {
                if !named.contains(&id) {
                    named.push(id);
                }
            }
        }
        Ok((fuse_passage_lists(lists), named))
    }

    /// True when at least one passage for this document is in the index.
    pub fn has_document(&self, document_id: &str) -> bool {
        let searcher = self.reader.searcher();
        let query = TermQuery::new(
            Term::from_field_text(self.fields.document_id, document_id),
            IndexRecordOption::Basic,
        );
        searcher.search(&query, &Count).unwrap_or(0) > 0
    }

    /// Body of a stored passage, used to find that spot in the extracted file.
    pub fn passage_needle(
        &self,
        document_id: &str,
        page: Option<u32>,
        section: Option<&str>,
    ) -> Option<String> {
        if document_id.is_empty() || (page.is_none() && section.is_none()) {
            return None;
        }
        let searcher = self.reader.searcher();
        let query = TermQuery::new(
            Term::from_field_text(self.fields.document_id, document_id),
            IndexRecordOption::Basic,
        );
        let top = searcher
            .search(&query, &TopDocs::with_limit(600).order_by_score())
            .ok()?;
        let section = section.map(str::trim).filter(|s| !s.is_empty());
        let mut by_section = None;
        for (_score, address) in top {
            let Ok(doc) = searcher.doc::<TantivyDocument>(address) else {
                continue;
            };
            let body = doc
                .get_first(self.fields.body)
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if body.is_empty() {
                continue;
            }
            let page_start = doc
                .get_first(self.fields.page_start)
                .and_then(|v| v.as_u64())
                .map(|v| v as u32);
            if page.is_some() && page_start == page {
                return Some(body.to_string());
            }
            if by_section.is_none() {
                let have = doc
                    .get_first(self.fields.section)
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if section.is_some_and(|want| have.eq_ignore_ascii_case(want)) {
                    by_section = Some(body.to_string());
                }
            }
        }
        by_section
    }

    /// Search older conversation memory.
    /// `only_threads` limits hits to those conversations.
    /// `exclude_message_ids` skips turns already in the standing prompt.
    pub fn search_messages(
        &self,
        message: &str,
        exclude_thread: Option<&str>,
        only_threads: Option<&[String]>,
        exclude_message_ids: &[String],
        limit: usize,
    ) -> Result<Vec<MemorySnippet>> {
        let tokens = self.content_tokens(message);
        if tokens.is_empty() {
            return Ok(Vec::new());
        }
        let mut should: Vec<(Occur, Box<dyn Query>)> = Vec::new();
        for token in &tokens {
            let term = Term::from_field_text(self.fields.body, token);
            should.push((
                Occur::Should,
                Box::new(TermQuery::new(term, IndexRecordOption::WithFreqs)),
            ));
        }
        let stem_text = tokens.join(" ");
        for lang in STEM_LANGS {
            let stem_field = self.fields.stems[*lang];
            for token in self.stem_tokens(lang, &stem_text) {
                if is_search_stopword(&token) {
                    continue;
                }
                let term = Term::from_field_text(stem_field, &token);
                let tq: Box<dyn Query> =
                    Box::new(TermQuery::new(term, IndexRecordOption::WithFreqs));
                should.push((Occur::Should, Box::new(BoostQuery::new(tq, 0.9))));
            }
        }
        let relevance: Box<dyn Query> = Box::new(BooleanQuery::new(should));
        let mut clauses: Vec<(Occur, Box<dyn Query>)> = vec![
            (
                Occur::Must,
                Box::new(TermQuery::new(
                    Term::from_field_text(self.fields.record_type, "message"),
                    IndexRecordOption::Basic,
                )),
            ),
            (Occur::Must, relevance),
        ];
        if let Some(thread) = exclude_thread {
            clauses.push((
                Occur::MustNot,
                Box::new(TermQuery::new(
                    Term::from_field_text(self.fields.thread_id, thread),
                    IndexRecordOption::Basic,
                )),
            ));
        }
        for message_id in exclude_message_ids {
            if message_id.is_empty() {
                continue;
            }
            clauses.push((
                Occur::MustNot,
                Box::new(TermQuery::new(
                    Term::from_field_text(self.fields.message_id, message_id),
                    IndexRecordOption::Basic,
                )),
            ));
        }
        match only_threads {
            Some([]) => return Ok(Vec::new()),
            Some(threads) => {
                let any: Vec<(Occur, Box<dyn Query>)> = threads
                    .iter()
                    .map(|id| {
                        (
                            Occur::Should,
                            Box::new(TermQuery::new(
                                Term::from_field_text(self.fields.thread_id, id),
                                IndexRecordOption::Basic,
                            )) as Box<dyn Query>,
                        )
                    })
                    .collect();
                clauses.push((Occur::Must, Box::new(BooleanQuery::new(any))));
            }
            None => {}
        }
        let query = BooleanQuery::new(clauses);
        let searcher = self.reader.searcher();
        let top = searcher.search(&query, &TopDocs::with_limit(limit).order_by_score())?;
        let mut hits = Vec::new();
        for (score, address) in top {
            let doc: TantivyDocument = searcher.doc(address)?;
            let get_str = |f: Field| -> String {
                doc.get_first(f)
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string()
            };
            let created = doc
                .get_first(self.fields.created_at)
                .and_then(|v| v.as_datetime())
                .map(|d| {
                    chrono::DateTime::<chrono::Utc>::from_timestamp(d.into_timestamp_secs(), 0)
                        .map(|d| d.to_rfc3339())
                        .unwrap_or_default()
                })
                .unwrap_or_default();
            hits.push(MemorySnippet {
                thread_id: get_str(self.fields.thread_id),
                message_id: get_str(self.fields.message_id),
                role: get_str(self.fields.role),
                body: get_str(self.fields.body),
                created_at: created,
                score,
            });
        }
        Ok(hits)
    }

    /// Total records, for diagnostics.
    pub fn num_docs(&self) -> u64 {
        self.reader.searcher().num_docs()
    }

    /// Query tokens used by the gate's overlap check (stopwords already gone).
    pub fn query_tokens(&self, text: &str) -> Vec<String> {
        self.content_tokens(text)
    }
}

#[cfg(test)]
mod tests {
    use crate::search::{searchable_filename, SearchIndex};
    use crate::types::{DocStatus, DocumentMeta, Passage, SourceType};

    fn index() -> (tempfile::TempDir, SearchIndex) {
        let dir = tempfile::tempdir().unwrap();
        let search = SearchIndex::open(dir.path()).unwrap();
        (dir, search)
    }

    fn meta_named(id: &str, shelf: &str, file_name: &str) -> DocumentMeta {
        DocumentMeta {
            id: id.into(),
            shelf_id: shelf.into(),
            source_id: "imported".into(),
            source_type: SourceType::Imported,
            path: format!("/{file_name}"),
            rel_path: file_name.into(),
            file_name: file_name.into(),
            format: "md".into(),
            size_bytes: 32,
            mtime_ms: 0,
            hash: format!("sha256:{id}"),
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

    fn meta(id: &str, shelf: &str) -> DocumentMeta {
        meta_named(id, shelf, &format!("{id}.md"))
    }

    fn passage(body: &str) -> Passage {
        Passage {
            section: None,
            page_start: None,
            page_end: None,
            body: body.into(),
        }
    }

    fn add(search: &SearchIndex, id: &str, shelf: &str, title: &str, body: &str) {
        search
            .index_document(
                &meta(id, shelf),
                "",
                &[],
                &[passage(body)],
                Some("en"),
                "full",
                title,
            )
            .unwrap();
    }

    fn add_file(
        search: &SearchIndex,
        id: &str,
        shelf: &str,
        title: &str,
        file_name: &str,
        body: &str,
    ) {
        search
            .index_document(
                &meta_named(id, shelf, file_name),
                "",
                &[],
                &[passage(body)],
                Some("en"),
                "full",
                title,
            )
            .unwrap();
    }

    #[test]
    fn empty_or_punctuation_query_returns_nothing() {
        let (_dir, search) = index();
        add(&search, "d1", "s1", "Invoice", "INV-2026-0042 is due");
        assert!(search.search_passages("   ", "s1", 8).unwrap().is_empty());
        assert!(search.search_passages("...", "s1", 8).unwrap().is_empty());
    }

    #[test]
    fn stopwords_alone_return_nothing() {
        let (_dir, search) = index();
        add(&search, "d1", "s1", "Invoice", "INV-2026-0042 is due");
        assert!(search
            .search_passages("which of the", "s1", 8)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn passage_needle_prefers_the_matching_page() {
        let (_dir, search) = index();
        let early = Passage {
            section: Some("Intro".into()),
            page_start: Some(2),
            page_end: Some(2),
            body: "Opening hours stay as written.".into(),
        };
        let cited = Passage {
            section: Some("Indemnity".into()),
            page_start: Some(48),
            page_end: Some(48),
            body: "The indemnity clause lasts ninety days.".into(),
        };
        search
            .index_document(
                &meta("d1", "s1"),
                "",
                &[],
                &[early, cited],
                Some("en"),
                "full",
                "Handbook",
            )
            .unwrap();
        assert_eq!(
            search.passage_needle("d1", Some(48), None).as_deref(),
            Some("The indemnity clause lasts ninety days.")
        );
        assert_eq!(
            search.passage_needle("d1", None, Some("Intro")).as_deref(),
            Some("Opening hours stay as written.")
        );
        assert!(search.passage_needle("d1", None, None).is_none());
    }

    #[test]
    fn stopwords_do_not_hide_content_tokens() {
        let (_dir, search) = index();
        add(
            &search,
            "d1",
            "s1",
            "Agreement",
            "Either party may terminate this agreement with notice.",
        );
        let tokens = search.query_tokens("Which of the termination terms apply?");
        assert!(!tokens
            .iter()
            .any(|t| t == "which" || t == "the" || t == "of"));
        assert!(tokens.iter().any(|t| t.starts_with("terminat")));
        let hits = search
            .search_passages("Which of the termination terms apply?", "s1", 8)
            .unwrap();
        assert!(!hits.is_empty());
    }

    #[test]
    fn exact_identifier_ranks_above_a_fuzzy_neighbour() {
        let (_dir, search) = index();
        add(
            &search,
            "exact",
            "s1",
            "Invoice INV-2026-0042",
            "Pay INV-2026-0042 by Friday.",
        );
        add(
            &search,
            "other",
            "s1",
            "Office move",
            "The painter booked Friday. Nothing about invoices here.",
        );
        let hits = search.search_passages("INV-2026-0042", "s1", 8).unwrap();
        assert!(!hits.is_empty());
        assert!(
            hits[0].title.contains("INV-2026-0042") || hits[0].body.contains("INV-2026-0042"),
            "exact business id should win, got {:?}",
            hits[0].title
        );
    }

    #[test]
    fn shelf_scoping_excludes_the_other_shelf() {
        let (_dir, search) = index();
        add(
            &search,
            "a",
            "shelf-a",
            "Secret A",
            "unique-alpha-token lives here",
        );
        add(
            &search,
            "b",
            "shelf-b",
            "Secret B",
            "unique-alpha-token lives here",
        );
        let a = search
            .search_passages("unique-alpha-token", "shelf-a", 8)
            .unwrap();
        let b = search
            .search_passages("unique-alpha-token", "shelf-b", 8)
            .unwrap();
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].document_id, "a");
        assert_eq!(b.len(), 1);
        assert_eq!(b[0].document_id, "b");
        assert!(search
            .search_passages("unique-alpha-token", "shelf-missing", 8)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn short_tokens_do_not_fuzzy_match() {
        let (_dir, search) = index();
        add(&search, "d1", "s1", "Note", "The team meets on Monday.");
        // "term" is 4 letters — below the fuzzy threshold of 5.
        assert!(search.search_passages("term", "s1", 8).unwrap().is_empty());
    }

    #[test]
    fn longer_typos_still_reach_the_body() {
        let (_dir, search) = index();
        add(
            &search,
            "d1",
            "s1",
            "Agreement",
            "Either party may terminate this agreement with notice.",
        );
        let hits = search
            .search_passages("terminaton of the agreement", "s1", 8)
            .unwrap();
        assert!(!hits.is_empty(), "5+ letter typos should fuzzy-match");
    }

    #[test]
    fn searchable_filename_drops_extension_and_punctuation() {
        assert_eq!(
            searchable_filename("NORTHWIND - Services agreement Studio Lead (J.K) (DRAFT).pdf"),
            "NORTHWIND Services agreement Studio Lead (J K) (DRAFT)"
        );
    }

    #[test]
    fn filename_ranks_above_a_generic_title() {
        let (_dir, search) = index();
        add_file(
            &search,
            "services",
            "s1",
            "SERVICES AGREEMENT FOR STUDIO LEAD",
            "NORTHWIND - Services agreement Studio Lead (J.K) (DRAFT).pdf",
            "The parties agree the recitals of this commercial relationship.",
        );
        add_file(
            &search,
            "invest",
            "s1",
            "Investment Agreement",
            "Riverbank.Harbor - Investment Agreement (DRAFT).docx.pdf",
            "The company and the company and the company are parties hereto.",
        );
        let query = "Check the important info from NORTHWIND - Services agreement Studio Lead (J.K) (DRAFT).pdf";
        let files = vec![
            (
                "services".into(),
                "NORTHWIND - Services agreement Studio Lead (J.K) (DRAFT).pdf".into(),
            ),
            (
                "invest".into(),
                "Riverbank.Harbor - Investment Agreement (DRAFT).docx.pdf".into(),
            ),
        ];
        let named = search.named_document_ids(query, &files);
        assert!(
            named.contains(&"services".to_string()),
            "expected the named services file, got {named:?}"
        );
        let (hits, _) = search
            .search_and_merge_named(query, "s1", &files, 8)
            .unwrap();
        assert!(!hits.is_empty());
        assert_eq!(hits[0].document_id, "services");
    }

    #[test]
    fn has_document_tracks_index_contents() {
        let (_dir, search) = index();
        assert!(!search.has_document("d1"));
        add(&search, "d1", "s1", "Note", "hello searchable body");
        assert!(search.has_document("d1"));
    }

    #[test]
    fn a_single_common_filename_token_does_not_pin_every_file() {
        let (_dir, search) = index();
        let files = vec![
            ("a".into(), "Services agreement one.pdf".into()),
            ("b".into(), "Investment agreement two.pdf".into()),
        ];
        let named = search.named_document_ids("agreement", &files);
        assert!(named.is_empty(), "got {named:?}");
    }

    #[test]
    fn hyphen_in_the_query_still_matches_unhyphenated_text() {
        let (_dir, search) = index();
        add(
            &search,
            "d1",
            "s1",
            "Pact",
            "The noncompete clause lasts two years.",
        );
        let hits = search
            .search_passages("non-compete clause", "s1", 8)
            .unwrap();
        assert!(
            hits.iter().any(|h| h.document_id == "d1"),
            "hyphenated query should hit unhyphenated body, got {:?}",
            hits.iter().map(|h| &h.body).collect::<Vec<_>>()
        );
    }

    #[test]
    fn unhyphenated_query_matches_hyphenated_body() {
        let (_dir, search) = index();
        add(
            &search,
            "d1",
            "s1",
            "Pact",
            "The non-compete clause lasts two years.",
        );
        let hits = search.search_passages("noncompete", "s1", 8).unwrap();
        assert!(
            hits.iter().any(|h| h.document_id == "d1"),
            "compact query should hit hyphenated body, got {:?}",
            hits.iter().map(|h| &h.body).collect::<Vec<_>>()
        );
    }

    fn hit(doc: &str, score: f32, body: &str) -> crate::types::SourcePassage {
        crate::types::SourcePassage {
            anchor: None,
            sid: String::new(),
            document_id: doc.into(),
            shelf_id: "s1".into(),
            title: doc.into(),
            section: None,
            page_start: None,
            page_end: None,
            body: body.into(),
            path: format!("/{doc}.md"),
            score,
        }
    }

    #[test]
    fn fuse_prefers_passages_that_appear_in_several_lists() {
        let shared = hit("d1", 5.0, "shared body of the clause");
        let only_first = hit("d2", 9.0, "only in the first list");
        let fused = super::fuse_passage_lists(vec![
            vec![only_first.clone(), shared.clone()],
            vec![shared.clone()],
            vec![shared],
        ]);
        assert_eq!(fused[0].document_id, "d1");
        assert!(fused.iter().any(|p| p.document_id == "d2"));
        assert!(fused[0].score > 4.0);
    }

    #[test]
    fn fused_search_keeps_hits_from_each_query() {
        let (_dir, search) = index();
        add(
            &search,
            "d1",
            "s1",
            "A",
            "The zebra lives in the east wing.",
        );
        add(
            &search,
            "d2",
            "s1",
            "B",
            "Bad leaver clauses apply on termination.",
        );
        let files = vec![("d1".into(), "a.md".into()), ("d2".into(), "b.md".into())];
        let (hits, _) = search
            .search_fused(
                &["zebra east".into(), "bad leaver termination".into()],
                "s1",
                &files,
                8,
            )
            .unwrap();
        let ids: std::collections::HashSet<_> =
            hits.iter().map(|h| h.document_id.as_str()).collect();
        assert!(ids.contains("d1"), "{ids:?}");
        assert!(ids.contains("d2"), "{ids:?}");
    }

    #[test]
    fn message_search_can_limit_to_listed_threads() {
        let (_dir, search) = index();
        let now = chrono::Utc::now();
        search
            .index_message(
                "t_kitchen",
                "m1",
                "user",
                "The office move budget is twelve thousand euros.",
                Some("en"),
                now,
            )
            .unwrap();
        search
            .index_message(
                "t_office",
                "m2",
                "user",
                "The office move budget is ninety thousand euros.",
                Some("en"),
                now,
            )
            .unwrap();
        let kitchen = vec!["t_kitchen".to_string()];
        let hits = search
            .search_messages("office move budget", Some("t_now"), Some(&kitchen), &[], 8)
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].thread_id, "t_kitchen");
        assert!(search
            .search_messages("office move budget", Some("t_now"), Some(&[]), &[], 8)
            .unwrap()
            .is_empty());
        let skip = vec!["m1".to_string()];
        assert!(search
            .search_messages("office move budget", None, Some(&kitchen), &skip, 8)
            .unwrap()
            .is_empty());
    }
}
