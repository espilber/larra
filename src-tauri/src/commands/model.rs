//! Engine status and model install commands.

use serde::Serialize;
use serde_json::json;
use std::sync::Arc;
use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;

use super::{friendly, CmdResult};
use crate::core::Ctx;
use crate::engine::models::{self, MachineProfile, ModelSearchResult, Recommendation};
use crate::engine::{Engine, EngineStatus};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MachineView {
    pub profile: MachineProfile,
    pub recommendation: Recommendation,
    pub alternatives: Vec<Recommendation>,
    /// Catalog picks that fit and are not the installed model (max two).
    pub suggestions: Vec<Recommendation>,
}

/// Current llama-server state.
#[tauri::command]
pub fn engine_status(engine: State<'_, Arc<Engine>>) -> EngineStatus {
    engine.status()
}

/// Hardware profile plus catalog recommendations.
#[tauri::command]
pub fn machine_profile(ctx: State<'_, Arc<Ctx>>) -> MachineView {
    let profile = MachineProfile::detect(ctx.paths.base());
    let recommendation = models::recommend(&profile);
    let alternatives = models::smaller_alternatives(&profile, 2);
    let installed = crate::core::read_lock(&ctx.settings)
        .active_model
        .as_ref()
        .map(|model| model.reference.clone());
    let suggestions = models::uninstalled_suggestions(&profile, installed.as_deref(), 2);
    MachineView {
        profile,
        recommendation,
        alternatives,
        suggestions,
    }
}

/// Search Hugging Face and Ollama catalogs for GGUF models.
#[tauri::command]
pub async fn models_search(
    ctx: State<'_, Arc<Ctx>>,
    engine: State<'_, Arc<Engine>>,
    query: String,
) -> CmdResult<Vec<ModelSearchResult>> {
    let profile = MachineProfile::detect(ctx.paths.base());
    models::search_models(engine.catalog_client(), &query, &profile)
        .await
        .map_err(|_| {
            "Couldn't reach the AI catalogs. Check your connection and try again.".to_string()
        })
}

/// Ask an external OpenAI-compatible endpoint which models it serves, for
/// the Settings picker. Sends no documents — only the probe request.
#[tauri::command]
pub async fn external_endpoint_probe(
    engine: State<'_, Arc<Engine>>,
    base_url: String,
    api_key: Option<String>,
) -> CmdResult<Vec<crate::engine::endpoint::EndpointModel>> {
    let (base_url, api_key) =
        crate::settings::EndpointConfig::parse_url(&base_url, api_key.as_deref())?;
    crate::engine::endpoint::probe_models(&engine.client, &base_url, api_key.as_deref())
        .await
        .map_err(|error| format!("{error:#}"))
}

/// Make an external OpenAI-compatible endpoint the active AI. Chat then
/// talks to that server instead of the bundled llama.cpp engine.
#[tauri::command]
pub async fn external_model_set(
    ctx: State<'_, Arc<Ctx>>,
    engine: State<'_, Arc<Engine>>,
    base_url: String,
    model_id: String,
    api_key: Option<String>,
    display_name: Option<String>,
) -> CmdResult<()> {
    let config = crate::settings::EndpointConfig::parse(&base_url, &model_id, api_key.as_deref())?;
    let name = display_name
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| config.model_id.clone());
    let name = crate::limits::clip_chars(&name, crate::limits::ENDPOINT_MODEL_NAME_MAX_CHARS);
    ctx.update_settings(|settings| {
        settings.active_model = Some(crate::settings::ActiveModel {
            file: String::new(),
            name,
            source: "external".into(),
            reference: config.base_url.clone(),
            license: None,
            size_bytes: 0,
            endpoint: Some(config),
        });
        // A benchmark measured llama.cpp on this machine; it says nothing
        // about a remote server, so drop it and let the conservative
        // default budget apply.
        settings.benchmark = None;
        settings.context_budget_chars = None;
    })
    .map_err(|error| format!("{error:#}"))?;
    // Drop whatever engine state the previous model had; the next chat
    // starts fresh against the endpoint.
    engine.stop().await;
    Ok(())
}

/// Stop using the external endpoint. The active AI is unset; a local one
/// can be picked again from Settings or Onboarding.
#[tauri::command]
pub async fn external_model_clear(
    ctx: State<'_, Arc<Ctx>>,
    engine: State<'_, Arc<Engine>>,
) -> CmdResult<()> {
    ctx.update_settings(|settings| {
        settings.active_model = None;
    })
    .map_err(|error| format!("{error:#}"))?;
    engine.stop().await;
    Ok(())
}

/// Open the public Hugging Face or Ollama page for a catalog AI.
#[tauri::command]
pub fn open_model_page(app: AppHandle, source: String, reference: String) -> CmdResult<()> {
    let url = models::catalog_page_url(&source, &reference)
        .map_err(|_| "That page isn't available.".to_string())?;
    app.opener().open_url(url, None::<String>).map_err(friendly)
}

/// Install a model (the recommended one or a chosen one). Progress arrives
/// via `larra://download` events.
#[tauri::command]
pub fn model_install(
    engine: State<'_, Arc<Engine>>,
    source: String,
    reference: String,
    name: String,
    license: Option<String>,
) -> CmdResult<()> {
    models::normalize_source(&source)
        .and_then(|source| models::validate_reference(source, &reference))
        .map_err(friendly)?;
    let engine = engine.inner().clone();
    tauri::async_runtime::spawn(async move {
        if let Err(error) = engine
            .install_model(&source, &reference, &name, license)
            .await
        {
            log::error!("model install failed: {error:#}");
            let message = error.to_string();
            let error = if message.contains("stalled") {
                "stalled"
            } else if message.contains("SHA-256") || message.contains("verification") {
                "verification failed"
            } else if message.contains("incompatible-format") {
                "incompatible-format"
            } else if message.contains("invalid") || message.contains("unsupported model") {
                "That AI source isn't allowed."
            } else if message.contains("switch-failed") {
                "switch-failed"
            } else if message.contains("warmup-failed") {
                "warmup-failed"
            } else {
                "The download didn't finish. Try again."
            };
            engine.ctx().events.emit(
                "larra://download",
                json!({
                    "kind": "model",
                    "id": format!("model:{reference}"),
                    "name": name,
                    "done": false,
                    "error": error,
                }),
            );
        }
    });
    Ok(())
}

/// Cancel an in-flight model download.
#[tauri::command]
pub fn download_cancel(engine: State<'_, Arc<Engine>>, id: String) {
    engine.cancel_download(&id);
}

/// Skip SHA-256 check for a model download that stalled on verify.
#[tauri::command]
pub fn download_skip_verify(engine: State<'_, Arc<Engine>>, id: String) {
    if id.starts_with("model:") {
        engine.skip_download_verify(&id);
    }
}

/// Recalibrate the installed AI on the active engine.
#[tauri::command]
pub async fn engine_remeasure(engine: State<'_, Arc<Engine>>) -> CmdResult<()> {
    engine.inner().clone().remeasure().await.map_err(friendly)
}
