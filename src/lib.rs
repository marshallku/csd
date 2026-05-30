//! csd (Claude Session Driver) — drive an interactive `claude` REPL in a detached tmux session on
//! the flat-rate subscription seat, inject input reliably, and detect session state, emitting JSON
//! for programmatic consumers (copad, life-assistant). See `docs/poc-findings.md` for the empirical
//! basis and `~/dev/copad/docs/decisions.md` #49 for why this exists.

pub mod backend;
pub mod cli;
pub mod commands;
pub mod detect;
pub mod error;
pub mod session;
pub mod tmux;

pub use error::{Error, Result};
