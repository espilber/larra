//! `settings.json` — the small amount of durable configuration.

use serde::{Deserialize, Deserializer, Serialize};
use std::path::Path;

use crate::i18n::UiLocalePref;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TextSize {
    #[default]
    Default,
    Large,
    Larger,
}

impl<'de> Deserialize<'de> for TextSize {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Ok(match raw.as_str() {
            "large" => Self::Large,
            "larger" => Self::Larger,
            _ => Self::Default,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    /// Standing instructions, sent with every Chat message and Recipe run.
    /// Never mixed with retrieved documents.
    #[serde(alias = "house_rules")]
    pub house_rules: String,
    /// The AI Chat uses. The previous file stays until a new process is Ready.
    #[serde(alias = "active_model")]
    pub active_model: Option<ActiveModel>,
    /// Measured prompt-processing budget: how many characters of local
    /// context this machine can comfortably feed to the model.
    #[serde(alias = "context_budget_chars")]
    pub context_budget_chars: Option<usize>,
    /// Result of the installation benchmark (kept for diagnostics).
    pub benchmark: Option<BenchmarkResult>,
    /// First-run onboarding finished.
    #[serde(alias = "onboarding_done")]
    pub onboarding_done: bool,
    /// When true, Chat may look up public web pages from this computer.
    #[serde(alias = "allow_online_research")]
    pub allow_online_research: bool,
    /// Window type size: the current default, then two larger steps.
    #[serde(alias = "text_size")]
    pub text_size: TextSize,
    /// UI language: follow the computer, or pin a shipped catalog.
    #[serde(alias = "ui_locale")]
    pub ui_locale: UiLocalePref,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveModel {
    /// GGUF file name inside `<app-data>/models/`.
    pub file: String,
    /// User-facing model name, e.g. "Gemma 4 12B".
    pub name: String,
    /// Where it came from ("huggingface" | "ollama").
    pub source: String,
    /// Repo or library reference, for Settings display.
    pub reference: String,
    /// License identifier shown before download.
    pub license: Option<String>,
    /// Approximate download size in bytes.
    #[serde(alias = "size_bytes")]
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BenchmarkResult {
    /// Prompt tokens processed per second, as measured on this machine.
    #[serde(alias = "prompt_tokens_per_second")]
    pub prompt_tokens_per_second: f64,
    /// Generation tokens per second.
    #[serde(alias = "generation_tokens_per_second")]
    pub generation_tokens_per_second: f64,
    /// When the benchmark ran (RFC 3339).
    #[serde(alias = "measured_at")]
    pub measured_at: String,
    /// Model file that was measured.
    #[serde(alias = "model_file")]
    pub model_file: String,
    /// Missing on older measurements, which must be recalibrated.
    #[serde(default)]
    pub runtime: Option<BenchmarkRuntime>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BenchmarkRuntime {
    pub engine_build: String,
    pub accelerator: String,
    pub context_tokens: u32,
    pub batch: u32,
    pub ubatch: u32,
    pub gpu_layers: u32,
}

impl Settings {
    /// Load `settings.json`, or defaults when the file is missing or unreadable.
    pub fn load(path: &Path) -> Self {
        let mut settings: Self = crate::paths::read_json(path).unwrap_or_default();
        settings.house_rules =
            crate::limits::clip_chars(&settings.house_rules, crate::limits::HOUSE_RULES_MAX_CHARS);
        settings
    }

    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        crate::paths::write_json(path, self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corrupt_json_yields_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, "{not json").unwrap();
        let settings = Settings::load(&path);
        assert!(!settings.onboarding_done);
        assert!(settings.house_rules.is_empty());
        assert!(!settings.allow_online_research);
    }

    #[test]
    fn roundtrip_and_snake_case_alias() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(
            &path,
            r#"{"house_rules":"Answer in Catalan.","onboarding_done":true}"#,
        )
        .unwrap();
        let loaded = Settings::load(&path);
        assert_eq!(loaded.house_rules, "Answer in Catalan.");
        assert!(loaded.onboarding_done);
        loaded.save(&path).unwrap();
        let again = Settings::load(&path);
        assert_eq!(again.house_rules, "Answer in Catalan.");
        assert!(!again.allow_online_research);
    }

    #[test]
    fn online_research_roundtrips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let settings = Settings {
            allow_online_research: true,
            ..Default::default()
        };
        settings.save(&path).unwrap();
        let loaded = Settings::load(&path);
        assert!(loaded.allow_online_research);
    }

    #[test]
    fn text_size_roundtrips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let settings = Settings {
            text_size: TextSize::Larger,
            ..Default::default()
        };
        settings.save(&path).unwrap();
        let loaded = Settings::load(&path);
        assert_eq!(loaded.text_size, TextSize::Larger);
    }

    #[test]
    fn unknown_text_size_becomes_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(
            &path,
            r#"{"houseRules":"Keep it short.","textSize":"huge"}"#,
        )
        .unwrap();
        let loaded = Settings::load(&path);
        assert_eq!(loaded.house_rules, "Keep it short.");
        assert_eq!(loaded.text_size, TextSize::Default);
    }

    #[test]
    fn ui_locale_roundtrips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let settings = Settings {
            ui_locale: UiLocalePref::Ca,
            ..Default::default()
        };
        settings.save(&path).unwrap();
        let loaded = Settings::load(&path);
        assert_eq!(loaded.ui_locale, UiLocalePref::Ca);
    }

    #[test]
    fn unknown_ui_locale_becomes_system() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, r#"{"uiLocale":"klingon"}"#).unwrap();
        let loaded = Settings::load(&path);
        assert_eq!(loaded.ui_locale, UiLocalePref::System);
    }

    #[test]
    fn house_rules_are_clipped_on_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let long = "x".repeat(crate::limits::HOUSE_RULES_MAX_CHARS + 50);
        std::fs::write(&path, serde_json::json!({ "houseRules": long }).to_string()).unwrap();
        let loaded = Settings::load(&path);
        assert_eq!(
            loaded.house_rules.chars().count(),
            crate::limits::HOUSE_RULES_MAX_CHARS
        );
    }
}
