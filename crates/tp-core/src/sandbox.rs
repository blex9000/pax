//! Flatpak sandbox detection and host-command spawning.
//!
//! Pax is a terminal workspace manager: the terminals it opens must run
//! the user's *host* tools (docker, ssh, their PATH), not the minimal
//! runtime shipped inside the Flatpak sandbox. When packaged as a
//! Flatpak we therefore detect the sandbox and wrap spawned commands
//! with `flatpak-spawn --host`, which runs them on the host through the
//! session portal.
//!
//! The pure argv-construction logic lives here (in `pax-core`, which
//! builds without GTK) so it can be unit-tested; the GUI wires it into
//! its spawn sites.

use std::path::Path;

/// Canonical marker present in every Flatpak sandbox.
const FLATPAK_INFO: &str = "/.flatpak-info";

/// True when the process is running inside a Flatpak sandbox.
pub fn in_flatpak_sandbox() -> bool {
    sandbox_marker_exists(Path::new(FLATPAK_INFO))
}

fn sandbox_marker_exists(marker: &Path) -> bool {
    marker.exists()
}

/// Build the argv to run `program` (with `args`) on the host when
/// `in_sandbox` is true, forwarding only the `env` entries (each a
/// `KEY=VALUE` string) via `--env=` and starting the host process in
/// `cwd` (via `--directory=`) when given. Outside a sandbox the command
/// is returned unchanged (the caller sets the working directory itself).
///
/// Only the explicitly-listed `env` is forwarded: the sandbox's own
/// environment (PATH pointing at `/app`, runtime `LD_LIBRARY_PATH`, …)
/// must NOT leak onto the host, so the host process inherits the host's
/// real environment plus these overrides.
pub fn host_spawn_argv(
    in_sandbox: bool,
    program: &str,
    args: &[&str],
    env: &[String],
    cwd: Option<&str>,
) -> Vec<String> {
    if !in_sandbox {
        let mut argv = Vec::with_capacity(1 + args.len());
        argv.push(program.to_string());
        argv.extend(args.iter().map(|a| a.to_string()));
        return argv;
    }

    let mut argv = Vec::with_capacity(4 + env.len() + 1 + args.len());
    argv.push("flatpak-spawn".to_string());
    argv.push("--host".to_string());
    // Tie the host process to the sandbox lifetime: if the sandbox goes
    // away, flatpak-spawn terminates the host child instead of orphaning it.
    argv.push("--watch-bus".to_string());
    if let Some(dir) = cwd {
        argv.push(format!("--directory={}", dir));
    }
    for entry in env {
        argv.push(format!("--env={}", entry));
    }
    argv.push(program.to_string());
    argv.extend(args.iter().map(|a| a.to_string()));
    argv
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marker_absent_means_not_sandboxed() {
        assert!(!sandbox_marker_exists(Path::new(
            "/definitely/not/a/real/flatpak-info"
        )));
    }

    #[test]
    fn marker_present_means_sandboxed() {
        // The crate manifest dir is an absolute path that always exists,
        // independent of the test harness's working directory.
        let existing = Path::new(env!("CARGO_MANIFEST_DIR"));
        assert!(sandbox_marker_exists(existing));
    }

    #[test]
    fn outside_sandbox_returns_command_unchanged() {
        let argv = host_spawn_argv(false, "bash", &["-l"], &[], None);
        assert_eq!(argv, vec!["bash".to_string(), "-l".to_string()]);
    }

    #[test]
    fn outside_sandbox_ignores_cwd() {
        let argv = host_spawn_argv(false, "bash", &[], &[], Some("/home/me/proj"));
        assert_eq!(argv, vec!["bash".to_string()]);
    }

    #[test]
    fn inside_sandbox_wraps_with_flatpak_spawn_host() {
        let argv = host_spawn_argv(true, "bash", &[], &[], None);
        assert_eq!(
            argv,
            vec![
                "flatpak-spawn".to_string(),
                "--host".to_string(),
                "--watch-bus".to_string(),
                "bash".to_string(),
            ]
        );
    }

    #[test]
    fn inside_sandbox_forwards_env_before_program() {
        let env = vec!["TERM=xterm-256color".to_string()];
        let argv = host_spawn_argv(true, "bash", &[], &env, None);
        assert_eq!(
            argv,
            vec![
                "flatpak-spawn".to_string(),
                "--host".to_string(),
                "--watch-bus".to_string(),
                "--env=TERM=xterm-256color".to_string(),
                "bash".to_string(),
            ]
        );
    }

    #[test]
    fn inside_sandbox_preserves_args_after_program() {
        let argv = host_spawn_argv(true, "ssh", &["host", "-p", "22"], &[], None);
        assert_eq!(
            argv,
            vec![
                "flatpak-spawn".to_string(),
                "--host".to_string(),
                "--watch-bus".to_string(),
                "ssh".to_string(),
                "host".to_string(),
                "-p".to_string(),
                "22".to_string(),
            ]
        );
    }

    #[test]
    fn inside_sandbox_sets_directory_after_watch_bus() {
        let argv = host_spawn_argv(true, "bash", &[], &[], Some("/home/me/proj"));
        assert_eq!(
            argv,
            vec![
                "flatpak-spawn".to_string(),
                "--host".to_string(),
                "--watch-bus".to_string(),
                "--directory=/home/me/proj".to_string(),
                "bash".to_string(),
            ]
        );
    }

    #[test]
    fn inside_sandbox_directory_precedes_env() {
        let env = vec!["TERM=xterm-256color".to_string()];
        let argv = host_spawn_argv(true, "bash", &[], &env, Some("/tmp/w"));
        // --directory before --env, both before the program.
        let dir_idx = argv.iter().position(|a| a == "--directory=/tmp/w").unwrap();
        let env_idx = argv
            .iter()
            .position(|a| a == "--env=TERM=xterm-256color")
            .unwrap();
        let prog_idx = argv.iter().position(|a| a == "bash").unwrap();
        assert!(dir_idx < env_idx, "--directory must precede --env");
        assert!(env_idx < prog_idx, "--env must precede program");
    }
}
