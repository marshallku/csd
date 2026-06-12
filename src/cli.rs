//! Command-line surface (clap derive). Kept declarative; `main` maps these onto [`crate::commands`].

use std::path::PathBuf;

use clap::{Parser, Subcommand};

use crate::commands::{DEFAULT_RUN_TIMEOUT_SECS, DEFAULT_SEND_RETRIES};

#[derive(Debug, Parser)]
#[command(
    name = "csd",
    version,
    about = "Claude Session Driver — drive interactive Claude over tmux"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Spawn a detached interactive agent session on the subscription seat.
    Spawn {
        /// Working directory for the agent (default: current dir).
        #[arg(long)]
        cwd: Option<PathBuf>,
        /// Pin the session id (UUID). Default: a fresh v4 UUID.
        #[arg(long)]
        session_id: Option<String>,
        /// `claude --permission-mode` value (plan, acceptEdits, auto, bypassPermissions, default, dontAsk).
        #[arg(long)]
        permission_mode: Option<String>,
        /// tmux session name. Default: derived from cwd + session id.
        #[arg(long)]
        name: Option<String>,
        /// Driving backend.
        #[arg(long, default_value = "claude")]
        backend: String,
        /// Convenience for `--permission-mode acceptEdits`.
        #[arg(long)]
        auto_accept: bool,
        /// Convenience for `--permission-mode bypassPermissions` (skip all permission checks).
        #[arg(long)]
        bypass_permissions: bool,
        /// Zero-friction: claude's `--dangerously-skip-permissions` plus auto-clearing the folder-trust gate.
        #[arg(long)]
        yolo: bool,
        /// Auto-clear the one-time "trust this folder?" startup gate so the session is driveable.
        #[arg(long)]
        trust: bool,
    },

    /// Run a prompt synchronously (the `claude -p` shape): spawn or reuse a session, send the
    /// prompt, block until the turn finishes, print the answer.
    ///
    /// Exit codes: 0 done · 1 error · 2 timeout · 3 awaiting_answer · 4 gated. Non-done outcomes
    /// keep the session alive and report its name so the turn stays answerable/inspectable.
    Run {
        /// Working directory for a freshly spawned agent (default: current dir).
        #[arg(long, conflicts_with = "session")]
        cwd: Option<PathBuf>,
        /// Pin the new session's id (UUID) — makes the run resumable later via --resume.
        #[arg(long, conflicts_with_all = ["resume", "session"])]
        session_id: Option<String>,
        /// Resume a previous run's transcript by session id (requires the original --cwd).
        #[arg(long, conflicts_with = "session")]
        resume: Option<String>,
        /// Reuse a LIVE csd session (e.g. kept with --keep, or reported by a non-done outcome).
        #[arg(long)]
        session: Option<String>,
        /// `claude --permission-mode` value (plan, acceptEdits, auto, bypassPermissions, default, dontAsk).
        #[arg(long, conflicts_with = "session")]
        permission_mode: Option<String>,
        /// Convenience for `--permission-mode acceptEdits`.
        #[arg(long, conflicts_with = "session")]
        auto_accept: bool,
        /// Convenience for `--permission-mode bypassPermissions` (skip all permission checks).
        #[arg(long, conflicts_with = "session")]
        bypass_permissions: bool,
        /// Zero-friction: claude's `--dangerously-skip-permissions` plus folder auto-trust.
        #[arg(long, conflicts_with = "session")]
        yolo: bool,
        /// Keep the session alive after a successful run (follow up with --session).
        #[arg(long)]
        keep: bool,
        /// Seconds to wait for the turn to finish (0 = no limit).
        #[arg(long, default_value_t = DEFAULT_RUN_TIMEOUT_SECS)]
        timeout: u64,
        /// Auto-approve plan gates ("Yes, and use auto mode"). Permission gates are never
        /// auto-approved — use a permission posture (--auto-accept/--bypass-permissions/--yolo).
        #[arg(long)]
        approve_plan: bool,
        /// Print every outcome as JSON on stdout (default: answer text on stdout, non-done
        /// outcomes as JSON on stderr).
        #[arg(long)]
        json: bool,
        /// Prompt text (omit to read it from stdin).
        #[arg(num_args = 0.., trailing_var_arg = true)]
        prompt: Vec<String>,
    },

    /// Inject a prompt into a session (send → verify echo → retry → submit).
    Send {
        /// Session name.
        session: String,
        /// Prompt text (the remaining args are joined with spaces).
        #[arg(required = true, num_args = 1.., trailing_var_arg = true)]
        prompt: Vec<String>,
        /// Type the prompt but do not press Enter.
        #[arg(long)]
        no_submit: bool,
        /// Max send→verify-echo attempts.
        #[arg(long, default_value_t = DEFAULT_SEND_RETRIES)]
        retries: u32,
    },

    /// Report the hybrid-detector state of a session (JSON).
    State {
        /// Session name.
        session: String,
    },

    /// Answer a plan-approval or permission gate by selecting a menu option.
    Approve {
        /// Session name.
        session: String,
        /// Menu option to select (default 1 = "Yes, and use auto mode").
        #[arg(long, default_value_t = 1)]
        option: u32,
    },

    /// List tracked sessions with liveness and live state.
    Ps {
        /// Emit JSON instead of a human table.
        #[arg(long)]
        json: bool,
    },

    /// Kill a session and remove its sidecar.
    Kill {
        /// Session name.
        session: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_session_sources_are_mutually_exclusive() {
        assert!(Cli::try_parse_from(["csd", "run", "--resume", "a", "--session", "b", "hi"]).is_err());
        assert!(Cli::try_parse_from(["csd", "run", "--session-id", "a", "--resume", "b", "hi"]).is_err());
        assert!(Cli::try_parse_from(["csd", "run", "--session", "a", "--yolo", "hi"]).is_err());
        assert!(Cli::try_parse_from(["csd", "run", "--resume", "a", "hi"]).is_ok());
    }

    #[test]
    fn run_prompt_may_be_omitted_for_stdin() {
        let cli = Cli::try_parse_from(["csd", "run"]).unwrap();
        let Command::Run { prompt, timeout, .. } = cli.command else {
            panic!("expected run");
        };
        assert!(prompt.is_empty());
        assert_eq!(timeout, DEFAULT_RUN_TIMEOUT_SECS);
    }
}
