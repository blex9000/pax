//! Per-source-tab note state: the live set of `FileNote` records attached
//! to a buffer, each anchored by a `gtk::TextMark` that GTK moves along
//! with edits. Loading resolves DB rows to marks; saving flushes the
//! current mark positions + line contents back to the DB so the next open
//! is robust to edits the user made during the session.

use gtk4::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

use pax_db::notes::FileNote;

/// Fuzzy-match window: on reload, if the saved line_number no longer
/// matches the anchor text, scan this many lines above and below looking
/// for the anchor's exact content.
pub const ANCHOR_FUZZY_RADIUS: i32 = 20;

/// A single live note attached to a buffer. `mark` is `None` when the
/// note couldn't be resolved to any current line in the buffer (orphan).
#[derive(Debug, Clone)]
pub struct LiveNote {
    pub db_id: i64,
    pub text: String,
    pub saved_line: i32,
    pub saved_anchor: Option<String>,
    pub mark: Option<gtk4::TextMark>,
}

/// Holds every note currently loaded for a source tab.
#[derive(Debug, Default, Clone)]
pub struct NotesState {
    pub entries: Rc<RefCell<Vec<LiveNote>>>,
}

impl NotesState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Current note lines (0-based) for ruler painting.
    pub fn current_lines(&self, buffer: &sourceview5::Buffer) -> Vec<i32> {
        let entries = self.entries.borrow();
        entries
            .iter()
            .filter_map(|e| e.mark.as_ref().map(|m| line_of_mark(buffer, m)))
            .collect()
    }

    pub fn push(&self, note: LiveNote) {
        self.entries.borrow_mut().push(note);
    }

    /// Remove a note by id. Also deletes its mark from the buffer when present.
    pub fn remove(&self, db_id: i64, buffer: &sourceview5::Buffer) {
        let mut entries = self.entries.borrow_mut();
        if let Some(pos) = entries.iter().position(|e| e.db_id == db_id) {
            let removed = entries.remove(pos);
            if let Some(mark) = removed.mark {
                buffer.delete_mark(&mark);
            }
        }
    }

    /// Update a note's text in place.
    pub fn set_text(&self, db_id: i64, new_text: &str) {
        for entry in self.entries.borrow_mut().iter_mut() {
            if entry.db_id == db_id {
                entry.text = new_text.to_string();
            }
        }
    }

    /// Notes whose mark currently sits on `line`.
    pub fn notes_on_line(
        &self,
        buffer: &sourceview5::Buffer,
        line: i32,
    ) -> Vec<LiveNote> {
        let entries = self.entries.borrow();
        entries
            .iter()
            .filter(|e| {
                e.mark
                    .as_ref()
                    .map(|m| line_of_mark(buffer, m) == line)
                    .unwrap_or(false)
            })
            .cloned()
            .collect()
    }
}

/// Attach DB-loaded notes to a buffer. For each FileNote, resolve its line
/// via exact (line_number + anchor) match; fall back to a ±ANCHOR_FUZZY_RADIUS
/// scan by anchor content. Notes that can't be resolved stay as orphans
/// (mark = None) but remain in the state so they show up in the Notes list.
pub fn apply_loaded_notes(
    state: &NotesState,
    buffer: &sourceview5::Buffer,
    notes: Vec<FileNote>,
) {
    for note in notes {
        let resolved = resolve_anchor(buffer, note.line_number, note.line_anchor.as_deref());
        let mark = resolved.map(|line| create_mark_at_line(buffer, line));
        state.push(LiveNote {
            db_id: note.id,
            text: note.text,
            saved_line: note.line_number,
            saved_anchor: note.line_anchor,
            mark,
        });
    }
}

/// Create a `TextMark` at the start of `line`. `left_gravity = true`
/// means text typed before the mark pushes it right, matching "the note
/// lives at the start of this line".
pub fn create_mark_at_line(buffer: &sourceview5::Buffer, line: i32) -> gtk4::TextMark {
    let iter = buffer
        .iter_at_line(line)
        .unwrap_or_else(|| buffer.start_iter());
    buffer.create_mark(None, &iter, true)
}

/// 0-based line number of a mark's current position.
pub fn line_of_mark(buffer: &sourceview5::Buffer, mark: &gtk4::TextMark) -> i32 {
    buffer.iter_at_mark(mark).line()
}

/// Content of `line` in `buffer`, without the trailing newline.
pub fn line_content(buffer: &sourceview5::Buffer, line: i32) -> String {
    let Some(start) = buffer.iter_at_line(line) else {
        return String::new();
    };
    let mut end = start.clone();
    if !end.ends_line() {
        end.forward_to_line_end();
    }
    buffer.text(&start, &end, false).to_string()
}

fn resolve_anchor(
    buffer: &sourceview5::Buffer,
    saved_line: i32,
    anchor: Option<&str>,
) -> Option<i32> {
    let total = buffer.line_count();
    if saved_line < 0 || saved_line >= total {
        return fuzzy_find(buffer, anchor, saved_line);
    }
    let at_saved = line_content(buffer, saved_line);
    if anchor.is_none() || anchor == Some(at_saved.as_str()) {
        return Some(saved_line);
    }
    fuzzy_find(buffer, anchor, saved_line)
}

fn fuzzy_find(
    buffer: &sourceview5::Buffer,
    anchor: Option<&str>,
    center: i32,
) -> Option<i32> {
    let anchor = anchor?;
    if anchor.trim().is_empty() {
        // Don't fuzzy-match against blank lines — too many false hits.
        return None;
    }
    let total = buffer.line_count();
    for offset in 1..=ANCHOR_FUZZY_RADIUS {
        for candidate in [center - offset, center + offset] {
            if candidate < 0 || candidate >= total {
                continue;
            }
            if line_content(buffer, candidate) == anchor {
                return Some(candidate);
            }
        }
    }
    None
}
