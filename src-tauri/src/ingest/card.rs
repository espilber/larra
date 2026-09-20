//! Deterministic Cards (`larra-card/v1`) — compact per-document metadata,
//! generated without an LLM, stored as YAML in application data.

use crate::pii::PiiSummary;
use crate::types::{Card, OutlineEntry, SourceType};
use anyhow::Result;
use std::path::Path;

pub struct CardInputs<'a> {
    pub doc_id: &'a str,
    pub source_type: SourceType,
    pub path: &'a str,
    pub hash: &'a str,
    pub title: &'a str,
    pub format: &'a str,
    pub language: Option<&'a str>,
    pub summary: &'a str,
    pub keywords: &'a [String],
    pub outline: &'a [OutlineEntry],
    pub ocr_used: bool,
    pub privacy: &'a PiiSummary,
}

pub fn build_card(inputs: CardInputs) -> Card {
    Card {
        schema: Card::SCHEMA.to_string(),
        id: inputs.doc_id.to_string(),
        source: inputs.source_type,
        path: inputs.path.to_string(),
        hash: inputs.hash.to_string(),
        title: inputs.title.to_string(),
        format: inputs.format.to_string(),
        language: inputs.language.map(str::to_string),
        summary: inputs.summary.to_string(),
        keywords: inputs.keywords.to_vec(),
        outline: inputs.outline.to_vec(),
        quality: if inputs.ocr_used { "ocr" } else { "full" }.to_string(),
        privacy: inputs.privacy.clone(),
    }
}

pub fn write_card(path: &Path, card: &Card) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, serde_yaml_ng::to_string(card)?)?;
    Ok(())
}

pub fn read_card(path: &Path) -> Result<Card> {
    let text = std::fs::read_to_string(path)?;
    Ok(serde_yaml_ng::from_str(&text)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn card_roundtrip_never_stores_pii_values() {
        let dir = tempfile::tempdir().unwrap();
        let mut privacy = PiiSummary {
            total: 3,
            ..Default::default()
        };
        privacy.categories.insert("email".into(), 2);
        privacy.categories.insert("iban".into(), 1);
        let card = build_card(CardInputs {
            doc_id: "d_7f3a",
            source_type: SourceType::Linked,
            path: "/Company/Legal/Northwind.pdf",
            hash: "sha256:9c1e4b",
            title: "Framework Services Agreement — Northwind Trading S.L.",
            format: "pdf",
            language: Some("es"),
            summary: "Services agreement covering delivery and termination.",
            keywords: &["Northwind Trading".into(), "termination".into()],
            outline: &[OutlineEntry {
                title: "Termination".into(),
                page: Some(14),
            }],
            ocr_used: false,
            privacy: &privacy,
        });
        let path = dir.path().join("card.yml");
        write_card(&path, &card).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("schema: larra-card/v1"));
        assert!(text.contains("quality: full"));
        // Counts only — no raw identifiers anywhere.
        assert!(text.contains("email: 2"));
        let read = read_card(&path).unwrap();
        assert_eq!(read.privacy.total, 3);
        assert_eq!(read.title, card.title);
    }
}
