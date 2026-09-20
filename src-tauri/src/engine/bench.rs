//! Calibrate context size for the loaded model and engine configuration.

use super::{Engine, ENGINE_BUILD};
use crate::settings::{BenchmarkResult, BenchmarkRuntime, Settings};
use anyhow::{anyhow, Result};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

const BENCH_TARGET_SECONDS: f64 = 2.0;
const CHARS_PER_TOKEN: f64 = 3.6;
const MIN_BUDGET: usize = 4_000;
const MAX_BUDGET: usize = 26_000;

fn bench_prompt() -> String {
    // ~700 tokens of plain business prose; content is irrelevant, size isn't.
    let paragraph = "The quarterly review covers revenue, invoices, payroll, supplier \
contracts, renewal dates, termination clauses, payment schedules, and the summary of \
open positions across departments. Each section lists the responsible owner, the due \
date, and the current state of the work in plain language. ";
    paragraph.repeat(18)
}

pub(super) fn runtime(
    pin: &super::pin::EnginePin,
    plan: &super::tune::SpawnPlan,
) -> BenchmarkRuntime {
    BenchmarkRuntime {
        engine_build: format!("{ENGINE_BUILD}/{}-{}", pin.os, pin.arch),
        accelerator: if plan.gpu_layers == 0 {
            "CPU"
        } else {
            pin.accelerator
        }
        .into(),
        context_tokens: plan.context_tokens,
        batch: plan.batch,
        ubatch: plan.ubatch,
        gpu_layers: plan.gpu_layers,
    }
}

fn matches(result: &BenchmarkResult, model_file: &str, runtime: &BenchmarkRuntime) -> bool {
    result.model_file == model_file && result.runtime.as_ref() == Some(runtime)
}

pub(super) fn invalidate(
    settings: &mut Settings,
    model_file: &str,
    runtime: &BenchmarkRuntime,
) -> bool {
    if settings
        .benchmark
        .as_ref()
        .is_some_and(|b| matches(b, model_file, runtime))
    {
        return false;
    }
    let changed = settings.benchmark.is_some() || settings.context_budget_chars.is_some();
    settings.benchmark = None;
    settings.context_budget_chars = None;
    changed
}

/// Measure after an idle pause; a user request takes priority.
pub fn schedule_after_chat(engine: &Arc<Engine>, base_url: String) {
    let settings = crate::core::read_lock(&engine.ctx.settings);
    if settings.benchmark.is_some() {
        return;
    }
    drop(settings);
    let engine = engine.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(3)).await;
        if let Err(error) = run(&engine, &base_url, false).await {
            log::debug!("calibration deferred: {error:#}");
        }
    });
}

async fn run(engine: &Arc<Engine>, base: &str, force: bool) -> Result<()> {
    let _measure = engine
        .benchmark_lock
        .try_lock()
        .map_err(|_| anyhow!("calibration busy"))?;
    let (model_file, runtime) = {
        let inner = engine.inner.lock().await;
        if inner.child.is_none() || base != format!("http://127.0.0.1:{}", inner.port) {
            return Err(anyhow!("engine changed before calibration"));
        }
        (
            inner.model_file.clone(),
            inner
                .benchmark_runtime
                .clone()
                .ok_or_else(|| anyhow!("engine not ready"))?,
        )
    };
    if !force
        && crate::core::read_lock(&engine.ctx.settings)
            .benchmark
            .as_ref()
            .is_some_and(|b| matches(b, &model_file, &runtime))
    {
        return Ok(());
    }
    // Count calibration as generation so model switching waits for it too.
    let _generation = super::stream::GenerationSlot::try_acquire(&engine.generation_in_flight)
        .ok_or_else(|| anyhow!("Chat is busy. Try again after the answer finishes."))?;
    let chat_started = async {
        loop {
            if engine.generation_in_flight.load(Ordering::Relaxed) > 1 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    };
    let prompt = bench_prompt();
    let timings = tokio::select! {
        biased;
        _ = chat_started => return Err(anyhow!("Chat is busy. Try again after the answer finishes.")),
        result = tokio::time::timeout(Duration::from_secs(90), engine.completion_timings(base, &prompt)) => result??,
    };
    if !timings.prompt_per_second.is_finite() || timings.prompt_per_second <= 0.0 {
        return Err(anyhow!("engine returned no usable timings"));
    }
    let budget = (timings.prompt_per_second * BENCH_TARGET_SECONDS * CHARS_PER_TOKEN) as usize;
    let budget = budget.clamp(MIN_BUDGET, MAX_BUDGET);
    let inner = engine.inner.lock().await;
    if inner.child.is_none()
        || base != format!("http://127.0.0.1:{}", inner.port)
        || inner.model_file != model_file
        || inner.benchmark_runtime.as_ref() != Some(&runtime)
    {
        return Err(anyhow!("engine changed during calibration"));
    }
    engine.ctx.update_settings(|settings| {
        if settings
            .active_model
            .as_ref()
            .is_some_and(|m| m.file == model_file)
        {
            settings.benchmark = Some(BenchmarkResult {
                prompt_tokens_per_second: timings.prompt_per_second,
                generation_tokens_per_second: timings.predicted_per_second,
                measured_at: chrono::Utc::now().to_rfc3339(),
                model_file,
                runtime: Some(runtime),
            });
            settings.context_budget_chars = Some(budget);
        }
    })?;
    Ok(())
}

impl Engine {
    pub async fn remeasure(self: &Arc<Self>) -> Result<()> {
        let base = self.ensure_ready().await?;
        run(self, &base, true).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> BenchmarkRuntime {
        BenchmarkRuntime {
            engine_build: "b1/windows-x86_64".into(),
            accelerator: "CUDA".into(),
            context_tokens: 8192,
            batch: 1024,
            ubatch: 512,
            gpu_layers: 99,
        }
    }

    fn measured(runtime: Option<BenchmarkRuntime>) -> Settings {
        Settings {
            benchmark: Some(BenchmarkResult {
                prompt_tokens_per_second: 500.0,
                generation_tokens_per_second: 30.0,
                measured_at: "2026-09-06T00:00:00Z".into(),
                model_file: "model.gguf".into(),
                runtime,
            }),
            context_budget_chars: Some(12_000),
            ..Default::default()
        }
    }

    #[test]
    fn keeps_calibration_only_for_the_same_model_build_backend_and_plan() {
        let original = config();
        let mut settings = measured(Some(original.clone()));
        assert!(!invalidate(&mut settings, "model.gguf", &original));
        assert_eq!(settings.context_budget_chars, Some(12_000));
        let mut build = original.clone();
        build.engine_build = "b2/windows-x86_64".into();
        let mut fallback = original.clone();
        fallback.accelerator = "CPU".into();
        fallback.gpu_layers = 0;
        let mut plan = original.clone();
        plan.context_tokens = 4096;
        for changed in [build, fallback, plan] {
            let mut settings = measured(Some(original.clone()));
            assert!(invalidate(&mut settings, "model.gguf", &changed));
            assert!(settings.benchmark.is_none());
            assert!(settings.context_budget_chars.is_none());
        }
        assert!(invalidate(&mut settings, "other.gguf", &original));
    }

    #[test]
    fn legacy_measurements_load_but_are_invalidated_before_use() {
        let mut settings = measured(None);
        let mut json = serde_json::to_value(&settings).unwrap();
        json["benchmark"].as_object_mut().unwrap().remove("runtime");
        settings = serde_json::from_value(json).unwrap();
        assert!(invalidate(&mut settings, "model.gguf", &config()));
        assert!(settings.context_budget_chars.is_none());
        assert!(!invalidate(&mut settings, "model.gguf", &config()));
    }

    #[test]
    fn calibration_reserves_idle_generation_and_releases_on_drop() {
        let counter = std::sync::atomic::AtomicUsize::new(1);
        assert!(super::super::stream::GenerationSlot::try_acquire(&counter).is_none());
        counter.store(0, Ordering::Relaxed);
        let slot = super::super::stream::GenerationSlot::try_acquire(&counter).unwrap();
        assert_eq!(counter.load(Ordering::Relaxed), 1);
        assert!(super::super::stream::GenerationSlot::try_acquire(&counter).is_none());
        drop(slot);
        assert_eq!(counter.load(Ordering::Relaxed), 0);
    }
}
