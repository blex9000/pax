use gtk4::prelude::*;

use super::PanelBackend;

// ── VTE backend (Linux) ──────────────────────────────────────────────────────

#[cfg(feature = "vte")]
mod backend {
    use gtk4::glib;
    use gtk4::prelude::*;
    use vte4::prelude::*;

    #[derive(Debug)]
    pub struct TerminalInner {
        pub vte: vte4::Terminal,
        pub widget: gtk4::Widget,
    }

    impl TerminalInner {
        pub fn new(shell: &str, cwd: Option<&str>, env: &[(String, String)]) -> Self {
            let vte = vte4::Terminal::new();

            vte.set_scroll_on_output(true);
            vte.set_scroll_on_keystroke(true);
            vte.set_scrollback_lines(10_000);
            vte.set_allow_hyperlink(true);

            // Build environment
            let mut spawn_env: Vec<String> = std::env::vars()
                .map(|(k, v)| format!("{}={}", k, v))
                .collect();
            for (k, v) in env {
                spawn_env.push(format!("{}={}", k, v));
            }
            spawn_env.push("TERM=xterm-256color".to_string());

            let env_refs: Vec<&str> = spawn_env.iter().map(|s| s.as_str()).collect();
            let argv = [shell];
            let working_dir = cwd.unwrap_or(".");

            vte.spawn_async(
                vte4::PtyFlags::DEFAULT,
                Some(working_dir),
                &argv,
                &env_refs,
                glib::SpawnFlags::DEFAULT,
                || {},
                -1,
                gtk4::gio::Cancellable::NONE,
                |_result| {},
            );

            let widget = vte.clone().upcast::<gtk4::Widget>();
            Self { vte, widget }
        }

        pub fn send_commands(&self, commands: &[String]) {
            for cmd in commands {
                let line = format!("{}\n", cmd);
                self.vte.feed_child(line.as_bytes());
            }
        }

        pub fn write_input(&self, data: &[u8]) -> bool {
            self.vte.feed_child(data);
            true
        }

        pub fn widget(&self) -> &gtk4::Widget {
            &self.widget
        }

        pub fn grab_focus(&self) {
            self.vte.grab_focus();
        }
    }
}

// ── PTY + TextView fallback (macOS / no-vte) ────────────────────────────────

#[cfg(not(feature = "vte"))]
mod backend {
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
        pub fn new(shell: &str, cwd: Option<&str>, env: &[(String, String)]) -> Self {
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

            // Spawn PTY
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
            cmd.env("TERM", "dumb");

            let _child = pair.slave.spawn_command(cmd).expect("Failed to spawn shell");
            drop(pair.slave);

            let writer = pair.master.take_writer().expect("Failed to take writer");
            let mut reader = pair.master.try_clone_reader().expect("Failed to clone reader");

            let writer = Arc::new(Mutex::new(writer));

            // Read PTY output in a thread, feed to TextView via glib idle callback
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

            // Poll output changes every 50ms
            let output_text_poll = output_text.clone();
            glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
                if let Ok(text) = output_text_poll.lock() {
                    if !text.is_empty() {
                        buffer.set_text(&text);
                    }
                }
                glib::ControlFlow::Continue
            });

            // Key input: capture keypresses and send to PTY
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

        pub fn send_commands(&self, commands: &[String]) {
            if let Ok(mut w) = self.writer.lock() {
                for cmd in commands {
                    let line = format!("{}\n", cmd);
                    let _ = w.write_all(line.as_bytes());
                    let _ = w.flush();
                }
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
}

// ── Public API (same for both backends) ──────────────────────────────────────

use backend::TerminalInner;

/// Terminal panel — uses VTE4 on Linux, PTY+TextView fallback on macOS.
#[derive(Debug)]
pub struct TerminalPanel {
    inner: TerminalInner,
}

impl TerminalPanel {
    pub fn new(shell: &str, cwd: Option<&str>, env: &[(String, String)]) -> Self {
        Self {
            inner: TerminalInner::new(shell, cwd, env),
        }
    }

    pub fn send_commands(&self, commands: &[String]) {
        self.inner.send_commands(commands);
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

    fn accepts_input(&self) -> bool {
        true
    }
}
