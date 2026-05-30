//! Plan-file signal. In plan mode `claude` writes the plan to a markdown file under
//! `~/.claude/plans/` before the approval gate, and the plan *content* lives in that file rather
//! than the transcript message (PoC §3.3).
//!
//! NOTE: the filename is NOT `plan-*.md` (the PoC's guess) — on claude v2.1.158 it is a slug of the
//! prompt plus random words, e.g. `make-a-trivial-plan-parsed-whisper.md`. So we match any `*.md`.
//!
//! The plans dir is GLOBAL (not per-session), so we correlate by time: a `*.md` modified at or after
//! the session's spawn epoch is attributed to it. This is a documented heuristic — the combiner
//! additionally requires the capture-pane plan marker before declaring `plan_ready`, which makes a
//! stale-file false positive harmless.

use std::path::PathBuf;

use crate::error::Result;
use crate::session::{mtime_epoch, plans_dir};

/// The newest `plan-*.md` modified at/after `since_epoch`, with its content if readable.
pub fn latest_plan_file(since_epoch: u64) -> Result<Option<(PathBuf, Option<String>)>> {
    let dir = plans_dir()?;
    if !dir.exists() {
        return Ok(None);
    }

    let mut best: Option<(u64, PathBuf)> = None;
    for entry in std::fs::read_dir(&dir).map_err(|e| crate::error::Error::io(&dir, e))? {
        let Ok(entry) = entry else { continue };
        let path = entry.path();
        let is_plan = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.eq_ignore_ascii_case("md"))
            .unwrap_or(false);
        if !is_plan {
            continue;
        }
        let Some(mtime) = mtime_epoch(&path) else { continue };
        if mtime < since_epoch {
            continue;
        }
        if best.as_ref().map(|(t, _)| mtime > *t).unwrap_or(true) {
            best = Some((mtime, path));
        }
    }

    Ok(best.map(|(_, path)| {
        let content = std::fs::read_to_string(&path).ok();
        (path, content)
    }))
}
