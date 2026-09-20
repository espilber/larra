//! Offline, real-model quality gate. See docs/experience-quality.md for inputs and thresholds.
mod common;

use common::*;
use larra::chat::{conversations::Conversations, ChatService};
use larra::core::{Ctx, Events};
use larra::engine::Engine;
use larra::ingest::extract::ExtractorSettings;
use larra::settings::ActiveModel;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Default)]
struct Timeline(Mutex<Vec<(Instant, Value)>>);
impl Events for Timeline {
    fn emit(&self, name: &str, value: Value) {
        if name == "larra://chat" {
            self.0.lock().unwrap().push((Instant::now(), value));
        }
    }
}
fn threshold(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|v| *v > 0)
        .unwrap_or(default)
}

#[ignore = "requires a local pinned engine archive and GGUF; run scripts/quality-gate.mjs"]
#[tokio::test(flavor = "multi_thread")]
async fn real_model_experience_gate() {
    if let Ok(path) = std::env::var("REBOST_QUALITY_REPORT") {
        std::fs::write(path, "{\"status\":\"running\"}").unwrap();
    }
    let model = std::path::PathBuf::from(std::env::var("REBOST_TEST_MODEL").expect("local GGUF"));
    let archive = std::env::var("REBOST_ENGINE_ARCHIVE").expect("local pinned archive");
    assert!(std::path::Path::new(&archive).is_file());
    let dir = tempfile::tempdir().unwrap();
    let events = Arc::new(Timeline::default());
    let ctx = Ctx::new(
        larra::paths::Paths::new(dir.path().join("data")),
        events.clone(),
        ExtractorSettings::default(),
    )
    .unwrap();
    let app = TestApp {
        ctx: ctx.clone(),
        dir,
    };
    let shelf = make_shelf(&app, "Quality fixtures");
    let path = app.dir.path().join("Handbook.md");
    let text = format!("# Handbook\n\n{}\n## Delivery / Entrega / Lliurament\n\nThe delivery code is 7391. El código de entrega es 7391. El codi de lliurament és 7391.\n", "General background about the office kitchen and shared equipment.\n\n".repeat(600));
    std::fs::write(&path, &text).unwrap();
    let reading = Instant::now();
    let doc = ingest_file(&app, &shelf, &path).await;
    assert_eq!(doc.status, larra::types::DocStatus::Ready);
    let ingest_ms = reading.elapsed().as_millis();
    assert!(ingest_ms < threshold("REBOST_MAX_INGEST_MS", 30_000) as u128);
    let dest = ctx.paths.models_dir().join("quality.gguf");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&model, &dest).unwrap();
    #[cfg(windows)]
    if std::fs::hard_link(&model, &dest).is_err() {
        std::fs::copy(&model, &dest).unwrap();
    }
    ctx.settings.write().unwrap().active_model = Some(ActiveModel {
        file: "quality.gguf".into(),
        name: "Quality model".into(),
        source: "local".into(),
        reference: "local/quality".into(),
        license: None,
        size_bytes: model.metadata().unwrap().len(),
    });
    let machine = larra::engine::catalog::MachineProfile::detect(ctx.paths.base());
    let engine = Engine::new(ctx.clone());
    let warm = Instant::now();
    engine.ensure_ready().await.expect("engine ready");
    let warm_ms = warm.elapsed().as_millis();
    let chat = ChatService::new(ctx.clone(), engine.clone());
    let mut cases = Vec::new();
    for (language, question) in [
        (
            "en",
            "In Handbook.md, what is the delivery code? Answer with the code and a citation.",
        ),
        (
            "es",
            "Según Handbook.md, ¿cuál es el código de entrega? Responde con el código y una cita.",
        ),
        (
            "ca",
            "Segons Handbook.md, quin és el codi de lliurament? Respon amb el codi i una citació.",
        ),
    ] {
        let thread = Conversations::create(&ctx.paths, Some(shelf.clone())).unwrap();
        let started = Instant::now();
        let answer = tokio::time::timeout(
            Duration::from_secs(180),
            chat.send_message(&thread.id, question, Some(shelf.clone())),
        )
        .await
        .expect("answer deadline")
        .unwrap();
        let ttft = events
            .0
            .lock()
            .unwrap()
            .iter()
            .find(|(time, event)| {
                *time >= started && event["threadId"] == thread.id && event["kind"] == "delta"
            })
            .map(|(time, _)| time.duration_since(started).as_millis())
            .expect("visible answer delta");
        eprintln!("QUALITY_CASE {language}: {}", answer.text);
        assert_eq!(answer.status, "done", "{language}: {}", answer.text);
        assert!(
            answer.text.contains("7391"),
            "grounded fact: {language}: {}",
            answer.text
        );
        assert!(
            !answer.sources.is_empty(),
            "citation required: {language}: {}",
            answer.text
        );
        let extracted = std::fs::read_to_string(ctx.paths.extracted_path(&shelf, &doc.id)).unwrap();
        for source in &answer.sources {
            let anchor = source.anchor.as_ref().expect("saved citation anchor");
            assert_eq!(anchor.hash, doc.hash, "file version");
            let start = anchor.start_char.expect("exact start") as usize;
            let end = anchor.end_char.expect("exact end") as usize;
            assert_eq!(
                extracted
                    .chars()
                    .skip(start)
                    .take(end - start)
                    .collect::<String>(),
                anchor.quote
            );
            assert!(
                anchor.quote.contains("7391"),
                "citation supports the answer"
            );
        }
        assert!(
            ttft < threshold("REBOST_MAX_TTFT_MS", 30_000) as u128,
            "{language}: TTFT={ttft}ms"
        );
        cases.push(json!({"language": language, "ttft_ms": ttft, "total_ms": started.elapsed().as_millis(), "grounded": true, "exact_citations": answer.sources.len()}));
    }
    // Cancellation is timed after the first visible token of a real streamed answer.
    let thread = Conversations::create(&ctx.paths, None).unwrap();
    let chat_task = chat.clone();
    let thread_id = thread.id.clone();
    let running = tokio::spawn(async move {
        chat_task
            .send_message(
                &thread_id,
                "Write a very long detailed guide to maintaining an office kitchen.",
                None,
            )
            .await
    });
    let id = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let found = events
                .0
                .lock()
                .unwrap()
                .iter()
                .find(|(_, e)| e["threadId"] == thread.id && e["kind"] == "delta")
                .and_then(|(_, e)| e["messageId"].as_str().map(str::to_string));
            if let Some(id) = found {
                break id;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("streaming event");
    let stopped = Instant::now();
    chat.cancel(&id);
    let result = tokio::time::timeout(Duration::from_secs(2), running)
        .await
        .expect("Stop deadline")
        .unwrap()
        .unwrap();
    let cancel_ms = stopped.elapsed().as_millis();
    assert_eq!(result.status, "stopped");
    assert!(
        cancel_ms < threshold("REBOST_MAX_CANCEL_MS", 250) as u128,
        "cancel={cancel_ms}ms"
    );
    engine.stop().await;
    let report = json!({"schema":1,"status":"passed","platform":std::env::consts::OS,"architecture":std::env::consts::ARCH,"model_bytes":model.metadata().unwrap().len(),"model_file":model.file_name().unwrap().to_string_lossy(),"machine":machine,"engine_release":larra::engine::ENGINE_RELEASE,"warmup_ms":warm_ms,"ingest_ms":ingest_ms,"ingest_bytes":text.len(),"cancel_ms":cancel_ms,"cases":cases});
    println!("EXPERIENCE_QUALITY {report}");
    if let Ok(path) = std::env::var("REBOST_QUALITY_REPORT") {
        std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    }
}
