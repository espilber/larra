//! UI locale: system language by default, or a language chosen in Settings.

use serde::{Deserialize, Deserializer, Serialize};
use tauri::AppHandle;

/// Languages Larra ships catalogs for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AppLocale {
    En,
    Es,
    Ca,
    Pt,
    Fr,
    Ja,
    De,
    It,
    Sv,
    Nb,
    Nl,
    Cs,
    El,
    Da,
    Fi,
}

impl AppLocale {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::En => "en",
            Self::Es => "es",
            Self::Ca => "ca",
            Self::Pt => "pt",
            Self::Fr => "fr",
            Self::Ja => "ja",
            Self::De => "de",
            Self::It => "it",
            Self::Sv => "sv",
            Self::Nb => "nb",
            Self::Nl => "nl",
            Self::Cs => "cs",
            Self::El => "el",
            Self::Da => "da",
            Self::Fi => "fi",
        }
    }
}

/// Preference stored in settings.json.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum UiLocalePref {
    #[default]
    System,
    En,
    Es,
    Ca,
    Pt,
    Fr,
    Ja,
    De,
    It,
    Sv,
    Nb,
    Nl,
    Cs,
    El,
    Da,
    Fi,
}

impl<'de> Deserialize<'de> for UiLocalePref {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Ok(match raw.as_str() {
            "en" => Self::En,
            "es" => Self::Es,
            "ca" => Self::Ca,
            "pt" => Self::Pt,
            "fr" => Self::Fr,
            "ja" => Self::Ja,
            "de" => Self::De,
            "it" => Self::It,
            "sv" => Self::Sv,
            "nb" => Self::Nb,
            "nl" => Self::Nl,
            "cs" => Self::Cs,
            "el" => Self::El,
            "da" => Self::Da,
            "fi" => Self::Fi,
            _ => Self::System,
        })
    }
}

impl UiLocalePref {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::En => "en",
            Self::Es => "es",
            Self::Ca => "ca",
            Self::Pt => "pt",
            Self::Fr => "fr",
            Self::Ja => "ja",
            Self::De => "de",
            Self::It => "it",
            Self::Sv => "sv",
            Self::Nb => "nb",
            Self::Nl => "nl",
            Self::Cs => "cs",
            Self::El => "el",
            Self::Da => "da",
            Self::Fi => "fi",
        }
    }
}

/// Match a BCP-47 tag to a shipped catalog.
pub fn negotiate(tag: &str) -> AppLocale {
    let lower = tag.trim().replace('_', "-").to_ascii_lowercase();
    let lang = lower.split('-').next().unwrap_or("");
    match lang {
        "es" => AppLocale::Es,
        "ca" => AppLocale::Ca,
        "en" => AppLocale::En,
        "pt" => AppLocale::Pt,
        "fr" => AppLocale::Fr,
        "ja" => AppLocale::Ja,
        "de" => AppLocale::De,
        "it" => AppLocale::It,
        "sv" => AppLocale::Sv,
        "nb" | "no" | "nn" => AppLocale::Nb,
        "nl" => AppLocale::Nl,
        "cs" => AppLocale::Cs,
        "el" => AppLocale::El,
        "da" => AppLocale::Da,
        "fi" => AppLocale::Fi,
        _ => AppLocale::En,
    }
}

pub fn resolve(pref: UiLocalePref, system_tag: Option<&str>) -> AppLocale {
    match pref {
        UiLocalePref::En => AppLocale::En,
        UiLocalePref::Es => AppLocale::Es,
        UiLocalePref::Ca => AppLocale::Ca,
        UiLocalePref::Pt => AppLocale::Pt,
        UiLocalePref::Fr => AppLocale::Fr,
        UiLocalePref::Ja => AppLocale::Ja,
        UiLocalePref::De => AppLocale::De,
        UiLocalePref::It => AppLocale::It,
        UiLocalePref::Sv => AppLocale::Sv,
        UiLocalePref::Nb => AppLocale::Nb,
        UiLocalePref::Nl => AppLocale::Nl,
        UiLocalePref::Cs => AppLocale::Cs,
        UiLocalePref::El => AppLocale::El,
        UiLocalePref::Da => AppLocale::Da,
        UiLocalePref::Fi => AppLocale::Fi,
        UiLocalePref::System => negotiate(system_tag.unwrap_or("en")),
    }
}

pub fn system_locale_tag() -> Option<String> {
    tauri_plugin_os::locale()
}

/// Set rust-i18n to the resolved catalog. Returns that catalog's code.
pub fn apply(pref: UiLocalePref) -> AppLocale {
    let resolved = resolve(pref, system_locale_tag().as_deref());
    rust_i18n::set_locale(resolved.as_str());
    resolved
}

pub fn rebuild_menu(app: &AppHandle) -> tauri::Result<()> {
    app.set_menu(crate::menu::build_menu(app)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn negotiate_matches_language_subtag() {
        assert_eq!(negotiate("es-MX"), AppLocale::Es);
        assert_eq!(negotiate("es"), AppLocale::Es);
        assert_eq!(negotiate("ca-ES"), AppLocale::Ca);
        assert_eq!(negotiate("ca"), AppLocale::Ca);
        assert_eq!(negotiate("en-GB"), AppLocale::En);
        assert_eq!(negotiate("fr-FR"), AppLocale::Fr);
        assert_eq!(negotiate("pt-BR"), AppLocale::Pt);
        assert_eq!(negotiate("nb-NO"), AppLocale::Nb);
        assert_eq!(negotiate("no-NO"), AppLocale::Nb);
        assert_eq!(negotiate("ja-JP"), AppLocale::Ja);
        assert_eq!(negotiate("zh-Hans-CN"), AppLocale::En);
    }

    #[test]
    fn system_pref_uses_os_tag() {
        assert_eq!(resolve(UiLocalePref::System, Some("ca-ES")), AppLocale::Ca);
        assert_eq!(resolve(UiLocalePref::System, None), AppLocale::En);
        assert_eq!(resolve(UiLocalePref::Es, Some("en-US")), AppLocale::Es);
        assert_eq!(resolve(UiLocalePref::De, Some("en-US")), AppLocale::De);
    }

    #[test]
    fn unknown_pref_json_becomes_system() {
        let pref: UiLocalePref = serde_json::from_str("\"klingon\"").unwrap();
        assert_eq!(pref, UiLocalePref::System);
        let pref: UiLocalePref = serde_json::from_str("\"system\"").unwrap();
        assert_eq!(pref, UiLocalePref::System);
        let pref: UiLocalePref = serde_json::from_str("\"ja\"").unwrap();
        assert_eq!(pref, UiLocalePref::Ja);
    }

    #[test]
    fn view_labels_follow_catalog_locale() {
        assert_eq!(rust_i18n::t!("menu.chat", locale = "en"), "Chat");
        assert_eq!(rust_i18n::t!("menu.chat", locale = "es"), "Chats");
        assert_eq!(rust_i18n::t!("menu.chat", locale = "ca"), "Xat");
        assert_eq!(rust_i18n::t!("nav.shelves", locale = "es"), "Estantes");
        assert_eq!(rust_i18n::t!("nav.recipes", locale = "ca"), "Receptes");
        assert_eq!(rust_i18n::t!("menu.chat", locale = "fr"), "Discussions");
        assert_eq!(rust_i18n::t!("nav.shelves", locale = "de"), "Regale");
    }
}
