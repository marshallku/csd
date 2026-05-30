//! Backend abstraction. `csd` drives `claude` today and is designed to drive `codex` later
//! (decision #49: keep the dispatch backend swappable so a forced move off the interactive REPL is
//! a config flip, not a rewrite). Everything release-dependent — the spawn command and the
//! capture-pane gate markers — lives behind this trait in ONE place (PoC gotcha #4).

use crate::error::{Error, Result};

/// Options needed to build a spawn command. cwd is handled by tmux (`-c`), not the command itself.
#[derive(Debug, Clone)]
pub struct SpawnOpts {
    pub session_id: String,
    pub permission_mode: Option<String>,
}

/// A driveable agent CLI.
pub trait Backend {
    /// Stable identifier persisted in the sidecar (`claude`, `codex`).
    fn name(&self) -> &'static str;

    /// The shell command tmux execs in the new session (PoC §2.1 for claude).
    fn spawn_command(&self, opts: &SpawnOpts) -> String;

    /// capture-pane substrings that indicate a plan-approval gate is on screen (PoC §3.3).
    fn plan_markers(&self) -> &'static [&'static str];

    /// capture-pane substrings that indicate a tool-permission gate is on screen.
    ///
    /// NOTE: only the plan gate was exercised in the PoC; these are best-effort and MUST be
    /// re-verified against a live permission prompt on first run, then pinned here.
    fn permission_markers(&self) -> &'static [&'static str];

    /// capture-pane substrings for the one-time "trust this folder?" startup gate that `claude`
    /// shows on a directory it has not seen before. Blocks the session until answered.
    fn trust_markers(&self) -> &'static [&'static str];
}

/// Permission modes accepted by `claude --permission-mode` (PoC §2.1).
pub const CLAUDE_PERMISSION_MODES: &[&str] =
    &["plan", "acceptEdits", "auto", "bypassPermissions", "default", "dontAsk"];

pub struct Claude;

impl Backend for Claude {
    fn name(&self) -> &'static str {
        "claude"
    }

    fn spawn_command(&self, opts: &SpawnOpts) -> String {
        // Strip the nested-CLI markers so the child doesn't think it runs inside another Claude
        // session, then pin the session id so the transcript path is known up front.
        let mut cmd = format!(
            "env -u CLAUDECODE -u CLAUDE_CODE_ENTRYPOINT claude --session-id {}",
            opts.session_id
        );
        if let Some(mode) = &opts.permission_mode {
            cmd.push_str(" --permission-mode ");
            cmd.push_str(mode);
        }
        cmd
    }

    fn plan_markers(&self) -> &'static [&'static str] {
        &["Here is Claude's plan", "Would you like to proceed?"]
    }

    fn permission_markers(&self) -> &'static [&'static str] {
        // Unverified — see trait doc. Distinct from the plan gate's "Would you like to proceed?".
        &[
            "Do you want to proceed?",
            "Do you want to make this edit?",
            "Allow this tool",
        ]
    }

    fn trust_markers(&self) -> &'static [&'static str] {
        &[
            "Is this a project you created or one you trust?",
            "Yes, I trust this folder",
        ]
    }
}

/// Resolve a backend by name.
pub fn resolve(name: &str) -> Result<Box<dyn Backend>> {
    match name {
        "claude" => Ok(Box::new(Claude)),
        other => Err(Error::UnknownBackend(other.to_string())),
    }
}
