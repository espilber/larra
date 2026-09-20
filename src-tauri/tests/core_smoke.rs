//! End-to-end core smoke: shelf → ingest → retrieval gate → real
//! llama-server (pinned build) → streamed, cited answer → conversation
//! memory. This is the whole product loop minus the UI.
//!
//! Needs two env vars so it stays offline and deterministic:
//!   REBOST_ENGINE_ARCHIVE — path to the host llama.cpp archive from engine/pin.rs
//!   REBOST_TEST_MODEL     — path to a small chat GGUF
//! Without them the test is skipped.

mod common;

use common::*;
use larra::chat::ChatService;
use larra::engine::{Engine, EngineState};
use larra::settings::ActiveModel;

#[ignore = "needs REBOST_ENGINE_ARCHIVE and REBOST_TEST_MODEL"]
#[tokio::test(flavor = "multi_thread")]
async fn full_loop_answers_with_citations() {
    let engine_archive = std::env::var("REBOST_ENGINE_ARCHIVE")
        .expect("REBOST_ENGINE_ARCHIVE must point at the pinned llama.cpp archive");
    let model_path = std::env::var("REBOST_TEST_MODEL")
        .expect("REBOST_TEST_MODEL must point at a small chat GGUF");
    // The engine manager reads this env var itself for offline installs.
    // (set by the caller; asserted here)
    assert!(std::path::Path::new(&engine_archive).exists());
    assert!(std::path::Path::new(&model_path).exists());

    let app = test_app();

    // Ingest a shelf file so retrieval has something to cite.
    let shelf_id = make_shelf(&app, "Contracts");
    let meta = ingest_file(&app, &shelf_id, &fixture_contract_md(app.dir.path())).await;
    assert_eq!(meta.status, larra::types::DocStatus::Ready);

    // Activate a local GGUF. Download is covered in the app, not here.
    let model_file = "smoke-model.gguf".to_string();
    std::fs::create_dir_all(app.ctx.paths.models_dir()).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&model_path, app.ctx.paths.models_dir().join(&model_file)).unwrap();
    #[cfg(windows)]
    if std::fs::hard_link(&model_path, app.ctx.paths.models_dir().join(&model_file)).is_err() {
        std::fs::copy(&model_path, app.ctx.paths.models_dir().join(&model_file)).unwrap();
    }
    {
        let mut settings = app.ctx.settings.write().unwrap();
        settings.active_model = Some(ActiveModel {
            file: model_file.clone(),
            name: "Smoke test model".into(),
            source: "local".into(),
            reference: "local/smoke".into(),
            license: Some("Apache-2.0".into()),
            size_bytes: std::fs::metadata(&model_path).map(|m| m.len()).unwrap_or(0),
        });
    }

    // Start llama-server from the pinned local archive.
    let engine = Engine::new(app.ctx.clone());
    let base = engine.ensure_ready().await.expect("engine ready");
    assert!(base.starts_with("http://127.0.0.1:"));
    assert_eq!(engine.status().state, EngineState::Ready);

    // Ask something the agreement actually answers.
    let chat = ChatService::new(app.ctx.clone(), engine.clone());
    let thread =
        larra::chat::conversations::Conversations::create(&app.ctx.paths, Some(shelf_id.clone()))
            .unwrap();
    let answer = chat
        .send_message(
            &thread.id,
            "According to the agreement, how many days of written notice are needed to terminate?",
            Some(shelf_id.clone()),
        )
        .await
        .expect("chat answer");

    assert_eq!(answer.status, "done");
    assert!(
        answer.text.trim().len() > 10,
        "expected a real answer, got: {:?}",
        answer.text
    );
    // No invented citation markers survive sanitization.
    let re = regex::Regex::new(r"\[S(\d+)\]").unwrap();
    for capture in re.captures_iter(&answer.text) {
        let sid = format!("S{}", &capture[1]);
        assert!(
            answer.sources.iter().any(|s| s.sid == sid),
            "answer cites {sid} which was not provided"
        );
    }
    eprintln!(
        "─ answer ─\n{}\n─ sources: {:?}",
        answer.text,
        answer.sources.len()
    );

    // The exchange is indexed so later turns can find it.
    let memories = app
        .ctx
        .search
        .search_messages("written notice to terminate", None, None, &[], 8)
        .unwrap();
    assert!(
        !memories.is_empty(),
        "conversation messages must reach the memory index"
    );

    // With no shelf selected, chat still answers.
    let thread2 = larra::chat::conversations::Conversations::create(&app.ctx.paths, None).unwrap();
    let general = chat
        .send_message(&thread2.id, "In one short sentence: what is EBITDA?", None)
        .await
        .expect("general answer");
    assert_eq!(general.status, "done");
    assert!(general.sources.is_empty(), "No Shelf → no document sources");
    assert!(general.text.trim().len() > 10);
    eprintln!("─ general ─\n{}", general.text);

    engine.stop().await;
}
