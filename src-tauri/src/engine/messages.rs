//! Shaping a message list into something every chat template can render.
//!
//! Templates disagree about what is legal. Qwen accepts a system message only
//! at index 0, Gemma 3 wants user and assistant turns to alternate and has no
//! tool role at all, and Ministral exempts tool turns from that same rule.
//! `single_leading_system` runs on every request; `flatten_tool_turns` is the
//! fallback for a template that rejected the list outright.

use std::borrow::Cow;

use super::ChatMessage;

/// Last stop before the wire: keep every system message at index 0. Qwen and
/// other templates raise `System message must be at the beginning` for a later
/// one, which comes back as a 500 and reads to the user as a failed answer.
/// Callers are expected to build a legal list, so this logs when it fires.
pub(super) fn single_leading_system(messages: &[ChatMessage]) -> Cow<'_, [ChatMessage]> {
    if !messages.iter().skip(1).any(|m| m.role == "system") {
        return Cow::Borrowed(messages);
    }
    log::warn!("system message after the first turn; folding it into the system prompt");
    let mut kept: Vec<ChatMessage> = Vec::with_capacity(messages.len());
    let mut folded = String::new();
    for (index, message) in messages.iter().enumerate() {
        if index > 0 && message.role == "system" {
            append_paragraph(&mut folded, message.as_text().trim());
            continue;
        }
        kept.push(message.clone());
    }
    if folded.is_empty() {
        return Cow::Owned(kept);
    }
    match kept.first_mut() {
        Some(first) if first.role == "system" => {
            let mut text = first.as_text().trim().to_string();
            append_paragraph(&mut text, &folded);
            first.content = Some(text);
        }
        _ => kept.insert(0, ChatMessage::text("system", folded)),
    }
    Cow::Owned(kept)
}

/// The plainest shape a chat template can be asked to render: one leading
/// system message, then user and assistant turns that alternate. Retrieved
/// passages move into the question they belong to, because templates such as
/// Gemma 3's have no tool role and reject two turns in a row from one role.
pub(super) fn flatten_tool_turns(messages: &[ChatMessage]) -> Vec<ChatMessage> {
    let mut out: Vec<ChatMessage> = Vec::with_capacity(messages.len());
    let mut carried = String::new();
    for (index, message) in messages.iter().enumerate() {
        let text = message.as_text().trim();
        if index == 0 && message.role == "system" {
            out.push(message.clone());
            continue;
        }
        // A tool result, a stray system turn, and an assistant turn that was
        // nothing but a tool call all have to ride along with a real turn.
        if message.role == "tool" || message.role == "system" || text.is_empty() {
            append_paragraph(&mut carried, text);
            continue;
        }
        let mut body = text.to_string();
        if message.role == "user" {
            append_paragraph(&mut body, &std::mem::take(&mut carried));
        }
        merge_or_push(&mut out, &message.role, &body);
    }
    if !carried.is_empty() {
        merge_or_push(&mut out, "user", &carried);
    }
    out
}

fn merge_or_push(out: &mut Vec<ChatMessage>, role: &str, body: &str) {
    match out.last_mut() {
        Some(last) if last.role == role => {
            let mut merged = last.as_text().to_string();
            append_paragraph(&mut merged, body);
            last.content = Some(merged);
        }
        _ => out.push(ChatMessage::text(role, body)),
    }
}

fn append_paragraph(target: &mut String, text: &str) {
    if text.is_empty() {
        return;
    }
    if !target.is_empty() {
        target.push_str("\n\n");
    }
    target.push_str(text);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::ToolCall;

    fn roles(messages: &[ChatMessage]) -> Vec<&str> {
        messages.iter().map(|m| m.role.as_str()).collect()
    }

    fn tool_call() -> ChatMessage {
        ChatMessage {
            role: "assistant".into(),
            content: None,
            tool_calls: Some(vec![ToolCall::function("c1", "search_shelf", "{}")]),
            tool_call_id: None,
            name: None,
        }
    }

    #[test]
    fn a_legal_message_list_is_not_copied() {
        let messages = vec![
            ChatMessage::text("system", "house rules"),
            ChatMessage::text("user", "when do we open?"),
        ];
        assert!(matches!(single_leading_system(&messages), Cow::Borrowed(_)));
    }

    #[test]
    fn a_late_system_message_is_folded_into_the_first() {
        let messages = vec![
            ChatMessage::text("system", "house rules"),
            ChatMessage::text("user", "when do we open?"),
            ChatMessage::text("assistant", "nine to five."),
            ChatMessage::text("system", "opening hours excerpt"),
            ChatMessage::text("user", "and saturday?"),
        ];
        let fixed = single_leading_system(&messages);
        assert_eq!(roles(&fixed), ["system", "user", "assistant", "user"]);
        assert_eq!(fixed[0].as_text(), "house rules\n\nopening hours excerpt");
    }

    #[test]
    fn a_late_system_message_gets_a_front_seat_when_there_is_none() {
        let messages = vec![
            ChatMessage::text("user", "when do we open?"),
            ChatMessage::text("system", "opening hours excerpt"),
        ];
        let fixed = single_leading_system(&messages);
        assert_eq!(roles(&fixed), ["system", "user"]);
        assert_eq!(fixed[0].as_text(), "opening hours excerpt");
    }

    #[test]
    fn flattening_inlines_the_retrieved_passages_into_the_question() {
        let messages = vec![
            ChatMessage::text("system", "house rules"),
            ChatMessage::text("user", "and saturday?"),
            tool_call(),
            ChatMessage::text("tool", "opening hours excerpt"),
        ];
        let flat = flatten_tool_turns(&messages);
        assert_eq!(roles(&flat), ["system", "user"]);
        assert_eq!(flat[1].as_text(), "and saturday?\n\nopening hours excerpt");
        assert!(flat[1].tool_calls.is_none());
        assert!(flat[1].tool_call_id.is_none());
    }

    #[test]
    fn flattening_keeps_history_alternating() {
        let messages = vec![
            ChatMessage::text("system", "house rules"),
            ChatMessage::text("user", "when do we open?"),
            ChatMessage::text("assistant", "nine to five."),
            ChatMessage::text("user", "and saturday?"),
            tool_call(),
            ChatMessage::text("tool", "first excerpt"),
            tool_call(),
            ChatMessage::text("tool", "second excerpt"),
        ];
        let flat = flatten_tool_turns(&messages);
        assert_eq!(roles(&flat), ["system", "user", "assistant", "user"]);
        assert_eq!(
            flat[3].as_text(),
            "and saturday?\n\nfirst excerpt\n\nsecond excerpt"
        );
    }

    #[test]
    fn flattening_leaves_a_plain_conversation_alone() {
        let messages = vec![
            ChatMessage::text("system", "house rules"),
            ChatMessage::text("user", "when do we open?"),
        ];
        let flat = flatten_tool_turns(&messages);
        assert_eq!(roles(&flat), ["system", "user"]);
        assert_eq!(flat[1].as_text(), "when do we open?");
    }

    #[test]
    fn flattening_survives_a_tool_result_with_no_question_after_it() {
        let messages = vec![
            ChatMessage::text("system", "house rules"),
            ChatMessage::text("assistant", "nine to five."),
            tool_call(),
            ChatMessage::text("tool", "opening hours excerpt"),
        ];
        let flat = flatten_tool_turns(&messages);
        assert_eq!(roles(&flat), ["system", "assistant", "user"]);
        assert_eq!(flat[2].as_text(), "opening hours excerpt");
    }
}
