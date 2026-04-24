//! Helpers for opening workspaces — either inside the current window
//! (replaces the current workspace after the existing dirty-check flow
//! in `actions::show_recent_dialog`) or in a freshly-spawned Pax
//! process so the user keeps both sessions side by side.
//!
//! Spawning a new process is the simplest implementation of "open in
//! a new window": GTK's multi-window-per-Application is more elegant
//! but would require restructuring how `run_app` builds and owns the
//! main window. A separate process reuses the existing `pax launch`
//! CLI entry point for free and keeps per-workspace state (DB handles,
//! theme providers, alert schedulers) fully isolated.

use std::io;
use std::path::Path;
use std::process::Command;

/// Launch a new Pax process that opens the given workspace config file.
/// Returns once the child has been spawned — does not wait for it.
pub fn open_in_new_window(config_path: &Path) -> io::Result<()> {
    let exe = std::env::current_exe()?;
    Command::new(exe).arg("launch").arg(config_path).spawn()?;
    Ok(())
}
