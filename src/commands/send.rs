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
        // Clear any partial input from a previous failed attempt, then type the prompt.
        tmux::send_key(&args.session, "C-u")?;
        tmux::send_literal(&args.session, &args.prompt)?;
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
    !needle.is_empty() && normalize(pane).contains(needle)
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
}
