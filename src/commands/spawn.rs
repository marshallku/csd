//! `csd spawn` — start a detached interactive agent on the subscription seat (PoC §2.1).

use std::path::PathBuf;
use std::thread::sleep;

use uuid::Uuid;

use crate::backend::{self, Backend, SpawnOpts, CLAUDE_PERMISSION_MODES};
use crate::commands::{APPROVE_DELAY, DEFAULT_HEIGHT, DEFAULT_WIDTH, TRUST_POLL_ATTEMPTS, TRUST_POLL_INTERVAL};
use crate::detect::pane;
use crate::error::{Error, Result};
use crate::session::{self, Session};
use crate::tmux;

/// Parsed `spawn` inputs (mirrors the CLI; cwd/session_id default lazily).
#[derive(Debug, Clone)]
pub struct SpawnArgs {
    pub cwd: Option<PathBuf>,
    pub session_id: Option<String>,
    pub permission_mode: Option<String>,
    pub name: Option<String>,
    pub backend: String,
    pub auto_accept: bool,
    /// Auto-clear the one-time folder-trust gate so the session becomes immediately driveable.
    pub trust: bool,
    pub width: u16,
    pub height: u16,
}

impl Default for SpawnArgs {
    fn default() -> Self {
        SpawnArgs {
            cwd: None,
            session_id: None,
            permission_mode: None,
            name: None,
            backend: "claude".to_string(),
            auto_accept: false,
            trust: false,
            width: DEFAULT_WIDTH,
            height: DEFAULT_HEIGHT,
        }
    }
}

pub fn run(args: SpawnArgs) -> Result<Session> {
    let backend = backend::resolve(&args.backend)?;

    let cwd = resolve_cwd(args.cwd)?;
    let session_id = match args.session_id {
        Some(id) => validate_uuid(id)?,
        None => Uuid::new_v4().to_string(),
    };
    let permission_mode = resolve_permission_mode(args.permission_mode, args.auto_accept)?;
    let name = args.name.unwrap_or_else(|| default_name(&cwd, &session_id));
    // Reject hostile names before they become a tmux session name or a sidecar filename.
    session::validate_name(&name)?;

    // Assemble the full identity up front so post-spawn steps take a single value, and so a bad
    // jsonl path fails before we ever start a session.
    let session = Session {
        jsonl_path: session::jsonl_path(&cwd, &session_id)?,
        name,
        backend: backend.name().to_string(),
        cwd,
        permission_mode: permission_mode.clone(),
        created: session::now_epoch(),
        session_id: session_id.clone(),
    };

    let command = backend.spawn_command(&SpawnOpts {
        session_id,
        permission_mode,
    });
    tmux::new_session(&session.name, args.width, args.height, &session.cwd, &command)?;

    // Anything that fails after the session is live must not leave an orphaned, untracked session.
    if let Err(e) = post_spawn(&session, args.trust, backend.as_ref()) {
        let _ = tmux::kill_session(&session.name);
        return Err(e);
    }
    Ok(session)
}

/// Clear the trust gate (if requested) and persist the sidecar, now that the session is live.
fn post_spawn(session: &Session, trust: bool, backend: &dyn Backend) -> Result<()> {
    if trust {
        clear_trust_gate(&session.name, backend)?;
    }
    session.save()
}

/// Default cwd is the current dir; make it absolute so the transcript slug matches `claude`'s.
fn resolve_cwd(cwd: Option<PathBuf>) -> Result<String> {
    let path = match cwd {
        Some(p) => p,
        None => std::env::current_dir().map_err(|e| Error::io(".", e))?,
    };
    let abs = std::fs::canonicalize(&path).unwrap_or(path);
    Ok(abs.to_string_lossy().into_owned())
}

/// Explicit `--permission-mode` wins; otherwise `--auto-accept` maps to `acceptEdits`.
fn resolve_permission_mode(explicit: Option<String>, auto_accept: bool) -> Result<Option<String>> {
    if let Some(mode) = explicit {
        if !CLAUDE_PERMISSION_MODES.contains(&mode.as_str()) {
            return Err(Error::InvalidPermissionMode(
                mode,
                format!("{CLAUDE_PERMISSION_MODES:?}"),
            ));
        }
        return Ok(Some(mode));
    }
    if auto_accept {
        return Ok(Some("acceptEdits".to_string()));
    }
    Ok(None)
}

fn validate_uuid(id: String) -> Result<String> {
    Uuid::parse_str(&id).map_err(|e| Error::InvalidSessionId(id.clone(), e.to_string()))?;
    Ok(id)
}

/// Watch for the one-time "trust this folder?" startup gate and answer it (option 1 = trust).
///
/// Returns `true` if a gate was found and cleared. On a folder `claude` already trusts the gate
/// never appears; we break as soon as the pane renders any non-trust content so trusted dirs don't
/// pay the full window.
fn clear_trust_gate(name: &str, backend: &dyn Backend) -> Result<bool> {
    // Give the gate time to render before treating other content as "already trusted" — the first
    // frames can show the workspace header a beat before the trust question appears.
    const MIN_POLLS_BEFORE_READY: u32 = 4;
    for attempt in 0..TRUST_POLL_ATTEMPTS {
        sleep(TRUST_POLL_INTERVAL);
        let captured = tmux::capture_pane(name)?;
        if pane::contains_any(&captured, backend.trust_markers()) {
            tmux::send_literal(name, "1")?;
            sleep(APPROVE_DELAY);
            tmux::send_key(name, "Enter")?;
            return Ok(true);
        }
        if attempt >= MIN_POLLS_BEFORE_READY && !captured.trim().is_empty() {
            return Ok(false);
        }
    }
    Ok(false)
}

/// `csd-<cwd-basename>-<first 8 of uuid>` — readable and collision-resistant.
fn default_name(cwd: &str, session_id: &str) -> String {
    let base = cwd.rsplit('/').find(|s| !s.is_empty()).unwrap_or("agent");
    let short = &session_id[..session_id.len().min(8)];
    format!("csd-{base}-{short}")
}
