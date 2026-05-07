use gtk4::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Debug, Clone, Copy)]
pub(super) enum OverviewRulerKind {
    /// Red marks for deleted lines (on the PREVIOUS side).
    Delete,
    /// Green marks for inserted lines (on the CURRENT side).
    Insert,
    /// Gold marks for search-match lines (main editor overview).
    Match,
}

/// Pixel width of the overview ruler strip. Wide enough to be tappable
/// without crowding the editor's scrollbar.
const OVERVIEW_RULER_WIDTH: i32 = 10;
/// Minimum pixel height of a single marker so it stays visible and clickable
/// even in very long files where each line would otherwise collapse to
/// sub-pixel size.
const OVERVIEW_RULER_MARK_MIN_HEIGHT: f64 = 2.0;
/// Alpha for the neutral backdrop behind the marks. Low enough to blend
/// with the surrounding chrome but present so the strip has a visual
/// identity even when the file has no changes.
const OVERVIEW_RULER_BG_ALPHA: f64 = 0.05;

fn overview_ruler_color(kind: OverviewRulerKind) -> (f64, f64, f64) {
    // Match the rgba fills already used for diff-del / diff-add paragraph
    // backgrounds so the minimap reads as the same language as the inline
    // highlighting.
    match kind {
        OverviewRulerKind::Delete => (220.0 / 255.0, 50.0 / 255.0, 47.0 / 255.0),
        OverviewRulerKind::Insert => (40.0 / 255.0, 180.0 / 255.0, 60.0 / 255.0),
        // Gold, matches the `#e5a50a` highlight used for search matches.
        OverviewRulerKind::Match => (229.0 / 255.0, 165.0 / 255.0, 10.0 / 255.0),
    }
}

/// Build a narrow clickable strip that shows every changed line at its
/// proportional position in the file. Clicking a marker (or anywhere in the
/// strip) scrolls `view` to the nearest change and places the cursor there.
pub(super) fn build_overview_ruler(
    change_lines: Vec<i32>,
    total_lines: i32,
    kind: OverviewRulerKind,
    view: &sourceview5::View,
) -> gtk4::DrawingArea {
    let bar = gtk4::DrawingArea::new();
    bar.set_width_request(OVERVIEW_RULER_WIDTH);
    bar.set_vexpand(true);
    bar.add_css_class("diff-overview-ruler");
    bar.set_tooltip_text(Some("Click a marker to jump to that change"));
    bar.set_cursor_from_name(Some("pointer"));

    let lines = Rc::new(change_lines);
    let total = total_lines.max(1);

    {
        let lines = lines.clone();
        bar.set_draw_func(move |_, cr, w, h| {
            let (r, g, b) = overview_ruler_color(kind);
            let h_f = h as f64;
            let w_f = w as f64;
            cr.set_source_rgba(0.5, 0.5, 0.5, OVERVIEW_RULER_BG_ALPHA);
            let _ = cr.paint();
            cr.set_source_rgba(r, g, b, 0.9);
            let mark_h = (h_f / total as f64).max(OVERVIEW_RULER_MARK_MIN_HEIGHT);
            for &line in lines.iter() {
                let y = (line as f64 / total as f64) * h_f;
                cr.rectangle(0.0, y, w_f, mark_h);
            }
            let _ = cr.fill();
        });
    }

    {
        let view = view.clone();
        let lines = lines.clone();
        let bar_for_click = bar.clone();
        let gesture = gtk4::GestureClick::new();
        gesture.set_propagation_phase(gtk4::PropagationPhase::Capture);
        gesture.connect_pressed(move |g, _n, _x, y| {
            // Claim the event so the enclosing Paned doesn't start a
            // drag-to-resize on the same press.
            g.set_state(gtk4::EventSequenceState::Claimed);
            let h = bar_for_click.height().max(1) as f64;
            let proportion = (y / h).clamp(0.0, 1.0);
            let clicked = (proportion * total as f64) as i32;
            // Snap to the nearest known change so clicking the backdrop
            // between two markers still lands on a real change.
            let target = lines
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
        bar.add_controller(gesture);
    }

    bar
}

/// Like `build_overview_ruler` but the marked lines and total line count are
/// re-read on every draw, so the ruler can follow the active buffer as the
/// user edits, switches tabs, or changes the search query. `lines` is shared
/// state the caller mutates; after mutating, call `queue_draw` on the returned
/// widget to repaint.
pub(super) fn build_match_overview_ruler(
    lines: Rc<RefCell<Vec<i32>>>,
    kind: OverviewRulerKind,
    active_view: Rc<dyn Fn() -> sourceview5::View>,
) -> gtk4::DrawingArea {
    let bar = gtk4::DrawingArea::new();
    bar.set_width_request(OVERVIEW_RULER_WIDTH);
    bar.set_vexpand(true);
    bar.add_css_class("editor-match-ruler");
    bar.set_tooltip_text(Some("Click a marker to jump to that match"));
    bar.set_cursor_from_name(Some("pointer"));

    {
        let lines = lines.clone();
        let av = active_view.clone();
        bar.set_draw_func(move |_, cr, w, h| {
            let total = av().buffer().line_count().max(1);
            let (r, g, b) = overview_ruler_color(kind);
            let h_f = h as f64;
            let w_f = w as f64;
            cr.set_source_rgba(0.5, 0.5, 0.5, OVERVIEW_RULER_BG_ALPHA);
            let _ = cr.paint();
            cr.set_source_rgba(r, g, b, 0.9);
            let mark_h = (h_f / total as f64).max(OVERVIEW_RULER_MARK_MIN_HEIGHT);
            let ls = lines.borrow();
            for &line in ls.iter() {
                let y = (line as f64 / total as f64) * h_f;
                cr.rectangle(0.0, y, w_f, mark_h);
            }
            let _ = cr.fill();
        });
    }

    {
        let av = active_view.clone();
        let lines = lines.clone();
        let bar_for_click = bar.clone();
        let gesture = gtk4::GestureClick::new();
        gesture.connect_pressed(move |_, _n, _x, y| {
            let view = av();
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
        bar.add_controller(gesture);
    }

    bar
}

/// Scan `buf` for `query` (case-insensitive, substring) and return the 0-based
/// line numbers of every matching line. Used to populate the match overview
/// ruler without depending on a `SearchContext`.
pub(super) fn collect_match_lines(buf: &sourceview5::Buffer, query: &str) -> Vec<i32> {
    if query.is_empty() {
        return Vec::new();
    }
    let start = buf.start_iter();
    let end = buf.end_iter();
    let text = buf.text(&start, &end, true).to_string();
    let needle = query.to_lowercase();
    let mut out = Vec::new();
    for (idx, line) in text.split('\n').enumerate() {
        if line.to_lowercase().contains(&needle) {
            out.push(idx as i32);
        }
    }
    out
}
