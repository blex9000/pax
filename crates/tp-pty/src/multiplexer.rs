use anyhow::Result;
use tp_core::safety::{self, SafetyCheck};
use tp_core::workspace::Workspace;

use crate::manager::PtyManager;

/// Result of attempting to broadcast input.
#[derive(Debug)]
pub enum BroadcastResult {
    Sent(usize),
    Blocked(String),
    NeedsConfirmation(String),
}

/// Broadcast input to all panels in a group.
pub fn broadcast_to_group(
    pty: &mut PtyManager,
    ws: &Workspace,
    group_name: &str,
    input: &[u8],
) -> Result<BroadcastResult> {
    let group = ws
        .groups
        .iter()
        .find(|g| g.name == group_name)
        .ok_or_else(|| anyhow::anyhow!("Group '{}' not found", group_name))?;

    // Safety check: interpret input as a command line for checking
    let input_str = String::from_utf8_lossy(input);
    let trimmed = input_str.trim();

    // Only check complete lines (ending with newline in original input)
    if !trimmed.is_empty() && input.last() == Some(&b'\n') {
        match safety::check_command(trimmed, group)? {
            SafetyCheck::Blocked(reason) => return Ok(BroadcastResult::Blocked(reason)),
            SafetyCheck::NeedsConfirmation => {
                return Ok(BroadcastResult::NeedsConfirmation(format!(
                    "Group '{}' requires confirmation for: {}",
                    group_name, trimmed
                )));
            }
            SafetyCheck::Allowed => {}
        }
    }

    // Find all panels in this group and send input
    let panel_ids: Vec<String> = ws
        .panels
        .iter()
        .filter(|p| p.groups.iter().any(|g| g == group_name))
        .map(|p| p.id.clone())
        .collect();

    let mut sent = 0;
    for pid in &panel_ids {
        if pty.write_to_panel(pid, input).is_ok() {
            sent += 1;
        }
    }

    Ok(BroadcastResult::Sent(sent))
}
