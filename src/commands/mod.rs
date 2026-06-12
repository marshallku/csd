//! Command implementations — the orchestration layer between the CLI ([`crate::cli`]) and the
//! primitives ([`crate::tmux`], [`crate::session`], [`crate::backend`], [`crate::detect`]).
//!
//! Each command returns a `Serialize` result; `main` renders it as JSON.

pub mod approve;
pub mod kill;
pub mod ps;
pub mod run;
pub mod send;
pub mod spawn;
pub mod state;

use std::time::Duration;

/// Default detached-pane size — generous so the TUI renders and capture-pane has room (PoC §2.1).
pub const DEFAULT_WIDTH: u16 = 220;
pub const DEFAULT_HEIGHT: u16 = 50;

/// How long to wait for injected keystrokes to settle before checking the echo (PoC gotcha #1).
pub const SEND_SETTLE: Duration = Duration::from_millis(800);
/// Pause between a confirmed prompt echo and pressing Enter (PoC §2.2).
pub const SUBMIT_DELAY: Duration = Duration::from_millis(1000);
/// Pause between sending a menu digit and pressing Enter (PoC §2.3).
pub const APPROVE_DELAY: Duration = Duration::from_millis(1000);
/// Default number of send→verify-echo attempts before giving up.
pub const DEFAULT_SEND_RETRIES: u32 = 5;

/// How long to watch for the one-time folder-trust gate after spawning with `--trust`.
pub const TRUST_POLL_ATTEMPTS: u32 = 12;
pub const TRUST_POLL_INTERVAL: Duration = Duration::from_millis(600);

/// How often `run` re-detects state while blocking on a turn.
pub const RUN_POLL_INTERVAL: Duration = Duration::from_millis(1500);
/// Default `run --timeout` (seconds); 0 disables the limit.
pub const DEFAULT_RUN_TIMEOUT_SECS: u64 = 600;
