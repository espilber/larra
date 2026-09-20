//! Tantivy schema, stem tokenizers, and language tags.

use std::collections::BTreeMap;
use tantivy::schema::{
    DateOptions, Field, IndexRecordOption, NumericOptions, Schema, TextFieldIndexing, TextOptions,
    STORED, STRING,
};
use tantivy::tokenizer::{
    AsciiFoldingFilter, Language, LowerCaser, RemoveLongFilter, SimpleTokenizer, Stemmer,
    TextAnalyzer,
};
use tantivy::Index;

use super::stems::{extra_algorithm, ExtraStemmer};

/// Languages with a dedicated stem field.
///
/// Most of these use Tantivy's built-in Snowball `Stemmer`. Codes that
/// `tantivy::tokenizer::Language` does not include (Catalan, Polish, Czech,
/// Basque, …) go through [`extra_algorithm`].
pub const STEM_LANGS: &[&str] = &[
    "ar", "ca", "cs", "da", "de", "el", "en", "es", "et", "eu", "fi", "fr", "ga", "hi", "hu", "hy",
    "id", "it", "lt", "ne", "nl", "no", "pl", "pt", "ro", "ru", "sv", "ta", "tr", "yi",
];

pub(crate) fn tantivy_language(code: &str) -> Option<Language> {
    Some(match code {
        "ar" => Language::Arabic,
        "da" => Language::Danish,
        "de" => Language::German,
        "el" => Language::Greek,
        "en" => Language::English,
        "es" => Language::Spanish,
        "fi" => Language::Finnish,
        "fr" => Language::French,
        "hu" => Language::Hungarian,
        "it" => Language::Italian,
        "nl" => Language::Dutch,
        "no" => Language::Norwegian,
        "pt" => Language::Portuguese,
        "ro" => Language::Romanian,
        "ru" => Language::Russian,
        "sv" => Language::Swedish,
        "ta" => Language::Tamil,
        "tr" => Language::Turkish,
        _ => return None,
    })
}

/// Normalize a detected language tag (ISO 639-1/639-3/name) to a stem code.
pub fn normalize_lang(tag: &str) -> Option<&'static str> {
    let t = tag.trim().to_lowercase();
    let code = match t.as_str() {
        "ar" | "ara" | "arabic" => "ar",
        "ca" | "cat" | "catalan" | "català" | "valencià" => "ca",
        "cs" | "ces" | "cze" | "czech" => "cs",
        "da" | "dan" | "danish" => "da",
        "de" | "deu" | "ger" | "german" | "deutsch" => "de",
        "el" | "ell" | "gre" | "greek" => "el",
        "en" | "eng" | "english" => "en",
        "es" | "spa" | "spanish" | "español" | "castellano" => "es",
        "et" | "est" | "estonian" => "et",
        "eu" | "eus" | "baq" | "basque" | "euskara" => "eu",
        "fi" | "fin" | "finnish" => "fi",
        "fr" | "fra" | "fre" | "french" => "fr",
        "ga" | "gle" | "irish" | "gaeilge" => "ga",
        "hi" | "hin" | "hindi" => "hi",
        "hu" | "hun" | "hungarian" => "hu",
        "hy" | "hye" | "arm" | "armenian" => "hy",
        "id" | "ind" | "indonesian" => "id",
        "it" | "ita" | "italian" => "it",
        "lt" | "lit" | "lithuanian" => "lt",
        "ne" | "nep" | "nepali" => "ne",
        "nl" | "nld" | "dut" | "dutch" => "nl",
        "no" | "nor" | "nb" | "nob" | "nn" | "nno" | "norwegian" | "bokmål" => "no",
        "pl" | "pol" | "polish" => "pl",
        "pt" | "por" | "portuguese" => "pt",
        "ro" | "ron" | "rum" | "romanian" => "ro",
        "ru" | "rus" | "russian" => "ru",
        "sv" | "swe" | "swedish" => "sv",
        "ta" | "tam" | "tamil" => "ta",
        "tr" | "tur" | "turkish" => "tr",
        "yi" | "yid" | "yiddish" => "yi",
        _ => return None,
    };
    Some(code)
}

pub(crate) const SCHEMA_VERSION: &str = "larra-search/v3";

/// Ranking weights — implementation constants tuned against the retrieval
/// eval suite (see `tests/retrieval_eval.rs`), not product settings.
pub(crate) mod weights {
    pub const TITLE: f32 = 4.0;
    /// On-disk name, kept even when PDF metadata replaces the title.
    pub const FILENAME: f32 = 4.0;
    pub const KEYWORDS: f32 = 3.0;
    pub const SECTION: f32 = 2.5;
    pub const BODY: f32 = 1.0;
    pub const SUMMARY: f32 = 0.8;
    pub const STEMMED: f32 = 1.1;
    pub const PHRASE: f32 = 2.2;
    pub const FUZZY: f32 = 0.3;
}

pub(crate) struct Fields {
    pub(crate) record_type: Field,
    pub(crate) shelf_id: Field,
    pub(crate) document_id: Field,
    pub(crate) source_id: Field,
    pub(crate) source_type: Field,
    pub(crate) path: Field,
    pub(crate) filename: Field,
    pub(crate) title: Field,
    pub(crate) summary: Field,
    pub(crate) keywords: Field,
    pub(crate) section: Field,
    pub(crate) body: Field,
    pub(crate) stems: BTreeMap<&'static str, Field>,
    pub(crate) page_start: Field,
    pub(crate) page_end: Field,
    pub(crate) language: Field,
    pub(crate) quality: Field,
    pub(crate) thread_id: Field,
    pub(crate) message_id: Field,
    pub(crate) role: Field,
    pub(crate) created_at: Field,
}

pub(crate) fn build_schema() -> (Schema, Fields) {
    let mut builder = Schema::builder();

    let exact_indexing = TextFieldIndexing::default()
        .set_tokenizer("larra_exact")
        .set_index_option(IndexRecordOption::WithFreqsAndPositions);
    let exact_stored = TextOptions::default()
        .set_indexing_options(exact_indexing.clone())
        .set_stored();

    let record_type = builder.add_text_field("record_type", STRING | STORED);
    let shelf_id = builder.add_text_field("shelf_id", STRING | STORED);
    let document_id = builder.add_text_field("document_id", STRING | STORED);
    let source_id = builder.add_text_field("source_id", STRING | STORED);
    let source_type = builder.add_text_field("source_type", STRING | STORED);
    let path = builder.add_text_field("path", STORED);
    let filename = builder.add_text_field("filename", exact_stored.clone());
    let title = builder.add_text_field("title", exact_stored.clone());
    let summary = builder.add_text_field("summary", exact_stored.clone());
    let keywords = builder.add_text_field("keywords", exact_stored.clone());
    let section = builder.add_text_field("section", exact_stored.clone());
    let body = builder.add_text_field("body", exact_stored.clone());

    let mut stems = BTreeMap::new();
    for lang in STEM_LANGS {
        let indexing = TextFieldIndexing::default()
            .set_tokenizer(&format!("larra_stem_{lang}"))
            .set_index_option(IndexRecordOption::WithFreqs);
        let opts = TextOptions::default().set_indexing_options(indexing);
        let field = builder.add_text_field(&format!("stem_{lang}"), opts);
        stems.insert(*lang, field);
    }

    let numeric = NumericOptions::default().set_stored();
    let page_start = builder.add_u64_field("page_start", numeric.clone());
    let page_end = builder.add_u64_field("page_end", numeric);
    let language = builder.add_text_field("language", STRING | STORED);
    let quality = builder.add_text_field("quality", STRING | STORED);
    let thread_id = builder.add_text_field("thread_id", STRING | STORED);
    let message_id = builder.add_text_field("message_id", STRING | STORED);
    let role = builder.add_text_field("role", STRING | STORED);
    let created_at = builder.add_date_field(
        "created_at",
        DateOptions::default().set_stored().set_indexed(),
    );

    let schema = builder.build();
    let fields = Fields {
        record_type,
        shelf_id,
        document_id,
        source_id,
        source_type,
        path,
        filename,
        title,
        summary,
        keywords,
        section,
        body,
        stems,
        page_start,
        page_end,
        language,
        quality,
        thread_id,
        message_id,
        role,
        created_at,
    };
    (schema, fields)
}

pub(crate) fn register_tokenizers(index: &Index) {
    let exact = TextAnalyzer::builder(SimpleTokenizer::default())
        .filter(RemoveLongFilter::limit(64))
        .filter(LowerCaser)
        .filter(AsciiFoldingFilter)
        .build();
    index.tokenizers().register("larra_exact", exact);

    for lang in STEM_LANGS {
        let name = format!("larra_stem_{lang}");
        let analyzer = if let Some(algorithm) = extra_algorithm(lang) {
            TextAnalyzer::builder(SimpleTokenizer::default())
                .filter(RemoveLongFilter::limit(64))
                .filter(LowerCaser)
                .filter(ExtraStemmer::new(algorithm))
                .filter(AsciiFoldingFilter)
                .build()
        } else {
            let language = tantivy_language(lang).unwrap_or_else(|| {
                panic!("stem language {lang} must map to Tantivy or an extra algorithm")
            });
            TextAnalyzer::builder(SimpleTokenizer::default())
                .filter(RemoveLongFilter::limit(64))
                .filter(LowerCaser)
                .filter(Stemmer::new(language))
                .filter(AsciiFoldingFilter)
                .build()
        };
        index.tokenizers().register(&name, analyzer);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_stem_lang_has_exactly_one_backend() {
        for lang in STEM_LANGS {
            let extra = extra_algorithm(lang).is_some();
            let builtin = tantivy_language(lang).is_some();
            assert!(
                extra ^ builtin,
                "{lang} must be either Tantivy built-in or extra, not both/neither"
            );
        }
    }

    #[test]
    fn normalize_lang_maps_iso_and_names() {
        assert_eq!(normalize_lang("spa"), Some("es"));
        assert_eq!(normalize_lang("cat"), Some("ca"));
        assert_eq!(normalize_lang("fra"), Some("fr"));
        assert_eq!(normalize_lang("pol"), Some("pl"));
        assert_eq!(normalize_lang("ces"), Some("cs"));
        assert_eq!(normalize_lang("nob"), Some("no"));
        assert_eq!(normalize_lang("xyz"), None);
    }
}
