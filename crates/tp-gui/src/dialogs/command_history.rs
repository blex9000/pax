//! Command history popover for terminal panels.
//!
//! Shows the per-panel command history from `command_history` table,
//! deduplicated by command text and ordered by last execution. Clicking
//! a row writes the command text into the terminal (no Enter), letting
//! the user edit before executing.

use gtk4::prelude::*;

use crate::panels::PanelInputCallback;

/// Maximum number of distinct commands shown in the popover. Older
/// distinct commands beyond this cap are not loaded — keeps popover
/// snappy on long-lived terminals.
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

    let heading = gtk4::Label::new(Some("Cronologia comandi"));
    heading.add_css_class("heading");
    heading.set_halign(gtk4::Align::Start);
    outer.append(&heading);

    let scroll = gtk4::ScrolledWindow::new();
    scroll.set_min_content_width(420);
    scroll.set_min_content_height(280);
    scroll.set_max_content_height(420);
    scroll.set_propagate_natural_height(true);

    let list = gtk4::ListBox::new();
    list.add_css_class("command-history-list");
    list.set_selection_mode(gtk4::SelectionMode::None);

    let db_result = pax_db::Database::open(&pax_db::Database::default_path())
        .and_then(|db| db.latest_distinct_commands(panel_uuid, HISTORY_LIMIT));

    match db_result {
        Ok(records) if !records.is_empty() => {
            for rec in records {
                list.append(&build_history_row(&rec, input_cb.clone(), &popover));
            }
        }
        _ => {
            let empty = gtk4::Label::new(Some("Nessun comando registrato"));
            empty.add_css_class("dim-label");
            empty.set_margin_top(24);
            empty.set_margin_bottom(24);
            list.append(&empty);
        }
    }

    scroll.set_child(Some(&list));
    outer.append(&scroll);
    popover.set_child(Some(&outer));
    popover
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
