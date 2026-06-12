//! Session-transcript (JSONL) classification — the reliable signal for questions and normal turn
//! completion (PoC §3.1–3.4). One JSON object per line.
//!
//! [`classify`] is a PURE function over already-parsed events so it can be unit-tested against
//! captured fixtures. The JSONL lags live state by ~one tool-call (PoC gotcha #3), so the caller
//! ([`crate::detect::combine`]) layers capture-pane + plan-file on top for TUI-interrupt gates.

use std::fs;
use std::path::Path;

use serde_json::Value;

use crate::error::{Error, Result};

/// What the transcript alone says about the session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JsonlState {
    /// Last assistant turn ended with a question and nothing has answered it.
    AwaitingAnswer { question: String },
    /// An `ExitPlanMode` tool_use is present (rare in the transcript — usually only post-approval).
    PlanReady { plan: Option<String> },
    /// A tool call is in flight (assistant emitted tool_use, no result yet).
    ToolPending { tools: Vec<String> },
    /// Assistant finished cleanly and is waiting for the human.
    IdleDone { text: String },
    /// Assistant is mid-stream or a tool result just returned — work is ongoing.
    Working,
    /// No assistant event yet, or shape not understood.
    Unknown,
}

/// Parse a transcript file into one [`Value`] per non-empty line. Unparseable lines are skipped.
pub fn load(path: &Path) -> Result<Vec<Value>> {
    let body = fs::read_to_string(path).map_err(|e| Error::io(path, e))?;
    Ok(parse(&body))
}

/// Parse newline-delimited JSON, skipping blank and malformed lines.
pub fn parse(body: &str) -> Vec<Value> {
    body.lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

/// Count the turn-relevant events — `type` `user`/`assistant`, the only kinds [`classify`] keys
/// off. `run` records this before sending as a freshness watermark: a transcript carries far more
/// metadata events (`system`, `ai-title`, `attachment`, …) than turn events, and metadata keeps
/// landing AFTER the final assistant message (live census: 6 turn events in a 38-line transcript),
/// so a raw line count would unblock a wait on stale state.
pub fn turn_event_count(events: &[Value]) -> usize {
    events
        .iter()
        .filter(|e| matches!(event_type(e), Some("user") | Some("assistant")))
        .count()
}

/// Classify a turn from the transcript events alone (PoC §3.4, with the later-user refinement).
pub fn classify(events: &[Value]) -> JsonlState {
    let Some(last_ai_idx) = events.iter().rposition(|e| event_type(e) == Some("assistant")) else {
        // No assistant turn yet: a user event means a prompt was sent and the first reply is
        // pending (Working); otherwise the session just started and awaits a prompt (Unknown).
        return if events.iter().any(|e| event_type(e) == Some("user")) {
            JsonlState::Working
        } else {
            JsonlState::Unknown
        };
    };
    let last_ai = &events[last_ai_idx];
    let message = &last_ai["message"];
    let stop = message["stop_reason"].as_str();
    let blocks = content_blocks(message);

    // A plan tool_use is authoritative when present — but it usually only lands post-approval, so
    // the real plan-ready detection happens via capture-pane + plan file in `detect::combine`.
    if let Some(plan) = plan_tool_input(&blocks) {
        return JsonlState::PlanReady { plan };
    }

    // A user event after the last assistant event means a tool result returned (or a new turn
    // started) — the model is about to speak again. Not waiting on the human.
    let later_user = events[last_ai_idx + 1..].iter().any(|e| event_type(e) == Some("user"));
    if later_user {
        return JsonlState::Working;
    }

    let text = text_of(&blocks);
    let tools = tool_names(&blocks);

    if stop == Some("end_turn") && tools.is_empty() && looks_like_question(&text) {
        return JsonlState::AwaitingAnswer { question: text };
    }
    if !tools.is_empty() && matches!(stop, None | Some("tool_use")) {
        return JsonlState::ToolPending { tools };
    }
    if stop == Some("end_turn") {
        return JsonlState::IdleDone { text };
    }
    // stop is None / unrecognized with no tool calls → still generating.
    JsonlState::Working
}

/// Whether an end-of-turn message reads as a question to the user (PoC §3.2, hardened).
///
/// A plain `ends_with('?')` is too strict: live clarifying questions trail clarifiers ("...the
/// goal? (I don't see a task yet.)") or a polite closing line after the question. So we look for a
/// `?` anywhere — but first drop backtick-delimited code spans/fences so a `?` inside code (regex,
/// ternary, URL query) on a completion turn doesn't read as a question.
fn looks_like_question(text: &str) -> bool {
    text.split('`').step_by(2).any(|outside| outside.contains('?'))
}

fn event_type(event: &Value) -> Option<&str> {
    event["type"].as_str()
}

/// Assistant `message.content` is an array of blocks; tolerate a bare string too.
fn content_blocks(message: &Value) -> Vec<Value> {
    match &message["content"] {
        Value::Array(blocks) => blocks.clone(),
        Value::String(text) => vec![serde_json::json!({ "type": "text", "text": text })],
        _ => Vec::new(),
    }
}

fn text_of(blocks: &[Value]) -> String {
    blocks
        .iter()
        .filter(|b| b["type"] == "text")
        .filter_map(|b| b["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

fn tool_names(blocks: &[Value]) -> Vec<String> {
    blocks
        .iter()
        .filter(|b| b["type"] == "tool_use")
        .filter_map(|b| b["name"].as_str().map(String::from))
        .collect()
}

/// If a block is an `ExitPlanMode` tool_use, return its `input.plan` text (None if absent/empty).
fn plan_tool_input(blocks: &[Value]) -> Option<Option<String>> {
    blocks
        .iter()
        .find(|b| {
            b["type"] == "tool_use" && matches!(b["name"].as_str(), Some("ExitPlanMode") | Some("exit_plan_mode"))
        })
        .map(|b| b["input"]["plan"].as_str().map(String::from))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turn_event_count_ignores_metadata_events() {
        let events = parse(
            r#"{"type":"user"}
{"type":"assistant"}
{"type":"system"}
{"type":"ai-title"}
{"type":"attachment"}
{"type":"file-history-snapshot"}
{"type":"assistant"}
{"no_type_at_all":true}"#,
        );
        assert_eq!(turn_event_count(&events), 3);
        assert_eq!(turn_event_count(&[]), 0);
    }

    #[test]
    fn question_with_trailing_parenthetical_is_detected() {
        // Observed live (claude v2.1.158): the '?' is not the last char.
        let text = "What would you like me to work on? (I don't see a task yet — tell me the goal.)";
        assert!(looks_like_question(text));
    }

    #[test]
    fn plain_question_is_detected() {
        assert!(looks_like_question("What's the goal you'd like me to work on?"));
    }

    #[test]
    fn question_followed_by_closing_line_is_detected() {
        assert!(looks_like_question(
            "What would you like me to work on?\nLet me know and I'll get started."
        ));
    }

    #[test]
    fn completion_message_is_not_a_question() {
        assert!(!looks_like_question("Done. The file was written and the tests pass."));
    }

    #[test]
    fn question_mark_only_in_code_is_not_a_question() {
        assert!(!looks_like_question(
            "Updated the regex to `\\d+?` and reran the suite."
        ));
    }
}
