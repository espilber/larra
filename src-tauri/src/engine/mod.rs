//! llama.cpp `llama-server` as a child process.
//!
//! Generation is its only job. The UI never sees a server, port, or PID;
//! it sees "Warming up..." and then tokens.

pub mod bench;
mod binary;
mod budget;
pub mod catalog;
pub mod download;
mod gguf;
mod gpu;
mod install;
mod messages;
pub mod models;
mod pin;
mod process;
mod ready;
mod reasoning;
mod stream;
mod think;
pub(crate) mod tune;

pub use pin::{
    current_engine_pin, find_bundled_engine_archive, preferred_engine_pin, ENGINE_BUILD,
    ENGINE_RELEASE,
};

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize};
use std::sync::Arc;
use std::time::Duration;
use tokio::process::Child;

use crate::core::Ctx;
use crate::settings::ActiveModel;
use process::{kill_stale_llama_servers, USER_AGENT};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum EngineState {
    NoModel,
    Downloading,
    Stopped,
    Starting,
    Ready,
    Error,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineStatus {
    pub state: EngineState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_name: Option<String>,
}

struct Inner {
    child: Option<Child>,
    port: u16,
    model_file: String,
    benchmark_runtime: Option<crate::settings::BenchmarkRuntime>,
    reasoning: Option<reasoning::ReasoningCaps>,
    chat_stall: Duration,
    flatten_tools: bool,
    tools_rejected: bool,
}

pub struct Engine {
    ctx: Arc<Ctx>,
    pub client: reqwest::Client,
    download_client: reqwest::Client,
    inner: tokio::sync::Mutex<Inner>,
    /// Serializes first-time engine download + spawn so two chat/warmup
    /// callers cannot both fetch llama.cpp.
    start_lock: tokio::sync::Mutex<()>,
    benchmark_lock: tokio::sync::Mutex<()>,
    status: std::sync::Mutex<EngineStatus>,
    downloads: std::sync::Mutex<HashMap<String, download::DownloadControl>>,
    /// Chat completions in flight — the install benchmark waits until this is 0.
    generation_in_flight: AtomicUsize,
    /// Optional CUDA/OpenCL already failed this session; stay on the bundled pin.
    skip_optional: AtomicBool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    /// `None` serializes as JSON null — required for assistant tool-call turns.
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

impl ChatMessage {
    pub fn text(role: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: role.into(),
            content: Some(content.into()),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        }
    }

    pub fn as_text(&self) -> &str {
        self.content.as_deref().unwrap_or("")
    }
}

/// One OpenAI-style tool call (`type: function`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub call_type: String,
    pub function: ToolCallFunction,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolCallFunction {
    pub name: String,
    pub arguments: String,
}

impl ToolCall {
    pub fn function(
        id: impl Into<String>,
        name: impl Into<String>,
        arguments: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            call_type: "function".into(),
            function: ToolCallFunction {
                name: name.into(),
                arguments: arguments.into(),
            },
        }
    }
}

/// Whether this completion should use native model thinking.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ChatThinking {
    /// Force thinking off. Extra search queries and Off / Light stay here.
    #[default]
    Off,
    /// Cheapest native think the loaded chat template supports.
    Deep,
}

/// Options for one `/v1/chat/completions` round.
#[derive(Debug, Clone)]
pub struct ChatOptions {
    pub temperature: f32,
    pub max_tokens: u32,
    pub cache_prompt: bool,
    pub thinking: ChatThinking,
    pub tools: Option<serde_json::Value>,
    pub tool_choice: Option<String>,
}

impl ChatOptions {
    pub fn stream(temperature: f32, max_tokens: u32) -> Self {
        Self {
            temperature,
            max_tokens,
            cache_prompt: true,
            thinking: ChatThinking::Off,
            tools: None,
            tool_choice: None,
        }
    }

    pub fn once(temperature: f32, max_tokens: u32) -> Self {
        Self {
            temperature,
            max_tokens,
            cache_prompt: false,
            thinking: ChatThinking::Off,
            tools: None,
            tool_choice: None,
        }
    }
}

pub struct Timings {
    pub prompt_per_second: f64,
    pub predicted_per_second: f64,
}

impl Engine {
    pub fn ctx(&self) -> &Arc<Ctx> {
        &self.ctx
    }

    pub fn catalog_client(&self) -> &reqwest::Client {
        &self.download_client
    }

    pub fn new(ctx: Arc<Ctx>) -> Arc<Self> {
        let model_name = crate::core::read_lock(&ctx.settings)
            .active_model
            .as_ref()
            .map(|m| m.name.clone());
        let state = if model_name.is_some() {
            EngineState::Stopped
        } else {
            EngineState::NoModel
        };
        kill_stale_llama_servers(ctx.paths.base());
        Arc::new(Self {
            ctx,
            client: reqwest::Client::builder()
                .user_agent(USER_AGENT)
                .connect_timeout(Duration::from_secs(30))
                .build()
                .expect("reqwest client"),
            download_client: reqwest::Client::builder()
                .user_agent(USER_AGENT)
                .https_only(true)
                .redirect(reqwest::redirect::Policy::limited(5))
                .http1_only()
                .pool_max_idle_per_host(8)
                .connect_timeout(Duration::from_secs(30))
                .tcp_nodelay(true)
                .build()
                .expect("reqwest download client"),
            inner: tokio::sync::Mutex::new(Inner {
                child: None,
                port: 0,
                model_file: String::new(),
                benchmark_runtime: None,
                reasoning: None,
                chat_stall: Duration::from_secs(90),
                flatten_tools: false,
                tools_rejected: false,
            }),
            start_lock: tokio::sync::Mutex::new(()),
            benchmark_lock: tokio::sync::Mutex::new(()),
            status: std::sync::Mutex::new(EngineStatus {
                state,
                detail: None,
                model_name,
            }),
            downloads: std::sync::Mutex::new(HashMap::new()),
            generation_in_flight: AtomicUsize::new(0),
            skip_optional: AtomicBool::new(false),
        })
    }

    pub fn status(&self) -> EngineStatus {
        crate::core::mutex_lock(&self.status).clone()
    }

    fn set_status(&self, state: EngineState, detail: Option<String>) {
        let model_name = crate::core::read_lock(&self.ctx.settings)
            .active_model
            .as_ref()
            .map(|m| m.name.clone());
        let status = EngineStatus {
            state,
            detail,
            model_name,
        };
        *crate::core::mutex_lock(&self.status) = status.clone();
        if let Ok(payload) = serde_json::to_value(&status) {
            self.ctx.events.emit("larra://engine", payload);
        }
    }

    fn active_model(&self) -> Option<ActiveModel> {
        crate::core::read_lock(&self.ctx.settings)
            .active_model
            .clone()
    }
}

/// Resolves as soon as Chat Stop is set. Used with `select!` so warmup and a
/// stalled HTTP body do not ignore cancel until the next byte.
pub(crate) async fn wait_if_cancelled(cancel: &AtomicBool) {
    loop {
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// A streamed piece of model output, already sorted into thinking vs answer.
pub enum StreamEvent<'a> {
    ResetAnswer,
    Thinking(&'a str),
    Answer(&'a str),
    /// The stream turned out to be reasoning-first without an opening tag
    /// (a lone `</think>` appeared): everything streamed as answer so far
    /// was actually thinking.
    PromoteAnswerToThinking,
}

#[derive(Debug, Default, Clone)]
pub struct ChatOutput {
    pub answer: String,
    pub thinking: String,
    pub tool_calls: Vec<ToolCall>,
    pub finish_reason: Option<String>,
}
