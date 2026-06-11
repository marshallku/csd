//! `csd ps` — list tracked sessions with liveness + live state, mirroring `tmx agents --json`.

use serde::Serialize;

use crate::backend;
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub marker_warning: Option<String>,
}

pub fn run() -> Result<PsResult> {
    let mut entries = Vec::new();
    for session in Session::list()? {
        let alive = tmux::has_session(&session.name)?;
        let state = if alive { detect::detect(&session)? } else { State::Dead };
        let marker_warning = backend::resolve(&session.backend)
            .ok()
            .and_then(|b| backend::marker_warning(b.as_ref(), session.backend_version.as_deref()));
        entries.push(PsEntry {
            session,
            alive,
            state,
            marker_warning,
        });
    }
    Ok(PsResult { sessions: entries })
}
