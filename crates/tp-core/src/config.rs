use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

use crate::workspace::Workspace;

/// Default on-disk location for an unnamed-or-implicit workspace. Lives
/// under `$XDG_DATA_HOME/pax/workspaces/` (falling back to
/// `~/.local/share/pax/workspaces/` if `XDG_DATA_HOME` is unset or empty)
/// so per-panel UUIDs persist across launches that reuse the same name
/// — and the per-panel command-history popover keeps showing the
/// previous session.
///
/// `name` is sanitised by replacing path separators / control chars
/// with `_`. Returns `None` only when neither `$XDG_DATA_HOME` nor
/// `$HOME` is set (extremely unusual).
pub fn default_workspace_path(name: &str) -> Option<PathBuf> {
    let data_dir = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .or_else(|| {
            std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local").join("share"))
        })?;
    let safe: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | '\0' | ':' => '_',
            _ => c,
        })
        .collect();
    let trimmed = safe.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(
        data_dir
            .join("pax")
            .join("workspaces")
            .join(format!("{}.json", trimmed)),
    )
}

/// Load the workspace at `path` if the file exists, otherwise build a
/// fresh one with `factory`, write it to `path`, and return it. Used by
/// both `pax new "<name>"` (CLI) and the welcome-screen "New Workspace"
/// button so per-panel UUIDs survive restarts that reuse the same name.
pub fn open_or_create<F>(path: &Path, factory: F) -> Result<Workspace>
where
    F: FnOnce() -> Workspace,
{
    if path.is_file() {
        return load_workspace(path);
    }
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let ws = factory();
    save_workspace(&ws, path)?;
    Ok(ws)
}

/// Load workspace from a JSON file.
///
/// When the source file is missing any of the per-panel `uuid` fields
/// added in the command-history feature, this helper re-serialises and
/// writes the workspace back so the freshly-defaulted UUIDs are
/// persisted. Without that migration step every subsequent launch
/// would mint a different UUID and orphan the panel's history.
pub fn load_workspace(path: &Path) -> Result<Workspace> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Cannot read workspace file: {}", path.display()))?;
    let mut ws: Workspace = serde_json::from_str(&content)
        .with_context(|| format!("Invalid workspace JSON: {}", path.display()))?;
    ws.ensure_layout_tab_ids();
    validate_workspace(&ws)?;

    // Migration: bake any defaulted UUIDs back into the JSON. The check
    // is intentionally cheap — count occurrences of the `"uuid"` key in
    // the source text and compare to the panel count. Anything less
    // means at least one panel has no uuid in the file.
    let raw_uuid_count = content.matches("\"uuid\"").count();
    if raw_uuid_count < ws.panels.len() {
        save_workspace(&ws, path)?;
    }
    Ok(ws)
}

/// Save workspace to a JSON file.
pub fn save_workspace(ws: &Workspace, path: &Path) -> Result<()> {
    let mut normalized = ws.clone();
    normalized.ensure_layout_tab_ids();
    let json = serde_json::to_string_pretty(&normalized)?;
    std::fs::write(path, json)
        .with_context(|| format!("Cannot write workspace file: {}", path.display()))?;
    Ok(())
}

/// Validate workspace consistency.
fn validate_workspace(ws: &Workspace) -> Result<()> {
    let layout_ids: Vec<&str> = ws.layout.panel_ids();

    // Every panel ID in layout must have a matching PanelConfig
    for id in &layout_ids {
        if ws.panel(id).is_none() {
            anyhow::bail!("Layout references panel '{}' but no PanelConfig found", id);
        }
    }

    // Every PanelConfig must be referenced in layout
    for panel in &ws.panels {
        if !layout_ids.contains(&panel.id.as_str()) {
            anyhow::bail!(
                "PanelConfig '{}' exists but is not referenced in layout",
                panel.id
            );
        }
    }

    // Every group referenced by panels must exist
    let group_names: Vec<&str> = ws.groups.iter().map(|g| g.name.as_str()).collect();
    for panel in &ws.panels {
        for g in &panel.groups {
            if !group_names.contains(&g.as_str()) {
                anyhow::bail!(
                    "Panel '{}' references group '{}' which is not defined",
                    panel.id,
                    g
                );
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn sample_json() -> &'static str {
        r#"{
            "name": "test",
            "layout": {
                "type": "hsplit",
                "children": [
                    { "type": "panel", "id": "p1" },
                    { "type": "panel", "id": "p2" }
                ]
            },
            "panels": [
                { "id": "p1", "name": "Shell 1" },
                { "id": "p2", "name": "Shell 2" }
            ]
        }"#
    }

    #[test]
    fn test_load_workspace() {
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(sample_json().as_bytes()).unwrap();
        let ws = load_workspace(f.path()).unwrap();
        assert_eq!(ws.name, "test");
        assert_eq!(ws.panels.len(), 2);
    }

    #[test]
    fn test_load_default_workspace() {
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        let path = std::path::Path::new(manifest_dir)
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("config/default_workspace.json");
        if path.exists() {
            let ws = load_workspace(&path).unwrap();
            assert_eq!(ws.name, "default");
            assert_eq!(ws.panels.len(), 3);
            assert_eq!(ws.groups.len(), 1);
            assert_eq!(ws.alerts.len(), 1);
            assert_eq!(ws.alerts[0].actions.len(), 2);
        }
    }

    #[test]
    fn test_load_saved_workspace() {
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        let path = std::path::Path::new(manifest_dir)
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("config/workspace_save_test.json");
        if path.exists() {
            let ws = load_workspace(&path).unwrap();
            assert_eq!(ws.panels.len(), 4);
            // Verify layout has all panel IDs
            let ids = ws.layout.panel_ids();
            assert_eq!(ids.len(), 4);
            assert!(ids.contains(&"p1"));
            assert!(ids.contains(&"p2"));
            assert!(ids.contains(&"p3"));
            assert!(ids.contains(&"p4"));
            // Verify types
            for p in &ws.panels {
                println!("  {} -> {:?}", p.id, p.effective_type());
                assert_ne!(p.effective_type(), crate::workspace::PanelType::Empty);
            }
        }
    }

    #[test]
    fn test_save_load_roundtrip() {
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(sample_json().as_bytes()).unwrap();
        let ws = load_workspace(f.path()).unwrap();

        let out = NamedTempFile::new().unwrap();
        save_workspace(&ws, out.path()).unwrap();
        let ws2 = load_workspace(out.path()).unwrap();
        assert_eq!(ws.name, ws2.name);
        assert_eq!(ws.panels.len(), ws2.panels.len());
    }

    #[test]
    fn test_code_editor_roundtrip() {
        let json = r#"{
            "name": "editor-test",
            "layout": { "type": "panel", "id": "ed1" },
            "panels": [
                {
                    "id": "ed1",
                    "name": "Code",
                    "panel_type": { "type": "code_editor", "root_dir": "/tmp/project" }
                }
            ]
        }"#;
        let ws: crate::workspace::Workspace = serde_json::from_str(json).unwrap();
        assert_eq!(
            ws.panels[0].effective_type(),
            crate::workspace::PanelType::CodeEditor {
                root_dir: "/tmp/project".to_string(),
                ssh: None,
                remote_path: None,
                poll_interval: None
            }
        );

        // Roundtrip
        let serialized = serde_json::to_string_pretty(&ws).unwrap();
        let ws2: crate::workspace::Workspace = serde_json::from_str(&serialized).unwrap();
        assert_eq!(
            ws2.panels[0].effective_type(),
            ws.panels[0].effective_type()
        );
    }

    #[test]
    fn unknown_panel_type_loads_as_empty_panel() {
        let json = r#"{
            "name": "unknown-panel-test",
            "layout": { "type": "panel", "id": "legacy" },
            "panels": [
                {
                    "id": "legacy",
                    "name": "Legacy Browser",
                    "panel_type": { "type": "browser", "url": "https://example.com" }
                }
            ]
        }"#;

        let mut f = NamedTempFile::new().unwrap();
        f.write_all(json.as_bytes()).unwrap();
        let ws = load_workspace(f.path()).unwrap();

        assert_eq!(
            ws.panels[0].effective_type(),
            crate::workspace::PanelType::Empty
        );
    }

    #[test]
    fn legacy_tabs_are_normalized_with_tab_ids_on_load() {
        let json = r#"{
            "name": "tab-test",
            "layout": {
                "type": "tabs",
                "children": [
                    { "type": "panel", "id": "p1" },
                    { "type": "panel", "id": "p2" }
                ],
                "labels": ["One", "Two"]
            },
            "panels": [
                { "id": "p1", "name": "One" },
                { "id": "p2", "name": "Two" }
            ]
        }"#;

        let mut f = NamedTempFile::new().unwrap();
        f.write_all(json.as_bytes()).unwrap();
        let ws = load_workspace(f.path()).unwrap();

        match ws.layout {
            crate::workspace::LayoutNode::Tabs { tab_ids, .. } => {
                assert_eq!(tab_ids.len(), 2);
                assert_ne!(tab_ids[0], tab_ids[1]);
            }
            _ => panic!("expected tabs layout"),
        }
    }
}
