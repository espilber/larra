//! Settings, privacy helpers, and diagnostics.

use serde::Serialize;
use std::sync::Arc;
use tauri::{AppHandle, State};

use tauri_plugin_opener::OpenerExt;

use super::{friendly, CmdResult};
use crate::core::Ctx;
use crate::engine::models::MachineProfile;
use crate::engine::{Engine, EngineStatus};
use crate::i18n::{AppLocale, UiLocalePref};
use crate::paths::Paths;
use crate::settings::{ActiveModel, TextSize};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsView {
    pub house_rules: String,
    pub onboarding_done: bool,
    pub active_model: Option<ActiveModel>,
    pub allow_online_research: bool,
    pub text_size: TextSize,
    pub ui_locale: UiLocalePref,
    pub resolved_locale: AppLocale,
}

fn settings_view(ctx: &Ctx) -> SettingsView {
    let settings = crate::core::read_lock(&ctx.settings);
    let resolved_locale = crate::i18n::resolve(
        settings.ui_locale,
        crate::i18n::system_locale_tag().as_deref(),
    );
    SettingsView {
        house_rules: settings.house_rules.clone(),
        onboarding_done: settings.onboarding_done,
        active_model: settings.active_model.clone(),
        allow_online_research: settings.allow_online_research,
        text_size: settings.text_size,
        ui_locale: settings.ui_locale,
        resolved_locale,
    }
}

/// Return the settings the UI needs.
#[tauri::command]
pub fn settings_get(ctx: State<'_, Arc<Ctx>>) -> SettingsView {
    settings_view(&ctx)
}

/// Save House rules.
#[tauri::command]
pub fn settings_set_house_rules(ctx: State<'_, Arc<Ctx>>, text: String) -> CmdResult<()> {
    ctx.update_settings(|settings| {
        settings.house_rules =
            crate::limits::clip_chars(&text, crate::limits::HOUSE_RULES_MAX_CHARS)
    })
    .map_err(friendly)
}

/// Allow Chat to look things up on the public web from this computer.
#[tauri::command]
pub fn settings_set_allow_online_research(
    ctx: State<'_, Arc<Ctx>>,
    enabled: bool,
) -> CmdResult<()> {
    ctx.update_settings(|settings| settings.allow_online_research = enabled)
        .map_err(friendly)
}

/// Window type size: default, or one of the two larger steps.
#[tauri::command]
pub fn settings_set_text_size(ctx: State<'_, Arc<Ctx>>, size: TextSize) -> CmdResult<()> {
    ctx.update_settings(|settings| settings.text_size = size)
        .map_err(friendly)
}

/// Follow the computer, or pin a shipped UI catalog. Rebuilds menus.
#[tauri::command]
pub fn settings_set_ui_locale(
    app: AppHandle,
    ctx: State<'_, Arc<Ctx>>,
    locale: UiLocalePref,
) -> CmdResult<SettingsView> {
    ctx.update_settings(|settings| settings.ui_locale = locale)
        .map_err(friendly)?;
    crate::i18n::apply(locale);
    crate::i18n::rebuild_menu(&app).map_err(friendly)?;
    Ok(settings_view(&ctx))
}

/// Mark first-run as done.
#[tauri::command]
pub fn settings_finish_onboarding(ctx: State<'_, Arc<Ctx>>) -> CmdResult<()> {
    ctx.update_settings(|settings| settings.onboarding_done = true)
        .map_err(friendly)
}

pub(crate) fn require_reset_confirmation(confirmation: &str) -> CmdResult<()> {
    if confirmation.trim() != crate::reset::CONFIRMATION {
        return Err(rust_i18n::t!("errors.typeDelete").into());
    }
    Ok(())
}

/// Wipe app data on the next launch (same idea as `scripts/reset.sh`).
/// Requires typing DELETE. User Shelf files in `library/` stay.
#[tauri::command]
pub async fn settings_reset_workspace(
    app: AppHandle,
    ctx: State<'_, Arc<Ctx>>,
    engine: State<'_, Arc<Engine>>,
    confirmation: String,
) -> CmdResult<()> {
    require_reset_confirmation(&confirmation)?;
    crate::reset::mark_pending(ctx.paths.base()).map_err(friendly)?;
    crate::instance::mark_relaunch(ctx.paths.base()).map_err(friendly)?;
    engine.cancel_all_downloads();
    engine.stop().await;
    // `tauri dev` must exec this binary; request_restart() detaches from the CLI.
    #[cfg(debug_assertions)]
    {
        let _ = app;
        crate::reset::relaunch_current_exe();
    }
    #[cfg(not(debug_assertions))]
    {
        app.request_restart();
        Ok(())
    }
}

/// "Copy without personal information" — local replacement, no history.
#[tauri::command]
pub fn redact_text(ctx: State<'_, Arc<Ctx>>, text: String) -> String {
    ctx.pii.redact(&text)
}

/// True when the text contains recognized personal information.
#[tauri::command]
pub fn text_has_pii(ctx: State<'_, Arc<Ctx>>, text: String) -> bool {
    ctx.pii.contains_pii(&text)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Diagnostics {
    pub version: String,
    pub data_dir: String,
    pub engine_build: String,
    pub engine_state: EngineStatus,
    pub model: Option<ActiveModel>,
    pub index_records: u64,
    pub context_budget_chars: usize,
    pub benchmark: Option<crate::settings::BenchmarkResult>,
    pub machine: MachineProfile,
    /// Path only. The log itself can name local files; it is not sent to the UI.
    pub engine_log_path: String,
    pub engine_log_present: bool,
    pub supported_formats: Vec<String>,
}

/// Local diagnostics for Settings (paths and counts, not log bodies).
#[tauri::command]
pub fn diagnostics(
    app: AppHandle,
    ctx: State<'_, Arc<Ctx>>,
    engine: State<'_, Arc<Engine>>,
) -> Diagnostics {
    let settings = crate::core::read_lock(&ctx.settings).clone();
    let log_path = ctx.paths.engine_log_path();
    let mut formats: Vec<String> = crate::ingest::extract::supported_extensions()
        .iter()
        .cloned()
        .collect();
    formats.sort();
    Diagnostics {
        version: app.package_info().version.to_string(),
        data_dir: ctx.paths.base().to_string_lossy().to_string(),
        engine_build: crate::engine::ENGINE_RELEASE.to_string(),
        engine_state: engine.status(),
        model: settings.active_model.clone(),
        index_records: ctx.search.num_docs(),
        context_budget_chars: ctx.context_budget(),
        benchmark: settings.benchmark.clone(),
        machine: MachineProfile::detect(ctx.paths.base()),
        engine_log_path: log_path.to_string_lossy().to_string(),
        engine_log_present: log_path.is_file(),
        supported_formats: formats,
    }
}

pub(crate) fn require_engine_log(paths: &Paths) -> CmdResult<std::path::PathBuf> {
    let log_path = paths.engine_log_path();
    if !log_path.is_file() {
        return Err(rust_i18n::t!("errors.engineLogMissing").into());
    }
    Ok(log_path)
}

/// Open the engine log in the default app. Path is fixed; the UI never sends one.
#[tauri::command]
pub fn open_engine_log(app: AppHandle, ctx: State<'_, Arc<Ctx>>) -> CmdResult<()> {
    let log_path = require_engine_log(&ctx.paths)?;
    app.opener()
        .open_path(log_path.to_string_lossy().to_string(), None::<String>)
        .map_err(friendly)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reset_confirmation_must_be_delete() {
        assert_eq!(require_reset_confirmation("DELETE"), Ok(()));
        assert_eq!(require_reset_confirmation("  DELETE  "), Ok(()));
        assert_eq!(
            require_reset_confirmation("delete"),
            Err("Type DELETE to confirm.".into())
        );
        assert_eq!(
            require_reset_confirmation(""),
            Err("Type DELETE to confirm.".into())
        );
    }

    #[test]
    fn engine_log_open_requires_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path());
        paths.ensure().unwrap();
        assert_eq!(
            require_engine_log(&paths),
            Err("The engine log isn't on this computer yet.".into())
        );
        std::fs::write(paths.engine_log_path(), "ok").unwrap();
        assert_eq!(require_engine_log(&paths).unwrap(), paths.engine_log_path());
    }
}
