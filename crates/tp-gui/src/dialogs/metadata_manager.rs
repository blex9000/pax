//! Cross-workspace metadata inspector. Lets the user browse every
//! workspace_file_metadata_entries row in the DB, filter by workspace and
//! entry type, search by substring, and bulk-delete (multi-selection or
//! whole workspace). Reached from the app menu's Workspace Metadata entry.

use gtk4::prelude::*;
use pax_db::metadata_entries::MetadataEntry;
use pax_db::Database;
use std::cell::RefCell;
use std::rc::Rc;

const DIALOG_WIDTH_PX: i32 = 820;
const DIALOG_HEIGHT_PX: i32 = 520;
const ALL_WORKSPACES: &str = "(All workspaces)";
const ALL_TYPES: &str = "(All types)";
const PREVIEW_MAX_CHARS: usize = 80;

#[derive(Debug, Clone)]
struct WorkspaceRow {
    label: String,
    record_key: String,
}

pub fn show_metadata_manager(parent: Option<&gtk4::Window>) {
    let dialog = gtk4::Window::builder()
        .title("Workspace Metadata")
        .modal(true)
        .default_width(DIALOG_WIDTH_PX)
        .default_height(DIALOG_HEIGHT_PX)
        .build();
    crate::theme::configure_dialog_window(&dialog);
    if let Some(win) = parent {
        dialog.set_transient_for(Some(win));
    }

    let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
    vbox.set_margin_top(8);
    vbox.set_margin_bottom(8);
    vbox.set_margin_start(8);
    vbox.set_margin_end(8);

    // Filters row.
    let filters = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    let workspace_dropdown = gtk4::DropDown::from_strings(&[ALL_WORKSPACES]);
    let type_dropdown = gtk4::DropDown::from_strings(&[ALL_TYPES]);
    let search = gtk4::SearchEntry::new();
    search.set_placeholder_text(Some("Search file path or text…"));
    search.set_hexpand(true);
    filters.append(&workspace_dropdown);
    filters.append(&type_dropdown);
    filters.append(&search);
    vbox.append(&filters);

    // Results list (ListBox, multi-select).
    let list_box = gtk4::ListBox::new();
    list_box.set_selection_mode(gtk4::SelectionMode::Multiple);
    list_box.add_css_class("boxed-list");
    let scroll = gtk4::ScrolledWindow::new();
    scroll.set_child(Some(&list_box));
    scroll.set_vexpand(true);
    vbox.append(&scroll);

    // Actions row.
    let actions = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    actions.set_halign(gtk4::Align::End);
    let refresh_btn = gtk4::Button::with_label("Refresh");
    let delete_selected_btn = gtk4::Button::with_label("Delete selected");
    delete_selected_btn.add_css_class("destructive-action");
    let delete_workspace_btn = gtk4::Button::with_label("Delete all for workspace");
    delete_workspace_btn.add_css_class("destructive-action");
    delete_workspace_btn.set_sensitive(false);
    let close_btn = gtk4::Button::with_label("Close");
    actions.append(&refresh_btn);
    actions.append(&delete_selected_btn);
    actions.append(&delete_workspace_btn);
    actions.append(&close_btn);
    vbox.append(&actions);

    let workspaces: Rc<RefCell<Vec<WorkspaceRow>>> = Rc::new(RefCell::new(Vec::new()));
    let types: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let rows: Rc<RefCell<Vec<MetadataEntry>>> = Rc::new(RefCell::new(Vec::new()));

    let repopulate_filters = {
        let workspaces = workspaces.clone();
        let types = types.clone();
        let workspace_dropdown = workspace_dropdown.clone();
        let type_dropdown = type_dropdown.clone();
        Rc::new(move || {
            let db_path = Database::default_path();
            let Ok(db) = Database::open(&db_path) else {
                return;
            };
            let ws_rows: Vec<WorkspaceRow> = db
                .list_workspaces_limit(500)
                .unwrap_or_default()
                .into_iter()
                .map(|w| {
                    let rk = pax_db::workspaces::compute_record_key(
                        &w.name,
                        w.config_path.as_deref(),
                    );
                    let label = match &w.config_path {
                        Some(path) if !path.trim().is_empty() => {
                            format!("{} ({})", w.name, path)
                        }
                        _ => w.name.clone(),
                    };
                    WorkspaceRow {
                        label,
                        record_key: rk,
                    }
                })
                .collect();
            let mut labels = vec![ALL_WORKSPACES.to_string()];
            labels.extend(ws_rows.iter().map(|r| r.label.clone()));
            let labels_ref: Vec<&str> = labels.iter().map(String::as_str).collect();
            workspace_dropdown.set_model(Some(&gtk4::StringList::new(&labels_ref)));
            workspace_dropdown.set_selected(0);
            *workspaces.borrow_mut() = ws_rows;

            let type_rows: Vec<String> =
                db.list_metadata_entry_types().unwrap_or_default();
            let mut tlabels = vec![ALL_TYPES.to_string()];
            tlabels.extend(type_rows.iter().cloned());
            let tlabels_ref: Vec<&str> = tlabels.iter().map(String::as_str).collect();
            type_dropdown.set_model(Some(&gtk4::StringList::new(&tlabels_ref)));
            type_dropdown.set_selected(0);
            *types.borrow_mut() = type_rows;
        })
    };

    let reload = {
        let list_box = list_box.clone();
        let rows = rows.clone();
        let workspaces = workspaces.clone();
        let types = types.clone();
        let workspace_dropdown = workspace_dropdown.clone();
        let type_dropdown = type_dropdown.clone();
        let search = search.clone();
        let delete_workspace_btn = delete_workspace_btn.clone();
        Rc::new(move || {
            while let Some(child) = list_box.first_child() {
                list_box.remove(&child);
            }
            let db_path = Database::default_path();
            let Ok(db) = Database::open(&db_path) else {
                return;
            };

            let search_text = search.text().to_string();
            let search_opt = if search_text.is_empty() {
                None
            } else {
                Some(search_text.clone())
            };

            let type_idx = type_dropdown.selected() as usize;
            let type_opt = if type_idx == 0 {
                None
            } else {
                types.borrow().get(type_idx - 1).cloned()
            };

            let ws_idx = workspace_dropdown.selected() as usize;
            let ws_filter_key = if ws_idx == 0 {
                None
            } else {
                workspaces
                    .borrow()
                    .get(ws_idx - 1)
                    .map(|w| w.record_key.clone())
            };
            delete_workspace_btn.set_sensitive(ws_filter_key.is_some());

            let entries: Vec<MetadataEntry> = if let Some(rk) = &ws_filter_key {
                db.list_metadata_for_workspace(rk, type_opt.as_deref())
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|e| match &search_opt {
                        Some(q) => {
                            let q = q.to_lowercase();
                            e.file_path.to_lowercase().contains(&q)
                                || e.payload.to_lowercase().contains(&q)
                        }
                        None => true,
                    })
                    .collect()
            } else {
                db.list_metadata_across_workspaces(
                    search_opt.as_deref(),
                    type_opt.as_deref(),
                )
                .unwrap_or_default()
            };

            for (idx, entry) in entries.iter().enumerate() {
                list_box.append(&build_row(entry, idx));
            }
            *rows.borrow_mut() = entries;
        })
    };

    repopulate_filters();
    reload();

    {
        let reload = reload.clone();
        search.connect_search_changed(move |_| reload());
    }
    {
        let reload = reload.clone();
        workspace_dropdown.connect_selected_notify(move |_| reload());
    }
    {
        let reload = reload.clone();
        type_dropdown.connect_selected_notify(move |_| reload());
    }
    {
        let repopulate_filters = repopulate_filters.clone();
        let reload = reload.clone();
        refresh_btn.connect_clicked(move |_| {
            repopulate_filters();
            reload();
        });
    }

    {
        let list_box_c = list_box.clone();
        let rows = rows.clone();
        let reload = reload.clone();
        let dialog_c = dialog.clone();
        delete_selected_btn.connect_clicked(move |_| {
            let selected: Vec<i64> = list_box_c
                .selected_rows()
                .iter()
                .filter_map(|row| {
                    let idx: usize = row.widget_name().parse().ok()?;
                    rows.borrow().get(idx).map(|e| e.id)
                })
                .collect();
            if selected.is_empty() {
                return;
            }
            let count = selected.len();
            let reload = reload.clone();
            confirm_delete(
                Some(&dialog_c),
                &format!("Delete {} selected entries?", count),
                move || {
                    let db_path = Database::default_path();
                    if let Ok(db) = Database::open(&db_path) {
                        for id in &selected {
                            let _ = db.delete_metadata_entry(*id);
                        }
                    }
                    reload();
                },
            );
        });
    }

    {
        let workspaces = workspaces.clone();
        let workspace_dropdown = workspace_dropdown.clone();
        let reload = reload.clone();
        let dialog_c = dialog.clone();
        delete_workspace_btn.connect_clicked(move |_| {
            let ws_idx = workspace_dropdown.selected() as usize;
            if ws_idx == 0 {
                return;
            }
            let Some(ws) = workspaces.borrow().get(ws_idx - 1).cloned() else {
                return;
            };
            let reload = reload.clone();
            let record_key = ws.record_key.clone();
            confirm_delete(
                Some(&dialog_c),
                &format!("Delete every metadata entry for \"{}\"?", ws.label),
                move || {
                    let db_path = Database::default_path();
                    if let Ok(db) = Database::open(&db_path) {
                        let _ = db.delete_metadata_for_workspace(&record_key);
                    }
                    reload();
                },
            );
        });
    }

    {
        let d = dialog.clone();
        close_btn.connect_clicked(move |_| d.close());
    }

    dialog.set_child(Some(&vbox));
    dialog.present();
}

fn build_row(entry: &MetadataEntry, idx: usize) -> gtk4::ListBoxRow {
    let row = gtk4::ListBoxRow::new();
    row.set_widget_name(&idx.to_string());

    let hbox = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    hbox.set_margin_start(8);
    hbox.set_margin_end(8);
    hbox.set_margin_top(4);
    hbox.set_margin_bottom(4);

    let type_label = gtk4::Label::new(Some(&entry.entry_type));
    type_label.add_css_class("dim-label");
    type_label.set_width_chars(8);
    type_label.set_xalign(0.0);
    hbox.append(&type_label);

    let ws_label = gtk4::Label::new(Some(&entry.record_key));
    ws_label.add_css_class("caption");
    ws_label.set_width_chars(28);
    ws_label.set_xalign(0.0);
    ws_label.set_ellipsize(gtk4::pango::EllipsizeMode::Middle);
    hbox.append(&ws_label);

    let file_label = gtk4::Label::new(Some(&format!(
        "{}:{}",
        entry.file_path,
        entry.line_number + 1
    )));
    file_label.set_hexpand(true);
    file_label.set_xalign(0.0);
    file_label.set_ellipsize(gtk4::pango::EllipsizeMode::Middle);
    hbox.append(&file_label);

    let preview = gtk4::Label::new(Some(&preview_of(&entry.payload)));
    preview.add_css_class("dim-label");
    preview.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    preview.set_width_chars(32);
    preview.set_xalign(0.0);
    hbox.append(&preview);

    row.set_child(Some(&hbox));
    row
}

fn preview_of(payload: &str) -> String {
    // Try to parse as {"text": "..."}; fall back to raw payload truncated.
    match serde_json::from_str::<serde_json::Value>(payload) {
        Ok(v) => {
            if let Some(text) = v.get("text").and_then(|v| v.as_str()) {
                return truncate_line(text);
            }
            truncate_line(payload)
        }
        Err(_) => truncate_line(payload),
    }
}

fn truncate_line(s: &str) -> String {
    let first = s.lines().next().unwrap_or("");
    if first.chars().count() > PREVIEW_MAX_CHARS {
        let t: String = first.chars().take(PREVIEW_MAX_CHARS).collect();
        format!("{}…", t)
    } else {
        first.to_string()
    }
}

fn confirm_delete(
    parent: Option<&gtk4::Window>,
    message: &str,
    on_confirm: impl Fn() + 'static,
) {
    let dialog = gtk4::Window::builder()
        .title("Confirm delete")
        .modal(true)
        .default_width(380)
        .default_height(120)
        .build();
    crate::theme::configure_dialog_window(&dialog);
    if let Some(win) = parent {
        dialog.set_transient_for(Some(win));
    }
    let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    vbox.set_margin_top(16);
    vbox.set_margin_bottom(16);
    vbox.set_margin_start(16);
    vbox.set_margin_end(16);
    vbox.append(&gtk4::Label::new(Some(message)));
    let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    row.set_halign(gtk4::Align::End);
    let cancel = gtk4::Button::with_label("Cancel");
    let confirm = gtk4::Button::with_label("Delete");
    confirm.add_css_class("destructive-action");
    row.append(&cancel);
    row.append(&confirm);
    vbox.append(&row);
    {
        let d = dialog.clone();
        cancel.connect_clicked(move |_| d.close());
    }
    {
        let d = dialog.clone();
        confirm.connect_clicked(move |_| {
            on_confirm();
            d.close();
        });
    }
    dialog.set_child(Some(&vbox));
    dialog.present();
}
