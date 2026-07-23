use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const WORKSPACE_SNAPSHOT_VERSION: u32 = 2;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkspaceSnapshot {
    pub version: u32,
    pub workspace_id: Uuid,
    pub record_key: String,
    pub name: String,
    pub config_path: Option<String>,
    pub dirty: bool,
    pub focused_panel_id: Option<String>,
    pub zoomed_panel_id: Option<String>,
    pub active_tabs: Vec<ActiveTabSnapshot>,
    pub layout: LayoutSnapshot,
    pub panels: Vec<PanelSnapshot>,
}

impl WorkspaceSnapshot {
    pub fn provider_context(&self) -> serde_json::Value {
        crate::redacted_json(self).unwrap_or_else(|_| {
            serde_json::json!({
                "version": self.version,
                "workspace_id": self.workspace_id,
                "name": self.name,
                "error": "workspace snapshot serialization failed"
            })
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActiveTabSnapshot {
    pub path: Vec<usize>,
    pub selected_index: usize,
    pub tab_id: Option<String>,
    pub label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LayoutSnapshot {
    Panel {
        panel_id: String,
    },
    HorizontalSplit {
        children: Vec<LayoutSnapshot>,
        ratios: Vec<f64>,
    },
    VerticalSplit {
        children: Vec<LayoutSnapshot>,
        ratios: Vec<f64>,
    },
    Tabs {
        children: Vec<LayoutSnapshot>,
        labels: Vec<String>,
        tab_ids: Vec<String>,
    },
}

impl From<&pax_core::workspace::LayoutNode> for LayoutSnapshot {
    fn from(node: &pax_core::workspace::LayoutNode) -> Self {
        use pax_core::workspace::LayoutNode;

        match node {
            LayoutNode::Panel { id } => Self::Panel {
                panel_id: id.clone(),
            },
            LayoutNode::Hsplit { children, ratios } => Self::HorizontalSplit {
                children: children.iter().map(Self::from).collect(),
                ratios: ratios.clone(),
            },
            LayoutNode::Vsplit { children, ratios } => Self::VerticalSplit {
                children: children.iter().map(Self::from).collect(),
                ratios: ratios.clone(),
            },
            LayoutNode::Tabs {
                children,
                labels,
                tab_ids,
            } => Self::Tabs {
                children: children.iter().map(Self::from).collect(),
                labels: labels.clone(),
                tab_ids: tab_ids.clone(),
            },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PanelSnapshot {
    pub id: String,
    pub uuid: Uuid,
    pub name: String,
    pub kind: PanelKind,
    pub focused: bool,
    pub visible: bool,
    #[serde(default)]
    pub collapsed: bool,
    #[serde(default)]
    pub sync_input: bool,
    pub context: PanelContextSnapshot,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PanelKind {
    Empty,
    Terminal,
    Markdown,
    CodeEditor,
    DockerHelp,
    Note,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PanelContextSnapshot {
    Empty,
    Terminal {
        configured_cwd: Option<String>,
        ssh_enabled: bool,
        remote: Option<RemoteTargetSnapshot>,
    },
    Markdown {
        storage: String,
        file: Option<String>,
    },
    CodeEditor {
        root_dir: String,
        remote_path: Option<String>,
        remote: Option<RemoteTargetSnapshot>,
    },
    DockerHelp {
        docker_context: Option<String>,
        remote: Option<RemoteTargetSnapshot>,
    },
    Note,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RemoteTargetSnapshot {
    pub host: String,
    pub port: u16,
    pub user: Option<String>,
    pub tmux_session: Option<String>,
}
