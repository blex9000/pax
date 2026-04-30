//! Command history popover for terminal panels.
//!
//! Shows the per-panel command history from `command_history` table.
//! By default only the latest occurrence of each unique command is
//! listed; toggling "Solo unici" off switches to the full chronological
//! history. Clicking a row writes the command text into the terminal
//! (no Enter), letting the user edit before executing.

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::prelude::*;

use crate::panels::PanelInputCallback;

/// Maximum number of rows the popover will load at once. Applies to
/// both modes (distinct and full).
const HISTORY_LIMIT: usize = 500;

/// Build (or rebuild) the contents of the command-history popover for
/// `panel_uuid`. Each row, when clicked, writes its command into the
/// terminal via `input_cb` (no `\r` appended) and pops the popover down.
pub fn build_command_history_popover(
    panel_uuid: &str,
    input_cb: PanelInputCallback,
) -> gtk4::Popover {
    let popover = gtk4::Popover::new();
    crate::theme::configure_popover(&popover);
    popover.add_css_class("command-history-popover");

    let outer = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
    outer.set_margin_top(6);
    outer.set_margin_bottom(6);
    outer.set_margin_start(6);
    outer.set_margin_end(6);

    let header_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    let heading = gtk4::Label::new(Some("Command history"));
    heading.add_css_class("heading");
    heading.set_halign(gtk4::Align::Start);
    heading.set_hexpand(true);
    header_row.append(&heading);

    let distinct_toggle = gtk4::CheckButton::with_label("Distinct");
    distinct_toggle.set_active(true);
    distinct_toggle.set_halign(gtk4::Align::End);
    distinct_toggle.add_css_class("command-history-toggle");
    distinct_toggle.set_tooltip_text(Some(
        "When checked, hide duplicate commands and show only the most recent run of each.",
    ));
    header_row.append(&distinct_toggle);
    outer.append(&header_row);

    let search_entry = gtk4::SearchEntry::new();
    search_entry.set_placeholder_text(Some("Filter…"));
    search_entry.add_css_class("command-history-search");
    outer.append(&search_entry);

    let scroll = gtk4::ScrolledWindow::new();
    scroll.set_min_content_width(420);
    scroll.set_min_content_height(280);
    scroll.set_max_content_height(420);
    scroll.set_propagate_natural_height(true);

    let list = gtk4::ListBox::new();
    list.add_css_class("command-history-list");
    list.set_selection_mode(gtk4::SelectionMode::None);

    let panel_uuid_owned: Rc<String> = Rc::new(panel_uuid.to_string());
    let popover_for_refresh = popover.clone();
    let input_cb_for_refresh: Rc<RefCell<PanelInputCallback>> =
        Rc::new(RefCell::new(input_cb));
    let list_for_refresh = list.clone();
    let toggle_for_refresh = distinct_toggle.clone();
    let entry_for_refresh = search_entry.clone();

    let refresh = Rc::new({
        let panel_uuid = panel_uuid_owned.clone();
        let popover = popover_for_refresh.clone();
        let input_cb = input_cb_for_refresh.clone();
        let list = list_for_refresh.clone();
        let toggle = toggle_for_refresh.clone();
        let entry = entry_for_refresh.clone();
        move || {
            populate_list(
                &list,
                &panel_uuid,
                toggle.is_active(),
                entry.text().as_str(),
                &input_cb.borrow(),
                &popover,
            );
        }
    });

    refresh();

    // GtkCheckButton's `toggled` signal does not fire reliably in some
    // GTK4 builds when the active state is changed via mouse click —
    // listen on the `notify::active` property instead, which is the
    // GObject-blessed way to react to state transitions.
    {
        let refresh = refresh.clone();
        distinct_toggle.connect_active_notify(move |_| refresh());
    }
    {
        let refresh = refresh.clone();
        search_entry.connect_search_changed(move |_| refresh());
    }

    // Pressing Enter on the search box pastes the topmost remaining row,
    // which mirrors how shell history reverse-search behaves.
    {
        let list = list.clone();
        let popover = popover.clone();
        search_entry.connect_activate(move |_| {
            if let Some(first_row) = list.row_at_index(0) {
                if let Some(btn) = first_row.child().and_downcast::<gtk4::Button>() {
                    btn.emit_clicked();
                    let _ = popover; // popdown happens inside row click
                }
            }
        });
    }

    scroll.set_child(Some(&list));
    outer.append(&scroll);
    popover.set_child(Some(&outer));
    popover
}

/// (Re)fill the list with rows for `panel_uuid`. `distinct=true` shows
/// the latest unique commands; `distinct=false` shows every execution
/// ordered by recency. `query` is a case-insensitive substring filter
/// applied to the command text — empty string disables filtering.
fn populate_list(
    list: &gtk4::ListBox,
    panel_uuid: &str,
    distinct: bool,
    query: &str,
    input_cb: &PanelInputCallback,
    popover: &gtk4::Popover,
) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }

    let db_result = pax_db::Database::open(&pax_db::Database::default_path()).and_then(|db| {
        if distinct {
            db.latest_distinct_commands(panel_uuid, HISTORY_LIMIT)
        } else {
            db.recent_commands_for_panel(panel_uuid, HISTORY_LIMIT)
        }
    });
    let records = db_result.unwrap_or_default();
    let needle = query.trim().to_lowercase();
    let filtered: Vec<&pax_db::CommandRecord> = if needle.is_empty() {
        records.iter().collect()
    } else {
        records
            .iter()
            .filter(|r| r.command.to_lowercase().contains(&needle))
            .collect()
    };

    if filtered.is_empty() {
        let msg = if needle.is_empty() {
            "No commands recorded yet"
        } else {
            "No matches"
        };
        let empty = gtk4::Label::new(Some(msg));
        empty.add_css_class("dim-label");
        empty.set_margin_top(24);
        empty.set_margin_bottom(24);
        list.append(&empty);
        return;
    }

    for rec in filtered {
        list.append(&build_history_row(rec, input_cb.clone(), popover));
    }
}

fn build_history_row(
    rec: &pax_db::CommandRecord,
    input_cb: PanelInputCallback,
    popover: &gtk4::Popover,
) -> gtk4::Widget {
    let row_btn = gtk4::Button::new();
    row_btn.add_css_class("flat");
    row_btn.add_css_class("command-history-row");

    let h = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    h.set_margin_start(6);
    h.set_margin_end(6);
    h.set_margin_top(2);
    h.set_margin_bottom(2);

    let time = extract_hh_mm_ss(&rec.executed_at);
    let time_lbl = gtk4::Label::new(Some(&format!("[{}]", time)));
    time_lbl.add_css_class("dim-label");
    time_lbl.add_css_class("command-history-time");
    time_lbl.set_halign(gtk4::Align::Start);
    h.append(&time_lbl);

    let cmd_lbl = gtk4::Label::new(Some(&rec.command));
    cmd_lbl.add_css_class("monospace");
    cmd_lbl.add_css_class("command-history-cmd");
    cmd_lbl.set_halign(gtk4::Align::Start);
    cmd_lbl.set_hexpand(true);
    cmd_lbl.set_xalign(0.0);
    cmd_lbl.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    h.append(&cmd_lbl);

    row_btn.set_child(Some(&h));
    row_btn.set_tooltip_text(Some(&rec.executed_at));

    let cmd = rec.command.clone();
    let popover = popover.clone();
    row_btn.connect_clicked(move |_| {
        input_cb(cmd.as_bytes());
        popover.popdown();
    });

    row_btn.upcast::<gtk4::Widget>()
}

fn extract_hh_mm_ss(executed_at: &str) -> String {
    // `executed_at` is SQLite `datetime('now')` format: "YYYY-MM-DD HH:MM:SS".
    // ASCII in practice; `get` returns None on a multi-byte boundary so we
    // never panic on unexpected formats.
    executed_at
        .get(11..19)
        .map(str::to_string)
        .unwrap_or_else(|| executed_at.to_string())
}
