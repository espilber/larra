//! Split llama.cpp token streams into thinking vs answer.

use super::ChatOutput;

const THINK_OPEN: &str = "<think>";
const THINK_CLOSE: &str = "</think>";

pub(crate) enum SplitOut {
    Thinking(String),
    Answer(String),
    Promote,
}

impl ChatOutput {
    pub(crate) fn apply(&mut self, event: &SplitOut) {
        match event {
            SplitOut::Thinking(t) => self.thinking.push_str(t),
            SplitOut::Answer(t) => self.answer.push_str(t),
            SplitOut::Promote => {
                let moved = std::mem::take(&mut self.answer);
                self.thinking.push_str(&moved);
            }
        }
    }
}

/// Splits a token stream into thinking and answer parts. Handles all three
/// shapes seen in the wild: `<think>…</think>answer`, reasoning-first with
/// only a closing tag (`…</think>answer`), and plain answers. A small
/// holdback keeps tags from slipping through when they split across chunks.
pub(crate) struct ThinkSplitter {
    state: SplitState,
    pending: String,
    saw_think: bool,
}

enum SplitState {
    Start,
    Thinking,
    Answer,
}

impl ThinkSplitter {
    pub(crate) fn new() -> Self {
        Self {
            state: SplitState::Start,
            pending: String::new(),
            saw_think: false,
        }
    }

    pub(crate) fn push(&mut self, piece: &str) -> Vec<SplitOut> {
        self.pending.push_str(piece);
        let mut out = Vec::new();
        loop {
            match self.state {
                SplitState::Start => {
                    let trimmed = self.pending.trim_start();
                    if trimmed.is_empty() {
                        break;
                    }
                    if let Some(after) = trimmed.strip_prefix(THINK_OPEN) {
                        self.pending = after.to_string();
                        self.state = SplitState::Thinking;
                        self.saw_think = true;
                        continue;
                    }
                    if THINK_OPEN.starts_with(trimmed) {
                        break;
                    }
                    self.state = SplitState::Answer;
                    continue;
                }
                SplitState::Thinking => {
                    if let Some(pos) = self.pending.find(THINK_CLOSE) {
                        if pos > 0 {
                            out.push(SplitOut::Thinking(self.pending[..pos].to_string()));
                        }
                        self.pending = self.pending[pos + THINK_CLOSE.len()..]
                            .trim_start()
                            .to_string();
                        self.state = SplitState::Answer;
                        continue;
                    }
                    self.drain_with_holdback(&mut out, true);
                    break;
                }
                SplitState::Answer => {
                    if !self.saw_think {
                        if let Some(pos) = self.pending.find(THINK_CLOSE) {
                            out.push(SplitOut::Promote);
                            if pos > 0 {
                                out.push(SplitOut::Thinking(self.pending[..pos].to_string()));
                            }
                            self.pending = self.pending[pos + THINK_CLOSE.len()..]
                                .trim_start()
                                .to_string();
                            self.saw_think = true;
                            continue;
                        }
                        self.drain_with_holdback(&mut out, false);
                        break;
                    }
                    if !self.pending.is_empty() {
                        out.push(SplitOut::Answer(std::mem::take(&mut self.pending)));
                    }
                    break;
                }
            }
        }
        out
    }

    fn drain_with_holdback(&mut self, out: &mut Vec<SplitOut>, thinking: bool) {
        let keep = self
            .pending
            .len()
            .saturating_sub(THINK_CLOSE.len().saturating_sub(1));
        let mut cut = keep;
        while cut > 0 && !self.pending.is_char_boundary(cut) {
            cut -= 1;
        }
        if cut == 0 {
            return;
        }
        let emit: String = self.pending.drain(..cut).collect();
        out.push(if thinking {
            SplitOut::Thinking(emit)
        } else {
            SplitOut::Answer(emit)
        });
    }

    pub(crate) fn flush(&mut self) -> Vec<SplitOut> {
        let mut out = Vec::new();
        if !self.pending.is_empty() {
            let rest = std::mem::take(&mut self.pending);
            out.push(match self.state {
                SplitState::Thinking => SplitOut::Thinking(rest),
                _ => SplitOut::Answer(rest),
            });
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_splitter(pieces: &[&str]) -> ChatOutput {
        let mut splitter = ThinkSplitter::new();
        let mut output = ChatOutput::default();
        for piece in pieces {
            for event in splitter.push(piece) {
                output.apply(&event);
            }
        }
        for event in splitter.flush() {
            output.apply(&event);
        }
        output.answer = output.answer.trim().to_string();
        output.thinking = output.thinking.trim().to_string();
        output
    }

    #[test]
    fn splitter_separates_tagged_reasoning() {
        let out = run_splitter(&[
            "<th",
            "ink>reasoning here",
            " more</think>",
            "The answer",
            " is 4.",
        ]);
        assert_eq!(out.answer, "The answer is 4.");
        assert_eq!(out.thinking, "reasoning here more");
    }

    #[test]
    fn splitter_passes_normal_text() {
        let out = run_splitter(&["Hello", " there"]);
        assert_eq!(out.answer, "Hello there");
        assert!(out.thinking.is_empty());
    }

    #[test]
    fn splitter_promotes_reasoning_without_opening_tag() {
        let out = run_splitter(&[
            "Let me think about th",
            "is carefully.",
            "</th",
            "ink>",
            "It is 4.",
        ]);
        assert_eq!(out.answer, "It is 4.");
        assert_eq!(out.thinking, "Let me think about this carefully.");
    }
}
