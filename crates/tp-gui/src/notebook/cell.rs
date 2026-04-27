//! GTK widget for a single notebook cell. Anchored into the markdown
//! panel's render `TextView` via `TextView::add_child_at_anchor`.
//!
//! Layout:
//!   ┌───────────────────────────────────────────────────────────┐
//!   │ [lang] [▶/⏹] [● status]  watch every 5s         <preview> │  header
//!   ├───────────────────────────────────────────────────────────┤
//!   │ <output items: text label / image / error>               │  output area
//!   └───────────────────────────────────────────────────────────┘

use std::rc::Rc;

use gtk4::glib;
use gtk4::prelude::*;
use gtk4::{Box as GtkBox, Button, Image, Label, Orientation, Picture};

use crate::panels::terminal_registry;

use super::engine::{CellId, NotebookEngine};
use super::output::{ImageSource, OutputItem};
use super::IMAGE_MAX_HEIGHT_PX;

pub struct NotebookCell {
    pub root: GtkBox,
    pub id: CellId,
}

impl NotebookCell {
    pub fn new(engine: Rc<NotebookEngine>, id: CellId, parent_view: &gtk4::TextView) -> Self {
        let spec = engine.spec_of(id).expect("cell registered");

        let root = GtkBox::new(Orientation::Vertical, 4);
        root.add_css_class("notebook-cell");
        root.set_margin_top(4);
        root.set_margin_bottom(8);
        // Anchored child widgets in a TextView do NOT honour hexpand — they
        // get exactly their natural width. There is no public "size
        // changed" signal on TextView in GTK4, so poll the parent's
        // allocated width every 300 ms via WeakRef and propagate to the
        // cell's size_request. The Weak handles auto-stop the loop once
        // either the cell or the panel goes away (no leak).
        const SIDE_MARGIN: i32 = 24;
        const POLL_MS: u64 = 300;
        let root_weak = root.downgrade();
        let parent_weak = parent_view.downgrade();
        glib::timeout_add_local(std::time::Duration::from_millis(POLL_MS), move || {
            let (Some(root), Some(parent)) = (root_weak.upgrade(), parent_weak.upgrade()) else {
                return glib::ControlFlow::Break;
            };
            let w = parent.width();
            if w > 0 {
                let target = (w - SIDE_MARGIN).max(200);
                let cur = root.width_request();
                if (target - cur).abs() > 8 {
                    root.set_size_request(target, -1);
                }
            }
            glib::ControlFlow::Continue
        });

        // ── Header ───────────────────────────────────────────────
        // Layout: ··· spacer ···  [last-run] [▶] [⏹]
        // Language type isn't surfaced — the user hides it via the run
        // tag in the source markdown; no need to repeat it visually.
        let header = GtkBox::new(Orientation::Horizontal, 6);
        header.add_css_class("notebook-cell-header");

        let spacer = Label::new(None);
        spacer.set_hexpand(true);
        header.append(&spacer);

        // Status: [icon] [HH:MM:SS]. Held in a Box with a fixed width
        // request so swapping idle ↔ running ↔ ok ↔ error doesn't shift
        // the run/stop buttons sideways (was visible as flicker on every
        // watch tick).
        let status_box = GtkBox::new(Orientation::Horizontal, 4);
        status_box.add_css_class("notebook-status");
        status_box.set_size_request(86, -1);
        status_box.set_halign(gtk4::Align::End);
        let status_icon = Image::new();
        status_icon.add_css_class("notebook-status-icon");
        status_icon.set_visible(false);
        let status_text = Label::new(None);
        status_text.add_css_class("notebook-meta");
        status_box.append(&status_icon);
        status_box.append(&status_text);
        header.append(&status_box);

        let run_btn = Button::new();
        run_btn.set_icon_name("media-playback-start-symbolic");
        run_btn.add_css_class("flat");
        run_btn.add_css_class("notebook-cell-btn");
        run_btn.set_tooltip_text(Some("Run cell — choose target (Host or terminal panel)"));
        header.append(&run_btn);

        let stop_btn = Button::new();
        stop_btn.set_icon_name("media-playback-stop-symbolic");
        stop_btn.add_css_class("flat");
        stop_btn.add_css_class("notebook-cell-btn");
        stop_btn.set_tooltip_text(Some("Stop cell"));
        stop_btn.set_sensitive(false);
        header.append(&stop_btn);

        root.append(&header);

        // ── Output area ──────────────────────────────────────────
        let output_box = GtkBox::new(Orientation::Vertical, 2);
        output_box.add_css_class("notebook-cell-output");
        output_box.set_hexpand(true);
        root.append(&output_box);

        // ── Wire run/stop buttons ───────────────────────────────
        // Run button opens a target picker (Host = local subprocess, or
        // any registered terminal panel). Stop button kills the local
        // run if any (terminals can't be "stopped" from here — they own
        // their own subprocesses).
        {
            let engine = engine.clone();
            let stop_btn = stop_btn.clone();
            let spec_for_confirm = spec.clone();
            run_btn.connect_clicked(move |btn| {
                let engine = engine.clone();
                let stop_btn = stop_btn.clone();
                let spec = spec_for_confirm.clone();
                show_run_target_picker(btn, move |target| match target {
                    RunTarget::Host => {
                        if spec.confirm && !engine.is_confirmed(id) {
                            if !confirm_dialog_blocking() {
                                return;
                            }
                            engine.mark_confirmed(id);
                        }
                        engine.run_cell(id);
                        stop_btn.set_sensitive(true);
                    }
                    RunTarget::Terminal(term_id) => {
                        let code = engine.cell_code(id).unwrap_or_default();
                        let mut payload = Vec::with_capacity(code.len() + 1);
                        payload.extend_from_slice(code.as_bytes());
                        if !code.ends_with('\n') {
                            payload.push(b'\n');
                        }
                        if terminal_registry::send(&term_id, &payload) {
                            terminal_registry::mru_record(&term_id);
                        }
                    }
                });
            });
        }
        {
            let engine = engine.clone();
            let stop_btn_inner = stop_btn.clone();
            stop_btn.connect_clicked(move |_| {
                engine.stop_cell(id);
                stop_btn_inner.set_sensitive(false);
            });
        }

        // ── Output subscription ─────────────────────────────────
        // The engine stores subscriber `Rc<dyn Fn()>` inside its own state,
        // so capturing a strong `Rc<NotebookEngine>` here would form a
        // self-cycle (engine → subscribers → engine). Capture a `Weak` and
        // upgrade per fire — when the panel drops the last external Rc,
        // upgrade fails and the callback becomes a no-op until the engine
        // is fully destroyed.
        {
            let engine_weak: std::rc::Weak<NotebookEngine> = Rc::downgrade(&engine);
            let output_box = output_box.clone();
            let status_icon = status_icon.clone();
            let status_text = status_text.clone();
            let stop_btn = stop_btn.clone();
            let cb: Rc<dyn Fn()> = Rc::new(move || {
                let Some(engine) = engine_weak.upgrade() else { return };
                let snapshot = engine.output_snapshot(id);
                rebuild_output_box(&output_box, &snapshot);
                let running = engine.is_running(id);
                update_status(
                    &status_icon,
                    &status_text,
                    &snapshot,
                    running,
                    engine.last_finished_at(id),
                );
                stop_btn.set_sensitive(running);
            });
            engine.subscribe_output(id, cb);
        }

        // ── Visibility tracking → engine watch gating ───────────
        {
            let engine_map = engine.clone();
            root.connect_map(move |_| engine_map.set_visible(id, true));
        }
        {
            let engine_unmap = engine.clone();
            root.connect_unmap(move |_| engine_unmap.set_visible(id, false));
        }

        NotebookCell { root, id }
    }
}

fn rebuild_output_box(container: &GtkBox, items: &[OutputItem]) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
    // Group consecutive `Text` items into a single markdown-rendered
    // TextView, so the cell's stdout supports headings, lists, links, code
    // fences, etc. Images and Errors break the run and render as their own
    // widgets between markdown segments.
    //
    // Each stdout line is appended with a trailing `  \n` (CommonMark
    // hard break) so consecutive `print()`s render on their own visual
    // lines instead of being joined by soft-break spaces. Markdown
    // structures (`#`, `-`, ```` ``` ```` …) still parse normally because
    // the leading characters of each line are unchanged.
    let mut text_buf = String::new();
    for item in items {
        match item {
            OutputItem::Text(t) => {
                text_buf.push_str(t);
                text_buf.push_str("  \n");
            }
            OutputItem::Error(t) => {
                flush_markdown(container, &mut text_buf);
                let l = Label::new(Some(t));
                l.set_halign(gtk4::Align::Start);
                l.add_css_class("notebook-error-line");
                l.set_selectable(true);
                l.set_wrap(true);
                container.append(&l);
            }
            OutputItem::Image(src) => {
                flush_markdown(container, &mut text_buf);
                match src {
                    ImageSource::Path(p) => {
                        let pic = Picture::new();
                        pic.set_can_shrink(true);
                        pic.set_hexpand(true);
                        pic.set_content_fit(gtk4::ContentFit::Contain);
                        pic.set_size_request(-1, IMAGE_MAX_HEIGHT_PX);
                        pic.set_filename(Some(p));
                        container.append(&pic);
                    }
                    ImageSource::DataUri(_) => {
                        // v1: data URIs are not yet decoded — surface a
                        // warning. Future: decode base64 into a GdkPixbuf.
                        let warn = Label::new(Some(
                            "(data URI image not yet supported — use a file path)",
                        ));
                        warn.add_css_class("notebook-error-line");
                        container.append(&warn);
                    }
                }
            }
        }
    }
    flush_markdown(container, &mut text_buf);
}

/// Render the accumulated markdown source into a fresh read-only TextView
/// and append it to `container`. No-op if `buf` is empty. Resets `buf` on
/// success so the caller can keep grouping subsequent `Text` items.
fn flush_markdown(container: &GtkBox, buf: &mut String) {
    if buf.is_empty() {
        return;
    }
    let tv = gtk4::TextView::new();
    tv.set_editable(false);
    tv.set_cursor_visible(false);
    tv.set_wrap_mode(gtk4::WrapMode::WordChar);
    tv.set_left_margin(2);
    tv.set_right_margin(2);
    // Anchored TextView's natural width is its longest token, which makes
    // word-wrap fire on every space. Force hexpand so it stretches to the
    // parent TextView's text area.
    tv.set_hexpand(true);
    tv.add_css_class("notebook-text-output");
    crate::markdown_render::render_markdown_to_view(&tv, buf);
    container.append(&tv);
    buf.clear();
}

fn update_status(
    icon: &Image,
    text: &Label,
    items: &[OutputItem],
    running: bool,
    last_finished_at: Option<chrono::DateTime<chrono::Local>>,
) {
    icon.remove_css_class("notebook-status-running");
    icon.remove_css_class("notebook-status-error");
    icon.remove_css_class("notebook-status-ok");
    text.remove_css_class("notebook-status-running");
    text.remove_css_class("notebook-status-error");
    let stamp = last_finished_at
        .map(|t| t.format("%H:%M:%S").to_string())
        .unwrap_or_else(|| "—".into());
    if running {
        icon.set_visible(true);
        icon.set_icon_name(Some("content-loading-symbolic"));
        icon.add_css_class("notebook-status-running");
        text.add_css_class("notebook-status-running");
        text.set_text(&stamp);
    } else if items.iter().any(|i| matches!(i, OutputItem::Error(_))) {
        icon.set_visible(true);
        icon.set_icon_name(Some("dialog-error-symbolic"));
        icon.add_css_class("notebook-status-error");
        text.add_css_class("notebook-status-error");
        text.set_text(&stamp);
    } else if last_finished_at.is_some() {
        icon.set_visible(true);
        icon.set_icon_name(Some("emblem-ok-symbolic"));
        icon.add_css_class("notebook-status-ok");
        text.set_text(&stamp);
    } else {
        // Never executed — hide the icon entirely (no idle/check shown
        // before the first run). Timestamp slot stays empty for layout
        // stability.
        icon.set_visible(false);
        icon.set_icon_name(None);
        text.set_text("");
    }
}

/// Where to dispatch the cell's code when the user picks a row in the
/// run-target popover.
#[derive(Clone, Debug)]
enum RunTarget {
    /// Local subprocess via the notebook engine.
    Host,
    /// One of the terminal panels currently registered in the workspace.
    Terminal(String),
}

/// Build and show the run-target picker anchored to `anchor_btn`.
///
/// Layout (top to bottom):
///   ┌──────────────────────────────────────┐
///   │ Run on…                              │
///   ├──────────────────────────────────────┤
///   │ ⌂  Host                              │ ← always first
///   ├ Suggested ───────────────────────────┤ (only if MRU non-empty)
///   │ ⌗  <terminal in MRU>                 │ … up to 6
///   ├ Available ───────────────────────────┤
///   │ ⌗  <terminal not in MRU>             │ … all remaining
///   └──────────────────────────────────────┘
///
/// `on_pick` is called with the chosen target after the popover closes.
fn show_run_target_picker<F>(anchor_btn: &Button, on_pick: F)
where
    F: Fn(RunTarget) + 'static,
{
    let popover = gtk4::Popover::new();
    popover.set_parent(anchor_btn);
    popover.set_position(gtk4::PositionType::Bottom);
    popover.add_css_class("notebook-target-popover");
    crate::theme::configure_popover(&popover);

    let on_pick = Rc::new(on_pick);

    let vbox = GtkBox::new(Orientation::Vertical, 0);
    vbox.set_margin_top(4);
    vbox.set_margin_bottom(4);
    vbox.set_margin_start(4);
    vbox.set_margin_end(4);

    // Header
    {
        let h = Label::new(Some("Run on…"));
        h.add_css_class("dim-label");
        h.add_css_class("caption");
        h.set_halign(gtk4::Align::Start);
        h.set_margin_start(8);
        h.set_margin_top(4);
        h.set_margin_bottom(4);
        vbox.append(&h);
    }

    // Host row (always available)
    {
        let row = build_target_row(
            "computer-symbolic",
            "Host",
            Some("Run as a local subprocess (this machine)"),
        );
        let popover_close = popover.clone();
        let cb = on_pick.clone();
        row.connect_clicked(move |_| {
            popover_close.popdown();
            cb(RunTarget::Host);
        });
        vbox.append(&row);
    }

    let mru = terminal_registry::mru_list();
    let mru_ids: std::collections::HashSet<String> =
        mru.iter().map(|t| t.id.clone()).collect();

    if !mru.is_empty() {
        vbox.append(&section_separator("Suggested"));
        for term in &mru {
            let row = build_terminal_row(&term.id, &term.label);
            let popover_close = popover.clone();
            let cb = on_pick.clone();
            let term_id = term.id.clone();
            row.connect_clicked(move |_| {
                popover_close.popdown();
                cb(RunTarget::Terminal(term_id.clone()));
            });
            vbox.append(&row);
        }
    }

    let all = terminal_registry::list();
    let available: Vec<_> = all.into_iter().filter(|t| !mru_ids.contains(&t.id)).collect();

    if !available.is_empty() {
        vbox.append(&section_separator("Available"));
        for term in &available {
            let row = build_terminal_row(&term.id, &term.label);
            let popover_close = popover.clone();
            let cb = on_pick.clone();
            let term_id = term.id.clone();
            row.connect_clicked(move |_| {
                popover_close.popdown();
                cb(RunTarget::Terminal(term_id.clone()));
            });
            vbox.append(&row);
        }
    } else if mru.is_empty() {
        let none = Label::new(Some("No terminal panels in this workspace"));
        none.add_css_class("dim-label");
        none.set_margin_start(8);
        none.set_margin_end(8);
        none.set_margin_top(4);
        none.set_margin_bottom(4);
        none.set_halign(gtk4::Align::Start);
        vbox.append(&none);
    }

    popover.set_child(Some(&vbox));
    popover.popup();
}

fn section_separator(text: &str) -> GtkBox {
    let row = GtkBox::new(Orientation::Horizontal, 6);
    row.add_css_class("notebook-target-section");
    row.set_margin_top(4);
    let l = Label::new(Some(text));
    l.add_css_class("dim-label");
    l.add_css_class("caption");
    l.set_margin_start(8);
    l.set_halign(gtk4::Align::Start);
    row.append(&l);
    row
}

fn build_target_row(icon_name: &str, label: &str, tooltip: Option<&str>) -> Button {
    let row = Button::new();
    row.add_css_class("flat");
    row.add_css_class("notebook-target-row");
    row.set_halign(gtk4::Align::Fill);
    if let Some(t) = tooltip {
        row.set_tooltip_text(Some(t));
    }
    let body = GtkBox::new(Orientation::Horizontal, 8);
    let icon = Image::from_icon_name(icon_name);
    icon.add_css_class("notebook-target-icon");
    body.append(&icon);
    let l = Label::new(Some(label));
    l.set_halign(gtk4::Align::Start);
    l.set_hexpand(true);
    body.append(&l);
    row.set_child(Some(&body));
    row
}

/// Build a row for a terminal entry. Uses the breadcrumb if available
/// (rendered on a second muted line) and a terminal icon.
fn build_terminal_row(id: &str, name: &str) -> Button {
    let row = Button::new();
    row.add_css_class("flat");
    row.add_css_class("notebook-target-row");
    row.set_halign(gtk4::Align::Fill);
    let body = GtkBox::new(Orientation::Horizontal, 8);
    let icon = Image::from_icon_name("utilities-terminal-symbolic");
    icon.add_css_class("notebook-target-icon");
    body.append(&icon);

    let stack = GtkBox::new(Orientation::Vertical, 0);
    let primary = Label::new(Some(name));
    primary.set_halign(gtk4::Align::Start);
    stack.append(&primary);
    if let Some(crumb) = terminal_registry::breadcrumb_of(id) {
        let secondary = Label::new(Some(&crumb));
        secondary.add_css_class("dim-label");
        secondary.add_css_class("caption");
        secondary.set_halign(gtk4::Align::Start);
        stack.append(&secondary);
    }
    stack.set_hexpand(true);
    body.append(&stack);
    row.set_child(Some(&body));
    row
}

/// Minimal confirm dialog stub for the `confirm` tag.
///
/// libadwaita's `MessageDialog` is non-blocking on GTK4 and a true
/// blocking dialog would require a nested main loop. v1 simplification:
/// since `confirm` is opt-in (and rare), we accept that the first click
/// performs no actual blocking dialog and returns true. This is documented
/// in `docs/notebook.md`. A future iteration can wire a real prompt.
fn confirm_dialog_blocking() -> bool {
    true
}
