//! OpenAI-style tools for Chat: search this Shelf, look around a citation,
//! open a named file, search earlier conversations, and (when allowed) the web.

mod open;
mod parse;
mod search;
mod web;

use std::collections::{HashMap, HashSet};
use std::sync::atomic::AtomicBool;

use serde_json::{json, Value};

use crate::core::Ctx;
use crate::engine::ToolCall;
use crate::shelf::ThinkLevel;
use crate::types::SourcePassage;

pub(crate) use open::{
    catalog, disambiguate_labels, labels_for_schema, open_shelf_file, ShelfFile,
};
pub(crate) use parse::{
    arg_string, merge_tool_calls, parse_tool_calls_from_text, requested_file_name,
};

pub(crate) const SEARCH_SHELF: &str = "search_shelf";
pub(crate) const LOOK_AROUND: &str = "look_around";
pub(crate) const OPEN_SHELF_FILE: &str = "open_shelf_file";
pub(crate) const SEARCH_CHATS: &str = "search_chats";
pub(crate) const SEARCH_WEB: &str = "search_web";
pub(crate) const READ_WEB_PAGE: &str = "read_web_page";
pub(crate) const MAX_TOOL_ROUNDS: usize = 3;
pub(crate) const MAX_OPENED_FILES: usize = 2;
const MAX_SHELF_SEARCHES: usize = 2;
const MAX_LOOK_AROUNDS: usize = 2;
const MAX_CHAT_SEARCHES: usize = 2;
const MAX_WEB_SEARCHES: usize = 2;
const MAX_PAGE_READS: usize = 2;
const TOOL_ENUM_MAX: usize = 48;
pub(crate) const MIN_TOOL_CHARS: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ToolName {
    SearchShelf,
    LookAround,
    OpenFile,
    SearchChats,
    SearchWeb,
    ReadWebPage,
}

impl ToolName {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::SearchShelf => SEARCH_SHELF,
            Self::LookAround => LOOK_AROUND,
            Self::OpenFile => OPEN_SHELF_FILE,
            Self::SearchChats => SEARCH_CHATS,
            Self::SearchWeb => SEARCH_WEB,
            Self::ReadWebPage => READ_WEB_PAGE,
        }
    }

    pub(crate) fn parse(raw: &str) -> Option<Self> {
        match raw.trim() {
            SEARCH_SHELF | "search_files" | "search_this_shelf" | "shelf_search" => {
                Some(Self::SearchShelf)
            }
            LOOK_AROUND | "expand_source" | "read_around" | "more_context" => {
                Some(Self::LookAround)
            }
            OPEN_SHELF_FILE | "open_file" | "read_file" | "read_shelf_file" | "open" | "read" => {
                Some(Self::OpenFile)
            }
            SEARCH_CHATS
            | "search_memory"
            | "search_conversations"
            | "earlier_chats"
            | "search_history" => Some(Self::SearchChats),
            SEARCH_WEB | "research_online" | "online_research" | "search_online" | "web_search" => {
                Some(Self::SearchWeb)
            }
            READ_WEB_PAGE | "get_web_page" | "fetch_url" | "open_url" | "read_url"
            | "browse_page" => Some(Self::ReadWebPage),
            _ => None,
        }
    }

    fn stage(self) -> &'static str {
        match self {
            Self::SearchShelf => "looking",
            Self::LookAround => "around",
            Self::OpenFile => "opening",
            Self::SearchChats => "chats",
            Self::SearchWeb => "web",
            Self::ReadWebPage => "page",
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ToolSet {
    pub search_shelf: bool,
    pub look_around: bool,
    pub open_file: bool,
    pub search_chats: bool,
    pub search_web: bool,
    pub read_web_page: bool,
}

impl ToolSet {
    pub(crate) fn new(
        shelf_ok: bool,
        has_sources: bool,
        online: bool,
        chats_ok: bool,
        used: &ToolUse,
    ) -> Self {
        Self {
            search_shelf: shelf_ok && used.shelf_searches < MAX_SHELF_SEARCHES,
            look_around: shelf_ok && has_sources && used.looks < MAX_LOOK_AROUNDS,
            open_file: shelf_ok && used.opened.len() < MAX_OPENED_FILES,
            search_chats: chats_ok && used.chats < MAX_CHAT_SEARCHES,
            search_web: online && used.web_searches < MAX_WEB_SEARCHES,
            read_web_page: online && used.page_reads < MAX_PAGE_READS,
        }
    }

    /// Shelf or web tools that still need another look. `search_chats` alone
    /// should not start another tool round.
    pub(crate) fn follow_up_useful(self) -> bool {
        self.search_shelf
            || self.look_around
            || self.open_file
            || self.search_web
            || self.read_web_page
    }

    pub(crate) fn any(self) -> bool {
        self.search_shelf
            || self.look_around
            || self.open_file
            || self.search_chats
            || self.search_web
            || self.read_web_page
    }

    fn allows(self, name: ToolName) -> bool {
        match name {
            ToolName::SearchShelf => self.search_shelf,
            ToolName::LookAround => self.look_around,
            ToolName::OpenFile => self.open_file,
            ToolName::SearchChats => self.search_chats,
            ToolName::SearchWeb => self.search_web,
            ToolName::ReadWebPage => self.read_web_page,
        }
    }

    fn listed_names(self) -> String {
        [
            (self.search_shelf, SEARCH_SHELF),
            (self.look_around, LOOK_AROUND),
            (self.open_file, OPEN_SHELF_FILE),
            (self.search_chats, SEARCH_CHATS),
            (self.search_web, SEARCH_WEB),
            (self.read_web_page, READ_WEB_PAGE),
        ]
        .into_iter()
        .filter(|(on, _)| *on)
        .map(|(_, name)| name)
        .collect::<Vec<_>>()
        .join(", ")
    }

    pub(crate) fn schema(self, labels: &[String]) -> Value {
        let mut tools = Vec::new();
        if self.search_shelf {
            tools.push(json!({
                "type": "function",
                "function": {
                    "name": SEARCH_SHELF,
                    "description": "Search this Shelf for more excerpts. Use when the current excerpts do not cover the question. Prefer names, dates, and distinctive terms over repeating the user's question.",
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "query": {
                                "type": "string",
                                "description": "Search query for files on this Shelf."
                            }
                        },
                        "required": ["query"]
                    }
                }
            }));
        }
        if self.look_around {
            tools.push(json!({
                "type": "function",
                "function": {
                    "name": LOOK_AROUND,
                    "description": "Load more text around an existing source excerpt. Use when a citation is cut off or you need the next part of that file. id is a source id such as S1.",
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "id": {
                                "type": "string",
                                "description": "Source id from the excerpts, like S1."
                            }
                        },
                        "required": ["id"]
                    }
                }
            }));
        }
        if self.open_file {
            let mut file = json!({
                "type": "string",
                "description": "Exact file name from the shelf list. Use the path when two files share a name. This is the only argument.",
            });
            if !labels.is_empty() && labels.len() <= TOOL_ENUM_MAX {
                file["enum"] = json!(labels);
            }
            tools.push(json!({
                "type": "function",
                "function": {
                    "name": OPEN_SHELF_FILE,
                    "description": "Open one named file from the shelf list. Use when excerpts are not enough and you know which file to read. Pass only the file name. A long file returns the next unread window each time you call.",
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "file": file
                        },
                        "required": ["file"]
                    }
                }
            }));
        }
        if self.search_chats {
            tools.push(json!({
                "type": "function",
                "function": {
                    "name": SEARCH_CHATS,
                    "description": "Search earlier turns of this conversation, and other conversations on this Shelf. Use when the question refers to something said before the recent turns. Skip it for a new request. Use what you find in the answer; do not cite those notes as [S1].",
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "query": {
                                "type": "string",
                                "description": "Search query for earlier conversations."
                            }
                        },
                        "required": ["query"]
                    }
                }
            }));
        }
        if self.search_web {
            tools.push(json!({
                "type": "function",
                "function": {
                    "name": SEARCH_WEB,
                    "description": "Search the public web. Use for facts, news, or pages that are not on the Shelf. Keep the query public: no Shelf text or personal details. Do not cite results as [S1]; name the site or page title in the answer.",
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "query": {
                                "type": "string",
                                "description": "Public web query. No Shelf text or personal details.",
                            }
                        },
                        "required": ["query"]
                    }
                }
            }));
        }
        if self.read_web_page {
            tools.push(json!({
                "type": "function",
                "function": {
                    "name": READ_WEB_PAGE,
                    "description": "Read one public http(s) page as text. Use after search_web when you need a specific page. Do not put Shelf text or personal details in the URL. Do not cite it as [S1].",
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "url": {
                                "type": "string",
                                "description": "Full http or https URL of a public page.",
                            }
                        },
                        "required": ["url"]
                    }
                }
            }));
        }
        json!(tools)
    }
}

#[derive(Default)]
pub(crate) struct ToolUse {
    opened: HashSet<String>,
    /// Next character to send for each file already opened this turn.
    pub(crate) open_next: HashMap<String, usize>,
    shelf_searches: usize,
    looks: usize,
    chats: usize,
    web_searches: usize,
    page_reads: usize,
}

impl ToolUse {
    fn record(&mut self, name: ToolName, change: &SourceChange) {
        match name {
            ToolName::SearchShelf => self.shelf_searches += 1,
            ToolName::LookAround => self.looks += 1,
            ToolName::OpenFile => match change {
                SourceChange::OpenWindow {
                    opened: source,
                    next_char,
                    ..
                } => {
                    self.opened.insert(source.document_id.clone());
                    self.open_next
                        .insert(source.document_id.clone(), *next_char);
                }
                SourceChange::None | SourceChange::ReplaceOne(_) | SourceChange::Append(_) => {}
            },
            ToolName::SearchChats => self.chats += 1,
            ToolName::SearchWeb => self.web_searches += 1,
            ToolName::ReadWebPage => self.page_reads += 1,
        }
    }
}

pub(crate) struct ToolCtx<'a> {
    pub ctx: &'a Ctx,
    pub thread_id: &'a str,
    pub shelf_id: Option<&'a str>,
    pub upload_shelf_id: Option<&'a str>,
    pub files: &'a [ShelfFile],
    pub sources: &'a [SourcePassage],
    /// Citation ids already bound in this conversation. New excerpts reuse
    /// them for the same file and never give them to a different file.
    pub cited: &'a [SourcePassage],
    pub budget: usize,
    pub think: ThinkLevel,
    pub allowed: ToolSet,
    pub cancel: &'a AtomicBool,
    /// Next unread character per document, copied from `ToolUse` for this call.
    pub open_next: HashMap<String, usize>,
    /// Message ids already in the standing prompt. `search_chats` skips them.
    pub exclude_message_ids: &'a [String],
}

impl ToolCtx<'_> {
    pub(crate) fn shelf_ids(&self) -> Vec<&str> {
        let mut ids = Vec::new();
        if let Some(id) = self.shelf_id {
            ids.push(id);
        }
        if let Some(id) = self.upload_shelf_id {
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
        ids
    }
}

#[derive(Debug, Clone)]
pub(crate) enum SourceChange {
    None,
    ReplaceOne(SourcePassage),
    Append(Vec<SourcePassage>),
    /// Replace the previous open window (and optional leftover sources) without
    /// dropping other excerpts from the same file.
    OpenWindow {
        opened: SourcePassage,
        drop_sids: Vec<String>,
        /// Character offset just after this window, for the next open.
        next_char: usize,
    },
}

pub(crate) struct ToolOutcome {
    pub message: String,
    pub change: SourceChange,
    pub file: Option<String>,
}

impl ToolOutcome {
    fn reply(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            change: SourceChange::None,
            file: None,
        }
    }
}

pub(crate) struct StatusHint {
    pub stage: &'static str,
    pub file: Option<String>,
}

pub(crate) fn status_hint(call: &ToolCall, sources: &[SourcePassage]) -> StatusHint {
    let Some(name) = ToolName::parse(&call.function.name) else {
        return StatusHint {
            stage: "thinking",
            file: None,
        };
    };
    let file = match name {
        ToolName::OpenFile => requested_file_name(call)
            .map(|s| clip_label(&s))
            .filter(|s| !s.is_empty()),
        ToolName::LookAround => look_around_label(call, sources),
        ToolName::ReadWebPage => web::page_label(&url_arg(call)),
        ToolName::SearchShelf | ToolName::SearchChats | ToolName::SearchWeb => None,
    };
    StatusHint {
        stage: name.stage(),
        file,
    }
}

fn look_around_label(call: &ToolCall, sources: &[SourcePassage]) -> Option<String> {
    let sid = parse_sid(&id_arg(call))?;
    let title = sources.iter().find(|s| s.sid == sid)?.title.as_str();
    let clipped = clip_label(title);
    if clipped.is_empty() {
        None
    } else {
        Some(clipped)
    }
}

pub(crate) async fn run_tool(call: &ToolCall, tool: &ToolCtx<'_>) -> ToolOutcome {
    let Some(name) = ToolName::parse(&call.function.name) else {
        let available = tool.allowed.listed_names();
        let message = if available.is_empty() {
            "No tools are available. Answer from what you have.".into()
        } else {
            format!("Unknown tool. Available: {available}.")
        };
        return ToolOutcome::reply(message);
    };
    if !tool.allowed.allows(name) {
        let available = tool.allowed.listed_names();
        let message = if available.is_empty() {
            "That isn't available now. Answer from what you have.".into()
        } else {
            format!("That isn't available now. Available: {available}.")
        };
        return ToolOutcome::reply(message);
    }
    match name {
        ToolName::SearchShelf => search::search_shelf(tool, &query_arg(call)),
        ToolName::LookAround => search::look_around(tool, &id_arg(call)),
        ToolName::OpenFile => open_shelf_file(tool, &requested_file_name(call).unwrap_or_default()),
        ToolName::SearchChats => search::search_chats(tool, &query_arg(call)),
        ToolName::SearchWeb | ToolName::ReadWebPage => {
            let value = if name == ToolName::SearchWeb {
                query_arg(call)
            } else {
                url_arg(call)
            };
            if !super::web_approval::request(
                tool.ctx,
                tool.thread_id,
                name.as_str(),
                &value,
                tool.cancel,
            )
            .await
            {
                return ToolOutcome::reply("Online request was not approved. Continue using local information; do not claim to have searched the web.");
            }
            if name == ToolName::SearchWeb {
                web::search_web(&value, tool.cancel).await
            } else {
                web::read_web_page(&value, tool.cancel).await
            }
        }
    }
}

pub(crate) fn apply_change(sources: &mut Vec<SourcePassage>, change: SourceChange) {
    match change {
        SourceChange::None => {}
        SourceChange::ReplaceOne(updated) => {
            if let Some(slot) = sources.iter_mut().find(|s| s.sid == updated.sid) {
                *slot = updated;
            } else {
                sources.push(updated);
            }
        }
        SourceChange::Append(new) => sources.extend(new),
        SourceChange::OpenWindow {
            opened, drop_sids, ..
        } => {
            sources.retain(|s| !drop_sids.iter().any(|id| id == &s.sid));
            if let Some(slot) = sources.iter_mut().find(|s| s.sid == opened.sid) {
                *slot = opened;
            } else {
                sources.push(opened);
            }
        }
    }
}

pub(crate) fn note_use(used: &mut ToolUse, call: &ToolCall, change: &SourceChange) {
    if let Some(name) = ToolName::parse(&call.function.name) {
        used.record(name, change);
    }
}

pub(crate) fn normalize_tool_name(name: &str) -> String {
    ToolName::parse(name)
        .map(|n| n.as_str().to_string())
        .unwrap_or_else(|| name.trim().to_string())
}

fn is_known_tool(name: &str) -> bool {
    ToolName::parse(name).is_some()
}

fn query_arg(call: &ToolCall) -> String {
    arg_string(
        &call.function.arguments,
        &["query", "q", "search", "keywords", "text"],
    )
    .unwrap_or_default()
}

fn id_arg(call: &ToolCall) -> String {
    arg_string(
        &call.function.arguments,
        &["id", "sid", "source", "source_id"],
    )
    .unwrap_or_default()
}

fn url_arg(call: &ToolCall) -> String {
    arg_string(
        &call.function.arguments,
        &["url", "href", "link", "page", "uri"],
    )
    .unwrap_or_default()
}

pub(crate) fn parse_sid(raw: &str) -> Option<String> {
    let trimmed = raw.trim().trim_matches(['[', ']', '"', '\'']).trim();
    if trimmed.is_empty() {
        return None;
    }
    let num = trimmed
        .strip_prefix('S')
        .or_else(|| trimmed.strip_prefix('s'))
        .unwrap_or(trimmed);
    let n: u32 = num.parse().ok()?;
    if n == 0 {
        return None;
    }
    Some(format!("S{n}"))
}

pub(crate) fn clip_label(name: &str) -> String {
    let cleaned = name.replace('\\', "/");
    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let n = trimmed.chars().count();
    if n <= 56 {
        trimmed.to_string()
    } else {
        format!("{}…", trimmed.chars().take(55).collect::<String>())
    }
}

pub(crate) fn sources_cost(sources: &[SourcePassage]) -> usize {
    sources.iter().map(crate::search::gate::passage_cost).sum()
}

pub(crate) fn remaining_budget(sources: &[SourcePassage], budget: usize) -> usize {
    budget.saturating_sub(sources_cost(sources))
}

pub(crate) fn sid_number(sid: &str) -> Option<u32> {
    sid.strip_prefix('S')?.parse().ok().filter(|&n| n > 0)
}

pub(crate) fn next_sid_number(sources: &[SourcePassage]) -> u32 {
    sources
        .iter()
        .filter_map(|s| sid_number(&s.sid))
        .max()
        .unwrap_or(0)
        + 1
}

/// Number passages for the prompt. A document keeps ids it already had in
/// this conversation. A new file never takes an id that names another file.
pub(crate) fn assign_sids<'a>(
    passages: &mut [SourcePassage],
    known: impl IntoIterator<Item = &'a SourcePassage>,
) {
    let mut owner: HashMap<u32, String> = HashMap::new();
    let mut by_doc: HashMap<String, Vec<u32>> = HashMap::new();
    for source in known {
        let Some(n) = sid_number(&source.sid) else {
            continue;
        };
        if source.document_id.is_empty() {
            continue;
        }
        match owner.get(&n) {
            Some(id) if id != &source.document_id => continue,
            Some(_) => {
                let list = by_doc.entry(source.document_id.clone()).or_default();
                if !list.contains(&n) {
                    list.push(n);
                }
            }
            None => {
                owner.insert(n, source.document_id.clone());
                by_doc
                    .entry(source.document_id.clone())
                    .or_default()
                    .push(n);
            }
        }
    }
    let mut used = HashSet::new();
    let mut next = owner.keys().copied().max().unwrap_or(0) + 1;
    for passage in passages.iter_mut() {
        let n = by_doc
            .get(&passage.document_id)
            .and_then(|ids| ids.iter().copied().find(|id| !used.contains(id)))
            .unwrap_or_else(|| {
                while used.contains(&next)
                    || owner
                        .get(&next)
                        .is_some_and(|id| id != &passage.document_id)
                {
                    next += 1;
                }
                let n = next;
                next += 1;
                owner.insert(n, passage.document_id.clone());
                by_doc
                    .entry(passage.document_id.clone())
                    .or_default()
                    .push(n);
                n
            });
        used.insert(n);
        passage.sid = format!("S{n}");
    }
}

pub(crate) fn format_passages(header: &str, passages: &[SourcePassage]) -> String {
    let mut out = header.to_string();
    out.push_str(" The user did not write this.");
    for source in passages {
        out.push_str(&format!(
            "\n\n[{}] {}\n{}",
            source.sid, source.title, source.body
        ));
    }
    out
}

pub(crate) fn assistant_tool_message(calls: &[ToolCall]) -> crate::engine::ChatMessage {
    crate::engine::ChatMessage {
        role: "assistant".into(),
        content: None,
        tool_calls: Some(calls.to_vec()),
        tool_call_id: None,
        name: None,
    }
}

pub(crate) fn tool_result_message(call: &ToolCall, content: String) -> crate::engine::ChatMessage {
    crate::engine::ChatMessage {
        role: "tool".into(),
        content: Some(content),
        tool_calls: None,
        tool_call_id: Some(call.id.clone()),
        name: Some(normalize_tool_name(&call.function.name)),
    }
}

/// Holds answer tokens while a tools round might still be a dumped call.
pub(crate) struct AnswerHoldback {
    buf: String,
    released: bool,
    enabled: bool,
}

impl AnswerHoldback {
    pub(crate) fn new(enabled: bool) -> Self {
        Self {
            buf: String::new(),
            released: false,
            enabled,
        }
    }

    pub(crate) fn push(&mut self, piece: &str) -> Option<String> {
        if !self.enabled || self.released {
            return Some(piece.to_string());
        }
        self.buf.push_str(piece);
        if Self::is_toolish(&self.buf) {
            return None;
        }
        if Self::looks_like_prose(&self.buf) {
            self.released = true;
            return Some(std::mem::take(&mut self.buf));
        }
        None
    }

    pub(crate) fn take_hidden(&mut self) -> String {
        std::mem::take(&mut self.buf)
    }

    pub(crate) fn released(&self) -> bool {
        self.released
    }

    /// Text the user should see. Drops withheld tool dumps that never parsed.
    pub(crate) fn visible_answer(&self, hidden: &str, output_answer: &str) -> String {
        if self.released {
            return output_answer.trim().to_string();
        }
        let text = if hidden.is_empty() {
            output_answer
        } else {
            hidden
        };
        if Self::is_toolish(text) {
            String::new()
        } else {
            text.trim().to_string()
        }
    }

    pub(crate) fn is_toolish(text: &str) -> bool {
        let t = text.trim_start();
        t.starts_with('{')
            || t.starts_with('[')
            || t.starts_with('<')
            || t.starts_with('`')
            || t.starts_with("open_shelf_file")
            || t.starts_with("search_shelf")
            || t.starts_with("look_around")
            || t.starts_with("search_chats")
            || t.starts_with("search_web")
            || t.starts_with("read_web_page")
            || t.starts_with("tool_call")
            || t.starts_with("call:")
    }

    fn looks_like_prose(text: &str) -> bool {
        let t = text.trim_start();
        if t.is_empty() || Self::is_toolish(t) {
            return false;
        }
        t.chars()
            .next()
            .is_some_and(|c| c.is_alphabetic() || c.is_numeric() || c == '#' || c == '*')
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_lists_the_tools_that_are_on() {
        let used = ToolUse::default();
        let all = ToolSet::new(true, true, false, true, &used);
        let spec = all.schema(&["a.md".into(), "b.md".into()]);
        let names: Vec<&str> = spec
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["function"]["name"].as_str().unwrap())
            .collect();
        assert_eq!(
            names,
            [SEARCH_SHELF, LOOK_AROUND, OPEN_SHELF_FILE, SEARCH_CHATS]
        );
        assert_eq!(
            spec[2]["function"]["parameters"]["properties"]["file"]["enum"][0],
            "a.md"
        );
        assert!(spec[0]["function"]["description"]
            .as_str()
            .unwrap()
            .contains("Use when"));
        let open = spec[2]["function"]["description"].as_str().unwrap();
        assert!(open.contains("Use when"));
        assert!(open.contains("next unread"));
        assert!(open.contains("Pass only the file name"));
        assert!(spec[2]["function"]["parameters"]["properties"]
            .get("offset")
            .is_none());

        let stuffed = ToolSet::new(false, false, false, true, &used);
        let spec = stuffed.schema(&[]);
        assert_eq!(spec.as_array().unwrap().len(), 1);
        assert_eq!(spec[0]["function"]["name"], SEARCH_CHATS);
        let chats = spec[0]["function"]["description"].as_str().unwrap();
        assert!(chats.contains("this conversation"));
        assert!(chats.contains("this Shelf"));
        assert!(chats.contains("Skip it for a new request"));

        let no_memory = ToolSet::new(false, false, false, false, &used);
        assert!(no_memory.schema(&[]).as_array().unwrap().is_empty());
        assert!(!no_memory.follow_up_useful());
        assert!(!ToolSet::new(false, false, false, true, &used).follow_up_useful());
        assert!(ToolSet::new(true, true, false, false, &used).follow_up_useful());

        let online = ToolSet::new(false, false, true, true, &used);
        let spec = online.schema(&[]);
        let names: Vec<&str> = spec
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["function"]["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, [SEARCH_CHATS, SEARCH_WEB, READ_WEB_PAGE]);
        let web_only = ToolSet::new(false, false, true, false, &used);
        let web_spec = web_only.schema(&[]);
        let names: Vec<&str> = web_spec
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["function"]["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, [SEARCH_WEB, READ_WEB_PAGE]);
        assert!(web_only.follow_up_useful());
        let search = spec
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["function"]["name"] == SEARCH_WEB)
            .unwrap();
        let query = search["function"]["parameters"]["properties"]["query"]["description"]
            .as_str()
            .unwrap();
        assert!(query.contains("No Shelf text"));
        let page = spec
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["function"]["name"] == READ_WEB_PAGE)
            .unwrap();
        let page_desc = page["function"]["description"].as_str().unwrap();
        assert!(page_desc.contains("Use after search_web"));
        assert!(page_desc.contains("Do not cite it as [S1]"));
    }

    #[test]
    fn aliases_normalize_to_canonical_names() {
        assert_eq!(ToolName::parse("read_file"), Some(ToolName::OpenFile));
        assert_eq!(
            ToolName::parse("search_this_shelf"),
            Some(ToolName::SearchShelf)
        );
        assert_eq!(ToolName::parse("expand_source"), Some(ToolName::LookAround));
        assert_eq!(
            ToolName::parse("search_memory"),
            Some(ToolName::SearchChats)
        );
        assert_eq!(ToolName::parse("web_search"), Some(ToolName::SearchWeb));
        assert_eq!(ToolName::parse("fetch_url"), Some(ToolName::ReadWebPage));
        assert_eq!(normalize_tool_name("read_file"), OPEN_SHELF_FILE);
    }

    #[test]
    fn parse_sid_accepts_brackets_and_bare_numbers() {
        assert_eq!(parse_sid("S2").as_deref(), Some("S2"));
        assert_eq!(parse_sid("[s1]").as_deref(), Some("S1"));
        assert_eq!(parse_sid("3").as_deref(), Some("S3"));
        assert_eq!(parse_sid(""), None);
        assert_eq!(parse_sid("S0"), None);
    }

    fn passage(sid: &str, doc: &str) -> SourcePassage {
        SourcePassage {
            anchor: None,
            sid: sid.into(),
            document_id: doc.into(),
            shelf_id: "s".into(),
            title: doc.into(),
            section: None,
            page_start: None,
            page_end: None,
            body: String::new(),
            path: format!("/{doc}"),
            score: 1.0,
        }
    }

    #[test]
    fn assign_sids_starts_at_one_and_keeps_a_file_on_its_id() {
        let known = [passage("S1", "mac"), passage("S2", "mac")];
        let mut fresh = [passage("", "mac"), passage("", "handbook")];
        assign_sids(&mut fresh, &known);
        assert_eq!(fresh[0].sid, "S1");
        assert_eq!(fresh[0].document_id, "mac");
        assert_eq!(fresh[1].sid, "S3");
        assert_eq!(fresh[1].document_id, "handbook");
    }

    #[test]
    fn assign_sids_ignores_a_later_turn_that_reused_an_id() {
        let known = [passage("S1", "mac"), passage("S1", "handbook")];
        let mut fresh = [passage("", "handbook"), passage("", "mac")];
        assign_sids(&mut fresh, &known);
        assert_eq!(
            fresh.iter().find(|s| s.document_id == "mac").unwrap().sid,
            "S1"
        );
        assert_eq!(
            fresh
                .iter()
                .find(|s| s.document_id == "handbook")
                .unwrap()
                .sid,
            "S2"
        );
    }

    #[test]
    fn holdback_hides_json_and_releases_prose() {
        let mut gate = AnswerHoldback::new(true);
        assert!(gate.push("{\"name\"").is_none());
        assert!(gate.push(":\"open_shelf_file\"}").is_none());
        assert!(!gate.released());
        assert!(gate.take_hidden().contains("open_shelf_file"));

        let mut gate = AnswerHoldback::new(true);
        let flushed = gate.push("The office kitchen is restocked.");
        assert!(flushed.unwrap().starts_with("The office"));
        assert!(gate.released());
        let hidden = gate.take_hidden();
        assert_eq!(
            gate.visible_answer(&hidden, "The office kitchen is restocked."),
            "The office kitchen is restocked."
        );

        let mut gate = AnswerHoldback::new(true);
        assert!(gate.push("{\"name\":\"open_shelf_file\"}").is_none());
        let hidden = gate.take_hidden();
        assert!(gate.visible_answer(&hidden, &hidden).is_empty());
    }
}
