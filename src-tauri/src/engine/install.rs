//! Model download + switch-over.
//!
//! The previous AI stays usable while the new file downloads. Its file is
//! removed only after the new process is Ready. A failed start rolls back.

use anyhow::{anyhow, Result};
use serde_json::json;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use super::download;
use super::models;
use super::{Engine, EngineState};
use crate::settings::{ActiveModel, BenchmarkResult};

struct PreviousAi {
    model: ActiveModel,
    benchmark: Option<BenchmarkResult>,
    context_budget_chars: Option<usize>,
}

impl Engine {
    pub fn cancel_download(&self, id: &str) {
        if let Some(control) = crate::core::mutex_lock(&self.downloads).get(id) {
            control.request_cancel();
        }
    }

    pub fn skip_download_verify(&self, id: &str) {
        if let Some(control) = crate::core::mutex_lock(&self.downloads).get(id) {
            control.request_skip_verify();
        }
    }

    pub fn cancel_all_downloads(&self) {
        for control in crate::core::mutex_lock(&self.downloads).values() {
            control.request_cancel();
        }
    }

    /// Download and switch to a model. Chat keeps the previous AI until the
    /// new process is Ready; only then is the old file removed.
    pub async fn install_model(
        self: &Arc<Self>,
        source: &str,
        reference: &str,
        display_name: &str,
        license: Option<String>,
    ) -> Result<()> {
        let source = models::normalize_source(source)?;
        models::validate_reference(source, reference)?;

        let ticket = download::DownloadTicket {
            kind: "model",
            id: format!("model:{reference}"),
            name: display_name.to_string(),
        };
        let control = download::DownloadControl::new();
        crate::core::mutex_lock(&self.downloads).insert(ticket.id.clone(), control.clone());
        let had_model = self.active_model().is_some();
        if !had_model {
            self.set_status(EngineState::Downloading, Some(display_name.to_string()));
        }
        self.ctx.events.emit(
            "larra://download",
            json!({
                "kind": "model",
                "id": ticket.id,
                "name": display_name,
                "received": 0,
                "total": null,
                "done": false,
            }),
        );

        let result = self
            .download_and_switch(&ticket, &control, source, reference, display_name, license)
            .await;
        crate::core::mutex_lock(&self.downloads).remove(&ticket.id);
        if result.is_err() && self.active_model().is_none() {
            self.set_status(EngineState::NoModel, None);
        }
        result
    }

    async fn download_and_switch(
        self: &Arc<Self>,
        ticket: &download::DownloadTicket,
        control: &download::DownloadControl,
        source: &str,
        reference: &str,
        display_name: &str,
        license: Option<String>,
    ) -> Result<()> {
        let resolved = models::resolve_download(&self.download_client, source, reference).await?;
        let sha256 = resolved
            .sha256
            .as_deref()
            .ok_or_else(|| anyhow!("model file has no SHA-256 checksum; refusing to install"))?;
        let requested = models::safe_model_file_name(&resolved.file_name)?;
        let previous = snapshot_previous(self);
        let file_name =
            install_file_name(&requested, previous.as_ref().map(|p| p.model.file.as_str()));
        let dest = self.ctx.paths.models_dir().join(&file_name);

        download::download(
            &self.download_client,
            &resolved.url,
            &dest,
            ticket,
            Some(sha256),
            resolved.size,
            &self.ctx.events.clone(),
            control,
        )
        .await?;

        if let Err(error) = super::gguf::require_engine_compatible(&dest) {
            let _ = std::fs::remove_file(&dest);
            return Err(error);
        }

        if previous.is_some() {
            self.wait_until_chat_idle().await;
        }

        {
            let mut settings = crate::core::write_lock(&self.ctx.settings);
            settings.active_model = Some(ActiveModel {
                file: file_name.clone(),
                name: display_name.to_string(),
                source: source.to_string(),
                reference: reference.to_string(),
                license,
                size_bytes: resolved.size.unwrap_or(0),
            });
            settings.benchmark = None;
            settings.context_budget_chars = None;
        }
        self.ctx.save_settings();

        match self.ensure_ready().await {
            Ok(_) => {
                if let Some(old) = previous_file_to_remove(
                    previous.as_ref().map(|p| p.model.file.as_str()),
                    &file_name,
                ) {
                    let _ = std::fs::remove_file(self.ctx.paths.models_dir().join(old));
                }
                Ok(())
            }
            Err(error) => {
                log::error!("engine warmup after install failed: {error:#}");
                if let Some(previous) = previous {
                    restore_previous(self, previous);
                    if let Err(restart) = self.ensure_ready().await {
                        log::error!("could not restore previous AI: {restart:#}");
                    }
                    Err(anyhow!("switch-failed"))
                } else {
                    clear_failed_first_install(self);
                    Err(anyhow!("warmup-failed"))
                }
            }
        }
    }

    async fn wait_until_chat_idle(&self) {
        // A long answer can run well past a few seconds. Switching AI here
        // would kill llama-server under the stream.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30 * 60);
        loop {
            if self.generation_in_flight.load(Ordering::Relaxed) == 0 {
                tokio::time::sleep(Duration::from_millis(200)).await;
                if self.generation_in_flight.load(Ordering::Relaxed) == 0 {
                    return;
                }
            }
            if tokio::time::Instant::now() >= deadline {
                log::warn!("timed out waiting for Chat to finish before switching AI");
                return;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }
}

fn snapshot_previous(engine: &Engine) -> Option<PreviousAi> {
    let settings = crate::core::read_lock(&engine.ctx.settings);
    Some(PreviousAi {
        model: settings.active_model.clone()?,
        benchmark: settings.benchmark.clone(),
        context_budget_chars: settings.context_budget_chars,
    })
}

fn restore_previous(engine: &Engine, previous: PreviousAi) {
    {
        let mut settings = crate::core::write_lock(&engine.ctx.settings);
        settings.active_model = Some(previous.model);
        settings.benchmark = previous.benchmark;
        settings.context_budget_chars = previous.context_budget_chars;
    }
    engine.ctx.save_settings();
}

/// First-install warmup failed: keep the file, forget it as the active AI
/// so the next launch does not retry a load that already died.
fn clear_failed_first_install(engine: &Engine) {
    {
        let mut settings = crate::core::write_lock(&engine.ctx.settings);
        settings.active_model = None;
        settings.benchmark = None;
        settings.context_budget_chars = None;
    }
    engine.ctx.save_settings();
    engine.set_status(EngineState::NoModel, None);
}

/// If the live AI already uses this file name, write beside it so the
/// running process keeps the old bytes until the new one is Ready.
fn install_file_name(requested: &str, live_file: Option<&str>) -> String {
    if live_file == Some(requested) {
        sibling_install_name(requested)
    } else {
        requested.to_string()
    }
}

fn sibling_install_name(file_name: &str) -> String {
    let stem = file_name
        .strip_suffix(".gguf")
        .or_else(|| file_name.strip_suffix(".GGUF"))
        .unwrap_or(file_name);
    format!("{stem}.next.gguf")
}

fn previous_file_to_remove(previous: Option<&str>, new_file: &str) -> Option<String> {
    previous
        .filter(|file| *file != new_file)
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_file_name_writes_beside_the_live_file() {
        assert_eq!(
            install_file_name("gemma.gguf", Some("gemma.gguf")),
            "gemma.next.gguf"
        );
        assert_eq!(
            install_file_name("gemma.gguf", Some("other.gguf")),
            "gemma.gguf"
        );
        assert_eq!(install_file_name("gemma.gguf", None), "gemma.gguf");
    }

    #[test]
    fn previous_file_stays_until_the_new_name_differs() {
        assert_eq!(
            previous_file_to_remove(Some("old.gguf"), "new.gguf").as_deref(),
            Some("old.gguf")
        );
        assert_eq!(
            previous_file_to_remove(Some("same.gguf"), "same.gguf"),
            None
        );
        assert_eq!(previous_file_to_remove(None, "new.gguf"), None);
    }
}
