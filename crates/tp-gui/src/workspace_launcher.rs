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
    let config_path = workspace_launch_path(config_path)?;

    tracing::debug!(
        executable = %exe.display(),
        workspace = %config_path.display(),
        "spawning workspace in a new Pax window"
    );

    Command::new(exe)
        .arg("launch")
        .arg(config_path)
        .env(crate::app::SECONDARY_INSTANCE_ENV, "1")
        .spawn()?;
    Ok(())
}

fn workspace_launch_path(path: &Path) -> io::Result<PathBuf> {
    if let Ok(canonical) = path.canonicalize() {
        return Ok(canonical);
    }

    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }

    Ok(absolutize_relative_path(path, &std::env::current_dir()?))
}

fn absolutize_relative_path(path: &Path, cwd: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    }
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

    #[test]
    fn workspace_launch_path_canonicalizes_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("nested");
        std::fs::create_dir(&nested).unwrap();
        let workspace = nested.join("workspace.json");
        std::fs::write(&workspace, "{}").unwrap();

        let non_canonical = nested.join("..").join("nested").join("workspace.json");

        assert_eq!(workspace_launch_path(&non_canonical).unwrap(), workspace);
    }

    #[test]
    fn workspace_launch_path_keeps_absolute_missing_file() {
        let path = Path::new("/tmp/pax-missing-workspace.json");

        assert_eq!(workspace_launch_path(path).unwrap(), path);
    }

    #[test]
    fn absolutize_relative_workspace_path_against_cwd() {
        assert_eq!(
            absolutize_relative_path(Path::new("workspaces/foo.json"), Path::new("/tmp/pax")),
            PathBuf::from("/tmp/pax/workspaces/foo.json")
        );
    }
}
