//! `csd state` — report the hybrid-detector verdict for one session.

use crate::detect::{self, State};
use crate::error::Result;
use crate::session::Session;

pub fn run(session_name: &str) -> Result<State> {
    let session = Session::load(session_name)?;
    detect::detect(&session)
}
