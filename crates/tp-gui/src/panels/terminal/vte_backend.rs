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

use std::cell::{Cell, RefCell};
use std::os::fd::AsRawFd;
use std::rc::Rc;
use std::time::Duration;

use super::script_runner::prepare_startup_command;
use super::shell_bootstrap::{bootstrap_lines, BootstrapConfig};

/// Minimum VTE version exposing the `shell-precmd` / `shell-preexec`
/// GObject signals used for OSC 133 shell integration.
const VTE_SHELL_INTEGRATION_MINOR: u32 = 80;

/// Polling interval for the `tcgetpgrp` fallback used when the linked
/// VTE runtime does not emit OSC 133 signals. 200ms is imperceptible
/// from the user's side and CPU-free.
const TCGETPGRP_POLL_MS: u64 = 200;
const AUTO_SCROLL_BOTTOM_EPSILON: f64 = 1.0;

/// True when the linked libvte runtime exposes the OSC 133 shell
/// integration signals. Older runtimes (e.g. VTE 0.76 on Debian stable)
/// abort with `assertion failed: handle > 0` inside `glib::signal::connect_raw`
/// if we try to connect a signal the library does not ship.
fn vte_has_shell_integration_signals() -> bool {
    let minor = unsafe { vte4::ffi::vte_get_minor_version() };
    minor >= VTE_SHELL_INTEGRATION_MINOR
}

fn setup_smart_scroll_on_output(vte: &vte4::Terminal) {
    let update = Rc::new({
        let vte = vte.clone();
        move || {
            let at_bottom = vte
                .vadjustment()
                .map(|adj| adjustment_is_at_bottom(&adj))
                .unwrap_or(true);
            vte.set_scroll_on_output(at_bottom);
        }
    });

    if let Some(adj) = vte.vadjustment() {
        let update = update.clone();
        adj.connect_value_changed(move |_| update());
    }

    {
        let update = update.clone();
        vte.connect_vadjustment_notify(move |term| {
            if let Some(adj) = term.vadjustment() {
                let update = update.clone();
                adj.connect_value_changed(move |_| update());
            }
            update();
        });
    }

    update();
}

fn adjustment_is_at_bottom(adj: &gtk4::Adjustment) -> bool {
    adjustment_values_are_at_bottom(adj.value(), adj.lower(), adj.upper(), adj.page_size())
}

fn adjustment_values_are_at_bottom(value: f64, lower: f64, upper: f64, page_size: f64) -> bool {
    let bottom = (upper - page_size).max(lower);
    value >= bottom - AUTO_SCROLL_BOTTOM_EPSILON
}

/// OSC 133 fallback: poll `tcgetpgrp` on the PTY master every
/// `TCGETPGRP_POLL_MS` and emit status transitions. The shell is
/// "busy" whenever the foreground process group differs from the
/// shell's own PID — i.e. a child command is in the foreground.
/// Holds a weak ref to the terminal so the timer stops when the
/// panel is dropped.
fn spawn_tcgetpgrp_poller(
    vte: &vte4::Terminal,
    shell_pid: Rc<Cell<Option<i32>>>,
    status_cb: Rc<RefCell<Option<crate::panels::PanelStatusCallback>>>,
) {
    let vte_weak = vte.downgrade();
    let last_busy: Rc<Cell<Option<bool>>> = Rc::new(Cell::new(None));
    glib::timeout_add_local(Duration::from_millis(TCGETPGRP_POLL_MS), move || {
        let Some(term) = vte_weak.upgrade() else {
            return glib::ControlFlow::Break;
        };
        if !term.is_mapped() {
            return glib::ControlFlow::Continue;
        }
        let Some(pid) = shell_pid.get() else {
            return glib::ControlFlow::Continue;
        };
        let Some(pty) = term.pty() else {
            return glib::ControlFlow::Continue;
        };
        let pgrp = unsafe { libc::tcgetpgrp(pty.fd().as_raw_fd()) };
        if pgrp < 0 {
            return glib::ControlFlow::Continue;
        }
        let busy = pgrp != pid;
        if last_busy.get() != Some(busy) {
            last_busy.set(Some(busy));
            if let Ok(borrowed) = status_cb.try_borrow() {
                if let Some(ref cb) = *borrowed {
                    cb(busy);
                }
            }
        }
        glib::ControlFlow::Continue
    });
}

pub struct TerminalInner {
    pub vte: vte4::Terminal,
    pub widget: gtk4::Widget,
    pending_commands: Rc<RefCell<Vec<String>>>,
    _spawned: Rc<RefCell<bool>>,
    workspace_dir: Option<String>,
    pub(super) panel_uuid: Option<uuid::Uuid>,
    pub(super) cmd_file: std::path::PathBuf,
    /// Held to keep the gio::FileMonitor alive for the panel's lifetime.
    _cmd_file_monitor: Option<gtk4::gio::FileMonitor>,
    input_cb: Rc<RefCell<Option<crate::panels::PanelInputCallback>>>,
    title_cb: Rc<RefCell<Option<crate::panels::PanelTitleCallback>>>,
    status_cb: Rc<RefCell<Option<crate::panels::PanelStatusCallback>>>,
    cwd_cb: Rc<RefCell<Option<crate::panels::PanelCwdCallback>>>,
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
        panel_uuid: Option<uuid::Uuid>,
    ) -> Self {
        let vte = vte4::Terminal::new();

        vte.set_scroll_on_output(true);
        vte.set_scroll_on_keystroke(true);
        vte.set_scrollback_lines(10_000);
        vte.set_allow_hyperlink(true);
        vte.set_font(Some(&super::terminal_font_description()));
        setup_smart_scroll_on_output(&vte);

        // Hide terminal during init commands (PS1, LS_COLORS, PROMPT_COMMAND).
        // After init, reset + reveal, then run startup scripts with output visible.
        vte.set_opacity(0.0);

        let pending_commands: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
        let spawned: Rc<RefCell<bool>> = Rc::new(RefCell::new(false));
        let input_cb: Rc<RefCell<Option<crate::panels::PanelInputCallback>>> =
            Rc::new(RefCell::new(None));
        let title_cb: Rc<RefCell<Option<crate::panels::PanelTitleCallback>>> =
            Rc::new(RefCell::new(None));
        let status_cb: Rc<RefCell<Option<crate::panels::PanelStatusCallback>>> =
            Rc::new(RefCell::new(None));
        let cwd_cb: Rc<RefCell<Option<crate::panels::PanelCwdCallback>>> =
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

        // Resolve the per-panel sidechannel path once and create the file
        // upfront with mode 0600 so the FileMonitor below has something to
        // watch and the shell's first redirect does not race the chmod.
        let cmd_file_path: std::path::PathBuf = panel_uuid
            .map(|u| super::shell_bootstrap::cmd_file_path(&u))
            .unwrap_or_default();
        super::shell_bootstrap::prepare_cmd_file(&cmd_file_path);

        // Spawn the shell process in the PTY
        let vte_for_cb = vte.clone();
        let pending_for_cb = pending_commands.clone();
        let spawned_for_cb = spawned.clone();
        let shell_for_cb = shell.to_string();
        let panel_uuid_for_cb: Option<uuid::Uuid> = panel_uuid;
        // PID captured on successful spawn and consumed by the tcgetpgrp
        // fallback poller below. Stays None on spawn failure, which
        // silently disables the fallback.
        let shell_pid: Rc<Cell<Option<i32>>> = Rc::new(Cell::new(None));
        let shell_pid_for_cb = shell_pid.clone();

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
                if let Ok(pid) = &result {
                    shell_pid_for_cb.set(Some(pid.0));
                }
                if result.is_ok() && !*spawned_for_cb.borrow() {
                    *spawned_for_cb.borrow_mut() = true;
                    // Feed pax's standard bootstrap (see `shell_bootstrap` for
                    // the full rationale). Same payload used by the PTY
                    // backend modulo two switches:
                    //   - override_ps1: VTE replaces the distro PS1 with the
                    //     minimal green prompt ("$: ").
                    //   - emit_osc7: only VTE consumes OSC 7 to drive the
                    //     footer via `current-directory-uri-changed`.
                    let shell_kind =
                        super::shell_bootstrap::ShellKind::detect_from_path(&shell_for_cb);
                    let cmd_file = match panel_uuid_for_cb {
                        Some(u) => super::shell_bootstrap::cmd_file_path(&u),
                        None => std::path::PathBuf::new(),
                    };
                    for line in bootstrap_lines(&BootstrapConfig {
                        shell: shell_kind,
                        override_ps1: true,
                        emit_osc7: true,
                        cmd_file: &cmd_file,
                    }) {
                        let mut bytes = line.into_bytes();
                        bytes.push(b'\n');
                        vte_for_cb.feed_child(&bytes);
                    }

                    let cmds = pending_for_cb.borrow().clone();
                    pending_for_cb.borrow_mut().clear();

                    // Reset wipes init noise, reveal the terminal, then
                    // run startup commands so their output is visible.
                    let vte_show = vte_for_cb.clone();
                    glib::timeout_add_local_once(
                        std::time::Duration::from_millis(800),
                        move || {
                            vte_show.reset(true, true);
                            vte_show.feed_child(b"\n");
                            vte_show.set_opacity(1.0);
                            for cmd in &cmds {
                                let line = format!(" {}\n", cmd);
                                vte_show.feed_child(line.as_bytes());
                            }
                        },
                    );
                }
            },
        );

        // Right-click context menu for copy/paste
        Self::setup_context_menu(&vte);
        Self::setup_input_observer(&vte, input_cb.clone());
        Self::setup_hyperlink_click(&vte);

        // Forward OSC 7 (current-directory-uri) updates to the registered
        // callback. The host formats the URI into the footer bar —
        // identical path to the PTY backend's scanner-driven route.
        {
            let cwd_cb_ref = cwd_cb.clone();
            vte.connect_current_directory_uri_changed(move |term| {
                let uri = term
                    .current_directory_uri()
                    .map(|s| s.to_string())
                    .unwrap_or_default();
                if let Ok(borrowed) = cwd_cb_ref.try_borrow() {
                    if let Some(ref cb) = *borrowed {
                        cb(&uri);
                    }
                }
            });
        }

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

        // Forward OSC 133 shell integration: preexec (C) = command started
        // → indicator ON (busy); precmd (A) = prompt back → indicator OFF.
        // These signals were added in VTE 0.80; skip the wiring on older
        // runtimes (Debian/Ubuntu LTS ships 0.76) to avoid a
        // `connect_raw: handle > 0` assertion on missing GObject signals.
        //
        // Note: command history capture is NOT done here. It runs through
        // `gio::FileMonitor` (see `cmd_file_monitor` field) so that builtins
        // (which never change the foreground pgroup) are also captured and
        // it works the same on every VTE version.
        if vte_has_shell_integration_signals() {
            let status_cb_ref = status_cb.clone();
            vte.connect_shell_precmd(move |_| {
                if let Ok(borrowed) = status_cb_ref.try_borrow() {
                    if let Some(ref cb) = *borrowed {
                        cb(false);
                    }
                }
            });
            let status_cb_ref = status_cb.clone();
            vte.connect_shell_preexec(move |_| {
                if let Ok(borrowed) = status_cb_ref.try_borrow() {
                    if let Some(ref cb) = *borrowed {
                        cb(true);
                    }
                }
            });
        } else {
            // VTE < 0.80 does not ship shell-precmd / shell-preexec. Fall
            // back to polling `tcgetpgrp` on the PTY master: the foreground
            // process group equals the shell PID only while the shell is
            // at its prompt. Independent of any OSC 133 cooperation from
            // the shell, so it also works on zsh/fish/dash.
            spawn_tcgetpgrp_poller(&vte, shell_pid.clone(), status_cb.clone());
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
            panel_uuid,
            cmd_file: cmd_file_path.clone(),
            _cmd_file_monitor: super::shell_bootstrap::spawn_cmd_file_watcher(
                &cmd_file_path,
                panel_uuid.map(|u| u.simple().to_string()),
                workspace_dir.map(|s| s.to_string()),
            ),
            input_cb,
            title_cb,
            status_cb,
            cwd_cb,
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

    /// Open OSC 8 hyperlinks under the cursor on Ctrl+left-click.
    /// VTE renders the underline/hover itself when `allow_hyperlink` is on;
    /// we only wire the click → URI launch. Plain left-click without the
    /// modifier is left to VTE for text selection.
    fn setup_hyperlink_click(vte: &vte4::Terminal) {
        let gesture = gtk4::GestureClick::new();
        gesture.set_button(1);
        gesture.set_propagation_phase(gtk4::PropagationPhase::Capture);
        let vte_for_click = vte.clone();
        gesture.connect_released(move |g, n_press, x, y| {
            if n_press != 1 {
                return;
            }
            if !g
                .current_event_state()
                .contains(gtk4::gdk::ModifierType::CONTROL_MASK)
            {
                return;
            }
            let Some(uri) = vte_for_click.check_hyperlink_at(x, y) else {
                return;
            };
            let uri_str = uri.to_string();
            if let Err(e) = gtk4::gio::AppInfo::launch_default_for_uri(
                &uri_str,
                None::<&gtk4::gio::AppLaunchContext>,
            ) {
                tracing::warn!("Failed to launch hyperlink {}: {}", uri_str, e);
            }
            g.set_state(gtk4::EventSequenceState::Claimed);
        });
        vte.add_controller(gesture);
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
        if let Some(line) = prepare_startup_command(commands, self.workspace_dir.as_deref()) {
            self.pending_commands.borrow_mut().push(line);
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

    pub fn set_status_callback(&self, callback: Option<crate::panels::PanelStatusCallback>) {
        *self.status_cb.borrow_mut() = callback;
    }

    pub fn set_cwd_callback(&self, callback: Option<crate::panels::PanelCwdCallback>) {
        *self.cwd_cb.borrow_mut() = callback;
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
        if !self.cmd_file.as_os_str().is_empty() {
            let _ = std::fs::remove_file(&self.cmd_file);
        }
        crate::theme::unregister_vte_terminal(&self.vte);
        self.vte.feed_child(b"\x03");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adjustment_bottom_detection_allows_small_float_jitter() {
        assert!(adjustment_values_are_at_bottom(89.5, 0.0, 100.0, 10.0));
        assert!(!adjustment_values_are_at_bottom(88.0, 0.0, 100.0, 10.0));
    }
}
