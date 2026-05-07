//! Sync-input bridge for text-buffer-based panels.
//!
//! Translates between two worlds:
//!
//! 1. **Terminal byte stream** — what `PanelBackend::write_input` carries.
//!    Includes printable UTF-8, BS/DEL (`0x7f`/`0x08`), CR/LF, tabs, and
//!    ANSI escape sequences emitted by terminal key encoders (arrows, etc).
//!
//! 2. **GTK `TextBuffer` mutations** — `insert-text` / `delete-range` on a
//!    plain text buffer used by editors and the markdown editor.
//!
//! `apply_input_to_buffer` decodes a byte slice and applies the visible
//! effects to a buffer (printable text, newlines, single-char deletes for
//! BS/DEL). ANSI control sequences are skipped — there's no cursor-motion
//! analogue in a text editor.
//!
//! Outgoing direction is mirrored by `connect_buffer_emit_input`, which
//! hooks `insert-text` / `delete-range` on a buffer and emits matching
//! bytes through a `PanelInputCallback`. The shared `suppress` cell breaks
//! feedback loops: backends set it while applying remote input so the
//! buffer mutations they cause do not bounce back through the callback.

use std::cell::Cell;
use std::rc::Rc;

use gtk4::prelude::*;

use super::PanelInputCallback;

/// DEL byte sent by terminal backspace; treated as "delete previous char"
/// when received by a text editor. `0x08` is also accepted on the receive
/// side but we always emit `0x7f` for consistency with terminal encoders.
const BACKSPACE_BYTE: u8 = 0x7f;

/// Apply a sync-input byte slice to a `TextBuffer` while holding the
/// `suppress` flag, so the resulting buffer mutations are not re-emitted
/// by `connect_buffer_emit_input` and do not loop.
///
/// Decoding rules:
/// - `0x7f` / `0x08` → delete one cursor position before the insertion mark.
/// - `\r`, `\n`, `\r\n` → single newline.
/// - `\t` → tab.
/// - `0x1b [ … final` (CSI) → skipped; arrow keys / cursor moves have no
///   equivalent action in a passive text buffer.
/// - `0x1b X` (other ESC sequences) → ESC + next byte skipped.
/// - Other bytes `>= 0x20` or `>= 0x80` → buffered and inserted as UTF-8.
/// - All remaining C0 control bytes → dropped.
pub fn apply_input_to_buffer(buffer: &gtk4::TextBuffer, bytes: &[u8], suppress: &Rc<Cell<bool>>) {
    let prev_suppress = suppress.get();
    suppress.set(true);

    let mut pending: Vec<u8> = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == 0x1b {
            flush_text(buffer, &mut pending);
            i += 1;
            if i < bytes.len() && bytes[i] == b'[' {
                i += 1;
                while i < bytes.len() && !(0x40..=0x7e).contains(&bytes[i]) {
                    i += 1;
                }
                if i < bytes.len() {
                    i += 1;
                }
            } else if i < bytes.len() {
                i += 1;
            }
        } else if b == BACKSPACE_BYTE || b == 0x08 {
            flush_text(buffer, &mut pending);
            delete_previous_char(buffer);
            i += 1;
        } else if b == b'\r' {
            pending.push(b'\n');
            i += 1;
            if i < bytes.len() && bytes[i] == b'\n' {
                i += 1;
            }
        } else if b == b'\n' || b == b'\t' {
            pending.push(b);
            i += 1;
        } else if b >= 0x20 || b >= 0x80 {
            pending.push(b);
            i += 1;
        } else {
            i += 1;
        }
    }
    flush_text(buffer, &mut pending);

    suppress.set(prev_suppress);
}

fn flush_text(buffer: &gtk4::TextBuffer, pending: &mut Vec<u8>) {
    if pending.is_empty() {
        return;
    }
    let text = match std::str::from_utf8(pending) {
        Ok(s) => s.to_string(),
        Err(_) => String::from_utf8_lossy(pending).into_owned(),
    };
    buffer.insert_at_cursor(&text);
    pending.clear();
}

fn delete_previous_char(buffer: &gtk4::TextBuffer) {
    let mark = buffer.get_insert();
    let iter = buffer.iter_at_mark(&mark);
    let mut prev = iter;
    if prev.backward_cursor_position() {
        let mut end = iter;
        buffer.delete(&mut prev, &mut end);
    }
}

/// Wire `insert-text` / `delete-range` on `buffer` so that user-driven
/// edits emit a byte stream through `input_cb`. Programmatic mutations
/// performed while `suppress.get() == true` are ignored to break feedback
/// loops with `apply_input_to_buffer`.
///
/// `gate` is consulted before every emission and may suppress further
/// (e.g. markdown editor only emits while in Edit mode).
pub fn connect_buffer_emit_input(
    buffer: &gtk4::TextBuffer,
    input_cb: Rc<std::cell::RefCell<Option<PanelInputCallback>>>,
    suppress: Rc<Cell<bool>>,
    gate: Rc<dyn Fn() -> bool>,
) {
    {
        let input_cb = input_cb.clone();
        let suppress = suppress.clone();
        let gate = gate.clone();
        buffer.connect_insert_text(move |_buf, _iter, text| {
            if suppress.get() || !gate() {
                return;
            }
            let bytes = text.as_bytes();
            if bytes.is_empty() {
                return;
            }
            if let Ok(borrowed) = input_cb.try_borrow() {
                if let Some(ref cb) = *borrowed {
                    cb(bytes);
                }
            }
        });
    }
    {
        buffer.connect_delete_range(move |_buf, start, end| {
            if suppress.get() || !gate() {
                return;
            }
            let count = (end.offset() - start.offset()).max(0) as usize;
            if count == 0 {
                return;
            }
            let bytes = vec![BACKSPACE_BYTE; count];
            if let Ok(borrowed) = input_cb.try_borrow() {
                if let Some(ref cb) = *borrowed {
                    cb(&bytes);
                }
            }
        });
    }
}
