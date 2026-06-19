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

use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Launch a new Pax process that opens the given workspace config file.
/// Returns once the child has been spawned — does not wait for it.
///
/// Sets `PAX_SECONDARY_INSTANCE=1` so the child uses a per-PID
/// `application_id` (see `app::run_app`). Without that, both windows
/// end up sharing the same xdg-shell app_id and the compositor groups
/// them into a single taskbar entry — minimize-then-restore could only
/// surface one of them at a time.
pub fn open_in_new_window(config_path: &Path) -> io::Result<()> {
    let exe = launcher_executable(std::env::var_os("APPIMAGE"))?;
    Command::new(exe)
        .arg("launch")
        .arg(config_path)
        .env(crate::app::SECONDARY_INSTANCE_ENV, "1")
        .spawn()?;
    Ok(())
}

fn launcher_executable(appimage: Option<OsString>) -> io::Result<PathBuf> {
    if let Some(path) = appimage.filter(|path| !path.is_empty()) {
        return Ok(PathBuf::from(path));
    }
    std::env::current_exe()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launcher_executable_prefers_appimage_path() {
        let appimage = Some(OsString::from("/opt/Pax-x86_64.AppImage"));

        assert_eq!(
            launcher_executable(appimage).unwrap(),
            PathBuf::from("/opt/Pax-x86_64.AppImage")
        );
    }

    #[test]
    fn launcher_executable_ignores_empty_appimage_path() {
        assert_eq!(
            launcher_executable(Some(OsString::new())).unwrap(),
            std::env::current_exe().unwrap()
        );
    }
}
