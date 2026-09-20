//! Length caps for text the user types (House rules, Recipes, Chat).

/// Standing instructions go out on every turn. Keep this well under a Chat
/// message so House rules cannot fill the window on their own.
pub const HOUSE_RULES_MAX_CHARS: usize = 4_000;

/// Saved thinking on a message. Enough to skim, not a second copy of the answer.
pub const THINKING_MAX_CHARS: usize = 8_000;

/// Recipe prompts and the Chat composer. About three pages — enough for a
/// long paste, short of crowding out history and retrieved files.
pub const PROMPT_MAX_CHARS: usize = 12_000;

pub fn clip_chars(text: &str, max: usize) -> String {
    let count = text.chars().count();
    if count <= max {
        text.to_string()
    } else {
        text.chars().take(max).collect()
    }
}

/// Clip to `max` characters and end with an ellipsis when cut.
pub fn clip_chars_ellipsis(text: &str, max: usize) -> String {
    let count = text.chars().count();
    if count <= max {
        return text.to_string();
    }
    format!(
        "{}…",
        text.chars().take(max.saturating_sub(1)).collect::<String>()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clip_chars_keeps_short_text() {
        assert_eq!(clip_chars("hello", 12), "hello");
    }

    #[test]
    fn clip_chars_cuts_at_the_limit() {
        let text: String = "á".repeat(PROMPT_MAX_CHARS + 8);
        let clipped = clip_chars(&text, PROMPT_MAX_CHARS);
        assert_eq!(clipped.chars().count(), PROMPT_MAX_CHARS);
        assert!(clipped.chars().all(|c| c == 'á'));
    }

    #[test]
    fn clip_chars_ellipsis_marks_a_cut() {
        assert_eq!(clip_chars_ellipsis("hello", 12), "hello");
        assert_eq!(clip_chars_ellipsis("abcdefghij", 6), "abcde…");
    }
}
