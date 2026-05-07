use anyhow::Result;
use regex::Regex;

use crate::workspace::{AlertAction, AlertRule, AlertScope};

/// Compiled alert rule ready for matching.
pub struct CompiledAlert {
    pub rule: AlertRule,
    pub regex: Regex,
}

impl CompiledAlert {
    pub fn compile(rule: AlertRule) -> Result<Self> {
        let regex = Regex::new(&rule.pattern)?;
        Ok(Self { rule, regex })
    }

    /// Check if a line of output matches this alert for the given panel/groups.
    pub fn matches(&self, line: &str, panel_id: &str, panel_groups: &[String]) -> bool {
        if !self.regex.is_match(line) {
            return false;
        }
        match &self.rule.scope {
            AlertScope::All => true,
            AlertScope::Panels(ids) => ids.iter().any(|id| id == panel_id),
            AlertScope::Groups(groups) => panel_groups.iter().any(|pg| groups.contains(pg)),
        }
    }
}

/// Compile all alert rules in a workspace.
pub fn compile_alerts(rules: &[AlertRule]) -> Result<Vec<CompiledAlert>> {
    rules.iter().cloned().map(CompiledAlert::compile).collect()
}

/// Scan a line against all compiled alerts, return matching actions.
pub fn scan_line<'a>(
    alerts: &'a [CompiledAlert],
    line: &str,
    panel_id: &str,
    panel_groups: &[String],
) -> Vec<&'a AlertAction> {
    alerts
        .iter()
        .filter(|a| a.matches(line, panel_id, panel_groups))
        .flat_map(|a| &a.rule.actions)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::{AlertAction, AlertRule, AlertScope};

    #[test]
    fn test_alert_matching() {
        let rule = AlertRule {
            pattern: r"(?i)error".to_string(),
            scope: AlertScope::All,
            actions: vec![AlertAction::BorderColor("red".to_string())],
        };
        let compiled = CompiledAlert::compile(rule).unwrap();
        assert!(compiled.matches("Something ERROR happened", "p1", &[]));
        assert!(!compiled.matches("All good", "p1", &[]));
    }
}
