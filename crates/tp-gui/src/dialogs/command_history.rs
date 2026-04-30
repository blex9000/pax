//! Command history popover for terminal panels.
//!
//! Shows the per-panel command history from `command_history` table.
//! By default only the latest occurrence of each unique command is
//! listed; toggling "Distinct" off switches to the full chronological
//! history. A search box filters the visible rows by case-insensitive
//! substring; pagination controls in the footer let the user browse
//! older entries when the result set is larger than `PAGE_SIZE`.
//!
//! Clicking a row writes the command text into the terminal (no Enter),
//! letting the user edit before executing.

use std::cell::Cell;
use std::cell::RefCell;
use std::rc::Rc;

use gtk4::prelude::*;

use crate::panels::PanelInputCallback;

/// Maximum number of rows fetched from the DB at once. Caps both the
/// distinct and full-history queries; pagination operates on the
/// in-memory slice that comes back. Bumping this trades memory for
/// reach into older history.
const HISTORY_LIMIT: usize = 5000;

/// Number of rows shown on a single popover page.
const PAGE_SIZE: usize = 20;

/// Hard cap on popover width, in pixels. Keeps the popup compact even
/// when a row carries a very long command. Inner labels ellipsize at
/// `MAX_CMD_CHARS` so the natural row size stays bounded; the
/// horizontal scrollbar policy is `Automatic` as a safety net for any
/// monospace label that still exceeds it.
const POPOVER_WIDTH: i32 = 540;

/// Max character cells the command label asks for at its natural size.
/// Combined with `EllipsizeMode::End`, this keeps a row narrow enough
/// to fit inside `POPOVER_WIDTH` next to the timestamp column.
const MAX_CMD_CHARS: i32 = 56;

/// Build the contents of the command-history popover for `panel_uuid`.
/// Each row, when clicked, writes its command into the terminal via
/// `input_cb` (no `\r` appended) and pops the popover down.
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
    // Cap the popover width: long commands ellipsize and the list
    // scrolls horizontally if needed instead of stretching the popover
    // off the screen.
    outer.set_size_request(POPOVER_WIDTH, -1);

    // ── Header: title + distinct toggle ────────────────────────────────
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

    // ── Search ─────────────────────────────────────────────────────────
    let search_entry = gtk4::SearchEntry::new();
    search_entry.set_placeholder_text(Some("Filter…"));
    search_entry.add_css_class("command-history-search");
    outer.append(&search_entry);

    // ── List ───────────────────────────────────────────────────────────
    let scroll = gtk4::ScrolledWindow::new();
    scroll.set_policy(gtk4::PolicyType::Automatic, gtk4::PolicyType::Automatic);
    scroll.set_min_content_height(280);
    scroll.set_max_content_height(420);
    scroll.set_propagate_natural_height(true);
    scroll.set_propagate_natural_width(false);

    let list = gtk4::ListBox::new();
    list.add_css_class("command-history-list");
    list.set_selection_mode(gtk4::SelectionMode::None);
    scroll.set_child(Some(&list));
    outer.append(&scroll);

    // ── Footer: pagination ─────────────────────────────────────────────
    let footer = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    footer.add_css_class("command-history-footer");

    let prev_btn = gtk4::Button::from_icon_name("go-previous-symbolic");
    prev_btn.add_css_class("flat");
    prev_btn.set_tooltip_text(Some("Previous page"));

    let next_btn = gtk4::Button::from_icon_name("go-next-symbolic");
    next_btn.add_css_class("flat");
    next_btn.set_tooltip_text(Some("Next page"));

    let page_label = gtk4::Label::new(Some(""));
    page_label.add_css_class("dim-label");
    page_label.add_css_class("command-history-page");
    page_label.set_hexpand(true);
    page_label.set_halign(gtk4::Align::Center);

    footer.append(&prev_btn);
    footer.append(&page_label);
    footer.append(&next_btn);
    outer.append(&footer);

    // ── State + refresh closure ────────────────────────────────────────
    let panel_uuid_owned: Rc<String> = Rc::new(panel_uuid.to_string());
    let popover_for_refresh = popover.clone();
    let input_cb_for_refresh: Rc<RefCell<PanelInputCallback>> =
        Rc::new(RefCell::new(input_cb));
    let page: Rc<Cell<usize>> = Rc::new(Cell::new(0));

    let refresh = Rc::new({
        let panel_uuid = panel_uuid_owned.clone();
        let popover = popover_for_refresh.clone();
        let input_cb = input_cb_for_refresh.clone();
        let list = list.clone();
        let toggle = distinct_toggle.clone();
        let entry = search_entry.clone();
        let page = page.clone();
        let page_label = page_label.clone();
        let prev_btn = prev_btn.clone();
        let next_btn = next_btn.clone();
        move || {
            populate_list(
                &list,
                &panel_uuid,
                toggle.is_active(),
                entry.text().as_str(),
                page.get(),
                &page_label,
                &prev_btn,
                &next_btn,
                &input_cb.borrow(),
                &popover,
            );
        }
    });

    refresh();

    // Reset to page 0 whenever the result set changes.
    {
        let refresh = refresh.clone();
        let page = page.clone();
        distinct_toggle.connect_active_notify(move |_| {
            page.set(0);
            refresh();
        });
    }
    {
        let refresh = refresh.clone();
        let page = page.clone();
        search_entry.connect_search_changed(move |_| {
            page.set(0);
            refresh();
        });
    }

    {
        let refresh = refresh.clone();
        let page = page.clone();
        prev_btn.connect_clicked(move |_| {
            let cur = page.get();
            if cur > 0 {
                page.set(cur - 1);
                refresh();
            }
        });
    }
    {
        let refresh = refresh.clone();
        let page = page.clone();
        next_btn.connect_clicked(move |_| {
            page.set(page.get() + 1);
            refresh();
        });
    }

    // Enter on the search box pastes the topmost row currently visible —
    // mirrors shell reverse-i-search.
    {
        let list = list.clone();
        search_entry.connect_activate(move |_| {
            if let Some(first_row) = list.row_at_index(0) {
                if let Some(btn) = first_row.child().and_downcast::<gtk4::Button>() {
                    btn.emit_clicked();
                }
            }
        });
    }

    popover.set_child(Some(&outer));
    popover
}

#[allow(clippy::too_many_arguments)]
fn populate_list(
    list: &gtk4::ListBox,
    panel_uuid: &str,
    distinct: bool,
    query: &str,
    requested_page: usize,
    page_label: &gtk4::Label,
    prev_btn: &gtk4::Button,
    next_btn: &gtk4::Button,
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
        update_pager(page_label, prev_btn, next_btn, 0, 0, 0);
        return;
    }

    let total = filtered.len();
    let total_pages = total.div_ceil(PAGE_SIZE).max(1);
    // Clamp the requested page in case the previous one is now past the
    // end (e.g. the user typed a more selective filter).
    let page = requested_page.min(total_pages - 1);
    let start = page * PAGE_SIZE;
    let end = (start + PAGE_SIZE).min(total);
    for rec in &filtered[start..end] {
        list.append(&build_history_row(rec, input_cb.clone(), popover));
    }
    update_pager(page_label, prev_btn, next_btn, page, total_pages, total);
}

fn update_pager(
    label: &gtk4::Label,
    prev_btn: &gtk4::Button,
    next_btn: &gtk4::Button,
    page: usize,
    total_pages: usize,
    total_rows: usize,
) {
    if total_rows == 0 {
        label.set_text("");
        prev_btn.set_visible(false);
        next_btn.set_visible(false);
        return;
    }
    prev_btn.set_visible(true);
    next_btn.set_visible(true);
    prev_btn.set_sensitive(page > 0);
    next_btn.set_sensitive(page + 1 < total_pages);
    label.set_text(&format!(
        "Page {} / {}  ·  {} entries",
        page + 1,
        total_pages,
        total_rows,
    ));
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
    cmd_lbl.set_max_width_chars(MAX_CMD_CHARS);
    h.append(&cmd_lbl);

    row_btn.set_child(Some(&h));
    // Hovering reveals the full command + timestamp, since long lines
    // ellipsize inside the bounded popover width.
    row_btn.set_tooltip_text(Some(&format!(
        "{}\n{}",
        rec.command, rec.executed_at,
    )));

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
