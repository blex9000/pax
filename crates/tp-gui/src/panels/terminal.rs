use super::PanelBackend;

// ── VTE backend (Linux) ──────────────────────────────────────────────────────

#[cfg(feature = "vte")]
mod backend {
    use gtk4::glib;
    use gtk4::prelude::*;
    use vte4::prelude::*;

    use std::cell::RefCell;
    use std::rc::Rc;

    /// Resolve a script path: if relative, resolve against workspace_dir.
    fn resolve_script_path(path: &str, workspace_dir: &Option<String>) -> String {
        let p = std::path::Path::new(path);
        if p.is_absolute() {
            return path.to_string();
        }
        if let Some(ref dir) = workspace_dir {
            let resolved = std::path::Path::new(dir).join(path);
            return resolved.to_string_lossy().to_string();
        }
        path.to_string()
    }

    #[derive(Debug)]
    pub struct TerminalInner {
        pub vte: vte4::Terminal,
        pub widget: gtk4::Widget,
        pending_commands: Rc<RefCell<Vec<String>>>,
        _spawned: Rc<RefCell<bool>>,
        workspace_dir: Option<String>,
    }

    impl TerminalInner {
        pub fn new(shell: &str, cwd: Option<&str>, env: &[(String, String)], workspace_dir: Option<&str>) -> Self {
            let vte = vte4::Terminal::new();

            vte.set_scroll_on_output(true);
            vte.set_scroll_on_keystroke(true);
            vte.set_scrollback_lines(10_000);
            vte.set_allow_hyperlink(true);

            let pending_commands: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
            let spawned: Rc<RefCell<bool>> = Rc::new(RefCell::new(false));

            // Build environment
            let mut spawn_env: Vec<String> = std::env::vars()
                .map(|(k, v)| format!("{}={}", k, v))
                .collect();
            for (k, v) in env {
                spawn_env.push(format!("{}={}", k, v));
            }
            spawn_env.push("TERM=xterm-256color".to_string());

            let env_refs: Vec<&str> = spawn_env.iter().map(|s| s.as_str()).collect();
            let working_dir = cwd.unwrap_or(".");

            // Build argv: if there are pending startup commands, spawn shell with --init-file
            // to execute them seamlessly. Otherwise spawn plain shell.
            // We defer this — commands are queued via send_commands() and the spawn
            // callback will create a temp rcfile that sources them.
            let vte_for_cb = vte.clone();
            let pending_for_cb = pending_commands.clone();
            let spawned_for_cb = spawned.clone();

            let argv = [shell];
            vte.spawn_async(
                vte4::PtyFlags::DEFAULT,
                Some(working_dir),
                &argv,
                &env_refs,
                glib::SpawnFlags::DEFAULT,
                || {},
                -1,
                gtk4::gio::Cancellable::NONE,
                move |result| {
                    if result.is_ok() && !*spawned_for_cb.borrow() {
                        *spawned_for_cb.borrow_mut() = true;
                        // Override PS1 and set PROMPT_COMMAND for OSC 7 directory tracking
                        // (after .bashrc has run, so it sticks)
                        vte_for_cb.feed_child(b" export PS1='\\[\\033[32m\\]$:\\[\\033[0m\\] '\n");
                        vte_for_cb.feed_child(b" export PROMPT_COMMAND='printf \"\\033]7;file://%s%s\\033\\\\\" \"$HOSTNAME\" \"$PWD\"'\n");
                        vte_for_cb.feed_child(b" export LS_COLORS='di=38;2;85;136;255:ln=36:so=35:pi=33:ex=32:bd=34;46:cd=34;43:su=30;41:sg=30;46:tw=30;42:ow=34;42'\n");
                        // Clear screen to hide setup commands (before running user commands)
                        vte_for_cb.feed_child(b" clear\n");
                        // Run pending startup commands (SSH, scripts, etc.)
                        let cmds = pending_for_cb.borrow().clone();
                        for cmd in &cmds {
                            let silent = format!(" {}\n", cmd);
                            vte_for_cb.feed_child(silent.as_bytes());
                        }
                        pending_for_cb.borrow_mut().clear();
                    }
                },
            );

            // Right-click context menu for copy/paste
            let gesture = gtk4::GestureClick::new();
            gesture.set_button(3); // right click
            let vte_for_menu = vte.clone();
            gesture.connect_pressed(move |_gesture, _n, x, y| {
                let vte = &vte_for_menu;
                let popover = gtk4::PopoverMenu::from_model(None::<&gtk4::gio::MenuModel>);

                let menu_box = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
                menu_box.set_margin_top(4);
                menu_box.set_margin_bottom(4);
                menu_box.set_margin_start(4);
                menu_box.set_margin_end(4);

                // Copy
                let copy_btn = gtk4::Button::new();
                copy_btn.add_css_class("flat");
                let copy_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
                copy_box.append(&gtk4::Image::from_icon_name("edit-copy-symbolic"));
                let copy_lbl = gtk4::Label::new(Some("Copy"));
                copy_lbl.set_hexpand(true);
                copy_lbl.set_halign(gtk4::Align::Start);
                copy_box.append(&copy_lbl);
                let copy_hint = gtk4::Label::new(Some("Ctrl+Shift+C"));
                copy_hint.add_css_class("dim-label");
                copy_box.append(&copy_hint);
                copy_btn.set_child(Some(&copy_box));
                let v = vte.clone();
                let p = popover.clone();
                copy_btn.connect_clicked(move |_| {
                    v.copy_clipboard_format(vte4::Format::Text);
                    p.popdown();
                });
                menu_box.append(&copy_btn);

                // Paste
                let paste_btn = gtk4::Button::new();
                paste_btn.add_css_class("flat");
                let paste_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
                paste_box.append(&gtk4::Image::from_icon_name("edit-paste-symbolic"));
                let paste_lbl = gtk4::Label::new(Some("Paste"));
                paste_lbl.set_hexpand(true);
                paste_lbl.set_halign(gtk4::Align::Start);
                paste_box.append(&paste_lbl);
                let paste_hint = gtk4::Label::new(Some("Ctrl+Shift+V"));
                paste_hint.add_css_class("dim-label");
                paste_box.append(&paste_hint);
                paste_btn.set_child(Some(&paste_box));
                let v = vte.clone();
                let p = popover.clone();
                paste_btn.connect_clicked(move |_| {
                    v.paste_clipboard();
                    p.popdown();
                });
                menu_box.append(&paste_btn);

                popover.set_child(Some(&menu_box));
                popover.set_parent(vte);
                popover.set_pointing_to(Some(&gtk4::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
                popover.popup();
            });
            vte.add_controller(gesture);

            // Register VTE for theme color updates
            crate::theme::register_vte_terminal(&vte);

            let widget = vte.clone().upcast::<gtk4::Widget>();
            Self { vte, widget, pending_commands, _spawned: spawned, workspace_dir: workspace_dir.map(|s| s.to_string()) }
        }

        /// Queue a script to run once the shell is ready.
        /// Supports two formats:
        /// - "file:<interpreter>:<path>" — run an existing script file
        /// - Inline script text with optional shebang (#!) — written to temp file
        pub fn send_commands(&self, commands: &[String]) {
            if commands.is_empty() {
                return;
            }

            let full_text = commands.join("\n");
            if full_text.trim().is_empty() {
                return;
            }

            // Simple command mode: single line without shebang → run directly
            if !full_text.contains('\n') && !full_text.starts_with("#!") && !full_text.starts_with("file:") {
                tracing::info!("send_commands: direct command: {}", &full_text[..full_text.len().min(80)]);
                self.pending_commands.borrow_mut().push(full_text);
                return;
            }

            // File mode: "file:/bin/bash:/path/to/script.sh"
            if full_text.starts_with("file:") {
                let rest = full_text.trim_start_matches("file:");
                let (interpreter, path) = if let Some(idx) = rest[1..].find(':') {
                    let idx = idx + 1;
                    (&rest[..idx], &rest[idx + 1..])
                } else {
                    ("/bin/bash", rest)
                };
                let resolved = resolve_script_path(path, &self.workspace_dir);
                // Store as ready-to-run command
                self.pending_commands.borrow_mut().push(format!("{} {}", interpreter, resolved));
                return;
            }

            // Inline mode: write to temp file (unique per panel)
            use std::sync::atomic::{AtomicU64, Ordering};
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let tmp = std::env::temp_dir().join(format!(
                "pax_startup_{}_{}.sh",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::Relaxed),
            ));

            let interpreter = full_text.lines()
                .next()
                .filter(|l| l.starts_with("#!"))
                .map(|l| l.trim_start_matches("#!").trim().to_string())
                .unwrap_or_else(|| "/bin/bash".to_string());

            let script = if full_text.starts_with("#!") {
                full_text.clone()
            } else {
                format!("#!{}\n{}", interpreter, full_text)
            };

            if std::fs::write(&tmp, &script).is_ok() {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755));
                }
                // Ready-to-run: source the temp file and clean up
                self.pending_commands.borrow_mut().push(
                    format!("source {} ; rm -f {}", tmp.display(), tmp.display())
                );
            }
        }

        pub fn queue_raw(&self, text: &str) {
            self.pending_commands.borrow_mut().push(text.to_string());
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
    pub fn new(shell: &str, cwd: Option<&str>, env: &[(String, String)], workspace_dir: Option<&str>) -> Self {
        Self {
            inner: TerminalInner::new(shell, cwd, env, workspace_dir),
        }
    }

    pub fn send_commands(&self, commands: &[String]) {
        self.inner.send_commands(commands);
    }

    /// Queue raw text to be sent to the terminal after spawn.
    /// No processing — text is sent as-is via feed_child.
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

    fn accepts_input(&self) -> bool {
        true
    }
}
