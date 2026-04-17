//! # Shell bootstrap payload
//!
//! Single source of truth for the bash snippets pax injects into every
//! terminal panel on startup. Both VTE and PTY backends consume this so
//! future refinements to shell integration (new OSC sequences, extra
//! trap hooks, …) do not drift between the two implementations.
//!
//! The payload sets up:
//! - `LS_COLORS` / `CLICOLOR` / `LSCOLORS` / `dircolors` / `alias ls`
//!   for consistent colored output across Linux + macOS `ls` flavours.
//! - Optional minimal PS1 override (VTE only — spawn happens after
//!   `.bashrc` so the override sticks; PTY keeps the user's PS1).
//! - `__pax_prompt`: emitted via `PROMPT_COMMAND` on every prompt.
//!   Sends OSC 0 (window title), optional OSC 7 (directory URI, VTE
//!   only), OSC 133;A (shell integration — "prompt starting"), and
//!   resets the preexec guard flag.
//! - `__pax_preexec`: DEBUG trap callback. Emits OSC 133;C on the
//!   first command of each prompt cycle (guarded by `__pax_preexec_fired`
//!   so bursts inside `PROMPT_COMMAND` itself stay silent).
//! - `PROMPT_COMMAND` is **appended** (not replaced) so any user-supplied
//!   hook from `.bashrc` survives.
//!
//! The block is wrapped in `set +o history` / `set -o history` so none
//! of these lines leak into the user's `.bash_history`.
//!
//! See `docs/shell-integration.md` for the full rationale.

/// Per-backend switches that pick the right payload variant.
pub struct BootstrapConfig {
    /// Override PS1 with pax's minimal green prompt. VTE: true — the
    /// spawn callback runs after user `.bashrc`, so the override sticks.
    /// PTY: false — we inherit the user's prompt unchanged.
    pub override_ps1: bool,
    /// Emit OSC 7 (directory URI) from `__pax_prompt`. VTE consumes this
    /// via `current-directory-uri-changed` to drive the footer; the PTY
    /// backend does not currently track OSC 7.
    pub emit_osc7: bool,
}

/// Returns the ordered list of bash lines to feed to the shell after
/// `.bashrc` has run. Both backends feed each line followed by `\n`.
pub fn bootstrap_lines(cfg: &BootstrapConfig) -> Vec<String> {
    let mut lines: Vec<String> = Vec::with_capacity(16);
    lines.push("set +o history".to_string());
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
    lines.push(prompt_function(cfg.emit_osc7));
    lines.push(preexec_function());
    lines.push(r#"PROMPT_COMMAND="${PROMPT_COMMAND:+$PROMPT_COMMAND; }__pax_prompt""#.to_string());
    lines.push("trap '__pax_preexec' DEBUG".to_string());
    lines.push("set -o history".to_string());
    // `clear` wipes the visible bootstrap output. Kept outside the
    // no-history block because a single `clear` in history is harmless
    // and having it inside would require reorganising the whole block.
    lines.push("clear".to_string());
    lines
}

fn prompt_function(emit_osc7: bool) -> String {
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

fn preexec_function() -> String {
    r##"__pax_preexec() { [[ -n "$__pax_preexec_fired" ]] && return; __pax_preexec_fired=1; printf '\033]133;C\007'; }"##.to_string()
}
