//! Unit tests for the pure transcript classifier against captured fixtures (PoC §3.4).

use std::path::Path;

use csd::detect::jsonl::{self, JsonlState};

fn classify_fixture(name: &str) -> JsonlState {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name);
    let events = jsonl::load(&path).expect("fixture should load");
    jsonl::classify(&events)
}

#[test]
fn detects_clarifying_question() {
    match classify_fixture("awaiting-answer.jsonl") {
        JsonlState::AwaitingAnswer { question } => {
            assert_eq!(question, "What's the goal you'd like me to work on?");
        }
        other => panic!("expected AwaitingAnswer, got {other:?}"),
    }
}

#[test]
fn detects_idle_done() {
    match classify_fixture("idle-done.jsonl") {
        JsonlState::IdleDone { text } => assert!(text.contains("complete")),
        other => panic!("expected IdleDone, got {other:?}"),
    }
}

#[test]
fn detects_tool_pending() {
    match classify_fixture("tool-pending.jsonl") {
        JsonlState::ToolPending { tools } => assert_eq!(tools, vec!["Bash".to_string()]),
        other => panic!("expected ToolPending, got {other:?}"),
    }
}

#[test]
fn detects_plan_ready_from_exit_plan_mode() {
    match classify_fixture("plan-ready.jsonl") {
        JsonlState::PlanReady { plan } => {
            assert!(plan.unwrap().contains("Edit foo.rs"));
        }
        other => panic!("expected PlanReady, got {other:?}"),
    }
}

#[test]
fn tool_result_returns_working() {
    // A user (tool_result) event after the last assistant means work is ongoing, not awaiting human.
    assert_eq!(classify_fixture("working.jsonl"), JsonlState::Working);
}

#[test]
fn empty_transcript_is_unknown() {
    assert_eq!(jsonl::classify(&[]), JsonlState::Unknown);
}
