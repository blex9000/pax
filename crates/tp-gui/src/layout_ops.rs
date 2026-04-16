use pax_core::workspace::{new_tab_id, LayoutNode};

/// Log the full layout tree with paths for debugging.
pub fn debug_layout_tree(node: &LayoutNode, path: &str) {
    match node {
        LayoutNode::Panel { id } => {
            tracing::debug!("LAYOUT {}: Panel({})", path, id);
        }
        LayoutNode::Hsplit { children, ratios } => {
            tracing::debug!(
                "LAYOUT {}: Hsplit({} children, ratios={:?})",
                path,
                children.len(),
                ratios
                    .iter()
                    .map(|r| format!("{:.2}", r))
                    .collect::<Vec<_>>()
            );
            for (i, c) in children.iter().enumerate() {
                debug_layout_tree(c, &format!("{}/h{}", path, i));
            }
        }
        LayoutNode::Vsplit { children, ratios } => {
            tracing::debug!(
                "LAYOUT {}: Vsplit({} children, ratios={:?})",
                path,
                children.len(),
                ratios
                    .iter()
                    .map(|r| format!("{:.2}", r))
                    .collect::<Vec<_>>()
            );
            for (i, c) in children.iter().enumerate() {
                debug_layout_tree(c, &format!("{}/v{}", path, i));
            }
        }
        LayoutNode::Tabs {
            children,
            labels,
            tab_ids,
        } => {
            tracing::debug!(
                "LAYOUT {}: Tabs({} children, labels={:?}, tab_ids={:?})",
                path,
                children.len(),
                labels,
                tab_ids
            );
            for (i, c) in children.iter().enumerate() {
                debug_layout_tree(c, &format!("{}/t{}", path, i));
            }
        }
    }
}

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
        LayoutNode::Tabs {
            children,
            labels,
            tab_ids,
        } => LayoutNode::Tabs {
            children: children
                .iter()
                .map(|c| replace_in_layout(c, panel_id, replacer))
                .collect(),
            labels: labels.clone(),
            tab_ids: tab_ids.clone(),
        },
    }
}

/// Check if a node has any panels left after removal.
fn has_panels(node: &LayoutNode) -> bool {
    match node {
        LayoutNode::Panel { .. } => true,
        LayoutNode::Hsplit { children, .. }
        | LayoutNode::Vsplit { children, .. }
        | LayoutNode::Tabs { children, .. } => children.iter().any(|c| has_panels(c)),
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
                if is_panel_direct(child, panel_id) {
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
                if is_panel_direct(child, panel_id) {
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
        LayoutNode::Tabs {
            children,
            labels,
            tab_ids,
        } => {
            let mut new_children = Vec::new();
            let mut new_labels = Vec::new();
            let mut new_tab_ids = Vec::new();
            for (i, child) in children.iter().enumerate() {
                if is_panel_direct(child, panel_id) {
                    continue;
                }
                let processed = remove_from_layout(child, panel_id);
                if has_panels(&processed) {
                    new_children.push(processed);
                    new_labels.push(
                        labels
                            .get(i)
                            .cloned()
                            .unwrap_or_else(|| format!("Tab {}", i + 1)),
                    );
                    new_tab_ids.push(
                        tab_ids
                            .get(i)
                            .cloned()
                            .unwrap_or_else(new_tab_id),
                    );
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
                    tab_ids: new_tab_ids,
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
pub fn add_to_existing_tabs(
    node: &mut LayoutNode,
    panel_id: &str,
    new_id: &str,
    new_label: &str,
    new_tab_id_value: &str,
) -> bool {
    match node {
        LayoutNode::Tabs {
            children,
            labels,
            tab_ids,
        } => {
            for child in children.iter_mut() {
                if add_to_existing_tabs(child, panel_id, new_id, new_label, new_tab_id_value) {
                    return true;
                }
            }
            let contains = children.iter().any(|c| is_panel_with_id(c, panel_id));
            if contains {
                children.push(LayoutNode::Panel {
                    id: new_id.to_string(),
                });
                labels.push(new_label.to_string());
                tab_ids.push(new_tab_id_value.to_string());
                return true;
            }
            false
        }
        LayoutNode::Hsplit { children, .. } | LayoutNode::Vsplit { children, .. } => {
            for child in children.iter_mut() {
                if add_to_existing_tabs(child, panel_id, new_id, new_label, new_tab_id_value) {
                    return true;
                }
            }
            false
        }
        LayoutNode::Panel { .. } => false,
    }
}

/// Add a new panel to the exact Tabs node at the provided layout path.
/// The path addresses the Tabs node itself, not one of its page children.
pub fn add_to_tabs_at_path(
    node: &mut LayoutNode,
    tabs_path: &[usize],
    new_id: &str,
    new_label: &str,
    new_tab_id_value: &str,
) -> bool {
    if tabs_path.is_empty() {
        if let LayoutNode::Tabs {
            children,
            labels,
            tab_ids,
        } = node
        {
            children.push(LayoutNode::Panel {
                id: new_id.to_string(),
            });
            labels.push(new_label.to_string());
            tab_ids.push(new_tab_id_value.to_string());
            return true;
        }
        return false;
    }

    let (next, rest) = match tabs_path.split_first() {
        Some(parts) => parts,
        None => return false,
    };

    match node {
        LayoutNode::Tabs { children, .. }
        | LayoutNode::Hsplit { children, .. }
        | LayoutNode::Vsplit { children, .. } => children
            .get_mut(*next)
            .map(|child| add_to_tabs_at_path(child, rest, new_id, new_label, new_tab_id_value))
            .unwrap_or(false),
        LayoutNode::Panel { .. } => false,
    }
}

/// Check if a node IS exactly this panel (direct match only).
pub fn is_panel_direct(node: &LayoutNode, panel_id: &str) -> bool {
    matches!(node, LayoutNode::Panel { id } if id == panel_id)
}

/// Check if a node IS the panel, or CONTAINS the panel anywhere in its subtree.
pub fn is_panel_with_id(node: &LayoutNode, panel_id: &str) -> bool {
    match node {
        LayoutNode::Panel { id } => id == panel_id,
        LayoutNode::Hsplit { children, .. }
        | LayoutNode::Vsplit { children, .. }
        | LayoutNode::Tabs { children, .. } => {
            children.iter().any(|c| is_panel_with_id(c, panel_id))
        }
    }
}

/// Update a tab label in the layout tree for the given panel ID.
/// Renames the INNERMOST (closest) Tabs node that directly contains the panel,
/// not an ancestor Tabs that contains it deeply nested.
pub fn update_tab_label_in_layout(node: &mut LayoutNode, panel_id: &str, new_label: &str) -> bool {
    match node {
        LayoutNode::Tabs { children, labels, .. } => {
            // First: recurse into children to find a deeper Tabs match
            for child in children.iter_mut() {
                if update_tab_label_in_layout(child, panel_id, new_label) {
                    return true; // Handled by a deeper Tabs node
                }
            }
            // No deeper match — check if WE directly contain it
            for (i, child) in children.iter().enumerate() {
                if is_panel_with_id(child, panel_id) {
                    if let Some(l) = labels.get_mut(i) {
                        *l = new_label.to_string();
                    }
                    return true;
                }
            }
            false
        }
        LayoutNode::Hsplit { children, .. } | LayoutNode::Vsplit { children, .. } => {
            for child in children.iter_mut() {
                if update_tab_label_in_layout(child, panel_id, new_label) {
                    return true;
                }
            }
            false
        }
        LayoutNode::Panel { .. } => false,
    }
}

pub fn update_tab_label_in_layout_by_id(node: &mut LayoutNode, tab_id: &str, new_label: &str) -> bool {
    match node {
        LayoutNode::Tabs {
            children,
            labels,
            tab_ids,
        } => {
            for child in children.iter_mut() {
                if update_tab_label_in_layout_by_id(child, tab_id, new_label) {
                    return true;
                }
            }
            for (i, current_tab_id) in tab_ids.iter().enumerate() {
                if current_tab_id == tab_id {
                    if let Some(label) = labels.get_mut(i) {
                        *label = new_label.to_string();
                    }
                    return true;
                }
            }
            false
        }
        LayoutNode::Hsplit { children, .. } | LayoutNode::Vsplit { children, .. } => {
            for child in children.iter_mut() {
                if update_tab_label_in_layout_by_id(child, tab_id, new_label) {
                    return true;
                }
            }
            false
        }
        LayoutNode::Panel { .. } => false,
    }
}

/// Update a tab label by its exact layout path.
/// The path indexes through the layout tree and ends at the tab index inside
/// the owning Tabs node, so nested tabs are unambiguous.
pub fn update_tab_label_in_layout_by_path(
    node: &mut LayoutNode,
    path: &[usize],
    new_label: &str,
) -> bool {
    match node {
        LayoutNode::Tabs {
            children,
            labels,
            tab_ids: _,
        } => {
            let Some((index, rest)) = path.split_first() else {
                return false;
            };
            if *index >= children.len() {
                return false;
            }
            if rest.is_empty() {
                if let Some(label) = labels.get_mut(*index) {
                    *label = new_label.to_string();
                    return true;
                }
                return false;
            }
            update_tab_label_in_layout_by_path(&mut children[*index], rest, new_label)
        }
        LayoutNode::Hsplit { children, .. } | LayoutNode::Vsplit { children, .. } => {
            let Some((index, rest)) = path.split_first() else {
                return false;
            };
            if *index >= children.len() {
                return false;
            }
            update_tab_label_in_layout_by_path(&mut children[*index], rest, new_label)
        }
        LayoutNode::Panel { .. } => false,
    }
}

/// Move the INNERMOST tab containing `panel_id` by one position.
/// Returns true if a move happened.
pub fn move_tab_in_layout(node: &mut LayoutNode, panel_id: &str, direction: i32) -> bool {
    match node {
        LayoutNode::Tabs {
            children,
            labels,
            tab_ids,
        } => {
            for child in children.iter_mut() {
                if move_tab_in_layout(child, panel_id, direction) {
                    return true;
                }
            }

            for (index, child) in children.iter().enumerate() {
                if is_panel_with_id(child, panel_id) {
                    let target = index as i32 + direction;
                    if !(0..children.len() as i32).contains(&target) {
                        return false;
                    }
                    let target = target as usize;
                    children.swap(index, target);
                    if index < labels.len() && target < labels.len() {
                        labels.swap(index, target);
                    }
                    if index < tab_ids.len() && target < tab_ids.len() {
                        tab_ids.swap(index, target);
                    }
                    return true;
                }
            }
            false
        }
        LayoutNode::Hsplit { children, .. } | LayoutNode::Vsplit { children, .. } => {
            for child in children.iter_mut() {
                if move_tab_in_layout(child, panel_id, direction) {
                    return true;
                }
            }
            false
        }
        LayoutNode::Panel { .. } => false,
    }
}

/// Move the tab at an exact layout path by one position within its owning Tabs node.
/// Returns the updated path when a move happened.
pub fn move_tab_in_layout_by_path(
    node: &mut LayoutNode,
    path: &[usize],
    direction: i32,
) -> Option<Vec<usize>> {
    match node {
        LayoutNode::Tabs {
            children,
            labels,
            tab_ids,
        } => {
            let (index, rest) = path.split_first()?;
            if *index >= children.len() {
                return None;
            }
            if rest.is_empty() {
                let target = *index as i32 + direction;
                if !(0..children.len() as i32).contains(&target) {
                    return None;
                }
                let target = target as usize;
                children.swap(*index, target);
                if *index < labels.len() && target < labels.len() {
                    labels.swap(*index, target);
                }
                if *index < tab_ids.len() && target < tab_ids.len() {
                    tab_ids.swap(*index, target);
                }
                return Some(vec![target]);
            }

            move_tab_in_layout_by_path(&mut children[*index], rest, direction).map(
                |mut child_path| {
                    let mut updated = vec![*index];
                    updated.append(&mut child_path);
                    updated
                },
            )
        }
        LayoutNode::Hsplit { children, .. } | LayoutNode::Vsplit { children, .. } => {
            let (index, rest) = path.split_first()?;
            if *index >= children.len() {
                return None;
            }
            move_tab_in_layout_by_path(&mut children[*index], rest, direction).map(
                |mut child_path| {
                    let mut updated = vec![*index];
                    updated.append(&mut child_path);
                    updated
                },
            )
        }
        LayoutNode::Panel { .. } => None,
    }
}

/// Move the INNERMOST tab containing `panel_id` by a signed offset.
/// Applies the move one step at a time so the final order matches repeated
/// left/right actions in the UI.
pub fn move_tab_in_layout_steps(node: &mut LayoutNode, panel_id: &str, offset: i32) -> bool {
    let step = offset.signum();
    if step == 0 {
        return false;
    }

    let mut moved = false;
    for _ in 0..offset.abs() {
        if !move_tab_in_layout(node, panel_id, step) {
            break;
        }
        moved = true;
    }
    moved
}

#[cfg(test)]
mod tests {
    use super::*;

    fn panel(id: &str) -> LayoutNode {
        LayoutNode::Panel { id: id.to_string() }
    }

    fn hsplit(children: Vec<LayoutNode>) -> LayoutNode {
        let n = children.len();
        LayoutNode::Hsplit {
            children,
            ratios: vec![1.0 / n as f64; n],
        }
    }

    fn vsplit(children: Vec<LayoutNode>) -> LayoutNode {
        let n = children.len();
        LayoutNode::Vsplit {
            children,
            ratios: vec![1.0 / n as f64; n],
        }
    }

    fn tabs(children: Vec<LayoutNode>, labels: Vec<&str>) -> LayoutNode {
        LayoutNode::Tabs {
            children,
            tab_ids: (0..labels.len()).map(|_| new_tab_id()).collect(),
            labels: labels.into_iter().map(|s| s.to_string()).collect(),
        }
    }

    fn panel_ids(node: &LayoutNode) -> Vec<String> {
        node.panel_ids()
            .into_iter()
            .map(|s| s.to_string())
            .collect()
    }

    // ── remove_from_layout ──

    #[test]
    fn remove_direct_panel_from_hsplit() {
        let layout = hsplit(vec![panel("a"), panel("b"), panel("c")]);
        let result = remove_from_layout(&layout, "b");
        assert_eq!(panel_ids(&result), vec!["a", "c"]);
    }

    #[test]
    fn remove_last_leaves_single_panel() {
        let layout = hsplit(vec![panel("a"), panel("b")]);
        let result = remove_from_layout(&layout, "b");
        // Should collapse to single panel
        assert!(matches!(result, LayoutNode::Panel { id } if id == "a"));
    }

    #[test]
    fn remove_nested_panel_keeps_siblings() {
        // Tabs [ Vsplit[a, b], c ]
        let layout = tabs(
            vec![vsplit(vec![panel("a"), panel("b")]), panel("c")],
            vec!["tab1", "tab2"],
        );
        let result = remove_from_layout(&layout, "a");
        let ids = panel_ids(&result);
        assert!(ids.contains(&"b".to_string()), "sibling b must survive");
        assert!(ids.contains(&"c".to_string()), "sibling c must survive");
        assert!(!ids.contains(&"a".to_string()), "removed a must be gone");
    }

    #[test]
    fn remove_deeply_nested_no_orphans() {
        // Hsplit [ Vsplit[Tabs[a, b], c], d ]
        let layout = hsplit(vec![
            vsplit(vec![
                tabs(vec![panel("a"), panel("b")], vec!["t1", "t2"]),
                panel("c"),
            ]),
            panel("d"),
        ]);
        let result = remove_from_layout(&layout, "a");
        let ids = panel_ids(&result);
        assert_eq!(ids.len(), 3);
        assert!(ids.contains(&"b".to_string()));
        assert!(ids.contains(&"c".to_string()));
        assert!(ids.contains(&"d".to_string()));
    }

    #[test]
    fn remove_panel_from_tabs_collapses() {
        let layout = tabs(vec![panel("a"), panel("b")], vec!["t1", "t2"]);
        let result = remove_from_layout(&layout, "a");
        assert!(matches!(result, LayoutNode::Panel { id } if id == "b"));
    }

    // ── is_panel_direct vs is_panel_with_id ──

    #[test]
    fn is_panel_direct_only_matches_exact() {
        let layout = vsplit(vec![panel("a"), panel("b")]);
        assert!(!is_panel_direct(&layout, "a")); // vsplit is not panel a
        assert!(is_panel_direct(&panel("a"), "a"));
    }

    #[test]
    fn is_panel_with_id_matches_nested() {
        let layout = vsplit(vec![panel("a"), panel("b")]);
        assert!(is_panel_with_id(&layout, "a")); // a is inside vsplit
        assert!(!is_panel_with_id(&layout, "c"));
    }

    // ── update_tab_label_in_layout ──

    #[test]
    fn rename_tab_with_direct_panel() {
        let mut layout = tabs(vec![panel("a"), panel("b")], vec!["old_a", "old_b"]);
        update_tab_label_in_layout(&mut layout, "a", "new_a");
        if let LayoutNode::Tabs { labels, .. } = &layout {
            assert_eq!(labels[0], "new_a");
            assert_eq!(labels[1], "old_b");
        } else {
            panic!("expected Tabs");
        }
    }

    #[test]
    fn rename_tab_with_nested_panel() {
        // Tab contains Vsplit[a, b] — renaming by panel a should update tab label
        let mut layout = tabs(
            vec![vsplit(vec![panel("a"), panel("b")]), panel("c")],
            vec!["old_tab", "tab_c"],
        );
        update_tab_label_in_layout(&mut layout, "a", "new_tab");
        if let LayoutNode::Tabs { labels, .. } = &layout {
            assert_eq!(labels[0], "new_tab");
            assert_eq!(labels[1], "tab_c");
        } else {
            panic!("expected Tabs");
        }
    }

    #[test]
    fn move_tab_left_swaps_children_and_labels() {
        let mut layout = tabs(
            vec![panel("a"), panel("b"), panel("c")],
            vec!["first", "second", "third"],
        );

        let moved = move_tab_in_layout(&mut layout, "b", -1);

        assert!(moved);
        if let LayoutNode::Tabs { children, labels, .. } = &layout {
            assert!(matches!(&children[0], LayoutNode::Panel { id } if id == "b"));
            assert!(matches!(&children[1], LayoutNode::Panel { id } if id == "a"));
            assert_eq!(labels, &["second", "first", "third"]);
        } else {
            panic!("expected Tabs");
        }
    }

    #[test]
    fn move_tab_right_uses_innermost_tabs_node() {
        let mut layout = tabs(
            vec![
                vsplit(vec![
                    panel("a"),
                    tabs(vec![panel("b"), panel("c")], vec!["inner-b", "inner-c"]),
                ]),
                panel("d"),
            ],
            vec!["outer-left", "outer-right"],
        );

        let moved = move_tab_in_layout(&mut layout, "b", 1);

        assert!(moved);
        if let LayoutNode::Tabs {
            children: outer_children,
            labels: outer_labels,
            ..
        } = &layout
        {
            assert_eq!(outer_labels, &["outer-left", "outer-right"]);
            if let LayoutNode::Vsplit { children, .. } = &outer_children[0] {
                if let LayoutNode::Tabs { labels, children, .. } = &children[1] {
                    assert_eq!(labels, &["inner-c", "inner-b"]);
                    assert!(matches!(&children[0], LayoutNode::Panel { id } if id == "c"));
                    assert!(matches!(&children[1], LayoutNode::Panel { id } if id == "b"));
                } else {
                    panic!("expected inner tabs");
                }
            } else {
                panic!("expected outer vsplit");
            }
        } else {
            panic!("expected outer tabs");
        }
    }

    #[test]
    fn move_tab_out_of_bounds_returns_false() {
        let mut layout = tabs(vec![panel("a"), panel("b")], vec!["first", "second"]);

        assert!(!move_tab_in_layout(&mut layout, "a", -1));
        assert!(!move_tab_in_layout(&mut layout, "b", 1));
        if let LayoutNode::Tabs { labels, .. } = &layout {
            assert_eq!(labels, &["first", "second"]);
        } else {
            panic!("expected tabs");
        }
    }

    #[test]
    fn move_tab_steps_matches_repeated_single_moves() {
        let mut layout = tabs(
            vec![panel("a"), panel("b"), panel("c")],
            vec!["first", "second", "third"],
        );

        let moved = move_tab_in_layout_steps(&mut layout, "a", 2);

        assert!(moved);
        if let LayoutNode::Tabs { children, labels, .. } = &layout {
            assert!(matches!(&children[0], LayoutNode::Panel { id } if id == "b"));
            assert!(matches!(&children[1], LayoutNode::Panel { id } if id == "c"));
            assert!(matches!(&children[2], LayoutNode::Panel { id } if id == "a"));
            assert_eq!(labels, &["second", "third", "first"]);
        } else {
            panic!("expected Tabs");
        }
    }

    // ── add_to_existing_tabs ──

    #[test]
    fn add_tab_to_existing_notebook() {
        let mut layout = tabs(vec![panel("a"), panel("b")], vec!["t1", "t2"]);
        let added = add_to_existing_tabs(&mut layout, "a", "c", "t3", "tab-c");
        assert!(added);
        if let LayoutNode::Tabs {
            children,
            labels,
            tab_ids,
        } = &layout
        {
            assert_eq!(children.len(), 3);
            assert_eq!(labels, &["t1", "t2", "t3"]);
            assert_eq!(tab_ids[2], "tab-c");
        } else {
            panic!("expected Tabs");
        }
    }

    #[test]
    fn add_tab_finds_nested_notebook() {
        // Hsplit [ Tabs[a, b], c ]
        let mut layout = hsplit(vec![
            tabs(vec![panel("a"), panel("b")], vec!["t1", "t2"]),
            panel("c"),
        ]);
        let added = add_to_existing_tabs(&mut layout, "a", "d", "t3", "tab-d");
        assert!(added);
        let ids = panel_ids(&layout);
        assert!(ids.contains(&"d".to_string()));
    }

    #[test]
    fn add_tab_prefers_innermost_tabs_over_root_tabs() {
        let mut layout = tabs(
            vec![
                vsplit(vec![
                    tabs(vec![panel("a"), panel("b")], vec!["inner-a", "inner-b"]),
                    panel("c"),
                ]),
                panel("d"),
            ],
            vec!["outer-left", "outer-right"],
        );

        let added = add_to_existing_tabs(&mut layout, "a", "e", "inner-e", "tab-e");

        assert!(added);
        if let LayoutNode::Tabs {
            children: outer_children,
            labels: outer_labels,
            ..
        } = &layout
        {
            assert_eq!(outer_labels, &["outer-left", "outer-right"]);
            if let LayoutNode::Vsplit { children, .. } = &outer_children[0] {
                if let LayoutNode::Tabs {
                    labels,
                    children,
                    tab_ids,
                } = &children[0]
                {
                    assert_eq!(labels, &["inner-a", "inner-b", "inner-e"]);
                    assert!(matches!(&children[2], LayoutNode::Panel { id } if id == "e"));
                    assert_eq!(tab_ids[2], "tab-e");
                } else {
                    panic!("expected inner tabs");
                }
            } else {
                panic!("expected left vsplit");
            }
        } else {
            panic!("expected outer tabs");
        }
    }

    // ── replace_in_layout ──

    #[test]
    fn replace_panel_in_nested_layout() {
        let layout = vsplit(vec![panel("a"), panel("b")]);
        let result = replace_in_layout(&layout, "a", &|_| hsplit(vec![panel("a"), panel("c")]));
        let ids = panel_ids(&result);
        assert!(ids.contains(&"a".to_string()));
        assert!(ids.contains(&"b".to_string()));
        assert!(ids.contains(&"c".to_string()));
    }

    // ── Label preservation tests ──

    fn get_tab_labels(node: &LayoutNode) -> Option<Vec<String>> {
        if let LayoutNode::Tabs { labels, .. } = node {
            Some(labels.clone())
        } else {
            None
        }
    }

    /// Find first Tabs node in tree and return its labels
    fn find_tab_labels(node: &LayoutNode) -> Option<Vec<String>> {
        if let LayoutNode::Tabs { labels, .. } = node {
            return Some(labels.clone());
        }
        match node {
            LayoutNode::Hsplit { children, .. }
            | LayoutNode::Vsplit { children, .. }
            | LayoutNode::Tabs { children, .. } => {
                for c in children {
                    if let Some(l) = find_tab_labels(c) {
                        return Some(l);
                    }
                }
                None
            }
            _ => None,
        }
    }

    #[test]
    fn rename_tab_then_remove_child_preserves_label() {
        // Tabs [ Vsplit[a, b], c ] with labels ["custom", "tab_c"]
        let mut layout = tabs(
            vec![vsplit(vec![panel("a"), panel("b")]), panel("c")],
            vec!["custom", "tab_c"],
        );
        // Rename first tab
        update_tab_label_in_layout(&mut layout, "a", "renamed");
        assert_eq!(get_tab_labels(&layout).unwrap()[0], "renamed");

        // Remove panel b (inside the vsplit that is the first tab)
        let layout = remove_from_layout(&layout, "b");

        // The first tab should still be "renamed"
        let labels = find_tab_labels(&layout).unwrap();
        assert_eq!(labels[0], "renamed", "label must survive child removal");
        assert_eq!(labels[1], "tab_c");
    }

    #[test]
    fn rename_tab_then_add_child_preserves_label() {
        let mut layout = tabs(vec![panel("a"), panel("b")], vec!["custom_a", "custom_b"]);
        update_tab_label_in_layout(&mut layout, "a", "renamed_a");

        // Add a new tab
        add_to_existing_tabs(&mut layout, "a", "c", "new_tab", "tab-c");

        let labels = get_tab_labels(&layout).unwrap();
        assert_eq!(labels[0], "renamed_a", "renamed label must survive add");
        assert_eq!(labels[1], "custom_b");
        assert_eq!(labels[2], "new_tab");
    }

    #[test]
    fn remove_first_tab_preserves_remaining_labels() {
        let layout = tabs(
            vec![panel("a"), panel("b"), panel("c")],
            vec!["first", "second", "third"],
        );
        let result = remove_from_layout(&layout, "a");
        let labels = get_tab_labels(&result).unwrap();
        assert_eq!(labels, vec!["second", "third"]);
    }

    #[test]
    fn remove_middle_tab_preserves_remaining_labels() {
        let layout = tabs(
            vec![panel("a"), panel("b"), panel("c")],
            vec!["first", "second", "third"],
        );
        let result = remove_from_layout(&layout, "b");
        let labels = get_tab_labels(&result).unwrap();
        assert_eq!(labels, vec!["first", "third"]);
    }

    #[test]
    fn split_inside_tab_preserves_labels() {
        let mut layout = tabs(vec![panel("a"), panel("b")], vec!["custom_a", "custom_b"]);
        // Simulate split: replace panel a with Vsplit[a, c]
        layout = replace_in_layout(&layout, "a", &|_| vsplit(vec![panel("a"), panel("c")]));
        let labels = get_tab_labels(&layout).unwrap();
        assert_eq!(labels[0], "custom_a", "label must survive split");
        assert_eq!(labels[1], "custom_b");
    }

    #[test]
    fn rename_inner_tab_does_not_rename_outer() {
        // Root: Tabs["outer1", "outer2"]
        //   outer1 = Vsplit[panel(a), InnerTabs["inner1", "inner2"]]
        //   outer2 = panel(c)
        // InnerTabs: inner1=panel(b), inner2=panel(d)
        // Rename inner tab "inner2" (via panel d) → should rename inner, NOT outer
        let mut layout = tabs(
            vec![
                vsplit(vec![
                    panel("a"),
                    tabs(vec![panel("b"), panel("d")], vec!["inner1", "inner2"]),
                ]),
                panel("c"),
            ],
            vec!["outer1", "outer2"],
        );
        update_tab_label_in_layout(&mut layout, "d", "renamed_inner");

        // Outer labels unchanged
        let outer = get_tab_labels(&layout).unwrap();
        assert_eq!(outer[0], "outer1", "outer must NOT be renamed");
        assert_eq!(outer[1], "outer2");

        // Inner labels: "inner2" → "renamed_inner"
        if let LayoutNode::Tabs { children, .. } = &layout {
            let inner = find_tab_labels(&children[0]).unwrap();
            assert_eq!(inner[0], "inner1");
            assert_eq!(inner[1], "renamed_inner", "inner tab must be renamed");
        }
    }

    #[test]
    fn rename_tab_by_path_targets_exact_nested_tab() {
        let mut layout = tabs(
            vec![
                vsplit(vec![
                    panel("a"),
                    tabs(vec![panel("b"), panel("d")], vec!["inner1", "inner2"]),
                ]),
                panel("c"),
            ],
            vec!["outer1", "outer2"],
        );

        assert!(update_tab_label_in_layout_by_path(
            &mut layout,
            &[0, 1, 1],
            "renamed_inner"
        ));

        let outer = get_tab_labels(&layout).unwrap();
        assert_eq!(outer[0], "outer1");
        assert_eq!(outer[1], "outer2");

        if let LayoutNode::Tabs { children, .. } = &layout {
            let inner = find_tab_labels(&children[0]).unwrap();
            assert_eq!(inner[0], "inner1");
            assert_eq!(inner[1], "renamed_inner");
        }
    }

    #[test]
    fn add_tab_to_tabs_at_path_targets_exact_root_tabs_node() {
        let mut layout = tabs(
            vec![
                vsplit(vec![
                    tabs(vec![panel("a"), panel("b")], vec!["inner-a", "inner-b"]),
                    panel("c"),
                ]),
                panel("d"),
            ],
            vec!["outer-left", "outer-right"],
        );

        let added = add_to_tabs_at_path(&mut layout, &[], "e", "outer-e", "tab-e");

        assert!(added);
        if let LayoutNode::Tabs {
            children,
            labels,
            tab_ids,
        } = &layout
        {
            assert_eq!(labels, &["outer-left", "outer-right", "outer-e"]);
            assert_eq!(tab_ids[2], "tab-e");
            assert!(matches!(&children[2], LayoutNode::Panel { id } if id == "e"));
            if let LayoutNode::Vsplit { children, .. } = &children[0] {
                if let LayoutNode::Tabs { labels, .. } = &children[0] {
                    assert_eq!(labels, &["inner-a", "inner-b"]);
                } else {
                    panic!("expected nested tabs");
                }
            } else {
                panic!("expected vsplit");
            }
        } else {
            panic!("expected root tabs");
        }
    }
}

// ── Insert sibling in parent split ─────────────────────────────────────────

/// Where to insert the new panel relative to the current one.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum InsertPosition {
    Before,
    After,
}

/// Insert a new panel as a sibling of `panel_id` in the nearest ancestor
/// split (Hsplit or Vsplit).
///
/// If `panel_id` is a direct child of a split, the new panel is inserted
/// before or after it in that split's `children`/`ratios` vectors.
///
/// If `panel_id` lives inside a Tabs node, the function climbs to the Tabs'
/// parent split and inserts relative to the Tabs branch (not inside it).
///
/// Returns `true` if the insertion happened.
pub fn insert_sibling_in_layout(
    layout: &mut LayoutNode,
    panel_id: &str,
    new_panel_id: &str,
    position: InsertPosition,
) -> bool {
    // The strategy: recursively walk the tree. At each split node, check if
    // any DIRECT child contains the target panel_id. If yes, insert next to
    // that child. For Tabs parents, the recursion naturally means we find
    // the split above the Tabs.
    match layout {
        LayoutNode::Panel { .. } => false,

        LayoutNode::Hsplit { children, ratios }
        | LayoutNode::Vsplit { children, ratios } => {
            // Check each direct child: does it contain the target panel?
            if let Some(idx) = children
                .iter()
                .position(|child| subtree_contains_panel(child, panel_id))
            {
                // If the child IS the panel directly, insert here.
                let child_is_panel = matches!(&children[idx], LayoutNode::Panel { id } if id == panel_id);
                // If the child is a Tabs node whose DIRECT children include
                // the target panel (no intermediate split), the panel has no
                // split parent inside the Tabs → climb to this split and
                // insert next to the Tabs branch. When the panel is deeper
                // (Tabs > Split > Panel), we must recurse so the inner split
                // handles the insertion instead of the outer one.
                let child_is_tabs_with_direct_panel = match &children[idx] {
                    LayoutNode::Tabs {
                        children: tabs_children,
                        ..
                    } => tabs_children
                        .iter()
                        .any(|c| matches!(c, LayoutNode::Panel { id } if id == panel_id)),
                    _ => false,
                };

                if child_is_panel || child_is_tabs_with_direct_panel {
                    let insert_idx = match position {
                        InsertPosition::Before => idx,
                        InsertPosition::After => idx + 1,
                    };
                    children.insert(
                        insert_idx,
                        LayoutNode::Panel {
                            id: new_panel_id.to_string(),
                        },
                    );
                    // Redistribute ratios evenly.
                    let n = children.len();
                    *ratios = (0..n).map(|_| 1.0 / n as f64).collect();
                    return true;
                }

                // Child is a nested split or other container — recurse into it.
                return insert_sibling_in_layout(
                    &mut children[idx],
                    panel_id,
                    new_panel_id,
                    position,
                );
            }
            false
        }

        LayoutNode::Tabs { children, .. } => {
            // Don't insert inside Tabs — recurse so the caller (a split) can
            // handle it. This just passes the search through.
            for child in children.iter_mut() {
                if insert_sibling_in_layout(child, panel_id, new_panel_id, position) {
                    return true;
                }
            }
            false
        }
    }
}

/// Check whether `node` (or any descendant) contains a panel with the given id.
fn subtree_contains_panel(node: &LayoutNode, panel_id: &str) -> bool {
    match node {
        LayoutNode::Panel { id } => id == panel_id,
        LayoutNode::Hsplit { children, .. }
        | LayoutNode::Vsplit { children, .. }
        | LayoutNode::Tabs { children, .. } => {
            children.iter().any(|c| subtree_contains_panel(c, panel_id))
        }
    }
}
