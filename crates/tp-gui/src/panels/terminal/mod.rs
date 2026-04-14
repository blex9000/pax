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

mod input;

#[cfg(feature = "vte")]
#[path = "vte_backend.rs"]
mod backend;

#[cfg(not(feature = "vte"))]
#[path = "pty_backend.rs"]
mod backend;

use super::PanelBackend;
use crate::panels::PanelInputCallback;
use backend::TerminalInner;

// ── Shared terminal font configuration ──────────────────────────────────────

/// Default terminal font (matches `.editor-code-view` CSS).
/// 8.25pt ≈ 11px at standard 96 DPI (11 × 72/96 = 8.25).
const DEFAULT_TERMINAL_FONT: &str = "JetBrains Mono 8.25";

/// Pango `FontDescription` for the terminal.
pub(crate) fn terminal_font_description() -> gtk4::pango::FontDescription {
    let spec = std::env::var("PAX_TERMINAL_FONT")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_TERMINAL_FONT.to_string());
    gtk4::pango::FontDescription::from_string(&spec)
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
    ) -> Self {
        Self {
            inner: TerminalInner::new(shell, cwd, env, workspace_dir),
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

    fn ssh_label(&self) -> Option<String> {
        self.ssh_info.clone()
    }

    fn accepts_input(&self) -> bool {
        true
    }
}
