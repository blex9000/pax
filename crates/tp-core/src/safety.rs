use anyhow::Result;
use regex::Regex;

use crate::workspace::Group;

/// Result of checking a command against safety rules.
#[derive(Debug)]
pub enum SafetyCheck {
    Allowed,
    Blocked(String),
    NeedsConfirmation,
}

/// Check if a command is allowed for a given group.
pub fn check_command(command: &str, group: &Group) -> Result<SafetyCheck> {
    for pattern_str in &group.blocked_patterns {
        let re = Regex::new(pattern_str)?;
        if re.is_match(command) {
            return Ok(SafetyCheck::Blocked(format!(
                "Command matches blocked pattern '{}' in group '{}'",
                pattern_str, group.name
            )));
        }
    }

    if group.confirm_before_execute {
        return Ok(SafetyCheck::NeedsConfirmation);
    }

    Ok(SafetyCheck::Allowed)
}

/// Convenience: check against multiple groups, return first blocking result.
pub fn check_command_all_groups(command: &str, groups: &[&Group]) -> Result<SafetyCheck> {
    for group in groups {
        match check_command(command, group)? {
            SafetyCheck::Allowed => {}
            other => return Ok(other),
        }
    }
    Ok(SafetyCheck::Allowed)
}

/// Default blocklist applied to notebook cell code (markdown notebook
/// feature). Patterns are regex strings, matched as `regex::Regex::is_match`
/// against the full block body. Conservative defaults — extend if real
/// false-negatives appear in practice.
pub fn notebook_blocklist() -> Vec<String> {
    vec![
        r"\brm\s+-rf\s+/".to_string(),
        r"\brm\s+-rf\s+\$HOME".to_string(),
        r"\brm\s+-rf\s+~(\s|/|$)".to_string(),
        r"\bmkfs\b".to_string(),
        r"\bdd\s+if=.*of=/dev/".to_string(),
        r":\(\)\s*\{\s*:\|:&\s*\};:".to_string(), // fork bomb
        r"\bshutdown\b".to_string(),
        r"\breboot\b".to_string(),
        r"\bhalt\b".to_string(),
    ]
}

/// Apply the notebook blocklist to a piece of code about to be run by the
/// markdown notebook engine. Returns `Allowed` or `Blocked(reason)`. Never
/// returns `NeedsConfirmation` (notebook cells use the `confirm` tag, not
/// the group-level confirmation flow).
pub fn check_notebook_command(code: &str) -> Result<SafetyCheck> {
    for pattern_str in notebook_blocklist() {
        let re = Regex::new(&pattern_str)?;
        if re.is_match(code) {
            return Ok(SafetyCheck::Blocked(format!(
                "Code matches blocked pattern '{}'",
                pattern_str
            )));
        }
    }
    Ok(SafetyCheck::Allowed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_group() -> Group {
        Group {
            name: "production".to_string(),
            color: "red".to_string(),
            blocked_patterns: vec![
                r"^rm\s+-rf\s+/".to_string(),
                r"shutdown".to_string(),
                r"reboot".to_string(),
            ],
            confirm_before_execute: true,
        }
    }

    #[test]
    fn test_blocked_command() {
        let g = test_group();
        let result = check_command("rm -rf /var", &g).unwrap();
        assert!(matches!(result, SafetyCheck::Blocked(_)));
    }

    #[test]
    fn test_allowed_but_confirm() {
        let g = test_group();
        let result = check_command("ls -la", &g).unwrap();
        assert!(matches!(result, SafetyCheck::NeedsConfirmation));
    }

    #[test]
    fn test_fully_allowed() {
        let g = Group {
            name: "dev".to_string(),
            color: "green".to_string(),
            blocked_patterns: vec![],
            confirm_before_execute: false,
        };
        let result = check_command("ls -la", &g).unwrap();
        assert!(matches!(result, SafetyCheck::Allowed));
    }

    #[test]
    fn notebook_blocks_rm_rf_root() {
        let r = check_notebook_command("rm -rf /").unwrap();
        assert!(matches!(r, SafetyCheck::Blocked(_)));
    }

    #[test]
    fn notebook_blocks_fork_bomb() {
        let r = check_notebook_command(":(){ :|:& };:").unwrap();
        assert!(matches!(r, SafetyCheck::Blocked(_)));
    }

    #[test]
    fn notebook_blocks_shutdown() {
        let r = check_notebook_command("sudo shutdown -h now").unwrap();
        assert!(matches!(r, SafetyCheck::Blocked(_)));
    }

    #[test]
    fn notebook_allows_normal_python() {
        let r = check_notebook_command("import sys\nprint(sys.version)").unwrap();
        assert!(matches!(r, SafetyCheck::Allowed));
    }

    #[test]
    fn notebook_allows_rm_in_subdir() {
        // Relative paths (./, ../) do not start with `/` and are not matched.
        let r = check_notebook_command("rm -rf ./build").unwrap();
        assert!(matches!(r, SafetyCheck::Allowed));
    }

    #[test]
    fn notebook_blocks_rm_rf_absolute_paths() {
        for cmd in &["rm -rf /home/user", "rm -rf /tmp", "rm -rf /etc", "rm -rf /var/log"] {
            let r = check_notebook_command(cmd).unwrap();
            assert!(matches!(r, SafetyCheck::Blocked(_)), "should block: {}", cmd);
        }
    }
}
