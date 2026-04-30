//! Command popover for terminal panels.
//!
//! Two tabs:
//! - **History** (clock icon) — `command_history` rows for the panel,
//!   either deduplicated or full chronological. Hover on a row reveals
//!   a star toggle (pin / unpin) and a trash icon (delete the row).
//! - **Favorites** (star icon) — `pinned_commands` rows for the panel.
//!   Hover reveals an edit icon (rename inline) and a trash icon (unpin).
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

/// Hard cap on popover width, in pixels.
const POPOVER_WIDTH: i32 = 540;

/// Fixed scrolling list height. Pinning it keeps the popover the same
/// size regardless of how many rows the current page or filter shows.
const LIST_HEIGHT: i32 = 280;

/// Max character cells the command label asks for at its natural size.
const MAX_CMD_CHARS: i32 = 56;

/// Build the contents of the commands popover for `panel_uuid`.
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
    outer.set_size_request(POPOVER_WIDTH, -1);

    let stack = gtk4::Stack::new();
    stack.set_transition_type(gtk4::StackTransitionType::Crossfade);
    stack.set_transition_duration(120);

    let switcher = gtk4::StackSwitcher::new();
    switcher.set_stack(Some(&stack));
    switcher.set_halign(gtk4::Align::Center);
    switcher.add_css_class("command-history-tabs");
    outer.append(&switcher);

    let panel_uuid_owned: Rc<String> = Rc::new(panel_uuid.to_string());
    let input_cb_shared: Rc<PanelInputCallback> = Rc::new(input_cb);

    let history_page = build_history_tab(
        panel_uuid_owned.clone(),
        input_cb_shared.clone(),
        popover.clone(),
    );
    let favorites_page = build_favorites_tab(
        panel_uuid_owned.clone(),
        input_cb_shared.clone(),
        popover.clone(),
    );

    let history_child = stack.add_titled(&history_page.widget, Some("history"), "History");
    history_child.set_icon_name("document-open-recent-symbolic");
    let favorites_child =
        stack.add_titled(&favorites_page.widget, Some("favorites"), "Favorites");
    favorites_child.set_icon_name("starred-symbolic");

    // Refresh the favourites tab whenever it becomes visible — pinning
    // from the history side is invisible to it otherwise.
    {
        let favorites_refresh = favorites_page.refresh.clone();
        let history_refresh = history_page.refresh.clone();
        stack.connect_visible_child_notify(move |stack| {
            match stack.visible_child_name().as_deref() {
                Some("favorites") => favorites_refresh(),
                Some("history") => history_refresh(),
                _ => {}
            }
        });
    }

    outer.append(&stack);
    popover.set_child(Some(&outer));
    popover
}

// ── Common state plumbed through both tabs ─────────────────────────────

#[derive(Clone)]
struct TabBundle {
    widget: gtk4::Widget,
    refresh: Rc<dyn Fn()>,
}

// ── History tab ────────────────────────────────────────────────────────

fn build_history_tab(
    panel_uuid: Rc<String>,
    input_cb: Rc<PanelInputCallback>,
    popover: gtk4::Popover,
) -> TabBundle {
    let outer = gtk4::Box::new(gtk4::Orientation::Vertical, 6);

    // Header: distinct toggle.
    let header_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    let spacer = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    header_row.append(&spacer);

    let distinct_toggle = gtk4::CheckButton::with_label("Distinct");
    distinct_toggle.set_active(true);
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
    scroll.set_policy(gtk4::PolicyType::Automatic, gtk4::PolicyType::Automatic);
    scroll.set_min_content_height(LIST_HEIGHT);
    scroll.set_max_content_height(LIST_HEIGHT);
    scroll.set_propagate_natural_height(false);
    scroll.set_propagate_natural_width(false);

    let list = gtk4::ListBox::new();
    list.add_css_class("command-history-list");
    list.set_selection_mode(gtk4::SelectionMode::None);
    scroll.set_child(Some(&list));
    outer.append(&scroll);

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

    let page: Rc<Cell<usize>> = Rc::new(Cell::new(0));

    let refresh: Rc<dyn Fn()> = Rc::new({
        let panel_uuid = panel_uuid.clone();
        let popover = popover.clone();
        let input_cb = input_cb.clone();
        let list = list.clone();
        let toggle = distinct_toggle.clone();
        let entry = search_entry.clone();
        let page = page.clone();
        let page_label = page_label.clone();
        let prev_btn = prev_btn.clone();
        let next_btn = next_btn.clone();
        move || {
            populate_history_list(
                &list,
                &panel_uuid,
                toggle.is_active(),
                entry.text().as_str(),
                page.get(),
                &page_label,
                &prev_btn,
                &next_btn,
                input_cb.as_ref(),
                &popover,
                &refresh_self_handle(),
            );
        }
    });

    // Workaround: the closure above needs to refer to itself for re-render
    // after row actions. Build a thread-local "current refresh" pointer.
    set_history_refresh(refresh.clone());

    refresh();

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
    // Enter on the search box pastes the topmost row currently visible.
    {
        let list = list.clone();
        search_entry.connect_activate(move |_| {
            if let Some(first_row) = list.row_at_index(0) {
                if let Some(btn) = first_row
                    .child()
                    .and_downcast::<gtk4::Box>()
                    .and_then(|b| b.first_child())
                    .and_downcast::<gtk4::Button>()
                {
                    btn.emit_clicked();
                }
            }
        });
    }

    TabBundle {
        widget: outer.upcast(),
        refresh,
    }
}

// Thread-local handle to the history-tab refresh closure, used by the
// row hover-action callbacks (pin/delete) to re-render the list after
// they mutate the database. Lives only on the GLib main thread, which
// is where every widget and callback in this module runs.
thread_local! {
    static HISTORY_REFRESH: RefCell<Option<Rc<dyn Fn()>>> = const { RefCell::new(None) };
    static FAVORITES_REFRESH: RefCell<Option<Rc<dyn Fn()>>> = const { RefCell::new(None) };
}

fn set_history_refresh(refresh: Rc<dyn Fn()>) {
    HISTORY_REFRESH.with(|cell| *cell.borrow_mut() = Some(refresh));
}

fn set_favorites_refresh(refresh: Rc<dyn Fn()>) {
    FAVORITES_REFRESH.with(|cell| *cell.borrow_mut() = Some(refresh));
}

fn refresh_self_handle() -> impl Fn() {
    || {
        HISTORY_REFRESH.with(|cell| {
            if let Some(r) = cell.borrow().as_ref() {
                r();
            }
        });
    }
}

fn refresh_favorites_handle() -> impl Fn() {
    || {
        FAVORITES_REFRESH.with(|cell| {
            if let Some(r) = cell.borrow().as_ref() {
                r();
            }
        });
    }
}

#[allow(clippy::too_many_arguments)]
fn populate_history_list(
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
    refresh: &dyn Fn(),
) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }

    let db = pax_db::Database::open(&pax_db::Database::default_path()).ok();
    let records = db
        .as_ref()
        .and_then(|d| {
            if distinct {
                d.latest_distinct_commands(panel_uuid, HISTORY_LIMIT).ok()
            } else {
                d.recent_commands_for_panel(panel_uuid, HISTORY_LIMIT).ok()
            }
        })
        .unwrap_or_default();
    let pinned_set = db
        .as_ref()
        .and_then(|d| d.pinned_command_set_for_panel(panel_uuid).ok())
        .unwrap_or_default();

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
        list.append(&empty_label(msg));
        update_pager(page_label, prev_btn, next_btn, 0, 0, 0);
        return;
    }

    let total = filtered.len();
    let total_pages = total.div_ceil(PAGE_SIZE).max(1);
    let page = requested_page.min(total_pages - 1);
    let start = page * PAGE_SIZE;
    let end = (start + PAGE_SIZE).min(total);

    let panel_uuid_owned = panel_uuid.to_string();
    for rec in &filtered[start..end] {
        let pinned = pinned_set.contains(&rec.command);
        list.append(&build_history_row(
            rec,
            pinned,
            input_cb.clone(),
            popover.clone(),
            panel_uuid_owned.clone(),
            refresh,
        ));
    }
    update_pager(page_label, prev_btn, next_btn, page, total_pages, total);
}

fn build_history_row(
    rec: &pax_db::CommandRecord,
    pinned: bool,
    input_cb: PanelInputCallback,
    popover: gtk4::Popover,
    panel_uuid: String,
    refresh: &dyn Fn(),
) -> gtk4::Widget {
    let outer = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
    outer.add_css_class("command-history-row-outer");

    // Main click target (paste-into-terminal).
    let row_btn = paste_button(&rec.command, &rec.executed_at, input_cb, popover);

    // Reveal action icons on hover.
    let actions = gtk4::Box::new(gtk4::Orientation::Horizontal, 2);
    actions.add_css_class("command-history-actions");
    actions.set_halign(gtk4::Align::End);
    actions.set_valign(gtk4::Align::Center);
    actions.set_margin_end(4);

    let star_btn = gtk4::Button::from_icon_name(if pinned {
        "starred-symbolic"
    } else {
        "non-starred-symbolic"
    });
    star_btn.add_css_class("flat");
    star_btn.add_css_class("command-history-action");
    star_btn.set_tooltip_text(Some(if pinned { "Unpin" } else { "Pin to favorites" }));
    {
        let cmd = rec.command.clone();
        let panel_uuid = panel_uuid.clone();
        let refresh = capture_refresh(refresh);
        star_btn.connect_clicked(move |_| {
            if let Ok(db) = pax_db::Database::open(&pax_db::Database::default_path()) {
                if pinned {
                    let _ = db.unpin_command(&panel_uuid, &cmd);
                } else {
                    let _ = db.pin_command(&panel_uuid, &cmd);
                }
            }
            refresh();
        });
    }

    let trash_btn = gtk4::Button::from_icon_name("user-trash-symbolic");
    trash_btn.add_css_class("flat");
    trash_btn.add_css_class("command-history-action");
    trash_btn.set_tooltip_text(Some("Delete from history"));
    {
        let row_id = rec.id;
        let refresh = capture_refresh(refresh);
        trash_btn.connect_clicked(move |_| {
            if let Ok(db) = pax_db::Database::open(&pax_db::Database::default_path()) {
                let _ = db.delete_command_history_row(row_id);
            }
            refresh();
        });
    }

    actions.append(&star_btn);
    actions.append(&trash_btn);

    install_hover_reveal(&outer, &actions);

    outer.append(&row_btn);
    outer.append(&actions);
    outer.upcast()
}

// ── Favorites tab ──────────────────────────────────────────────────────

fn build_favorites_tab(
    panel_uuid: Rc<String>,
    input_cb: Rc<PanelInputCallback>,
    popover: gtk4::Popover,
) -> TabBundle {
    let outer = gtk4::Box::new(gtk4::Orientation::Vertical, 6);

    let search_entry = gtk4::SearchEntry::new();
    search_entry.set_placeholder_text(Some("Filter…"));
    search_entry.add_css_class("command-history-search");
    outer.append(&search_entry);

    let scroll = gtk4::ScrolledWindow::new();
    scroll.set_policy(gtk4::PolicyType::Automatic, gtk4::PolicyType::Automatic);
    scroll.set_min_content_height(LIST_HEIGHT);
    scroll.set_max_content_height(LIST_HEIGHT);
    scroll.set_propagate_natural_height(false);
    scroll.set_propagate_natural_width(false);

    let list = gtk4::ListBox::new();
    list.add_css_class("command-history-list");
    list.set_selection_mode(gtk4::SelectionMode::None);
    scroll.set_child(Some(&list));
    outer.append(&scroll);

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

    let page: Rc<Cell<usize>> = Rc::new(Cell::new(0));

    let refresh: Rc<dyn Fn()> = Rc::new({
        let panel_uuid = panel_uuid.clone();
        let popover = popover.clone();
        let input_cb = input_cb.clone();
        let list = list.clone();
        let entry = search_entry.clone();
        let page = page.clone();
        let page_label = page_label.clone();
        let prev_btn = prev_btn.clone();
        let next_btn = next_btn.clone();
        move || {
            populate_favorites_list(
                &list,
                &panel_uuid,
                entry.text().as_str(),
                page.get(),
                &page_label,
                &prev_btn,
                &next_btn,
                input_cb.as_ref(),
                &popover,
                &refresh_favorites_handle(),
            );
        }
    });
    set_favorites_refresh(refresh.clone());

    refresh();

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

    TabBundle {
        widget: outer.upcast(),
        refresh,
    }
}

#[allow(clippy::too_many_arguments)]
fn populate_favorites_list(
    list: &gtk4::ListBox,
    panel_uuid: &str,
    query: &str,
    requested_page: usize,
    page_label: &gtk4::Label,
    prev_btn: &gtk4::Button,
    next_btn: &gtk4::Button,
    input_cb: &PanelInputCallback,
    popover: &gtk4::Popover,
    refresh: &dyn Fn(),
) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }

    let records = pax_db::Database::open(&pax_db::Database::default_path())
        .ok()
        .and_then(|d| d.pinned_commands_for_panel(panel_uuid, HISTORY_LIMIT).ok())
        .unwrap_or_default();

    let needle = query.trim().to_lowercase();
    let filtered: Vec<&pax_db::PinnedCommand> = if needle.is_empty() {
        records.iter().collect()
    } else {
        records
            .iter()
            .filter(|r| r.command.to_lowercase().contains(&needle))
            .collect()
    };

    if filtered.is_empty() {
        let msg = if needle.is_empty() {
            "No favorite commands yet — pin from the History tab"
        } else {
            "No matches"
        };
        list.append(&empty_label(msg));
        update_pager(page_label, prev_btn, next_btn, 0, 0, 0);
        return;
    }

    let total = filtered.len();
    let total_pages = total.div_ceil(PAGE_SIZE).max(1);
    let page = requested_page.min(total_pages - 1);
    let start = page * PAGE_SIZE;
    let end = (start + PAGE_SIZE).min(total);

    for rec in &filtered[start..end] {
        list.append(&build_favorites_row(
            rec,
            input_cb.clone(),
            popover.clone(),
            refresh,
        ));
    }
    update_pager(page_label, prev_btn, next_btn, page, total_pages, total);
}

fn build_favorites_row(
    rec: &pax_db::PinnedCommand,
    input_cb: PanelInputCallback,
    popover: gtk4::Popover,
    refresh: &dyn Fn(),
) -> gtk4::Widget {
    let outer = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
    outer.add_css_class("command-history-row-outer");

    let row_btn = paste_button(&rec.command, &rec.created_at, input_cb, popover);

    let actions = gtk4::Box::new(gtk4::Orientation::Horizontal, 2);
    actions.add_css_class("command-history-actions");
    actions.set_halign(gtk4::Align::End);
    actions.set_valign(gtk4::Align::Center);
    actions.set_margin_end(4);

    let edit_btn = gtk4::Button::from_icon_name("document-edit-symbolic");
    edit_btn.add_css_class("flat");
    edit_btn.add_css_class("command-history-action");
    edit_btn.set_tooltip_text(Some("Edit"));
    {
        let row_id = rec.id;
        let original_cmd = rec.command.clone();
        let refresh = capture_refresh(refresh);
        let outer_weak = outer.downgrade();
        edit_btn.connect_clicked(move |_| {
            let Some(row) = outer_weak.upgrade() else {
                return;
            };
            replace_with_inline_edit(&row, row_id, &original_cmd, refresh.clone());
        });
    }

    let trash_btn = gtk4::Button::from_icon_name("user-trash-symbolic");
    trash_btn.add_css_class("flat");
    trash_btn.add_css_class("command-history-action");
    trash_btn.set_tooltip_text(Some("Remove from favorites"));
    {
        let cmd = rec.command.clone();
        let panel_uuid = rec.panel_uuid.clone();
        let refresh = capture_refresh(refresh);
        trash_btn.connect_clicked(move |_| {
            if let Ok(db) = pax_db::Database::open(&pax_db::Database::default_path()) {
                let _ = db.unpin_command(&panel_uuid, &cmd);
            }
            refresh();
        });
    }

    actions.append(&edit_btn);
    actions.append(&trash_btn);

    install_hover_reveal(&outer, &actions);

    outer.append(&row_btn);
    outer.append(&actions);
    outer.upcast()
}

fn replace_with_inline_edit(
    row: &gtk4::Box,
    row_id: i64,
    original_cmd: &str,
    refresh: Rc<dyn Fn()>,
) {
    while let Some(child) = row.first_child() {
        row.remove(&child);
    }
    let entry = gtk4::Entry::new();
    entry.set_text(original_cmd);
    entry.set_hexpand(true);
    entry.add_css_class("command-history-edit-entry");
    entry.select_region(0, -1);
    row.append(&entry);

    let commit = {
        let entry = entry.clone();
        let refresh = refresh.clone();
        Rc::new(move || {
            let new_text = entry.text().to_string();
            let trimmed = new_text.trim();
            if !trimmed.is_empty() {
                if let Ok(db) = pax_db::Database::open(&pax_db::Database::default_path()) {
                    let _ = db.update_pinned_command(row_id, trimmed);
                }
            }
            refresh();
        })
    };

    {
        let commit = commit.clone();
        entry.connect_activate(move |_| commit());
    }
    {
        let key_ctrl = gtk4::EventControllerKey::new();
        let refresh = refresh.clone();
        key_ctrl.connect_key_pressed(move |_, key, _, _| {
            if key == gtk4::gdk::Key::Escape {
                refresh();
                return gtk4::glib::Propagation::Stop;
            }
            gtk4::glib::Propagation::Proceed
        });
        entry.add_controller(key_ctrl);
    }
    {
        let commit = commit.clone();
        entry.connect_has_focus_notify(move |entry| {
            if !entry.has_focus() {
                commit();
            }
        });
    }

    entry.grab_focus();
}

// ── Shared row primitives ──────────────────────────────────────────────

fn paste_button(
    command: &str,
    timestamp: &str,
    input_cb: PanelInputCallback,
    popover: gtk4::Popover,
) -> gtk4::Button {
    let btn = gtk4::Button::new();
    btn.add_css_class("flat");
    btn.add_css_class("command-history-row");
    btn.set_hexpand(true);

    let h = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    h.set_margin_start(6);
    h.set_margin_end(2);
    h.set_margin_top(2);
    h.set_margin_bottom(2);

    let time = extract_hh_mm_ss(timestamp);
    let time_lbl = gtk4::Label::new(Some(&format!("[{}]", time)));
    time_lbl.add_css_class("dim-label");
    time_lbl.add_css_class("command-history-time");
    time_lbl.set_halign(gtk4::Align::Start);
    h.append(&time_lbl);

    let cmd_lbl = gtk4::Label::new(Some(command));
    cmd_lbl.add_css_class("monospace");
    cmd_lbl.add_css_class("command-history-cmd");
    cmd_lbl.set_halign(gtk4::Align::Start);
    cmd_lbl.set_hexpand(true);
    cmd_lbl.set_xalign(0.0);
    cmd_lbl.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    cmd_lbl.set_max_width_chars(MAX_CMD_CHARS);
    h.append(&cmd_lbl);

    btn.set_child(Some(&h));
    btn.set_tooltip_text(Some(&format!("{}\n{}", command, timestamp)));

    let cmd = command.to_string();
    btn.connect_clicked(move |_| {
        input_cb(cmd.as_bytes());
        popover.popdown();
    });
    btn
}

fn install_hover_reveal(row: &gtk4::Box, actions: &gtk4::Box) {
    actions.set_visible(false);
    let motion = gtk4::EventControllerMotion::new();
    let actions_for_enter = actions.clone();
    motion.connect_enter(move |_, _, _| {
        actions_for_enter.set_visible(true);
    });
    let actions_for_leave = actions.clone();
    motion.connect_leave(move |_| {
        actions_for_leave.set_visible(false);
    });
    row.add_controller(motion);
}

fn capture_refresh(refresh: &dyn Fn()) -> Rc<dyn Fn()> {
    // The closure passed in is borrowed; we need a `'static` clone for
    // signal handlers. We can't clone a `&dyn Fn()` directly, so we
    // route the callback through the thread-local refresh handles set
    // up by `set_history_refresh` / `set_favorites_refresh`. Both tabs
    // register on construction, so by the time row callbacks fire the
    // appropriate handle is populated.
    let _ = refresh;
    Rc::new(|| {
        HISTORY_REFRESH.with(|cell| {
            if let Some(r) = cell.borrow().as_ref() {
                r();
            }
        });
        FAVORITES_REFRESH.with(|cell| {
            if let Some(r) = cell.borrow().as_ref() {
                r();
            }
        });
    })
}

fn empty_label(msg: &str) -> gtk4::Widget {
    let lbl = gtk4::Label::new(Some(msg));
    lbl.add_css_class("dim-label");
    lbl.set_margin_top(24);
    lbl.set_margin_bottom(24);
    lbl.upcast()
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

fn extract_hh_mm_ss(executed_at: &str) -> String {
    executed_at
        .get(11..19)
        .map(str::to_string)
        .unwrap_or_else(|| executed_at.to_string())
}
