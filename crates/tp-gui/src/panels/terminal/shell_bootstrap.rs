//! # Shell bootstrap payload
//!
//! Single source of truth for the shell snippets pax injects into every
//! terminal panel on startup. Both VTE and PTY backends consume this so
//! future refinements to shell integration (new OSC sequences, extra
//! trap hooks, …) do not drift between the two implementations.
//!
//! Supported shells: **bash**, **zsh**. Any other shell receives an empty
//! payload (silent no-op) — history capture, OSC 133 indicators, and OSC 7
//! footer are all inert for unknown shells.
//!
//! The payload sets up:
//! - `PAX_CMD_FILE` env var pointing to the per-panel sidechannel file.
//!   The preexec hook writes the command into this file; the GUI reads it
//!   when OSC 133;C arrives to populate `command_history`.
//! - `LS_COLORS` / `CLICOLOR` / `LSCOLORS` / `dircolors` / `alias ls`
//!   for consistent colored output across Linux + macOS `ls` flavours.
//! - Optional minimal PS1 / PROMPT override (VTE only — spawn happens after
//!   the rc file so the override sticks; PTY keeps the user's prompt).
//! - `__pax_prompt` / `__pax_prompt_zsh`: emitted via `PROMPT_COMMAND` /
//!   `precmd_functions` on every prompt. Sends OSC 0 (window title), optional
//!   OSC 7 (directory URI, VTE only), OSC 133;A (shell integration —
//!   "prompt starting"), and resets the preexec guard flag (bash).
//! - `__pax_preexec` / `__pax_preexec_zsh`: DEBUG trap / preexec hook.
//!   Writes the just-entered command to `$PAX_CMD_FILE`, then emits
//!   OSC 133;C ("command started").
//! - (bash) `PROMPT_COMMAND` is **appended** (not replaced) so any
//!   user-supplied hook from `.bashrc` survives.
//! - (bash) Wrapped in `set +o history` / `set -o history` so none of these
//!   lines leak into `.bash_history`.
//!
//! See `docs/shell-integration.md` for the full rationale.

use std::path::PathBuf;
use uuid::Uuid;

/// Path of the per-panel sidechannel file used to ferry the just-executed
/// command from the shell preexec hook to the GUI. Lives under
/// `XDG_RUNTIME_DIR` if set (Linux), otherwise `/tmp`. Filename is
/// `pax-cmd-<simple-uuid>.txt`.
///
/// On macOS `XDG_RUNTIME_DIR` is unset by default and the file lands in
/// `/tmp`. Use [`prepare_cmd_file`] to create it with mode `0600` so the
/// just-executed command is not world-readable.
pub fn cmd_file_path(panel_uuid: &Uuid) -> PathBuf {
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    dir.join(format!("pax-cmd-{}.txt", panel_uuid.simple()))
}

/// Eagerly create `cmd_file` with mode `0600` so that subsequent shell
/// `>` redirects truncate-without-chmod and the file stays user-only.
/// Best-effort: errors are swallowed.
pub fn prepare_cmd_file(cmd_file: &std::path::Path) {
    if cmd_file.as_os_str().is_empty() {
        return;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let _ = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .mode(0o600)
            .open(cmd_file);
    }
    #[cfg(not(unix))]
    {
        let _ = std::fs::File::create(cmd_file);
    }
}

/// Whether `cmd` is something pax injects itself (bootstrap helpers,
/// startup-command source statements, prompt callbacks) rather than
/// something the user typed. These should be filtered out before the
/// row lands in `command_history` so the popup shows only real user
/// commands.
fn is_internal_pax_command(cmd: &str) -> bool {
    if cmd.starts_with("__pax_") {
        return true;
    }
    // Pax materialises each entry of `startup_commands` as a generated
    // shell script under `/tmp/pax_startup_<pid>_<n>.sh` and runs them
    // via `source`. The `source …` statement itself is plumbing — the
    // user-meaningful commands inside the script are captured by their
    // own preexec cycle, so the wrapper is just noise.
    if cmd.starts_with("source /tmp/pax_startup_") {
        return true;
    }
    matches!(cmd, "clear" | "set -o history" | "set +o history")
}

/// Watch `cmd_file` and insert into `command_history` on every change.
///
/// This is the authoritative capture path for command history: it sees
/// every shell preexec write, regardless of whether the command was a
/// builtin (which leaves the foreground process group unchanged and is
/// therefore invisible to the `tcgetpgrp` poller) or whether the host's
/// VTE runtime exposes the `shell-preexec` signal (added in 0.80).
///
/// Returns the live `gio::FileMonitor`; the caller must hold onto it for
/// the lifetime of the panel — dropping the monitor stops notifications.
/// Returns `None` if `cmd_file` is empty (panel without a UUID, e.g. a
/// chooser slot still showing the type picker).
pub fn spawn_cmd_file_watcher(
    cmd_file: &std::path::Path,
    panel_uuid_str: Option<String>,
    workspace_name: Option<String>,
) -> Option<gtk4::gio::FileMonitor> {
    use gtk4::gio;
    use gtk4::prelude::*;

    if cmd_file.as_os_str().is_empty() {
        return None;
    }
    let file = gio::File::for_path(cmd_file);
    let monitor = file
        .monitor_file(gio::FileMonitorFlags::NONE, gio::Cancellable::NONE)
        .ok()?;

    // Deduplicate back-to-back identical reads: gio::FileMonitor may
    // dispatch multiple signals for a single shell write (one for the
    // truncate-on-open, one for the bytes, one for the close-write
    // hint). We only want to insert each distinct command once per
    // burst.
    let last_seen: std::rc::Rc<std::cell::RefCell<Option<String>>> =
        std::rc::Rc::new(std::cell::RefCell::new(None));

    let cmd_file_owned = cmd_file.to_path_buf();
    monitor.connect_changed(move |_monitor, _file, _other, event| {
        // Ignore deletes / unmounts / metadata-only changes.
        if !matches!(
            event,
            gio::FileMonitorEvent::Changed
                | gio::FileMonitorEvent::ChangesDoneHint
                | gio::FileMonitorEvent::Created
        ) {
            return;
        }
        let Ok(raw) = std::fs::read_to_string(&cmd_file_owned) else {
            return;
        };
        let cmd = raw.trim_end_matches(['\n', '\r']);
        if cmd.is_empty() || is_internal_pax_command(cmd) {
            return;
        }
        if last_seen
            .borrow()
            .as_deref()
            .is_some_and(|prev| prev == cmd)
        {
            return;
        }
        *last_seen.borrow_mut() = Some(cmd.to_string());
        if let Ok(db) = pax_db::Database::open(&pax_db::Database::default_path()) {
            let _ = db.insert_command(
                workspace_name.as_deref(),
                panel_uuid_str.as_deref(),
                cmd,
                None,
            );
        }
    });

    Some(monitor)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellKind {
    Bash,
    Zsh,
    /// Any unrecognised shell — we do not inject hooks. Command history,
    /// OSC 133 indicators, OSC 7 footer all stay inert (silent fallback).
    Other,
}

impl ShellKind {
    pub fn detect_from_path(shell_path: &str) -> Self {
        let basename = shell_path
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or(shell_path)
            .trim();
        match basename {
            "bash" => Self::Bash,
            "zsh" => Self::Zsh,
            _ => Self::Other,
        }
    }
}

pub struct BootstrapConfig<'a> {
    pub shell: ShellKind,
    /// Override PS1 (bash) / PROMPT (zsh) with pax's minimal green prompt.
    pub override_ps1: bool,
    /// Emit OSC 7 (directory URI) from the prompt callback.
    pub emit_osc7: bool,
    /// Path the preexec hook will write the command into. Borrowed so we
    /// do not allocate per call. Use `cmd_file_path(panel_uuid)`.
    pub cmd_file: &'a std::path::Path,
}

/// Returns the ordered list of shell lines to feed after the shell has
/// loaded the user's rc file. Both backends feed each line followed by
/// `\n`. For unknown shells returns an empty vec (silent no-op).
pub fn bootstrap_lines(cfg: &BootstrapConfig) -> Vec<String> {
    match cfg.shell {
        ShellKind::Bash => bash_lines(cfg),
        ShellKind::Zsh => zsh_lines(cfg),
        ShellKind::Other => Vec::new(),
    }
}

fn bash_lines(cfg: &BootstrapConfig) -> Vec<String> {
    let cmd_file = cfg.cmd_file.display().to_string();
    let mut lines: Vec<String> = Vec::with_capacity(16);
    lines.push("set +o history".to_string());
    lines.push(format!(
        "export PAX_CMD_FILE='{}'",
        shell_escape_single(&cmd_file)
    ));
    if cfg.override_ps1 {
        lines.push(r"export PS1='\[\033[32m\]$:\[\033[0m\] '".to_string());
    }
    lines.push(
        "export LS_COLORS='di=38;2;85;136;255:ln=36:so=35:pi=33:ex=32:bd=34;46:cd=34;43:su=30;41:sg=30;46:tw=30;42:ow=34;42'"
            .to_string(),
    );
    lines.push("export CLICOLOR=1".to_string());
    lines.push("export LSCOLORS='ExFxCxDxBxegedabagacad'".to_string());
    lines.push(
        r#"if command -v dircolors >/dev/null 2>&1; then eval "$(dircolors -b 2>/dev/null)"; fi"#
            .to_string(),
    );
    lines.push(
        r#"if command ls --color=auto . >/dev/null 2>&1; then alias ls='ls --color=auto'; else alias ls='ls -G'; fi"#
            .to_string(),
    );
    lines.push(bash_prompt_function(cfg.emit_osc7));
    lines.push(bash_preexec_function());
    lines.push(r#"PROMPT_COMMAND="${PROMPT_COMMAND:+$PROMPT_COMMAND; }__pax_prompt""#.to_string());
    lines.push("trap '__pax_preexec' DEBUG".to_string());
    // Pre-arm the guard so the first prompt's PROMPT_COMMAND chain
    // (which would otherwise show up as $BASH_COMMAND="__pax_prompt"
    // when the DEBUG trap fires before any user input) does not write
    // bookkeeping commands into $PAX_CMD_FILE. __pax_prompt clears the
    // flag at the end of every prompt cycle so user commands are still
    // captured normally.
    lines.push("__pax_preexec_fired=1".to_string());
    lines.push("set -o history".to_string());
    lines.push("clear".to_string());
    lines
}

fn zsh_lines(cfg: &BootstrapConfig) -> Vec<String> {
    let cmd_file = cfg.cmd_file.display().to_string();
    let mut lines: Vec<String> = Vec::with_capacity(14);
    lines.push(format!(
        "export PAX_CMD_FILE='{}'",
        shell_escape_single(&cmd_file)
    ));
    if cfg.override_ps1 {
        // zsh prompt syntax (single-quoted to keep %F/%f literal until
        // the prompt is evaluated each time).
        lines.push(r"export PROMPT='%F{green}$:%f '".to_string());
    }
    lines.push(
        "export LS_COLORS='di=38;2;85;136;255:ln=36:so=35:pi=33:ex=32:bd=34;46:cd=34;43:su=30;41:sg=30;46:tw=30;42:ow=34;42'"
            .to_string(),
    );
    lines.push("export CLICOLOR=1".to_string());
    lines.push("export LSCOLORS='ExFxCxDxBxegedabagacad'".to_string());
    lines.push(
        r#"if command ls --color=auto . >/dev/null 2>&1; then alias ls='ls --color=auto'; else alias ls='ls -G'; fi"#
            .to_string(),
    );
    lines.push(zsh_prompt_function(cfg.emit_osc7));
    lines.push(zsh_preexec_function());
    lines.push("typeset -ga precmd_functions preexec_functions".to_string());
    lines.push("precmd_functions+=(__pax_prompt_zsh)".to_string());
    lines.push("preexec_functions+=(__pax_preexec_zsh)".to_string());
    lines.push("clear".to_string());
    lines
}

fn shell_escape_single(s: &str) -> String {
    // POSIX single-quote escaping: ' → '\''
    s.replace('\'', r"'\''")
}

fn bash_prompt_function(emit_osc7: bool) -> String {
    let osc7 = if emit_osc7 {
        r#"printf '\033]7;file://%s%s\033\\' "$HOSTNAME" "$PWD"; "#
    } else {
        ""
    };
    format!(
        r##"__pax_prompt() {{ local d="${{PWD/#$HOME/~}}"; printf '\033]0;%s@%s: %s\007' "$USER" "$HOSTNAME" "$d"; {}printf '\033]133;A\007'; __pax_preexec_fired=; }}"##,
        osc7
    )
}

fn bash_preexec_function() -> String {
    // Write the just-recognised command to $PAX_CMD_FILE before signalling
    // 133;C, so the GUI side can read it when 133;C arrives.
    r##"__pax_preexec() { [[ -n "$__pax_preexec_fired" ]] && return; __pax_preexec_fired=1; if [[ -n "$PAX_CMD_FILE" ]]; then printf '%s' "$BASH_COMMAND" > "$PAX_CMD_FILE" 2>/dev/null; fi; printf '\033]133;C\007'; }"##.to_string()
}

fn zsh_prompt_function(emit_osc7: bool) -> String {
    let osc7 = if emit_osc7 {
        r#"print -n -- $'\e]7;file://'"$HOST$PWD"$'\e\\'; "#
    } else {
        ""
    };
    format!(
        r##"__pax_prompt_zsh() {{ local d="${{PWD/#$HOME/~}}"; print -n -- $'\e]0;'"$USER@$HOST: $d"$'\a'; {}print -n -- $'\e]133;A\a'; }}"##,
        osc7
    )
}

fn zsh_preexec_function() -> String {
    // zsh preexec receives the command as $1 (raw) / $2 (after history
    // expansion) — we use $1 to preserve what the user actually typed.
    r##"__pax_preexec_zsh() { if [[ -n "$PAX_CMD_FILE" ]]; then print -nr -- "$1" > "$PAX_CMD_FILE" 2>/dev/null; fi; print -n -- $'\e]133;C\a'; }"##.to_string()
}
