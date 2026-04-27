//! Process-wide registry of terminal panels currently alive in the
//! workspace, used by markdown notebook cells to discover and feed code
//! into a chosen terminal via the cell's "Send to terminal" button.
//!
//! Ownership model: the terminal panel registers itself on construction
//! (passing a closure that wraps `write_input`) and unregisters in
//! `shutdown()`. Consumers (notebook cells) call `list()` / `send()` —
//! they never hold a reference to the panel directly, so a closed panel
//! simply disappears from the next list snapshot.
//!
//! Thread-locality: GTK is single-threaded; the registry uses
//! `thread_local!` to avoid sync overhead.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

#[derive(Clone, Debug)]
pub struct TerminalRef {
    pub id: String,
    pub label: String,
}

struct Entry {
    label: String,
    send: Rc<dyn Fn(&[u8]) -> bool>,
}

thread_local! {
    static REGISTRY: RefCell<HashMap<String, Entry>> = RefCell::new(HashMap::new());
}

/// Register (or replace) a terminal under `id` with a user-facing `label`
/// and a sender. The sender is invoked by `send()` and should pass its
/// argument to the terminal's underlying PTY (typically `write_input`).
pub fn register(id: &str, label: &str, send: Rc<dyn Fn(&[u8]) -> bool>) {
    REGISTRY.with(|r| {
        r.borrow_mut().insert(
            id.to_string(),
            Entry {
                label: label.to_string(),
                send,
            },
        );
    });
}

/// Update only the user-facing label for an already-registered terminal
/// (e.g. after the panel's footer cwd changed). No-op if `id` is absent.
pub fn relabel(id: &str, label: &str) {
    REGISTRY.with(|r| {
        if let Some(e) = r.borrow_mut().get_mut(id) {
            e.label = label.to_string();
        }
    });
}

pub fn unregister(id: &str) {
    REGISTRY.with(|r| {
        r.borrow_mut().remove(id);
    });
}

/// Snapshot of all currently-registered terminals, sorted by id for a
/// stable display order.
pub fn list() -> Vec<TerminalRef> {
    REGISTRY.with(|r| {
        let map = r.borrow();
        let mut v: Vec<TerminalRef> = map
            .iter()
            .map(|(id, e)| TerminalRef {
                id: id.clone(),
                label: e.label.clone(),
            })
            .collect();
        v.sort_by(|a, b| a.id.cmp(&b.id));
        v
    })
}

/// Returns true if the bytes were delivered. False if `id` is unknown or
/// the registered sender returned false (e.g. the underlying panel has
/// rejected the write).
pub fn send(id: &str, data: &[u8]) -> bool {
    REGISTRY.with(|r| {
        let map = r.borrow();
        if let Some(e) = map.get(id) {
            (e.send)(data)
        } else {
            false
        }
    })
}
