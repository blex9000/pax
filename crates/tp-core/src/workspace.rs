use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// Root workspace definition — loaded from JSON config.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workspace {
    pub name: String,
    #[serde(default = "Uuid::new_v4")]
    pub id: Uuid,
    pub layout: LayoutNode,
    pub panels: Vec<PanelConfig>,
    #[serde(default)]
    pub groups: Vec<Group>,
    #[serde(default)]
    pub alerts: Vec<AlertRule>,
    #[serde(default)]
    pub startup_script: Option<String>,
    #[serde(default)]
    pub notes_file: Option<String>,
    #[serde(default)]
    pub settings: WorkspaceSettings,
    /// Saved SSH configurations reusable across panels.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ssh_configs: Vec<NamedSshConfig>,
}

/// A named SSH configuration saved at workspace level.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NamedSshConfig {
    pub name: String,
    pub config: SshConfig,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_path: Option<String>,
}

/// Recursive layout tree.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LayoutNode {
    Panel {
        id: String,
    },
    Hsplit {
        children: Vec<LayoutNode>,
        #[serde(default = "default_ratios")]
        ratios: Vec<f64>,
    },
    Vsplit {
        children: Vec<LayoutNode>,
        #[serde(default = "default_ratios")]
        ratios: Vec<f64>,
    },
    Tabs {
        children: Vec<LayoutNode>,
        #[serde(default)]
        labels: Vec<String>,
        #[serde(default)]
        tab_ids: Vec<String>,
    },
}

fn default_ratios() -> Vec<f64> {
    vec![1.0, 1.0]
}

/// Panel configuration — what gets spawned in a layout slot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PanelConfig {
    pub id: String,
    #[serde(default)]
    pub name: String,
    /// New: panel type determines what kind of widget is created.
    /// Falls back to `target` for backward compat if not present.
    #[serde(default)]
    pub panel_type: PanelType,
    /// Legacy field — kept for backward compatibility.
    /// If `panel_type` is default (Terminal) and `target` is set, use target.
    #[serde(default)]
    pub target: PanelTarget,
    #[serde(default)]
    pub startup_commands: Vec<String>,
    #[serde(default)]
    pub groups: Vec<String>,
    #[serde(default)]
    pub record_output: bool,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default)]
    pub pre_script: Option<String>,
    #[serde(default)]
    pub post_script: Option<String>,
    /// Script executed before closing the panel (cleanup, kill processes, etc.)
    #[serde(default)]
    pub before_close: Option<String>,
    /// Minimum width in pixels (0 = no minimum, panel shrinks freely).
    #[serde(default)]
    pub min_width: u32,
    /// Minimum height in pixels (0 = no minimum, panel shrinks freely).
    #[serde(default)]
    pub min_height: u32,
    /// SSH/remote connection settings (only for Terminal panels).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssh: Option<SshConfig>,
}

/// What kind of panel to create — determines the widget type.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PanelType {
    /// Empty panel — shows type chooser
    Empty,
    /// Terminal (local, SSH, or remote tmux)
    #[default]
    Terminal,
    /// Legacy SSH — deserialized as Terminal + SshConnection
    Ssh {
        host: String,
        #[serde(default = "default_ssh_port")]
        port: u16,
        #[serde(default)]
        user: Option<String>,
        #[serde(default)]
        password: Option<String>,
        #[serde(default)]
        identity_file: Option<String>,
    },
    /// Legacy RemoteTmux — deserialized as Terminal + TmuxConnection
    RemoteTmux {
        host: String,
        session: String,
        #[serde(default)]
        user: Option<String>,
    },
    /// Markdown viewer
    Markdown {
        file: String,
    },
    /// Browser panel: embedded WebKitGTK on Linux, native browser launcher on macOS.
    Browser {
        url: String,
    },
    /// Embedded code editor (local or remote via SSHFS)
    CodeEditor {
        root_dir: String,
        /// SSH config for remote editing.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ssh: Option<SshConfig>,
        /// Path on the remote host (used with ssh). Defaults to root_dir if not set.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        remote_path: Option<String>,
        /// File watcher poll interval in seconds (default 5 for remote, 2 for local).
        #[serde(default)]
        poll_interval: Option<u64>,
    },
}

/// SSH connection settings (stored in PanelConfig, not PanelType).
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct SshConfig {
    pub host: String,
    #[serde(default = "default_ssh_port")]
    pub port: u16,
    #[serde(default)]
    pub user: Option<String>,
    #[serde(default)]
    pub password: Option<String>,
    #[serde(default)]
    pub identity_file: Option<String>,
    /// If set, attach/create this tmux session on the remote host.
    #[serde(default)]
    pub tmux_session: Option<String>,
}

/// Legacy panel target — for backward compat with old configs.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PanelTarget {
    #[default]
    Local,
    Ssh {
        host: String,
        #[serde(default = "default_ssh_port")]
        port: u16,
        #[serde(default)]
        user: Option<String>,
        #[serde(default)]
        identity_file: Option<String>,
    },
    RemoteTmux {
        host: String,
        session: String,
        #[serde(default)]
        user: Option<String>,
    },
}

fn default_ssh_port() -> u16 {
    22
}

impl PanelConfig {
    /// Resolve the effective panel type.
    /// Legacy Ssh/RemoteTmux types are treated as Terminal (ssh config is in self.ssh).
    pub fn effective_type(&self) -> PanelType {
        match &self.panel_type {
            PanelType::Empty => PanelType::Empty,
            PanelType::Ssh { .. } | PanelType::RemoteTmux { .. } | PanelType::Terminal => PanelType::Terminal,
            other => other.clone(),
        }
    }

    /// Get the effective SSH config, merging from legacy PanelType::Ssh/RemoteTmux
    /// and legacy PanelTarget, into the modern PanelConfig.ssh field.
    pub fn effective_ssh(&self) -> Option<SshConfig> {
        // Modern field first
        if self.ssh.is_some() {
            return self.ssh.clone();
        }
        // Legacy PanelType::Ssh
        if let PanelType::Ssh { host, port, user, password, identity_file } = &self.panel_type {
            return Some(SshConfig {
                host: host.clone(), port: *port, user: user.clone(),
                password: password.clone(), identity_file: identity_file.clone(),
                tmux_session: None,
            });
        }
        // Legacy PanelType::RemoteTmux
        if let PanelType::RemoteTmux { host, session, user } = &self.panel_type {
            return Some(SshConfig {
                host: host.clone(), port: 22, user: user.clone(),
                password: None, identity_file: None,
                tmux_session: Some(session.clone()),
            });
        }
        // Legacy PanelTarget
        match &self.target {
            PanelTarget::Ssh { host, port, user, identity_file } => Some(SshConfig {
                host: host.clone(), port: *port, user: user.clone(),
                password: None, identity_file: identity_file.clone(),
                tmux_session: None,
            }),
            PanelTarget::RemoteTmux { host, session, user } => Some(SshConfig {
                host: host.clone(), port: 22, user: user.clone(),
                password: None, identity_file: None,
                tmux_session: Some(session.clone()),
            }),
            PanelTarget::Local => None,
        }
    }

    /// Returns true if this panel type supports text input (terminal-like).
    pub fn accepts_input(&self) -> bool {
        self.effective_type() == PanelType::Terminal
    }
}

/// Broadcast group with safety rules.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Group {
    pub name: String,
    #[serde(default = "default_color")]
    pub color: String,
    #[serde(default)]
    pub blocked_patterns: Vec<String>,
    #[serde(default)]
    pub confirm_before_execute: bool,
}

fn default_color() -> String {
    "yellow".to_string()
}

/// Alert rule: pattern match on terminal output → actions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertRule {
    pub pattern: String,
    #[serde(default)]
    pub scope: AlertScope,
    #[serde(default)]
    pub actions: Vec<AlertAction>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AlertScope {
    #[default]
    All,
    Panels(Vec<String>),
    Groups(Vec<String>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlertAction {
    BorderColor(String),
    DesktopNotification,
    Sound,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceSettings {
    #[serde(default = "default_shell")]
    pub default_shell: String,
    #[serde(default = "default_scrollback")]
    pub scrollback_lines: usize,
    #[serde(default)]
    pub output_retention_days: Option<u32>,
    #[serde(default = "default_theme")]
    pub theme: String,
}

impl Default for WorkspaceSettings {
    fn default() -> Self {
        Self {
            default_shell: default_shell(),
            scrollback_lines: default_scrollback(),
            output_retention_days: None,
            theme: default_theme(),
        }
    }
}

fn default_shell() -> String {
    std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string())
}

fn default_scrollback() -> usize {
    10_000
}

fn default_theme() -> String {
    "nord".to_string()
}

pub fn new_tab_id() -> String {
    format!("tab-{}", Uuid::new_v4().simple())
}

impl LayoutNode {
    /// Collect all panel IDs referenced in this layout.
    pub fn panel_ids(&self) -> Vec<&str> {
        match self {
            LayoutNode::Panel { id } => vec![id.as_str()],
            LayoutNode::Hsplit { children, .. }
            | LayoutNode::Vsplit { children, .. }
            | LayoutNode::Tabs { children, .. } => {
                children.iter().flat_map(|c| c.panel_ids()).collect()
            }
        }
    }

    /// Ensure every Tabs node has a stable tab id for each child.
    /// Legacy workspaces may deserialize with no tab_ids at all.
    pub fn ensure_tab_ids(&mut self) {
        match self {
            LayoutNode::Panel { .. } => {}
            LayoutNode::Hsplit { children, .. } | LayoutNode::Vsplit { children, .. } => {
                for child in children {
                    child.ensure_tab_ids();
                }
            }
            LayoutNode::Tabs {
                children, tab_ids, ..
            } => {
                for child in children.iter_mut() {
                    child.ensure_tab_ids();
                }
                if tab_ids.len() > children.len() {
                    tab_ids.truncate(children.len());
                }
                while tab_ids.len() < children.len() {
                    tab_ids.push(new_tab_id());
                }
            }
        }
    }
}

impl Workspace {
    /// Find a panel config by ID.
    pub fn panel(&self, id: &str) -> Option<&PanelConfig> {
        self.panels.iter().find(|p| p.id == id)
    }

    /// Get all panels in a given group.
    pub fn panels_in_group(&self, group: &str) -> Vec<&PanelConfig> {
        self.panels
            .iter()
            .filter(|p| p.groups.iter().any(|g| g == group))
            .collect()
    }

    pub fn ensure_layout_tab_ids(&mut self) {
        self.layout.ensure_tab_ids();
    }
}

#[cfg(test)]
mod tests {
    use super::{new_tab_id, LayoutNode, Workspace, WorkspaceSettings};

    #[test]
    fn workspace_settings_default_to_nord_theme() {
        assert_eq!(WorkspaceSettings::default().theme, "nord");
    }

    #[test]
    fn new_tab_id_has_expected_prefix() {
        assert!(new_tab_id().starts_with("tab-"));
    }

    #[test]
    fn ensure_layout_tab_ids_backfills_legacy_tabs() {
        let mut workspace = Workspace {
            name: "demo".to_string(),
            id: uuid::Uuid::new_v4(),
            layout: LayoutNode::Tabs {
                children: vec![
                    LayoutNode::Panel {
                        id: "a".to_string(),
                    },
                    LayoutNode::Panel {
                        id: "b".to_string(),
                    },
                ],
                labels: vec!["A".to_string(), "B".to_string()],
                tab_ids: Vec::new(),
            },
            panels: Vec::new(),
            groups: Vec::new(),
            alerts: Vec::new(),
            startup_script: None,
            notes_file: None,
            settings: WorkspaceSettings::default(),
            ssh_configs: Vec::new(),
        };

        workspace.ensure_layout_tab_ids();

        match &workspace.layout {
            LayoutNode::Tabs { tab_ids, .. } => {
                assert_eq!(tab_ids.len(), 2);
                assert_ne!(tab_ids[0], tab_ids[1]);
                assert!(tab_ids.iter().all(|id| id.starts_with("tab-")));
            }
            _ => panic!("expected tabs layout"),
        }
    }
}
