use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    println!("cargo:rerun-if-changed=build.rs");

    if let Some(git_dir) = git_dir(&manifest_dir) {
        emit_git_rerun_hints(&git_dir);
    }

    let git_commit = git_output(&manifest_dir, &["rev-parse", "--short=8", "HEAD"])
        .unwrap_or_else(|| "unknown".to_string());
    let git_date = git_output(&manifest_dir, &["show", "-s", "--format=%cs", "HEAD"])
        .unwrap_or_else(|| "unknown-date".to_string());

    println!("cargo:rustc-env=PAX_GIT_COMMIT={git_commit}");
    println!("cargo:rustc-env=PAX_GIT_DATE={git_date}");
}

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

fn git_dir(cwd: &Path) -> Option<PathBuf> {
    let output = git_output(cwd, &["rev-parse", "--git-dir"])?;
    let path = PathBuf::from(output);
    if path.is_absolute() {
        Some(path)
    } else {
        Some(cwd.join(path))
    }
}

fn emit_git_rerun_hints(git_dir: &Path) {
    let head = git_dir.join("HEAD");
    println!("cargo:rerun-if-changed={}", head.display());

    let packed_refs = git_dir.join("packed-refs");
    println!("cargo:rerun-if-changed={}", packed_refs.display());

    if let Ok(head_contents) = std::fs::read_to_string(&head) {
        if let Some(reference) = head_contents.strip_prefix("ref: ").map(str::trim) {
            let ref_path = git_dir.join(reference);
            println!("cargo:rerun-if-changed={}", ref_path.display());
        }
    }
}
