//! A model cannot send local context over the network without reviewing the exact payload.

use crate::core::{mutex_lock, read_lock, Ctx};
use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::{Mutex, OnceLock};
use tokio::sync::oneshot;

fn pending() -> &'static Mutex<HashMap<String, oneshot::Sender<bool>>> {
    static PENDING: OnceLock<Mutex<HashMap<String, oneshot::Sender<bool>>>> = OnceLock::new();
    PENDING.get_or_init(|| Mutex::new(HashMap::new()))
}

pub(crate) fn resolve(id: &str, allowed: bool) {
    if let Some(sender) = mutex_lock(pending()).remove(id) {
        let _ = sender.send(allowed);
    }
}

pub(super) async fn request(
    ctx: &Ctx,
    thread_id: &str,
    action: &str,
    value: &str,
    cancel: &AtomicBool,
) -> bool {
    if !read_lock(&ctx.settings).allow_online_research {
        return false;
    }
    let id = crate::ids::message_id();
    let (sender, receiver) = oneshot::channel();
    mutex_lock(pending()).insert(id.clone(), sender);
    ctx.events.emit(
        "larra://web-approval",
        serde_json::json!({
            "id": id, "threadId": thread_id, "action": action, "value": value,
        }),
    );
    let allowed = tokio::select! {
        biased;
        _ = crate::engine::wait_if_cancelled(cancel) => false,
        result = receiver => result.unwrap_or(false),
    };
    mutex_lock(pending()).remove(&id);
    ctx.events.emit(
        "larra://web-approval",
        serde_json::json!({ "id": id, "threadId": thread_id, "resolved": true }),
    );
    allowed && read_lock(&ctx.settings).allow_online_research
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    #[derive(Default)]
    struct Record(Mutex<Vec<serde_json::Value>>);
    impl crate::core::Events for Record {
        fn emit(&self, name: &str, value: serde_json::Value) {
            if name == "larra://web-approval" {
                mutex_lock(&self.0).push(value);
            }
        }
    }
    fn fixture() -> (tempfile::TempDir, Arc<Ctx>, Arc<Record>) {
        let dir = tempfile::tempdir().unwrap();
        let events = Arc::new(Record::default());
        let ctx = Ctx::new(
            crate::paths::Paths::new(dir.path().join("data")),
            events.clone(),
            Default::default(),
        )
        .unwrap();
        (dir, ctx, events)
    }
    #[tokio::test]
    async fn offline_setting_never_offers_an_outbound_request() {
        let (_dir, ctx, events) = fixture();
        assert!(
            !request(
                &ctx,
                "t",
                "search_web",
                "private phrase",
                &AtomicBool::new(false)
            )
            .await
        );
        assert!(mutex_lock(&events.0).is_empty());
    }
    #[tokio::test]
    async fn exact_payload_is_reviewed_and_revocation_is_respected() {
        let (_dir, ctx, events) = fixture();
        crate::core::write_lock(&ctx.settings).allow_online_research = true;
        let task_ctx = ctx.clone();
        let task = tokio::spawn(async move {
            request(
                &task_ctx,
                "t",
                "read_web_page",
                "https://example.com/private?secret=123",
                &AtomicBool::new(false),
            )
            .await
        });
        let id = tokio::time::timeout(std::time::Duration::from_secs(1), async {
            loop {
                let event = mutex_lock(&events.0).first().cloned();
                if let Some(event) = event {
                    assert_eq!(event["value"], "https://example.com/private?secret=123");
                    break event["id"].as_str().unwrap().to_string();
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(!task.is_finished());
        crate::core::write_lock(&ctx.settings).allow_online_research = false;
        resolve(&id, true);
        assert!(!task.await.unwrap());
        assert!(mutex_lock(&events.0).last().unwrap()["resolved"]
            .as_bool()
            .unwrap());
    }
}
