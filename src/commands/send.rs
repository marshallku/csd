//! `csd send` — inject a prompt with readiness/retry (PoC §2.2 + gotcha #1).
//!
//! Keystrokes sent before the TUI finishes initializing are silently dropped, and no fixed delay
//! is reliable. So: send literal text → wait → verify the text echoed in the input box → retry if
//! absent. Only once the echo is confirmed do we press Enter.

use std::thread::sleep;

use serde::Serialize;

use crate::commands::{SEND_SETTLE, SUBMIT_DELAY};
use crate::error::{Error, Result};
use crate::tmux;

/// How many trailing chars of the prompt to look for in the pane (robust to input-box wrapping).
const ECHO_TAIL_CHARS: usize = 40;

#[derive(Debug, Clone)]
pub struct SendArgs {
    pub session: String,
    pub prompt: String,
    pub submit: bool,
    pub retries: u32,
}

#[derive(Debug, Serialize)]
pub struct SendResult {
    pub ok: bool,
    pub attempts: u32,
}

pub fn run(args: SendArgs) -> Result<SendResult> {
    crate::session::validate_name(&args.session)?;
    let needle = echo_needle(&args.prompt);

    for attempt in 1..=args.retries {
        // Clear any partial input from a previous failed attempt, then paste the prompt.
        // paste_text (load-buffer + bracketed paste-buffer) is used instead of send-keys -l so
        // arbitrarily large, multi-line prompts land atomically: send-keys -l caps out (tmux
        // rejects a ~73 KiB argv) and types char-by-char, while a bracketed paste is delivered as
        // one unit that the TUI collapses to a "[Pasted text …]" placeholder (handled by echo).
        tmux::send_key(&args.session, "C-u")?;
        tmux::paste_text(&args.session, &args.prompt)?;
        sleep(SEND_SETTLE);

        let pane = tmux::capture_pane(&args.session)?;
        if echo_present(&pane, &needle) {
            if args.submit {
                sleep(SUBMIT_DELAY);
                tmux::send_key(&args.session, "Enter")?;
            }
            return Ok(SendResult {
                ok: true,
                attempts: attempt,
            });
        }
    }

    Err(Error::InputNotAccepted(args.retries))
}

/// The trailing run of the prompt we expect to see echoed, reduced to lowercase alphanumerics.
fn echo_needle(prompt: &str) -> String {
    let normalized = normalize(prompt);
    let tail: Vec<char> = normalized.chars().rev().take(ECHO_TAIL_CHARS).collect();
    tail.into_iter().rev().collect()
}

fn echo_present(pane: &str, needle: &str) -> bool {
    (!needle.is_empty() && normalize(pane).contains(needle)) || pasted_placeholder_present(pane)
}

/// claude's TUI collapses a large / multi-line paste into a `[Pasted text #N +K lines]`
/// placeholder instead of echoing the literal characters — so the trailing-needle check can
/// never match for a long prompt (e.g. a ~900-char system-prompt + message, which `send-keys -l`
/// delivers fast enough to be detected as a paste). The placeholder IS confirmation the input
/// landed, and a following Enter submits the full collapsed content, so treat it as a valid echo.
///
/// The match is scoped to the CURRENT input line — the text after the last prompt caret (`❯`) —
/// so a stale `[Pasted text …]` left in the scrollback / conversation history above (common in a
/// resumed multi-turn session) can't be mistaken for the current prompt landing and trigger a
/// premature submit. If the caret isn't found (e.g. a TUI glyph change), this returns false and
/// the primary tail-needle path stands — a loud miss, never a wrong submit.
fn pasted_placeholder_present(pane: &str) -> bool {
    match pane.rfind('❯') {
        Some(idx) => pane[idx..].contains("Pasted text"),
        None => false,
    }
}

/// Reduce text to lowercase alphanumerics, dropping whitespace, punctuation, and TUI box glyphs —
/// the input box wraps long prompts across bordered lines, so only the letters survive reliably.
fn normalize(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn echo_matches_despite_wrapping() {
        let prompt = "Ask me one clarifying question before doing anything.";
        let needle = echo_needle(prompt);
        // Simulate the TUI wrapping the prompt across lines with box padding.
        let pane = "│ Ask me one clarifying question\n│ before doing anything. │";
        assert!(echo_present(pane, &needle));
    }

    #[test]
    fn echo_absent_when_not_typed() {
        let needle = echo_needle("hello world this is a fairly long prompt to type");
        assert!(!echo_present("│ > │  (empty input box)", &needle));
    }

    #[test]
    fn echo_accepts_collapsed_paste_placeholder() {
        // A long multi-line prompt is collapsed by claude's TUI to a placeholder, so the literal
        // tail never appears — the placeholder must count as the input having landed.
        let needle = echo_needle("a very long multi line prompt ending with the word pong");
        let pane = "❯ [Pasted text #1 +8 lines]\n  paste again to expand";
        assert!(!normalize(pane).contains(&needle), "sanity: literal tail is absent");
        assert!(echo_present(pane, &needle));
    }

    #[test]
    fn stale_placeholder_above_the_input_caret_does_not_confirm() {
        // A "[Pasted text]" from a PRIOR turn sits in the scrollback above the current (empty)
        // input caret. It must NOT be read as the current short prompt landing — otherwise Enter
        // would submit an empty/old box. The needle for the current prompt is genuinely absent.
        let needle = echo_needle("brand new short prompt that was never typed");
        let pane = "  > earlier turn\n  [Pasted text #1 +8 lines]\n────────\n❯ ";
        assert!(!echo_present(pane, &needle));
    }
}
