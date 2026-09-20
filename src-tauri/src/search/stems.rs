//! Extra Snowball stemmers for languages Tantivy 0.26 does not ship.
//!
//! `tantivy-stemmers` exposes each algorithm as `fn(&str) -> Cow<str>`, but
//! its `TokenFilter` targets an older `tantivy-tokenizer-api` than tantivy
//! 0.26 uses. This adapter wraps those functions in a tantivy-0.26 filter.

use std::borrow::Cow;
use tantivy::tokenizer::{Token, TokenFilter, TokenStream, Tokenizer};
use tantivy_stemmers::algorithms::Algorithm;

/// Snowball (or Polish Yarovoy) function for a language code, when Tantivy's
/// built-in [`tantivy::tokenizer::Language`] enum does not cover it.
pub(crate) fn extra_algorithm(lang: &str) -> Option<Algorithm> {
    Some(match lang {
        "ca" => tantivy_stemmers::algorithms::catalan,
        "cs" => tantivy_stemmers::algorithms::czech_dolamic_light,
        "et" => tantivy_stemmers::algorithms::estonian_freienthal,
        "eu" => tantivy_stemmers::algorithms::basque,
        "ga" => tantivy_stemmers::algorithms::irish_gaelic,
        "hi" => tantivy_stemmers::algorithms::hindi_lightweight,
        "hy" => tantivy_stemmers::algorithms::armenian_mkrtchyan,
        "id" => tantivy_stemmers::algorithms::indonesian_tala,
        "lt" => tantivy_stemmers::algorithms::lithuanian_jocas,
        "ne" => tantivy_stemmers::algorithms::nepali,
        "pl" => tantivy_stemmers::algorithms::polish_yarovoy,
        "yi" => tantivy_stemmers::algorithms::yiddish_urieli,
        _ => return None,
    })
}

#[derive(Clone)]
pub(crate) struct ExtraStemmer {
    algorithm: Algorithm,
}

impl ExtraStemmer {
    pub(crate) fn new(algorithm: Algorithm) -> Self {
        Self { algorithm }
    }
}

impl TokenFilter for ExtraStemmer {
    type Tokenizer<T: Tokenizer> = ExtraStemmerFilter<T>;

    fn transform<T: Tokenizer>(self, tokenizer: T) -> ExtraStemmerFilter<T> {
        ExtraStemmerFilter {
            algorithm: self.algorithm,
            inner: tokenizer,
        }
    }
}

#[derive(Clone)]
pub(crate) struct ExtraStemmerFilter<T> {
    algorithm: Algorithm,
    inner: T,
}

impl<T: Tokenizer> Tokenizer for ExtraStemmerFilter<T> {
    type TokenStream<'a> = ExtraStemmerStream<T::TokenStream<'a>>;

    fn token_stream<'a>(&'a mut self, text: &'a str) -> Self::TokenStream<'a> {
        ExtraStemmerStream {
            algorithm: self.algorithm,
            tail: self.inner.token_stream(text),
        }
    }
}

pub(crate) struct ExtraStemmerStream<T> {
    algorithm: Algorithm,
    tail: T,
}

impl<T: TokenStream> TokenStream for ExtraStemmerStream<T> {
    fn advance(&mut self) -> bool {
        if !self.tail.advance() {
            return false;
        }
        let token = self.tail.token_mut();
        match (self.algorithm)(&token.text) {
            Cow::Owned(s) => token.text = s,
            Cow::Borrowed(s) => {
                if s.len() != token.text.len() {
                    token.text = s.to_string();
                }
            }
        }
        true
    }

    fn token(&self) -> &Token {
        self.tail.token()
    }

    fn token_mut(&mut self) -> &mut Token {
        self.tail.token_mut()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tantivy::tokenizer::{LowerCaser, SimpleTokenizer, TextAnalyzer};

    fn tokens(algorithm: Algorithm, text: &str) -> Vec<String> {
        let mut analyzer = TextAnalyzer::builder(SimpleTokenizer::default())
            .filter(LowerCaser)
            .filter(ExtraStemmer::new(algorithm))
            .build();
        let mut stream = analyzer.token_stream(text);
        let mut out = Vec::new();
        while let Some(token) = stream.next() {
            out.push(token.text.clone());
        }
        out
    }

    #[test]
    fn extra_stemmer_collapses_inflections() {
        let ca = tokens(
            tantivy_stemmers::algorithms::catalan,
            "terminació terminacions",
        );
        assert_eq!(ca.len(), 2);
        assert_eq!(ca[0], ca[1]);

        let pl = tokens(tantivy_stemmers::algorithms::polish_yarovoy, "umowy umowa");
        assert_eq!(pl.len(), 2);
        assert_eq!(pl[0], pl[1]);
    }

    #[test]
    fn extra_algorithm_covers_only_non_tantivy_langs() {
        assert!(extra_algorithm("ca").is_some());
        assert!(extra_algorithm("pl").is_some());
        assert!(extra_algorithm("en").is_none());
        assert!(extra_algorithm("es").is_none());
    }
}
