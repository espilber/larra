//! Chat SSE consumption: thinking vs answer, cancellation, compute retry.

use anyhow::{anyhow, Context, Result};
use futures_util::StreamExt;
use serde_json::json;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use super::process::{is_compute_failure, is_template_failure};
use super::think::{SplitOut, ThinkSplitter};
use super::{ChatOutput, Engine, StreamEvent, ToolCall};

/// Counts this completion so a model switch can wait until Chat is idle.
pub(super) struct GenerationSlot<'a> {
    counter: &'a AtomicUsize,
}

impl<'a> GenerationSlot<'a> {
    pub(super) fn try_acquire(counter: &'a AtomicUsize) -> Option<Self> {
        counter
            .compare_exchange(0, 1, Ordering::Relaxed, Ordering::Relaxed)
            .ok()
            .map(|_| Self { counter })
    }

    fn acquire(counter: &'a AtomicUsize) -> Self {
        counter.fetch_add(1, Ordering::Relaxed);
        Self { counter }
    }
}

impl Drop for GenerationSlot<'_> {
    fn drop(&mut self) {
        self.counter.fetch_sub(1, Ordering::Relaxed);
    }
}

impl Engine {
    pub(crate) fn cached_base_url(&self) -> Option<String> {
        let inner = self.inner.try_lock().ok()?;
        if let Some(url) = &inner.external_base_url {
            return Some(url.clone());
        }
        (inner.child.is_some() && inner.port != 0)
            .then(|| format!("http://127.0.0.1:{}", inner.port))
    }

    fn chat_stall(&self) -> Duration {
        self.inner
            .try_lock()
            .ok()
            .map(|inner| inner.chat_stall)
            .unwrap_or(CHAT_STALL_TIMEOUT)
    }

    /// One-shot completion that must not pollute the KV cache used by Chat.
    pub async fn chat_once(
        self: &Arc<Self>,
        messages: &[super::ChatMessage],
        temperature: f32,
        max_tokens: u32,
        cancel: &Arc<AtomicBool>,
    ) -> Result<ChatOutput> {
        self.complete(
            messages,
            super::ChatOptions::once(temperature, max_tokens),
            cancel,
            |_| {},
        )
        .await
    }

    pub async fn complete(
        self: &Arc<Self>,
        messages: &[super::ChatMessage],
        options: super::ChatOptions,
        cancel: &Arc<AtomicBool>,
        mut on_event: impl FnMut(StreamEvent<'_>),
    ) -> Result<ChatOutput> {
        let _slot = GenerationSlot::acquire(&self.generation_in_flight);
        let messages = super::messages::single_leading_system(messages);
        let result = async {
            let mut body = json!({
                "messages": messages.as_ref(),
                "stream": true,
                "temperature": options.temperature,
                "max_tokens": options.max_tokens,
                "cache_prompt": options.cache_prompt,
            });
            let mut dropped_tools = false;
            if let Some(tools) = &options.tools {
                body["tools"] = tools.clone();
                body["parallel_tool_calls"] = json!(false);
                if let Some(choice) = &options.tool_choice {
                    body["tool_choice"] = json!(choice);
                }
            }
            let mut retried = false;
            let mut flattened = false;
            loop {
                if cancel.load(Ordering::Relaxed) {
                    return Ok(ChatOutput::default());
                }
                let base = match self.ensure_ready_cancel(cancel).await {
                    Ok(url) => url,
                    Err(_) if cancel.load(Ordering::Relaxed) => {
                        return Ok(ChatOutput::default());
                    }
                    Err(error) => return Err(error),
                };
                if cancel.load(Ordering::Relaxed) {
                    return Ok(ChatOutput::default());
                }
                body["messages"] = json!(messages.as_ref());
                // External endpoint variant: LM Studio and friends require
                // `model`, and `cache_prompt` is a llama.cpp extension strict
                // servers reject. The local engine never needs `model`.
                let endpoint = self.active_model().and_then(|model| model.endpoint.clone());
                apply_endpoint_body_fields(&mut body, endpoint.as_ref(), options.cache_prompt);
                let caps = {
                    let inner = tokio::select! {
                        biased;
                        _ = super::wait_if_cancelled(cancel) => return Ok(ChatOutput::default()),
                        inner = self.inner.lock() => inner,
                    };
                    if inner.flatten_tools && !flattened {
                        flattened = true;
                    }
                    if inner.tools_rejected {
                        dropped_tools = true;
                        if let Some(obj) = body.as_object_mut() {
                            obj.remove("tools");
                            obj.remove("tool_choice");
                            obj.remove("parallel_tool_calls");
                        }
                    }
                    inner.reasoning.clone().unwrap_or_default()
                };
                super::reasoning::apply_to_body(&mut body, options.thinking, &caps);
                tokio::select! {
                    _ = super::wait_if_cancelled(cancel) => return Ok(ChatOutput::default()),
                    result = self.fit_request(&base, &mut body, flattened) => result?,
                }
                let send = self
                    .client
                    .post(format!("{base}/v1/chat/completions"))
                    .json(&body)
                    .send();
                let response = tokio::select! {
                    biased;
                    _ = super::wait_if_cancelled(cancel) => {
                        return Ok(ChatOutput::default());
                    }
                    result = send => result.context("chat request")?,
                };
                if !response.status().is_success() {
                    let status = response.status();
                    let text = tokio::select! {
                        biased;
                        _ = super::wait_if_cancelled(cancel) => return Ok(ChatOutput::default()),
                        text = response.text() => text.unwrap_or_default(),
                    };
                    if !retried && is_compute_failure(&text) {
                        retried = true;
                        self.recover_after_compute_failure(&text).await;
                        continue;
                    }
                    if !flattened && is_template_failure(&text) {
                        flattened = true;
                        if let Ok(mut inner) = self.inner.try_lock() {
                            inner.flatten_tools = true;
                        }
                        log::warn!(
                            "chat template rejected the message list ({status}); \
inlining tool turns and trying again: {text}"
                        );
                        continue;
                    }
                    if !dropped_tools
                        && options.tools.is_some()
                        && status.is_client_error()
                        && text.to_ascii_lowercase().contains("tool")
                        && !is_compute_failure(&text)
                    {
                        log::warn!(
                            "engine rejected tools ({status}); answering without them: {text}"
                        );
                        dropped_tools = true;
                        if let Ok(mut inner) = self.inner.try_lock() {
                            inner.tools_rejected = true;
                        }
                        if let Some(obj) = body.as_object_mut() {
                            obj.remove("tools");
                            obj.remove("tool_choice");
                            obj.remove("parallel_tool_calls");
                        }
                        continue;
                    }
                    return Err(anyhow!("generation failed ({status}): {text}"));
                }

                let stream = response.bytes_stream();
                let stall = self.chat_stall();
                match consume_sse_timed(stream, cancel, &mut on_event, stall).await {
                    Ok(mut output) => {
                        output.answer = output.answer.trim().to_string();
                        output.thinking = output.thinking.trim().to_string();
                        return Ok(output);
                    }
                    Err(error) => {
                        let message = error.to_string();
                        if !retried && is_compute_failure(&message) {
                            retried = true;
                            on_event(StreamEvent::ResetAnswer);
                            self.recover_after_compute_failure(&message).await;
                            continue;
                        }
                        return Err(error);
                    }
                }
            }
        }
        .await;
        if options.cache_prompt {
            if let Some(base) = self.cached_base_url() {
                super::bench::schedule_after_chat(self, base);
            }
        }
        result
    }

    async fn recover_after_compute_failure(&self, detail: &str) {
        log::error!("engine compute failed; restarting: {detail}");
        self.stop().await;
    }

    /// Read llama.cpp timings for calibration using the already loaded engine.
    pub async fn completion_timings(
        self: &Arc<Self>,
        base: &str,
        prompt: &str,
    ) -> Result<super::Timings> {
        let response = self
            .client
            .post(format!("{base}/completion"))
            .json(&json!({
                "prompt": prompt,
                "n_predict": 24,
                "cache_prompt": false,
            }))
            .send()
            .await?
            .error_for_status()?;
        let value: serde_json::Value = response.json().await?;
        let timings = &value["timings"];
        Ok(super::Timings {
            prompt_per_second: timings["prompt_per_second"].as_f64().unwrap_or(0.0),
            predicted_per_second: timings["predicted_per_second"].as_f64().unwrap_or(0.0),
        })
    }
}

/// Give up when llama-server sends no bytes for this long (prefill or a hung follow-up).
/// Align the completion body with the engine mode. An external endpoint
/// gets its `model` id and loses `cache_prompt` (a llama.cpp extension that
/// strict OpenAI servers reject). The local engine never sends `model`
/// (llama-server serves its own file) and keeps the caller's `cache_prompt`.
pub(crate) fn apply_endpoint_body_fields(
    body: &mut serde_json::Value,
    endpoint: Option<&crate::settings::EndpointConfig>,
    cache_prompt: bool,
) {
    match endpoint {
        Some(config) => {
            body["model"] = json!(config.model_id);
            if let Some(obj) = body.as_object_mut() {
                obj.remove("cache_prompt");
            }
        }
        None => {
            if let Some(obj) = body.as_object_mut() {
                obj.remove("model");
            }
            body["cache_prompt"] = json!(cache_prompt);
        }
    }
}

pub(crate) const CHAT_STALL_TIMEOUT: Duration = Duration::from_secs(90);

/// External endpoints get a much wider window: the remote machine may need
/// minutes to chew through a big Shelf prompt before the first token, and
/// tool-call rounds put another long quiet stretch between chunks. A server
/// that is truly gone fails earlier at the HTTP layer.
pub(crate) const ENDPOINT_CHAT_STALL_TIMEOUT: Duration = Duration::from_secs(300);

/// Read an OpenAI-style SSE chat stream until `[DONE]`, cancel, or error.
#[cfg(test)]
pub(crate) async fn consume_sse<S, E>(
    stream: S,
    cancel: &Arc<AtomicBool>,
    on_event: &mut impl FnMut(StreamEvent<'_>),
) -> Result<ChatOutput>
where
    S: futures_util::Stream<Item = Result<bytes::Bytes, E>> + Unpin,
    E: std::fmt::Display + Send + Sync + 'static,
{
    consume_sse_timed(stream, cancel, on_event, CHAT_STALL_TIMEOUT).await
}

pub(crate) async fn consume_sse_timed<S, E>(
    mut stream: S,
    cancel: &Arc<AtomicBool>,
    on_event: &mut impl FnMut(StreamEvent<'_>),
    stall: Duration,
) -> Result<ChatOutput>
where
    S: futures_util::Stream<Item = Result<bytes::Bytes, E>> + Unpin,
    E: std::fmt::Display + Send + Sync + 'static,
{
    let mut output = ChatOutput::default();
    let mut splitter = ThinkSplitter::new();
    let mut buffer: Vec<u8> = Vec::new();
    let mut tool_acc = ToolCallAcc::default();

    let apply = |events: Vec<SplitOut>,
                 output: &mut ChatOutput,
                 on_event: &mut dyn FnMut(StreamEvent<'_>)| {
        for event in events {
            output.apply(&event);
            match &event {
                SplitOut::Thinking(t) => on_event(StreamEvent::Thinking(t)),
                SplitOut::Answer(t) => on_event(StreamEvent::Answer(t)),
                SplitOut::Promote => on_event(StreamEvent::PromoteAnswerToThinking),
            }
        }
    };

    let mut stream_error: Option<String> = None;
    let mut finished = false;
    'outer: loop {
        let chunk = tokio::select! {
            biased;
            _ = crate::engine::wait_if_cancelled(cancel) => break,
            result = tokio::time::timeout(stall, stream.next()) => match result {
                Ok(Some(chunk)) => chunk,
                Ok(None) => break,
                Err(_) => {
                    log::warn!("chat stream stalled after {stall:?} without bytes");
                    return Err(anyhow!("generation stalled"));
                }
            },
        };
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let chunk = chunk.map_err(|error| anyhow!("chat stream: {error}"))?;
        buffer.extend_from_slice(&chunk);
        while let Some(line_end) = buffer.iter().position(|byte| *byte == b'\n') {
            let line = String::from_utf8_lossy(&buffer[..line_end])
                .trim()
                .to_string();
            buffer.drain(..=line_end);
            let Some(data) = line.strip_prefix("data:").map(str::trim_start) else {
                continue;
            };
            if data == "[DONE]" {
                finished = true;
                break 'outer;
            }
            let Ok(value) = serde_json::from_str::<serde_json::Value>(data) else {
                continue;
            };
            if let Some(error) = value["error"]["message"].as_str() {
                stream_error = Some(error.to_string());
                break 'outer;
            }
            if let Some(error) = value["error"].as_str() {
                stream_error = Some(error.to_string());
                break 'outer;
            }
            let choice = &value["choices"][0];
            if let Some(reason) = choice["finish_reason"].as_str() {
                if !reason.is_empty() && reason != "null" {
                    output.finish_reason = Some(reason.to_string());
                }
            }
            let delta = &choice["delta"];
            if !delta.is_null() {
                tool_acc.apply(delta);
                if let Some(piece) = delta["reasoning_content"].as_str() {
                    if !piece.is_empty() {
                        output.thinking.push_str(piece);
                        on_event(StreamEvent::Thinking(piece));
                    }
                }
                if let Some(piece) = delta["content"].as_str() {
                    apply(splitter.push(piece), &mut output, on_event);
                }
            }
            let message = &choice["message"];
            if message.is_object() {
                tool_acc.apply(message);
                if let Some(piece) = message["reasoning_content"].as_str() {
                    if !piece.is_empty() && output.thinking.is_empty() {
                        output.thinking.push_str(piece);
                    }
                }
                if let Some(piece) = message["content"].as_str() {
                    if output.answer.is_empty() && output.thinking.is_empty() {
                        apply(splitter.push(piece), &mut output, on_event);
                    }
                }
            }
        }
    }
    if let Some(error) = stream_error {
        return Err(anyhow!("generation failed: {error}"));
    }
    apply(splitter.flush(), &mut output, on_event);
    if !cancel.load(Ordering::Relaxed) && !finished && output.finish_reason.is_none() {
        return Err(anyhow!("chat stream ended before the answer finished"));
    }
    output.tool_calls = tool_acc.finish();
    if output.finish_reason.is_none() && !output.tool_calls.is_empty() {
        output.finish_reason = Some("tool_calls".into());
    }
    Ok(output)
}

#[derive(Default)]
struct ToolCallAcc {
    slots: Vec<Option<PartialCall>>,
}

#[derive(Default)]
struct PartialCall {
    id: String,
    name: String,
    arguments: String,
}

impl ToolCallAcc {
    fn apply(&mut self, container: &serde_json::Value) {
        let Some(items) = container["tool_calls"].as_array() else {
            return;
        };
        for item in items {
            let index = item["index"].as_u64().unwrap_or(0) as usize;
            while self.slots.len() <= index {
                self.slots.push(None);
            }
            let slot = self.slots[index].get_or_insert_with(PartialCall::default);
            if let Some(id) = item["id"].as_str() {
                if !id.is_empty() {
                    slot.id = id.to_string();
                }
            }
            let function = &item["function"];
            if let Some(name) = function["name"].as_str() {
                if slot.name.is_empty()
                    || (name.len() > slot.name.len() && name.starts_with(&slot.name))
                {
                    slot.name = name.to_string();
                } else if !slot.name.starts_with(name) && !name.starts_with(&slot.name) {
                    slot.name.push_str(name);
                }
            }
            if let Some(args) = function["arguments"].as_str() {
                slot.arguments.push_str(args);
            } else if function["arguments"].is_object() {
                slot.arguments = function["arguments"].to_string();
            }
        }
    }

    fn finish(self) -> Vec<ToolCall> {
        self.slots
            .into_iter()
            .enumerate()
            .filter_map(|(i, slot)| {
                let slot = slot?;
                if slot.name.trim().is_empty() {
                    return None;
                }
                let id = if slot.id.is_empty() {
                    format!("call_{}", i + 1)
                } else {
                    slot.id
                };
                Some(ToolCall::function(id, slot.name, slot.arguments))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::EndpointConfig;
    use bytes::Bytes;
    use futures_util::stream;
    use std::sync::Mutex;

    #[test]
    fn external_body_carries_the_model_and_drops_cache_prompt() {
        let mut body = json!({
            "messages": [],
            "cache_prompt": true,
            "max_tokens": 512,
        });
        let config =
            EndpointConfig::parse("http://127.0.0.1:1234/v1", "gemma-4-12b", None).unwrap();
        apply_endpoint_body_fields(&mut body, Some(&config), true);
        assert_eq!(body["model"], "gemma-4-12b");
        assert!(body.get("cache_prompt").is_none());
        assert_eq!(body["max_tokens"], 512);
    }

    #[test]
    fn local_body_keeps_cache_prompt_and_no_model() {
        let mut body = json!({
            "messages": [],
            "model": "stale",
            "cache_prompt": false,
            "max_tokens": 512,
        });
        apply_endpoint_body_fields(&mut body, None, true);
        assert!(body.get("model").is_none());
        assert_eq!(body["cache_prompt"], true);
        assert_eq!(body["max_tokens"], 512);
    }

    fn delta(text: &str) -> Bytes {
        Bytes::from(format!(
            "data: {{\"choices\":[{{\"delta\":{{\"content\":{}}}}}]}}\n\n",
            serde_json::to_string(text).unwrap()
        ))
    }

    #[tokio::test]
    async fn multilingual_text_survives_every_byte_boundary() {
        let expected = "Aquí está el larra: 日本語 · Ελληνικά · 🥕. This answer is complete.";
        let mut bytes = delta(expected).to_vec();
        bytes.extend_from_slice(b"data:[DONE]\n\n");
        let pieces = bytes
            .into_iter()
            .map(|b| Ok::<_, std::io::Error>(Bytes::from(vec![b])));
        let output = consume_sse(
            stream::iter(pieces),
            &Arc::new(AtomicBool::new(false)),
            &mut |_| {},
        )
        .await
        .unwrap();
        assert_eq!(output.answer, expected);
    }

    #[tokio::test]
    async fn early_eof_is_interrupted_and_keeps_visible_text() {
        let expected = "The retained partial answer contains useful information.";
        let stream = stream::iter([Ok::<_, std::io::Error>(delta(expected))]);
        let mut visible = String::new();
        let error = consume_sse(stream, &Arc::new(AtomicBool::new(false)), &mut |event| {
            if let StreamEvent::Answer(text) = event {
                visible.push_str(text);
            }
        })
        .await
        .unwrap_err();
        assert_eq!(visible, expected);
        assert!(error.to_string().contains("before the answer finished"));
    }

    #[tokio::test]
    async fn cancel_aborts_a_stalled_sse() {
        let cancel = Arc::new(AtomicBool::new(false));
        let stream = stream::pending::<Result<Bytes, std::io::Error>>();
        let flag = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(40)).await;
            flag.store(true, Ordering::Relaxed);
        });
        let started = std::time::Instant::now();
        let output = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            consume_sse(stream, &cancel, &mut |_| {}),
        )
        .await
        .expect("stalled stream ignored Stop")
        .unwrap();
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
        assert!(output.answer.is_empty(), "{:?}", output.answer);
    }

    #[tokio::test]
    async fn cancel_stops_sse_mid_stream() {
        let cancel = Arc::new(AtomicBool::new(false));
        let stream = stream::iter([
            Ok::<_, std::io::Error>(delta("Hello there, this is the visible answer.")),
            Ok(delta(" SECRET")),
        ]);
        let collected = Mutex::new(String::new());
        let output = consume_sse(stream, &cancel, &mut |event| {
            if let StreamEvent::Answer(t) = event {
                crate::core::mutex_lock(&collected).push_str(t);
                cancel.store(true, Ordering::Relaxed);
            }
        })
        .await
        .unwrap();
        assert!(
            output.answer.starts_with("Hello there"),
            "{:?}",
            output.answer
        );
        assert!(
            !output.answer.contains("SECRET"),
            "cancelled stream still included {:?}",
            output.answer
        );
    }

    #[tokio::test]
    async fn stall_timeout_fails_a_hung_stream() {
        let cancel = Arc::new(AtomicBool::new(false));
        let stream = stream::pending::<Result<Bytes, std::io::Error>>();
        let started = std::time::Instant::now();
        let err = consume_sse_timed(
            stream,
            &cancel,
            &mut |_| {},
            std::time::Duration::from_millis(40),
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("stalled"), "{err:#}");
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
    }

    #[tokio::test]
    async fn sse_error_object_is_a_failure() {
        let cancel = Arc::new(AtomicBool::new(false));
        let stream = stream::iter([Ok::<_, std::io::Error>(Bytes::from(
            "data: {\"error\":{\"message\":\"compute error\"}}\n\n",
        ))]);
        let err = consume_sse(stream, &cancel, &mut |_| {}).await.unwrap_err();
        assert!(err.to_string().contains("compute error"), "{err:#}");
    }

    fn tool_delta(index: u32, extra: &str) -> Bytes {
        Bytes::from(format!(
            "data: {{\"choices\":[{{\"delta\":{{\"tool_calls\":[{{\"index\":{index},{extra}}}]}}}}]}}\n\n"
        ))
    }

    #[tokio::test]
    async fn sse_assembles_streamed_tool_calls() {
        let cancel = Arc::new(AtomicBool::new(false));
        let stream = stream::iter([
            Ok::<_, std::io::Error>(tool_delta(
                0,
                "\"id\":\"call_1\",\"type\":\"function\",\"function\":{\"name\":\"open_shelf_file\",\"arguments\":\"\"}",
            )),
            Ok(tool_delta(
                0,
                "\"function\":{\"arguments\":\"{\\\"file\\\":\\\"notes.md\\\"}\"}",
            )),
            Ok(Bytes::from(
                "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\n",
            )),
        ]);
        let output = consume_sse(stream, &cancel, &mut |_| {}).await.unwrap();
        assert_eq!(output.finish_reason.as_deref(), Some("tool_calls"));
        assert_eq!(output.tool_calls.len(), 1);
        assert_eq!(output.tool_calls[0].function.name, "open_shelf_file");
        assert_eq!(
            output.tool_calls[0].function.arguments,
            "{\"file\":\"notes.md\"}"
        );
        assert!(output.answer.is_empty(), "{:?}", output.answer);
    }
}
