//! `csd run` — the synchronous one-shot wrapper (`claude -p` parity on the subscription seat):
//! spawn (or reuse/resume) a session, inject the prompt, block until the turn reaches a terminal
//! state, and report it. Non-`done` outcomes deliberately keep the session alive so the caller can
//! answer a question or inspect a gate with `csd run --session` / `csd state`.

use std::path::PathBuf;
use std::thread::sleep;
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::commands::{approve, send, spawn, DEFAULT_SEND_RETRIES, RUN_POLL_INTERVAL};
use crate::detect::{self, jsonl, State};
use crate::error::{Error, Result};
use crate::session::Session;
use crate::tmux;

#[derive(Debug, Clone)]
pub struct RunArgs {
    pub cwd: Option<PathBuf>,
    pub session_id: Option<String>,
    /// Resume this transcript (session id) in a fresh tmux session.
    pub resume: Option<String>,
    /// Reuse a live, caller-owned csd session instead of spawning.
    pub session: Option<String>,
    pub permission_mode: Option<String>,
    pub auto_accept: bool,
    pub bypass_permissions: bool,
    pub yolo: bool,
    /// Keep the session alive after a successful run (enables fast `--session` follow-ups).
    pub keep: bool,
    /// Seconds to wait for a terminal state after the prompt is submitted; 0 = no limit.
    pub timeout: u64,
    /// Auto-approve plan gates (option 1). Permission gates are NEVER auto-approved here —
    /// unattended tool permissions go through claude's own postures, not TUI menu injection.
    pub approve_plan: bool,
    pub prompt: String,
}

/// Terminal outcome of one `run`. `session` is present whenever the tmux session was kept alive
/// (always on non-`done` outcomes; on `done` only with `--keep`).
#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum RunOutcome {
    /// The assistant finished the turn; `text` is the final answer.
    Done {
        text: String,
        session_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        session: Option<String>,
    },
    /// The assistant asked a clarifying question — answer with `csd run --session <name> "..."`.
    AwaitingAnswer {
        question: String,
        session: String,
        session_id: String,
    },
    /// A plan/permission/trust gate is up — inspect with `csd state`, answer with `csd approve`.
    Gated {
        gate: String,
        prompt: String,
        options: Vec<String>,
        session: String,
        session_id: String,
    },
    /// No terminal state within `--timeout`; the turn may still be working.
    TimedOut {
        waited_secs: u64,
        session: String,
        session_id: String,
    },
}

impl RunOutcome {
    /// Process exit code contract: 0 done · 2 timeout · 3 awaiting_answer · 4 gated (1 = error).
    pub fn exit_code(&self) -> i32 {
        match self {
            RunOutcome::Done { .. } => 0,
            RunOutcome::TimedOut { .. } => 2,
            RunOutcome::AwaitingAnswer { .. } => 3,
            RunOutcome::Gated { .. } => 4,
        }
    }
}

pub fn run(args: RunArgs) -> Result<RunOutcome> {
    let (session, owned) = resolve_session(&args)?;

    let outcome = drive(&session, &args);

    // Ownership contract: sessions `run` spawned are cleaned up on errors and (without --keep) on
    // success; non-done outcomes keep the session on purpose — the question must stay answerable
    // and in-flight work must not be destroyed. Caller-owned (`--session`) sessions are never
    // killed here, on any path.
    match outcome {
        Err(e) => {
            if owned {
                let _ = super::kill::run(&session.name);
            }
            Err(e)
        }
        Ok(RunOutcome::Done { text, session_id, .. }) => {
            let kept = !owned || args.keep;
            if !kept {
                super::kill::run(&session.name)?;
            }
            Ok(RunOutcome::Done {
                text,
                session_id,
                session: kept.then(|| session.name.clone()),
            })
        }
        Ok(other) => Ok(other),
    }
}

/// Resolve the target session; the bool is whether `run` owns its lifecycle (it spawned it).
fn resolve_session(args: &RunArgs) -> Result<(Session, bool)> {
    if let Some(name) = &args.session {
        let session = Session::load(name)?;
        if !tmux::has_session(&session.name)? {
            return Err(Error::SessionNotAlive(name.clone()));
        }
        return Ok((session, false));
    }
    let result = spawn::run(spawn::SpawnArgs {
        cwd: args.cwd.clone(),
        session_id: args.resume.clone().or_else(|| args.session_id.clone()),
        permission_mode: args.permission_mode.clone(),
        backend: "claude".to_string(),
        auto_accept: args.auto_accept,
        bypass_permissions: args.bypass_permissions,
        yolo: args.yolo,
        // One-shot semantics: never wedge on the first-use folder-trust prompt.
        trust: true,
        resume: args.resume.is_some(),
        ..Default::default()
    })?;
    Ok((result.session, true))
}

/// Send the prompt and block until a terminal state. Stale-state safety: `idle_done` and
/// `awaiting_answer` come from the transcript, which on a reused/resumed session still ends with
/// the PREVIOUS turn's terminal state — so both are trusted only once the turn-event count has
/// grown past the pre-send watermark (our own prompt's user event is the first thing to cross it,
/// after which the classifier's later-user rule reports Working until a fresh assistant turn).
/// Pane-detected gates need no watermark: they are on screen now.
fn drive(session: &Session, args: &RunArgs) -> Result<RunOutcome> {
    // A gate already on screen would swallow the prompt keystrokes into a menu. Surface it
    // instead of typing — a leftover gate belongs to the previous turn, so even --approve-plan
    // does not blindly answer it.
    match detect::detect(session)? {
        State::Dead => return Err(Error::SessionNotAlive(session.name.clone())),
        State::PlanReady { plan, .. } => return Ok(gated("plan", plan.unwrap_or_default(), Vec::new(), session)),
        State::Blocked { gate, prompt, options } => return Ok(gated(&gate, prompt, options, session)),
        _ => {}
    }

    let watermark = turn_count(session)?;
    send::run(send::SendArgs {
        session: session.name.clone(),
        prompt: args.prompt.clone(),
        submit: true,
        retries: DEFAULT_SEND_RETRIES,
    })?;

    let started = Instant::now();
    // Approve a plan gate once per sighting: re-sending "1 Enter" after the gate cleared would
    // submit a stray "1" as a user message. Reset when the gate leaves the screen.
    let mut plan_gate_active = false;
    loop {
        sleep(RUN_POLL_INTERVAL);
        let state = detect::detect(session)?;
        if !matches!(state, State::PlanReady { .. }) {
            plan_gate_active = false;
        }
        match state {
            State::Dead => return Err(Error::SessionNotAlive(session.name.clone())),
            State::PlanReady { .. } if args.approve_plan && !plan_gate_active => {
                plan_gate_active = true;
                approve::run(&session.name, 1)?;
            }
            // Already approved; the gate is still clearing — keep waiting.
            State::PlanReady { .. } if args.approve_plan => {}
            State::PlanReady { plan, .. } => {
                return Ok(gated("plan", plan.unwrap_or_default(), Vec::new(), session));
            }
            State::Blocked { gate, prompt, options } => return Ok(gated(&gate, prompt, options, session)),
            State::AwaitingAnswer { question } if turn_count(session)? > watermark => {
                return Ok(RunOutcome::AwaitingAnswer {
                    question,
                    session: session.name.clone(),
                    session_id: session.session_id.clone(),
                });
            }
            State::IdleDone { text } if turn_count(session)? > watermark => {
                return Ok(RunOutcome::Done {
                    text,
                    session_id: session.session_id.clone(),
                    session: Some(session.name.clone()),
                });
            }
            _ => {}
        }
        if args.timeout != 0 && started.elapsed() >= Duration::from_secs(args.timeout) {
            return Ok(RunOutcome::TimedOut {
                waited_secs: started.elapsed().as_secs(),
                session: session.name.clone(),
                session_id: session.session_id.clone(),
            });
        }
    }
}

fn gated(gate: &str, prompt: String, options: Vec<String>, session: &Session) -> RunOutcome {
    RunOutcome::Gated {
        gate: gate.to_string(),
        prompt,
        options,
        session: session.name.clone(),
        session_id: session.session_id.clone(),
    }
}

/// Turn-relevant event count of the session transcript (0 when it doesn't exist yet).
fn turn_count(session: &Session) -> Result<usize> {
    if !session.jsonl_path.exists() {
        return Ok(0);
    }
    Ok(jsonl::turn_event_count(&jsonl::load(&session.jsonl_path)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn done() -> RunOutcome {
        RunOutcome::Done {
            text: "pong".to_string(),
            session_id: "id".to_string(),
            session: None,
        }
    }

    #[test]
    fn exit_codes_match_the_contract() {
        assert_eq!(done().exit_code(), 0);
        assert_eq!(
            RunOutcome::TimedOut {
                waited_secs: 1,
                session: "s".into(),
                session_id: "id".into(),
            }
            .exit_code(),
            2
        );
        assert_eq!(
            RunOutcome::AwaitingAnswer {
                question: "q".into(),
                session: "s".into(),
                session_id: "id".into(),
            }
            .exit_code(),
            3
        );
        assert_eq!(
            RunOutcome::Gated {
                gate: "permission".into(),
                prompt: "p".into(),
                options: Vec::new(),
                session: "s".into(),
                session_id: "id".into(),
            }
            .exit_code(),
            4
        );
    }

    #[test]
    fn done_serializes_with_status_tag_and_omits_killed_session() {
        let json = serde_json::to_value(done()).unwrap();
        assert_eq!(json["status"], "done");
        assert_eq!(json["text"], "pong");
        assert!(json.get("session").is_none());
    }

    #[test]
    fn awaiting_answer_serializes_session_for_follow_up() {
        let json = serde_json::to_value(RunOutcome::AwaitingAnswer {
            question: "which file?".into(),
            session: "csd-x-1".into(),
            session_id: "id".into(),
        })
        .unwrap();
        assert_eq!(json["status"], "awaiting_answer");
        assert_eq!(json["session"], "csd-x-1");
    }
}
