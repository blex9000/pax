//! # VTE Terminal Backend (Linux)
//!
//! Full-featured terminal backend using VTE4 (libvte-2.91-gtk4).
//! This is the primary terminal experience on Linux.
//!
//! ## Features
//! - True PTY with `xterm-256color` TERM
//! - 10,000 line scrollback buffer
//! - Hyperlink detection and rendering
//! - OSC 7 directory tracking (updates panel footer with `user@host:path`)
//! - Custom PS1 prompt and LS_COLORS
//! - Right-click context menu with Copy/Paste
//! - Theme color integration (background/foreground follow app theme)
//! - Startup command queuing (executed after shell initialization)
//! - Script execution: inline scripts, file references, shebang support
//!
//! ## Why VTE and not a custom terminal?
//! VTE4 is the GNOME terminal widget used by gnome-terminal, Tilix, and others.
//! It handles the full complexity of terminal emulation (mouse tracking, alternate
//! screen, true color, Unicode, ligatures, etc.) which would be impractical to
//! reimplement. The trade-off is it's Linux-only, hence the PTY fallback.

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

pub struct TerminalInner {
    pub vte: vte4::Terminal,
    pub widget: gtk4::Widget,
    pending_commands: Rc<RefCell<Vec<String>>>,
    _spawned: Rc<RefCell<bool>>,
    workspace_dir: Option<String>,
    input_cb: Rc<RefCell<Option<crate::panels::PanelInputCallback>>>,
    title_cb: Rc<RefCell<Option<crate::panels::PanelTitleCallback>>>,
}

impl std::fmt::Debug for TerminalInner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TerminalInner")
            .field("type", &"vte")
            .finish()
    }
}

impl TerminalInner {
    pub fn new(
        shell: &str,
        cwd: Option<&str>,
        env: &[(String, String)],
        workspace_dir: Option<&str>,
    ) -> Self {
        let vte = vte4::Terminal::new();

        vte.set_scroll_on_output(true);
        vte.set_scroll_on_keystroke(true);
        vte.set_scrollback_lines(10_000);
        vte.set_allow_hyperlink(true);
        vte.set_font(Some(&super::terminal_font_description()));

        // Hide terminal during init commands (PS1, LS_COLORS, PROMPT_COMMAND).
        // After init, reset + reveal, then run startup scripts with output visible.
        vte.set_opacity(0.0);


        let pending_commands: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
        let spawned: Rc<RefCell<bool>> = Rc::new(RefCell::new(false));
        let input_cb: Rc<RefCell<Option<crate::panels::PanelInputCallback>>> =
            Rc::new(RefCell::new(None));
        let title_cb: Rc<RefCell<Option<crate::panels::PanelTitleCallback>>> =
            Rc::new(RefCell::new(None));

        // Build environment: inherit current env + user overrides + TERM
        let mut spawn_env: Vec<String> = std::env::vars()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect();
        for (k, v) in env {
            spawn_env.push(format!("{}={}", k, v));
        }
        spawn_env.push("TERM=xterm-256color".to_string());

        let env_refs: Vec<&str> = spawn_env.iter().map(|s| s.as_str()).collect();
        let working_dir = cwd.unwrap_or(".");

        // Spawn the shell process in the PTY
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
                    // Override PS1 with a minimal prompt. The replacement PS1
                    // does not contain the distro-default OSC 0 title sequence,
                    // so we emit both OSC 0 (title) and OSC 7 (directory URI)
                    // from PROMPT_COMMAND on every prompt. We append rather
                    // than replace so any user PROMPT_COMMAND survives.
                    vte_for_cb.feed_child(b" export PS1='\\[\\033[32m\\]$:\\[\\033[0m\\] '\n");
                    vte_for_cb.feed_child(
                        b" __pax_prompt() { \
                             local d=\"${PWD/#$HOME/~}\"; \
                             printf '\\033]0;%s@%s: %s\\007' \"$USER\" \"$HOSTNAME\" \"$d\"; \
                             printf '\\033]7;file://%s%s\\033\\\\' \"$HOSTNAME\" \"$PWD\"; \
                         }\n",
                    );
                    vte_for_cb.feed_child(b" PROMPT_COMMAND=\"${PROMPT_COMMAND:+$PROMPT_COMMAND; }__pax_prompt\"\n");
                    vte_for_cb.feed_child(b" export LS_COLORS='di=38;2;85;136;255:ln=36:so=35:pi=33:ex=32:bd=34;46:cd=34;43:su=30;41:sg=30;46:tw=30;42:ow=34;42'\n");
                    // Clear screen to hide setup commands
                    vte_for_cb.feed_child(b" clear\n");

                    let cmds = pending_for_cb.borrow().clone();
                    pending_for_cb.borrow_mut().clear();

                    // Reset wipes init noise, reveal the terminal, then
                    // run startup commands so their output is visible.
                    let vte_show = vte_for_cb.clone();
                    glib::timeout_add_local_once(std::time::Duration::from_millis(800), move || {
                        vte_show.reset(true, true);
                        vte_show.feed_child(b"\n");
                        vte_show.set_opacity(1.0);
                        for cmd in &cmds {
                            let line = format!(" {}\n", cmd);
                            vte_show.feed_child(line.as_bytes());
                        }
                    });
                }
            },
        );

        // Right-click context menu for copy/paste
        Self::setup_context_menu(&vte);
        Self::setup_input_observer(&vte, input_cb.clone());

        // Forward OSC 0/2 title changes to the registered callback. VTE
        // emits this signal whenever the PTY sends ESC]0; or ESC]2; — we
        // pass the raw string and let PanelHost sanitize and render.
        {
            let title_cb_ref = title_cb.clone();
            vte.connect_window_title_changed(move |term| {
                if let Ok(borrowed) = title_cb_ref.try_borrow() {
                    if let Some(ref cb) = *borrowed {
                        let title = term
                            .window_title()
                            .map(|s| s.to_string())
                            .unwrap_or_default();
                        cb(&title);
                    }
                }
            });
        }

        // Register VTE for theme color updates
        crate::theme::register_vte_terminal(&vte);

        let widget = vte.clone().upcast::<gtk4::Widget>();
        Self {
            vte,
            widget,
            pending_commands,
            _spawned: spawned,
            workspace_dir: workspace_dir.map(|s| s.to_string()),
            input_cb,
            title_cb,
        }
    }

    fn setup_input_observer(
        vte: &vte4::Terminal,
        input_cb: Rc<RefCell<Option<crate::panels::PanelInputCallback>>>,
    ) {
        let key_controller = gtk4::EventControllerKey::new();
        key_controller.set_propagation_phase(gtk4::PropagationPhase::Capture);
        let vte_for_keys = vte.clone();
        key_controller.connect_key_pressed(move |_ctrl, key, _code, modifiers| {
            if let Some(action) = super::input::terminal_clipboard_action(key, modifiers) {
                match action {
                    super::input::TerminalClipboardAction::Copy => {
                        vte_for_keys.copy_clipboard_format(vte4::Format::Text);
                    }
                    super::input::TerminalClipboardAction::Paste => {
                        vte_for_keys.paste_clipboard();
                    }
                }
                return glib::Propagation::Stop;
            }

            if let Some(bytes) = super::input::encode_key_input(key, modifiers) {
                if let Ok(borrowed) = input_cb.try_borrow() {
                    if let Some(ref cb) = *borrowed {
                        cb(&bytes);
                    }
                }
            }
            glib::Propagation::Proceed
        });
        vte.add_controller(key_controller);
    }

    /// Build the right-click copy/paste context menu.
    fn setup_context_menu(vte: &vte4::Terminal) {
        let gesture = gtk4::GestureClick::new();
        gesture.set_button(3);
        let vte_for_menu = vte.clone();
        gesture.connect_pressed(move |_gesture, _n, x, y| {
            let vte = &vte_for_menu;
            let popover = gtk4::PopoverMenu::from_model(None::<&gtk4::gio::MenuModel>);
            crate::theme::configure_popover(&popover);

            let menu_box = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
            menu_box.set_margin_top(4);
            menu_box.set_margin_bottom(4);
            menu_box.set_margin_start(4);
            menu_box.set_margin_end(4);

            // Copy button
            let copy_btn = gtk4::Button::new();
            copy_btn.add_css_class("flat");
            copy_btn.add_css_class("app-popover-button");
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

            // Paste button
            let paste_btn = gtk4::Button::new();
            paste_btn.add_css_class("flat");
            paste_btn.add_css_class("app-popover-button");
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
    }

    /// Queue a script to run once the shell is ready.
    ///
    /// Supports three formats:
    /// - Simple command string → run directly in shell
    /// - `"file:<interpreter>:<path>"` → run an existing script file
    /// - Multi-line or shebang text → written to temp file, sourced, then deleted
    pub fn send_commands(&self, commands: &[String]) {
        if commands.is_empty() {
            return;
        }

        let full_text = commands.join("\n");
        if full_text.trim().is_empty() {
            return;
        }

        // Simple command: single line without shebang → run directly
        if !full_text.contains('\n')
            && !full_text.starts_with("#!")
            && !full_text.starts_with("file:")
        {
            tracing::info!(
                "send_commands: direct command: {}",
                &full_text[..full_text.len().min(80)]
            );
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
            self.pending_commands
                .borrow_mut()
                .push(format!("{} {}", interpreter, resolved));
            return;
        }

        // Inline script mode: write to temp file, source it, clean up
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let tmp = std::env::temp_dir().join(format!(
            "pax_startup_{}_{}.sh",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed),
        ));

        let interpreter = full_text
            .lines()
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
            self.pending_commands.borrow_mut().push(format!(
                "source {} ; rm -f {}",
                tmp.display(),
                tmp.display()
            ));
        }
    }

    pub fn queue_raw(&self, text: &str) {
        self.pending_commands.borrow_mut().push(text.to_string());
    }

    pub fn write_input(&self, data: &[u8]) -> bool {
        self.vte.feed_child(data);
        true
    }

    pub fn set_input_callback(&self, callback: Option<crate::panels::PanelInputCallback>) {
        *self.input_cb.borrow_mut() = callback;
    }

    pub fn set_title_callback(&self, callback: Option<crate::panels::PanelTitleCallback>) {
        *self.title_cb.borrow_mut() = callback;
    }

    pub fn widget(&self) -> &gtk4::Widget {
        &self.widget
    }

    pub fn grab_focus(&self) {
        self.vte.grab_focus();
    }

    /// Terminate the child process and release resources.
    /// Unregisters from theme tracking (breaks the strong reference that
    /// prevented GObject finalization) and sends Ctrl+C to gracefully
    /// stop the foreground process. When the VTE widget is finalized,
    /// the PTY closes and the child receives SIGHUP.
    pub fn shutdown(&self) {
        crate::theme::unregister_vte_terminal(&self.vte);
        self.vte.feed_child(b"\x03");
    }
}
