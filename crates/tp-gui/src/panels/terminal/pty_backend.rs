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
//! - 50ms polling interval for output updates
//!
//! ## Planned improvements
//! - ANSI color support (parse vt100 cell colors → GTK TextTags)
//! - Copy/paste context menu
//! - Scrollback buffer with ScrolledWindow
//! - Ctrl+C/D/Z signal forwarding
//!
//! ## Why this exists
//! VTE4 is a Linux-only library (requires X11/Wayland). macOS can run GTK4 apps
//! via Homebrew, but VTE4 is not practically available. This fallback ensures
//! Pax's terminal panel works on all platforms, even if with reduced features.

use gtk4::glib;
use gtk4::prelude::*;
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use std::cell::RefCell;
use std::fmt;
use std::io::{Read, Write};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

pub struct TerminalInner {
    pub text_view: gtk4::TextView,
    pub widget: gtk4::Widget,
    pub writer: Arc<Mutex<Box<dyn Write + Send>>>,
    input_cb: Rc<RefCell<Option<crate::panels::PanelInputCallback>>>,
}

impl fmt::Debug for TerminalInner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TerminalInner")
            .field("type", &"pty-fallback")
            .finish()
    }
}

impl TerminalInner {
    pub fn new(
        shell: &str,
        cwd: Option<&str>,
        env: &[(String, String)],
        _workspace_dir: Option<&str>,
    ) -> Self {
        let text_view = gtk4::TextView::new();
        text_view.set_editable(false);
        text_view.set_cursor_visible(true);
        text_view.set_monospace(true);
        text_view.add_css_class("terminal-fallback");
        text_view.set_top_margin(0);
        text_view.set_bottom_margin(0);
        text_view.set_valign(gtk4::Align::Fill);
        text_view.set_vexpand(true);
        text_view.set_hexpand(true);

        let scrolled = gtk4::ScrolledWindow::new();
        scrolled.set_child(Some(&text_view));
        scrolled.set_vexpand(true);
        scrolled.set_hexpand(true);
        scrolled.set_propagate_natural_height(false);
        scrolled.set_propagate_natural_width(false);

        let widget = scrolled.clone().upcast::<gtk4::Widget>();

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

        let _child = pair
            .slave
            .spawn_command(cmd)
            .expect("Failed to spawn shell");
        drop(pair.slave);

        let master: Arc<Mutex<Box<dyn MasterPty + Send>>> = Arc::new(Mutex::new(pair.master));
        let writer = {
            let guard = master.lock().expect("PTY master lock poisoned");
            guard.take_writer().expect("Failed to take writer")
        };
        let mut reader = {
            let guard = master.lock().expect("PTY master lock poisoned");
            guard.try_clone_reader().expect("Failed to clone reader")
        };

        let writer = Arc::new(Mutex::new(writer));
        let input_cb: Rc<RefCell<Option<crate::panels::PanelInputCallback>>> =
            Rc::new(RefCell::new(None));
        let parser = Arc::new(Mutex::new(vt100::Parser::new(24, 80, 0)));

        // Background thread: reads PTY output, parses through vt100, and stores
        // the rendered screen text. The GTK main loop polls this every 50ms.
        let buffer = text_view.buffer();
        let output_text = Arc::new(Mutex::new(String::new()));
        let output_text_reader = output_text.clone();
        let parser_reader = parser.clone();

        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        let rendered = {
                            let mut parser =
                                parser_reader.lock().expect("PTY parser lock poisoned");
                            parser.process(&buf[..n]);
                            render_screen_text(parser.screen())
                        };
                        if let Ok(mut t) = output_text_reader.lock() {
                            *t = rendered;
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

        // Poll allocation changes and keep the PTY/parser grid aligned to the
        // visible terminal area so the fallback backend uses the full height.
        let scrolled_resize = scrolled.clone();
        let text_view_resize = text_view.clone();
        let master_resize = master.clone();
        let parser_resize = parser.clone();
        let output_text_resize = output_text.clone();
        let last_grid = Rc::new(RefCell::new((24u16, 80u16)));
        let last_grid_resize = last_grid.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
            let alloc = scrolled_resize.allocation();
            if let Some(size) = widget_grid_size(&text_view_resize, alloc.width(), alloc.height()) {
                let new_grid = (size.rows, size.cols);
                if *last_grid_resize.borrow() != new_grid {
                    if let Ok(master) = master_resize.lock() {
                        let _ = master.resize(size);
                    }
                    if let Ok(mut parser) = parser_resize.lock() {
                        parser.set_size(size.rows, size.cols);
                        let rendered = render_screen_text(parser.screen());
                        if let Ok(mut text) = output_text_resize.lock() {
                            *text = rendered;
                        }
                    }
                    *last_grid_resize.borrow_mut() = new_grid;
                }
            }
            glib::ControlFlow::Continue
        });

        // Key input: capture keypresses and forward to the PTY.
        // Handles special keys (arrows, backspace, tab, escape) as ANSI sequences.
        let writer_clone = writer.clone();
        let input_cb_clone = input_cb.clone();
        let key_controller = gtk4::EventControllerKey::new();
        key_controller.connect_key_pressed(move |_ctrl, key, _code, modifiers| {
            let Some(bytes) = super::input::encode_key_input(key, modifiers) else {
                return glib::Propagation::Proceed;
            };
            if let Ok(mut w) = writer_clone.lock() {
                let _ = w.write_all(&bytes);
                let _ = w.flush();
            }
            if let Ok(borrowed) = input_cb_clone.try_borrow() {
                if let Some(ref cb) = *borrowed {
                    cb(&bytes);
                }
            }
            glib::Propagation::Stop
        });
        text_view.add_controller(key_controller);

        Self {
            text_view,
            widget,
            writer,
            input_cb,
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

    pub fn set_input_callback(&self, callback: Option<crate::panels::PanelInputCallback>) {
        *self.input_cb.borrow_mut() = callback;
    }

    pub fn widget(&self) -> &gtk4::Widget {
        &self.widget
    }

    pub fn grab_focus(&self) {
        self.text_view.grab_focus();
    }
}

fn render_screen_text(screen: &vt100::Screen) -> String {
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
    text
}

fn widget_grid_size(view: &gtk4::TextView, width: i32, height: i32) -> Option<PtySize> {
    let layout = view.create_pango_layout(Some("W"));
    let (char_width, char_height) = layout.pixel_size();
    grid_dimensions_for_area(width, height, char_width, char_height)
}

fn grid_dimensions_for_area(
    width: i32,
    height: i32,
    char_width: i32,
    char_height: i32,
) -> Option<PtySize> {
    if width <= 0 || height <= 0 || char_width <= 0 || char_height <= 0 {
        return None;
    }

    let rows = (height / char_height).max(1) as u16;
    let cols = (width / char_width).max(1) as u16;

    Some(PtySize {
        rows,
        cols,
        pixel_width: char_width.min(u16::MAX as i32) as u16,
        pixel_height: char_height.min(u16::MAX as i32) as u16,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_dimensions_scale_with_available_area() {
        let size = grid_dimensions_for_area(800, 480, 10, 20).expect("size");
        assert_eq!(size.cols, 80);
        assert_eq!(size.rows, 24);
        assert_eq!(size.pixel_width, 10);
        assert_eq!(size.pixel_height, 20);
    }

    #[test]
    fn grid_dimensions_clamp_to_minimum_cell() {
        let size = grid_dimensions_for_area(5, 5, 10, 20).expect("size");
        assert_eq!(size.cols, 1);
        assert_eq!(size.rows, 1);
    }

    #[test]
    fn grid_dimensions_reject_invalid_metrics() {
        assert!(grid_dimensions_for_area(0, 100, 10, 20).is_none());
        assert!(grid_dimensions_for_area(100, 100, 0, 20).is_none());
    }
}
