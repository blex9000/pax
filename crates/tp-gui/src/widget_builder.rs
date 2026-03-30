use gtk4::prelude::*;
use std::collections::HashMap;

use tp_core::workspace::{LayoutNode, PanelConfig, Workspace};

use crate::backend_factory::panel_type_to_id;
use crate::panel_host::{PanelAction, PanelActionCallback, PanelHost};

// ── Widget helpers ───────────────────────────────────────────────────────────

pub fn add_plus_buttons_recursive(widget: &gtk4::Widget, action_cb: &PanelActionCallback) {
    if let Ok(notebook) = widget.clone().downcast::<gtk4::Notebook>() {
        setup_notebook_menu_widget(&notebook, Some(action_cb.clone()));
    }
    let mut child = widget.first_child();
    while let Some(c) = child {
        add_plus_buttons_recursive(&c, action_cb);
        child = c.next_sibling();
    }
}

pub fn build_tab_label(name: &str, panel_type_id: &str, action_cb: &Option<PanelActionCallback>, child_widget: &gtk4::Widget) -> gtk4::Widget {
    let hbox = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);

    let icon_name = match panel_type_id {
        "terminal" => "utilities-terminal-symbolic",
        "markdown" => "text-x-generic-symbolic",
        "browser" => "web-browser-symbolic",
        _ => "radio-symbolic",
    };
    let type_icon = gtk4::Image::from_icon_name(icon_name);
    type_icon.add_css_class("panel-type-icon");
    hbox.append(&type_icon);

    let stack = gtk4::Stack::new();
    let label = gtk4::Label::new(Some(name));
    stack.add_named(&label, Some("label"));
    let entry = gtk4::Entry::new();
    entry.set_text(name);
    entry.set_width_chars(12);
    stack.add_named(&entry, Some("entry"));
    stack.set_visible_child_name("label");

    {
        let s = stack.clone();
        let e = entry.clone();
        let l = label.clone();
        let gesture = gtk4::GestureClick::new();
        gesture.set_button(1);
        gesture.set_propagation_phase(gtk4::PropagationPhase::Bubble);
        gesture.connect_released(move |g, n_press, _, _| {
            if n_press == 2 {
                e.set_text(&l.text());
                s.set_visible_child_name("entry");
                e.grab_focus();
                g.set_state(gtk4::EventSequenceState::Claimed);
            }
        });
        stack.add_controller(gesture);
    }

    {
        let s = stack.clone();
        let l = label.clone();
        let cb = action_cb.clone();
        let w = child_widget.clone();
        entry.connect_activate(move |entry| {
            let new_name = entry.text().to_string();
            if !new_name.trim().is_empty() {
                l.set_text(&new_name);
                if let Some(ref cb) = cb {
                    find_panel_id_recursive(&w, &|panel_id| {
                        cb(panel_id, PanelAction::Rename(new_name.clone()));
                    });
                }
            }
            s.set_visible_child_name("label");
        });
    }

    {
        let s = stack.clone();
        let key_ctrl = gtk4::EventControllerKey::new();
        key_ctrl.connect_key_pressed(move |_, key, _, _| {
            if key == gtk4::gdk::Key::Escape {
                s.set_visible_child_name("label");
                return gtk4::glib::Propagation::Stop;
            }
            gtk4::glib::Propagation::Proceed
        });
        entry.add_controller(key_ctrl);
    }

    hbox.append(&stack);

    let close_btn = gtk4::Button::new();
    close_btn.set_icon_name("window-close-symbolic");
    close_btn.add_css_class("flat");
    close_btn.add_css_class("circular");
    close_btn.add_css_class("tab-close-btn");
    close_btn.set_tooltip_text(Some("Close tab"));

    let cb = action_cb.clone();
    let widget = child_widget.clone();
    close_btn.connect_clicked(move |_| {
        if let Some(ref cb) = cb {
            find_panel_id_recursive(&widget, &|panel_id| {
                cb(&format!("nb:{}", panel_id), PanelAction::RemoveTab);
            });
        }
    });

    hbox.append(&close_btn);
    hbox.upcast::<gtk4::Widget>()
}

pub fn update_notebook_labels_recursive(
    widget: &gtk4::Widget,
    action_cb: &PanelActionCallback,
    _hosts: &HashMap<String, PanelHost>,
    workspace: &Workspace,
) {
    if let Ok(notebook) = widget.clone().downcast::<gtk4::Notebook>() {
        for i in 0..notebook.n_pages() {
            if let Some(page_widget) = notebook.nth_page(Some(i)) {
                let panel_id_cell = std::cell::RefCell::new(None);
                find_panel_id_recursive(&page_widget, &|pid| {
                    *panel_id_cell.borrow_mut() = Some(pid.to_string());
                });
                let panel_id = panel_id_cell.into_inner();
                if let Some(pid) = panel_id {
                    let label_text = workspace.panel(&pid).map(|p| p.name.clone()).unwrap_or_else(|| format!("Tab {}", i + 1));
                    let type_id = workspace.panel(&pid)
                        .map(|p| panel_type_to_id(&p.effective_type()))
                        .unwrap_or("__empty__");
                    let label = build_tab_label(&label_text, type_id, &Some(action_cb.clone()), &page_widget);
                    notebook.set_tab_label(&page_widget, Some(&label));
                }
            }
        }
    }
    let mut child = widget.first_child();
    while let Some(c) = child {
        update_notebook_labels_recursive(&c, action_cb, _hosts, workspace);
        child = c.next_sibling();
    }
}

fn setup_notebook_menu_widget(notebook: &gtk4::Notebook, action_cb: Option<PanelActionCallback>) {
    let btn = gtk4::Button::new();
    btn.set_icon_name("tab-new-symbolic");
    btn.add_css_class("flat");
    btn.set_margin_end(14);
    btn.set_tooltip_text(Some("Add tab"));

    let nb = notebook.clone();
    let cb = action_cb;
    btn.connect_clicked(move |_| {
        if let Some(ref cb) = cb {
            if let Some(page) = nb.nth_page(nb.current_page()) {
                find_panel_id_recursive(&page, &|panel_id| {
                    cb(&format!("nb:{}", panel_id), PanelAction::AddTabToNotebook);
                });
            }
        }
    });

    notebook.set_action_widget(&btn, gtk4::PackType::End);
}

pub fn find_panel_id_recursive(widget: &gtk4::Widget, callback: &dyn Fn(&str)) {
    if widget.has_css_class("panel-frame") {
        let name = widget.widget_name();
        let name_str = name.as_str();
        if !name_str.is_empty() {
            callback(name_str);
            return;
        }
    }
    let mut child = widget.first_child();
    while let Some(c) = child {
        find_panel_id_recursive(&c, callback);
        child = c.next_sibling();
    }
}

pub fn find_notebook_ancestor(widget: &gtk4::Widget) -> Option<gtk4::Notebook> {
    let mut current = widget.parent();
    for _ in 0..3 {
        let w = current?;
        if let Ok(nb) = w.clone().downcast::<gtk4::Notebook>() {
            return Some(nb);
        }
        current = w.parent();
    }
    None
}

pub fn detach_widget(widget: &gtk4::Widget) {
    if let Some(parent) = widget.parent() {
        if let Some(paned) = parent.downcast_ref::<gtk4::Paned>() {
            let is_start = paned.start_child().map(|w| w == *widget).unwrap_or(false);
            if is_start {
                paned.set_start_child(gtk4::Widget::NONE);
            } else {
                paned.set_end_child(gtk4::Widget::NONE);
            }
        } else if let Some(notebook) = parent.downcast_ref::<gtk4::Notebook>() {
            let page = notebook.page_num(widget);
            notebook.remove_page(page);
        } else if let Some(bx) = parent.downcast_ref::<gtk4::Box>() {
            bx.remove(widget);
        } else if let Some(notebook) = find_notebook_ancestor(widget) {
            let page = notebook.page_num(widget);
            notebook.remove_page(page);
        } else {
            widget.unparent();
        }
    }
}

// ── Layout widget building ───────────────────────────────────────────────────

fn get_panel_type_id(node: &LayoutNode, panels: &[PanelConfig]) -> &'static str {
    if let LayoutNode::Panel { id } = node {
        panels.iter().find(|p| p.id == *id)
            .map(|p| {
                let et = p.effective_type();
                panel_type_to_id(&et)
            })
            .unwrap_or("__empty__")
    } else {
        "terminal"
    }
}

pub fn build_layout_widget(
    node: &LayoutNode,
    hosts: &HashMap<String, PanelHost>,
    panels: &[PanelConfig],
) -> gtk4::Widget {
    build_layout_widget_inner(node, hosts, panels, &None)
}

pub fn build_layout_widget_inner(
    node: &LayoutNode,
    hosts: &HashMap<String, PanelHost>,
    panels: &[PanelConfig],
    action_cb: &Option<PanelActionCallback>,
) -> gtk4::Widget {
    match node {
        LayoutNode::Panel { id } => {
            if let Some(host) = hosts.get(id) {
                host.set_title_visible(true);
                let type_id = get_panel_type_id(node, panels);
                host.set_type_icon(type_id);
                host.widget().clone()
            } else {
                let label = gtk4::Label::new(Some(&format!("Missing panel: {}", id)));
                label.upcast::<gtk4::Widget>()
            }
        }
        LayoutNode::Hsplit { children, ratios } => {
            build_paned(children, ratios, hosts, panels, action_cb, gtk4::Orientation::Horizontal)
        }
        LayoutNode::Vsplit { children, ratios } => {
            build_paned(children, ratios, hosts, panels, action_cb, gtk4::Orientation::Vertical)
        }
        LayoutNode::Tabs { children, labels } => {
            let notebook = gtk4::Notebook::new();
            notebook.set_show_tabs(true);
            notebook.set_scrollable(true);

            for (i, child) in children.iter().enumerate() {
                let child_widget = build_layout_widget_inner(child, hosts, panels, action_cb);
                let label_text = labels.get(i).cloned().unwrap_or_else(|| format!("Tab {}", i + 1));
                let panel_type_id = get_panel_type_id(child, panels);
                let label = build_tab_label(&label_text, panel_type_id, action_cb, &child_widget);
                notebook.append_page(&child_widget, Some(&label));

                if let LayoutNode::Panel { id } = child {
                    if let Some(host) = hosts.get(id) {
                        host.set_title_visible(false);
                    }
                }
            }

            notebook.upcast::<gtk4::Widget>()
        }
    }
}

fn setup_paned_ratio(paned: &gtk4::Paned, ratio: f64, orientation: gtk4::Orientation) {
    use gtk4::glib;
    paned.set_position((ratio * 800.0) as i32);
    let r = ratio;
    let p = paned.clone();
    glib::idle_add_local_once(move || {
        let alloc = p.allocation();
        let total = match orientation {
            gtk4::Orientation::Horizontal => alloc.width(),
            _ => alloc.height(),
        };
        if total > 0 {
            p.set_position((r * total as f64) as i32);
        }
    });
}

fn build_paned(
    children: &[LayoutNode],
    ratios: &[f64],
    hosts: &HashMap<String, PanelHost>,
    panels: &[PanelConfig],
    action_cb: &Option<PanelActionCallback>,
    orientation: gtk4::Orientation,
) -> gtk4::Widget {
    if children.is_empty() {
        return gtk4::Box::new(orientation, 0).upcast::<gtk4::Widget>();
    }
    if children.len() == 1 {
        return build_layout_widget_inner(&children[0], hosts, panels, action_cb);
    }

    let sum: f64 = ratios.iter().take(children.len()).sum();
    let normalized: Vec<f64> = if sum > 0.0 {
        ratios.iter().take(children.len()).map(|r| r / sum).collect()
    } else {
        vec![1.0 / children.len() as f64; children.len()]
    };

    if children.len() == 2 {
        let paned = gtk4::Paned::new(orientation);
        let w1 = build_layout_widget_inner(&children[0], hosts, panels, action_cb);
        let w2 = build_layout_widget_inner(&children[1], hosts, panels, action_cb);
        let c1_fixed = subtree_has_min_size(&children[0], panels);
        let c2_fixed = subtree_has_min_size(&children[1], panels);
        paned.set_start_child(Some(&w1));
        paned.set_end_child(Some(&w2));
        paned.set_shrink_start_child(!c1_fixed);
        paned.set_shrink_end_child(!c2_fixed);
        paned.set_resize_start_child(!c1_fixed || !c2_fixed);
        paned.set_resize_end_child(!c2_fixed || !c1_fixed);
        setup_paned_ratio(&paned, normalized[0], orientation);
        return paned.upcast::<gtk4::Widget>();
    }

    let paned = gtk4::Paned::new(orientation);
    let w1 = build_layout_widget_inner(&children[0], hosts, panels, action_cb);
    let rest_nodes = &children[1..];
    let rest = build_paned(rest_nodes, &ratios[1..], hosts, panels, action_cb, orientation);
    let c1_fixed = subtree_has_min_size(&children[0], panels);
    let rest_fixed = rest_nodes.iter().any(|n| subtree_has_min_size(n, panels));
    paned.set_start_child(Some(&w1));
    paned.set_end_child(Some(&rest));
    paned.set_shrink_start_child(!c1_fixed);
    paned.set_shrink_end_child(!rest_fixed);
    paned.set_resize_start_child(true);
    paned.set_resize_end_child(true);
    setup_paned_ratio(&paned, normalized[0], orientation);
    paned.upcast::<gtk4::Widget>()
}

pub fn apply_min_size(host: &PanelHost, cfg: &PanelConfig) {
    let w = if cfg.min_width > 0 { cfg.min_width as i32 } else { -1 };
    let h = if cfg.min_height > 0 { cfg.min_height as i32 } else { -1 };
    if w > 0 || h > 0 {
        host.widget().set_size_request(w, h);
    }
}

fn subtree_has_min_size(node: &LayoutNode, panels: &[PanelConfig]) -> bool {
    match node {
        LayoutNode::Panel { id } => {
            panels.iter().any(|p| p.id == *id && (p.min_width > 0 || p.min_height > 0))
        }
        LayoutNode::Hsplit { children, .. }
        | LayoutNode::Vsplit { children, .. }
        | LayoutNode::Tabs { children, .. } => {
            children.iter().any(|c| subtree_has_min_size(c, panels))
        }
    }
}

pub fn sync_ratios_recursive(widget: &gtk4::Widget, node: &mut LayoutNode) {
    let is_hsplit = matches!(node, LayoutNode::Hsplit { .. });
    match node {
        LayoutNode::Panel { .. } => {}
        LayoutNode::Hsplit { children, ratios } | LayoutNode::Vsplit { children, ratios } => {
            if children.len() < 2 { return; }
            if let Ok(paned) = widget.clone().downcast::<gtk4::Paned>() {
                let alloc = paned.allocation();
                let total = if paned.orientation() == gtk4::Orientation::Horizontal { alloc.width() } else { alloc.height() };
                if total > 0 {
                    let pos = paned.position();
                    let r1 = pos as f64 / total as f64;
                    let r2 = 1.0 - r1;
                    if children.len() == 2 {
                        if ratios.len() >= 2 { ratios[0] = r1; ratios[1] = r2; }
                        if let Some(w1) = paned.start_child() { sync_ratios_recursive(&w1, &mut children[0]); }
                        if let Some(w2) = paned.end_child() { sync_ratios_recursive(&w2, &mut children[1]); }
                    } else {
                        if !ratios.is_empty() { ratios[0] = r1; }
                        if let Some(w1) = paned.start_child() { sync_ratios_recursive(&w1, &mut children[0]); }
                        if let Some(w2) = paned.end_child() {
                            let rest_children = children[1..].to_vec();
                            let rest_ratios = if ratios.len() > 1 { ratios[1..].to_vec() } else { vec![1.0; rest_children.len()] };
                            let mut rest_node = if is_hsplit {
                                LayoutNode::Hsplit { children: rest_children, ratios: rest_ratios }
                            } else {
                                LayoutNode::Vsplit { children: rest_children, ratios: rest_ratios }
                            };
                            sync_ratios_recursive(&w2, &mut rest_node);
                            match rest_node {
                                LayoutNode::Hsplit { children: rc, ratios: rr } | LayoutNode::Vsplit { children: rc, ratios: rr } => {
                                    for (i, c) in rc.into_iter().enumerate() { children[i + 1] = c; }
                                    for (i, r) in rr.into_iter().enumerate() { if i + 1 < ratios.len() { ratios[i + 1] = r; } }
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
        }
        LayoutNode::Tabs { children, .. } => {
            if let Ok(notebook) = widget.clone().downcast::<gtk4::Notebook>() {
                for (i, child) in children.iter_mut().enumerate() {
                    if let Some(page_widget) = notebook.nth_page(Some(i as u32)) {
                        sync_ratios_recursive(&page_widget, child);
                    }
                }
            }
        }
    }
}
