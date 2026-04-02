//! # PTY Terminal Backend (Cross-platform fallback)
//!
//! Fallback terminal backend for platforms where VTE4 is not available (macOS).
//! Uses `portable-pty` for cross-platform PTY creation and `vt100` for basic
//! terminal escape sequence parsing, rendered into a GTK4 TextView.
//!
//! ## Current limitations (vs VTE backend)
//! - No color rendering (TERM=dumb, output is plain text)
//! - No hyperlink detection
//! - No right-click copy/paste menu
//! - No theme color integration (uses GTK theme defaults)
//! - No OSC 7 directory tracking
//! - No sync input between terminals
//! - Fixed 24x80 grid (no dynamic resize)
//! - 50ms polling interval for output updates
//!
//! ## Planned improvements
//! - ANSI color support (parse vt100 cell colors → GTK TextTags)
//! - Copy/paste context menu
//! - Scrollback buffer with ScrolledWindow
//! - Dynamic terminal size based on widget allocation
//! - Ctrl+C/D/Z signal forwarding
//!
//! ## Why this exists
//! VTE4 is a Linux-only library (requires X11/Wayland). macOS can run GTK4 apps
//! via Homebrew, but VTE4 is not practically available. This fallback ensures
//! Pax's terminal panel works on all platforms, even if with reduced features.

use gtk4::gdk;
use gtk4::glib;
use gtk4::prelude::*;
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use std::fmt;
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};

pub struct TerminalInner {
    pub text_view: gtk4::TextView,
    pub widget: gtk4::Widget,
    pub writer: Arc<Mutex<Box<dyn Write + Send>>>,
}

impl fmt::Debug for TerminalInner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TerminalInner").field("type", &"pty-fallback").finish()
    }
}

impl TerminalInner {
    pub fn new(shell: &str, cwd: Option<&str>, env: &[(String, String)], _workspace_dir: Option<&str>) -> Self {
        let text_view = gtk4::TextView::new();
        text_view.set_editable(false);
        text_view.set_cursor_visible(true);
        text_view.set_monospace(true);
        text_view.add_css_class("terminal-fallback");

        let scrolled = gtk4::ScrolledWindow::new();
        scrolled.set_child(Some(&text_view));
        scrolled.set_vexpand(true);
        scrolled.set_hexpand(true);

        let widget = scrolled.upcast::<gtk4::Widget>();

        // Create a PTY pair (master/slave) using the platform's native PTY system.
        // On macOS this uses posix_openpt, on Linux it uses /dev/ptmx.
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("Failed to open PTY");

        let mut cmd = CommandBuilder::new(shell);
        if let Some(dir) = cwd {
            cmd.cwd(dir);
        }
        for (k, v) in env {
            cmd.env(k, v);
        }
        // TERM=dumb: disables most escape sequences. The vt100 parser handles
        // basic ones, but without color rendering in the TextView, complex
        // sequences would just produce garbled output.
        cmd.env("TERM", "dumb");

        let _child = pair.slave.spawn_command(cmd).expect("Failed to spawn shell");
        drop(pair.slave);

        let writer = pair.master.take_writer().expect("Failed to take writer");
        let mut reader = pair.master.try_clone_reader().expect("Failed to clone reader");

        let writer = Arc::new(Mutex::new(writer));

        // Background thread: reads PTY output, parses through vt100, and stores
        // the rendered screen text. The GTK main loop polls this every 50ms.
        let buffer = text_view.buffer();
        let output_text = Arc::new(Mutex::new(String::new()));
        let output_text_reader = output_text.clone();

        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            let mut parser = vt100::Parser::new(24, 80, 0);
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        parser.process(&buf[..n]);
                        let screen = parser.screen();

                        // Convert the vt100 screen grid to plain text.
                        // TODO: Parse cell.fgcolor()/bgcolor() and apply GTK TextTags
                        // for ANSI color support.
                        let mut text = String::new();
                        for row in 0..screen.size().0 {
                            for col in 0..screen.size().1 {
                                if let Some(cell) = screen.cell(row, col) {
                                    let c = cell.contents();
                                    if c.is_empty() {
                                        text.push(' ');
                                    } else {
                                        text.push_str(&c);
                                    }
                                }
                            }
                            text.push('\n');
                        }
                        if let Ok(mut t) = output_text_reader.lock() {
                            *t = text;
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        // Poll output changes every 50ms and update the TextView buffer.
        // This is a simple approach; a more efficient one would use a channel
        // to signal when new output is available.
        let output_text_poll = output_text.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
            if let Ok(text) = output_text_poll.lock() {
                if !text.is_empty() {
                    buffer.set_text(&text);
                }
            }
            glib::ControlFlow::Continue
        });

        // Key input: capture keypresses and forward to the PTY.
        // Handles special keys (arrows, backspace, tab, escape) as ANSI sequences.
        let writer_clone = writer.clone();
        let key_controller = gtk4::EventControllerKey::new();
        key_controller.connect_key_pressed(move |_ctrl, key, _code, _modifiers| {
            let bytes: Vec<u8> = match key {
                gdk::Key::Return => vec![b'\r'],
                gdk::Key::BackSpace => vec![0x7f],
                gdk::Key::Tab => vec![b'\t'],
                gdk::Key::Escape => vec![0x1b],
                gdk::Key::Up => b"\x1b[A".to_vec(),
                gdk::Key::Down => b"\x1b[B".to_vec(),
                gdk::Key::Right => b"\x1b[C".to_vec(),
                gdk::Key::Left => b"\x1b[D".to_vec(),
                other => {
                    if let Some(c) = other.to_unicode() {
                        let mut buf = [0u8; 4];
                        let s = c.encode_utf8(&mut buf);
                        s.as_bytes().to_vec()
                    } else {
                        return glib::Propagation::Proceed;
                    }
                }
            };
            if let Ok(mut w) = writer_clone.lock() {
                let _ = w.write_all(&bytes);
                let _ = w.flush();
            }
            glib::Propagation::Stop
        });
        text_view.add_controller(key_controller);

        Self {
            text_view,
            widget,
            writer,
        }
    }

    /// Send commands to the running shell.
    pub fn send_commands(&self, commands: &[String]) {
        if let Ok(mut w) = self.writer.lock() {
            for cmd in commands {
                let line = format!("{}\n", cmd);
                let _ = w.write_all(line.as_bytes());
                let _ = w.flush();
            }
        }
    }

    /// Queue raw text (same as send_commands for PTY backend since there's
    /// no spawn callback — commands go directly to the shell).
    pub fn queue_raw(&self, text: &str) {
        if let Ok(mut w) = self.writer.lock() {
            let _ = w.write_all(text.as_bytes());
            let _ = w.write_all(b"\n");
            let _ = w.flush();
        }
    }

    pub fn write_input(&self, data: &[u8]) -> bool {
        if let Ok(mut w) = self.writer.lock() {
            w.write_all(data).is_ok() && w.flush().is_ok()
        } else {
            false
        }
    }

    pub fn widget(&self) -> &gtk4::Widget {
        &self.widget
    }

    pub fn grab_focus(&self) {
        self.text_view.grab_focus();
    }
}
