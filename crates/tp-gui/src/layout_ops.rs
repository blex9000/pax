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

/// Remove a panel from the layout tree, collapsing empty containers.
pub fn remove_from_layout(node: &LayoutNode, panel_id: &str) -> LayoutNode {
    match node {
        LayoutNode::Panel { id } if id == panel_id => node.clone(),
        LayoutNode::Panel { .. } => node.clone(),
        LayoutNode::Hsplit { children, ratios } => {
            let filtered: Vec<LayoutNode> = children
                .iter()
                .filter(|c| !is_panel_with_id(c, panel_id))
                .map(|c| remove_from_layout(c, panel_id))
                .collect();
            let new_ratios: Vec<f64> = children
                .iter()
                .zip(ratios.iter())
                .filter(|(c, _)| !is_panel_with_id(c, panel_id))
                .map(|(_, r)| *r)
                .collect();
            if filtered.len() == 1 {
                filtered.into_iter().next().unwrap()
            } else {
                LayoutNode::Hsplit {
                    children: filtered,
                    ratios: new_ratios,
                }
            }
        }
        LayoutNode::Vsplit { children, ratios } => {
            let filtered: Vec<LayoutNode> = children
                .iter()
                .filter(|c| !is_panel_with_id(c, panel_id))
                .map(|c| remove_from_layout(c, panel_id))
                .collect();
            let new_ratios: Vec<f64> = children
                .iter()
                .zip(ratios.iter())
                .filter(|(c, _)| !is_panel_with_id(c, panel_id))
                .map(|(_, r)| *r)
                .collect();
            if filtered.len() == 1 {
                filtered.into_iter().next().unwrap()
            } else {
                LayoutNode::Vsplit {
                    children: filtered,
                    ratios: new_ratios,
                }
            }
        }
        LayoutNode::Tabs { children, labels } => {
            let mut new_children = Vec::new();
            let mut new_labels = Vec::new();
            for (i, child) in children.iter().enumerate() {
                if !is_panel_with_id(child, panel_id) {
                    new_children.push(remove_from_layout(child, panel_id));
                    if let Some(l) = labels.get(i) {
                        new_labels.push(l.clone());
                    }
                }
            }
            if new_children.len() == 1 {
                new_children.into_iter().next().unwrap()
            } else {
                LayoutNode::Tabs {
                    children: new_children,
                    labels: new_labels,
                }
            }
        }
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
