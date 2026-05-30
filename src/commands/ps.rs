//! `csd ps` — list tracked sessions with liveness + live state, mirroring `tmx agents --json`.

use serde::Serialize;

use crate::detect::{self, State};
use crate::error::Result;
use crate::session::Session;
use crate::tmux;

#[derive(Debug, Serialize)]
pub struct PsResult {
    pub sessions: Vec<PsEntry>,
}

#[derive(Debug, Serialize)]
pub struct PsEntry {
    #[serde(flatten)]
    pub session: Session,
    pub alive: bool,
    pub state: State,
}

pub fn run() -> Result<PsResult> {
    let mut entries = Vec::new();
    for session in Session::list()? {
        let alive = tmux::has_session(&session.name)?;
        let state = if alive { detect::detect(&session)? } else { State::Dead };
        entries.push(PsEntry { session, alive, state });
    }
    Ok(PsResult { sessions: entries })
}
