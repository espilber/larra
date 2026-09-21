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
    /// GGUF file name inside `<app-data>/models/`. Empty for an external
    /// endpoint, where no file is managed locally.
    pub file: String,
    /// User-facing model name, e.g. "Gemma 4 12B".
    pub name: String,
    /// Where it came from ("huggingface" | "ollama" | "external").
    pub source: String,
    /// Repo or library reference, for Settings display.
    pub reference: String,
    /// License identifier shown before download.
    pub license: Option<String>,
    /// Approximate download size in bytes.
    #[serde(alias = "size_bytes")]
    pub size_bytes: u64,
    /// Set when Chat talks to an OpenAI-compatible server instead of the
    /// bundled llama.cpp engine. `None` means the local GGUF variant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<EndpointConfig>,
}

/// An external OpenAI-compatible model endpoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EndpointConfig {
    /// Base URL, e.g. `http://127.0.0.1:1234/v1`. Normalized: no trailing
    /// slash, no userinfo, loopback host only in this first version.
    pub base_url: String,
    /// The identifier sent in the `model` field of a request.
    pub model_id: String,
    /// Optional bearer token. `None` for servers that need no auth, which is
    /// the common case on the same machine.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
}

impl EndpointConfig {
    /// Validate and normalize the values the user typed. Returns the
    /// normalized `EndpointConfig` or a short reason for the Settings UI.
    pub fn parse(base_url: &str, model_id: &str, api_key: Option<&str>) -> Result<Self, String> {
        let base_url = Self::normalize_base_url(base_url)?;

        let model_id = model_id.trim();
        if model_id.is_empty() {
            return Err("endpointModelIdMissing".into());
        }
        if model_id.chars().count() > crate::limits::ENDPOINT_MODEL_ID_MAX_CHARS {
            return Err("endpointModelIdTooLong".into());
        }

        let api_key = Self::normalize_api_key(api_key)?;

        Ok(Self {
            base_url,
            model_id: model_id.to_string(),
            api_key,
        })
    }

    /// Validate and normalize just the base URL and the key, for probing an
    /// endpoint before a model has been picked.
    pub fn parse_url(
        base_url: &str,
        api_key: Option<&str>,
    ) -> Result<(String, Option<String>), String> {
        let base_url = Self::normalize_base_url(base_url)?;
        let api_key = Self::normalize_api_key(api_key)?;
        Ok((base_url, api_key))
    }

    fn normalize_base_url(base_url: &str) -> Result<String, String> {
        let base_url = base_url.trim();
        if base_url.is_empty() {
            return Err("endpointBaseUrlMissing".into());
        }
        if base_url.chars().count() > crate::limits::ENDPOINT_URL_MAX_CHARS {
            return Err("endpointUrlTooLong".into());
        }
        let mut url = url::Url::parse(base_url).map_err(|_| "endpointUrlInvalid".to_string())?;
        match url.scheme() {
            "http" | "https" => {}
            _ => return Err("endpointSchemeUnsupported".into()),
        }
        if !url.username().is_empty() || url.password().is_some() {
            return Err("endpointUserInfoNotAllowed".into());
        }
        if !url.path_segments().map(|s| s.count() > 0).unwrap_or(false) {
            url.set_path("/");
        }
        // Documents on a Shelf are sent to this host. Loopback means the
        // chat never leaves the machine; anything else (a home server, a
        // NAS, a rented box) is a real choice the user makes, and the
        // Settings UI shows a warning for it.
        url.set_fragment(None);
        if url.query().is_none() {
            url.set_query(None);
        }
        let mut base_url = url.to_string();
        if base_url.ends_with('/') {
            base_url.pop();
        }
        if base_url.is_empty() {
            return Err("endpointUrlInvalid".into());
        }
        Ok(base_url)
    }

    fn normalize_api_key(api_key: Option<&str>) -> Result<Option<String>, String> {
        let api_key = api_key
            .map(|key| key.trim().to_string())
            .filter(|key| !key.is_empty());
        if let Some(key) = &api_key {
            if key.chars().count() > crate::limits::ENDPOINT_API_KEY_MAX_CHARS {
                return Err("endpointApiKeyTooLong".into());
            }
        }
        Ok(api_key)
    }
}

impl EndpointConfig {
    /// True when the base URL points at this machine, so chat answers and
    /// Shelf documents never touch the network.
    pub fn is_loopback(&self) -> bool {
        let url = url::Url::parse(&self.base_url)
            .unwrap_or_else(|_| url::Url::parse("http://invalid.invalid").expect("static url"));
        matches!(
            url.host_str().unwrap_or("").trim_end_matches('.'),
            "localhost" | "127.0.0.1" | "::1" | "[::1]"
        )
    }
}

impl ActiveModel {
    /// True when Chat talks to an external server instead of the local
    /// llama-server process.
    pub fn is_external(&self) -> bool {
        self.endpoint.is_some()
    }
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

    fn external_model() -> ActiveModel {
        ActiveModel {
            file: String::new(),
            name: "Gemma 4 12B (LM Studio)".into(),
            source: "external".into(),
            reference: "LM Studio".into(),
            license: None,
            size_bytes: 0,
            endpoint: Some(EndpointConfig {
                base_url: "http://127.0.0.1:1234/v1".into(),
                model_id: "gemma-4-12b".into(),
                api_key: None,
            }),
        }
    }

    #[test]
    fn external_model_roundtrips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let settings = Settings {
            active_model: Some(external_model()),
            ..Default::default()
        };
        settings.save(&path).unwrap();
        let loaded = Settings::load(&path);
        let model = loaded.active_model.expect("active model");
        assert!(model.is_external());
        let endpoint = model.endpoint.unwrap();
        assert_eq!(endpoint.base_url, "http://127.0.0.1:1234/v1");
        assert_eq!(endpoint.model_id, "gemma-4-12b");
        assert_eq!(endpoint.api_key, None);
    }

    #[test]
    fn settings_without_endpoint_stay_compatible() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        // An older Rebost settings.json: no endpoint key at all.
        std::fs::write(
            &path,
            serde_json::json!({
                "onboardingDone": true,
                "activeModel": {
                    "file": "gemma.gguf",
                    "name": "Gemma 4 12B",
                    "source": "huggingface",
                    "reference": "google/gemma-4-12b",
                    "sizeBytes": 8_000_000_000u64
                }
            })
            .to_string(),
        )
        .unwrap();
        let loaded = Settings::load(&path);
        let model = loaded.active_model.expect("active model");
        assert!(!model.is_external());
        assert_eq!(model.file, "gemma.gguf");
    }

    #[test]
    fn external_model_json_uses_camel_case() {
        let json = serde_json::to_value(external_model()).unwrap();
        let endpoint = json["endpoint"].as_object().unwrap();
        assert_eq!(
            endpoint["baseUrl"].as_str(),
            Some("http://127.0.0.1:1234/v1")
        );
        assert_eq!(endpoint["modelId"].as_str(), Some("gemma-4-12b"));
        assert!(!endpoint.get("apiKey").is_some());
    }

    #[test]
    fn endpoint_parse_normalizes_url() {
        let endpoint = EndpointConfig::parse("http://127.0.0.1:1234/v1/", "gemma-4-12b", None)
            .expect("valid endpoint");
        assert_eq!(endpoint.base_url, "http://127.0.0.1:1234/v1");
        assert_eq!(endpoint.model_id, "gemma-4-12b");

        // A bare host gets no path instead of a broken one.
        let bare = EndpointConfig::parse("http://127.0.0.1:11434", "llama3", None)
            .expect("bare host valid");
        assert_eq!(bare.base_url, "http://127.0.0.1:11434");
    }

    #[test]
    fn endpoint_parse_accepts_localhost_name() {
        let endpoint =
            EndpointConfig::parse("http://localhost:1234/v1", "m", None).expect("localhost valid");
        assert_eq!(endpoint.base_url, "http://localhost:1234/v1");
    }

    #[test]
    fn endpoint_parse_keeps_key_and_trims() {
        let endpoint = EndpointConfig::parse(
            "  http://127.0.0.1:1234/v1  ",
            "  gemma-4-12b  ",
            Some("  sk-abc  "),
        )
        .expect("valid endpoint");
        assert_eq!(endpoint.api_key.as_deref(), Some("sk-abc"));
        assert_eq!(endpoint.base_url, "http://127.0.0.1:1234/v1");
        assert_eq!(endpoint.model_id, "gemma-4-12b");
    }

    #[test]
    fn endpoint_parse_accepts_lan_hosts_for_a_second_machine() {
        // The model may live on another machine in the home network; the
        // Settings UI warns instead of refusing.
        let lan =
            EndpointConfig::parse("http://192.168.1.10:1234", "m", None).expect("lan host valid");
        assert!(!lan.is_loopback());
        let hostname =
            EndpointConfig::parse("http://gpu-box.local:1234", "m", None).expect("mdns host valid");
        assert!(!hostname.is_loopback());
        let public =
            EndpointConfig::parse("https://example.com/v1", "m", None).expect("https host valid");
        assert!(!public.is_loopback());
    }

    #[test]
    fn endpoint_parse_flags_loopback() {
        for host in ["http://127.0.0.1:1234/v1", "http://localhost:11434"] {
            assert!(EndpointConfig::parse(host, "m", None)
                .expect("loopback valid")
                .is_loopback());
        }
    }

    #[test]
    fn endpoint_parse_rejects_bad_schemes_and_urls() {
        assert_eq!(
            EndpointConfig::parse("ftp://127.0.0.1", "m", None).unwrap_err(),
            "endpointSchemeUnsupported"
        );
        assert_eq!(
            EndpointConfig::parse("not a url", "m", None).unwrap_err(),
            "endpointUrlInvalid"
        );
        assert_eq!(
            EndpointConfig::parse("http://user:pass@127.0.0.1:1234", "m", None).unwrap_err(),
            "endpointUserInfoNotAllowed"
        );
        assert_eq!(
            EndpointConfig::parse("", "m", None).unwrap_err(),
            "endpointBaseUrlMissing"
        );
    }

    #[test]
    fn endpoint_parse_rejects_missing_model_id() {
        assert_eq!(
            EndpointConfig::parse("http://127.0.0.1:1234", "   ", None).unwrap_err(),
            "endpointModelIdMissing"
        );
    }

    #[test]
    fn endpoint_parse_rejects_long_values() {
        let long = "x".repeat(crate::limits::ENDPOINT_URL_MAX_CHARS + 1);
        assert_eq!(
            EndpointConfig::parse(&long, "m", None).unwrap_err(),
            "endpointUrlTooLong"
        );
        let long_model = "m".repeat(crate::limits::ENDPOINT_MODEL_ID_MAX_CHARS + 1);
        assert_eq!(
            EndpointConfig::parse("http://127.0.0.1:1234", &long_model, None).unwrap_err(),
            "endpointModelIdTooLong"
        );
        let long_key = "k".repeat(crate::limits::ENDPOINT_API_KEY_MAX_CHARS + 1);
        assert_eq!(
            EndpointConfig::parse("http://127.0.0.1:1234", "m", Some(&long_key)).unwrap_err(),
            "endpointApiKeyTooLong"
        );
    }

    #[test]
    fn endpoint_parse_drops_empty_key() {
        let endpoint = EndpointConfig::parse("http://127.0.0.1:1234", "m", Some("   "))
            .expect("valid endpoint");
        assert_eq!(endpoint.api_key, None);
    }
}
