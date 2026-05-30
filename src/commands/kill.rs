//! `csd kill` — stop a session and remove its sidecar. Idempotent: a session that is already gone
//! still gets its sidecar cleaned up.

use serde::Serialize;

use crate::error::{Error, Result};
use crate::session::Session;
use crate::tmux;

#[derive(Debug, Serialize)]
pub struct KillResult {
    pub ok: bool,
}

pub fn run(session_name: &str) -> Result<KillResult> {
    match tmux::kill_session(session_name) {
        Ok(()) | Err(Error::NoSuchSession(_)) => {}
        Err(e) => return Err(e),
    }
    Session::delete(session_name)?;
    Ok(KillResult { ok: true })
}
