//! Note-indicator drawing area. Mirrors the match-ruler pattern in
//! `editor_tabs::build_match_overview_ruler` but paints small amber dots
//! at every line carrying a note in the active source tab, and exposes a
//! click-to-jump gesture against whatever callback the owner wires up.

use gtk4::prelude::*;
use sourceview5::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

pub const NOTES_RULER_WIDTH: i32 = 14;
const NOTES_BG_ALPHA: f64 = 0.04;
const NOTES_DOT_RADIUS: f64 = 3.0;
const NOTES_DOT_R: f64 = 0.96;
const NOTES_DOT_G: f64 = 0.78;
const NOTES_DOT_B: f64 = 0.25;
/// Vertical pixel tolerance: the tooltip only shows when the pointer is
/// this close to a painted dot, so hovering on empty ruler space doesn't
/// resolve to a random nearby note.
const NOTES_TOOLTIP_HIT_RADIUS_PX: f64 = 6.0;

pub type OnNoteTooltip = Rc<RefCell<Option<Box<dyn Fn(i32) -> Option<String>>>>>;

pub struct NotesRuler {
    pub widget: gtk4::DrawingArea,
    lines: Rc<RefCell<Vec<i32>>>,
    total_lines: Rc<RefCell<i32>>,
    on_tooltip: OnNoteTooltip,
}

impl std::fmt::Debug for NotesRuler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NotesRuler")
            .field("lines", &self.lines.borrow().len())
            .finish()
    }
}

impl NotesRuler {
    pub fn new(view: sourceview5::View) -> Self {
        let widget = gtk4::DrawingArea::new();
        widget.set_width_request(NOTES_RULER_WIDTH);
        widget.set_vexpand(true);
        widget.add_css_class("editor-notes-ruler");
        // Default cursor; the motion controller below swaps to "pointer"
        // only when the pointer is close to a note dot.
        widget.set_cursor_from_name(Some("default"));

        let lines: Rc<RefCell<Vec<i32>>> = Rc::new(RefCell::new(Vec::new()));
        let total_lines: Rc<RefCell<i32>> = Rc::new(RefCell::new(1));
        let on_tooltip: OnNoteTooltip = Rc::new(RefCell::new(None));

        {
            let lines = lines.clone();
            let total = total_lines.clone();
            widget.set_draw_func(move |_, cr, w, h| {
                let w_f = w as f64;
                let h_f = h as f64;
                cr.set_source_rgba(0.5, 0.5, 0.5, NOTES_BG_ALPHA);
                let _ = cr.paint();
                let total = (*total.borrow()).max(1) as f64;
                cr.set_source_rgba(NOTES_DOT_R, NOTES_DOT_G, NOTES_DOT_B, 0.95);
                for &line in lines.borrow().iter() {
                    let y = (line as f64 / total) * h_f;
                    let cx = w_f / 2.0;
                    cr.arc(cx, y + NOTES_DOT_RADIUS, NOTES_DOT_RADIUS, 0.0, std::f64::consts::TAU);
                    let _ = cr.fill();
                }
            });
        }

        // Click-to-jump: exact mirror of build_match_overview_ruler. The
        // view is captured by clone and we call scroll_to_iter directly,
        // no callback indirection.
        {
            let view = view.clone();
            let lines = lines.clone();
            let bar_for_click = widget.clone();
            let gesture = gtk4::GestureClick::new();
            gesture.connect_pressed(move |_, _n, _x, y| {
                let total = view.buffer().line_count().max(1);
                let h = bar_for_click.height().max(1) as f64;
                let proportion = (y / h).clamp(0.0, 1.0);
                let clicked = (proportion * total as f64) as i32;
                let ls = lines.borrow();
                if ls.is_empty() {
                    return;
                }
                let target = ls
                    .iter()
                    .copied()
                    .min_by_key(|l| (*l - clicked).abs())
                    .unwrap_or(clicked);
                let buf = view.buffer();
                if let Some(iter) = buf.iter_at_line(target) {
                    buf.place_cursor(&iter);
                    view.scroll_to_iter(&mut iter.clone(), 0.1, true, 0.5, 0.5);
                }
            });
            widget.add_controller(gesture);
        }

        // Pointer cursor only when hovering near a note dot. Avoids the
        // "whole ruler is clickable" feel when 99% of the column is empty.
        {
            let lines_c = lines.clone();
            let total_c = total_lines.clone();
            let widget_for_motion = widget.clone();
            let motion = gtk4::EventControllerMotion::new();
            motion.connect_motion(move |_, _x, y| {
                let ls = lines_c.borrow();
                if ls.is_empty() {
                    widget_for_motion.set_cursor_from_name(Some("default"));
                    return;
                }
                let h = widget_for_motion.height().max(1) as f64;
                let total = (*total_c.borrow()).max(1) as f64;
                let clicked = ((y / h).clamp(0.0, 1.0) * total) as i32;
                let Some(target) =
                    ls.iter().copied().min_by_key(|l| (*l - clicked).abs())
                else {
                    widget_for_motion.set_cursor_from_name(Some("default"));
                    return;
                };
                let target_y = (target as f64 / total) * h;
                if (y - target_y).abs() <= NOTES_TOOLTIP_HIT_RADIUS_PX {
                    widget_for_motion.set_cursor_from_name(Some("pointer"));
                } else {
                    widget_for_motion.set_cursor_from_name(Some("default"));
                }
            });
            widget.add_controller(motion);
        }

        // Hover tooltip: on pointer movement, look up the nearest note dot
        // and ask the owner for its text.
        {
            let lines_c = lines.clone();
            let total_c = total_lines.clone();
            let widget_for_tip = widget.clone();
            let tooltip_cb = on_tooltip.clone();
            widget.set_has_tooltip(true);
            widget.connect_query_tooltip(move |_, _x, y, _keyboard, tooltip| {
                let ls = lines_c.borrow();
                if ls.is_empty() {
                    return false;
                }
                let h = widget_for_tip.height().max(1) as f64;
                let total = (*total_c.borrow()).max(1) as f64;
                let clicked = ((y as f64 / h).clamp(0.0, 1.0) * total) as i32;
                let Some(target) =
                    ls.iter().copied().min_by_key(|l| (*l - clicked).abs())
                else {
                    return false;
                };
                // Only show if the click is within a small pixel radius of
                // the dot — otherwise the entire column would show a
                // tooltip.
                let target_y = (target as f64 / total) * h;
                if (y as f64 - target_y).abs() > NOTES_TOOLTIP_HIT_RADIUS_PX {
                    return false;
                }
                let text = tooltip_cb
                    .borrow()
                    .as_ref()
                    .and_then(|cb| cb(target));
                match text {
                    Some(t) => {
                        tooltip.set_text(Some(&t));
                        true
                    }
                    None => false,
                }
            });
        }

        Self {
            widget,
            lines,
            total_lines,
            on_tooltip,
        }
    }

    /// Register a callback used to resolve a line number to the note text
    /// for tooltip display. Return `None` to suppress the tooltip.
    pub fn set_tooltip_callback(&self, cb: impl Fn(i32) -> Option<String> + 'static) {
        *self.on_tooltip.borrow_mut() = Some(Box::new(cb));
    }

    /// Refresh with the current set of note lines for a buffer.
    pub fn update(&self, new_lines: Vec<i32>, total_buffer_lines: i32) {
        *self.lines.borrow_mut() = new_lines;
        *self.total_lines.borrow_mut() = total_buffer_lines.max(1);
        let has_any = !self.lines.borrow().is_empty();
        self.widget.set_visible(has_any);
        self.widget.queue_draw();
    }

    pub fn clear(&self) {
        self.lines.borrow_mut().clear();
        self.widget.set_visible(false);
        self.widget.queue_draw();
    }

}
