//! Session metadata sidecar + path derivation.
//!
//! csd tracks each driven session with a small JSON sidecar under the state dir so `ps`/`state`
//! can recover the session id, cwd and transcript path without re-deriving them. tmux remains the
//! source of truth for liveness; the sidecar is the source of truth for identity.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// Persisted identity of one driven session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    /// tmux session name (also the sidecar filename stem).
    pub name: String,
    /// Pinned `--session-id` UUID; makes the transcript path deterministic.
    pub session_id: String,
    /// Working directory the agent was spawned in.
    pub cwd: String,
    /// Backend that drives this session (`claude`, later `codex`).
    pub backend: String,
    /// Path to the session transcript JSONL.
    pub jsonl_path: PathBuf,
    /// `--permission-mode` the session was spawned with, if any.
    pub permission_mode: Option<String>,
    /// Backend CLI version probed at spawn (the binary is pinned for the session's lifetime, so
    /// `state`/`ps` can check marker drift without a per-poll subprocess). `None` when the probe
    /// failed or the sidecar predates this field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend_version: Option<String>,
    /// Unix epoch seconds at spawn — used to correlate plan files (which live in a global dir).
    pub created: u64,
}

impl Session {
    /// Where this session's sidecar lives on disk. Validates `name` first so a hostile value
    /// (path separators, `..`) can never escape the sessions directory on save/load/delete.
    pub fn sidecar_path(name: &str) -> Result<PathBuf> {
        validate_name(name)?;
        Ok(sessions_dir()?.join(format!("{name}.json")))
    }

    /// Write the sidecar (creating the state dir if needed).
    pub fn save(&self) -> Result<()> {
        let path = Session::sidecar_path(&self.name)?; // validates the name
        let dir = sessions_dir()?;
        fs::create_dir_all(&dir).map_err(|e| Error::io(&dir, e))?;
        let body = serde_json::to_string_pretty(self)?;
        fs::write(&path, body).map_err(|e| Error::io(&path, e))
    }

    /// Load a sidecar by session name.
    pub fn load(name: &str) -> Result<Session> {
        let path = Session::sidecar_path(name)?;
        let body = fs::read_to_string(&path).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => Error::NoSuchSession(name.to_string()),
            _ => Error::io(&path, e),
        })?;
        Ok(serde_json::from_str(&body)?)
    }

    /// Remove the sidecar (best-effort; missing file is not an error).
    pub fn delete(name: &str) -> Result<()> {
        let path = Session::sidecar_path(name)?;
        match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(Error::io(&path, e)),
        }
    }

    /// All tracked sessions, sorted by name. Sidecars that fail to parse are skipped.
    pub fn list() -> Result<Vec<Session>> {
        let dir = sessions_dir()?;
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut sessions = Vec::new();
        for entry in fs::read_dir(&dir).map_err(|e| Error::io(&dir, e))? {
            let entry = entry.map_err(|e| Error::io(&dir, e))?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            if let Ok(body) = fs::read_to_string(&path) {
                if let Ok(session) = serde_json::from_str::<Session>(&body) {
                    sessions.push(session);
                }
            }
        }
        sessions.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(sessions)
    }
}

/// Reject session names that aren't a single safe path segment. A name becomes both a tmux session
/// name and a sidecar filename, so it must contain only `[A-Za-z0-9._-]`, start with an alphanumeric
/// or underscore (no leading `-` that tmux could read as a flag), and never contain `..`.
pub fn validate_name(name: &str) -> Result<()> {
    let starts_ok = name
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_');
    let chars_ok = name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'));
    if name.is_empty() || !starts_ok || !chars_ok || name.contains("..") {
        return Err(Error::InvalidSessionName(name.to_string()));
    }
    Ok(())
}

/// `~/.local/state/csd/sessions`.
fn sessions_dir() -> Result<PathBuf> {
    let base = dirs::state_dir()
        .or_else(|| dirs::home_dir().map(|h| h.join(".local/state")))
        .ok_or(Error::NoDir { what: "state" })?;
    Ok(base.join("csd").join("sessions"))
}

/// Current Unix epoch in seconds. (Clock-before-epoch is impossible in practice → 0 fallback.)
pub fn now_epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Slugify a cwd the way `claude` names its project transcript dir: every character that is not an
/// ASCII alphanumeric becomes `-` (claude's rule is `replace(/[^a-zA-Z0-9]/g, '-')`, applied
/// per-character with no collapsing). This matters for any cwd containing `_`, `.`, or a space —
/// e.g. the life-assistant bot home `~/bots/Marshall Ku` — where the previous `/`-only replacement
/// pointed at a non-existent transcript path, so the hybrid detector never saw the turn complete and
/// `run` hung until its timeout.
///
/// `/tmp/claude-itest1` → `-tmp-claude-itest1` (PoC §2.1).
/// `/home/me/bots/Marshall Ku` → `-home-me-bots-Marshall-Ku`.
pub fn cwd_slug(cwd: &str) -> String {
    cwd.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// Deterministic transcript path: `~/.claude/projects/<cwd-slug>/<session-id>.jsonl`.
pub fn jsonl_path(cwd: &str, session_id: &str) -> Result<PathBuf> {
    let home = dirs::home_dir().ok_or(Error::NoDir { what: "home" })?;
    Ok(home
        .join(".claude")
        .join("projects")
        .join(cwd_slug(cwd))
        .join(format!("{session_id}.jsonl")))
}

/// `~/.claude/plans` — where `claude` writes plan files in plan mode (global, not per-session).
pub fn plans_dir() -> Result<PathBuf> {
    let home = dirs::home_dir().ok_or(Error::NoDir { what: "home" })?;
    Ok(home.join(".claude").join("plans"))
}

/// Modification time of `path` as Unix epoch seconds, if available.
pub fn mtime_epoch(path: &Path) -> Option<u64> {
    fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_replaces_every_slash() {
        assert_eq!(cwd_slug("/tmp/claude-itest1"), "-tmp-claude-itest1");
        assert_eq!(cwd_slug("/home/marshall/dev/csd"), "-home-marshall-dev-csd");
    }

    #[test]
    fn slug_replaces_every_non_alphanumeric() {
        // Matches claude's `replace(/[^a-zA-Z0-9]/g, '-')`: underscores, dots, and spaces all
        // become `-`, so the transcript path resolves for paths like the bot home with a space.
        assert_eq!(
            cwd_slug("/tmp/Test_Case_E2E/001/tester"),
            "-tmp-Test-Case-E2E-001-tester"
        );
        assert_eq!(cwd_slug("/home/me/bots/Marshall Ku"), "-home-me-bots-Marshall-Ku");
        assert_eq!(cwd_slug("/tmp/a.b"), "-tmp-a-b");
    }

    #[test]
    fn accepts_normal_names() {
        for name in ["csd-csd-abc123", "agent_1", "Foo.bar-2"] {
            assert!(validate_name(name).is_ok(), "{name} should be valid");
        }
    }

    #[test]
    fn sidecar_without_backend_version_loads_and_none_is_not_serialized() {
        let old = r#"{
            "name": "csd-x-abc",
            "session_id": "00000000-0000-0000-0000-000000000000",
            "cwd": "/tmp/x",
            "backend": "claude",
            "jsonl_path": "/tmp/x.jsonl",
            "permission_mode": null,
            "created": 1
        }"#;
        let session: Session = serde_json::from_str(old).unwrap();
        assert_eq!(session.backend_version, None);
        let body = serde_json::to_string(&session).unwrap();
        assert!(!body.contains("backend_version"));
    }

    #[test]
    fn rejects_path_traversal_and_separators() {
        for name in ["../x", "..", "a/b", "/etc/passwd", "-rf", "", "a..b", "foo/../bar"] {
            assert!(validate_name(name).is_err(), "{name} should be rejected");
        }
    }
}
