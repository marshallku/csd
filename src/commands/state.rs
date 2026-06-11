//! `csd state` — report the hybrid-detector verdict for one session.

use crate::backend;
use crate::detect::{self, State};
use crate::error::Result;
use crate::session::Session;

/// State output: the detector verdict plus the marker drift warning (computed from the
/// spawn-time `backend_version`, so polling stays subprocess-free).
#[derive(Debug, serde::Serialize)]
pub struct StateResult {
    #[serde(flatten)]
    pub state: State,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub marker_warning: Option<String>,
}

pub fn run(session_name: &str) -> Result<StateResult> {
    let session = Session::load(session_name)?;
    let state = detect::detect(&session)?;
    let backend = backend::resolve(&session.backend)?;
    let marker_warning = backend::marker_warning(backend.as_ref(), session.backend_version.as_deref());
    Ok(StateResult { state, marker_warning })
}
