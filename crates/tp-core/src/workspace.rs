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
}

/// What kind of panel to create — determines the widget type.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PanelType {
    /// Empty panel — shows type chooser
    Empty,
    /// Local terminal (VTE)
    #[default]
    Terminal,
    /// SSH terminal (russh → VTE)
    Ssh {
        host: String,
        #[serde(default = "default_ssh_port")]
        port: u16,
        #[serde(default)]
        user: Option<String>,
        #[serde(default)]
        identity_file: Option<String>,
    },
    /// Remote tmux session via SSH
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
    /// Embedded browser (WebKitGTK)
    Browser {
        url: String,
    },
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
    /// Resolve the effective panel type, merging legacy `target` into `panel_type`.
    pub fn effective_type(&self) -> PanelType {
        // Empty is always empty (chooser)
        if self.panel_type == PanelType::Empty {
            return PanelType::Empty;
        }
        // If panel_type is explicitly set to something other than Terminal, use it
        if self.panel_type != PanelType::Terminal {
            return self.panel_type.clone();
        }
        // Otherwise, check legacy target field
        match &self.target {
            PanelTarget::Local => PanelType::Terminal,
            PanelTarget::Ssh { host, port, user, identity_file } => PanelType::Ssh {
                host: host.clone(),
                port: *port,
                user: user.clone(),
                identity_file: identity_file.clone(),
            },
            PanelTarget::RemoteTmux { host, session, user } => PanelType::RemoteTmux {
                host: host.clone(),
                session: session.clone(),
                user: user.clone(),
            },
        }
    }

    /// Returns true if this panel type supports text input (terminal-like).
    pub fn accepts_input(&self) -> bool {
        matches!(
            self.effective_type(),
            PanelType::Terminal | PanelType::Ssh { .. } | PanelType::RemoteTmux { .. }
        ) && self.effective_type() != PanelType::Empty
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
    #[serde(default)]
    pub theme: String,
}

impl Default for WorkspaceSettings {
    fn default() -> Self {
        Self {
            default_shell: default_shell(),
            scrollback_lines: default_scrollback(),
            output_retention_days: None,
            theme: String::new(),
        }
    }
}

fn default_shell() -> String {
    std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string())
}

fn default_scrollback() -> usize {
    10_000
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
}
