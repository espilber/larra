//! Command-shaped flows without a Tauri webview: the same core calls
//! `thread_*`, `settings_*`, `document_*`, and redact use.

mod common;

use common::*;
use larra::chat::conversations::Conversations;
use larra::settings::Settings;

#[test]
fn thread_lifecycle_matches_the_chat_commands() {
    let app = test_app();
    let shelf_id = make_shelf(&app, "Legal");

    let thread = Conversations::create(&app.ctx.paths, Some(shelf_id.clone())).unwrap();
    assert_eq!(thread.shelf_id.as_deref(), Some(shelf_id.as_str()));
    assert_eq!(Conversations::list(&app.ctx.paths).len(), 1);
    assert!(Conversations::messages(&app.ctx.paths, &thread.id).is_empty());

    Conversations::set_shelf(&app.ctx.paths, &thread.id, None).unwrap();
    assert_eq!(
        Conversations::get(&app.ctx.paths, &thread.id)
            .unwrap()
            .shelf_id,
        None
    );

    Conversations::delete(&app.ctx.paths, &thread.id).unwrap();
    app.ctx.search.remove_thread(&thread.id).unwrap();
    assert!(Conversations::list(&app.ctx.paths).is_empty());
}

#[test]
fn delete_thread_purges_conversation_uploads() {
    let app = test_app();
    let thread = Conversations::create(&app.ctx.paths, None).unwrap();
    let shelf = {
        let mut library = app.ctx.library.write().unwrap();
        library
            .ensure_conversation_shelf(&app.ctx.paths, &thread.id)
            .unwrap()
    };
    Conversations::set_upload_shelf(&app.ctx.paths, &thread.id, shelf.id.clone()).unwrap();
    let note = shelf.managed_path.join("note.md");
    std::fs::write(&note, "secret").unwrap();

    larra::chat::delete_thread(&app.ctx, &thread.id).unwrap();

    assert!(Conversations::list(&app.ctx.paths).is_empty());
    assert!(app.ctx.library.read().unwrap().shelf(&shelf.id).is_none());
    assert!(!note.exists());
}

#[test]
fn house_rules_and_onboarding_persist() {
    let app = test_app();
    {
        let mut settings = app.ctx.settings.write().unwrap();
        settings.house_rules = "Answer in Catalan.".into();
        settings.onboarding_done = true;
    }
    app.ctx.save_settings();

    let loaded = Settings::load(&app.ctx.paths.settings_path());
    assert_eq!(loaded.house_rules, "Answer in Catalan.");
    assert!(loaded.onboarding_done);
}

#[tokio::test(flavor = "multi_thread")]
async fn document_card_and_text_after_ingest() {
    let app = test_app();
    let shelf_id = make_shelf(&app, "Contracts");
    let meta = ingest_file(&app, &shelf_id, &fixture_contract_md(app.dir.path())).await;

    let mut docs = app.ctx.library.read().unwrap().documents(&shelf_id);
    docs.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    assert_eq!(docs[0].id, meta.id);

    let card = larra::ingest::card::read_card(&app.ctx.paths.card_path(&shelf_id, &meta.id))
        .expect("card");
    assert_eq!(card.schema, "larra-card/v1");

    let text = std::fs::read_to_string(app.ctx.paths.extracted_path(&shelf_id, &meta.id)).unwrap();
    assert!(text.contains("ninety (90) days"));
}

#[test]
fn redact_and_pii_helpers_use_the_local_scanner() {
    let app = test_app();
    let text = "Pay ES7620770024003102575766 and email maria@example.com.";
    assert!(app.ctx.pii.contains_pii(text));
    let redacted = app.ctx.pii.redact(text);
    assert!(!redacted.contains("ES7620770024003102575766"));
    assert!(!redacted.contains("maria@example.com"));
    assert!(!app.ctx.pii.contains_pii("no identifiers here"));
}
