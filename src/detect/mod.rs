//! The hybrid state detector. No single source is sufficient (PoC §3): the transcript is reliable
//! for questions/turns but lags TUI-interrupt gates; capture-pane catches the gates but its markers
//! shift per release; the plan file holds plan content. [`detect`] combines all three.

pub mod jsonl;
pub mod pane;
pub mod plan;

use std::path::PathBuf;

use serde::Serialize;

use crate::backend;
use crate::error::Result;
use crate::session::Session;
use crate::tmux;

/// The externally-visible session state, serialized under a `status` tag for JSON consumers.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum State {
    /// tmux session is up but the TUI hasn't produced a classifiable turn yet.
    Spawning,
    /// Assistant is streaming or a tool call is in flight (`tools` empty if just streaming).
    Working { tools: Vec<String> },
    /// Assistant asked a clarifying question and is waiting for an answer (PoC §3.2).
    AwaitingAnswer { question: String },
    /// A plan is ready and the approval gate is on screen (PoC §3.3).
    PlanReady {
        plan_file: Option<PathBuf>,
        plan: Option<String>,
    },
    /// A TUI gate is on screen (`gate` = "trust" | "permission") — answer it with `csd approve`.
    Blocked {
        gate: String,
        prompt: String,
        options: Vec<String>,
    },
    /// Assistant finished its turn cleanly and is waiting for the human.
    IdleDone { text: String },
    /// The tmux session is gone.
    Dead,
    /// Session is up but produced output we don't recognize.
    Unknown,
}

/// Combine all three signals into a single [`State`] (precedence: capture-pane gates → transcript).
pub fn detect(session: &Session) -> Result<State> {
    if !tmux::has_session(&session.name)? {
        return Ok(State::Dead);
    }

    let backend = backend::resolve(&session.backend)?;
    let pane = tmux::capture_pane(&session.name)?;

    // capture-pane wins for TUI-interrupt gates: these don't reach the transcript until answered.
    if pane::contains_any(&pane, backend.plan_markers()) {
        let (plan_file, plan) = match plan::latest_plan_file(session.created)? {
            Some((path, content)) => (Some(path), content),
            None => (None, None),
        };
        return Ok(State::PlanReady { plan_file, plan });
    }
    // The one-time folder-trust gate blocks the session at startup before any input is accepted.
    if pane::contains_any(&pane, backend.trust_markers()) {
        return Ok(State::Blocked {
            gate: "trust".to_string(),
            prompt: first_marker(&pane, backend.trust_markers()),
            options: pane::parse_menu_options(&pane),
        });
    }
    if pane::contains_any(&pane, backend.permission_markers()) {
        return Ok(State::Blocked {
            gate: "permission".to_string(),
            prompt: first_marker(&pane, backend.permission_markers()),
            options: pane::parse_menu_options(&pane),
        });
    }

    // Otherwise trust the transcript.
    let events = if session.jsonl_path.exists() {
        jsonl::load(&session.jsonl_path)?
    } else {
        Vec::new()
    };
    Ok(map_jsonl(jsonl::classify(&events), session))
}

fn map_jsonl(state: jsonl::JsonlState, session: &Session) -> State {
    use jsonl::JsonlState;
    match state {
        JsonlState::AwaitingAnswer { question } => State::AwaitingAnswer { question },
        JsonlState::PlanReady { plan } => {
            let plan_file = plan::latest_plan_file(session.created)
                .ok()
                .flatten()
                .map(|(path, _)| path);
            State::PlanReady { plan_file, plan }
        }
        JsonlState::ToolPending { tools } => State::Working { tools },
        JsonlState::IdleDone { text } => State::IdleDone { text },
        JsonlState::Working => State::Working { tools: Vec::new() },
        // No assistant turn and no prompt yet → the session is up but hasn't produced a turn.
        JsonlState::Unknown => State::Spawning,
    }
}

/// The first permission marker present in the pane, as the human-readable prompt for `Blocked`.
fn first_marker(pane: &str, markers: &[&str]) -> String {
    markers
        .iter()
        .find(|m| pane.contains(**m))
        .map(|m| m.to_string())
        .unwrap_or_default()
}
