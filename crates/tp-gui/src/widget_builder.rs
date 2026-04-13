use gtk4::prelude::*;
use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;

use pax_core::workspace::{LayoutNode, PanelConfig, Workspace};

use crate::backend_factory::panel_type_to_id;
use crate::panel_host::{
    PanelAction, PanelActionCallback, PanelHost, COLLAPSED_CHROME_SIZE, COLLAPSED_ICON_SIZE,
    COLLAPSED_PANEL_SIZE, COLLAPSE_SIZE,
};

const PANED_OVERLAY_CLASS: &str = "paned-overlay-shell";
const COLLAPSED_DRAG_STRIP_SIZE: i32 = 4;
const WORKSPACE_TAB_PAGE_SHELL_CLASS: &str = "workspace-tab-page-shell";
const COLLAPSED_PLACEHOLDER_CLASS: &str = "panel-collapsed-placeholder";

fn workspace_tabs_are_root(path: &[usize]) -> bool {
    path.is_empty()
}

fn tab_label_is_root(tab_path: &[usize]) -> bool {
    tab_path.len() <= 1
}

#[derive(Debug, Clone)]
pub struct TabLabelEditState {
    pub tab_id: String,
    pub draft_name: String,
}

pub fn encode_tab_path(path: &[usize]) -> String {
    path.iter()
        .map(|index| index.to_string())
        .collect::<Vec<_>>()
        .join(".")
}

pub fn decode_tab_path(path: &str) -> Option<Vec<usize>> {
    if path.is_empty() {
        return Some(Vec::new());
    }
    path.split('.')
        .map(|part| part.parse::<usize>().ok())
        .collect()
}

pub fn encode_tabs_widget_name(path: &[usize]) -> String {
    format!("pax-tabs:{}", encode_tab_path(path))
}

pub fn decode_tabs_widget_name(widget_name: &str) -> Option<Vec<usize>> {
    decode_tab_path(widget_name.strip_prefix("pax-tabs:")?)
}

// ── Widget helpers ───────────────────────────────────────────────────────────

pub fn add_plus_buttons_recursive(widget: &gtk4::Widget, action_cb: &PanelActionCallback) {
    if let Ok(notebook) = widget.clone().downcast::<gtk4::Notebook>() {
        // Only add "+" to workspace layout notebooks, not internal ones (code editor, etc.)
        if notebook.has_css_class("workspace-tabs") {
            setup_notebook_menu_widget(&notebook, Some(action_cb.clone()));
        }
    }
    let mut child = widget.first_child();
    while let Some(c) = child {
        add_plus_buttons_recursive(&c, action_cb);
        child = c.next_sibling();
    }
}

fn wrap_workspace_tab_page(child: gtk4::Widget) -> gtk4::Widget {
    let shell = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    shell.add_css_class(WORKSPACE_TAB_PAGE_SHELL_CLASS);
    shell.set_hexpand(true);
    shell.set_vexpand(true);
    shell.append(&child);
    shell.upcast()
}

pub fn build_tab_label(
    name: &str,
    panel_type_id: &str,
    action_cb: &Option<PanelActionCallback>,
    child_widget: &gtk4::Widget,
    edit_state: Option<&TabLabelEditState>,
    tab_id: &str,
    tab_path: &[usize],
) -> gtk4::Widget {
    let is_root_label = tab_label_is_root(tab_path);
    let is_layout = panel_type_id == "__layout__";
    let hbox = gtk4::Box::new(
        gtk4::Orientation::Horizontal,
        if is_root_label { 4 } else { 3 },
    );
    hbox.set_halign(gtk4::Align::Start);
    hbox.add_css_class("workspace-tab-label");
    hbox.add_css_class(if is_root_label {
        "workspace-tab-label-root"
    } else {
        "workspace-tab-label-nested"
    });
    hbox.add_css_class(if is_layout {
        "workspace-tab-label-layout"
    } else {
        "workspace-tab-label-panel"
    });
    let tab_path_key = encode_tab_path(tab_path);
    hbox.set_widget_name(&encode_tab_label_metadata(tab_id, &tab_path_key, is_layout));
    let active_edit = edit_state.filter(|state| state.tab_id == tab_id).cloned();
    let tab_id_owned = tab_id.to_string();

    let icon_name = if is_layout {
        "radio-symbolic"
    } else {
        match panel_type_id {
            "terminal" => "utilities-terminal-symbolic",
            "markdown" => "text-x-generic-symbolic",
            "code_editor" => "accessories-text-editor-symbolic",
            _ => "radio-symbolic",
        }
    };
    let type_icon = gtk4::Image::from_icon_name(icon_name);
    type_icon.add_css_class("panel-type-icon");
    type_icon.add_css_class("workspace-tab-type-icon");
    type_icon.set_margin_start(8);
    hbox.append(&type_icon);

    let stack = gtk4::Stack::new();
    stack.set_halign(gtk4::Align::Start);
    let label = gtk4::Label::new(Some(name));
    label.add_css_class("workspace-tab-text");
    label.set_halign(gtk4::Align::Start);
    label.set_xalign(0.0);
    stack.add_named(&label, Some("label"));

    let edit_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
    let suppress_entry_changed = Rc::new(Cell::new(false));
    let move_left_btn = gtk4::Button::new();
    move_left_btn.set_icon_name("go-previous-symbolic");
    move_left_btn.set_tooltip_text(Some("Move tab left"));
    move_left_btn.add_css_class("flat");
    move_left_btn.add_css_class("circular");
    move_left_btn.set_focus_on_click(false);
    move_left_btn.set_focusable(false);
    edit_box.append(&move_left_btn);

    let entry = gtk4::Entry::new();
    entry.set_text(name);
    entry.set_width_chars(12);
    edit_box.append(&entry);

    let move_right_btn = gtk4::Button::new();
    move_right_btn.set_icon_name("go-next-symbolic");
    move_right_btn.set_tooltip_text(Some("Move tab right"));
    move_right_btn.add_css_class("flat");
    move_right_btn.add_css_class("circular");
    move_right_btn.set_focus_on_click(false);
    move_right_btn.set_focusable(false);
    edit_box.append(&move_right_btn);

    stack.add_named(&edit_box, Some("edit"));
    stack.set_visible_child_name("label");
    let update_move_buttons: Rc<dyn Fn()> = Rc::new({
        let child_widget = child_widget.clone();
        let move_left_btn = move_left_btn.clone();
        let move_right_btn = move_right_btn.clone();
        move || {
            let Some(notebook) = find_notebook_ancestor(&child_widget) else {
                move_left_btn.set_sensitive(false);
                move_right_btn.set_sensitive(false);
                return;
            };
            let Some(position) = notebook.page_num(&child_widget) else {
                move_left_btn.set_sensitive(false);
                move_right_btn.set_sensitive(false);
                return;
            };
            move_left_btn.set_sensitive(position > 0);
            move_right_btn.set_sensitive(position + 1 < workspace_tab_real_page_count(&notebook));
        }
    });

    {
        let cb = action_cb.clone();
        let tab_id = tab_id_owned.clone();
        let suppress_entry_changed = suppress_entry_changed.clone();
        entry.connect_changed(move |entry| {
            if suppress_entry_changed.get() {
                return;
            }
            if let Some(ref cb) = cb {
                cb(
                    &format!("nb-tab:{}", tab_id),
                    PanelAction::UpdateTabDraft {
                        tab_id: tab_id.clone(),
                        name: entry.text().to_string(),
                    },
                );
            }
        });
    }

    {
        let cb = action_cb.clone();
        let tab_id = tab_id_owned.clone();
        entry.connect_activate(move |_| {
            if let Some(ref cb) = cb {
                cb(
                    &format!("nb-tab:{}", tab_id),
                    PanelAction::CommitTabEdit {
                        tab_id: tab_id.clone(),
                    },
                );
            }
        });
    }

    {
        let cb = action_cb.clone();
        let tab_id = tab_id_owned.clone();
        let key_ctrl = gtk4::EventControllerKey::new();
        key_ctrl.connect_key_pressed(move |_, key, _, _| {
            if key == gtk4::gdk::Key::Escape {
                if let Some(ref cb) = cb {
                    cb(
                        &format!("nb-tab:{}", tab_id),
                        PanelAction::CancelTabEdit {
                            tab_id: tab_id.clone(),
                        },
                    );
                }
                return gtk4::glib::Propagation::Stop;
            }
            gtk4::glib::Propagation::Proceed
        });
        entry.add_controller(key_ctrl);
    }

    {
        let cb = action_cb.clone();
        let tab_id = tab_id_owned.clone();
        let child_widget = child_widget.clone();
        let update_move_buttons = update_move_buttons.clone();
        move_left_btn.connect_clicked(move |_| {
            if let Some(ref cb) = cb {
                preview_move_workspace_tab(&child_widget, -1);
                cb(
                    &format!("nb-tab:{}", tab_id),
                    PanelAction::PreviewTabMove {
                        tab_id: tab_id.clone(),
                        offset: -1,
                    },
                );
                update_move_buttons();
            }
        });
    }

    {
        let cb = action_cb.clone();
        let tab_id = tab_id_owned.clone();
        let child_widget = child_widget.clone();
        let update_move_buttons = update_move_buttons.clone();
        move_right_btn.connect_clicked(move |_| {
            if let Some(ref cb) = cb {
                preview_move_workspace_tab(&child_widget, 1);
                cb(
                    &format!("nb-tab:{}", tab_id),
                    PanelAction::PreviewTabMove {
                        tab_id: tab_id.clone(),
                        offset: 1,
                    },
                );
                update_move_buttons();
            }
        });
    }

    if let Some(active_edit) = active_edit {
        label.set_text(&active_edit.draft_name);
        suppress_entry_changed.set(true);
        entry.set_text(&active_edit.draft_name);
        suppress_entry_changed.set(false);
        stack.set_visible_child_name("edit");
        let entry = entry.clone();
        let update_move_buttons = update_move_buttons.clone();
        gtk4::glib::idle_add_local_once(move || {
            update_move_buttons();
            entry.grab_focus();
            entry.set_position(-1);
        });
    }

    hbox.append(&stack);

    let close_btn = gtk4::Button::new();
    close_btn.set_icon_name("window-close-symbolic");
    close_btn.add_css_class("flat");
    close_btn.add_css_class("circular");
    close_btn.add_css_class("tab-close-btn");
    close_btn.add_css_class("workspace-tab-close-btn");
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

#[cfg(test)]
mod tests {
    use super::*;

    fn find_first_image(widget: &gtk4::Widget) -> Option<gtk4::Image> {
        if let Ok(image) = widget.clone().downcast::<gtk4::Image>() {
            return Some(image);
        }
        let mut child = widget.first_child();
        while let Some(current) = child {
            if let Some(image) = find_first_image(&current) {
                return Some(image);
            }
            child = current.next_sibling();
        }
        None
    }

    fn find_first_label(widget: &gtk4::Widget) -> Option<gtk4::Label> {
        if let Ok(label) = widget.clone().downcast::<gtk4::Label>() {
            return Some(label);
        }
        let mut child = widget.first_child();
        while let Some(current) = child {
            if let Some(label) = find_first_label(&current) {
                return Some(label);
            }
            child = current.next_sibling();
        }
        None
    }

    #[test]
    fn layout_tabs_use_radio_fallback_icon() {
        if gtk4::init().is_err() {
            return;
        }

        let child = gtk4::Box::new(gtk4::Orientation::Vertical, 0).upcast::<gtk4::Widget>();
        let label = build_tab_label("layout", "__layout__", &None, &child, None, "tab-1", &[0]);
        let image = find_first_image(&label).expect("layout tab should include fallback icon");
        let text = find_first_label(&label).expect("layout tab should include text label");

        assert_eq!(image.icon_name().as_deref(), Some("radio-symbolic"));
        assert_eq!(image.margin_start(), 8);
        assert_eq!(text.xalign(), 0.0);
    }

    #[test]
    fn notebook_add_button_uses_plus_label() {
        if gtk4::init().is_err() {
            return;
        }

        let notebook = gtk4::Notebook::new();
        notebook.add_css_class("workspace-tabs");
        let child = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        notebook.append_page(&child, Some(&gtk4::Label::new(Some("Tab"))));

        setup_notebook_menu_widget(&notebook, None);

        let add_page = notebook
            .nth_page(Some(notebook.n_pages() - 1))
            .expect("workspace notebook should contain add page");
        assert!(is_workspace_tab_add_page(&add_page));
        let add_wrap = notebook
            .tab_label(&add_page)
            .expect("workspace add page should have tab label");
        assert_eq!(add_wrap.halign(), gtk4::Align::Center);

        let add_label =
            find_first_label(add_wrap.upcast_ref()).expect("workspace add widget should contain label");
        assert_eq!(add_label.text().as_str(), "+");
    }

    #[test]
    fn workspace_tab_real_page_count_ignores_add_page() {
        if gtk4::init().is_err() {
            return;
        }

        let notebook = gtk4::Notebook::new();
        notebook.add_css_class("workspace-tabs");
        let child = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        notebook.append_page(&child, Some(&gtk4::Label::new(Some("Tab"))));

        setup_notebook_menu_widget(&notebook, None);

        assert_eq!(notebook.n_pages(), 2);
        assert_eq!(workspace_tab_real_page_count(&notebook), 1);
    }

    #[test]
    fn unwrap_layout_shell_skips_workspace_tab_page_shell() {
        if gtk4::init().is_err() {
            return;
        }

        let inner = gtk4::Box::new(gtk4::Orientation::Vertical, 0).upcast::<gtk4::Widget>();
        let shell = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        shell.add_css_class(WORKSPACE_TAB_PAGE_SHELL_CLASS);
        shell.append(&inner);

        let unwrapped = unwrap_layout_shell(shell.upcast_ref());
        assert_eq!(unwrapped, inner);
    }

}

fn preview_move_workspace_tab(child_widget: &gtk4::Widget, step: i32) -> bool {
    let Some(notebook) = find_notebook_ancestor(child_widget) else {
        return false;
    };
    let Some(position) = notebook.page_num(child_widget) else {
        return false;
    };
    let target = position as i32 + step;
    if !(0..workspace_tab_real_page_count(&notebook) as i32).contains(&target) {
        return false;
    }
    notebook.reorder_child(child_widget, Some(target as u32));
    notebook.set_current_page(Some(target as u32));
    true
}

/// Rebuild tab labels on existing Notebooks from the layout model.
/// Walks the widget tree and layout tree in parallel so each Notebook
/// gets labels from its corresponding Tabs node — no ambiguous matching.
pub fn update_notebook_labels_recursive(
    widget: &gtk4::Widget,
    action_cb: &PanelActionCallback,
    _hosts: &HashMap<String, PanelHost>,
    workspace: &Workspace,
    edit_state: Option<&TabLabelEditState>,
) {
    update_labels_with_layout(
        widget,
        action_cb,
        &workspace.layout,
        &workspace.panels,
        edit_state,
        &[],
    );
}

fn update_labels_with_layout(
    widget: &gtk4::Widget,
    action_cb: &PanelActionCallback,
    layout_node: &LayoutNode,
    panels: &[PanelConfig],
    edit_state: Option<&TabLabelEditState>,
    path: &[usize],
) {
    let widget = unwrap_layout_shell(widget);

    if let Ok(notebook) = widget.clone().downcast::<gtk4::Notebook>() {
        if let LayoutNode::Tabs {
            children,
            labels,
            tab_ids,
        } = layout_node
        {
            for i in 0..children.len() as u32 {
                if let Some(page_widget) = notebook.nth_page(Some(i)) {
                    let mut child_path = path.to_vec();
                    child_path.push(i as usize);
                    // Label: always from the model
                    let label_text = labels
                        .get(i as usize)
                        .cloned()
                        .unwrap_or_else(|| format!("Tab {}", i + 1));

                    // Type icon: check if it's a single panel or a layout
                    let type_id = if let Some(LayoutNode::Panel { id }) = children.get(i as usize) {
                        panels
                            .iter()
                            .find(|p| p.id == *id)
                            .map(|p| panel_type_to_id(&p.effective_type()))
                            .unwrap_or("__empty__")
                    } else {
                        "__layout__"
                    };

                    tracing::debug!(
                        "update_labels_with_layout: tab {}: label='{}' type='{}'",
                        i,
                        label_text,
                        type_id
                    );
                    let tab_id = tab_ids
                        .get(i as usize)
                        .cloned()
                        .unwrap_or_else(pax_core::workspace::new_tab_id);
                    let label = build_tab_label(
                        &label_text,
                        type_id,
                        &Some(action_cb.clone()),
                        &page_widget,
                        edit_state,
                        &tab_id,
                        &child_path,
                    );
                    notebook.set_tab_label(&page_widget, Some(&label));

                    // Recurse into child layout nodes and page widgets
                    if let Some(child_node) = children.get(i as usize) {
                        update_labels_with_layout(
                            &page_widget,
                            action_cb,
                            child_node,
                            panels,
                            edit_state,
                            &child_path,
                        );
                    }
                }
            }
            return; // Don't walk GTK children — already handled above
        }
    }

    // Not a Notebook or no matching Tabs node — recurse into GTK children
    // For split nodes, recurse into layout children in parallel
    match layout_node {
        LayoutNode::Hsplit { children, .. } | LayoutNode::Vsplit { children, .. } => {
            // Paned has start_child and end_child
            if let Ok(paned) = widget.clone().downcast::<gtk4::Paned>() {
                if let Some(w) = paned.start_child() {
                    let w = unwrap_layout_shell(&w);
                    if let Some(c) = children.first() {
                        let mut child_path = path.to_vec();
                        child_path.push(0);
                        update_labels_with_layout(
                            &w,
                            action_cb,
                            c,
                            panels,
                            edit_state,
                            &child_path,
                        );
                    }
                }
                if let Some(w) = paned.end_child() {
                    let w = unwrap_layout_shell(&w);
                    // For 2 children: second child. For 3+: rest is a nested Paned.
                    if children.len() == 2 {
                        if let Some(c) = children.get(1) {
                            let mut child_path = path.to_vec();
                            child_path.push(1);
                            update_labels_with_layout(
                                &w,
                                action_cb,
                                c,
                                panels,
                                edit_state,
                                &child_path,
                            );
                        }
                    }
                    // For 3+ children the rest_node is built recursively in build_paned,
                    // but we don't have a matching layout node. Skip deep recursion here.
                }
            }
        }
        LayoutNode::Panel { .. } => {} // Leaf — nothing to recurse
        _ => {}                        // Tabs handled above
    }
}

fn setup_notebook_menu_widget(notebook: &gtk4::Notebook, action_cb: Option<PanelActionCallback>) {
    // Add tab button only — collapse is handled by drag resize
    remove_existing_workspace_tab_add_page(notebook);
    let add_page = build_workspace_tab_add_page(notebook, action_cb.clone());
    let add_page_widget = add_page.upcast::<gtk4::Widget>();
    let add_label = build_workspace_tab_add_label(notebook, action_cb.clone());
    notebook.append_page(&add_page_widget, Some(&add_label));
    notebook.set_tab_reorderable(&add_page_widget, false);
    notebook.set_tab_detachable(&add_page_widget, false);

    if notebook.has_css_class("pax-tab-edit-gesture") {
        return;
    }
    notebook.add_css_class("pax-tab-edit-gesture");

    let nb = notebook.clone();
    let cb = action_cb;
    let gesture = gtk4::GestureClick::new();
    gesture.set_button(1);
    gesture.set_propagation_phase(gtk4::PropagationPhase::Bubble);
    gesture.connect_released(move |g, n_press, x, y| {
        if n_press != 2 {
            return;
        }
        let Some(picked) = nb.pick(x, y, gtk4::PickFlags::DEFAULT) else {
            return;
        };
        for i in 0..workspace_tab_real_page_count(&nb) {
            let Some(page_widget) = nb.nth_page(Some(i)) else {
                continue;
            };
            let Some(tab_label) = nb.tab_label(&page_widget) else {
                continue;
            };
            if !widget_is_same_or_descendant(&picked, &tab_label) {
                continue;
            }
            let Some((tab_id, tab_path, is_layout)) =
                decode_tab_label_metadata(&tab_label.widget_name())
            else {
                continue;
            };
            let panel_id = find_first_panel_id(&page_widget).unwrap_or_default();
            let draft_name =
                find_tab_label_text(&tab_label).unwrap_or_else(|| format!("Tab {}", i + 1));
            activate_tab_label_editor(&tab_label, &page_widget, &draft_name);
            if let Some(ref cb) = cb {
                cb(
                    &format!("nb:{}", panel_id),
                    PanelAction::BeginTabEdit {
                        tab_id,
                        tab_path,
                        panel_id,
                        name: draft_name,
                        is_layout,
                    },
                );
            }
            g.set_state(gtk4::EventSequenceState::Claimed);
            break;
        }
    });
    notebook.add_controller(gesture);
}

fn build_workspace_tab_add_label(
    notebook: &gtk4::Notebook,
    action_cb: Option<PanelActionCallback>,
) -> gtk4::Box {
    let add_label = gtk4::Label::new(Some("+"));
    add_label.add_css_class("workspace-tab-add-label");
    add_label.set_halign(gtk4::Align::Center);
    add_label.set_valign(gtk4::Align::Center);
    let add_wrap = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
    add_wrap.add_css_class("workspace-tab-add-wrap");
    add_wrap.set_halign(gtk4::Align::Center);
    add_wrap.set_valign(gtk4::Align::Center);
    add_wrap.append(&add_label);
    let nb = notebook.clone();
    let cb = action_cb.clone();
    let gesture = gtk4::GestureClick::new();
    gesture.set_button(1);
    gesture.set_propagation_phase(gtk4::PropagationPhase::Capture);
    gesture.connect_pressed(move |g, n_press, _, _| {
        if n_press != 1 {
            return;
        }
        g.set_state(gtk4::EventSequenceState::Claimed);
        let tab_path = decode_tabs_widget_name(&nb.widget_name());
        let cb = cb.clone();
        gtk4::glib::idle_add_local_once(move || {
            if let (Some(cb), Some(tab_path)) = (cb.as_ref(), tab_path.as_ref()) {
                cb(
                    &format!("nb-tabs:{}", encode_tab_path(tab_path)),
                    PanelAction::AddTabToNotebook,
                );
            }
        });
    });
    add_wrap.add_controller(gesture);
    add_wrap
}

fn build_workspace_tab_add_page(
    _notebook: &gtk4::Notebook,
    _action_cb: Option<PanelActionCallback>,
) -> gtk4::Box {
    let page = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    page.add_css_class("workspace-tab-add-page");
    page.set_can_focus(false);
    page.set_focusable(false);
    page.set_sensitive(false);
    page
}

fn is_workspace_tab_add_page(widget: &gtk4::Widget) -> bool {
    widget.has_css_class("workspace-tab-add-page")
}

fn workspace_tab_real_page_count(notebook: &gtk4::Notebook) -> u32 {
    let count = notebook.n_pages();
    if count == 0 {
        return 0;
    }
    if let Some(last_page) = notebook.nth_page(Some(count - 1)) {
        if is_workspace_tab_add_page(&last_page) {
            return count - 1;
        }
    }
    count
}

fn remove_existing_workspace_tab_add_page(notebook: &gtk4::Notebook) {
    let count = notebook.n_pages();
    if count == 0 {
        return;
    }
    if let Some(last_page) = notebook.nth_page(Some(count - 1)) {
        if is_workspace_tab_add_page(&last_page) {
            notebook.remove_page(Some(count - 1));
        }
    }
}

fn encode_tab_label_metadata(tab_id: &str, tab_path: &str, is_layout: bool) -> String {
    format!(
        "pax-tab:{}:{}:{}",
        if is_layout { "layout" } else { "panel" },
        tab_id,
        tab_path
    )
}

pub(crate) fn decode_tab_label_metadata(widget_name: &str) -> Option<(String, Vec<usize>, bool)> {
    let rest = widget_name.strip_prefix("pax-tab:")?;
    let (kind, rest) = rest.split_once(':')?;
    let (tab_id, path) = rest.split_once(':')?;
    let is_layout = match kind {
        "layout" => true,
        "panel" => false,
        _ => return None,
    };
    let tab_path = if path.is_empty() {
        Vec::new()
    } else {
        path.split('.')
            .map(|part| part.parse::<usize>().ok())
            .collect::<Option<Vec<_>>>()?
    };
    Some((tab_id.to_string(), tab_path, is_layout))
}

fn activate_tab_label_editor(
    tab_label: &gtk4::Widget,
    page_widget: &gtk4::Widget,
    draft_name: &str,
) {
    let Some(stack) = find_descendant_stack(tab_label) else {
        return;
    };
    let Some(entry) = find_descendant_entry(tab_label) else {
        return;
    };
    if entry.text().as_str() != draft_name {
        entry.set_text(draft_name);
    }
    update_tab_move_buttons(tab_label, page_widget);
    stack.set_visible_child_name("edit");
    gtk4::glib::idle_add_local_once(move || {
        entry.grab_focus();
        entry.set_position(-1);
    });
}

pub fn find_active_tab_editor_recursive(widget: &gtk4::Widget) -> Option<gtk4::Widget> {
    if let Ok(stack) = widget.clone().downcast::<gtk4::Stack>() {
        if stack.visible_child_name().as_deref() == Some("edit") {
            return stack.parent();
        }
    }
    let mut child = widget.first_child();
    while let Some(c) = child {
        if let Some(found) = find_active_tab_editor_recursive(&c) {
            return Some(found);
        }
        child = c.next_sibling();
    }
    None
}

fn update_tab_move_buttons(tab_label: &gtk4::Widget, page_widget: &gtk4::Widget) {
    let Some(notebook) = find_notebook_ancestor(page_widget) else {
        return;
    };
    let Some(position) = notebook.page_num(page_widget) else {
        return;
    };
    let buttons = collect_descendant_buttons(tab_label);
    if let Some(button) = buttons.first() {
        button.set_sensitive(position > 0);
    }
    if let Some(button) = buttons.get(1) {
        button.set_sensitive(position + 1 < workspace_tab_real_page_count(&notebook));
    }
}

fn find_tab_label_text(widget: &gtk4::Widget) -> Option<String> {
    find_descendant_label(widget).map(|label| label.text().to_string())
}

fn find_descendant_stack(widget: &gtk4::Widget) -> Option<gtk4::Stack> {
    if let Ok(stack) = widget.clone().downcast::<gtk4::Stack>() {
        return Some(stack);
    }
    let mut child = widget.first_child();
    while let Some(c) = child {
        if let Some(stack) = find_descendant_stack(&c) {
            return Some(stack);
        }
        child = c.next_sibling();
    }
    None
}

fn find_descendant_entry(widget: &gtk4::Widget) -> Option<gtk4::Entry> {
    if let Ok(entry) = widget.clone().downcast::<gtk4::Entry>() {
        return Some(entry);
    }
    let mut child = widget.first_child();
    while let Some(c) = child {
        if let Some(entry) = find_descendant_entry(&c) {
            return Some(entry);
        }
        child = c.next_sibling();
    }
    None
}

fn find_descendant_label(widget: &gtk4::Widget) -> Option<gtk4::Label> {
    if let Ok(label) = widget.clone().downcast::<gtk4::Label>() {
        return Some(label);
    }
    let mut child = widget.first_child();
    while let Some(c) = child {
        if let Some(label) = find_descendant_label(&c) {
            return Some(label);
        }
        child = c.next_sibling();
    }
    None
}

fn collect_descendant_buttons(widget: &gtk4::Widget) -> Vec<gtk4::Button> {
    let mut buttons = Vec::new();
    collect_descendant_buttons_inner(widget, &mut buttons);
    buttons
}

fn collect_descendant_buttons_inner(widget: &gtk4::Widget, out: &mut Vec<gtk4::Button>) {
    if let Ok(button) = widget.clone().downcast::<gtk4::Button>() {
        out.push(button);
    }
    let mut child = widget.first_child();
    while let Some(c) = child {
        collect_descendant_buttons_inner(&c, out);
        child = c.next_sibling();
    }
}

fn widget_is_same_or_descendant(widget: &gtk4::Widget, ancestor: &gtk4::Widget) -> bool {
    let mut current = Some(widget.clone());
    while let Some(w) = current {
        if w == *ancestor {
            return true;
        }
        current = w.parent();
    }
    false
}

/// Find the first panel ID inside a widget tree.
pub fn find_first_panel_id(widget: &gtk4::Widget) -> Option<String> {
    if widget.has_css_class("panel-frame") {
        let name = widget.widget_name();
        let name_str = name.as_str();
        if !name_str.is_empty() {
            return Some(name_str.to_string());
        }
    }
    let mut child = widget.first_child();
    while let Some(c) = child {
        if let Some(id) = find_first_panel_id(&c) {
            return Some(id);
        }
        child = c.next_sibling();
    }
    None
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
    for _ in 0..10 {
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
        panels
            .iter()
            .find(|p| p.id == *id)
            .map(|p| {
                let et = p.effective_type();
                panel_type_to_id(&et)
            })
            .unwrap_or("__empty__")
    } else {
        // Layout node (hsplit/vsplit) — not a single panel type
        "__layout__"
    }
}

pub fn build_layout_widget(
    node: &LayoutNode,
    hosts: &HashMap<String, PanelHost>,
    panels: &[PanelConfig],
    edit_state: Option<&TabLabelEditState>,
) -> gtk4::Widget {
    build_layout_widget_inner(node, hosts, panels, &None, edit_state, &[])
}

pub fn build_layout_widget_inner(
    node: &LayoutNode,
    hosts: &HashMap<String, PanelHost>,
    panels: &[PanelConfig],
    action_cb: &Option<PanelActionCallback>,
    edit_state: Option<&TabLabelEditState>,
    path: &[usize],
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
        LayoutNode::Hsplit { children, ratios } => build_paned(
            children,
            ratios,
            hosts,
            panels,
            action_cb,
            edit_state,
            path,
            gtk4::Orientation::Horizontal,
        ),
        LayoutNode::Vsplit { children, ratios } => build_paned(
            children,
            ratios,
            hosts,
            panels,
            action_cb,
            edit_state,
            path,
            gtk4::Orientation::Vertical,
        ),
        LayoutNode::Tabs {
            children,
            labels,
            tab_ids,
        } => {
            let notebook = gtk4::Notebook::new();
            notebook.set_show_tabs(true);
            notebook.set_scrollable(true);
            notebook.add_css_class("workspace-tabs");
            notebook.add_css_class(if workspace_tabs_are_root(path) {
                "workspace-tabs-root"
            } else {
                "workspace-tabs-nested"
            });
            notebook.set_widget_name(&encode_tabs_widget_name(path));

            for (i, child) in children.iter().enumerate() {
                let mut child_path = path.to_vec();
                child_path.push(i);
                let child_widget = build_layout_widget_inner(
                    child,
                    hosts,
                    panels,
                    action_cb,
                    edit_state,
                    &child_path,
                );
                let page_widget = wrap_workspace_tab_page(child_widget);
                let label_text = labels
                    .get(i)
                    .cloned()
                    .unwrap_or_else(|| format!("Tab {}", i + 1));
                tracing::debug!(
                    "build Notebook tab {}: label='{}' from model",
                    i,
                    label_text
                );
                let panel_type_id = get_panel_type_id(child, panels);
                let tab_id = tab_ids
                    .get(i)
                    .cloned()
                    .unwrap_or_else(pax_core::workspace::new_tab_id);
                let label = build_tab_label(
                    &label_text,
                    panel_type_id,
                    action_cb,
                    &page_widget,
                    edit_state,
                    &tab_id,
                    &child_path,
                );
                notebook.append_page(&page_widget, Some(&label));

                // Panels inside tabs keep their title bar visible
                // (includes collapse button at top-left)
            }

            // Click on notebook tab area when collapsed → expand to 50%
            {
                let nb = notebook.clone();
                let gesture = gtk4::GestureClick::new();
                gesture.set_button(1);
                gesture.connect_released(move |_, _, _, _| {
                    let mut widget = nb.parent();
                    while let Some(w) = widget {
                        if let Some(paned) = w.downcast_ref::<gtk4::Paned>() {
                            let total = if paned.orientation() == gtk4::Orientation::Horizontal {
                                paned.allocation().width()
                            } else {
                                paned.allocation().height()
                            };
                            let is_start = paned
                                .start_child()
                                .map(|c| {
                                    nb.is_ancestor(&c) || c.eq(nb.upcast_ref::<gtk4::Widget>())
                                })
                                .unwrap_or(false);
                            let my_size = if is_start {
                                paned.position()
                            } else {
                                total - paned.position()
                            };
                            if my_size <= 60 {
                                paned.set_position(total / 2);
                            }
                            break;
                        }
                        widget = w.parent();
                    }
                });
                notebook.add_controller(gesture);
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
    edit_state: Option<&TabLabelEditState>,
    path: &[usize],
    orientation: gtk4::Orientation,
) -> gtk4::Widget {
    if children.is_empty() {
        return gtk4::Box::new(orientation, 0).upcast::<gtk4::Widget>();
    }
    if children.len() == 1 {
        let mut child_path = path.to_vec();
        child_path.push(0);
        return build_layout_widget_inner(
            &children[0],
            hosts,
            panels,
            action_cb,
            edit_state,
            &child_path,
        );
    }

    let sum: f64 = ratios.iter().take(children.len()).sum();
    let normalized: Vec<f64> = if sum > 0.0 {
        ratios
            .iter()
            .take(children.len())
            .map(|r| r / sum)
            .collect()
    } else {
        vec![1.0 / children.len() as f64; children.len()]
    };

    // Helper: wrap non-PanelHost children (nested Paned/Notebook) for drag-collapse
    let maybe_wrap = |w: gtk4::Widget| -> gtk4::Widget {
        if hosts.contains_key(w.widget_name().as_str()) {
            w // Direct PanelHost — has its own collapse mechanism
        } else {
            wrap_layout_for_collapse(w)
        }
    };

    if children.len() == 2 {
        let paned = gtk4::Paned::new(orientation);
        let mut path1 = path.to_vec();
        path1.push(0);
        let w1 = maybe_wrap(build_layout_widget_inner(
            &children[0],
            hosts,
            panels,
            action_cb,
            edit_state,
            &path1,
        ));
        let mut path2 = path.to_vec();
        path2.push(1);
        let w2 = maybe_wrap(build_layout_widget_inner(
            &children[1],
            hosts,
            panels,
            action_cb,
            edit_state,
            &path2,
        ));
        let c1_fixed = subtree_has_min_size(&children[0], panels);
        let c2_fixed = subtree_has_min_size(&children[1], panels);
        paned.set_start_child(Some(&w1));
        paned.set_end_child(Some(&w2));
        paned.set_shrink_start_child(true);
        paned.set_shrink_end_child(true);
        paned.set_resize_start_child(true);
        paned.set_resize_end_child(true);
        setup_paned_ratio(&paned, normalized[0], orientation);
        return wrap_paned_with_collapse_overlay(&paned, hosts);
    }

    let paned = gtk4::Paned::new(orientation);
    let mut path1 = path.to_vec();
    path1.push(0);
    let w1 = maybe_wrap(build_layout_widget_inner(
        &children[0],
        hosts,
        panels,
        action_cb,
        edit_state,
        &path1,
    ));
    let rest_nodes = &children[1..];
    let mut rest_path = path.to_vec();
    rest_path.push(1);
    let rest = maybe_wrap(build_paned(
        rest_nodes,
        &ratios[1..],
        hosts,
        panels,
        action_cb,
        edit_state,
        &rest_path,
        orientation,
    ));
    let c1_fixed = subtree_has_min_size(&children[0], panels);
    let rest_fixed = rest_nodes.iter().any(|n| subtree_has_min_size(n, panels));
    paned.set_start_child(Some(&w1));
    paned.set_end_child(Some(&rest));
    paned.set_shrink_start_child(true);
    paned.set_shrink_end_child(true);
    paned.set_resize_start_child(true);
    paned.set_resize_end_child(true);
    setup_paned_ratio(&paned, normalized[0], orientation);
    wrap_paned_with_collapse_overlay(&paned, hosts)
}

/// CSS class used to identify collapse wrapper boxes around nested layouts.
const COLLAPSE_WRAPPER_CLASS: &str = "paned-collapse-wrapper";

/// Recursively find the first PanelHost inside a widget subtree.
fn find_panel_host_in<'a>(
    widget: &gtk4::Widget,
    hosts: &'a HashMap<String, PanelHost>,
) -> Option<&'a PanelHost> {
    let name = widget.widget_name();
    if let Some(host) = hosts.get(name.as_str()) {
        return Some(host);
    }
    let mut child = widget.first_child();
    while let Some(c) = child {
        if let Some(h) = find_panel_host_in(&c, hosts) {
            return Some(h);
        }
        child = c.next_sibling();
    }
    None
}

/// Wrap a nested layout widget (Paned/Notebook, not a direct PanelHost) in a Box
/// with a collapsed_view overlay for drag-collapse support on the parent Paned.
fn wrap_layout_for_collapse(child: gtk4::Widget) -> gtk4::Widget {
    let wrapper = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    wrapper.add_css_class(COLLAPSE_WRAPPER_CLASS);
    wrapper.set_vexpand(true);
    wrapper.set_hexpand(true);
    wrapper.set_size_request(COLLAPSE_SIZE, COLLAPSE_SIZE);
    wrapper.append(&child);

    let collapsed_view = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    collapsed_view.set_halign(gtk4::Align::Fill);
    collapsed_view.set_valign(gtk4::Align::Fill);
    collapsed_view.set_vexpand(true);
    collapsed_view.set_hexpand(true);
    collapsed_view.set_visible(false);
    collapsed_view.add_css_class("panel-collapsed-overlay");
    {
        let icon = gtk4::Image::from_icon_name("go-next-symbolic");
        icon.set_pixel_size(COLLAPSED_ICON_SIZE);
        icon.set_halign(gtk4::Align::Center);
        icon.set_valign(gtk4::Align::Center);
        icon.set_can_target(false);
        let chip = gtk4::CenterBox::new();
        chip.add_css_class("panel-collapsed-chip");
        chip.set_size_request(COLLAPSED_CHROME_SIZE, COLLAPSED_CHROME_SIZE);
        chip.set_halign(gtk4::Align::Fill);
        chip.set_valign(gtk4::Align::Fill);
        chip.set_hexpand(true);
        chip.set_vexpand(true);
        chip.set_center_widget(Some(&icon));
        collapsed_view.append(&chip);
    }
    collapsed_view.set_tooltip_text(Some("Click to expand"));
    wrapper.append(&collapsed_view);
    {
        let content_ref = child.clone();
        let cv_ref: gtk4::Widget = collapsed_view.clone().upcast();
        let wrapper_ref = wrapper.clone();
        let gesture = gtk4::GestureClick::new();
        gesture.set_button(1);
        gesture.connect_released(move |g, _, _, _| {
            if content_ref.is_visible() {
                return;
            }
            expand_wrapped_collapsed_layout(&content_ref, &cv_ref, &wrapper_ref);
            g.set_state(gtk4::EventSequenceState::Claimed);
        });
        wrapper.add_controller(gesture);
    }

    wrapper.upcast()
}

fn expand_wrapped_collapsed_layout(
    content: &gtk4::Widget,
    collapsed_view: &gtk4::Widget,
    wrapper: &gtk4::Box,
) {
    content.set_visible(true);
    collapsed_view.set_visible(false);
    wrapper.set_size_request(-1, -1);
    reset_collapsed_children(content);

    if let Some(parent) = wrapper.parent() {
        if let Some(paned) = parent.downcast_ref::<gtk4::Paned>() {
            let total = if paned.orientation() == gtk4::Orientation::Horizontal {
                paned.allocation().width()
            } else {
                paned.allocation().height()
            };
            if total > 0 {
                paned.set_position(total / 2);
            }
        }
    }
}

/// Reset any collapsed PanelHosts inside a widget subtree.
/// Called when a wrapper is expanded to clear inner collapsed states.
fn reset_collapsed_children(widget: &gtk4::Widget) {
    // PanelHost outer boxes have CSS class "panel-frame" and children:
    // container (Box) + collapsed_view (Widget) + footer_bar (Box)
    if let Ok(bx) = widget.clone().downcast::<gtk4::Box>() {
        if bx.has_css_class("panel-frame") {
            // This looks like a PanelHost outer box — check if collapsed
            if let Some(container) = bx.first_child() {
                if !container.is_visible() {
                    // Collapsed: restore container, hide collapsed_view
                    container.set_visible(true);
                    if let Some(cv) = container.next_sibling() {
                        cv.set_visible(false);
                    }
                    bx.set_size_request(-1, -1);
                }
            }
        }
    }
    // Recurse into children
    let mut child = widget.first_child();
    while let Some(c) = child {
        reset_collapsed_children(&c);
        child = c.next_sibling();
    }
}

/// If the widget is a collapse wrapper, return its content child. Otherwise return the widget.
fn unwrap_layout_shell(widget: &gtk4::Widget) -> gtk4::Widget {
    let mut current = widget.clone();
    loop {
        if let Ok(bx) = current.clone().downcast::<gtk4::Box>() {
            if bx.has_css_class(COLLAPSE_WRAPPER_CLASS) {
                if let Some(content) = bx.first_child() {
                    current = content;
                    continue;
                }
            }
            if bx.has_css_class(WORKSPACE_TAB_PAGE_SHELL_CLASS) {
                if let Some(content) = bx.first_child() {
                    current = content;
                    continue;
                }
            }
        }
        if let Ok(overlay) = current.clone().downcast::<gtk4::Overlay>() {
            if overlay.has_css_class(PANED_OVERLAY_CLASS) {
                if let Some(child) = overlay.child() {
                    current = child;
                    continue;
                }
            }
        }
        return current;
    }
}

/// Uniform collapse target for drag-collapse: works for both direct PanelHosts
/// and wrapper boxes around nested layouts.
#[derive(Clone)]
struct DragCollapseTarget {
    /// Widget to set_size_request on (PanelHost outer or wrapper Box)
    outer: gtk4::Box,
    /// Content to hide (PanelHost container or nested layout widget)
    content: gtk4::Widget,
    /// Optional secondary to hide (PanelHost footer_bar)
    footer: Option<gtk4::Box>,
    /// Overlay to show when collapsed
    collapsed_view: gtk4::Widget,
    /// Footer label (for restoring footer visibility on expand)
    footer_label: Option<gtk4::Label>,
}

impl DragCollapseTarget {
    fn is_collapsed(&self) -> bool {
        !self.content.is_visible()
    }
}

/// Build a DragCollapseTarget from a Paned's child.
fn find_collapse_target(
    child: &Option<gtk4::Widget>,
    hosts: &HashMap<String, PanelHost>,
) -> Option<DragCollapseTarget> {
    let c = child.as_ref()?;

    // Case 1: direct PanelHost
    let name = c.widget_name();
    if let Some(host) = hosts.get(name.as_str()) {
        return Some(DragCollapseTarget {
            outer: host.outer.clone(),
            content: host.container.clone().upcast(),
            footer: Some(host.footer_bar.clone()),
            collapsed_view: host.collapsed_view.clone(),
            footer_label: Some(host.footer_label.clone()),
        });
    }

    // Case 2: collapse wrapper box (wraps nested layout)
    if let Ok(wrapper) = c.clone().downcast::<gtk4::Box>() {
        if wrapper.has_css_class(COLLAPSE_WRAPPER_CLASS) {
            let content = wrapper.first_child()?;
            let collapsed_view = content.next_sibling()?;
            return Some(DragCollapseTarget {
                outer: wrapper,
                content,
                footer: None,
                collapsed_view,
                footer_label: None,
            });
        }
    }

    // Case 3: find PanelHost recursively (fallback for unwrapped nested layouts)
    let host = find_panel_host_in(c, hosts)?;
    Some(DragCollapseTarget {
        outer: host.outer.clone(),
        content: host.container.clone().upcast(),
        footer: Some(host.footer_bar.clone()),
        collapsed_view: host.collapsed_view.clone(),
        footer_label: Some(host.footer_label.clone()),
    })
}

fn collapsed_view_icon(collapsed_view: &gtk4::Widget) -> Option<gtk4::Image> {
    find_image_descendant(collapsed_view)
}

#[derive(Clone)]
struct CollapsedOverlayControl {
    root: gtk4::Box,
    icon: gtk4::Image,
}

fn collapsed_icon_name(orient: gtk4::Orientation, is_start: bool) -> &'static str {
    match (orient, is_start) {
        (gtk4::Orientation::Horizontal, true) => "go-next-symbolic",
        (gtk4::Orientation::Horizontal, false) => "go-previous-symbolic",
        (_, true) => "go-down-symbolic",
        (_, false) => "go-up-symbolic",
    }
}

fn build_collapsed_overlay_control(
    paned: &gtk4::Paned,
    orient: gtk4::Orientation,
    is_start: bool,
) -> CollapsedOverlayControl {
    let root = gtk4::Box::new(orient, 0);
    root.add_css_class("panel-collapsed-overlay");
    root.set_visible(false);
    root.set_can_focus(false);

    match (orient, is_start) {
        (gtk4::Orientation::Horizontal, true) => {
            root.set_halign(gtk4::Align::Start);
            root.set_valign(gtk4::Align::Fill);
            root.set_vexpand(true);
            root.set_size_request(COLLAPSED_PANEL_SIZE, -1);
        }
        (gtk4::Orientation::Horizontal, false) => {
            root.set_halign(gtk4::Align::End);
            root.set_valign(gtk4::Align::Fill);
            root.set_vexpand(true);
            root.set_size_request(COLLAPSED_PANEL_SIZE, -1);
        }
        (gtk4::Orientation::Vertical, true) => {
            root.set_halign(gtk4::Align::Fill);
            root.set_valign(gtk4::Align::Start);
            root.set_hexpand(true);
            root.set_size_request(-1, COLLAPSED_PANEL_SIZE);
        }
        (gtk4::Orientation::Vertical, false) => {
            root.set_halign(gtk4::Align::Fill);
            root.set_valign(gtk4::Align::End);
            root.set_hexpand(true);
            root.set_size_request(-1, COLLAPSED_PANEL_SIZE);
        }
        _ => {}
    }

    let expand_area = gtk4::CenterBox::new();
    expand_area.add_css_class("panel-collapsed-chip");
    expand_area.set_halign(gtk4::Align::Fill);
    expand_area.set_valign(gtk4::Align::Fill);
    expand_area.set_hexpand(true);
    expand_area.set_vexpand(true);
    let icon = gtk4::Image::from_icon_name(collapsed_icon_name(orient, is_start));
    icon.set_pixel_size(COLLAPSED_ICON_SIZE);
    icon.set_halign(gtk4::Align::Center);
    icon.set_valign(gtk4::Align::Center);
    icon.set_can_target(false);
    expand_area.set_center_widget(Some(&icon));

    {
        let paned = paned.clone();
        let gesture = gtk4::GestureClick::new();
        gesture.set_button(1);
        gesture.connect_released(move |g, _, _, _| {
            let total = if orient == gtk4::Orientation::Horizontal {
                paned.allocation().width()
            } else {
                paned.allocation().height()
            };
            if total > 0 {
                paned.set_position(total / 2);
            }
            g.set_state(gtk4::EventSequenceState::Claimed);
        });
        expand_area.add_controller(gesture);
    }

    let drag_strip = gtk4::Box::new(orient, 0);
    drag_strip.add_css_class("panel-collapsed-drag-strip");
    drag_strip.set_can_focus(false);
    if orient == gtk4::Orientation::Horizontal {
        drag_strip.set_size_request(COLLAPSED_DRAG_STRIP_SIZE, -1);
        drag_strip.set_vexpand(true);
        drag_strip.set_cursor_from_name(Some("col-resize"));
    } else {
        drag_strip.set_size_request(-1, COLLAPSED_DRAG_STRIP_SIZE);
        drag_strip.set_hexpand(true);
        drag_strip.set_cursor_from_name(Some("row-resize"));
    }
    {
        let drag_origin = Rc::new(Cell::new(0));
        let begin_origin = drag_origin.clone();
        let paned_begin = paned.clone();
        let paned_update = paned.clone();
        let drag = gtk4::GestureDrag::new();
        drag.set_button(1);
        drag.connect_drag_begin(move |g, _, _| {
            begin_origin.set(paned_begin.position());
            g.set_state(gtk4::EventSequenceState::Claimed);
        });
        drag.connect_drag_update(move |_, offset_x, offset_y| {
            let total = if orient == gtk4::Orientation::Horizontal {
                paned_update.allocation().width()
            } else {
                paned_update.allocation().height()
            };
            if total <= 0 {
                return;
            }
            let delta = if orient == gtk4::Orientation::Horizontal {
                offset_x.round() as i32
            } else {
                offset_y.round() as i32
            };
            paned_update.set_position((drag_origin.get() + delta).clamp(0, total));
        });
        drag_strip.add_controller(drag);
    }

    match (orient, is_start) {
        (gtk4::Orientation::Horizontal, true) | (gtk4::Orientation::Vertical, true) => {
            root.append(&expand_area);
            root.append(&drag_strip);
        }
        _ => {
            root.append(&drag_strip);
            root.append(&expand_area);
        }
    }

    CollapsedOverlayControl { root, icon }
}

fn wrap_paned_with_collapse_overlay(
    paned: &gtk4::Paned,
    hosts: &HashMap<String, PanelHost>,
) -> gtk4::Widget {
    let start = find_collapse_target(&paned.start_child(), hosts);
    let end = find_collapse_target(&paned.end_child(), hosts);
    let orient = paned.orientation();
    tracing::debug!(
        "setup_paned_drag_collapse: orient={:?}, start={}, end={}",
        orient,
        start.is_some(),
        end.is_some()
    );

    if start.is_none() && end.is_none() {
        return paned.clone().upcast();
    }

    let overlay = gtk4::Overlay::new();
    overlay.add_css_class(PANED_OVERLAY_CLASS);
    overlay.set_child(Some(paned));

    let start_control = start
        .as_ref()
        .map(|_| build_collapsed_overlay_control(paned, orient, true));
    let end_control = end
        .as_ref()
        .map(|_| build_collapsed_overlay_control(paned, orient, false));

    if let Some(control) = start_control.as_ref() {
        overlay.add_overlay(&control.root);
        overlay.set_measure_overlay(&control.root, false);
    }
    if let Some(control) = end_control.as_ref() {
        overlay.add_overlay(&control.root);
        overlay.set_measure_overlay(&control.root, false);
    }

    setup_paned_drag_collapse(paned, start, end, start_control, end_control);
    overlay.upcast()
}

fn find_image_descendant(widget: &gtk4::Widget) -> Option<gtk4::Image> {
    if let Ok(image) = widget.clone().downcast::<gtk4::Image>() {
        return Some(image);
    }

    let mut child = widget.first_child();
    while let Some(current) = child {
        if let Some(image) = find_image_descendant(&current) {
            return Some(image);
        }
        child = current.next_sibling();
    }

    None
}

/// Monitor Paned drag to auto-collapse/expand at threshold.
///
/// IMPORTANT: This handler must NEVER call set_position() or toggle set_shrink_*_child()
/// while reacting to notify. See panel_host.rs header comment for full explanation of
/// constraints. Visibility and size requests are safe here.
fn setup_paned_drag_collapse(
    paned: &gtk4::Paned,
    start: Option<DragCollapseTarget>,
    end: Option<DragCollapseTarget>,
    start_control: Option<CollapsedOverlayControl>,
    end_control: Option<CollapsedOverlayControl>,
) {
    let threshold = COLLAPSE_SIZE + 8; // slightly above collapse size for drag detection
    let orient = paned.orientation();

    if start.is_none() && end.is_none() {
        return;
    }
    let guard = std::rc::Rc::new(std::cell::Cell::new(false));
    // Shared guard for idle snap — prevents notify handler from reacting to our set_position
    let snap_guard = guard.clone();

    paned.connect_notify_local(Some("position"), move |paned, _| {
        if guard.get() {
            return;
        }
        guard.set(true);

        let total = if orient == gtk4::Orientation::Horizontal {
            paned.allocation().width()
        } else {
            paned.allocation().height()
        };
        if total <= 0 {
            guard.set(false);
            return;
        }

        let pos = paned.position();
        let start_size = pos;
        let end_size = total - pos;

        // Helper: collapse a target
        let do_collapse = |target: &DragCollapseTarget,
                           overlay: Option<&CollapsedOverlayControl>,
                           is_start: bool| {
            tracing::debug!(
                "drag_collapse orient={:?} side={} pos={} total={} target='{}'",
                orient,
                if is_start { "start" } else { "end" },
                pos,
                total,
                target.outer.widget_name()
            );
            target.content.set_visible(false);
            if let Some(ref f) = target.footer {
                f.set_visible(false);
            }
            target
                .outer
                .set_size_request(COLLAPSED_PANEL_SIZE, COLLAPSED_PANEL_SIZE);
            let icon = collapsed_icon_name(orient, is_start);
            if let Some(control) = overlay {
                target.collapsed_view.set_visible(false);
                target.outer.add_css_class(COLLAPSED_PLACEHOLDER_CLASS);
                control.icon.set_icon_name(Some(icon));
                control.root.set_visible(true);
            } else {
                target.outer.remove_css_class(COLLAPSED_PLACEHOLDER_CLASS);
                target.collapsed_view.set_visible(true);
                if let Some(img) = collapsed_view_icon(&target.collapsed_view) {
                    img.set_icon_name(Some(icon));
                }
            }
        };

        // Helper: expand a target
        let do_expand = |target: &DragCollapseTarget, overlay: Option<&CollapsedOverlayControl>| {
            tracing::debug!(
                "drag_expand orient={:?} pos={} total={} target='{}'",
                orient,
                pos,
                total,
                target.outer.widget_name()
            );
            target.outer.remove_css_class(COLLAPSED_PLACEHOLDER_CLASS);
            target.collapsed_view.set_visible(false);
            target.content.set_visible(true);
            target.outer.set_size_request(-1, -1);
            if let (Some(ref f), Some(ref lbl)) = (&target.footer, &target.footer_label) {
                f.set_visible(!lbl.text().is_empty());
            }
            if let Some(control) = overlay {
                control.root.set_visible(false);
            }
        };

        // Auto-collapse/expand start child
        if let Some(ref t) = start {
            if start_size <= threshold && !t.is_collapsed() {
                do_collapse(t, start_control.as_ref(), true);
            } else if start_size > threshold && t.is_collapsed() {
                do_expand(t, start_control.as_ref());
            }
        }

        // Auto-collapse/expand end child
        if let Some(ref t) = end {
            if end_size <= threshold && !t.is_collapsed() {
                do_collapse(t, end_control.as_ref(), false);
            } else if end_size > threshold && t.is_collapsed() {
                do_expand(t, end_control.as_ref());
            }
        }

        // Snap correction: collapsed panels should allocate the compact visual size,
        // not the larger drag threshold size.
        // schedule idle set_position with guard held to prevent cascading.
        let sc = start.as_ref().map_or(false, |t| t.is_collapsed());
        let ec = end.as_ref().map_or(false, |t| t.is_collapsed());
        let need_snap_start = sc && start_size != COLLAPSED_PANEL_SIZE;
        let need_snap_end = ec && end_size != COLLAPSED_PANEL_SIZE;
        if need_snap_start || need_snap_end {
            let p = paned.clone();
            let g = snap_guard.clone();
            gtk4::glib::idle_add_local_once(move || {
                g.set(true); // Block notify handler during our set_position
                let t = if orient == gtk4::Orientation::Horizontal {
                    p.allocation().width()
                } else {
                    p.allocation().height()
                };
                if t > 0 {
                    if need_snap_start {
                        p.set_position(COLLAPSED_PANEL_SIZE);
                    }
                    if need_snap_end {
                        p.set_position(t - COLLAPSED_PANEL_SIZE);
                    }
                }
                g.set(false);
            });
        }

        guard.set(false);
    });

    let paned = paned.clone();
    gtk4::glib::idle_add_local_once(move || {
        paned.notify("position");
    });
}

pub fn apply_min_size(host: &PanelHost, cfg: &PanelConfig) {
    let w = if cfg.min_width > 0 {
        cfg.min_width as i32
    } else {
        -1
    };
    let h = if cfg.min_height > 0 {
        cfg.min_height as i32
    } else {
        -1
    };
    if w > 0 || h > 0 {
        host.widget().set_size_request(w, h);
    }
}

fn subtree_has_min_size(node: &LayoutNode, panels: &[PanelConfig]) -> bool {
    match node {
        LayoutNode::Panel { id } => panels
            .iter()
            .any(|p| p.id == *id && (p.min_width > 0 || p.min_height > 0)),
        LayoutNode::Hsplit { children, .. }
        | LayoutNode::Vsplit { children, .. }
        | LayoutNode::Tabs { children, .. } => {
            children.iter().any(|c| subtree_has_min_size(c, panels))
        }
    }
}

pub fn sync_ratios_recursive(widget: &gtk4::Widget, node: &mut LayoutNode) {
    // See through collapse wrappers and paned overlay shells.
    let widget = unwrap_layout_shell(widget);
    let is_hsplit = matches!(node, LayoutNode::Hsplit { .. });
    match node {
        LayoutNode::Panel { .. } => {}
        LayoutNode::Hsplit { children, ratios } | LayoutNode::Vsplit { children, ratios } => {
            if children.len() < 2 {
                return;
            }
            if let Ok(paned) = widget.clone().downcast::<gtk4::Paned>() {
                let alloc = paned.allocation();
                let total = if paned.orientation() == gtk4::Orientation::Horizontal {
                    alloc.width()
                } else {
                    alloc.height()
                };
                if total > 0 {
                    let pos = paned.position();
                    let r1 = pos as f64 / total as f64;
                    let r2 = 1.0 - r1;
                    if children.len() == 2 {
                        if ratios.len() >= 2 {
                            ratios[0] = r1;
                            ratios[1] = r2;
                        }
                        if let Some(w1) = paned.start_child() {
                            sync_ratios_recursive(&w1, &mut children[0]);
                        }
                        if let Some(w2) = paned.end_child() {
                            sync_ratios_recursive(&w2, &mut children[1]);
                        }
                    } else {
                        if !ratios.is_empty() {
                            ratios[0] = r1;
                        }
                        if let Some(w1) = paned.start_child() {
                            sync_ratios_recursive(&w1, &mut children[0]);
                        }
                        if let Some(w2) = paned.end_child() {
                            let rest_children = children[1..].to_vec();
                            let rest_ratios = if ratios.len() > 1 {
                                ratios[1..].to_vec()
                            } else {
                                vec![1.0; rest_children.len()]
                            };
                            let mut rest_node = if is_hsplit {
                                LayoutNode::Hsplit {
                                    children: rest_children,
                                    ratios: rest_ratios,
                                }
                            } else {
                                LayoutNode::Vsplit {
                                    children: rest_children,
                                    ratios: rest_ratios,
                                }
                            };
                            sync_ratios_recursive(&w2, &mut rest_node);
                            match rest_node {
                                LayoutNode::Hsplit {
                                    children: rc,
                                    ratios: rr,
                                }
                                | LayoutNode::Vsplit {
                                    children: rc,
                                    ratios: rr,
                                } => {
                                    for (i, c) in rc.into_iter().enumerate() {
                                        children[i + 1] = c;
                                    }
                                    for (i, r) in rr.into_iter().enumerate() {
                                        if i + 1 < ratios.len() {
                                            ratios[i + 1] = r;
                                        }
                                    }
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
