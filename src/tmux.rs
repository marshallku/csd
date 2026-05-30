//! Thin wrappers over the `tmux` CLI. Every call shells out to the user's tmux server.
//!
//! These are deliberately dumb: build argv, run, map a non-zero exit to [`Error::Tmux`]. Higher
//! layers ([`crate::commands`]) own the orchestration (readiness retries, sidecar bookkeeping).

use std::process::Command;

use crate::error::{Error, Result};

/// Run a tmux subcommand, returning captured stdout on success.
fn run(args: &[&str]) -> Result<String> {
    let output = Command::new("tmux").args(args).output().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            Error::TmuxMissing
        } else {
            Error::Tmux(e.to_string())
        }
    })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(Error::Tmux(format!("`tmux {}` → {}", args.join(" "), stderr)));
    }

    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Spawn a detached session named `name`, sized `width`x`height`, with cwd `cwd`, running `command`
/// (a single shell string the login shell will exec). Returns [`Error::SessionExists`] if taken.
pub fn new_session(name: &str, width: u16, height: u16, cwd: &str, command: &str) -> Result<()> {
    if has_session(name)? {
        return Err(Error::SessionExists(name.to_string()));
    }
    run(&[
        "new-session",
        "-d",
        "-s",
        name,
        "-x",
        &width.to_string(),
        "-y",
        &height.to_string(),
        "-c",
        cwd,
        command,
    ])
    .map(|_| ())
}

/// Inject literal text into the session's active pane (no key interpretation).
///
/// `send-keys -l` is the only injection that landed in the PoC; `paste-buffer` did not.
pub fn send_literal(name: &str, text: &str) -> Result<()> {
    require_session(name)?;
    run(&["send-keys", "-t", name, "-l", text]).map(|_| ())
}

/// Send a named key (e.g. `Enter`, `Escape`, `C-u`) to the session's active pane.
pub fn send_key(name: &str, key: &str) -> Result<()> {
    require_session(name)?;
    run(&["send-keys", "-t", name, key]).map(|_| ())
}

/// Capture the visible contents of the session's active pane as plain text.
pub fn capture_pane(name: &str) -> Result<String> {
    require_session(name)?;
    run(&["capture-pane", "-p", "-t", name])
}

/// Whether a session with this exact name exists on the server.
pub fn has_session(name: &str) -> Result<bool> {
    let output = Command::new("tmux")
        .args(["has-session", "-t", name])
        .output()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                Error::TmuxMissing
            } else {
                Error::Tmux(e.to_string())
            }
        })?;
    // has-session exits non-zero when absent and also when there is no server at all; both mean "no".
    Ok(output.status.success())
}

/// Kill a session. Returns [`Error::NoSuchSession`] if it was not running.
pub fn kill_session(name: &str) -> Result<()> {
    require_session(name)?;
    run(&["kill-session", "-t", name]).map(|_| ())
}

fn require_session(name: &str) -> Result<()> {
    if !has_session(name)? {
        return Err(Error::NoSuchSession(name.to_string()));
    }
    Ok(())
}
