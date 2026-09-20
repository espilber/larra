use super::*;
use crate::core::NoopEvents;
use crate::engine::ToolCall;
use crate::ingest::extract::ExtractorSettings;
use crate::ingest::ProcessJob;
use crate::paths::Paths;
use crate::types::SourceType;

struct Fixture {
    ctx: Arc<Ctx>,
    shelf: crate::shelf::Shelf,
    _dir: tempfile::TempDir,
}

async fn shelf_with_file(name: &str, body: &str) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("Shelves");
    let paths = Paths::new(dir.path().join("appdata"));
    let ctx = Ctx::new(paths, Arc::new(NoopEvents), ExtractorSettings::default()).unwrap();
    let shelf = {
        let mut library = crate::core::write_lock(&ctx.library);
        library.create_shelf(&ctx.paths, "Notes", &root).unwrap()
    };
    let file = dir.path().join(name);
    std::fs::write(&file, body).unwrap();
    let copied =
        crate::shelf::import_into_shelf(&shelf, &[file], crate::shelf::MAX_FILES_PER_SHELF)
            .unwrap()
            .files;
    let abs = copied.into_iter().next().unwrap();
    let rel = abs
        .strip_prefix(&shelf.managed_path)
        .unwrap()
        .to_string_lossy()
        .to_string();
    crate::ingest::process_file(
        &ctx,
        &ProcessJob {
            shelf_id: shelf.id.clone(),
            source_id: crate::shelf::Shelf::IMPORTED_SOURCE.to_string(),
            source_type: SourceType::Imported,
            source_label: "Imported".into(),
            abs_path: abs,
            rel_path: rel,
            force: false,
            epoch: 0,
        },
    )
    .await
    .unwrap();
    Fixture {
        ctx,
        shelf,
        _dir: dir,
    }
}

/// Qwen and friends raise `System message must be at the beginning` for a
/// system turn anywhere but index 0, and the request comes back a 500.
fn assert_no_late_system_message(messages: &[ChatMessage]) {
    assert!(
        !messages.iter().skip(1).any(|m| m.role == "system"),
        "a system message after the first one is rejected by strict templates, got {:?}",
        messages.iter().map(|m| m.role.as_str()).collect::<Vec<_>>()
    );
}

fn tool_ctx<'a>(
    ctx: &'a Ctx,
    thread_id: &'a str,
    shelf_id: &'a str,
    files: &'a [tools::ShelfFile],
    sources: &'a [SourcePassage],
    budget: usize,
) -> tools::ToolCtx<'a> {
    tools::ToolCtx {
        ctx,
        thread_id,
        shelf_id: Some(shelf_id),
        upload_shelf_id: None,
        files,
        sources,
        cited: &[],
        budget,
        think: ThinkLevel::Off,
        allowed: tools::ToolSet::new(
            true,
            !sources.is_empty(),
            false,
            true,
            &tools::ToolUse::default(),
        ),
        cancel: idle_cancel(),
        open_next: HashMap::new(),
        exclude_message_ids: &[],
    }
}

fn tool_ctx_used<'a>(
    ctx: &'a Ctx,
    thread_id: &'a str,
    shelf_id: &'a str,
    files: &'a [tools::ShelfFile],
    sources: &'a [SourcePassage],
    budget: usize,
    used: &'a tools::ToolUse,
) -> tools::ToolCtx<'a> {
    tools::ToolCtx {
        ctx,
        thread_id,
        shelf_id: Some(shelf_id),
        upload_shelf_id: None,
        files,
        sources,
        cited: &[],
        budget,
        think: ThinkLevel::Off,
        allowed: tools::ToolSet::new(true, !sources.is_empty(), false, true, used),
        cancel: idle_cancel(),
        open_next: used.open_next.clone(),
        exclude_message_ids: &[],
    }
}

fn idle_cancel() -> &'static AtomicBool {
    static FLAG: AtomicBool = AtomicBool::new(false);
    &FLAG
}

#[tokio::test(flavor = "multi_thread")]
async fn prepare_turn_puts_shelf_passages_in_the_prompt() {
    let fixture = shelf_with_file(
        "handbook.md",
        "# Staff handbook\n\nThe office kitchen is restocked every Tuesday morning.\n",
    )
    .await;
    let thread = Conversations::create(&fixture.ctx.paths, Some(fixture.shelf.id.clone())).unwrap();
    let prepared = prepare_turn(
        &fixture.ctx,
        &thread.id,
        "When is the kitchen restocked?",
        Some(&fixture.shelf.id),
        None,
        "m_none",
        &[],
        ThinkLevel::Off,
        &[],
    )
    .unwrap();
    assert!(
        !prepared.sources.is_empty(),
        "expected retrieved passages, got none"
    );
    let user = prepared
        .messages
        .iter()
        .find(|m| m.role == "user")
        .expect("user message");
    assert_eq!(user.as_text(), "When is the kitchen restocked?");
    assert!(
        prepared
            .messages
            .iter()
            .filter(|m| m.role == "tool")
            .any(|m| m.as_text().to_lowercase().contains("tuesday")),
        "retrieved files should sit in a tool result, not the question"
    );
    assert_no_late_system_message(&prepared.messages);
    let system = prepared.messages[0].as_text();
    assert!(system.contains("Notes"));
    assert_eq!(
        prepared.sources.len(),
        1,
        "a short file should be sent whole, not searched"
    );
    assert!(prepared.sources[0].body.contains("Staff handbook"));
    assert!(prepared.sources[0].body.to_lowercase().contains("tuesday"));
    assert!(!user.as_text().contains("OLDER CONVERSATION NOTES"));
    assert!(system.contains("You are"));
    assert!(!system.contains("search_chats"));
}

#[tokio::test(flavor = "multi_thread")]
async fn prepare_turn_keeps_the_library_shelf_and_appends_uploads() {
    let fixture = shelf_with_file(
        "handbook.md",
        "# Staff handbook\n\nThe office kitchen is restocked every Tuesday morning.\n",
    )
    .await;
    let thread = Conversations::create(&fixture.ctx.paths, Some(fixture.shelf.id.clone())).unwrap();
    let upload = {
        let mut library = crate::core::write_lock(&fixture.ctx.library);
        library
            .ensure_conversation_shelf(&fixture.ctx.paths, &thread.id)
            .unwrap()
    };
    let dest = upload.managed_path.join("invoice.md");
    std::fs::write(
        &dest,
        "# Invoice\n\nThe attached invoice total is 480 euros.\n",
    )
    .unwrap();
    crate::ingest::process_file(
        &fixture.ctx,
        &ProcessJob {
            shelf_id: upload.id.clone(),
            source_id: crate::shelf::Shelf::IMPORTED_SOURCE.to_string(),
            source_type: SourceType::Imported,
            source_label: "Imported".into(),
            abs_path: dest,
            rel_path: "invoice.md".into(),
            force: false,
            epoch: 0,
        },
    )
    .await
    .unwrap();

    let prepared = prepare_turn(
        &fixture.ctx,
        &thread.id,
        "When is the kitchen restocked, and what is the invoice total?",
        Some(&fixture.shelf.id),
        Some(&upload.id),
        "m_none",
        &[],
        ThinkLevel::Off,
        &[],
    )
    .unwrap();
    assert!(
        prepared
            .sources
            .iter()
            .any(|s| s.body.to_lowercase().contains("tuesday")),
        "library shelf should still be retrieved"
    );
    assert!(
        prepared
            .sources
            .iter()
            .any(|s| s.body.to_lowercase().contains("480")),
        "attached file should be included"
    );
    assert!(
        prepared.sources.iter().any(|s| s.shelf_id == upload.id),
        "attached file should be in sources"
    );
    assert!(
        prepared
            .sources
            .iter()
            .any(|s| s.shelf_id == fixture.shelf.id),
        "library shelf should be in sources"
    );
    assert!(prepared.messages[0].as_text().contains("Notes"));
    assert!(prepared.messages[0].as_text().contains("Uploaded files"));
}

#[tokio::test(flavor = "multi_thread")]
async fn prepare_turn_keeps_the_upload_to_its_share() {
    let fixture = shelf_with_file(
        "handbook.md",
        "# Staff handbook\n\nThe office kitchen is restocked every Tuesday morning.\n",
    )
    .await;
    let thread = Conversations::create(&fixture.ctx.paths, Some(fixture.shelf.id.clone())).unwrap();
    let mut upload = None;
    for i in 0..6 {
        let mut body = format!("# Zebra note {i}\n\n");
        body.push_str(
            &format!("The office zebra in building {i} lives in the east wing.\n").repeat(40),
        );
        upload = Some(add_upload_file(&fixture, &thread.id, &format!("zebra-{i}.md"), &body).await);
    }
    let upload = upload.expect("upload shelf");
    let prepared = prepare_turn(
        &fixture.ctx,
        &thread.id,
        "Where does the office zebra live, and when is the kitchen restocked?",
        Some(&fixture.shelf.id),
        Some(&upload.id),
        "m_none",
        &[],
        ThinkLevel::Off,
        &[],
    )
    .unwrap();
    let budget = prepared.budget;
    let share = focus::upload_budget_chars(budget, true, false, false);
    let upload_cost: usize = prepared
        .sources
        .iter()
        .filter(|s| s.shelf_id == upload.id)
        .map(gate::passage_cost)
        .sum();
    assert!(
        upload_cost <= share,
        "attached files should leave room for the Shelf, cost={upload_cost} share={share} budget={budget}"
    );
    assert!(
        prepared
            .sources
            .iter()
            .any(|s| s.shelf_id == fixture.shelf.id && s.body.to_lowercase().contains("tuesday")),
        "library Shelf should still be retrieved"
    );
}

#[test]
fn stuffed_file_cost_includes_title_and_wrapper() {
    assert_eq!(super::stuffed_file_cost("ab", 3), 2 + 3 + 64);
}

#[test]
fn reading_status_only_when_a_shelf_is_in_play() {
    assert!(!super::should_emit_reading(None, None));
    assert!(super::should_emit_reading(Some("s_lib"), None));
    assert!(super::should_emit_reading(None, Some("s_up")));
    assert!(super::should_emit_reading(Some("s_lib"), Some("s_up")));
}

#[test]
fn turn_shelf_ids_drops_a_library_id_that_is_the_upload_shelf() {
    assert_eq!(
        super::turn_shelf_ids(Some("s_lib"), Some("s_up")),
        (Some("s_lib"), Some("s_up"))
    );
    assert_eq!(
        super::turn_shelf_ids(Some("s_up"), Some("s_up")),
        (None, Some("s_up"))
    );
    assert_eq!(
        super::turn_shelf_ids(None, Some("s_up")),
        (None, Some("s_up"))
    );
}

#[test]
fn retrieve_plan_matches_off_light_deep() {
    let off = retrieve_plan(ThinkLevel::Off);
    assert_eq!(off.extra_queries, 0);
    assert_eq!(off.expand_top, 0);
    assert_eq!(off.neighbor_radius, neighbors::OFF_RADIUS_CHARS);

    let light = retrieve_plan(ThinkLevel::Light);
    assert_eq!(light.extra_queries, 3);
    assert_eq!(light.expand_top, 0);
    assert_eq!(light.neighbor_radius, neighbors::LIGHT_RADIUS_CHARS);
    assert_eq!(light.caps.relative_floor, 0.15);

    let deep = retrieve_plan(ThinkLevel::Deep);
    assert_eq!(deep.extra_queries, 3);
    assert_eq!(deep.expand_top, 2);
    assert_eq!(deep.search_limit, 32);
    assert_eq!(deep.neighbor_radius, neighbors::DEEP_RADIUS_CHARS);
    assert_eq!(deep.caps.max_passages, 12);
}

#[tokio::test(flavor = "multi_thread")]
async fn prepare_turn_searches_when_the_file_is_too_long() {
    let mut body = String::from("# Encyclopedia\n\n");
    body.push_str(&"The office zebra lives in the east wing.\n".repeat(800));
    let fixture = shelf_with_file("encyclopedia.md", &body).await;
    let thread = Conversations::create(&fixture.ctx.paths, Some(fixture.shelf.id.clone())).unwrap();
    let prepared = prepare_turn(
        &fixture.ctx,
        &thread.id,
        "Where does the office zebra live?",
        Some(&fixture.shelf.id),
        None,
        "m_none",
        &[],
        ThinkLevel::Off,
        &[],
    )
    .unwrap();
    assert!(
        !prepared.sources.is_empty(),
        "expected retrieved passages, got none"
    );
    let extracted = std::fs::read_to_string(
        fixture
            .ctx
            .paths
            .extracted_path(&fixture.shelf.id, &prepared.sources[0].document_id),
    )
    .unwrap();
    assert!(extracted.chars().count() > 20_000);
    assert!(
        prepared
            .sources
            .iter()
            .all(|s| s.body.chars().count() < extracted.chars().count()),
        "long files should be searched, not stuffed whole"
    );
    assert!(prepared
        .sources
        .iter()
        .any(|s| s.body.to_lowercase().contains("zebra")));
}

#[tokio::test(flavor = "multi_thread")]
async fn prepare_turn_fuses_extra_queries_when_the_file_is_long() {
    let mut body = String::from("# Encyclopedia\n\n");
    body.push_str(&"The office zebra lives in the east wing.\n".repeat(800));
    let fixture = shelf_with_file("encyclopedia.md", &body).await;
    let thread = Conversations::create(&fixture.ctx.paths, Some(fixture.shelf.id.clone())).unwrap();
    let prepared = prepare_turn(
        &fixture.ctx,
        &thread.id,
        "Where does it live?",
        Some(&fixture.shelf.id),
        None,
        "m_none",
        &["office zebra east wing".into()],
        ThinkLevel::Light,
        &[],
    )
    .unwrap();
    assert!(prepared
        .sources
        .iter()
        .any(|s| s.body.to_lowercase().contains("zebra")));
}

async fn add_library_file(fixture: &Fixture, name: &str, body: &str) {
    let dest = fixture.shelf.managed_path.join(name);
    std::fs::write(&dest, body).unwrap();
    crate::ingest::process_file(
        &fixture.ctx,
        &ProcessJob {
            shelf_id: fixture.shelf.id.clone(),
            source_id: crate::shelf::Shelf::IMPORTED_SOURCE.to_string(),
            source_type: SourceType::Imported,
            source_label: "Imported".into(),
            abs_path: dest,
            rel_path: name.into(),
            force: false,
            epoch: 0,
        },
    )
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn prepare_turn_keeps_citation_ids_on_a_short_follow_up() {
    let mut mac = String::from("# Mac Employee Starter Guide\n\n");
    mac.push_str("Enroll into corporate services before using the Mac.\n");
    mac.push_str(&"Mac starter padding so the file is searched.\n".repeat(400));
    let mut exam = String::from("# B2 First Handbook for teachers\n\n");
    exam.push_str("Key terms for the exam and candidate answers.\n");
    exam.push_str(&"Exam handbook padding so the file is searched.\n".repeat(400));
    let fixture = shelf_with_file("Mac Employee Starter Guide.md", &mac).await;
    add_library_file(&fixture, "B2 First Handbook.md", &exam).await;
    let thread = Conversations::create(&fixture.ctx.paths, Some(fixture.shelf.id.clone())).unwrap();
    let first = prepare_turn(
        &fixture.ctx,
        &thread.id,
        "List the key terms from the Mac Employee Starter Guide",
        Some(&fixture.shelf.id),
        None,
        "m_first",
        &[],
        ThinkLevel::Off,
        &[],
    )
    .unwrap();
    let mac = first
        .sources
        .iter()
        .find(|source| source.title.contains("Mac") || source.body.contains("corporate services"))
        .expect("first turn should retrieve the Mac guide");
    let mac_id = mac.document_id.clone();
    let mac_sid = mac.sid.clone();
    Conversations::append(
        &fixture.ctx.paths,
        &thread.id,
        &user_line("List the key terms from the Mac Employee Starter Guide"),
    )
    .unwrap();
    let mut answer = user_line("Here are the terms. [S1]");
    answer.role = "assistant".into();
    answer.sources = first.sources.clone();
    conversations::compact_message(&mut answer);
    Conversations::append(&fixture.ctx.paths, &thread.id, &answer).unwrap();

    let follow = prepare_turn(
        &fixture.ctx,
        &thread.id,
        "shorten to only 5 terms",
        Some(&fixture.shelf.id),
        None,
        "m_follow",
        &[],
        ThinkLevel::Off,
        &[],
    )
    .unwrap();
    let mac = follow
        .sources
        .iter()
        .find(|source| source.document_id == mac_id)
        .expect("follow-up should keep the Mac guide");
    assert_eq!(mac.sid, mac_sid);
    let history = follow
        .messages
        .iter()
        .find(|message| message.role == "assistant")
        .expect("history should include the previous answer");
    assert!(
        history.as_text().contains(&mac_sid) && history.as_text().contains("Mac"),
        "history should name the file that [S1] referred to, got {}",
        history.as_text()
    );
    assert_no_late_system_message(&follow.messages);
    let retrieval = follow
        .messages
        .last()
        .expect("follow-up should end with the retrieved passages");
    assert_eq!(retrieval.role, "tool");
    assert!(
        retrieval.as_text().contains(&mac.sid) && retrieval.as_text().contains(&mac.body),
        "the tool result should carry the Mac excerpt that stayed in sources, got {}",
        retrieval.as_text()
    );
    let call = follow.messages[follow.messages.len() - 2]
        .tool_calls
        .as_ref()
        .expect("a tool result needs the call that produced it");
    assert_eq!(call[0].function.name, tools::SEARCH_SHELF);
    assert_eq!(retrieval.tool_call_id.as_deref(), Some(call[0].id.as_str()));
}

#[tokio::test(flavor = "multi_thread")]
async fn prepare_turn_does_not_stuff_earlier_chats() {
    let fixture = shelf_with_file(
        "handbook.md",
        "# Staff handbook\n\nThe office kitchen is restocked every Tuesday morning.\n",
    )
    .await;
    let now = chrono::Utc::now();
    fixture
        .ctx
        .search
        .index_message(
            "other_thread",
            "m_old",
            "user",
            "The office kitchen is restocked every Tuesday morning.",
            Some("en"),
            now,
        )
        .unwrap();
    let thread = Conversations::create(&fixture.ctx.paths, Some(fixture.shelf.id.clone())).unwrap();
    let prepared = prepare_turn(
        &fixture.ctx,
        &thread.id,
        "When is the kitchen restocked?",
        Some(&fixture.shelf.id),
        None,
        "m_none",
        &[],
        ThinkLevel::Off,
        &[],
    )
    .unwrap();
    let user = prepared
        .messages
        .iter()
        .find(|m| m.role == "user")
        .expect("user message");
    assert!(!user.as_text().contains("OLDER CONVERSATION NOTES"));
}

#[tokio::test(flavor = "multi_thread")]
async fn open_shelf_file_loads_a_long_file_into_sources() {
    let mut body = String::from("# Encyclopedia\n\n");
    body.push_str(&"The office zebra lives in the east wing.\n".repeat(800));
    let fixture = shelf_with_file("encyclopedia.md", &body).await;
    let files = super::tools::catalog(&fixture.ctx, &fixture.shelf.id);
    assert_eq!(files.len(), 1);
    let tool = tool_ctx(&fixture.ctx, "t", &fixture.shelf.id, &files, &[], 12_000);
    let outcome = super::tools::open_shelf_file(&tool, "encyclopedia.md");
    let source = match outcome.change {
        tools::SourceChange::OpenWindow { opened, .. } => opened,
        other => panic!("expected opened file, got {other:?}"),
    };
    assert_eq!(source.sid, "S1");
    assert!(source.body.to_lowercase().contains("zebra"));
    assert!(outcome.message.contains("[S1]"));
    assert!(matches!(
        super::tools::open_shelf_file(&tool, "no-such.md").change,
        tools::SourceChange::None
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn search_shelf_adds_a_fresh_excerpt() {
    let mut body = String::from("# Encyclopedia\n\n");
    body.push_str(&"Padding text so the file is too long to send whole.\n".repeat(400));
    body.push_str("The office zebra lives in the east wing.\n");
    body.push_str(&"More padding after the zebra sentence.\n".repeat(400));
    let fixture = shelf_with_file("encyclopedia.md", &body).await;
    let files = super::tools::catalog(&fixture.ctx, &fixture.shelf.id);
    let tool = tool_ctx(&fixture.ctx, "t", &fixture.shelf.id, &files, &[], 12_000);
    let call = ToolCall::function(
        "c1",
        tools::SEARCH_SHELF,
        serde_json::json!({ "query": "office zebra" }).to_string(),
    );
    let outcome = tools::run_tool(&call, &tool).await;
    let tools::SourceChange::Append(added) = outcome.change else {
        panic!("expected new excerpts, got {:?}", outcome.change);
    };
    assert!(added
        .iter()
        .any(|s| s.body.to_lowercase().contains("zebra")));
    assert!(outcome.message.contains("[S1]"));
}

#[tokio::test(flavor = "multi_thread")]
async fn look_around_widens_an_excerpt() {
    let mut body = String::from("# Encyclopedia\n\n");
    body.push_str("alpha paragraph before the hit.\n\n");
    body.push_str("The office zebra lives in the east wing.\n\n");
    body.push_str("omega paragraph after the hit.\n");
    body.push_str(&"Padding so stuffing does not apply.\n".repeat(800));
    let fixture = shelf_with_file("encyclopedia.md", &body).await;
    let files = super::tools::catalog(&fixture.ctx, &fixture.shelf.id);
    let excerpt = SourcePassage {
        anchor: None,
        sid: "S1".into(),
        document_id: files[0].id.clone(),
        shelf_id: fixture.shelf.id.clone(),
        title: "encyclopedia.md".into(),
        section: None,
        page_start: None,
        page_end: None,
        body: "The office zebra lives in the east wing.".into(),
        path: files[0].path.clone(),
        score: 1.0,
    };
    let sources = [excerpt];
    let tool = tool_ctx(
        &fixture.ctx,
        "t",
        &fixture.shelf.id,
        &files,
        &sources,
        12_000,
    );
    let call = ToolCall::function(
        "c1",
        tools::LOOK_AROUND,
        serde_json::json!({ "id": "S1" }).to_string(),
    );
    let outcome = tools::run_tool(&call, &tool).await;
    let tools::SourceChange::ReplaceOne(updated) = outcome.change else {
        panic!("expected widened excerpt, got {:?}", outcome.change);
    };
    assert_eq!(updated.sid, "S1");
    assert!(updated.body.contains("alpha"));
    assert!(updated.body.contains("omega"));
    assert!(updated.body.len() > sources[0].body.len());
}

fn user_line(text: &str) -> StoredMessage {
    StoredMessage {
        id: crate::ids::message_id(),
        role: "user".into(),
        text: text.into(),
        thinking: None,
        activity: Vec::new(),
        ts: chrono::Utc::now().to_rfc3339(),
        shelf_id: None,
        sources: Vec::new(),
        status: "done".into(),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn search_chats_returns_notes_from_other_threads_on_the_same_shelf() {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::new(dir.path().join("appdata"));
    let ctx = Ctx::new(paths, Arc::new(NoopEvents), ExtractorSettings::default()).unwrap();
    let kitchen = Conversations::create(&ctx.paths, Some("s_kitchen".into())).unwrap();
    let current = Conversations::create(&ctx.paths, Some("s_kitchen".into())).unwrap();
    let office = Conversations::create(&ctx.paths, Some("s_office".into())).unwrap();
    let now = chrono::Utc::now();
    let kitchen_note = user_line("The office move budget is twelve thousand euros.");
    Conversations::append(&ctx.paths, &kitchen.id, &kitchen_note).unwrap();
    ctx.search
        .index_message(
            &kitchen.id,
            &kitchen_note.id,
            "user",
            &kitchen_note.text,
            Some("en"),
            now,
        )
        .unwrap();
    let office_note = user_line("The office move budget is ninety thousand euros.");
    Conversations::append(&ctx.paths, &office.id, &office_note).unwrap();
    ctx.search
        .index_message(
            &office.id,
            &office_note.id,
            "user",
            &office_note.text,
            Some("en"),
            now,
        )
        .unwrap();
    let files: Vec<tools::ShelfFile> = Vec::new();
    let tool = tools::ToolCtx {
        ctx: ctx.as_ref(),
        thread_id: &current.id,
        shelf_id: Some("s_kitchen"),
        upload_shelf_id: None,
        files: &files,
        sources: &[],
        cited: &[],
        budget: 9_000,
        think: ThinkLevel::Off,
        allowed: tools::ToolSet::new(false, false, false, true, &tools::ToolUse::default()),
        cancel: idle_cancel(),
        open_next: HashMap::new(),
        exclude_message_ids: &[],
    };
    let call = ToolCall::function(
        "c1",
        tools::SEARCH_CHATS,
        serde_json::json!({ "query": "office move budget" }).to_string(),
    );
    let outcome = tools::run_tool(&call, &tool).await;
    assert!(matches!(outcome.change, tools::SourceChange::None));
    assert!(outcome.message.to_lowercase().contains("twelve thousand"));
    assert!(!outcome.message.to_lowercase().contains("ninety"));
    assert!(outcome.message.contains("do not cite"));
}

#[tokio::test(flavor = "multi_thread")]
async fn search_chats_finds_earlier_turns_of_this_thread() {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::new(dir.path().join("appdata"));
    let ctx = Ctx::new(paths, Arc::new(NoopEvents), ExtractorSettings::default()).unwrap();
    let current = Conversations::create(&ctx.paths, None).unwrap();
    let now = chrono::Utc::now();
    let old = user_line("The office move budget is twelve thousand euros.");
    Conversations::append(&ctx.paths, &current.id, &old).unwrap();
    ctx.search
        .index_message(&current.id, &old.id, "user", &old.text, Some("en"), now)
        .unwrap();
    let recent = user_line("The office move budget is ninety thousand euros.");
    Conversations::append(&ctx.paths, &current.id, &recent).unwrap();
    ctx.search
        .index_message(
            &current.id,
            &recent.id,
            "user",
            &recent.text,
            Some("en"),
            now,
        )
        .unwrap();
    let files: Vec<tools::ShelfFile> = Vec::new();
    let skip = vec![recent.id.clone()];
    let tool = tools::ToolCtx {
        ctx: ctx.as_ref(),
        thread_id: &current.id,
        shelf_id: None,
        upload_shelf_id: None,
        files: &files,
        sources: &[],
        cited: &[],
        budget: 9_000,
        think: ThinkLevel::Off,
        allowed: tools::ToolSet::new(false, false, false, true, &tools::ToolUse::default()),
        cancel: idle_cancel(),
        open_next: HashMap::new(),
        exclude_message_ids: &skip,
    };
    let call = ToolCall::function(
        "c1",
        tools::SEARCH_CHATS,
        serde_json::json!({ "query": "office move budget" }).to_string(),
    );
    let outcome = tools::run_tool(&call, &tool).await;
    assert!(outcome.message.to_lowercase().contains("twelve thousand"));
    assert!(!outcome.message.to_lowercase().contains("ninety"));
}

async fn add_upload_file(
    fixture: &Fixture,
    thread_id: &str,
    name: &str,
    body: &str,
) -> crate::shelf::Shelf {
    let upload = {
        let mut library = crate::core::write_lock(&fixture.ctx.library);
        library
            .ensure_conversation_shelf(&fixture.ctx.paths, thread_id)
            .unwrap()
    };
    let dest = upload.managed_path.join(name);
    std::fs::write(&dest, body).unwrap();
    crate::ingest::process_file(
        &fixture.ctx,
        &ProcessJob {
            shelf_id: upload.id.clone(),
            source_id: crate::shelf::Shelf::IMPORTED_SOURCE.to_string(),
            source_type: SourceType::Imported,
            source_label: "Imported".into(),
            abs_path: dest,
            rel_path: name.into(),
            force: false,
            epoch: 0,
        },
    )
    .await
    .unwrap();
    upload
}

#[tokio::test(flavor = "multi_thread")]
async fn prepare_turn_covers_a_named_attached_file() {
    let fixture = shelf_with_file(
        "handbook.md",
        "# Staff handbook\n\nThe office kitchen is restocked every Tuesday morning.\n",
    )
    .await;
    let thread = Conversations::create(&fixture.ctx.paths, Some(fixture.shelf.id.clone())).unwrap();
    let mut body = String::from("# ALPHA unique start\n\n");
    body.push_str(&"The office zebra lives in the east wing.\n".repeat(800));
    body.push_str("\n# OMEGA unique end\n");
    let upload = add_upload_file(&fixture, &thread.id, "encyclopedia.md", &body).await;
    let doc_id = crate::core::read_lock(&fixture.ctx.library)
        .documents(&upload.id)
        .into_iter()
        .next()
        .unwrap()
        .id;
    let extracted_path = fixture.ctx.paths.extracted_path(&upload.id, &doc_id);
    std::fs::write(&extracted_path, &body).unwrap();
    assert!(
        std::fs::read_to_string(&extracted_path)
            .unwrap()
            .chars()
            .count()
            > 20_000,
        "test extract should be the long file"
    );
    let covered = super::focus::coverage_passages(&fixture.ctx, &upload.id, &doc_id, 9_000);
    let covered_text = covered
        .iter()
        .map(|s| s.body.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        covered_text.contains("ALPHA") && covered_text.contains("OMEGA"),
        "coverage_passages should sample both ends, got {covered_text:?}"
    );
    assert!(
        covered
            .iter()
            .map(|s| s.body.chars().count())
            .sum::<usize>()
            > 1_000,
        "coverage should spend the window, got {} passages / {} chars",
        covered.len(),
        covered
            .iter()
            .map(|s| s.body.chars().count())
            .sum::<usize>()
    );
    // A 4k machine (CI's 7 GB Linux runner) only has ~1k chars after
    // wrappers. The file should still keep both ends and spend the room.
    let tight_budget = 1_004;
    let tight = super::focus::coverage_passages(&fixture.ctx, &upload.id, &doc_id, tight_budget);
    let tight_text = tight
        .iter()
        .map(|s| s.body.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let tight_cost: usize = tight.iter().map(gate::passage_cost).sum();
    assert!(
        tight_text.contains("ALPHA") && tight_text.contains("OMEGA"),
        "a small window should still keep both ends, got {tight_text:?}"
    );
    assert!(
        tight_cost + 256 >= tight_budget,
        "a small window should still be spent, cost={tight_cost} budget={tight_budget}"
    );
    let prepared = prepare_turn(
        &fixture.ctx,
        &thread.id,
        "Summarize this doc: encyclopedia.md",
        Some(&fixture.shelf.id),
        Some(&upload.id),
        "m_none",
        &[],
        ThinkLevel::Off,
        &[doc_id],
    )
    .unwrap();
    let upload_text = prepared
        .sources
        .iter()
        .filter(|s| s.shelf_id == upload.id)
        .map(|s| s.body.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        upload_text.contains("ALPHA"),
        "coverage should keep the start, got {upload_text:?}"
    );
    assert!(
        upload_text.contains("OMEGA"),
        "coverage should keep the end, got {upload_text:?}"
    );
    let budget = retrieval_budget_for_turn(
        &fixture.ctx,
        fixture.ctx.context_budget(),
        "Summarize this doc: encyclopedia.md",
    );
    let upload_cost: usize = prepared
        .sources
        .iter()
        .filter(|s| s.shelf_id == upload.id)
        .map(gate::passage_cost)
        .sum();
    let upload_chars: usize = prepared
        .sources
        .iter()
        .filter(|s| s.shelf_id == upload.id)
        .map(|s| s.body.chars().count())
        .sum();
    let library_chars: usize = prepared
        .sources
        .iter()
        .filter(|s| s.shelf_id == fixture.shelf.id)
        .map(|s| s.body.chars().count())
        .sum();
    assert!(
        upload_chars > library_chars && upload_cost + 256 >= budget,
        "named attachment should spend the retrieval window, cost={upload_cost} budget={budget} upload={upload_chars} library={library_chars} sources={:?}",
        prepared
            .sources
            .iter()
            .map(|s| (s.shelf_id.as_str(), s.body.chars().count()))
            .collect::<Vec<_>>()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn open_shelf_file_pages_and_keeps_other_excerpts() {
    let mut body = String::from("# Encyclopedia\n\n");
    body.push_str("START unique opening.\n\n");
    body.push_str(&"The office zebra lives in the east wing.\n".repeat(800));
    body.push_str("\nEND unique closing.\n");
    let fixture = shelf_with_file("encyclopedia.md", &body).await;
    let files = super::tools::catalog(&fixture.ctx, &fixture.shelf.id);
    let mid = SourcePassage {
        anchor: None,
        sid: "S1".into(),
        document_id: files[0].id.clone(),
        shelf_id: fixture.shelf.id.clone(),
        title: "encyclopedia.md".into(),
        section: Some("East wing".into()),
        page_start: None,
        page_end: None,
        body: "The office zebra lives in the east wing.".into(),
        path: files[0].path.clone(),
        score: 1.0,
    };
    let sources = [mid];
    let tool = tool_ctx(
        &fixture.ctx,
        "t",
        &fixture.shelf.id,
        &files,
        &sources,
        1_200,
    );
    let first = super::tools::open_shelf_file(&tool, "encyclopedia.md");
    let tools::SourceChange::OpenWindow {
        opened: window,
        drop_sids,
        ..
    } = &first.change
    else {
        panic!("expected a window, got {:?}", first.change);
    };
    assert!(!drop_sids.iter().any(|id| id == "S1"));
    assert!(window.body.contains("START"));
    assert!(!window.body.contains("END unique"));
    assert_eq!(window.sid, "S2");
    let window = window.clone();

    let mut after = sources.to_vec();
    tools::apply_change(&mut after, first.change);
    assert!(after.iter().any(|s| s.sid == "S1"));
    assert!(after.iter().any(|s| s.sid == "S2"));

    let tool = tool_ctx(&fixture.ctx, "t", &fixture.shelf.id, &files, &after, 1_200);
    let second = super::tools::open_shelf_file(&tool, "encyclopedia.md");
    let tools::SourceChange::OpenWindow { opened: next, .. } = second.change else {
        panic!("expected the next window, got {:?}", second.change);
    };
    assert_eq!(next.sid, "S2");
    assert_ne!(next.body, window.body);
    assert!(
        next.body.contains("zebra") || next.body.contains("END"),
        "next window should move forward, got {}",
        next.body.chars().take(80).collect::<String>()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn open_shelf_file_pages_when_the_model_repeats_offset_zero() {
    let mut body = String::from("START unique opening.\n\n");
    for i in 0..80 {
        body.push_str(&format!(
            "middle paragraph {i} with enough words to split windows.\n\n"
        ));
    }
    body.push_str("END unique closing.\n");
    let fixture = shelf_with_file("encyclopedia.md", &body).await;
    let files = super::tools::catalog(&fixture.ctx, &fixture.shelf.id);
    let mut used = tools::ToolUse::default();
    let mut sources = Vec::new();
    let mut windows = Vec::new();
    let mut cursors = Vec::new();
    for _ in 0..3 {
        let tool = tool_ctx_used(
            &fixture.ctx,
            "t",
            &fixture.shelf.id,
            &files,
            &sources,
            1_200,
            &used,
        );
        let call = ToolCall::function(
            "c1",
            tools::OPEN_SHELF_FILE,
            serde_json::json!({ "file": "encyclopedia.md", "offset": 0 }).to_string(),
        );
        let outcome = tools::run_tool(&call, &tool).await;
        let tools::SourceChange::OpenWindow {
            opened, next_char, ..
        } = &outcome.change
        else {
            panic!("expected a window, got {:?}", outcome.change);
        };
        windows.push(opened.body.clone());
        cursors.push(*next_char);
        tools::note_use(&mut used, &call, &outcome.change);
        tools::apply_change(&mut sources, outcome.change);
    }
    assert_ne!(windows[0], windows[1]);
    assert_ne!(windows[1], windows[2]);
    assert!(windows[0].contains("START unique opening."));
    assert!(!windows[1].contains("START unique opening."));
    assert!(cursors[0] < cursors[1] && cursors[1] < cursors[2]);
    assert!(
        windows[2].contains("middle paragraph") || windows[2].contains("END unique"),
        "third window should keep moving, got {}",
        windows[2].chars().take(80).collect::<String>()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn look_around_continues_a_prefix_when_the_window_is_full() {
    let mut body = String::from("START unique opening.\n\n");
    body.push_str(&"The office zebra lives in the east wing.\n".repeat(800));
    body.push_str("\nEND unique closing.\n");
    let fixture = shelf_with_file("encyclopedia.md", &body).await;
    let files = super::tools::catalog(&fixture.ctx, &fixture.shelf.id);
    let extracted = std::fs::read_to_string(
        fixture
            .ctx
            .paths
            .extracted_path(&fixture.shelf.id, &files[0].id),
    )
    .unwrap();
    let prefix = gate::truncate_at_boundary(&extracted, 400);
    let excerpt = SourcePassage {
        anchor: None,
        sid: "S1".into(),
        document_id: files[0].id.clone(),
        shelf_id: fixture.shelf.id.clone(),
        title: "encyclopedia.md".into(),
        section: Some(crate::ingest::excerpt::OPEN_WINDOW_START.into()),
        page_start: None,
        page_end: None,
        body: prefix,
        path: files[0].path.clone(),
        score: 2.0,
    };
    let sources = [excerpt];
    let tool = tool_ctx(&fixture.ctx, "t", &fixture.shelf.id, &files, &sources, 500);
    let call = ToolCall::function(
        "c1",
        tools::LOOK_AROUND,
        serde_json::json!({ "id": "S1" }).to_string(),
    );
    let outcome = tools::run_tool(&call, &tool).await;
    let tools::SourceChange::ReplaceOne(updated) = outcome.change else {
        panic!("expected continued excerpt, got {:?}", outcome.change);
    };
    assert_eq!(updated.sid, "S1");
    assert_eq!(
        updated.section.as_deref(),
        Some(crate::ingest::excerpt::OPEN_WINDOW_NEXT)
    );
    assert!(!updated.body.starts_with("START unique opening."));
    assert!(updated.body.to_lowercase().contains("zebra"));
}

#[tokio::test(flavor = "multi_thread")]
async fn look_around_does_not_rewind_a_continued_window() {
    let mut body = String::from("START unique opening.\n\n");
    body.push_str(&"The office zebra lives in the east wing.\n".repeat(200));
    body.push_str("MIDDLE unique marker.\n");
    body.push_str(&"The office zebra lives in the east wing.\n".repeat(200));
    body.push_str("\nEND unique closing.\n");
    let fixture = shelf_with_file("encyclopedia.md", &body).await;
    let files = super::tools::catalog(&fixture.ctx, &fixture.shelf.id);
    let excerpt = SourcePassage {
        anchor: None,
        sid: "S1".into(),
        document_id: files[0].id.clone(),
        shelf_id: fixture.shelf.id.clone(),
        title: "encyclopedia.md".into(),
        section: Some(crate::ingest::excerpt::OPEN_WINDOW_NEXT.into()),
        page_start: None,
        page_end: None,
        body: "MIDDLE unique marker.\nThe office zebra lives in the east wing.".into(),
        path: files[0].path.clone(),
        score: 2.0,
    };
    let sources = [excerpt];
    let tool = tool_ctx(
        &fixture.ctx,
        "t",
        &fixture.shelf.id,
        &files,
        &sources,
        12_000,
    );
    let call = ToolCall::function(
        "c1",
        tools::LOOK_AROUND,
        serde_json::json!({ "id": "S1" }).to_string(),
    );
    let outcome = tools::run_tool(&call, &tool).await;
    let tools::SourceChange::ReplaceOne(updated) = outcome.change else {
        panic!("expected continued excerpt, got {:?}", outcome.change);
    };
    assert_eq!(updated.sid, "S1");
    assert!(
        !updated.body.contains("START unique opening."),
        "a continued window should not jump back to the start, got {}",
        updated.body.chars().take(80).collect::<String>()
    );
    assert!(
        updated.body.contains("MIDDLE unique marker.")
            || updated.body.to_lowercase().contains("zebra")
            || updated.body.contains("END unique"),
        "should read onward from the current window"
    );
}

#[test]
fn slim_continue_keeps_system_user_and_the_partial_answer() {
    let mut messages = vec![
        ChatMessage::text("system", "You are Larra."),
        ChatMessage::text("user", "older"),
        ChatMessage::text("assistant", "ack"),
        ChatMessage::text("user", "Write a long brief."),
        ChatMessage::text("assistant", "tool call"),
        ChatMessage::text("tool", "excerpts"),
    ];
    slim_messages_for_continue(&mut messages, "First half of the brief.");
    assert_eq!(messages.len(), 4);
    assert_eq!(messages[0].role, "system");
    assert_eq!(messages[1].as_text(), "Write a long brief.");
    assert_eq!(messages[2].as_text(), "First half of the brief.");
    assert!(messages[3].as_text().contains("Continue"));
}

#[tokio::test(flavor = "multi_thread")]
async fn small_context_reclaims_unused_history_without_clipping_the_question() {
    let fixture = shelf_with_file("facts.md", "# Facts\n\nThe delivery code is 7391.\n").await;
    let question = format!(
        "According to facts.md, what is the delivery code? {}",
        "Please use the file. ".repeat(100)
    );
    let turn = prepare_turn(
        &fixture.ctx,
        "new",
        &question,
        Some(&fixture.shelf.id),
        None,
        "m_current",
        &[],
        ThinkLevel::Off,
        &[],
    )
    .unwrap();
    assert!(
        !turn.sources.is_empty(),
        "unused history space must remain available to retrieval"
    );
    assert_eq!(
        turn.messages
            .iter()
            .rev()
            .find(|m| m.role == "user")
            .unwrap()
            .as_text(),
        question
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn attachment_wait_includes_the_pre_extraction_stage() {
    let fixture = shelf_with_file("facts.md", "# Facts\n\nThe delivery code is 7391.\n").await;
    let work = fixture.ctx.ingest_queue.begin_work(&fixture.shelf.id);
    let ctx = fixture.ctx.clone();
    let shelf = fixture.shelf.id.clone();
    let waiting = tokio::spawn(async move { wait_for_uploads(&ctx, &shelf).await });
    tokio::time::sleep(std::time::Duration::from_millis(75)).await;
    assert!(
        !waiting.is_finished(),
        "the queue-to-extractor handoff is still pending"
    );
    drop(work);
    tokio::time::timeout(std::time::Duration::from_secs(1), waiting)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}
