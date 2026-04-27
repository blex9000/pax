//! Markdown notebook engine + GTK widgets.
//!
//! Wires fenced code blocks tagged with `run`/`once`/`watch=Ns` (parsed by
//! `pax_core::notebook_tag`) to a subprocess runner whose stdout/stderr is
//! rendered inline below the block. Lazy: a `NotebookEngine` is only
//! instantiated by `panels::markdown` after the renderer encounters the
//! first executable cell.

pub mod output;
pub mod runner;
// modules added in later tasks: engine, cell.

pub const DEFAULT_RUN_TIMEOUT_SECS: u64 = 30;
pub const MAX_NOTEBOOK_PROCESSES: usize = 8;
