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

mod footer;
mod input;
mod script_runner;
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
    PanelCwdCallback, PanelInputCallback, PanelStatusCallback, PanelTitleCallback,
};
use backend::TerminalInner;

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

/// Terminal panel — uses VTE4 on Linux, PTY+cell renderer fallback on macOS.
///
/// Created by the panel registry when a `PanelType::Terminal` config is loaded.
/// The backend is chosen at compile time via the `vte` feature flag.
#[derive(Debug)]
pub struct TerminalPanel {
    inner: TerminalInner,
    /// SSH connection label (e.g. "user@host") for remote terminals.
    ssh_info: Option<String>,
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
            ssh_info: None,
        }
    }

    /// Set the SSH label shown in the panel header.
    pub fn set_ssh_info(&mut self, label: String) {
        self.ssh_info = Some(label);
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
        self.inner.set_status_callback(callback);
    }

    fn set_cwd_callback(&self, callback: Option<PanelCwdCallback>) {
        self.inner.set_cwd_callback(callback);
    }

    fn panel_uuid(&self) -> Option<uuid::Uuid> {
        self.inner.panel_uuid
    }

    fn ssh_label(&self) -> Option<String> {
        self.ssh_info.clone()
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
