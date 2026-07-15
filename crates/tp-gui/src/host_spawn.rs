//! Route subprocess spawns onto the host when running inside a Flatpak
//! sandbox.
//!
//! Pax shells out to host tools all over the place — docker, ssh, code
//! formatters, transcription commands, terminal emulators. Inside a
//! Flatpak sandbox those tools live on the *host*, not in the runtime, so
//! the commands must be routed through `flatpak-spawn --host`. The pure
//! argv construction lives in `pax_core::sandbox`; this module adapts it
//! to `std::process::Command`.
//!
//! ## Usage contract
//!
//! Call [`hostify`] **after** the program, arguments and any explicit
//! `.env()` overrides are set, but **before** stdio (`.stdin`/`.stdout`/
//! `.stderr`), `.pre_exec`, and spawning. Rationale:
//!
//! * `Command` does not expose stdio / pre_exec closures for reading, so
//!   [`hostify`] cannot copy them — it only reads program, args, cwd and
//!   explicit envs. Anything set afterwards is applied to the returned
//!   (wrapped) command by the caller and therefore preserved.
//! * Only explicitly-set envs are forwarded to the host (via `--env=`);
//!   the sandbox's inherited environment (its `/app` PATH, runtime
//!   `LD_LIBRARY_PATH`, …) is deliberately NOT leaked onto the host.
//!
//! On native builds (not sandboxed) [`hostify`] returns the command
//! unchanged, so call sites behave exactly as before.

use std::process::Command;

use pax_core::sandbox::{host_spawn_argv, in_flatpak_sandbox};

/// Rewrite `cmd` to run on the host via `flatpak-spawn --host` when inside
/// a Flatpak sandbox; otherwise return it unchanged.
///
/// See the module docs for the ordering contract (call before stdio /
/// pre_exec are set).
pub fn hostify(cmd: Command) -> Command {
    if !in_flatpak_sandbox() {
        return cmd;
    }

    let program = cmd.get_program().to_string_lossy().into_owned();
    let args: Vec<String> = cmd
        .get_args()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    let cwd = cmd
        .get_current_dir()
        .map(|p| p.to_string_lossy().into_owned());
    // get_envs yields (key, Some(value)) for sets and (key, None) for
    // removals; forward only the sets.
    let env: Vec<String> = cmd
        .get_envs()
        .filter_map(|(k, v)| {
            v.map(|v| format!("{}={}", k.to_string_lossy(), v.to_string_lossy()))
        })
        .collect();

    let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let argv = host_spawn_argv(true, &program, &arg_refs, &env, cwd.as_deref());

    let mut wrapped = Command::new(&argv[0]);
    wrapped.args(&argv[1..]);
    wrapped
}
