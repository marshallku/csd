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
    /// Pass claude's standalone `--dangerously-skip-permissions` (the `--yolo` posture).
    pub dangerous: bool,
    /// Resume the transcript identified by `session_id` (`--resume <id>`) instead of starting a
    /// fresh session pinned to it (`--session-id <id>`). Verified live on v2.1.173: resume
    /// appends to the SAME transcript JSONL, so the sidecar's `jsonl_path` stays valid.
    pub resume: bool,
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
    fn permission_markers(&self) -> &'static [&'static str];

    /// capture-pane substrings for the one-time "trust this folder?" startup gate that `claude`
    /// shows on a directory it has not seen before. Blocks the session until answered.
    fn trust_markers(&self) -> &'static [&'static str];

    /// Backend versions the gate markers were live-verified on. Exact pins only — patch releases
    /// can change the TUI wording the markers match against.
    fn verified_versions(&self) -> &'static [&'static str];

    /// Version of the backend CLI currently on PATH, or `None` if it can't be determined. Probed
    /// once at spawn (the binary is pinned for the session's lifetime) — note this is the CLI
    /// `csd` resolves, which tmux normally execs too, but a divergent tmux environment could
    /// technically run a different copy.
    fn installed_version(&self) -> Option<String>;
}

/// Warning to attach to spawn/state/ps output when the session's backend version is not in the
/// marker-verified set, so a backend upgrade turns gate-detection drift from a silent stall into
/// a machine-readable signal. `CSD_VERIFIED_VERSIONS` (comma-separated) extends the built-in set
/// without a csd release once the user re-verifies the markers by hand.
pub fn marker_warning(backend: &dyn Backend, installed: Option<&str>) -> Option<String> {
    let extra = std::env::var("CSD_VERIFIED_VERSIONS").unwrap_or_default();
    marker_warning_with(backend.name(), backend.verified_versions(), installed, &extra)
}

fn marker_warning_with(backend_name: &str, verified: &[&str], installed: Option<&str>, extra: &str) -> Option<String> {
    let Some(installed) = installed else {
        return Some(format!(
            "could not determine the installed {backend_name} version; gate markers were \
             verified on {verified:?} and may not match — gate detection \
             (plan/permission/trust) can silently miss prompts"
        ));
    };
    let extra_match = extra.split(',').map(str::trim).any(|v| !v.is_empty() && v == installed);
    if verified.contains(&installed) || extra_match {
        return None;
    }
    Some(format!(
        "{backend_name} {installed} is not marker-verified (verified: {verified:?}); gate \
         detection (plan/permission/trust) may silently miss prompts. If gates still work on \
         {installed}, set CSD_VERIFIED_VERSIONS={installed} to silence this warning"
    ))
}

/// First whitespace token of `<cli> --version` stdout, if it looks like a version
/// (`"2.1.173 (Claude Code)"` → `"2.1.173"`).
fn parse_version_output(stdout: &str) -> Option<String> {
    let token = stdout.split_whitespace().next()?;
    token
        .starts_with(|c: char| c.is_ascii_digit())
        .then(|| token.to_string())
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
        // session, then pin (or resume) the session id so the transcript path is known up front.
        let id_flag = if opts.resume { "--resume" } else { "--session-id" };
        let mut cmd = format!(
            "env -u CLAUDECODE -u CLAUDE_CODE_ENTRYPOINT claude {id_flag} {}",
            opts.session_id
        );
        if let Some(mode) = &opts.permission_mode {
            cmd.push_str(" --permission-mode ");
            cmd.push_str(mode);
        }
        if opts.dangerous {
            cmd.push_str(" --dangerously-skip-permissions");
        }
        cmd
    }

    fn plan_markers(&self) -> &'static [&'static str] {
        &["Here is Claude's plan", "Would you like to proceed?"]
    }

    fn permission_markers(&self) -> &'static [&'static str] {
        // "Do you want to proceed?" verified live on v2.1.158 (Bash/tool gate) — distinct from the
        // plan gate's "Would you like to proceed?". The edit-gate variant is the historical wording,
        // kept as a fallback (couldn't be exercised here — Write/Edit were allowlisted).
        &["Do you want to proceed?", "Do you want to make this edit?"]
    }

    fn trust_markers(&self) -> &'static [&'static str] {
        &[
            "Is this a project you created or one you trust?",
            "Yes, I trust this folder",
        ]
    }

    fn verified_versions(&self) -> &'static [&'static str] {
        // Extend only after a live `scripts/e2e.sh` run proves all three gates on that release.
        &["2.1.158", "2.1.173", "2.1.174"]
    }

    fn installed_version(&self) -> Option<String> {
        let output = std::process::Command::new("claude").arg("--version").output().ok()?;
        if !output.status.success() {
            return None;
        }
        parse_version_output(&String::from_utf8_lossy(&output.stdout))
    }
}

/// Resolve a backend by name.
pub fn resolve(name: &str) -> Result<Box<dyn Backend>> {
    match name {
        "claude" => Ok(Box::new(Claude)),
        other => Err(Error::UnknownBackend(other.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_claude_version_output() {
        assert_eq!(
            parse_version_output("2.1.173 (Claude Code)\n"),
            Some("2.1.173".to_string())
        );
        assert_eq!(parse_version_output("2.1.158"), Some("2.1.158".to_string()));
        assert_eq!(parse_version_output("Claude Code 2.1.173"), None);
        assert_eq!(parse_version_output(""), None);
        assert_eq!(parse_version_output("   \n"), None);
    }

    #[test]
    fn spawn_command_pins_or_resumes_the_session_id() {
        let opts = SpawnOpts {
            session_id: "abc-123".to_string(),
            permission_mode: None,
            dangerous: false,
            resume: false,
        };
        let fresh = Claude.spawn_command(&opts);
        assert!(fresh.contains("--session-id abc-123"));
        assert!(!fresh.contains("--resume"));

        let resumed = Claude.spawn_command(&SpawnOpts { resume: true, ..opts });
        assert!(resumed.contains("--resume abc-123"));
        assert!(!resumed.contains("--session-id"));
    }

    #[test]
    fn no_warning_on_verified_version() {
        assert_eq!(marker_warning_with("claude", &["2.1.158"], Some("2.1.158"), ""), None);
    }

    #[test]
    fn warns_on_unverified_version() {
        let warning = marker_warning_with("claude", &["2.1.158"], Some("2.1.173"), "").unwrap();
        assert!(warning.contains("2.1.173"));
        assert!(warning.contains("2.1.158"));
        assert!(warning.contains("CSD_VERIFIED_VERSIONS=2.1.173"));
    }

    #[test]
    fn warns_on_unknown_version() {
        let warning = marker_warning_with("claude", &["2.1.158"], None, "").unwrap();
        assert!(warning.contains("could not determine"));
    }

    #[test]
    fn env_override_extends_verified_set() {
        // Entries are trimmed, so a human-typed "a, b" list matches.
        let extra = "2.1.170, 2.1.173";
        assert_eq!(
            marker_warning_with("claude", &["2.1.158"], Some("2.1.173"), extra),
            None
        );
        assert_eq!(
            marker_warning_with("claude", &["2.1.158"], Some("2.1.170"), extra),
            None
        );
        assert!(marker_warning_with("claude", &["2.1.158"], Some("2.1.171"), extra).is_some());
        // An empty entry never matches (e.g. trailing comma, or empty env var).
        assert!(marker_warning_with("claude", &["2.1.158"], Some(""), ",").is_some());
    }
}
