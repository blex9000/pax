//! # Startup script dispatcher
//!
//! Shared logic for turning a batch of user-configured startup commands
//! into a single shell command line that the backend feeds to the PTY.
//! Extracted so VTE and PTY backends execute identical scripts in
//! identical ways (avoids the drift that had PTY echoing every script
//! line at the prompt while VTE silently `source`d a temp file).
//!
//! Modes (in priority order):
//! 1. **Empty / whitespace-only** → `None` (nothing to run).
//! 2. **Single plain line** → returned as-is.
//! 3. **`file:<interpreter>:<path>`** → `<interpreter> <resolved path>`.
//! 4. **Multi-line or `#!` shebang** → written to
//!    `/tmp/pax_startup_<pid>_<n>.sh` and emitted as
//!    `source <tmp> ; rm -f <tmp>` so the source lines never appear
//!    at the prompt.

use std::sync::atomic::{AtomicU64, Ordering};

/// Collapse a batch of startup commands into a single shell command line.
pub fn prepare_startup_command(commands: &[String], workspace_dir: Option<&str>) -> Option<String> {
    if commands.is_empty() {
        return None;
    }
    let full_text = commands.join("\n");
    if full_text.trim().is_empty() {
        return None;
    }

    if !full_text.contains('\n') && !full_text.starts_with("#!") && !full_text.starts_with("file:")
    {
        return Some(full_text);
    }

    if full_text.starts_with("file:") {
        let rest = full_text.trim_start_matches("file:");
        let (interp, path) = if let Some(idx) = rest[1..].find(':') {
            let idx = idx + 1;
            (&rest[..idx], &rest[idx + 1..])
        } else {
            ("/bin/bash", rest)
        };
        return Some(format!(
            "{} {}",
            interp,
            resolve_script_path(path, workspace_dir)
        ));
    }

    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let tmp = std::env::temp_dir().join(format!(
        "pax_startup_{}_{}.sh",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed),
    ));

    let interp = full_text
        .lines()
        .next()
        .filter(|l| l.starts_with("#!"))
        .map(|l| l.trim_start_matches("#!").trim().to_string())
        .unwrap_or_else(|| "/bin/bash".to_string());
    let script = if full_text.starts_with("#!") {
        full_text.clone()
    } else {
        format!("#!{}\n{}", interp, full_text)
    };

    std::fs::write(&tmp, &script).ok()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755));
    }
    Some(format!(
        "source {} ; rm -f {}",
        tmp.display(),
        tmp.display()
    ))
}

/// Resolve a user-provided script path against the workspace root when
/// the path is relative — same semantics as VTE's legacy resolver.
fn resolve_script_path(path: &str, workspace_dir: Option<&str>) -> String {
    let p = std::path::Path::new(path);
    if p.is_absolute() {
        return path.to_string();
    }
    if let Some(dir) = workspace_dir {
        return std::path::Path::new(dir)
            .join(path)
            .to_string_lossy()
            .to_string();
    }
    path.to_string()
}
