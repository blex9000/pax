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
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

/// Drain bound for `attach_rx` per idle tick. Caps the work done in a
/// single GTK main-loop iteration so a fast-emitting subprocess can't
/// freeze the UI; lines beyond this are picked up on the next tick.
const MAX_DRAIN_PER_TICK: usize = 32;

/// Process-wide count of currently-running notebook subprocesses, summed
/// across every `NotebookEngine` instance. Enforces `MAX_NOTEBOOK_PROCESSES`
/// at the right scope (per-Pax-process, not per-panel).
static ACTIVE_NOTEBOOK_PROCESSES: AtomicUsize = AtomicUsize::new(0);

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
    /// On the next `push_output`, drop the existing `output` first. Set
    /// when a new run starts (watch tick or manual click) so the user keeps
    /// seeing the previous run's content until the new one produces its
    /// first item — eliminates the empty-then-refill flicker on `watch`.
    replace_on_next_push: bool,
    /// Wall-clock time of the last `Finished` event (success or kill).
    /// Surfaced in the cell header; `None` until the cell has run once.
    last_finished_at: Option<chrono::DateTime<chrono::Local>>,
    /// True after the engine has auto-started this cell once (on the
    /// first `set_visible(true)`). Prevents re-running `run`/`once` cells
    /// every time the panel becomes visible again (e.g., tab switches).
    auto_started: bool,
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

    pub fn last_finished_at(&self, id: CellId) -> Option<chrono::DateTime<chrono::Local>> {
        self.cells.borrow().get(&id).and_then(|c| c.last_finished_at)
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

    /// Notify the engine the cell widget became (in)visible — gates
    /// watch and triggers the first auto-run of `run`/`once` cells.
    pub fn set_visible(self: &Rc<Self>, id: CellId, visible: bool) {
        let mut cells = self.cells.borrow_mut();
        let Some(state) = cells.get_mut(&id) else { return };
        state.visible = visible;
        let spec = state.spec.clone();
        let already_started = state.auto_started;
        let confirmed = state.confirmed;
        drop(cells);
        match spec.as_ref().map(|s| &s.mode) {
            Some(ExecMode::Watch { interval }) => {
                if visible {
                    self.start_watch(id, *interval);
                } else {
                    self.stop_watch(id);
                }
            }
            Some(ExecMode::Once) => {
                // Auto-run on first visibility unless `confirm` is set
                // and the user hasn't accepted it yet.
                if visible && !already_started {
                    let needs_confirm = spec.as_ref().map(|s| s.confirm).unwrap_or(false);
                    if !needs_confirm || confirmed {
                        if let Some(c) = self.cells.borrow_mut().get_mut(&id) {
                            c.auto_started = true;
                        }
                        self.run_cell(id);
                    }
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
        // Global (process-wide) concurrency cap. Reserve a slot via CAS-loop
        // so two engines racing the limit can't both succeed.
        loop {
            let cur = ACTIVE_NOTEBOOK_PROCESSES.load(Ordering::SeqCst);
            if cur >= MAX_NOTEBOOK_PROCESSES {
                self.push_output(
                    id,
                    OutputItem::Error(format!(
                        "notebook process limit reached ({})",
                        MAX_NOTEBOOK_PROCESSES
                    )),
                );
                return;
            }
            if ACTIVE_NOTEBOOK_PROCESSES
                .compare_exchange(cur, cur + 1, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
            {
                break;
            }
        }
        let (spec, code) = {
            let cells = self.cells.borrow();
            let Some(state) = cells.get(&id) else {
                ACTIVE_NOTEBOOK_PROCESSES.fetch_sub(1, Ordering::SeqCst);
                return;
            };
            let Some(spec) = state.spec.clone() else {
                ACTIVE_NOTEBOOK_PROCESSES.fetch_sub(1, Ordering::SeqCst);
                return;
            };
            (spec, state.code.clone())
        };
        let helpers = self.ensure_helpers();
        match runner::spawn(&spec, &code, helpers.as_deref(), Some(&self.output_dir)) {
            Ok((handle, rx)) => {
                // Install handle and arm `replace_on_next_push` instead of
                // clearing now: the previous run's output stays visible
                // until the new one starts streaming, killing the flicker.
                if let Some(c) = self.cells.borrow_mut().get_mut(&id) {
                    c.handle = Some(handle);
                    c.replace_on_next_push = true;
                }
                self.notify(id); // status indicator → running, but content unchanged
                self.attach_rx(id, rx);
            }
            Err(reason) => {
                ACTIVE_NOTEBOOK_PROCESSES.fetch_sub(1, Ordering::SeqCst);
                // Errors should always replace prior output immediately
                // (so a broken cell doesn't keep showing yesterday's data).
                if let Some(c) = self.cells.borrow_mut().get_mut(&id) {
                    c.output.clear();
                    c.replace_on_next_push = false;
                }
                self.push_output(id, OutputItem::Error(format!("blocked: {}", reason)));
            }
        }
    }

    pub fn stop_cell(&self, id: CellId) {
        if let Some(c) = self.cells.borrow_mut().get_mut(&id) {
            if c.handle.take().is_some() {
                // Dropping the handle triggers SIGTERM/SIGKILL via RunHandle::drop.
                ACTIVE_NOTEBOOK_PROCESSES.fetch_sub(1, Ordering::SeqCst);
            }
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
            if c.handle.take().is_some() {
                ACTIVE_NOTEBOOK_PROCESSES.fetch_sub(1, Ordering::SeqCst);
            }
            c.last_finished_at = Some(chrono::Local::now());
        }
        self.notify(id);
    }

    fn push_output(&self, id: CellId, item: OutputItem) {
        let subs = {
            let mut cells = self.cells.borrow_mut();
            let Some(state) = cells.get_mut(&id) else { return };
            if state.replace_on_next_push {
                state.output.clear();
                state.replace_on_next_push = false;
            }
            state.output.push(item);
            state.subscribers.clone()
        };
        for s in subs {
            s();
        }
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
            // Skip auto-runs for `confirm` cells until the user has clicked
            // ▶ at least once — the confirm flag is the spec's safety net
            // for downloaded notebooks and would be a no-op otherwise.
            let needs_confirm = me
                .cells
                .borrow()
                .get(&id)
                .and_then(|c| c.spec.as_ref().map(|s| s.confirm))
                .unwrap_or(false);
            if needs_confirm && !me.is_confirmed(id) {
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
            if state.handle.take().is_some() {
                ACTIVE_NOTEBOOK_PROCESSES.fetch_sub(1, Ordering::SeqCst);
            }
        }
        // Best-effort cleanup of the per-engine output dir.
        let _ = std::fs::remove_dir_all(&self.output_dir);
    }
}
