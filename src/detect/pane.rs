//! capture-pane signal — the only reliable source for TUI-interrupt gates (plan approval,
//! tool-permission prompts) because those never reach the transcript until answered (PoC §3.3).
//!
//! Marker strings are backend-supplied and release-dependent (PoC gotcha #4).

/// Whether any of `markers` appears in the captured pane text.
pub fn contains_any(pane: &str, markers: &[&str]) -> bool {
    markers.iter().any(|m| pane.contains(m))
}

/// Parse a numbered selection menu out of the pane, e.g.
///
/// ```text
///  ❯ 1. Yes, and use auto mode
///    2. Yes, manually approve edits
/// ```
///
/// Returns the option labels in order (`["Yes, and use auto mode", ...]`), empty if no menu.
pub fn parse_menu_options(pane: &str) -> Vec<String> {
    let mut options = Vec::new();
    let mut expected = 1u32;
    for line in pane.lines() {
        // Strip the selection cursor and surrounding whitespace.
        let trimmed = line.trim_start_matches(['❯', '>', ' ', '\t']);
        let Some((num, rest)) = trimmed.split_once(". ") else {
            continue;
        };
        if num.parse::<u32>() == Ok(expected) {
            options.push(rest.trim().to_string());
            expected += 1;
        }
    }
    options
}

#[cfg(test)]
mod tests {
    use super::*;

    const MENU: &str = "\
 Claude has written up a plan and is ready to execute. Would you like to proceed?
 ❯ 1. Yes, and use auto mode
   2. Yes, manually approve edits
   3. No, refine with Ultraplan on Claude Code on the web
   4. Tell Claude what to change";

    #[test]
    fn parses_numbered_menu() {
        let opts = parse_menu_options(MENU);
        assert_eq!(opts.len(), 4);
        assert_eq!(opts[0], "Yes, and use auto mode");
        assert_eq!(opts[3], "Tell Claude what to change");
    }

    #[test]
    fn detects_marker() {
        assert!(contains_any(MENU, &["Would you like to proceed?"]));
        assert!(!contains_any(MENU, &["No such marker"]));
    }
}
