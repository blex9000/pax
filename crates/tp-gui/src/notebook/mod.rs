//! Markdown notebook engine + GTK widgets.
//!
//! Wires fenced code blocks tagged with `run`/`once`/`watch=Ns` (parsed by
//! `pax_core::notebook_tag`) to a subprocess runner whose stdout/stderr is
//! rendered inline below the block. Lazy: a `NotebookEngine` is only
//! instantiated by `panels::markdown` after the renderer encounters the
//! first executable cell.

pub mod cell;
pub mod engine;
pub mod output;
pub mod runner;

pub const DEFAULT_RUN_TIMEOUT_SECS: u64 = 30;
pub const MAX_NOTEBOOK_PROCESSES: usize = 8;
pub const IMAGE_MAX_HEIGHT_PX: i32 = 400;
