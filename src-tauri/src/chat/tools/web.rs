//! Public-web lookup from this computer: Wikipedia, DuckDuckGo Instant Answer,
//! You.com (no key), and fetching a page as markdown.

use std::future::Future;
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::time::Duration;

use futures_util::StreamExt;
use readabilityrs::{markdown::MarkdownOptions, Readability, ReadabilityOptions};
use reqwest::header::{CONTENT_TYPE, LOCATION};
use reqwest::redirect::Policy;
use serde_json::Value;
use url::Url;

use super::{clip_label, ToolOutcome};

const USER_AGENT: &str = concat!(
    "Larra/",
    env!("CARGO_PKG_VERSION"),
    " (https://github.com/espilber/larra; github.com/espilber/larra/issues)"
);

const SEARCH_MAX_CHARS: usize = 6_000;
const PAGE_MAX_CHARS: usize = 8_000;
const HTML_MAX_BYTES: usize = 1_500_000;
const SEARCH_BODY_MAX: usize = 400_000;
const WIKI_HITS: usize = 5;
const YOU_HITS: usize = 6;
const FETCH_HOPS: usize = 5;
const STOPPED: &str = "The lookup was stopped.";

pub(super) async fn search_web(query: &str, cancel: &AtomicBool) -> ToolOutcome {
    let query = query.trim();
    if query.chars().count() < 2 {
        return ToolOutcome::reply("Write a search query for the public web.");
    }
    if cancel.load(Ordering::Relaxed) {
        return ToolOutcome::reply(STOPPED);
    }

    let lookup = async {
        let (wiki, ddg, you) = tokio::join!(
            wikipedia(query, cancel),
            duckduckgo(query, cancel),
            you_com(query, cancel)
        );
        combine_lookup([("Wikipedia", wiki), ("DuckDuckGo", ddg), ("You.com", you)])
    };
    match stoppable(cancel, lookup).await {
        Ok(text) => ToolOutcome::reply(text),
        Err(message) => ToolOutcome::reply(message),
    }
}

pub(super) async fn read_web_page(raw_url: &str, cancel: &AtomicBool) -> ToolOutcome {
    let raw = raw_url.trim();
    if raw.is_empty() {
        return ToolOutcome::reply("Need a web page URL.");
    }
    if cancel.load(Ordering::Relaxed) {
        return ToolOutcome::reply(STOPPED);
    }
    match fetch_markdown(raw, cancel).await {
        Ok(text) => ToolOutcome::reply(text),
        Err(message) => ToolOutcome::reply(message),
    }
}

async fn stoppable<T>(cancel: &AtomicBool, fut: impl Future<Output = T>) -> Result<T, String> {
    tokio::select! {
        _ = wait_cancelled(cancel) => Err(STOPPED.into()),
        result = fut => Ok(result),
    }
}

async fn wait_cancelled(cancel: &AtomicBool) {
    while !cancel.load(Ordering::Relaxed) {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn combine_lookup(
    parts: impl IntoIterator<Item = (&'static str, Result<String, String>)>,
) -> String {
    let mut body = String::from("# Online lookup\n");
    let mut any_ok = false;
    let mut any_content = false;
    let mut failed = Vec::new();
    for (name, result) in parts {
        match result {
            Ok(text) if !text.trim().is_empty() => {
                any_ok = true;
                any_content = true;
                body.push('\n');
                body.push_str(&crate::limits::clip_chars(
                    text.trim(),
                    SEARCH_MAX_CHARS / 3,
                ));
                body.push('\n');
            }
            Ok(_) => any_ok = true,
            Err(error) => {
                log::warn!("search_web {name}: {error}");
                failed.push(name);
            }
        }
    }
    if !any_ok {
        return "None of the online sources could be reached. Answer from what you have.".into();
    }
    if !any_content {
        let mut msg = "Online lookup found nothing useful. Answer from what you have.".to_string();
        if !failed.is_empty() {
            msg.push_str(&format!(" Unreachable: {}.", failed.join(", ")));
        }
        return msg;
    }
    if !failed.is_empty() {
        body.push_str(&format!("\nUnreachable: {}.\n", failed.join(", ")));
    }
    body.push_str(
        "\nThese are public web notes, not Shelf sources. Do not cite them as [S1]. \
Name the site or page title in prose.\n",
    );
    crate::limits::clip_chars(&body, SEARCH_MAX_CHARS)
}

async fn wikipedia(query: &str, cancel: &AtomicBool) -> Result<String, String> {
    let url = Url::parse_with_params(
        "https://en.wikipedia.org/w/api.php",
        &[
            ("action", "query"),
            ("list", "search"),
            ("srsearch", query),
            ("srlimit", "5"),
            ("srnamespace", "0"),
            ("srprop", "snippet"),
            ("format", "json"),
            ("utf8", "1"),
        ],
    )
    .map_err(|_| "bad request".to_string())?;
    let json = get_json(url, SEARCH_BODY_MAX, cancel).await?;
    Ok(format_wikipedia(&json))
}

async fn duckduckgo(query: &str, cancel: &AtomicBool) -> Result<String, String> {
    let url = Url::parse_with_params(
        "https://api.duckduckgo.com/",
        &[
            ("q", query),
            ("format", "json"),
            ("no_html", "1"),
            ("skip_disambig", "1"),
            ("no_redirect", "1"),
            ("t", "larra"),
        ],
    )
    .map_err(|_| "bad request".to_string())?;
    let json = get_json(url, SEARCH_BODY_MAX, cancel).await?;
    Ok(format_duckduckgo(&json))
}

async fn you_com(query: &str, cancel: &AtomicBool) -> Result<String, String> {
    let payload = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "you-search",
            "arguments": { "query": query }
        }
    });
    let send = search_client()
        .post("https://api.you.com/mcp?profile=free")
        .header("Accept", "application/json, text/event-stream")
        .json(&payload)
        .send();
    let response = stoppable(cancel, send).await?.map_err(net_err)?;
    if !response.status().is_success() {
        return Err(format!("HTTP {}", response.status().as_u16()));
    }
    let bytes = read_limited(response, SEARCH_BODY_MAX, cancel).await?;
    let text = String::from_utf8_lossy(&bytes);
    let value = parse_mcp_sse(&text).ok_or_else(|| "empty reply".to_string())?;
    if let Some(message) = value.pointer("/error/message").and_then(Value::as_str) {
        return Err(message.to_string());
    }
    Ok(format_you_com(&value))
}

fn format_wikipedia(json: &Value) -> String {
    let Some(hits) = json.pointer("/query/search").and_then(Value::as_array) else {
        return String::new();
    };
    let mut out = String::from("## Wikipedia\n");
    let mut n = 0usize;
    for hit in hits.iter().take(WIKI_HITS) {
        let title = hit
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim();
        if title.is_empty() {
            continue;
        }
        let snippet = strip_tags(hit.get("snippet").and_then(Value::as_str).unwrap_or(""));
        let href = wiki_url(title);
        n += 1;
        out.push_str(&format!("- **{title}** — {href}\n"));
        if !snippet.is_empty() {
            out.push_str(&format!("  {snippet}\n"));
        }
    }
    if n == 0 {
        String::new()
    } else {
        out
    }
}

fn format_duckduckgo(json: &Value) -> String {
    let heading = json
        .get("Heading")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    let abstract_text = json
        .get("AbstractText")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    let abstract_url = json
        .get("AbstractURL")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    let answer = json
        .get("Answer")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    let definition = json
        .get("Definition")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    let mut lines = Vec::new();
    if !heading.is_empty() {
        lines.push(format!("**{heading}**"));
    }
    if !answer.is_empty() {
        lines.push(answer.to_string());
    }
    if !abstract_text.is_empty() {
        lines.push(abstract_text.to_string());
    }
    if !definition.is_empty() && definition != abstract_text {
        lines.push(definition.to_string());
    }
    if !abstract_url.is_empty() {
        lines.push(format!("Source: {abstract_url}"));
    }
    let mut related = Vec::new();
    collect_ddg_topics(json.get("RelatedTopics"), &mut related, 5);
    for (text, href) in related {
        lines.push(format!("- {text} — {href}"));
    }
    if lines.is_empty() {
        String::new()
    } else {
        format!("## DuckDuckGo\n{}\n", lines.join("\n"))
    }
}

fn collect_ddg_topics(value: Option<&Value>, out: &mut Vec<(String, String)>, cap: usize) {
    let Some(Value::Array(items)) = value else {
        return;
    };
    for item in items {
        if out.len() >= cap {
            return;
        }
        if let Some(topics) = item.get("Topics") {
            collect_ddg_topics(Some(topics), out, cap);
            continue;
        }
        let text = item
            .get("Text")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim();
        let href = item
            .get("FirstURL")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim();
        if !text.is_empty() && !href.is_empty() {
            out.push((text.to_string(), href.to_string()));
        }
    }
}

fn format_you_com(rpc: &Value) -> String {
    let text = rpc
        .pointer("/result/content/0/text")
        .and_then(Value::as_str)
        .unwrap_or("");
    if text.trim().is_empty() {
        return String::new();
    }
    if let Ok(payload) = serde_json::from_str::<Value>(text) {
        return format_you_payload(&payload);
    }
    format!(
        "## You.com\n{}\n",
        crate::limits::clip_chars(text.trim(), SEARCH_MAX_CHARS / 3)
    )
}

fn format_you_payload(payload: &Value) -> String {
    let Some(web) = payload.pointer("/results/web").and_then(Value::as_array) else {
        return String::new();
    };
    let mut out = String::from("## You.com\n");
    let mut n = 0usize;
    for hit in web.iter().take(YOU_HITS) {
        let title = hit
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim();
        let href = hit.get("url").and_then(Value::as_str).unwrap_or("").trim();
        if title.is_empty() || href.is_empty() {
            continue;
        }
        let blurb = hit
            .get("description")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .or_else(|| {
                hit.get("snippets")
                    .and_then(Value::as_array)
                    .and_then(|s| s.first())
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
            })
            .unwrap_or("");
        n += 1;
        out.push_str(&format!("- **{title}** — {href}\n"));
        if !blurb.is_empty() {
            out.push_str(&format!("  {blurb}\n"));
        }
    }
    if n == 0 {
        String::new()
    } else {
        out
    }
}

fn parse_mcp_sse(body: &str) -> Option<Value> {
    let trimmed = body.trim();
    if trimmed.starts_with('{') {
        return serde_json::from_str(trimmed).ok();
    }
    let mut found = None;
    for line in body.lines() {
        let Some(data) = line.strip_prefix("data:") else {
            continue;
        };
        let data = data.trim();
        if data.is_empty() || data == "[DONE]" {
            continue;
        }
        if let Ok(value) = serde_json::from_str::<Value>(data) {
            if value.get("result").is_some() || value.get("error").is_some() {
                found = Some(value);
            }
        }
    }
    found
}

fn wiki_url(title: &str) -> String {
    let mut url = Url::parse("https://en.wikipedia.org/wiki/").expect("wiki base");
    {
        let mut path = url.path_segments_mut().expect("wiki path");
        path.pop();
        path.push(&title.replace(' ', "_"));
    }
    url.to_string()
}

fn strip_tags(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    for ch in html.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    decode_basic_entities(&out)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn decode_basic_entities(text: &str) -> String {
    text.replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
}

async fn fetch_markdown(raw: &str, cancel: &AtomicBool) -> Result<String, String> {
    let mut current = parse_public_url(raw)?;
    for _ in 0..FETCH_HOPS {
        if cancel.load(Ordering::Relaxed) {
            return Err(STOPPED.into());
        }
        let addrs = public_addrs(&current).await?;
        let host = current
            .host_str()
            .ok_or_else(|| "That URL is not allowed.".to_string())?;
        let client = pinned_fetch_client(host, &addrs)?;
        let send = client
            .get(current.clone())
            .header(
                "Accept",
                "text/html,application/xhtml+xml;q=0.9,text/plain;q=0.8",
            )
            .send();
        let response = stoppable(cancel, send).await?.map_err(net_err)?;
        let status = response.status();
        if status.is_redirection() {
            let loc = response
                .headers()
                .get(LOCATION)
                .and_then(|v| v.to_str().ok())
                .ok_or_else(|| "That page redirected somewhere Larra cannot open.".to_string())?;
            current = current
                .join(loc)
                .map_err(|_| "That page redirected somewhere Larra cannot open.".to_string())?;
            continue;
        }
        if !status.is_success() {
            return Err(format!("That page returned HTTP {}.", status.as_u16()));
        }
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("text/html")
            .to_ascii_lowercase();
        if content_type.contains("json")
            || content_type.contains("pdf")
            || content_type.contains("image/")
            || content_type.contains("audio/")
            || content_type.contains("video/")
            || content_type.contains("octet-stream")
        {
            return Err("That URL is not a web page Larra can read.".into());
        }
        let bytes = read_limited(response, HTML_MAX_BYTES, cancel).await?;
        let html = String::from_utf8_lossy(&bytes);
        let markdown = html_to_markdown(current.as_str(), &html);
        if markdown.trim().is_empty() {
            return Err("That page had no readable text.".into());
        }
        let host = current.host_str().unwrap_or("page");
        let mut out = format!("# Page: {host}\nURL: {current}\n\n{markdown}\n");
        out.push_str(
            "\nThis is a public web page, not a Shelf source. Do not cite it as [S1]. \
Name the page in prose.\n",
        );
        return Ok(crate::limits::clip_chars(&out, PAGE_MAX_CHARS));
    }
    Err("That page redirected too many times.".into())
}

fn html_to_markdown(page_url: &str, html: &str) -> String {
    let options = ReadabilityOptions::builder()
        .output_markdown(true)
        .char_threshold(80)
        .sanitize_content(true)
        .build();
    if let Ok(parser) = Readability::new(html, Some(page_url), Some(options)) {
        if let Some(article) = parser.parse() {
            let mut md = article.markdown_content.unwrap_or_default();
            if md.trim().is_empty() {
                md = article.text_content.unwrap_or_default();
            }
            let title = article.title.unwrap_or_default();
            if !title.is_empty() && !md.contains(&title) {
                md = format!("# {title}\n\n{md}");
            }
            return strip_data_urls(&md);
        }
    }
    let fallback = readabilityrs::markdown::html_to_markdown(
        html,
        &MarkdownOptions {
            sanitize_urls: true,
            ..MarkdownOptions::default()
        },
    );
    strip_data_urls(&fallback)
}

fn strip_data_urls(markdown: &str) -> String {
    let mut out = String::with_capacity(markdown.len());
    let mut rest = markdown;
    while let Some(start) = rest.find("](data:") {
        out.push_str(&rest[..start]);
        out.push_str("](");
        rest = &rest[start + 2..];
        if let Some(end) = rest.find(')') {
            rest = &rest[end..];
        } else {
            break;
        }
    }
    out.push_str(rest);
    out
}

pub(super) fn page_label(raw_url: &str) -> Option<String> {
    let url = parse_public_url(raw_url).ok()?;
    let host = url.host_str()?.trim();
    if host.is_empty() {
        None
    } else {
        Some(clip_label(host))
    }
}

fn parse_public_url(raw: &str) -> Result<Url, String> {
    let candidate = if raw.contains("://") {
        raw.to_string()
    } else {
        format!("https://{raw}")
    };
    let url = Url::parse(&candidate).map_err(|_| "That is not a valid URL.".to_string())?;
    if url.scheme() != "https" && url.scheme() != "http" {
        return Err("Only http and https pages can be opened.".into());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("That URL is not allowed.".into());
    }
    let port_ok = match url.port() {
        None => true,
        Some(80) if url.scheme() == "http" => true,
        Some(443) if url.scheme() == "https" => true,
        _ => false,
    };
    if !port_ok {
        return Err("That URL is not allowed.".into());
    }
    Ok(url)
}

async fn public_addrs(url: &Url) -> Result<Vec<SocketAddr>, String> {
    let host = url
        .host_str()
        .ok_or_else(|| "That URL is not allowed.".to_string())?;
    let lower = host.to_ascii_lowercase();
    if lower == "localhost"
        || lower.ends_with(".localhost")
        || lower.ends_with(".local")
        || lower.ends_with(".internal")
        || lower == "metadata.google.internal"
    {
        return Err("That URL is not allowed.".into());
    }
    let port = url.port_or_known_default().unwrap_or(443);
    if let Ok(ip) = lower.parse::<IpAddr>() {
        if is_blocked_ip(ip) {
            return Err("That URL is not allowed.".into());
        }
        return Ok(vec![SocketAddr::new(ip, port)]);
    }
    let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host, port))
        .await
        .map_err(|_| "That page could not be reached.".to_string())?
        .collect();
    if addrs.is_empty() {
        return Err("That page could not be reached.".into());
    }
    if addrs.iter().any(|addr| is_blocked_ip(addr.ip())) {
        return Err("That URL is not allowed.".into());
    }
    Ok(addrs)
}

fn is_blocked_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v) => {
            v.is_loopback()
                || v.is_private()
                || v.is_link_local()
                || v.is_unspecified()
                || v.is_broadcast()
                || v.is_multicast()
                || v.is_documentation()
                || v.octets()[0] == 0
                || (v.octets()[0] == 100 && v.octets()[1] & 0b1100_0000 == 64)
        }
        IpAddr::V6(v) => {
            if let Some(mapped) = v.to_ipv4_mapped() {
                return is_blocked_ip(IpAddr::V4(mapped));
            }
            v.is_loopback()
                || v.is_unique_local()
                || v.is_unicast_link_local()
                || v.is_unspecified()
                || v.is_multicast()
        }
    }
}

fn search_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(Duration::from_secs(12))
            .connect_timeout(Duration::from_secs(8))
            .redirect(Policy::limited(3))
            .build()
            .expect("web search client")
    })
}

fn pinned_fetch_client(host: &str, addrs: &[SocketAddr]) -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(Duration::from_secs(15))
        .connect_timeout(Duration::from_secs(8))
        .redirect(Policy::none())
        .resolve_to_addrs(&host.to_ascii_lowercase(), addrs)
        .build()
        .map_err(|_| "That page could not be reached.".to_string())
}

async fn get_json(url: Url, max_bytes: usize, cancel: &AtomicBool) -> Result<Value, String> {
    let send = search_client()
        .get(url)
        .header("Api-User-Agent", USER_AGENT)
        .send();
    let response = stoppable(cancel, send).await?.map_err(net_err)?;
    if !response.status().is_success() {
        return Err(format!("HTTP {}", response.status().as_u16()));
    }
    let bytes = read_limited(response, max_bytes, cancel).await?;
    serde_json::from_slice(&bytes).map_err(|_| "bad reply".to_string())
}

async fn read_limited(
    response: reqwest::Response,
    max_bytes: usize,
    cancel: &AtomicBool,
) -> Result<Vec<u8>, String> {
    if response
        .content_length()
        .is_some_and(|n| n as usize > max_bytes)
    {
        return Err("That reply was too large.".into());
    }
    let mut out = Vec::new();
    let mut stream = response.bytes_stream();
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(STOPPED.into());
        }
        let Some(chunk) = stream.next().await else {
            break;
        };
        let chunk = chunk.map_err(net_err)?;
        if out.len().saturating_add(chunk.len()) > max_bytes {
            return Err("That reply was too large.".into());
        }
        out.extend_from_slice(&chunk);
    }
    Ok(out)
}

fn net_err(error: reqwest::Error) -> String {
    if error.is_timeout() {
        "timed out".into()
    } else {
        "unreachable".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn wikipedia_json_becomes_markdown_links() {
        let md = format_wikipedia(&json!({
            "query": {
                "search": [{
                    "title": "Paris",
                    "snippet": "Capital of <span class=\"searchmatch\">France</span>."
                }]
            }
        }));
        assert!(md.contains("**Paris**"));
        assert!(md.contains("https://en.wikipedia.org/wiki/Paris"));
        assert!(md.contains("Capital of France."));
        assert!(!md.contains("<span"));
    }

    #[test]
    fn duckduckgo_instant_answer_and_related() {
        let md = format_duckduckgo(&json!({
            "Heading": "Paris",
            "AbstractText": "Capital of France.",
            "AbstractURL": "https://en.wikipedia.org/wiki/Paris",
            "RelatedTopics": [
                {"Text": "France", "FirstURL": "https://duckduckgo.com/France"},
                {"Name": "More", "Topics": [
                    {"Text": "Eiffel Tower", "FirstURL": "https://duckduckgo.com/Eiffel_Tower"}
                ]}
            ]
        }));
        assert!(md.starts_with("## DuckDuckGo"));
        assert!(md.contains("Capital of France."));
        assert!(md.contains("Eiffel Tower"));
    }

    #[test]
    fn you_com_mcp_sse_json_payload() {
        let sse = concat!(
            "event: message\n",
            "data: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/message\"}\n\n",
            "event: message\n",
            r#"data: {"jsonrpc":"2.0","id":1,"result":{"content":[{"type":"text","text":"{\"results\":{\"web\":[{\"title\":\"Example Domain\",\"url\":\"https://example.com\",\"description\":\"Example blurb\",\"snippets\":[\"skip me\"]}]}}"}]}}"#,
            "\n\n"
        );
        let value = parse_mcp_sse(sse).unwrap();
        let md = format_you_com(&value);
        assert!(md.contains("## You.com"));
        assert!(md.contains("**Example Domain**"));
        assert!(md.contains("https://example.com"));
        assert!(md.contains("Example blurb"));
        assert!(!md.contains("skip me"));
    }

    #[test]
    fn combine_uses_live_sources_and_names_failures() {
        let text = combine_lookup([
            (
                "Wikipedia",
                Ok("## Wikipedia\n- **Paris** — https://en.wikipedia.org/wiki/Paris\n".into()),
            ),
            ("DuckDuckGo", Err("timeout".into())),
            ("You.com", Ok(String::new())),
        ]);
        assert!(text.contains("**Paris**"));
        assert!(text.contains("Unreachable: DuckDuckGo."));
        assert!(text.contains("Do not cite them as [S1]"));
        assert!(!text.contains("[S1] Paris"));
    }

    #[test]
    fn combine_all_down_is_a_clear_miss() {
        let text = combine_lookup([
            ("Wikipedia", Err("down".into())),
            ("DuckDuckGo", Err("down".into())),
            ("You.com", Err("down".into())),
        ]);
        assert!(text.contains("None of the online sources could be reached"));
    }

    #[test]
    fn combine_empty_hits_is_a_clear_miss() {
        let text = combine_lookup([
            ("Wikipedia", Ok(String::new())),
            ("DuckDuckGo", Ok(String::new())),
            ("You.com", Ok("  ".into())),
        ]);
        assert!(text.contains("Online lookup found nothing useful"));
    }

    #[tokio::test]
    async fn loopback_and_metadata_hosts_are_refused() {
        for raw in [
            "http://127.0.0.1/secret",
            "https://[::1]/",
            "http://169.254.169.254/latest/meta-data/",
        ] {
            let url = Url::parse(raw).unwrap();
            assert!(public_addrs(&url).await.is_err(), "{raw} should be refused");
        }
    }

    #[tokio::test]
    async fn public_ip_literal_is_pinned_for_connect() {
        let url = Url::parse("https://1.1.1.1/").unwrap();
        let addrs = public_addrs(&url).await.unwrap();
        assert_eq!(addrs[0].ip(), "1.1.1.1".parse::<IpAddr>().unwrap());
        assert!(pinned_fetch_client("1.1.1.1", &addrs).is_ok());
        let example = SocketAddr::from(([93, 184, 216, 34], 443));
        assert!(pinned_fetch_client("example.com", &[example]).is_ok());
    }

    #[tokio::test]
    async fn search_web_stops_without_waiting_on_the_network() {
        let cancel = AtomicBool::new(true);
        let started = std::time::Instant::now();
        let out = search_web("paris weather", &cancel).await;
        assert!(out.message.contains("stopped"));
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
    }

    #[test]
    fn private_urls_are_rejected() {
        assert!(parse_public_url("http://127.0.0.1/secret").is_ok());
        assert!(is_blocked_ip("127.0.0.1".parse().unwrap()));
        assert!(is_blocked_ip("10.0.0.4".parse().unwrap()));
        assert!(is_blocked_ip("192.168.1.1".parse().unwrap()));
        assert!(is_blocked_ip("169.254.169.254".parse().unwrap()));
        assert!(is_blocked_ip("::1".parse().unwrap()));
        assert!(!is_blocked_ip("1.1.1.1".parse().unwrap()));
        assert!(parse_public_url("file:///etc/passwd").is_err());
        assert!(parse_public_url("https://example.com:8080/").is_err());
        assert!(parse_public_url("https://user:pass@example.com/").is_err());
    }

    #[test]
    fn html_article_becomes_markdown_without_chrome() {
        let html = r#"<html><body>
            <nav>Skip me</nav>
            <article><h1>Office zebra</h1><p>The zebra lives in the east wing.</p></article>
            <footer>Ads</footer>
        </body></html>"#;
        let md = html_to_markdown("https://example.com/zebra", html);
        assert!(md.to_lowercase().contains("zebra"));
        assert!(!md.contains("<p>"));
        assert!(!md.contains("<nav>"));
    }

    #[test]
    fn page_label_uses_the_host() {
        assert_eq!(
            page_label("https://en.wikipedia.org/wiki/Paris").as_deref(),
            Some("en.wikipedia.org")
        );
        assert_eq!(page_label(""), None);
    }
}
