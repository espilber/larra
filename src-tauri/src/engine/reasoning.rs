//! Native thinking knobs from the loaded chat template.
//!
//! Families disagree on names and defaults (Qwen3.8, gpt-oss, Gemma 4, …).
//! The Jinja llama-server compiled is the source of truth, not the file name.

use std::time::Duration;

use serde_json::{json, Value};

use super::{ChatThinking, Engine};

/// Lowest-to-highest effort words templates actually use.
const EFFORT_LEVELS: [&str; 6] = ["minimal", "low", "medium", "high", "xhigh", "max"];

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReasoningCaps {
    /// Template reads `enable_thinking` (hybrid on/off).
    toggle: bool,
    /// Quoted effort levels, lowest first. Empty if the template has none.
    efforts: Vec<String>,
    /// Template reads `reasoning_strength` as well as, or instead of, effort.
    uses_strength: bool,
    /// Think markup with no toggle and no effort — cannot be switched off.
    always_on: bool,
}

impl ReasoningCaps {
    pub fn sniff(template: &str) -> Self {
        let toggle = template.contains("enable_thinking");
        let uses_effort = template.contains("reasoning_effort");
        let uses_strength = template.contains("reasoning_strength");
        let efforts = if uses_effort || uses_strength {
            extract_effort_levels(template)
        } else {
            Vec::new()
        };
        let always_on = !toggle && efforts.is_empty() && has_think_markup(template);
        Self {
            toggle,
            efforts,
            uses_strength,
            always_on,
        }
    }

    fn lowest_effort(&self) -> Option<&str> {
        self.efforts.first().map(String::as_str)
    }
}

fn extract_effort_levels(template: &str) -> Vec<String> {
    EFFORT_LEVELS
        .iter()
        .filter(|level| {
            template.contains(&format!("'{level}'")) || template.contains(&format!("\"{level}\""))
        })
        .map(|level| (*level).to_string())
        .collect()
}

fn has_think_markup(template: &str) -> bool {
    template.contains("<think>") || template.contains("</think>") || template.contains("<|think|>")
}

/// Set thinking fields on a `/v1/chat/completions` body.
pub fn apply_to_body(body: &mut Value, thinking: ChatThinking, caps: &ReasoningCaps) {
    match thinking {
        ChatThinking::Off => apply_off(body),
        ChatThinking::Deep => apply_deep(body, caps),
    }
}

fn apply_off(body: &mut Value) {
    clear_top_level_effort(body);
    body["chat_template_kwargs"] = json!({ "enable_thinking": false });
}

fn apply_deep(body: &mut Value, caps: &ReasoningCaps) {
    if let Some(effort) = caps.lowest_effort() {
        let mut kwargs = serde_json::Map::new();
        if caps.toggle {
            kwargs.insert("enable_thinking".into(), json!(true));
        }
        kwargs.insert("reasoning_effort".into(), json!(effort));
        if caps.uses_strength {
            kwargs.insert("reasoning_strength".into(), json!(effort));
        }
        body["chat_template_kwargs"] = Value::Object(kwargs);
        body["reasoning_effort"] = json!(effort);
        return;
    }
    if caps.toggle {
        clear_top_level_effort(body);
        body["chat_template_kwargs"] = json!({ "enable_thinking": true });
        return;
    }
    if caps.always_on {
        clear_thinking(body);
        return;
    }
    apply_off(body);
}

fn clear_thinking(body: &mut Value) {
    if let Some(obj) = body.as_object_mut() {
        obj.remove("chat_template_kwargs");
        obj.remove("reasoning_effort");
    }
}

fn clear_top_level_effort(body: &mut Value) {
    if let Some(obj) = body.as_object_mut() {
        obj.remove("reasoning_effort");
    }
}

impl Engine {
    pub(super) async fn load_reasoning_caps(&self, base: &str) -> ReasoningCaps {
        match self
            .client
            .get(format!("{base}/props"))
            .timeout(Duration::from_secs(2))
            .send()
            .await
        {
            Ok(response) if response.status().is_success() => {
                let value: Value = response.json().await.unwrap_or_default();
                let template = value["chat_template"].as_str().unwrap_or("");
                let caps = ReasoningCaps::sniff(template);
                if caps != ReasoningCaps::default() {
                    log::info!("chat template thinking: {caps:?}");
                }
                caps
            }
            Ok(response) => {
                log::warn!("engine /props: {}", response.status());
                ReasoningCaps::default()
            }
            Err(error) => {
                log::warn!("engine /props: {error:#}");
                ReasoningCaps::default()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kwargs(body: &Value) -> &Value {
        &body["chat_template_kwargs"]
    }

    #[test]
    fn qwen38_deep_uses_low_effort_and_turns_thinking_on() {
        let template = r#"
{%- if enable_thinking is undefined or enable_thinking is true %}
 {%- set resolved_reasoning_effort = reasoning_effort| default('xhigh') %}
 {%- if resolved_reasoning_effort not in ('xhigh', 'medium', 'low') %}
 {{- raise_exception('Unexpected reasoning effort') }}
 {%- endif %}
{%- endif %}
{%- if enable_thinking is defined and enable_thinking is false %}
 {{- '<think>\n\n</think>\n\n' }}
{%- endif %}
"#;
        let caps = ReasoningCaps::sniff(template);
        assert!(caps.toggle);
        assert_eq!(caps.efforts, ["low", "medium", "xhigh"]);
        let mut body = json!({});
        apply_to_body(&mut body, ChatThinking::Deep, &caps);
        assert_eq!(kwargs(&body)["enable_thinking"], json!(true));
        assert_eq!(kwargs(&body)["reasoning_effort"], json!("low"));
        assert_eq!(body["reasoning_effort"], json!("low"));
    }

    #[test]
    fn gemma_style_toggle_only_turns_thinking_on() {
        let template =
            "{% if enable_thinking is defined and enable_thinking %}{{ '<|think|>' }}{% endif %}";
        let caps = ReasoningCaps::sniff(template);
        assert!(caps.toggle);
        assert!(caps.efforts.is_empty());
        let mut body = json!({});
        apply_to_body(&mut body, ChatThinking::Deep, &caps);
        assert_eq!(kwargs(&body), &json!({ "enable_thinking": true }));
        assert!(body.get("reasoning_effort").is_none());
    }

    #[test]
    fn gpt_oss_deep_sends_lowest_effort_without_disable() {
        let template = r#"{% set reasoning_effort = reasoning_effort | default('medium') %}
{%- if reasoning_effort not in ('low', 'medium', 'high') -%}
{%- endif -%}"#;
        let caps = ReasoningCaps::sniff(template);
        assert!(!caps.toggle);
        assert_eq!(caps.efforts, ["low", "medium", "high"]);
        let mut body = json!({});
        apply_to_body(&mut body, ChatThinking::Deep, &caps);
        assert!(kwargs(&body).get("enable_thinking").is_none());
        assert_eq!(kwargs(&body)["reasoning_effort"], json!("low"));
        assert_eq!(body["reasoning_effort"], json!("low"));
    }

    #[test]
    fn always_on_reasoner_omits_the_disable_kwarg() {
        let template = "{% generation %}{{ '</think>' }}{% endgeneration %}";
        let caps = ReasoningCaps::sniff(template);
        assert!(caps.always_on);
        let mut body = json!({ "chat_template_kwargs": { "enable_thinking": false } });
        apply_to_body(&mut body, ChatThinking::Deep, &caps);
        assert!(body.get("chat_template_kwargs").is_none());
        assert!(body.get("reasoning_effort").is_none());
    }

    #[test]
    fn plain_instruct_stays_off_on_deep() {
        let template =
            "{% for message in messages %}{{ message.role }}{{ message.content }}{% endfor %}";
        let caps = ReasoningCaps::sniff(template);
        let mut body = json!({});
        apply_to_body(&mut body, ChatThinking::Deep, &caps);
        assert_eq!(kwargs(&body), &json!({ "enable_thinking": false }));
    }

    #[test]
    fn off_always_disables_thinking() {
        let template = r#"{%- set reasoning_effort = reasoning_effort | default('high') %}
{%- if reasoning_effort not in ('low', 'medium', 'high') %}{% endif %}
{% if enable_thinking %}"#;
        let caps = ReasoningCaps::sniff(template);
        let mut body = json!({ "reasoning_effort": "low" });
        apply_to_body(&mut body, ChatThinking::Off, &caps);
        assert_eq!(kwargs(&body), &json!({ "enable_thinking": false }));
        assert!(body.get("reasoning_effort").is_none());
    }

    #[test]
    fn effort_var_without_quoted_levels_does_not_guess() {
        let template = "{% if reasoning_effort %}{{ reasoning_effort }}{% endif %}";
        let caps = ReasoningCaps::sniff(template);
        assert!(caps.efforts.is_empty());
        let mut body = json!({});
        apply_to_body(&mut body, ChatThinking::Deep, &caps);
        assert_eq!(kwargs(&body), &json!({ "enable_thinking": false }));
    }

    #[test]
    fn reasoning_strength_is_sent_beside_effort() {
        let template = "{% set reasoning_strength = reasoning_strength | default('low') %}";
        let caps = ReasoningCaps::sniff(template);
        assert_eq!(caps.efforts, ["low"]);
        let mut body = json!({});
        apply_to_body(&mut body, ChatThinking::Deep, &caps);
        assert_eq!(kwargs(&body)["reasoning_effort"], json!("low"));
        assert_eq!(kwargs(&body)["reasoning_strength"], json!("low"));
    }

    #[test]
    fn quoted_low_is_not_a_substring_of_lowest() {
        let template = "{% set reasoning_effort = 'lowest' %}";
        let caps = ReasoningCaps::sniff(template);
        assert!(caps.efforts.is_empty());
    }
}
