//! Notebook engine: per-MarkdownPanel lazy-instantiated coordinator.
//!
//! Owns:
//!   - registered cells (spec + code), addressed by `CellId`
//!   - per-cell `RunHandle` (Some = running, None = idle)
//!   - per-cell output history (`Vec<OutputItem>`)
//!   - watch scheduler timers (one per watch cell)
//!   - subscriber callbacks (cell_id -> Vec<Rc<dyn Fn()>>) invoked on
//!     output change so the cell widget re-renders
//!   - helpers bootstrap state (extract Python helpers once per engine)
//!
//! All public methods are intended to be called from the GTK main thread.
//! Internally a single mpsc receiver per running cell is drained from a
//! `glib::idle_add_local` loop installed at spawn time, so widget updates
//! happen on the main thread.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::{Rc, Weak};
use std::time::Duration;

/// Drain bound for `attach_rx` per idle tick. Caps the work done in a
/// single GTK main-loop iteration so a fast-emitting subprocess can't
/// freeze the UI; lines beyond this are picked up on the next tick.
const MAX_DRAIN_PER_TICK: usize = 32;

use gtk4::glib;

use pax_core::notebook_tag::{ExecMode, NotebookCellSpec};

use super::output::OutputItem;
use super::runner::{self, RunHandle, RunMsg};
use super::MAX_NOTEBOOK_PROCESSES;

pub type CellId = u64;

#[derive(Default)]
struct CellState {
    spec: Option<NotebookCellSpec>,
    code: String,
    output: Vec<OutputItem>,
    handle: Option<RunHandle>,
    visible: bool,
    confirmed: bool, // for `confirm` tag, sticky once user accepts
    watch_source: Option<glib::SourceId>,
    subscribers: Vec<Rc<dyn Fn()>>,
}

pub struct NotebookEngine {
    next_id: Cell<CellId>,
    cells: RefCell<HashMap<CellId, CellState>>,
    helpers_dir: RefCell<Option<PathBuf>>,
    output_dir: PathBuf,
}

impl NotebookEngine {
    pub fn new() -> Rc<Self> {
        let tmp = std::env::temp_dir().join(format!("pax-notebook-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        Rc::new(Self {
            next_id: Cell::new(1),
            cells: RefCell::new(HashMap::new()),
            helpers_dir: RefCell::new(None),
            output_dir: tmp,
        })
    }

    pub fn register_cell(&self, spec: NotebookCellSpec, code: String) -> CellId {
        let id = self.next_id.get();
        self.next_id.set(id + 1);
        let mut cells = self.cells.borrow_mut();
        cells.insert(
            id,
            CellState {
                spec: Some(spec),
                code,
                ..Default::default()
            },
        );
        id
    }

    pub fn spec_of(&self, id: CellId) -> Option<NotebookCellSpec> {
        self.cells.borrow().get(&id).and_then(|c| c.spec.clone())
    }

    pub fn output_snapshot(&self, id: CellId) -> Vec<OutputItem> {
        self.cells
            .borrow()
            .get(&id)
            .map(|c| c.output.clone())
            .unwrap_or_default()
    }

    pub fn subscribe_output(&self, id: CellId, cb: Rc<dyn Fn()>) {
        if let Some(c) = self.cells.borrow_mut().get_mut(&id) {
            c.subscribers.push(cb);
        }
    }

    pub fn is_running(&self, id: CellId) -> bool {
        self.cells
            .borrow()
            .get(&id)
            .map(|c| c.handle.is_some())
            .unwrap_or(false)
    }

    pub fn mark_confirmed(&self, id: CellId) {
        if let Some(c) = self.cells.borrow_mut().get_mut(&id) {
            c.confirmed = true;
        }
    }

    pub fn is_confirmed(&self, id: CellId) -> bool {
        self.cells.borrow().get(&id).map(|c| c.confirmed).unwrap_or(false)
    }

    /// Notify the engine the cell widget became (in)visible — gates watch.
    pub fn set_visible(self: &Rc<Self>, id: CellId, visible: bool) {
        let mut cells = self.cells.borrow_mut();
        let Some(state) = cells.get_mut(&id) else { return };
        state.visible = visible;
        let spec = state.spec.clone();
        drop(cells);
        match spec.map(|s| s.mode) {
            Some(ExecMode::Watch { interval }) => {
                if visible {
                    self.start_watch(id, interval);
                } else {
                    self.stop_watch(id);
                }
            }
            _ => {}
        }
    }

    pub fn run_cell(self: &Rc<Self>, id: CellId) {
        // Skip if already running.
        if self.is_running(id) {
            return;
        }
        // Global concurrency cap.
        let active = self.cells.borrow().values().filter(|c| c.handle.is_some()).count();
        if active >= MAX_NOTEBOOK_PROCESSES {
            self.push_output(
                id,
                OutputItem::Error(format!(
                    "notebook process limit reached ({})",
                    MAX_NOTEBOOK_PROCESSES
                )),
            );
            return;
        }
        let (spec, code) = {
            let cells = self.cells.borrow();
            let Some(state) = cells.get(&id) else { return };
            let Some(spec) = state.spec.clone() else { return };
            (spec, state.code.clone())
        };
        let helpers = self.ensure_helpers();
        match runner::spawn(&spec, &code, helpers.as_deref(), Some(&self.output_dir)) {
            Ok((handle, rx)) => {
                // Install handle first so subscribers fired by clear_output
                // observe is_running == true (the runner is already alive).
                if let Some(c) = self.cells.borrow_mut().get_mut(&id) {
                    c.handle = Some(handle);
                }
                self.clear_output(id);
                self.attach_rx(id, rx);
            }
            Err(reason) => {
                self.push_output(id, OutputItem::Error(format!("blocked: {}", reason)));
            }
        }
    }

    pub fn stop_cell(&self, id: CellId) {
        if let Some(c) = self.cells.borrow_mut().get_mut(&id) {
            // Dropping the handle triggers SIGTERM/SIGKILL via RunHandle::drop.
            c.handle = None;
        }
    }

    fn attach_rx(self: &Rc<Self>, id: CellId, rx: std::sync::mpsc::Receiver<RunMsg>) {
        // Capture a `Weak` so the engine can be dropped while glib still
        // holds this idle source: the next tick `upgrade()` returns None
        // and the source returns Break, releasing the closure cleanly.
        let me_weak: Weak<Self> = Rc::downgrade(self);
        glib::idle_add_local(move || {
            let Some(me) = me_weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            for _ in 0..MAX_DRAIN_PER_TICK {
                match rx.try_recv() {
                    Ok(RunMsg::Output(item)) => me.push_output(id, item),
                    Ok(RunMsg::Finished { .. }) => {
                        me.detach_handle(id);
                        return glib::ControlFlow::Break;
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => {
                        return glib::ControlFlow::Continue;
                    }
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        me.detach_handle(id);
                        return glib::ControlFlow::Break;
                    }
                }
            }
            glib::ControlFlow::Continue
        });
    }

    fn detach_handle(&self, id: CellId) {
        if let Some(c) = self.cells.borrow_mut().get_mut(&id) {
            c.handle = None;
        }
        self.notify(id);
    }

    fn push_output(&self, id: CellId, item: OutputItem) {
        let subs = {
            let mut cells = self.cells.borrow_mut();
            let Some(state) = cells.get_mut(&id) else { return };
            state.output.push(item);
            state.subscribers.clone()
        };
        for s in subs {
            s();
        }
    }

    fn clear_output(&self, id: CellId) {
        if let Some(c) = self.cells.borrow_mut().get_mut(&id) {
            c.output.clear();
        }
        self.notify(id);
    }

    fn notify(&self, id: CellId) {
        let subs = self
            .cells
            .borrow()
            .get(&id)
            .map(|c| c.subscribers.clone())
            .unwrap_or_default();
        for s in subs {
            s();
        }
    }

    fn start_watch(self: &Rc<Self>, id: CellId, interval: Duration) {
        // Cancel any existing timer first (re-entry).
        self.stop_watch(id);
        // Capture a `Weak` so the engine isn't kept alive solely by this
        // periodic timer — when the panel drops its Rc, upgrade() fails
        // and the timer naturally tears itself down.
        let me_weak: Weak<Self> = Rc::downgrade(self);
        let source = glib::timeout_add_local(interval, move || {
            let Some(me) = me_weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            // Skip tick if a run is still alive (no queueing).
            if me.is_running(id) {
                return glib::ControlFlow::Continue;
            }
            // Skip tick if cell no longer visible (defense in depth).
            if !me
                .cells
                .borrow()
                .get(&id)
                .map(|c| c.visible)
                .unwrap_or(false)
            {
                return glib::ControlFlow::Continue;
            }
            me.run_cell(id);
            glib::ControlFlow::Continue
        });
        if let Some(c) = self.cells.borrow_mut().get_mut(&id) {
            c.watch_source = Some(source);
        }
    }

    fn stop_watch(&self, id: CellId) {
        if let Some(c) = self.cells.borrow_mut().get_mut(&id) {
            if let Some(src) = c.watch_source.take() {
                src.remove();
            }
        }
    }

    fn ensure_helpers(&self) -> Option<PathBuf> {
        if let Some(p) = self.helpers_dir.borrow().clone() {
            return Some(p);
        }
        let cache = dirs_cache_root().join("notebook_helpers");
        let pkg_dir = cache.join("pax");
        if std::fs::create_dir_all(&pkg_dir).is_err() {
            return None;
        }
        let init = pkg_dir.join("__init__.py");
        let body = include_str!("helpers.py");
        if std::fs::write(&init, body).is_err() {
            return None;
        }
        *self.helpers_dir.borrow_mut() = Some(cache.clone());
        Some(cache)
    }
}

fn dirs_cache_root() -> PathBuf {
    if let Ok(home) = std::env::var("XDG_CACHE_HOME") {
        return PathBuf::from(home).join("pax");
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home).join(".cache").join("pax");
    }
    std::env::temp_dir().join("pax-cache")
}

impl Drop for NotebookEngine {
    fn drop(&mut self) {
        // Stop all watch timers first so they don't restart pending runs.
        let ids: Vec<CellId> = self.cells.borrow().keys().copied().collect();
        for id in &ids {
            self.stop_watch(*id);
        }
        // Drop each handle: RunHandle::drop sends SIGTERM + SIGKILL fallback.
        for state in self.cells.borrow_mut().values_mut() {
            state.handle = None;
        }
        // Best-effort cleanup of the per-engine output dir.
        let _ = std::fs::remove_dir_all(&self.output_dir);
    }
}
