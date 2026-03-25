use crate::workspace::{LayoutNode, PanelConfig, PanelType, Workspace, WorkspaceSettings};
use uuid::Uuid;

/// Create an empty workspace with a single chooser panel.
pub fn empty_workspace(name: &str) -> Workspace {
    Workspace {
        name: name.to_string(),
        id: Uuid::new_v4(),
        layout: LayoutNode::Panel { id: "p1".into() },
        panels: vec![PanelConfig {
            id: "p1".to_string(),
            name: "New Panel".to_string(),
            panel_type: PanelType::Empty,
            target: Default::default(),
            startup_commands: vec![],
            groups: vec![],
            record_output: false,
            cwd: None,
            env: Default::default(),
            pre_script: None,
            post_script: None,
            before_close: None,
            min_width: 0,
            min_height: 0,
            ssh: None,
        }],
        groups: vec![],
        alerts: vec![],
        startup_script: None,
        notes_file: None,
        settings: WorkspaceSettings::default(),
    }
}

/// Create a simple workspace with N horizontal panels.
pub fn simple_hsplit(name: &str, count: usize) -> Workspace {
    let panels: Vec<PanelConfig> = (0..count)
        .map(|i| PanelConfig {
            id: format!("p{}", i + 1),
            name: format!("Shell {}", i + 1),
            panel_type: Default::default(),
            target: Default::default(),
            startup_commands: vec![],
            groups: vec![],
            record_output: false,
            cwd: None,
            env: Default::default(),
            pre_script: None,
            post_script: None,
            before_close: None,
            min_width: 0,
            min_height: 0,
            ssh: None,
        })
        .collect();

    let children: Vec<LayoutNode> = panels
        .iter()
        .map(|p| LayoutNode::Panel { id: p.id.clone() })
        .collect();

    let ratios = vec![1.0; count];

    Workspace {
        name: name.to_string(),
        id: Uuid::new_v4(),
        layout: LayoutNode::Hsplit { children, ratios },
        panels,
        groups: vec![],
        alerts: vec![],
        startup_script: None,
        notes_file: None,
        settings: WorkspaceSettings::default(),
    }
}

/// Create a 2x2 grid layout.
pub fn grid_2x2(name: &str) -> Workspace {
    let panels: Vec<PanelConfig> = (1..=4)
        .map(|i| PanelConfig {
            id: format!("p{}", i),
            name: format!("Shell {}", i),
            panel_type: Default::default(),
            target: Default::default(),
            startup_commands: vec![],
            groups: vec![],
            record_output: false,
            cwd: None,
            env: Default::default(),
            pre_script: None,
            post_script: None,
            before_close: None,
            min_width: 0,
            min_height: 0,
            ssh: None,
        })
        .collect();

    let layout = LayoutNode::Vsplit {
        children: vec![
            LayoutNode::Hsplit {
                children: vec![
                    LayoutNode::Panel { id: "p1".into() },
                    LayoutNode::Panel { id: "p2".into() },
                ],
                ratios: vec![1.0, 1.0],
            },
            LayoutNode::Hsplit {
                children: vec![
                    LayoutNode::Panel { id: "p3".into() },
                    LayoutNode::Panel { id: "p4".into() },
                ],
                ratios: vec![1.0, 1.0],
            },
        ],
        ratios: vec![1.0, 1.0],
    };

    Workspace {
        name: name.to_string(),
        id: Uuid::new_v4(),
        layout,
        panels,
        groups: vec![],
        alerts: vec![],
        startup_script: None,
        notes_file: None,
        settings: WorkspaceSettings::default(),
    }
}
