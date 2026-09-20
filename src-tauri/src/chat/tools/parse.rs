//! Best-effort parse for models that dump a call as text instead of `tool_calls`.

use serde_json::{json, Value};

use crate::engine::ToolCall;

use super::{is_known_tool, normalize_tool_name, ToolName};

pub(crate) fn requested_file_name(call: &ToolCall) -> Option<String> {
    arg_string(
        &call.function.arguments,
        &["file", "name", "filename", "path", "title"],
    )
}

pub(crate) fn arg_string(arguments: &str, keys: &[&str]) -> Option<String> {
    let value = parse_arguments_value(arguments)?;
    string_field(&value, keys)
}

fn parse_arguments_value(arguments: &str) -> Option<Value> {
    let trimmed = arguments.trim();
    if trimmed.is_empty() {
        return None;
    }
    serde_json::from_str::<Value>(trimmed).ok()
}

fn string_field(value: &Value, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(text) = value.get(*key).and_then(Value::as_str) {
            let cleaned = clean_requested(text);
            if !cleaned.is_empty() {
                return Some(cleaned);
            }
        }
    }
    None
}

pub(super) fn clean_requested(raw: &str) -> String {
    raw.trim()
        .trim_matches(|c| c == '"' || c == '\'' || c == '`')
        .trim()
        .replace('\\', "/")
        .to_string()
}

pub(crate) fn merge_tool_calls(native: Vec<ToolCall>, answer: &str) -> Vec<ToolCall> {
    if !native.is_empty() {
        return native
            .into_iter()
            .map(|mut call| {
                call.function.name = normalize_tool_name(&call.function.name);
                call
            })
            .collect();
    }
    parse_tool_calls_from_text(answer)
}

pub(crate) fn parse_tool_calls_from_text(text: &str) -> Vec<ToolCall> {
    let trimmed = strip_fences(text.trim());
    if trimmed.is_empty() || trimmed.chars().count() > 2_000 {
        return Vec::new();
    }
    if let Some(calls) = parse_json_tools(&trimmed) {
        return calls;
    }
    if let Some(calls) = parse_tagged_tools(&trimmed) {
        return calls;
    }
    if let Some(calls) = parse_gemma_tools(&trimmed) {
        return calls;
    }
    Vec::new()
}

fn strip_fences(text: &str) -> String {
    let mut t = text.trim();
    if let Some(rest) = t.strip_prefix("```") {
        t = rest.trim_start();
        if let Some(nl) = t.find('\n') {
            t = t[nl + 1..].trim_start();
        }
        if let Some(end) = t.rfind("```") {
            t = t[..end].trim();
        }
    }
    t.to_string()
}

fn parse_json_tools(text: &str) -> Option<Vec<ToolCall>> {
    let value: Value = serde_json::from_str(text).ok()?;
    let calls = calls_from_value(&value);
    if calls.is_empty() {
        None
    } else {
        Some(calls)
    }
}

fn calls_from_value(value: &Value) -> Vec<ToolCall> {
    match value {
        Value::Array(items) => items.iter().flat_map(calls_from_value).collect(),
        Value::Object(_) => {
            if let Some(arr) = value.get("tool_calls").and_then(Value::as_array) {
                return arr.iter().flat_map(call_from_object).collect();
            }
            call_from_object(value).into_iter().collect()
        }
        _ => Vec::new(),
    }
}

fn call_from_object(value: &Value) -> Option<ToolCall> {
    let name = value
        .get("name")
        .and_then(Value::as_str)
        .or_else(|| value.get("tool").and_then(Value::as_str))
        .or_else(|| value.get("tool_name").and_then(Value::as_str))
        .or_else(|| {
            value
                .get("function")
                .and_then(|f| f.get("name"))
                .and_then(Value::as_str)
        })
        .map(normalize_tool_name)?;
    if !is_known_tool(&name) {
        return None;
    }
    let args = if let Some(raw) = value.get("arguments") {
        if raw.is_string() {
            raw.as_str().unwrap_or("{}").to_string()
        } else {
            raw.to_string()
        }
    } else if let Some(raw) = value.get("parameters") {
        raw.to_string()
    } else if let Some(func) = value.get("function") {
        match func.get("arguments") {
            Some(Value::String(s)) => s.clone(),
            Some(other) => other.to_string(),
            None => leftover_args(value, &name),
        }
    } else {
        leftover_args(value, &name)
    };
    Some(ToolCall::function("call_1", name, args))
}

fn leftover_args(value: &Value, name: &str) -> String {
    let Some(tool) = ToolName::parse(name) else {
        return "{}".into();
    };
    match tool {
        ToolName::OpenFile => {
            if let Some(file) = string_field(value, &["file", "filename", "path", "title"]) {
                return json!({ "file": file }).to_string();
            }
            if let Some(file) = string_field(value, &["name"]) {
                if ToolName::parse(&file).is_none() {
                    return json!({ "file": file }).to_string();
                }
            }
        }
        ToolName::SearchShelf | ToolName::SearchChats | ToolName::SearchWeb => {
            if let Some(query) = string_field(value, &["query", "q", "search", "keywords", "text"])
            {
                return json!({ "query": query }).to_string();
            }
        }
        ToolName::LookAround => {
            if let Some(id) = string_field(value, &["id", "sid", "source", "source_id"]) {
                return json!({ "id": id }).to_string();
            }
        }
        ToolName::ReadWebPage => {
            if let Some(url) = string_field(value, &["url", "href", "link", "page", "uri"]) {
                return json!({ "url": url }).to_string();
            }
        }
    }
    "{}".into()
}

fn parse_tagged_tools(text: &str) -> Option<Vec<ToolCall>> {
    for (open, close, kind) in [
        ("<tool_call>", "</tool_call>", None),
        ("<tool_call>", "<tool_call|>", None),
        (
            "<open_shelf_file>",
            "</open_shelf_file>",
            Some(ToolName::OpenFile),
        ),
        (
            "<search_shelf>",
            "</search_shelf>",
            Some(ToolName::SearchShelf),
        ),
        (
            "<look_around>",
            "</look_around>",
            Some(ToolName::LookAround),
        ),
        (
            "<search_chats>",
            "</search_chats>",
            Some(ToolName::SearchChats),
        ),
        ("<search_web>", "</search_web>", Some(ToolName::SearchWeb)),
        (
            "<read_web_page>",
            "</read_web_page>",
            Some(ToolName::ReadWebPage),
        ),
    ] {
        if let Some(inner) = between(text, open, close) {
            if let Some(calls) = parse_json_tools(inner.trim()) {
                return Some(calls);
            }
            if let Some(kind) = kind {
                let body = clean_requested(inner);
                if body.is_empty() {
                    continue;
                }
                let args = match kind {
                    ToolName::OpenFile => json!({ "file": body }).to_string(),
                    ToolName::SearchShelf | ToolName::SearchChats | ToolName::SearchWeb => {
                        json!({ "query": body }).to_string()
                    }
                    ToolName::LookAround => json!({ "id": body }).to_string(),
                    ToolName::ReadWebPage => json!({ "url": body }).to_string(),
                };
                return Some(vec![ToolCall::function("call_1", kind.as_str(), args)]);
            }
        }
    }
    None
}

fn parse_gemma_tools(text: &str) -> Option<Vec<ToolCall>> {
    let start = text.find("call:")?;
    let rest = &text[start + 5..];
    let name_end = rest.find('{')?;
    let name = normalize_tool_name(rest[..name_end].trim());
    let tool = ToolName::parse(&name)?;
    let body = &rest[name_end..];
    let quoted = between(body, "<|\"|>", "<|\"|>").map(clean_requested);
    let args = match tool {
        ToolName::OpenFile => {
            let file = quoted.filter(|s| !s.is_empty()).or_else(|| {
                between(body, "file:", "}").map(|s| clean_requested(s.trim_end_matches(',')))
            })?;
            if file.is_empty() {
                return None;
            }
            json!({ "file": file }).to_string()
        }
        ToolName::SearchShelf | ToolName::SearchChats | ToolName::SearchWeb => {
            let query = quoted.filter(|s| !s.is_empty()).or_else(|| {
                between(body, "query:", "}").map(|s| clean_requested(s.trim_end_matches(',')))
            })?;
            if query.is_empty() {
                return None;
            }
            json!({ "query": query }).to_string()
        }
        ToolName::LookAround => {
            let id = quoted.filter(|s| !s.is_empty()).or_else(|| {
                between(body, "id:", "}").map(|s| clean_requested(s.trim_end_matches(',')))
            })?;
            if id.is_empty() {
                return None;
            }
            json!({ "id": id }).to_string()
        }
        ToolName::ReadWebPage => {
            let url = quoted.filter(|s| !s.is_empty()).or_else(|| {
                between(body, "url:", "}").map(|s| clean_requested(s.trim_end_matches(',')))
            })?;
            if url.is_empty() {
                return None;
            }
            json!({ "url": url }).to_string()
        }
    };
    Some(vec![ToolCall::function("call_1", name, args)])
}

fn between<'a>(text: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let from = text.find(start)? + start.len();
    let to = text[from..].find(end)? + from;
    Some(&text[from..to])
}

#[cfg(test)]
mod tests {
    use super::super::{
        LOOK_AROUND, OPEN_SHELF_FILE, READ_WEB_PAGE, SEARCH_CHATS, SEARCH_SHELF, SEARCH_WEB,
    };
    use super::*;

    #[test]
    fn parse_native_json_and_aliases() {
        let calls = parse_tool_calls_from_text(
            r#"{"name":"read_file","arguments":{"file":"Staff handbook.md"}}"#,
        );
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function.name, OPEN_SHELF_FILE);
        assert!(calls[0].function.arguments.contains("Staff handbook.md"));

        let search = parse_tool_calls_from_text(
            r#"{"name":"search_this_shelf","arguments":{"query":"zebra east wing"}}"#,
        );
        assert_eq!(search[0].function.name, SEARCH_SHELF);
        assert!(search[0].function.arguments.contains("zebra east wing"));
    }

    #[test]
    fn parse_tagged_and_gemma_shapes() {
        let tagged = parse_tool_calls_from_text(
            r#"<tool_call>{"name":"open_shelf_file","arguments":{"file":"notes.md"}}</tool_call>"#,
        );
        assert_eq!(tagged[0].function.name, OPEN_SHELF_FILE);
        let xml = parse_tool_calls_from_text("<open_shelf_file>notes.md</open_shelf_file>");
        assert!(xml[0].function.arguments.contains("notes.md"));
        let gemma = parse_tool_calls_from_text(
            r#"<|tool_call>call:open_shelf_file{file:<|"|>notes.md<|"|>}<tool_call|>"#,
        );
        assert_eq!(gemma.len(), 1);
        assert!(gemma[0].function.arguments.contains("notes.md"));

        let around = parse_tool_calls_from_text("<look_around>S2</look_around>");
        assert_eq!(around[0].function.name, LOOK_AROUND);
        assert!(around[0].function.arguments.contains("S2"));

        let chats = parse_tool_calls_from_text(
            r#"<|tool_call>call:search_chats{query:<|"|>office move<|"|>}<tool_call|>"#,
        );
        assert_eq!(chats[0].function.name, SEARCH_CHATS);
        assert!(chats[0].function.arguments.contains("office move"));

        let web = parse_tool_calls_from_text("<search_web>paris weather</search_web>");
        assert_eq!(web[0].function.name, SEARCH_WEB);
        assert!(web[0].function.arguments.contains("paris weather"));
        let page = parse_tool_calls_from_text("<read_web_page>https://example.com</read_web_page>");
        assert_eq!(page[0].function.name, READ_WEB_PAGE);
        assert!(page[0].function.arguments.contains("https://example.com"));
    }

    #[test]
    fn prose_is_not_a_tool_call() {
        assert!(parse_tool_calls_from_text("The kitchen is restocked on Tuesday.").is_empty());
        assert!(parse_tool_calls_from_text("").is_empty());
    }
}
