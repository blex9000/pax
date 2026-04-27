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

use pax_core::notebook_tag::{ExecMode, Lang};

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
        let header = GtkBox::new(Orientation::Horizontal, 6);
        header.add_css_class("notebook-cell-header");

        let lang_badge = Label::new(Some(lang_label(spec.lang)));
        lang_badge.add_css_class("notebook-lang-badge");
        header.append(&lang_badge);

        let run_btn = Button::new();
        run_btn.set_icon_name("media-playback-start-symbolic");
        run_btn.add_css_class("flat");
        run_btn.set_tooltip_text(Some("Run cell"));
        header.append(&run_btn);

        let stop_btn = Button::new();
        stop_btn.set_icon_name("media-playback-stop-symbolic");
        stop_btn.add_css_class("flat");
        stop_btn.set_tooltip_text(Some("Stop cell"));
        stop_btn.set_sensitive(false);
        header.append(&stop_btn);

        let status = Image::from_icon_name("emblem-default-symbolic");
        status.add_css_class("notebook-status-idle");
        header.append(&status);

        if let ExecMode::Watch { interval } = spec.mode {
            let label = Label::new(Some(&format!("watch every {}s", interval.as_secs_f64())));
            label.add_css_class("dim-label");
            label.add_css_class("caption");
            header.append(&label);
        }

        let spacer = Label::new(None);
        spacer.set_hexpand(true);
        header.append(&spacer);

        root.append(&header);

        // ── Output area ──────────────────────────────────────────
        let output_box = GtkBox::new(Orientation::Vertical, 2);
        output_box.add_css_class("notebook-cell-output");
        output_box.set_hexpand(true);
        root.append(&output_box);

        // ── Wire run/stop buttons ───────────────────────────────
        {
            let engine = engine.clone();
            let stop_btn = stop_btn.clone();
            let spec_for_confirm = spec.clone();
            run_btn.connect_clicked(move |_| {
                if spec_for_confirm.confirm && !engine.is_confirmed(id) {
                    if !confirm_dialog_blocking() {
                        return;
                    }
                    engine.mark_confirmed(id);
                }
                engine.run_cell(id);
                stop_btn.set_sensitive(true);
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
            let status = status.clone();
            let stop_btn = stop_btn.clone();
            let cb: Rc<dyn Fn()> = Rc::new(move || {
                let Some(engine) = engine_weak.upgrade() else { return };
                rebuild_output_box(&output_box, &engine.output_snapshot(id));
                let running = engine.is_running(id);
                update_status(&status, &engine.output_snapshot(id), running);
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

fn lang_label(l: Lang) -> &'static str {
    match l {
        Lang::Python => "python",
        Lang::Bash => "bash",
        Lang::Sh => "sh",
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
    let mut text_buf = String::new();
    for item in items {
        match item {
            OutputItem::Text(t) => {
                if !text_buf.is_empty() {
                    text_buf.push('\n');
                }
                text_buf.push_str(t);
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

fn update_status(img: &Image, items: &[OutputItem], running: bool) {
    img.remove_css_class("notebook-status-idle");
    img.remove_css_class("notebook-status-running");
    img.remove_css_class("notebook-status-ok");
    img.remove_css_class("notebook-status-error");
    if running {
        img.set_icon_name(Some("content-loading-symbolic"));
        img.add_css_class("notebook-status-running");
        return;
    }
    if items.iter().any(|i| matches!(i, OutputItem::Error(_))) {
        img.set_icon_name(Some("dialog-error-symbolic"));
        img.add_css_class("notebook-status-error");
    } else if items.is_empty() {
        img.set_icon_name(Some("emblem-default-symbolic"));
        img.add_css_class("notebook-status-idle");
    } else {
        img.set_icon_name(Some("object-select-symbolic"));
        img.add_css_class("notebook-status-ok");
    }
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
