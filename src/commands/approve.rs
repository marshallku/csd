//! `csd approve` — answer a plan-approval or tool-permission gate by selecting a menu option
//! (PoC §2.3). Default option `1` = "Yes, and use auto mode" for the plan gate.

use std::thread::sleep;

use serde::Serialize;

use crate::commands::APPROVE_DELAY;
use crate::error::Result;
use crate::tmux;

#[derive(Debug, Serialize)]
pub struct ApproveResult {
    pub ok: bool,
    pub option: u32,
}

pub fn run(session_name: &str, option: u32) -> Result<ApproveResult> {
    crate::session::validate_name(session_name)?;
    tmux::send_literal(session_name, &option.to_string())?;
    sleep(APPROVE_DELAY);
    tmux::send_key(session_name, "Enter")?;
    Ok(ApproveResult { ok: true, option })
}
