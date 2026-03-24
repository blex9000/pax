use tp_core::workspace::LayoutNode;

/// Replace a Panel node in the layout tree, returning a new tree.
pub fn replace_in_layout(
    node: &LayoutNode,
    panel_id: &str,
    replacer: &dyn Fn(&LayoutNode) -> LayoutNode,
) -> LayoutNode {
    match node {
        LayoutNode::Panel { id } if id == panel_id => replacer(node),
        LayoutNode::Panel { .. } => node.clone(),
        LayoutNode::Hsplit { children, ratios } => LayoutNode::Hsplit {
            children: children
                .iter()
                .map(|c| replace_in_layout(c, panel_id, replacer))
                .collect(),
            ratios: ratios.clone(),
        },
        LayoutNode::Vsplit { children, ratios } => LayoutNode::Vsplit {
            children: children
                .iter()
                .map(|c| replace_in_layout(c, panel_id, replacer))
                .collect(),
            ratios: ratios.clone(),
        },
        LayoutNode::Tabs { children, labels } => LayoutNode::Tabs {
            children: children
                .iter()
                .map(|c| replace_in_layout(c, panel_id, replacer))
                .collect(),
            labels: labels.clone(),
        },
    }
}

/// Check if a node has any panels left after removal.
fn has_panels(node: &LayoutNode) -> bool {
    match node {
        LayoutNode::Panel { .. } => true,
        LayoutNode::Hsplit { children, .. }
        | LayoutNode::Vsplit { children, .. }
        | LayoutNode::Tabs { children, .. } => {
            children.iter().any(|c| has_panels(c))
        }
    }
}

/// Remove a panel from the layout tree, collapsing containers that become
/// empty or have only one child. Works correctly for deeply nested panels.
pub fn remove_from_layout(node: &LayoutNode, panel_id: &str) -> LayoutNode {
    match node {
        LayoutNode::Panel { id } if id == panel_id => {
            // This node is the one to remove — caller will filter it out
            node.clone()
        }
        LayoutNode::Panel { .. } => node.clone(),
        LayoutNode::Hsplit { children, ratios } => {
            // Recurse into all children, then filter out empty ones
            let mut new_children = Vec::new();
            let mut new_ratios = Vec::new();
            for (i, child) in children.iter().enumerate() {
                if is_panel_with_id(child, panel_id) {
                    continue; // Direct match — skip
                }
                let processed = remove_from_layout(child, panel_id);
                if has_panels(&processed) {
                    new_children.push(processed);
                    new_ratios.push(ratios.get(i).copied().unwrap_or(1.0));
                }
            }
            collapse_split(new_children, new_ratios, true)
        }
        LayoutNode::Vsplit { children, ratios } => {
            let mut new_children = Vec::new();
            let mut new_ratios = Vec::new();
            for (i, child) in children.iter().enumerate() {
                if is_panel_with_id(child, panel_id) {
                    continue;
                }
                let processed = remove_from_layout(child, panel_id);
                if has_panels(&processed) {
                    new_children.push(processed);
                    new_ratios.push(ratios.get(i).copied().unwrap_or(1.0));
                }
            }
            collapse_split(new_children, new_ratios, false)
        }
        LayoutNode::Tabs { children, labels } => {
            let mut new_children = Vec::new();
            let mut new_labels = Vec::new();
            for (i, child) in children.iter().enumerate() {
                if is_panel_with_id(child, panel_id) {
                    continue;
                }
                let processed = remove_from_layout(child, panel_id);
                if has_panels(&processed) {
                    new_children.push(processed);
                    new_labels.push(labels.get(i).cloned().unwrap_or_else(|| format!("Tab {}", i + 1)));
                }
            }
            if new_children.len() == 1 {
                new_children.into_iter().next().unwrap()
            } else if new_children.is_empty() {
                // Should not happen if caller checks, but safety
                node.clone()
            } else {
                LayoutNode::Tabs {
                    children: new_children,
                    labels: new_labels,
                }
            }
        }
    }
}

/// Collapse a split node: if 0 children → panic safety, 1 child → unwrap, else keep split.
fn collapse_split(children: Vec<LayoutNode>, ratios: Vec<f64>, is_hsplit: bool) -> LayoutNode {
    if children.len() == 1 {
        children.into_iter().next().unwrap()
    } else if is_hsplit {
        LayoutNode::Hsplit { children, ratios }
    } else {
        LayoutNode::Vsplit { children, ratios }
    }
}

/// Try to add a new panel to an existing Tabs node that contains the given panel.
pub fn add_to_existing_tabs(node: &mut LayoutNode, panel_id: &str, new_id: &str, new_label: &str) -> bool {
    match node {
        LayoutNode::Tabs { children, labels } => {
            let contains = children.iter().any(|c| is_panel_with_id(c, panel_id));
            if contains {
                children.push(LayoutNode::Panel { id: new_id.to_string() });
                labels.push(new_label.to_string());
                return true;
            }
            for child in children.iter_mut() {
                if add_to_existing_tabs(child, panel_id, new_id, new_label) {
                    return true;
                }
            }
            false
        }
        LayoutNode::Hsplit { children, .. } | LayoutNode::Vsplit { children, .. } => {
            for child in children.iter_mut() {
                if add_to_existing_tabs(child, panel_id, new_id, new_label) {
                    return true;
                }
            }
            false
        }
        LayoutNode::Panel { .. } => false,
    }
}

pub fn is_panel_with_id(node: &LayoutNode, panel_id: &str) -> bool {
    matches!(node, LayoutNode::Panel { id } if id == panel_id)
}
