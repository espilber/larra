use super::*;
use crate::core::NoopEvents;
use crate::engine::ToolCall;
use crate::ingest::extract::ExtractorSettings;
use crate::paths::Paths;
use std::sync::atomic::Ordering;

struct Bundle {
    chat: Arc<ChatService>,
    _dir: tempfile::TempDir,
}

fn test_chat() -> Bundle {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::new(dir.path().join("appdata"));
    let ctx = Ctx::new(paths, Arc::new(NoopEvents), ExtractorSettings::default()).unwrap();
    let engine = Engine::new(ctx.clone());
    Bundle {
        chat: ChatService::new(ctx, engine),
        _dir: dir,
    }
}

#[test]
fn cancel_unknown_id_is_silent() {
    let bundle = test_chat();
    bundle.chat.cancel("no-such-message");
    assert_eq!(bundle.chat.cancel_count(), 0);
}

#[test]
fn cancel_sets_registered_flag() {
    let bundle = test_chat();
    let flag = bundle.chat.register_cancel_for_test("m1");
    assert!(!flag.load(Ordering::Relaxed));
    bundle.chat.cancel("m1");
    assert!(flag.load(Ordering::Relaxed));
}

struct CollectChat(std::sync::Mutex<Vec<serde_json::Value>>);

impl crate::core::Events for CollectChat {
    fn emit(&self, event: &str, payload: serde_json::Value) {
        if event == "larra://chat" {
            self.0.lock().unwrap().push(payload);
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn cancel_while_waiting_for_the_generation_slot_skips_the_engine() {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::new(dir.path().join("appdata"));
    let events = Arc::new(CollectChat(std::sync::Mutex::new(Vec::new())));
    let ctx = Ctx::new(paths, events.clone(), ExtractorSettings::default()).unwrap();
    let engine = Engine::new(ctx.clone());
    let chat = ChatService::new(ctx.clone(), engine);
    let thread = Conversations::create(&ctx.paths, None).unwrap();

    let guard = chat.generation.lock().await;
    let chat_send = chat.clone();
    let thread_id = thread.id.clone();
    let handle = tokio::spawn(async move {
        chat_send
            .send_message(&thread_id, "Hello there, please answer.", None)
            .await
    });

    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
    let mut message_id = None;
    let mut saw_waiting = false;
    while tokio::time::Instant::now() < deadline {
        {
            let captured = events.0.lock().unwrap();
            if message_id.is_none() {
                message_id = captured.iter().find_map(|payload| {
                    if payload.get("kind").and_then(|kind| kind.as_str()) == Some("queued") {
                        payload
                            .get("messageId")
                            .and_then(|id| id.as_str())
                            .map(str::to_string)
                    } else {
                        None
                    }
                });
            }
            saw_waiting = captured.iter().any(|payload| {
                payload.get("kind").and_then(|kind| kind.as_str()) == Some("status")
                    && payload.get("stage").and_then(|stage| stage.as_str()) == Some("waiting")
            });
        }
        if message_id.is_some() && saw_waiting {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let message_id = message_id.expect("queued event while waiting for the slot");
    assert!(
        saw_waiting,
        "waiting status while another answer holds the slot"
    );
    chat.cancel(&message_id);
    // The other turn still holds the slot. Stop must return without waiting
    // for that lock; a couple of seconds is still far shorter than waiting.
    let message = tokio::time::timeout(std::time::Duration::from_secs(2), handle)
        .await
        .expect("Stop must not wait for the occupied generation slot")
        .unwrap()
        .unwrap();
    drop(guard);
    assert_eq!(chat.cancel_count(), 0);
    assert_eq!(message.status, "stopped");
    let captured = events.0.lock().unwrap();
    assert!(
        captured.iter().all(|payload| {
            payload.get("kind").and_then(|kind| kind.as_str()) != Some("started")
        }),
        "engine should not have started"
    );
}

#[test]
fn notify_send_failed_emits_an_error_for_the_thread() {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::new(dir.path().join("appdata"));
    let events = Arc::new(CollectChat(std::sync::Mutex::new(Vec::new())));
    let ctx = Ctx::new(paths, events.clone(), ExtractorSettings::default()).unwrap();
    let engine = Engine::new(ctx.clone());
    let chat = ChatService::new(ctx, engine);
    chat.notify_send_failed("t1");
    let captured = events.0.lock().unwrap();
    assert_eq!(captured.len(), 1);
    assert_eq!(
        captured[0].get("kind").and_then(|kind| kind.as_str()),
        Some("error")
    );
    assert_eq!(
        captured[0].get("threadId").and_then(|id| id.as_str()),
        Some("t1")
    );
    assert_eq!(
        captured[0].get("messageId").and_then(|id| id.as_str()),
        Some("")
    );
}

#[test]
fn a_blank_finished_turn_is_a_failure() {
    assert!(should_fail_empty_answer("", false));
    assert!(should_fail_empty_answer("   ", false));
    assert!(!should_fail_empty_answer("Hello", false));
    assert!(!should_fail_empty_answer("", true));
}

#[test]
fn a_blank_follow_up_drops_tool_notes() {
    assert!(!should_drop_tool_transcript(0));
    assert!(should_drop_tool_transcript(1));
}

#[test]
fn follow_up_search_keeps_the_last_user_questions() {
    let user = |id: &str, text: &str| StoredMessage {
        id: id.into(),
        role: "user".into(),
        text: text.into(),
        thinking: None,
        activity: Vec::new(),
        ts: String::new(),
        shelf_id: None,
        sources: Vec::new(),
        status: "done".into(),
    };
    let history = vec![
        user(
            "m1",
            "List the key terms from the Mac Employee Starter Guide",
        ),
        user("m2", "shorten to only 5 terms"),
        user("m3", "ok do it"),
    ];
    assert_eq!(
        prior_search_queries(&history, "m3", "ok do it"),
        vec![
            "List the key terms from the Mac Employee Starter Guide".to_string(),
            "shorten to only 5 terms".to_string(),
        ]
    );
}

#[test]
fn history_keeps_the_citation_titles_the_user_saw() {
    let message = StoredMessage {
        id: "a1".into(),
        role: "assistant".into(),
        text: "Notice is 90 days. [S1]".into(),
        thinking: None,
        activity: Vec::new(),
        ts: String::new(),
        shelf_id: None,
        sources: vec![SourcePassage {
            anchor: None,
            sid: "S1".into(),
            document_id: "d1".into(),
            shelf_id: "s1".into(),
            title: "Lease.pdf".into(),
            section: None,
            page_start: Some(4),
            page_end: None,
            body: String::new(),
            path: String::new(),
            score: 1.0,
        }],
        status: "done".into(),
    };
    let text = history_message_text(&message);
    assert!(text.contains("Notice is 90 days. [S1]"));
    assert!(text.contains("S1 Lease.pdf (p. 4)"));
}

#[test]
fn rewind_keeps_the_prompt_and_drops_tool_turns() {
    let mut messages = vec![
        ChatMessage::text("system", "You are Larra."),
        ChatMessage::text("user", "Hello"),
    ];
    let prompt_len = messages.len();
    let call = ToolCall::function(
        "c1",
        tools::SEARCH_CHATS,
        r#"{"query":"office"}"#.to_string(),
    );
    messages.push(tools::assistant_tool_message(std::slice::from_ref(&call)));
    messages.push(tools::tool_result_message(&call, "Earlier notes.".into()));
    rewind_to_prompt(&mut messages, prompt_len);
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[1].as_text(), "Hello");
}

#[test]
fn citation_repair_cannot_change_words_numbers_or_language() {
    let ids = vec!["S1".to_string()];
    assert!(citation_only_revision(
        "El codi és 7391.",
        "El codi és 7391 [S1].",
        &ids
    ));
    assert!(!citation_only_revision(
        "El codi és 7391.",
        "El codi és 7000 [S1].",
        &ids
    ));
    assert!(!citation_only_revision(
        "El codi és 7391.",
        "The code is 7391 [S1].",
        &ids
    ));
    assert!(!citation_only_revision(
        "El codi és 7391.",
        "El codi és 7391 [S2].",
        &ids
    ));
}
