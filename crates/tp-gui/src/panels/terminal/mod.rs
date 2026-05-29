//! # Terminal Panel
//!
//! Cross-platform terminal emulator panel for Pax.
//!
//! This module provides two backend implementations selected at compile time:
//!
//! - **VTE backend** (`vte_backend.rs`): Full-featured terminal using VTE4
//!   (Virtual Terminal Emulator). Linux-only. Provides 256-color support,
//!   hyperlinks, 10k line scrollback, OSC 7 directory tracking, right-click
//!   copy/paste, and theme color integration.
//!
//! - **PTY backend** (`pty_backend.rs`): Cross-platform fallback using
//!   `portable-pty` + `alacritty_terminal` + a GTK DrawingArea renderer.
//!   Works on macOS and any platform where VTE4 is unavailable. Provides ANSI
//!   color rendering, scrollback, selection, copy/paste, and PTY resize.
//!
//! Both backends expose the same `TerminalInner` struct with identical public
//! API, so `TerminalPanel` works transparently regardless of platform.
//!
//! ## Feature flags
//!
//! - `vte` (default on Linux): Enables the VTE backend.
//!   Build without: `cargo build --no-default-features --features sourceview`
//!
//! ## Architecture
//!
//! ```text
//! terminal/
//! ├── mod.rs           ← This file: public API (TerminalPanel)
//! ├── vte_backend.rs   ← Linux VTE4 backend (#[cfg(feature = "vte")])
//! └── pty_backend.rs   ← Cross-platform PTY fallback (#[cfg(not(feature = "vte"))])
//! ```

mod export;
mod footer;
mod input;
mod script_runner;
mod scroll_to_bottom;
mod shell_bootstrap;

pub(crate) use footer::format_cwd_footer;

#[cfg(feature = "vte")]
#[path = "vte_backend.rs"]
mod backend;

#[cfg(not(feature = "vte"))]
#[path = "pty_backend.rs"]
mod backend;

use super::PanelBackend;
use crate::panels::{
    PanelCwdCallback, PanelInputCallback, PanelSshStateCallback, PanelStatusCallback,
    PanelTitleCallback, SshConnectionState,
};
use backend::TerminalInner;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

// ── Shared terminal font configuration ──────────────────────────────────────

/// Default terminal font family list. Pango's
/// `FontDescription::set_family` accepts a comma-separated list and resolves
/// families left-to-right, just like CSS `font-family`. The list below is
/// kept identical to the `.editor-code-view` CSS rule so the terminal and
/// the editor always agree on which concrete font is used — in particular
/// on macOS, where "JetBrains Mono" may not resolve through Pango's font
/// map even when CoreText has it, and the editor's CSS and the terminal's
/// standalone Pango lookup would otherwise fall back to different system
/// monospaces (SF Mono vs Menlo).
const DEFAULT_TERMINAL_FONT: &str =
    "JetBrains Mono, SF Mono, Cascadia Code, IBM Plex Mono, Fira Code, monospace";

/// Default terminal font size in pixels. Meant to match `font-size: 11px`
/// on `.editor-code-view` so the terminal and the editor render at the same
/// physical size regardless of the platform's default DPI (macOS defaults
/// to 72 DPI while Linux defaults to 96 DPI, which would make a points-based
/// Pango spec like `"JetBrains Mono 8.25"` render visibly smaller on macOS
/// than the CSS-based editor font).
///
/// An empirical note: Pango's `set_absolute_size` is documented as "device
/// units (pixels)", but on macOS Retina the renderer applies a subtle extra
/// scale factor that made 11 absolute units look slightly bigger than the
/// CSS 11px editor font. Nudging down to 10 closes that gap without
/// noticeably affecting the Linux path (11 vs 10 px is within a pixel of
/// rendering slack at this size).
const DEFAULT_TERMINAL_FONT_PX: f64 = 10.0;

const SSH_CONNECTED_MARKER_HOST: &str = "pax-ssh-connected";
const SSH_DISCONNECTED_MARKER_HOST: &str = "pax-ssh-disconnected";
const TERMINAL_LS_COLORS: &str =
    "di=38;2;85;136;255:ln=36:so=35:pi=33:ex=32:bd=34;46:cd=34;43:su=30;41:sg=30;46:tw=30;42:ow=34;42";

pub(crate) fn ssh_remote_bootstrap_command(cwd: Option<&str>) -> String {
    let mut command = r#"if [ -n "${SSH_CONNECTION:-}" ]; then "#.to_string();
    if let Some(cwd) = cwd.map(str::trim).filter(|cwd| !cwd.is_empty()) {
        command.push_str("cd ");
        command.push_str(&shell_quote(cwd));
        command.push_str(" || true; ");
    }
    command.push_str(r#"export PS1='\[\033[32m\]$:\[\033[0m\] '; "#);
    command.push_str(
        r#"export PROMPT_COMMAND='printf "\033]7;file://%s@%s%s\033\\" "$USER" "$HOSTNAME" "$PWD"'; "#,
    );
    command.push_str("export LS_COLORS=");
    command.push_str(&shell_quote(TERMINAL_LS_COLORS));
    command.push_str("; ");
    command.push_str(r#"printf '\033]7;file://pax-ssh-connected/%s\007' "$PWD"; "#);
    command.push_str("clear; ");
    command.push_str(r#"else printf '\033]7;file://pax-ssh-disconnected/%s\007' "$PWD"; fi"#);
    command
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

/// Padding (in pixels) between the terminal content and the widget edges.
///
/// Applied consistently to both backends:
/// - VTE backend: via CSS `vte-terminal { padding: ... }` in `BASE_CSS`.
/// - PTY backend: via a drawing offset and coordinate adjustment (this const).
///
/// Keep this value in sync with the `vte-terminal` padding rule in
/// `crate::theme::BASE_CSS` — that rule must hard-code the same number of
/// pixels, since CSS is a plain `&str` constant and cannot reference this.
#[cfg(not(feature = "vte"))]
pub(crate) const TERMINAL_PADDING_PX: i32 = 6;

/// Pango `FontDescription` for the terminal.
///
/// When `PAX_TERMINAL_FONT` is set, its value is passed verbatim to
/// `FontDescription::from_string` so user overrides keep their exact spec.
/// The built-in default sets the family list via `set_family` (which accepts
/// a comma-separated list and resolves it left-to-right, same as CSS
/// font-family) and uses an absolute pixel size so the terminal matches the
/// editor at any DPI — see `DEFAULT_TERMINAL_FONT_PX`.
pub(crate) fn terminal_font_description() -> gtk4::pango::FontDescription {
    use gtk4::pango;
    if let Ok(user_spec) = std::env::var("PAX_TERMINAL_FONT") {
        let trimmed = user_spec.trim();
        if !trimmed.is_empty() {
            return pango::FontDescription::from_string(trimmed);
        }
    }
    let mut desc = pango::FontDescription::new();
    desc.set_family(DEFAULT_TERMINAL_FONT);
    desc.set_absolute_size(DEFAULT_TERMINAL_FONT_PX * pango::SCALE as f64);
    desc
}

#[derive(Debug, Clone)]
pub struct SshControl {
    label: String,
    connect_commands: Vec<String>,
    connect_raw_commands: Vec<String>,
    state: Rc<Cell<SshConnectionState>>,
    connect_started: Rc<Cell<bool>>,
    connect_commands_sent: Rc<Cell<bool>>,
}

/// Terminal panel — uses VTE4 on Linux, PTY+cell renderer fallback on macOS.
///
/// Created by the panel registry when a `PanelType::Terminal` config is loaded.
/// The backend is chosen at compile time via the `vte` feature flag.
pub struct TerminalPanel {
    inner: TerminalInner,
    /// Saved SSH target and runtime state for the header connect/disconnect button.
    ssh_control: Option<SshControl>,
    ssh_state_cb: Rc<RefCell<Option<PanelSshStateCallback>>>,
}

impl std::fmt::Debug for TerminalPanel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TerminalPanel")
            .field("inner", &self.inner)
            .field("ssh_control", &self.ssh_control)
            .finish_non_exhaustive()
    }
}

impl TerminalPanel {
    pub fn new(
        shell: &str,
        cwd: Option<&str>,
        env: &[(String, String)],
        workspace_dir: Option<&str>,
        panel_uuid: Option<uuid::Uuid>,
    ) -> Self {
        Self {
            inner: TerminalInner::new(shell, cwd, env, workspace_dir, panel_uuid),
            ssh_control: None,
            ssh_state_cb: Rc::new(RefCell::new(None)),
        }
    }

    /// Set the SSH target shown in the panel header and controlled by its
    /// connect/disconnect button.
    pub fn set_ssh_control(
        &mut self,
        label: String,
        connect_commands: Vec<String>,
        connect_raw_commands: Vec<String>,
        connect_requested: bool,
    ) {
        self.ssh_control = Some(SshControl {
            label,
            connect_commands,
            connect_raw_commands,
            state: Rc::new(Cell::new(if connect_requested {
                SshConnectionState::Connecting
            } else {
                SshConnectionState::Disconnected
            })),
            connect_started: Rc::new(Cell::new(false)),
            connect_commands_sent: Rc::new(Cell::new(false)),
        });
    }

    /// Queue startup commands to execute once the shell is ready.
    pub fn send_commands(&self, commands: &[String]) {
        self.inner.send_commands(commands);
    }

    /// Queue raw text to be sent to the terminal after spawn.
    /// No processing — text is sent as-is.
    pub fn queue_raw(&self, text: &str) {
        self.inner.queue_raw(text);
    }
}

fn set_ssh_state(
    control: &SshControl,
    state: SshConnectionState,
    state_cb: &Rc<RefCell<Option<PanelSshStateCallback>>>,
) {
    if control.state.get() == state {
        return;
    }
    control.state.set(state);
    if state == SshConnectionState::Disconnected {
        control.connect_started.set(false);
        control.connect_commands_sent.set(false);
    }
    if let Ok(borrowed) = state_cb.try_borrow() {
        if let Some(ref cb) = *borrowed {
            cb(state);
        }
    }
}

fn update_ssh_state_from_status(
    control: &SshControl,
    busy: bool,
    state_cb: &Rc<RefCell<Option<PanelSshStateCallback>>>,
) {
    match control.state.get() {
        SshConnectionState::Connecting if busy => {
            control.connect_started.set(true);
        }
        SshConnectionState::Connecting if control.connect_started.get() => {
            set_ssh_state(control, SshConnectionState::Disconnected, state_cb);
        }
        SshConnectionState::Connected if !busy => {
            set_ssh_state(control, SshConnectionState::Disconnected, state_cb);
        }
        _ => {}
    }
}

fn is_ssh_connected_marker(uri: &str) -> bool {
    is_ssh_marker(uri, SSH_CONNECTED_MARKER_HOST)
}

fn is_ssh_disconnected_marker(uri: &str) -> bool {
    is_ssh_marker(uri, SSH_DISCONNECTED_MARKER_HOST)
}

fn is_ssh_marker(uri: &str, host: &str) -> bool {
    let Some(rest) = uri.strip_prefix("file://") else {
        return false;
    };
    rest.split('/').next() == Some(host)
}

impl PanelBackend for TerminalPanel {
    fn panel_type(&self) -> &str {
        "terminal"
    }

    fn widget(&self) -> &gtk4::Widget {
        self.inner.widget()
    }

    fn on_focus(&self) {
        self.inner.grab_focus();
    }

    fn write_input(&self, data: &[u8]) -> bool {
        self.inner.write_input(data)
    }

    fn set_input_callback(&self, callback: Option<PanelInputCallback>) {
        self.inner.set_input_callback(callback);
    }

    fn set_title_callback(&self, callback: Option<PanelTitleCallback>) {
        self.inner.set_title_callback(callback);
    }

    fn set_status_callback(&self, callback: Option<PanelStatusCallback>) {
        let ssh_control = self.ssh_control.clone();
        let state_cb = self.ssh_state_cb.clone();
        let wrapped = callback.map(|cb| {
            Rc::new(move |busy: bool| {
                if let Some(ref control) = ssh_control {
                    update_ssh_state_from_status(control, busy, &state_cb);
                }
                cb(busy);
            }) as PanelStatusCallback
        });
        self.inner.set_status_callback(wrapped);
    }

    fn set_cwd_callback(&self, callback: Option<PanelCwdCallback>) {
        let ssh_control = self.ssh_control.clone();
        let state_cb = self.ssh_state_cb.clone();
        let wrapped = callback.map(|cb| {
            Rc::new(move |uri: &str| {
                if is_ssh_connected_marker(uri) {
                    if let Some(ref control) = ssh_control {
                        set_ssh_state(control, SshConnectionState::Connected, &state_cb);
                    }
                    return;
                }
                if is_ssh_disconnected_marker(uri) {
                    if let Some(ref control) = ssh_control {
                        set_ssh_state(control, SshConnectionState::Disconnected, &state_cb);
                    }
                    return;
                }
                cb(uri);
            }) as PanelCwdCallback
        });
        self.inner.set_cwd_callback(wrapped);
    }

    fn panel_uuid(&self) -> Option<uuid::Uuid> {
        self.inner.panel_uuid
    }

    fn ssh_label(&self) -> Option<String> {
        self.ssh_control
            .as_ref()
            .map(|control| control.label.clone())
    }

    fn ssh_is_connected(&self) -> Option<bool> {
        self.ssh_control
            .as_ref()
            .map(|control| control.state.get() == SshConnectionState::Connected)
    }

    fn ssh_connection_state(&self) -> Option<SshConnectionState> {
        self.ssh_control.as_ref().map(|control| control.state.get())
    }

    fn ssh_connect(&self) -> bool {
        let Some(control) = self.ssh_control.as_ref() else {
            return false;
        };
        match control.state.get() {
            SshConnectionState::Connected => return true,
            SshConnectionState::Connecting if control.connect_commands_sent.get() => return true,
            SshConnectionState::Connecting => {}
            SshConnectionState::Disconnected => {
                set_ssh_state(control, SshConnectionState::Connecting, &self.ssh_state_cb);
                control.connect_started.set(false);
            }
        }
        control.connect_commands_sent.set(true);
        for command in &control.connect_commands {
            self.inner.send_commands(std::slice::from_ref(command));
        }
        for command in &control.connect_raw_commands {
            self.inner.queue_raw(command);
        }
        true
    }

    fn ssh_disconnect(&self) -> bool {
        let Some(control) = self.ssh_control.as_ref() else {
            return false;
        };
        if control.state.get() != SshConnectionState::Connected {
            set_ssh_state(
                control,
                SshConnectionState::Disconnected,
                &self.ssh_state_cb,
            );
            return true;
        }
        self.inner.write_input(b"exit\n");
        set_ssh_state(
            control,
            SshConnectionState::Disconnected,
            &self.ssh_state_cb,
        );
        true
    }

    fn set_ssh_state_callback(&self, callback: Option<PanelSshStateCallback>) {
        *self.ssh_state_cb.borrow_mut() = callback;
    }

    fn accepts_input(&self) -> bool {
        true
    }

    fn supports_sync(&self) -> bool {
        true
    }

    fn on_permanent_close(&self) {
        if let Some(uuid) = self.inner.panel_uuid {
            if let Ok(db) = pax_db::Database::open(&pax_db::Database::default_path()) {
                let key = uuid.simple().to_string();
                let _ = db.delete_command_history_for_panel(&key);
                let _ = db.delete_pinned_for_panel(&key);
            }
        }
    }

    fn shutdown(&self) {
        self.inner.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ssh_control(state: SshConnectionState) -> SshControl {
        SshControl {
            label: "dev@example.test".to_string(),
            connect_commands: Vec::new(),
            connect_raw_commands: Vec::new(),
            state: Rc::new(Cell::new(state)),
            connect_started: Rc::new(Cell::new(false)),
            connect_commands_sent: Rc::new(Cell::new(false)),
        }
    }

    fn no_state_callback() -> Rc<RefCell<Option<PanelSshStateCallback>>> {
        Rc::new(RefCell::new(None))
    }

    #[test]
    fn ssh_marker_uri_is_detected() {
        assert!(is_ssh_connected_marker(
            "file://pax-ssh-connected/home/user"
        ));
        assert!(is_ssh_disconnected_marker(
            "file://pax-ssh-disconnected/home/user"
        ));
        assert!(!is_ssh_connected_marker("file://remote-host/home/user"));
        assert!(!is_ssh_disconnected_marker("file://remote-host/home/user"));
        assert!(!is_ssh_connected_marker(""));
    }

    #[test]
    fn ssh_remote_bootstrap_clears_only_after_remote_success() {
        let command = ssh_remote_bootstrap_command(Some("/srv/app's"));

        assert!(command.starts_with(r#"if [ -n "${SSH_CONNECTION:-}" ]; then "#));
        assert!(command.contains("cd '/srv/app'\\''s' || true;"));
        assert!(command.contains("export PS1="));
        assert!(command.contains("file://%s@%s%s"));
        assert!(command.contains("pax-ssh-connected"));
        assert!(command.contains("clear; else"));
        assert!(command.contains("pax-ssh-disconnected"));

        let disconnected_branch = command.split(" else ").nth(1).unwrap_or_default();
        assert!(!disconnected_branch.contains("clear"));
        assert!(!disconnected_branch.contains("export PS1"));
    }

    #[test]
    fn queued_ssh_connect_ignores_prompt_before_command_start() {
        let control = ssh_control(SshConnectionState::Connecting);
        update_ssh_state_from_status(&control, false, &no_state_callback());

        assert_eq!(control.state.get(), SshConnectionState::Connecting);
        assert!(!control.connect_started.get());
    }

    #[test]
    fn failed_ssh_connect_returns_to_disconnected_after_prompt() {
        let control = ssh_control(SshConnectionState::Connecting);
        let cb = no_state_callback();

        update_ssh_state_from_status(&control, true, &cb);
        assert_eq!(control.state.get(), SshConnectionState::Connecting);
        assert!(control.connect_started.get());

        update_ssh_state_from_status(&control, false, &cb);
        assert_eq!(control.state.get(), SshConnectionState::Disconnected);
        assert!(!control.connect_started.get());
    }
}
