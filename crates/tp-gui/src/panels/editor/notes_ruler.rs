//! Note-indicator drawing area. Mirrors the match-ruler pattern in
//! `editor_tabs::build_match_overview_ruler` but paints small amber dots
//! at every line carrying a note in the active source tab, and exposes a
//! click-to-jump gesture against whatever callback the owner wires up.

use gtk4::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

pub const NOTES_RULER_WIDTH: i32 = 14;
const NOTES_BG_ALPHA: f64 = 0.04;
const NOTES_DOT_RADIUS: f64 = 3.0;
const NOTES_DOT_R: f64 = 0.96;
const NOTES_DOT_G: f64 = 0.78;
const NOTES_DOT_B: f64 = 0.25;

pub type OnNoteJump = Rc<RefCell<Option<Box<dyn Fn(i32)>>>>;

pub struct NotesRuler {
    pub widget: gtk4::DrawingArea,
    lines: Rc<RefCell<Vec<i32>>>,
    total_lines: Rc<RefCell<i32>>,
    on_jump: OnNoteJump,
}

impl NotesRuler {
    pub fn new() -> Self {
        let widget = gtk4::DrawingArea::new();
        widget.set_width_request(NOTES_RULER_WIDTH);
        widget.set_vexpand(true);
        widget.add_css_class("editor-notes-ruler");
        widget.set_tooltip_text(Some("Click a note marker to jump to it"));
        widget.set_cursor_from_name(Some("pointer"));

        let lines: Rc<RefCell<Vec<i32>>> = Rc::new(RefCell::new(Vec::new()));
        let total_lines: Rc<RefCell<i32>> = Rc::new(RefCell::new(1));
        let on_jump: OnNoteJump = Rc::new(RefCell::new(None));

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

        // Click-to-jump: on click, pick the nearest painted dot's line and
        // fire the owner's callback. Owner wires it to scroll the source
        // view + place the cursor. Matches the match ruler's UX.
        {
            let lines_c = lines.clone();
            let total_c = total_lines.clone();
            let widget_for_click = widget.clone();
            let jump = on_jump.clone();
            let gesture = gtk4::GestureClick::new();
            gesture.set_button(1);
            gesture.connect_pressed(move |_, _n, _x, y| {
                let h = widget_for_click.height().max(1) as f64;
                let total = (*total_c.borrow()).max(1) as f64;
                let ls = lines_c.borrow();
                if ls.is_empty() {
                    return;
                }
                let clicked = ((y / h).clamp(0.0, 1.0) * total) as i32;
                let Some(target) =
                    ls.iter().copied().min_by_key(|l| (*l - clicked).abs())
                else {
                    return;
                };
                if let Some(cb) = jump.borrow().as_ref() {
                    cb(target);
                }
            });
            widget.add_controller(gesture);
        }

        Self {
            widget,
            lines,
            total_lines,
            on_jump,
        }
    }

    /// Register a callback invoked with the 0-based line number of the
    /// nearest note dot whenever the user clicks on the ruler.
    pub fn set_jump_callback(&self, cb: impl Fn(i32) + 'static) {
        *self.on_jump.borrow_mut() = Some(Box::new(cb));
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

    /// Given a pixel y coordinate inside the widget, return the buffer
    /// line closest to a painted dot, or `None` when no dots exist.
    pub fn nearest_line(&self, y: f64, height_px: f64) -> Option<i32> {
        let lines = self.lines.borrow();
        if lines.is_empty() {
            return None;
        }
        let total = (*self.total_lines.borrow()).max(1) as f64;
        let clicked = ((y / height_px).clamp(0.0, 1.0) * total) as i32;
        lines.iter().copied().min_by_key(|l| (*l - clicked).abs())
    }
}
