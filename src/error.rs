use std::path::PathBuf;

use thiserror::Error;

/// All fallible operations in the csd library return this.
pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("tmux command failed: {0}")]
    Tmux(String),

    #[error("tmux is not installed or not on PATH")]
    TmuxMissing,

    #[error("session {0:?} does not exist")]
    NoSuchSession(String),

    #[error("session {0:?} already exists")]
    SessionExists(String),

    #[error("invalid session name {0:?}: use only [A-Za-z0-9._-], starting with a letter, digit or underscore")]
    InvalidSessionName(String),

    #[error("could not resolve {what} directory")]
    NoDir { what: &'static str },

    #[error("input was not accepted by the TUI after {0} attempts")]
    InputNotAccepted(u32),

    #[error("unknown backend {0:?}")]
    UnknownBackend(String),

    #[error("invalid session id {0:?}: {1}")]
    InvalidSessionId(String, String),

    #[error("invalid permission mode {0:?}, expected one of {1}")]
    InvalidPermissionMode(String, String),

    #[error("failed to read {path:?}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

impl Error {
    pub(crate) fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Error::Io {
            path: path.into(),
            source,
        }
    }
}
