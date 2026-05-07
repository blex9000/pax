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

const MRU_LIMIT: usize = 6;

thread_local! {
    static REGISTRY: RefCell<HashMap<String, Entry>> = RefCell::new(HashMap::new());
    /// Hierarchical breadcrumb (e.g. "root › tab1 › left › shell")
    /// per panel id. Populated by `WorkspaceView` and shown in the cell
    /// run-target picker.
    static BREADCRUMBS: RefCell<HashMap<String, String>> = RefCell::new(HashMap::new());
    /// Most-recently-used panel ids (newest first), capped at MRU_LIMIT.
    /// Populated by the picker when the user picks a target. Process-wide,
    /// resets on app restart.
    static MRU: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
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

/// Set or update the hierarchical breadcrumb (e.g.
/// "root › tab1 › left › shell") for a panel. Used by the run-target
/// picker to disambiguate panels with the same short name.
pub fn set_breadcrumb(id: &str, breadcrumb: &str) {
    BREADCRUMBS.with(|b| {
        b.borrow_mut()
            .insert(id.to_string(), breadcrumb.to_string());
    });
}

pub fn breadcrumb_of(id: &str) -> Option<String> {
    BREADCRUMBS.with(|b| b.borrow().get(id).cloned())
}

/// Push a panel id to the front of the MRU list, deduping. Trims to
/// `MRU_LIMIT` entries.
pub fn mru_record(id: &str) {
    MRU.with(|m| {
        let mut v = m.borrow_mut();
        v.retain(|x| x != id);
        v.insert(0, id.to_string());
        if v.len() > MRU_LIMIT {
            v.truncate(MRU_LIMIT);
        }
    });
}

/// Snapshot of the MRU list, newest first. Includes only ids still
/// present in the live registry — stale entries (panel closed) are
/// silently dropped from the returned slice.
pub fn mru_list() -> Vec<TerminalRef> {
    let ids: Vec<String> = MRU.with(|m| m.borrow().clone());
    REGISTRY.with(|r| {
        let map = r.borrow();
        ids.into_iter()
            .filter_map(|id| {
                map.get(&id).map(|e| TerminalRef {
                    id,
                    label: e.label.clone(),
                })
            })
            .collect()
    })
}
