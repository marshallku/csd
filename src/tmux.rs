//! Thin wrappers over the `tmux` CLI. Every call shells out to the user's tmux server.
//!
//! These are deliberately dumb: build argv, run, map a non-zero exit to [`Error::Tmux`]. Higher
//! layers ([`crate::commands`]) own the orchestration (readiness retries, sidecar bookkeeping).

use std::io::Write;
use std::process::{Command, Stdio};

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

/// Run a tmux subcommand feeding `stdin_data` to its stdin (for `load-buffer -`). The data is
/// written from a dedicated thread so a prompt larger than the OS pipe buffer (~64 KiB — committee
/// briefings are bigger) can't deadlock against tmux draining stdin. The args are NOT echoed into
/// any error (the payload can be huge), unlike `run`.
fn run_with_stdin(args: &[&str], stdin_data: &str) -> Result<()> {
    let mut child = Command::new("tmux")
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                Error::TmuxMissing
            } else {
                Error::Tmux(e.to_string())
            }
        })?;

    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| Error::Tmux("no stdin handle".into()))?;
    let data = stdin_data.as_bytes().to_vec();
    let writer = std::thread::spawn(move || stdin.write_all(&data));

    let output = child.wait_with_output().map_err(|e| Error::Tmux(e.to_string()))?;
    let write_result = writer.join().map_err(|_| Error::Tmux("stdin writer panicked".into()))?;
    write_result.map_err(|e| Error::Tmux(format!("write stdin: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(Error::Tmux(format!("`tmux {}` → {}", args.join(" "), stderr)));
    }
    Ok(())
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
/// Fine for short snippets (menu digits, single-line answers). For a full prompt use
/// [`paste_text`] — `send-keys -l` passes the text as one argv argument, which tmux rejects once
/// it gets large (committee briefings are ~73 KiB), and types char-by-char which the TUI may treat
/// as fast keystrokes rather than a paste.
pub fn send_literal(name: &str, text: &str) -> Result<()> {
    require_session(name)?;
    run(&["send-keys", "-t", name, "-l", text]).map(|_| ())
}

/// Inject `text` into the active pane as a single bracketed paste, via `load-buffer` (stdin, so no
/// argv size limit) + `paste-buffer -p`. This is the robust path for full prompts of any size: the
/// TUI receives it as one paste (newlines stay literal instead of submitting, large input collapses
/// to a `[Pasted text …]` placeholder) rather than a stream of keystrokes. A per-session buffer name
/// avoids clobbering a concurrent session's paste buffer; `-d` deletes it after pasting.
pub fn paste_text(name: &str, text: &str) -> Result<()> {
    require_session(name)?;
    let buffer = format!("csd-send-{name}");
    run_with_stdin(&["load-buffer", "-b", &buffer, "-"], text)?;
    run(&["paste-buffer", "-t", name, "-b", &buffer, "-d", "-p"]).map(|_| ())
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
