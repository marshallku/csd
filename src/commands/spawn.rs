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
    /// Convenience for `--permission-mode acceptEdits`.
    pub auto_accept: bool,
    /// Convenience for `--permission-mode bypassPermissions` (skip all permission checks).
    pub bypass_permissions: bool,
    /// claude's `--dangerously-skip-permissions` (skip permission checks); also implies `trust`,
    /// since that flag does NOT clear the separate folder-trust gate.
    pub yolo: bool,
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
            bypass_permissions: false,
            yolo: false,
            trust: false,
            width: DEFAULT_WIDTH,
            height: DEFAULT_HEIGHT,
        }
    }
}

/// Spawn output: the persisted session identity plus the (non-persisted) marker drift warning.
#[derive(Debug, serde::Serialize)]
pub struct SpawnResult {
    #[serde(flatten)]
    pub session: Session,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub marker_warning: Option<String>,
}

pub fn run(args: SpawnArgs) -> Result<SpawnResult> {
    let backend = backend::resolve(&args.backend)?;
    let (permission_mode, dangerous) = resolve_posture(&args)?;

    let cwd = resolve_cwd(args.cwd)?;
    let session_id = match args.session_id {
        Some(id) => validate_uuid(id)?,
        None => Uuid::new_v4().to_string(),
    };
    let name = args.name.unwrap_or_else(|| default_name(&cwd, &session_id));
    // Reject hostile names before they become a tmux session name or a sidecar filename.
    session::validate_name(&name)?;

    let backend_version = backend.installed_version();
    let marker_warning = backend::marker_warning(backend.as_ref(), backend_version.as_deref());

    // Assemble the full identity up front so post-spawn steps take a single value, and so a bad
    // jsonl path fails before we ever start a session.
    let session = Session {
        jsonl_path: session::jsonl_path(&cwd, &session_id)?,
        name,
        backend: backend.name().to_string(),
        cwd,
        permission_mode: permission_mode.clone(),
        backend_version,
        created: session::now_epoch(),
        session_id: session_id.clone(),
    };

    let command = backend.spawn_command(&SpawnOpts {
        session_id,
        permission_mode,
        dangerous,
    });
    tmux::new_session(&session.name, args.width, args.height, &session.cwd, &command)?;

    // Anything that fails after the session is live must not leave an orphaned, untracked session.
    // `--yolo` is the zero-friction posture, so it also clears the (separate) folder-trust gate —
    // `--dangerously-skip-permissions` skips permission checks but NOT the trust prompt.
    if let Err(e) = post_spawn(&session, args.trust || args.yolo, backend.as_ref()) {
        let _ = tmux::kill_session(&session.name);
        return Err(e);
    }
    Ok(SpawnResult {
        session,
        marker_warning,
    })
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

/// Resolve the permission posture into `(permission_mode, dangerous)`. The four posture flags are
/// mutually exclusive; `--yolo` uses claude's standalone skip flag, the rest map to a mode.
fn resolve_posture(args: &SpawnArgs) -> Result<(Option<String>, bool)> {
    let specified = [
        args.permission_mode.is_some(),
        args.auto_accept,
        args.bypass_permissions,
        args.yolo,
    ]
    .iter()
    .filter(|&&set| set)
    .count();
    if specified > 1 {
        return Err(Error::ConflictingPermissionFlags);
    }

    if args.yolo {
        return Ok((None, true));
    }
    if args.bypass_permissions {
        return Ok((Some("bypassPermissions".to_string()), false));
    }
    if args.auto_accept {
        return Ok((Some("acceptEdits".to_string()), false));
    }
    if let Some(mode) = &args.permission_mode {
        if !CLAUDE_PERMISSION_MODES.contains(&mode.as_str()) {
            return Err(Error::InvalidPermissionMode(
                mode.clone(),
                format!("{CLAUDE_PERMISSION_MODES:?}"),
            ));
        }
        return Ok((Some(mode.clone()), false));
    }
    Ok((None, false))
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

#[cfg(test)]
mod tests {
    use super::*;

    fn args(f: impl FnOnce(&mut SpawnArgs)) -> SpawnArgs {
        let mut a = SpawnArgs::default();
        f(&mut a);
        a
    }

    #[test]
    fn posture_flags_map_to_modes() {
        assert_eq!(resolve_posture(&SpawnArgs::default()).unwrap(), (None, false));
        assert_eq!(
            resolve_posture(&args(|a| a.auto_accept = true)).unwrap(),
            (Some("acceptEdits".into()), false)
        );
        assert_eq!(
            resolve_posture(&args(|a| a.bypass_permissions = true)).unwrap(),
            (Some("bypassPermissions".into()), false)
        );
        assert_eq!(resolve_posture(&args(|a| a.yolo = true)).unwrap(), (None, true));
        assert_eq!(
            resolve_posture(&args(|a| a.permission_mode = Some("plan".into()))).unwrap(),
            (Some("plan".into()), false)
        );
    }

    #[test]
    fn rejects_conflicting_and_invalid_postures() {
        assert!(resolve_posture(&args(|a| {
            a.yolo = true;
            a.bypass_permissions = true;
        }))
        .is_err());
        assert!(resolve_posture(&args(|a| a.permission_mode = Some("nope".into()))).is_err());
    }
}
