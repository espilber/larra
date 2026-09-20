# ADR 0003: Multilingual lexical stemming

- Status: accepted
- Date: 2026-08

Business documents inflect. The index keeps one stem field per supported language so "vacances" can match "vacança" (and the same for Spanish, Polish, and the rest) without a separate embedding model.

Languages in Tantivy's Snowball `Language` enum use the built-in `Stemmer`. Languages it does not ship use Snowball algorithms from `tantivy-stemmers`, wrapped in `search/stems.rs` because that crate's `TokenFilter` targets an older `tantivy-tokenizer-api` than tantivy 0.26.

Queries search every stem field. A document fills only the field for its detected language. Adding a language is a schema bump (`SCHEMA_VERSION`).
