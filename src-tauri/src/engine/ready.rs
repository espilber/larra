//! Spawn llama-server and wait until `/health` succeeds.

use anyhow::{anyhow, Context, Result};
use std::path::Path;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::process::Command;

use super::catalog::MachineProfile;
use super::pin::{current_engine_pin, EnginePin};
use super::process::{
    engine_log_tail, force_kill_pid, free_port, kill_stale_llama_servers, pipe_to_log,
    write_server_pid,
};
use super::tune::{ModelHint, SpawnPlan};
use super::{Engine, EngineState, Inner, ENGINE_RELEASE};
use crate::settings::{ActiveModel, EndpointConfig};

const HEALTH_TIMEOUT: Duration = Duration::from_secs(240);
const STOPPED: &str = "stopped";

fn stopped_error() -> anyhow::Error {
    anyhow!(STOPPED)
}

fn is_stopped(error: &anyhow::Error) -> bool {
    error.to_string() == STOPPED
}

struct SpawnFailed {
    /// Process died, never started, or never became healthy. Callers may try
    /// the bundled pin. Port errors stay false — Vulkan will not help.
    try_bundled: bool,
    error: anyhow::Error,
}

fn should_fallback_to_bundled(
    fail: &SpawnFailed,
    used_fallback: bool,
    pin: &EnginePin,
    bundled: &EnginePin,
) -> bool {
    fail.try_bundled && !used_fallback && !std::ptr::eq(pin, bundled)
}

fn llama_server_args(model_path: &Path, port: u16, plan: &SpawnPlan) -> Vec<String> {
    let mut args = vec![
        "-m".into(),
        model_path.to_string_lossy().into_owned(),
        "--host".into(),
        "127.0.0.1".into(),
        "--port".into(),
        port.to_string(),
        "-c".into(),
        plan.context_tokens.to_string(),
        // One conversation at a time. Auto slot count (4) times 8k
        // context blew GPU memory on a 30B model, especially if a
        // previous llama-server was still holding the last load.
        "-np".into(),
        "1".into(),
        "-b".into(),
        plan.batch.to_string(),
        "-ub".into(),
        plan.ubatch.to_string(),
        "-ngl".into(),
        plan.gpu_layers.to_string(),
        "-fa".into(),
        plan.flash_attn.into(),
        "--cache-type-k".into(),
        plan.cache_type.into(),
        "--cache-type-v".into(),
        plan.cache_type.into(),
        "--jinja".into(),
        // Extract template-declared reasoning into reasoning_content
        // (DeepSeek-R1, Qwen3, …); inline-tag models are split
        // client-side, and the system prompt asks untagged reasoners
        // to tag their thinking.
        "--reasoning-format".into(),
        "auto".into(),
        "--no-webui".into(),
    ];
    if plan.no_mmap {
        args.push("--no-mmap".into());
    }
    args
}

impl Engine {
    fn model_path(&self, model: &ActiveModel) -> std::path::PathBuf {
        self.ctx.paths.models_dir().join(&model.file)
    }

    async fn health_ok(&self, port: u16) -> bool {
        let url = format!("http://127.0.0.1:{port}/health");
        matches!(
            self.client
                .get(&url)
                .timeout(Duration::from_secs(2))
                .send()
                .await,
            Ok(response) if response.status().is_success()
        )
    }

    /// `/health` only means weights loaded. OpenCL on Adreno can pass that
    /// and then hang or return nothing on the first token.
    async fn generation_probe_ok(&self, port: u16) -> bool {
        let url = format!("http://127.0.0.1:{port}/completion");
        let send = self
            .client
            .post(&url)
            .json(&serde_json::json!({
                "prompt": "Hi",
                "n_predict": 4,
                "cache_prompt": false,
            }))
            .send();
        let Ok(Ok(response)) = tokio::time::timeout(Duration::from_secs(60), send).await else {
            return false;
        };
        if !response.status().is_success() {
            return false;
        }
        let value: serde_json::Value = response.json().await.unwrap_or_default();
        let predicted = value["tokens_predicted"].as_i64().unwrap_or(0);
        let content = value["content"].as_str().unwrap_or("");
        predicted > 0 || !content.is_empty()
    }

    /// Bring the engine up if Chat needs it. Queued callers simply await.
    /// Returns the base URL for requests.
    pub async fn ensure_ready(self: &std::sync::Arc<Self>) -> Result<String> {
        self.ensure_ready_cancel(&AtomicBool::new(false)).await
    }

    /// Same as [`Self::ensure_ready`], but Stop aborts spawn and the health wait.
    pub async fn ensure_ready_cancel(
        self: &std::sync::Arc<Self>,
        cancel: &AtomicBool,
    ) -> Result<String> {
        if cancel.load(Ordering::Relaxed) {
            return Err(stopped_error());
        }
        let Some(model) = self.active_model() else {
            self.set_status(EngineState::NoModel, None);
            return Err(anyhow!("no AI model installed yet"));
        };
        if let Some(endpoint) = model.endpoint.clone() {
            return self.ensure_endpoint_ready(&endpoint, cancel).await;
        }
        let model_path = self.model_path(&model);
        if !model_path.exists() {
            self.set_status(
                EngineState::NoModel,
                Some("Larra can't find the AI on this computer.".into()),
            );
            return Err(anyhow!("model file missing"));
        }

        {
            let mut inner = tokio::select! {
                _ = crate::engine::wait_if_cancelled(cancel) => return Err(stopped_error()),
                guard = self.inner.lock() => guard,
            };
            let live = tokio::select! {
                biased;
                _ = crate::engine::wait_if_cancelled(cancel) => return Err(stopped_error()),
                live = self.live_url(&mut inner, &model) => live,
            };
            if let Some(url) = live {
                return Ok(url);
            }
        }

        let _start = tokio::select! {
            biased;
            _ = crate::engine::wait_if_cancelled(cancel) => {
                return Err(stopped_error());
            }
            guard = self.start_lock.lock() => guard,
        };
        if cancel.load(Ordering::Relaxed) {
            return Err(stopped_error());
        }
        {
            let mut inner = tokio::select! {
                _ = crate::engine::wait_if_cancelled(cancel) => return Err(stopped_error()),
                guard = self.inner.lock() => guard,
            };
            let live = tokio::select! {
                biased;
                _ = crate::engine::wait_if_cancelled(cancel) => return Err(stopped_error()),
                live = self.live_url(&mut inner, &model) => live,
            };
            if let Some(url) = live {
                return Ok(url);
            }
        }

        // Download llama.cpp without holding the process lock — otherwise the
        // first chat sits on "Warming up…" with no engine log for minutes.
        let (mut binary, mut pin) = tokio::select! {
            _ = crate::engine::wait_if_cancelled(cancel) => return Err(stopped_error()),
            result = self.ensure_binary() => result?,
        };
        if cancel.load(Ordering::Relaxed) {
            return Err(stopped_error());
        }
        let bundled = current_engine_pin()?;
        let data_dir = self.ctx.paths.base().to_path_buf();
        if let Err(error) =
            tokio::task::spawn_blocking(move || kill_stale_llama_servers(&data_dir)).await
        {
            log::warn!("kill stale llama-servers: {error}");
        }

        let mut used_fallback = false;
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err(stopped_error());
            }
            let mut inner = tokio::select! {
                _ = crate::engine::wait_if_cancelled(cancel) => return Err(stopped_error()),
                guard = self.inner.lock() => guard,
            };
            let live = tokio::select! {
                biased;
                _ = crate::engine::wait_if_cancelled(cancel) => return Err(stopped_error()),
                live = self.live_url(&mut inner, &model) => live,
            };
            if let Some(url) = live {
                return Ok(url);
            }

            match self
                .spawn_and_wait(&mut inner, &binary, pin, &model, &model_path, cancel)
                .await
            {
                Ok(url) => return Ok(url),
                Err(fail) if is_stopped(&fail.error) => return Err(fail.error),
                Err(fail) if should_fallback_to_bundled(&fail, used_fallback, pin, bundled) => {
                    drop(inner);
                    used_fallback = true;
                    self.skip_optional
                        .store(true, std::sync::atomic::Ordering::Relaxed);
                    log::warn!(
                        "optional {} engine failed ({:#}); falling back to bundled {}",
                        pin.accelerator,
                        fail.error,
                        bundled.accelerator
                    );
                    let data_dir = self.ctx.paths.base().to_path_buf();
                    if let Err(error) =
                        tokio::task::spawn_blocking(move || kill_stale_llama_servers(&data_dir))
                            .await
                    {
                        log::warn!("kill stale llama-servers: {error}");
                    }
                    binary = self.ensure_pin_binary(bundled, true).await?;
                    pin = bundled;
                }
                Err(fail) => {
                    let timeout = fail.error.to_string().contains("timeout");
                    self.set_status(
                        EngineState::Error,
                        Some(if timeout {
                            "Larra took too long to get ready. Try again.".into()
                        } else {
                            "Larra isn't ready yet. Try again in a moment.".into()
                        }),
                    );
                    return Err(fail.error);
                }
            }
        }
    }

    /// `GET /models` on an external endpoint — the list call every
    /// OpenAI-compatible server answers, used here as the health check.
    async fn endpoint_health_ok(&self, base_url: &str) -> bool {
        let url = format!("{base_url}/models");
        matches!(
            self.client
                .get(&url)
                .timeout(Duration::from_secs(2))
                .send()
                .await,
            Ok(response) if response.status().is_success()
        )
    }

    /// Bring up the external-endment mode: no llama-server child process,
    /// no engine download — just confirm the endpoint answers, then hand
    /// its base URL to the stream. Kills a leftover local child first, so
    /// switching a GGUF AI off releases its memory.
    async fn ensure_endpoint_ready(
        self: &std::sync::Arc<Self>,
        endpoint: &EndpointConfig,
        cancel: &AtomicBool,
    ) -> Result<String> {
        {
            let mut inner = tokio::select! {
                _ = crate::engine::wait_if_cancelled(cancel) => return Err(stopped_error()),
                guard = self.inner.lock() => guard,
            };
            let live = tokio::select! {
                biased;
                _ = crate::engine::wait_if_cancelled(cancel) => return Err(stopped_error()),
                live = self.live_endpoint_url(&mut inner, &endpoint.base_url) => live,
            };
            if let Some(url) = live {
                return Ok(url);
            }
        }

        self.set_status(EngineState::Starting, None);
        let started = std::time::Instant::now();
        let mut last_note = started;
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err(stopped_error());
            }
            if self.endpoint_health_ok(&endpoint.base_url).await {
                let mut inner = self.inner.lock().await;
                if let Some(mut child) = inner.child.take() {
                    let _ = child.kill().await;
                }
                inner.external_base_url = Some(endpoint.base_url.clone());
                inner.model_file = String::new();
                inner.flatten_tools = false;
                inner.tools_rejected = false;
                inner.benchmark_runtime = None;
                inner.chat_stall = super::stream::CHAT_STALL_TIMEOUT;
                *crate::core::write_lock(&self.ctx.runtime_plan) = None;
                let url = endpoint.base_url.clone();
                inner.reasoning = Some(self.load_reasoning_caps(&url).await);
                self.set_status(EngineState::Ready, None);
                log::info!("external endpoint ready: {url}");
                return Ok(url);
            }
            if started.elapsed() > HEALTH_TIMEOUT {
                self.set_status(
                    EngineState::Error,
                    Some(format!(
                        "The model server at {} didn't answer. Is it running?",
                        endpoint.base_url
                    )),
                );
                return Err(anyhow!(
                    "external endpoint not reachable at {}",
                    endpoint.base_url
                ));
            }
            if last_note.elapsed() >= Duration::from_secs(10) {
                let secs = started.elapsed().as_secs();
                log::info!("still waiting for the model server ({}s)", secs);
                last_note = std::time::Instant::now();
            }
            tokio::select! {
                _ = super::wait_if_cancelled(cancel) => {},
                _ = tokio::time::sleep(Duration::from_millis(400)) => {},
            }
        }
    }

    /// External variant of [`Self::live_url`]: a live endpoint is one whose
    /// base URL matches the active model and answers `/models`.
    async fn live_endpoint_url(&self, inner: &mut Inner, base_url: &str) -> Option<String> {
        let same = inner.external_base_url.as_deref() == Some(base_url);
        if same && self.endpoint_health_ok(base_url).await {
            if inner.reasoning.is_none() {
                inner.reasoning = Some(self.load_reasoning_caps(base_url).await);
            }
            self.set_status(EngineState::Ready, None);
            Some(base_url.to_string())
        } else {
            self.clear_engine_state(inner).await;
            None
        }
    }

    /// Drop whatever engine state a previous model had (spawned llama-server
    /// or an external endpoint) when the active model changed.
    async fn clear_engine_state(&self, inner: &mut Inner) {
        if let Some(mut child) = inner.child.take() {
            let _ = child.kill().await;
        }
        inner.external_base_url = None;
        inner.reasoning = None;
        *crate::core::write_lock(&self.ctx.runtime_plan) = None;
    }

    async fn spawn_and_wait(
        &self,
        inner: &mut Inner,
        binary: &Path,
        pin: &EnginePin,
        model: &ActiveModel,
        model_path: &Path,
        cancel: &AtomicBool,
    ) -> Result<String, SpawnFailed> {
        let port = match free_port() {
            Ok(port) => port,
            Err(error) => {
                return Err(SpawnFailed {
                    try_bundled: false,
                    error,
                })
            }
        };
        let profile = MachineProfile::detect(self.ctx.paths.base());
        let hint = ModelHint::from_active(model, &self.ctx.paths.models_dir());
        let plan = SpawnPlan::for_model(&profile, pin, Some(&hint));
        if pin.accelerator == "Vulkan" && super::gpu::windows_host_is_arm64() {
            log::warn!(
                "this Windows copy is running on ARM; using the CPU path so Chat can answer"
            );
        }
        log::info!(
            "starting llama-server {} {} with {} (-c {} -b {} -ub {} -ngl {} -fa {} --cache-type {}{})",
            ENGINE_RELEASE,
            pin.accelerator,
            model.file,
            plan.context_tokens,
            plan.batch,
            plan.ubatch,
            plan.gpu_layers,
            plan.flash_attn,
            plan.cache_type,
            if plan.no_mmap { " --no-mmap" } else { "" }
        );
        self.set_status(EngineState::Starting, None);

        let log_path = self.ctx.paths.engine_log_path();
        let _ = std::fs::create_dir_all(self.ctx.paths.logs_dir());
        super::process::trim_log_file(&log_path, super::process::ENGINE_LOG_MAX_LINES);

        let mut command = Command::new(binary);
        if let Some(home) = binary.parent() {
            command.current_dir(home);
        }
        command
            .args(llama_server_args(model_path, port, &plan))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(unix)]
        {
            command.process_group(0);
        }
        #[cfg(windows)]
        {
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            command.creation_flags(CREATE_NO_WINDOW);
        }

        let mut child = match command.spawn().context("spawn llama-server") {
            Ok(child) => child,
            Err(error) => {
                return Err(SpawnFailed {
                    try_bundled: true,
                    error,
                })
            }
        };
        if let Some(pid) = child.id() {
            write_server_pid(self.ctx.paths.base(), pid);
        }

        if let Some(stdout) = child.stdout.take() {
            let log_path = log_path.clone();
            tokio::spawn(pipe_to_log(stdout, log_path));
        }
        if let Some(stderr) = child.stderr.take() {
            let log_path = log_path.clone();
            tokio::spawn(pipe_to_log(stderr, log_path));
        }

        let started = std::time::Instant::now();
        let mut last_note = started;
        loop {
            if cancel.load(Ordering::Relaxed) {
                if let Some(pid) = child.id() {
                    force_kill_pid(pid);
                }
                let _ = child.kill().await;
                return Err(SpawnFailed {
                    try_bundled: false,
                    error: stopped_error(),
                });
            }
            if let Ok(Some(status)) = child.try_wait() {
                let tail = engine_log_tail(&log_path);
                log::error!("llama-server exited early ({status}); log tail:\n{tail}");
                return Err(SpawnFailed {
                    try_bundled: true,
                    error: anyhow!("llama-server exited early ({status})"),
                });
            }
            let healthy = tokio::select! {
                biased;
                _ = super::wait_if_cancelled(cancel) => continue,
                ready = self.health_ok(port) => ready,
            };
            if healthy {
                let probe_ok = if pin.accelerator == "OpenCL" {
                    tokio::select! {
                        biased;
                        _ = super::wait_if_cancelled(cancel) => continue,
                        ready = self.generation_probe_ok(port) => ready,
                    }
                } else {
                    true
                };
                if !probe_ok {
                    log::warn!("OpenCL loaded but a test reply failed; falling back to CPU");
                    if let Some(pid) = child.id() {
                        force_kill_pid(pid);
                    }
                    let _ = child.kill().await;
                    return Err(SpawnFailed {
                        try_bundled: true,
                        error: anyhow!("OpenCL probe failed"),
                    });
                }
                break;
            }
            if started.elapsed() > HEALTH_TIMEOUT {
                if let Some(pid) = child.id() {
                    force_kill_pid(pid);
                }
                let _ = child.kill().await;
                let tail = engine_log_tail(&log_path);
                log::error!(
                    "llama-server health timeout after {}s; log tail:\n{tail}",
                    started.elapsed().as_secs()
                );
                return Err(SpawnFailed {
                    try_bundled: true,
                    error: anyhow!("llama-server health timeout"),
                });
            }
            if last_note.elapsed() >= Duration::from_secs(10) {
                let secs = started.elapsed().as_secs();
                log::info!("still loading {} ({}s)", model.name, secs);
                last_note = std::time::Instant::now();
            }
            tokio::select! {
                _ = super::wait_if_cancelled(cancel) => {},
                _ = tokio::time::sleep(Duration::from_millis(400)) => {},
            }
        }

        *crate::core::write_lock(&self.ctx.runtime_plan) = Some(plan.clone());
        inner.flatten_tools = false;
        inner.tools_rejected = false;
        inner.child = Some(child);
        inner.port = port;
        inner.model_file = model.file.clone();
        inner.external_base_url = None;
        let runtime = super::bench::runtime(pin, &plan);
        let invalidated = super::bench::invalidate(
            &mut crate::core::write_lock(&self.ctx.settings),
            &model.file,
            &runtime,
        );
        if invalidated {
            self.ctx.save_settings();
        }
        inner.benchmark_runtime = Some(runtime);
        inner.chat_stall = super::tune::chat_stall_timeout(&plan);
        let url = format!("http://127.0.0.1:{port}");
        inner.reasoning = Some(self.load_reasoning_caps(&url).await);

        log::info!(
            "llama-server ready on 127.0.0.1:{port} after {}s",
            started.elapsed().as_secs()
        );
        self.set_status(EngineState::Ready, None);

        Ok(url)
    }

    async fn live_url(&self, inner: &mut Inner, model: &ActiveModel) -> Option<String> {
        let child = inner.child.as_mut()?;
        let exited = child.try_wait().ok().flatten().is_some();
        let port = inner.port;
        let same_model = inner.model_file == model.file;
        if !exited && same_model && self.health_ok(port).await {
            if inner.reasoning.is_none() {
                let url = format!("http://127.0.0.1:{port}");
                inner.reasoning = Some(self.load_reasoning_caps(&url).await);
            }
            self.set_status(EngineState::Ready, None);
            Some(format!("http://127.0.0.1:{port}"))
        } else {
            self.clear_engine_state(inner).await;
            None
        }
    }

    /// Closing Larra stops the engine and releases its memory.
    pub async fn stop(&self) {
        let mut inner = self.inner.lock().await;
        if let Some(mut child) = inner.child.take() {
            let _ = child.kill().await;
        }
        inner.external_base_url = None;
        inner.reasoning = None;
        *crate::core::write_lock(&self.ctx.runtime_plan) = None;
        drop(inner);
        let data_dir = self.ctx.paths.base().to_path_buf();
        if let Err(error) =
            tokio::task::spawn_blocking(move || kill_stale_llama_servers(&data_dir)).await
        {
            log::warn!("kill stale llama-servers: {error}");
        }
        let has_model = self.active_model().is_some();
        self.set_status(
            if has_model {
                EngineState::Stopped
            } else {
                EngineState::NoModel
            },
            None,
        );
    }

    pub fn stop_blocking(&self) {
        if let Ok(mut inner) = self.inner.try_lock() {
            if let Some(child) = inner.child.as_mut() {
                if let Some(pid) = child.id() {
                    force_kill_pid(pid);
                }
            }
            inner.child = None;
            inner.external_base_url = None;
            inner.reasoning = None;
            *crate::core::write_lock(&self.ctx.runtime_plan) = None;
        }
        kill_stale_llama_servers(self.ctx.paths.base());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::NoopEvents;
    use crate::ingest::extract::ExtractorSettings;
    use crate::paths::Paths;
    use std::path::Path;
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// Answer `/models` with 200 and everything else with 404, for a bounded
    /// number of connections. The external lifecycle polls /models and reads
    /// /props (llama.cpp-only) after it turns Ready.
    async fn serve_endpoint_models() -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            for _ in 0..64 {
                let Ok((mut socket, _)) = listener.accept().await else {
                    break;
                };
                let mut buffer = vec![0u8; 4096];
                let read = socket.read(&mut buffer).await.unwrap_or(0);
                let request = String::from_utf8_lossy(&buffer[..read]).to_string();
                let response = if request.starts_with("GET /models") {
                    let body = "{\"data\":[{\"id\":\"remote-model\"}]}";
                    format!(
                        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    )
                } else {
                    "HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
                        .to_string()
                };
                let _ = socket.write_all(response.as_bytes()).await;
            }
        });
        format!("http://127.0.0.1:{port}")
    }

    #[tokio::test]
    async fn external_endpoint_becomes_ready_and_reports_its_base_url() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path().join("appdata"));
        let ctx = crate::core::Ctx::new(paths, Arc::new(NoopEvents), ExtractorSettings::default())
            .unwrap();
        let engine = Engine::new(ctx.clone());

        let base_url = serve_endpoint_models().await;
        {
            let mut settings = crate::core::write_lock(&ctx.settings);
            settings.active_model = Some(ActiveModel {
                file: String::new(),
                name: "Remote Gemma".into(),
                source: "external".into(),
                reference: "LM Studio".into(),
                license: None,
                size_bytes: 0,
                endpoint: Some(EndpointConfig {
                    base_url: base_url.clone(),
                    model_id: "gemma-4-12b".into(),
                    api_key: None,
                }),
            });
        }

        let url = engine
            .ensure_ready_cancel(&AtomicBool::new(false))
            .await
            .expect("external ready");
        assert_eq!(url, base_url);
        let status = engine.status();
        assert_eq!(status.state, EngineState::Ready);
        assert_eq!(status.model_name.as_deref(), Some("Remote Gemma"));

        // The stream asks for the cached URL without re-running the cycle.
        assert_eq!(engine.cached_base_url().as_deref(), Some(base_url.as_str()));

        engine.stop().await;
        assert_eq!(engine.status().state, EngineState::Stopped);
        assert_eq!(engine.cached_base_url(), None);
    }

    fn plan(no_mmap: bool, flash: &'static str) -> SpawnPlan {
        SpawnPlan {
            context_tokens: 8192,
            answer_tokens: 2048,
            batch: 2048,
            ubatch: 512,
            no_mmap,
            flash_attn: flash,
            cache_type: "q8_0",
            gpu_layers: 99,
        }
    }

    fn has_pair(args: &[String], key: &str, value: &str) -> bool {
        args.windows(2)
            .any(|pair| pair[0] == key && pair[1] == value)
    }

    #[test]
    fn vulkan_args_include_no_mmap() {
        let args = llama_server_args(Path::new("/tmp/m.gguf"), 8080, &plan(true, "on"));
        assert!(args.iter().any(|a| a == "--no-mmap"));
        assert!(has_pair(&args, "-fa", "on"));
        assert!(has_pair(&args, "-ub", "512"));
        assert!(has_pair(&args, "--cache-type-k", "q8_0"));
        assert!(has_pair(&args, "--cache-type-v", "q8_0"));
    }

    #[test]
    fn metal_args_keep_mmap() {
        let args = llama_server_args(Path::new("/tmp/m.gguf"), 8080, &plan(false, "on"));
        assert!(!args.iter().any(|a| a == "--no-mmap"));
    }

    #[test]
    fn cpu_args_use_flash_auto() {
        let args = llama_server_args(Path::new("/tmp/m.gguf"), 8080, &plan(false, "auto"));
        assert!(has_pair(&args, "-fa", "auto"));
        assert!(!args.iter().any(|a| a == "--no-mmap"));
    }

    #[test]
    fn opencl_args_use_f16_cache() {
        let mut plan = plan(false, "auto");
        plan.cache_type = "f16";
        plan.gpu_layers = 99;
        let args = llama_server_args(Path::new("/tmp/m.gguf"), 8080, &plan);
        assert!(has_pair(&args, "--cache-type-k", "f16"));
        assert!(has_pair(&args, "--cache-type-v", "f16"));
        assert!(has_pair(&args, "-ngl", "99"));
    }

    #[test]
    fn cpu_args_offload_no_layers() {
        let mut plan = plan(false, "auto");
        plan.gpu_layers = 0;
        let args = llama_server_args(Path::new("/tmp/m.gguf"), 8080, &plan);
        assert!(has_pair(&args, "-ngl", "0"));
    }

    #[test]
    fn optional_timeout_falls_back_once() {
        let bundled = crate::engine::pin::pin_for("windows", "x86_64").unwrap();
        let cuda = crate::engine::pin::optional_pin_for("windows", "x86_64", "CUDA").unwrap();
        let timeout = SpawnFailed {
            try_bundled: true,
            error: anyhow!("llama-server health timeout"),
        };
        assert!(should_fallback_to_bundled(&timeout, false, cuda, bundled));
        assert!(!should_fallback_to_bundled(&timeout, true, cuda, bundled));
        assert!(!should_fallback_to_bundled(
            &timeout, false, bundled, bundled
        ));
        let port = SpawnFailed {
            try_bundled: false,
            error: anyhow!("no free port"),
        };
        assert!(!should_fallback_to_bundled(&port, false, cuda, bundled));
    }

    #[test]
    fn stopped_is_not_an_engine_error() {
        assert!(is_stopped(&stopped_error()));
        assert!(!is_stopped(&anyhow!("llama-server health timeout")));
    }
}
