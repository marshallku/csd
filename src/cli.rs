//! Command-line surface (clap derive). Kept declarative; `main` maps these onto [`crate::commands`].

use std::path::PathBuf;

use clap::{Parser, Subcommand};

use crate::commands::DEFAULT_SEND_RETRIES;

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
        /// Convenience for `--permission-mode acceptEdits` (explicit --permission-mode wins).
        #[arg(long)]
        auto_accept: bool,
        /// Auto-clear the one-time "trust this folder?" startup gate so the session is driveable.
        #[arg(long)]
        trust: bool,
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
