use std::path::{Path, PathBuf};
use std::process::Command;

fn git_output(cwd: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8(output.stdout).ok()?;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn main() {
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    // No rerun-if-changed hints: we want build.rs to run on every cargo
    // build so the dirty-flag below reflects the *current* working tree,
    // not the state at the last commit. Cost is two git subprocesses
    // (~tens of ms) per build, which is negligible for a dev workflow
    // and invaluable when testing uncommitted fixes.

    let base_commit = git_output(&manifest_dir, &["rev-parse", "--short=8", "HEAD"])
        .unwrap_or_else(|| "unknown".to_string());
    let is_dirty = git_output(&manifest_dir, &["status", "--porcelain"])
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false);
    let git_commit = if is_dirty {
        format!("{base_commit}-dirty")
    } else {
        base_commit
    };
    let git_date = git_output(&manifest_dir, &["show", "-s", "--format=%cs", "HEAD"])
        .unwrap_or_else(|| "unknown-date".to_string());

    // Wall-clock time of the build itself — lets a developer testing
    // uncommitted fixes distinguish two "-dirty" builds on the same
    // commit. `date` is available on both Linux and macOS.
    let build_time = Command::new("date")
        .arg("+%Y-%m-%d %H:%M:%S")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown-time".to_string());

    println!("cargo:rustc-env=PAX_GIT_COMMIT={git_commit}");
    println!("cargo:rustc-env=PAX_GIT_DATE={git_date}");
    println!("cargo:rustc-env=PAX_BUILD_TIME={build_time}");
}
