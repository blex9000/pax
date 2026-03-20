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
}
