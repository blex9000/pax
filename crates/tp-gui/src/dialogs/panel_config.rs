use gtk4::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

use pax_core::workspace::{PanelType, SshConfig, NamedSshConfig};

/// Callback: (name, panel_type, cwd, ssh, startup_commands, before_close, min_width, min_height)
pub type ConfigDoneCallback = dyn Fn(String, PanelType, Option<String>, Option<SshConfig>, Vec<String>, Option<String>, u32, u32) + 'static;

/// Show a configuration dialog for the given panel type.
pub fn show_panel_config_dialog(
    parent: &impl IsA<gtk4::Window>,
    panel_name: &str,
    panel_type: &PanelType,
    cwd: Option<&str>,
    ssh: Option<&SshConfig>,
    startup_commands: &[String],
    before_close: Option<&str>,
    min_width: u32,
    min_height: u32,
    saved_ssh: Rc<RefCell<Vec<NamedSshConfig>>>,
    on_done: impl Fn(String, PanelType, Option<String>, Option<SshConfig>, Vec<String>, Option<String>, u32, u32) + 'static,
) {
    match panel_type {
        PanelType::Terminal | PanelType::Ssh { .. } | PanelType::RemoteTmux { .. } => {
            show_terminal_config(parent, panel_name, cwd, ssh, startup_commands, before_close, min_width, min_height, saved_ssh, on_done)
        }
        PanelType::Markdown { file } => show_markdown_config(parent, panel_name, file, min_width, min_height, on_done),
        PanelType::Browser { url } => show_browser_config(parent, panel_name, url, min_width, min_height, on_done),
        PanelType::CodeEditor { root_dir, ssh: editor_ssh, remote_path } => show_code_editor_config(parent, panel_name, root_dir, editor_ssh.as_ref(), remote_path.as_deref(), min_width, min_height, saved_ssh, on_done),
        PanelType::Empty => {}
    }
}

fn make_dialog(parent: &impl IsA<gtk4::Window>, title: &str) -> gtk4::Window {
    gtk4::Window::builder()
        .title(title)
        .transient_for(parent)
        .modal(true)
        .default_width(500)
        .default_height(400)
        .build()
}

fn add_field(vbox: &gtk4::Box, label: &str, value: &str, placeholder: &str) -> gtk4::Entry {
    let hbox = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    let lbl = gtk4::Label::new(Some(label));
    lbl.set_width_chars(15);
    lbl.set_halign(gtk4::Align::Start);
    let entry = gtk4::Entry::new();
    entry.set_text(value);
    entry.set_placeholder_text(Some(placeholder));
    entry.set_hexpand(true);
    hbox.append(&lbl);
    hbox.append(&entry);
    vbox.append(&hbox);
    entry
}

fn add_buttons(vbox: &gtk4::Box, dialog: &gtk4::Window, on_apply: impl Fn() + 'static) {
    let btn_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    btn_box.set_halign(gtk4::Align::End);
    btn_box.set_margin_top(12);

    let cancel_btn = gtk4::Button::with_label("Cancel");
    cancel_btn.add_css_class("flat");
    let apply_btn = gtk4::Button::with_label("Apply");
    apply_btn.add_css_class("suggested-action");

    let d = dialog.clone();
    cancel_btn.connect_clicked(move |_| d.close());

    let d = dialog.clone();
    apply_btn.connect_clicked(move |_| {
        on_apply();
        d.close();
    });

    btn_box.append(&cancel_btn);
    btn_box.append(&apply_btn);
    vbox.append(&btn_box);
}

/// Add min width/height spin buttons. Returns (min_width_spin, min_height_spin).
fn add_min_size_fields(vbox: &gtk4::Box, min_width: u32, min_height: u32) -> (gtk4::SpinButton, gtk4::SpinButton) {
    let sep = gtk4::Separator::new(gtk4::Orientation::Horizontal);
    sep.set_margin_top(8);
    sep.set_margin_bottom(4);
    vbox.append(&sep);

    let size_label = gtk4::Label::new(Some("Minimum size (0 = auto):"));
    size_label.set_halign(gtk4::Align::Start);
    size_label.add_css_class("dim-label");
    vbox.append(&size_label);

    let hbox = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    let w_lbl = gtk4::Label::new(Some("Width:"));
    w_lbl.set_halign(gtk4::Align::Start);
    let w_spin = gtk4::SpinButton::with_range(0.0, 10000.0, 50.0);
    w_spin.set_value(min_width as f64);
    let h_lbl = gtk4::Label::new(Some("Height:"));
    h_lbl.set_halign(gtk4::Align::Start);
    let h_spin = gtk4::SpinButton::with_range(0.0, 10000.0, 50.0);
    h_spin.set_value(min_height as f64);
    hbox.append(&w_lbl);
    hbox.append(&w_spin);
    hbox.append(&h_lbl);
    hbox.append(&h_spin);
    vbox.append(&hbox);

    (w_spin, h_spin)
}

/// Script source: either inline text or a file path.
struct ScriptEditor {
    mode_file: gtk4::CheckButton,
    _mode_inline: gtk4::CheckButton,
    file_entry: gtk4::Entry,
    script_view: gtk4::TextView,
}

impl ScriptEditor {
    /// Get the final script command(s) based on current mode.
    /// Returns: (is_file, content)
    /// - file mode: content = "file:/path/to/script.sh"
    /// - inline mode: content = the script text
    fn get_script(&self) -> Vec<String> {
        if self.mode_file.is_active() {
            let path = self.file_entry.text().to_string();
            if path.trim().is_empty() {
                return vec![];
            }
            vec![format!("file:{}", path)]
        } else {
            let buf = self.script_view.buffer();
            let text = buf.text(&buf.start_iter(), &buf.end_iter(), false).to_string();
            if text.trim().is_empty() {
                return vec![];
            }
            vec![text]
        }
    }
}

/// Build the startup script editor with two modes: file or inline.
fn add_script_editor(
    vbox: &gtk4::Box,
    dialog: &gtk4::Window,
    existing_commands: &[String],
) -> ScriptEditor {
    let label = gtk4::Label::new(Some("Startup script:"));
    label.set_halign(gtk4::Align::Start);
    vbox.append(&label);

    // Radio buttons for mode
    let mode_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 16);
    let mode_inline = gtk4::CheckButton::with_label("Inline script");
    let mode_file = gtk4::CheckButton::with_label("Script file");
    mode_file.set_group(Some(&mode_inline));
    mode_box.append(&mode_inline);
    mode_box.append(&mode_file);
    vbox.append(&mode_box);

    // Stack for switching between file and inline views
    let stack = gtk4::Stack::new();

    // -- File mode --
    let file_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    let file_entry = gtk4::Entry::new();
    file_entry.set_placeholder_text(Some("script.sh"));
    file_entry.set_hexpand(true);
    file_entry.set_max_width_chars(40);
    file_box.append(&file_entry);

    let browse_btn = gtk4::Button::new();
    browse_btn.set_icon_name("document-open-symbolic");
    browse_btn.add_css_class("flat");
    browse_btn.set_tooltip_text(Some("Browse…"));
    let fe = file_entry.clone();
    let d = dialog.clone();
    browse_btn.connect_clicked(move |_| {
        let file_dialog = gtk4::FileDialog::builder()
            .title("Select Script File")
            .modal(true)
            .build();
        let filter = gtk4::FileFilter::new();
        filter.set_name(Some("Scripts"));
        filter.add_pattern("*.sh");
        filter.add_pattern("*.bash");
        filter.add_pattern("*.py");
        filter.add_pattern("*.js");
        let all = gtk4::FileFilter::new();
        all.set_name(Some("All files"));
        all.add_pattern("*");
        let filters = gtk4::gio::ListStore::new::<gtk4::FileFilter>();
        filters.append(&filter);
        filters.append(&all);
        file_dialog.set_filters(Some(&filters));

        let fe2 = fe.clone();
        file_dialog.open(Some(&d), gtk4::gio::Cancellable::NONE, move |result| {
            if let Ok(file) = result {
                if let Some(path) = file.path() {
                    fe2.set_text(&path.to_string_lossy());
                }
            }
        });
    });
    file_box.append(&browse_btn);
    stack.add_named(&file_box, Some("file"));

    // -- Inline mode --
    let script_view = gtk4::TextView::new();
    script_view.set_monospace(true);
    script_view.set_wrap_mode(gtk4::WrapMode::Word);
    script_view.set_left_margin(8);
    script_view.set_top_margin(4);

    let scroll = gtk4::ScrolledWindow::new();
    scroll.set_child(Some(&script_view));
    scroll.set_min_content_height(120);
    scroll.set_vexpand(true);
    stack.add_named(&scroll, Some("inline"));

    vbox.append(&stack);

    // Detect existing mode and populate
    let existing = existing_commands.join("\n");
    if existing.starts_with("file:") {
        mode_file.set_active(true);
        file_entry.set_text(existing.trim_start_matches("file:"));
        stack.set_visible_child_name("file");
    } else {
        mode_inline.set_active(true);
        let text = if existing.is_empty() {
            "echo \"Hello World\"".to_string()
        } else {
            existing
        };
        script_view.buffer().set_text(&text);
        stack.set_visible_child_name("inline");
    }

    // Switch stack on radio change
    let s = stack.clone();
    mode_inline.connect_toggled(move |btn| {
        if btn.is_active() {
            s.set_visible_child_name("inline");
        }
    });
    let s = stack.clone();
    mode_file.connect_toggled(move |btn| {
        if btn.is_active() {
            s.set_visible_child_name("file");
        }
    });

    ScriptEditor {
        mode_file,
        _mode_inline: mode_inline,
        file_entry,
        script_view,
    }
}

/// Detect available interpreters on the system.
fn detect_interpreters() -> Vec<String> {
    let candidates = [
        "/bin/bash", "/bin/sh", "/bin/zsh", "/bin/fish",
        "/usr/bin/bash", "/usr/bin/zsh", "/usr/bin/fish",
        "/usr/bin/python3", "/usr/bin/python", "/usr/bin/node",
    ];
    candidates.iter()
        .filter(|p| std::path::Path::new(p).exists())
        .map(|p| p.to_string())
        .collect()
}

fn show_terminal_config(
    parent: &impl IsA<gtk4::Window>,
    panel_name: &str,
    cwd: Option<&str>,
    ssh: Option<&SshConfig>,
    startup_commands: &[String],
    before_close: Option<&str>,
    min_width: u32,
    min_height: u32,
    saved_ssh: Rc<RefCell<Vec<NamedSshConfig>>>,
    on_done: impl Fn(String, PanelType, Option<String>, Option<SshConfig>, Vec<String>, Option<String>, u32, u32) + 'static,
) {
    let dialog = make_dialog(parent, "Terminal Configuration");
    let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
    vbox.set_margin_top(16);
    vbox.set_margin_bottom(16);
    vbox.set_margin_start(16);
    vbox.set_margin_end(16);

    let name_entry = add_field(&vbox, "Name:", panel_name, "Terminal");

    // Working directory
    let cwd_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    let cwd_label = gtk4::Label::new(Some("Working dir:"));
    cwd_label.set_width_chars(15);
    cwd_label.set_halign(gtk4::Align::Start);
    let cwd_entry = gtk4::Entry::new();
    cwd_entry.set_text(cwd.unwrap_or(""));
    cwd_entry.set_placeholder_text(Some("(default)"));
    cwd_entry.set_hexpand(true);
    let cwd_browse = gtk4::Button::new();
    cwd_browse.set_icon_name("folder-open-symbolic");
    cwd_browse.add_css_class("flat");
    cwd_browse.set_tooltip_text(Some("Browse..."));
    let ce = cwd_entry.clone();
    let d_cwd = dialog.clone();
    cwd_browse.connect_clicked(move |_| {
        let fd = gtk4::FileDialog::builder().title("Select Working Directory").modal(true).build();
        let ce2 = ce.clone();
        fd.select_folder(Some(&d_cwd), gtk4::gio::Cancellable::NONE, move |r| {
            if let Ok(f) = r { if let Some(p) = f.path() { ce2.set_text(&p.to_string_lossy()); } }
        });
    });
    cwd_box.append(&cwd_label);
    cwd_box.append(&cwd_entry);
    cwd_box.append(&cwd_browse);
    vbox.append(&cwd_box);

    // ── SSH connection (optional) ────────────────────────────────────
    let ssh_sep = gtk4::Separator::new(gtk4::Orientation::Horizontal);
    ssh_sep.set_margin_top(4);
    ssh_sep.set_margin_bottom(2);
    vbox.append(&ssh_sep);

    let ssh_enabled = ssh.is_some();
    let ssh_check = gtk4::CheckButton::with_label("SSH connection");
    ssh_check.set_active(ssh_enabled);
    vbox.append(&ssh_check);

    let ssh_container = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
    let ssh_host_entry = add_field(&ssh_container, "Host:", ssh.map(|s| s.host.as_str()).unwrap_or(""), "hostname or IP");
    let ssh_port_entry = add_field(&ssh_container, "Port:", &ssh.map(|s| s.port).unwrap_or(22).to_string(), "22");
    let ssh_user_entry = add_field(&ssh_container, "User:", ssh.and_then(|s| s.user.as_deref()).unwrap_or(""), "username");

    // Password field
    let ssh_pw_hbox = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    let ssh_pw_lbl = gtk4::Label::new(Some("Password:"));
    ssh_pw_lbl.set_width_chars(15);
    ssh_pw_lbl.set_halign(gtk4::Align::Start);
    let ssh_pw_entry = gtk4::PasswordEntry::new();
    ssh_pw_entry.set_show_peek_icon(true);
    ssh_pw_entry.set_hexpand(true);
    if let Some(pw) = ssh.and_then(|s| s.password.as_deref()) {
        ssh_pw_entry.set_text(pw);
    }
    ssh_pw_entry.set_placeholder_text(Some("(key auth if empty)"));
    ssh_pw_hbox.append(&ssh_pw_lbl);
    ssh_pw_hbox.append(&ssh_pw_entry);
    ssh_container.append(&ssh_pw_hbox);

    let ssh_id_entry = add_field(&ssh_container, "Identity file:", ssh.and_then(|s| s.identity_file.as_deref()).unwrap_or(""), "~/.ssh/id_rsa");
    let ssh_tmux_entry = add_field(&ssh_container, "Tmux session:", ssh.and_then(|s| s.tmux_session.as_deref()).unwrap_or(""), "(optional)");

    // Remote working directory with browse button
    let remote_cwd_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    let remote_cwd_label = gtk4::Label::new(Some("Remote dir:"));
    remote_cwd_label.set_width_chars(15);
    remote_cwd_label.set_halign(gtk4::Align::Start);
    let remote_cwd_entry = gtk4::Entry::new();
    remote_cwd_entry.set_placeholder_text(Some("/home/user (default: home)"));
    remote_cwd_entry.set_hexpand(true);
    // Pre-fill from cwd if it looks like a remote path
    if let Some(c) = cwd {
        if ssh.is_some() {
            remote_cwd_entry.set_text(c);
        }
    }
    let remote_browse_btn = gtk4::Button::from_icon_name("folder-open-symbolic");
    remote_browse_btn.add_css_class("flat");
    remote_browse_btn.set_tooltip_text(Some("Browse remote directories"));
    remote_browse_btn.set_sensitive(ssh_enabled);
    remote_cwd_row.append(&remote_cwd_label);
    remote_cwd_row.append(&remote_cwd_entry);
    remote_cwd_row.append(&remote_browse_btn);
    ssh_container.append(&remote_cwd_row);

    // Enable browse when host is filled
    {
        let btn = remote_browse_btn.clone();
        let host = ssh_host_entry.clone();
        ssh_host_entry.connect_changed(move |_| {
            btn.set_sensitive(!host.text().is_empty());
        });
    }

    // Browse remote dirs
    {
        let host_e = ssh_host_entry.clone();
        let user_e = ssh_user_entry.clone();
        let pass_e = ssh_pw_entry.clone();
        let key_e = ssh_id_entry.clone();
        let port_e = ssh_port_entry.clone();
        let path_e = remote_cwd_entry.clone();
        remote_browse_btn.connect_clicked(move |btn| {
            let host = host_e.text().to_string();
            let user = user_e.text().to_string();
            let user = if user.is_empty() { "root".to_string() } else { user };
            let password = pass_e.text().to_string();
            let key = key_e.text().to_string();
            let port = port_e.text().to_string();
            let current = path_e.text().to_string();
            let start = if current.is_empty() { "/".to_string() } else { current };

            let pe = path_e.clone();
            if let Some(win) = btn.root().and_then(|r| r.downcast::<gtk4::Window>().ok()) {
                show_remote_browse_dialog(&win, &host, &user, &password, &key, &port, &start, move |selected| {
                    pe.set_text(&selected);
                });
            }
        });
    }

    let ssh_warn = gtk4::Label::new(Some("Password stored in plain text in workspace file."));
    ssh_warn.add_css_class("dim-label");
    ssh_warn.add_css_class("caption");
    ssh_warn.set_halign(gtk4::Align::Start);
    ssh_container.append(&ssh_warn);

    // Save/Load SSH config buttons
    add_ssh_save_load_buttons(
        &ssh_container, &saved_ssh,
        &ssh_host_entry, &ssh_port_entry, &ssh_user_entry,
        &ssh_pw_entry, &ssh_id_entry,
    );

    ssh_container.set_sensitive(ssh_enabled);
    vbox.append(&ssh_container);
    {
        let sc = ssh_container.clone();
        ssh_check.connect_toggled(move |btn| { sc.set_sensitive(btn.is_active()); });
    }

    let ssh_sep2 = gtk4::Separator::new(gtk4::Orientation::Horizontal);
    ssh_sep2.set_margin_top(4);
    ssh_sep2.set_margin_bottom(2);
    vbox.append(&ssh_sep2);

    // Interpreter selector
    let interp_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    let interp_label = gtk4::Label::new(Some("Interpreter:"));
    interp_label.set_width_chars(15);
    interp_label.set_halign(gtk4::Align::Start);
    interp_box.append(&interp_label);

    let interpreters = detect_interpreters();
    let interp_dropdown = gtk4::DropDown::from_strings(
        &interpreters.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
    );
    let default_idx = interpreters.iter().position(|s| s.contains("bash")).unwrap_or(0);
    interp_dropdown.set_selected(default_idx as u32);
    interp_dropdown.set_hexpand(true);
    interp_box.append(&interp_dropdown);
    vbox.append(&interp_box);

    // Detect current interpreter from existing shebang
    let existing = startup_commands.join("\n");
    if let Some(shebang) = existing.lines().next() {
        if shebang.starts_with("#!") {
            let interp = shebang.trim_start_matches("#!").trim();
            if let Some(idx) = interpreters.iter().position(|s| s == interp) {
                interp_dropdown.set_selected(idx as u32);
            }
        }
    }

    // ── Startup script (with enable checkbox) ────────────────────────────
    let startup_enabled = !startup_commands.is_empty();
    let startup_check = gtk4::CheckButton::with_label("Startup script");
    startup_check.set_active(startup_enabled);
    vbox.append(&startup_check);

    let startup_container = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
    let script_editor = add_script_editor(&startup_container, &dialog, startup_commands);
    startup_container.set_sensitive(startup_enabled);
    vbox.append(&startup_container);

    {
        let sc = startup_container.clone();
        startup_check.connect_toggled(move |btn| {
            sc.set_sensitive(btn.is_active());
        });
    }

    // ── Before close script (with enable checkbox) ───────────────────────
    let sep = gtk4::Separator::new(gtk4::Orientation::Horizontal);
    sep.set_margin_top(8);
    sep.set_margin_bottom(4);
    vbox.append(&sep);

    let close_enabled = before_close.is_some() && !before_close.unwrap_or("").trim().is_empty();
    let close_check = gtk4::CheckButton::with_label("Before close script");
    close_check.set_active(close_enabled);
    vbox.append(&close_check);

    let close_container = gtk4::Box::new(gtk4::Orientation::Vertical, 4);

    // Inline/file radio for before_close
    let close_mode_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 16);
    let close_mode_inline = gtk4::CheckButton::with_label("Inline");
    let close_mode_file = gtk4::CheckButton::with_label("Script file");
    close_mode_file.set_group(Some(&close_mode_inline));
    close_mode_box.append(&close_mode_inline);
    close_mode_box.append(&close_mode_file);
    close_container.append(&close_mode_box);

    let close_stack = gtk4::Stack::new();

    // Inline
    let close_view = gtk4::TextView::new();
    close_view.set_monospace(true);
    close_view.set_wrap_mode(gtk4::WrapMode::Word);
    close_view.set_left_margin(8);
    close_view.set_top_margin(4);
    let close_scroll = gtk4::ScrolledWindow::new();
    close_scroll.set_child(Some(&close_view));
    close_scroll.set_min_content_height(60);
    close_stack.add_named(&close_scroll, Some("inline"));

    // File
    let close_file_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    let close_file_entry = gtk4::Entry::new();
    close_file_entry.set_placeholder_text(Some("cleanup.sh"));
    close_file_entry.set_hexpand(true);
    close_file_entry.set_max_width_chars(40);
    close_file_box.append(&close_file_entry);
    let close_browse = gtk4::Button::new();
    close_browse.set_icon_name("document-open-symbolic");
    close_browse.add_css_class("flat");
    let cfe = close_file_entry.clone();
    let d2 = dialog.clone();
    close_browse.connect_clicked(move |_| {
        let fd = gtk4::FileDialog::builder().title("Select Script").modal(true).build();
        let cfe2 = cfe.clone();
        fd.open(Some(&d2), gtk4::gio::Cancellable::NONE, move |r| {
            if let Ok(f) = r { if let Some(p) = f.path() { cfe2.set_text(&p.to_string_lossy()); } }
        });
    });
    close_file_box.append(&close_browse);
    close_stack.add_named(&close_file_box, Some("file"));

    // Populate from existing
    let bc = before_close.unwrap_or("");
    if bc.starts_with("file:") {
        close_mode_file.set_active(true);
        close_file_entry.set_text(bc.trim_start_matches("file:"));
        close_stack.set_visible_child_name("file");
    } else {
        close_mode_inline.set_active(true);
        close_view.buffer().set_text(bc);
        close_stack.set_visible_child_name("inline");
    }

    let cs = close_stack.clone();
    close_mode_inline.connect_toggled(move |b| { if b.is_active() { cs.set_visible_child_name("inline"); } });
    let cs = close_stack.clone();
    close_mode_file.connect_toggled(move |b| { if b.is_active() { cs.set_visible_child_name("file"); } });

    close_container.append(&close_stack);
    close_container.set_sensitive(close_enabled);
    vbox.append(&close_container);

    {
        let cc = close_container.clone();
        close_check.connect_toggled(move |btn| {
            cc.set_sensitive(btn.is_active());
        });
    }

    let (mw_spin, mh_spin) = add_min_size_fields(&vbox, min_width, min_height);

    let ne = name_entry.clone();
    let ce = cwd_entry.clone();
    let id = interp_dropdown.clone();
    let interps = interpreters.clone();
    let cv = close_view.clone();
    let cmf = close_mode_file.clone();
    let cfe = close_file_entry.clone();
    let sc = startup_check.clone();
    let cc = close_check.clone();
    let ssh_chk = ssh_check.clone();
    let ssh_h = ssh_host_entry.clone();
    let ssh_p = ssh_port_entry.clone();
    let ssh_u = ssh_user_entry.clone();
    let ssh_pw = ssh_pw_entry.clone();
    let ssh_id = ssh_id_entry.clone();
    let ssh_tmux = ssh_tmux_entry.clone();
    let rcwd = remote_cwd_entry.clone();
    add_buttons(&vbox, &dialog, move || {
        let name = ne.text().to_string();
        let cwd_text = ce.text().to_string();
        let remote_cwd_text = rcwd.text().to_string();
        // Use remote dir as cwd when SSH is active
        let cwd = if ssh_chk.is_active() && !remote_cwd_text.trim().is_empty() {
            Some(remote_cwd_text)
        } else if cwd_text.trim().is_empty() {
            None
        } else {
            Some(cwd_text)
        };
        let selected = id.selected() as usize;
        let interpreter = interps.get(selected).cloned().unwrap_or_else(|| "/bin/bash".to_string());
        let mw = mw_spin.value() as u32;
        let mh = mh_spin.value() as u32;

        // SSH config (only if enabled)
        let ssh_config = if ssh_chk.is_active() {
            let host = ssh_h.text().to_string();
            if host.trim().is_empty() {
                None
            } else {
                Some(SshConfig {
                    host,
                    port: ssh_p.text().parse().unwrap_or(22),
                    user: if ssh_u.text().is_empty() { None } else { Some(ssh_u.text().to_string()) },
                    password: if ssh_pw.text().is_empty() { None } else { Some(ssh_pw.text().to_string()) },
                    identity_file: if ssh_id.text().is_empty() { None } else { Some(ssh_id.text().to_string()) },
                    tmux_session: if ssh_tmux.text().is_empty() { None } else { Some(ssh_tmux.text().to_string()) },
                })
            }
        } else {
            None
        };

        // Before close (only if enabled)
        let before_close = if cc.is_active() {
            if cmf.is_active() {
                let path = cfe.text().to_string();
                if path.trim().is_empty() { None } else { Some(format!("file:{}", path)) }
            } else {
                let close_buf = cv.buffer();
                let close_text = close_buf.text(&close_buf.start_iter(), &close_buf.end_iter(), false).to_string();
                if close_text.trim().is_empty() { None } else { Some(close_text) }
            }
        } else {
            None
        };

        // Startup script (only if enabled)
        if !sc.is_active() {
            on_done(name, PanelType::Terminal, cwd, ssh_config, vec![], before_close, mw, mh);
            return;
        }

        let cmds = script_editor.get_script();
        if cmds.is_empty() {
            on_done(name, PanelType::Terminal, cwd, ssh_config, vec![], before_close, mw, mh);
            return;
        }

        // For file mode, prepend interpreter info
        let first = &cmds[0];
        if first.starts_with("file:") {
            let path = first.trim_start_matches("file:");
            on_done(name, PanelType::Terminal, cwd, ssh_config, vec![format!("file:{}:{}", interpreter, path)], before_close, mw, mh);
        } else {
            let script = if first.starts_with("#!") {
                let rest = first.lines().skip(1).collect::<Vec<_>>().join("\n");
                format!("#!{}\n{}", interpreter, rest)
            } else {
                format!("#!{}\n{}", interpreter, first.clone())
            };
            on_done(name, PanelType::Terminal, cwd, ssh_config, vec![script], before_close, mw, mh);
        }
    });

    dialog.set_child(Some(&vbox));
    dialog.present();
}

fn show_markdown_config(
    parent: &impl IsA<gtk4::Window>,
    panel_name: &str,
    file: &str,
    min_width: u32,
    min_height: u32,
    on_done: impl Fn(String, PanelType, Option<String>, Option<SshConfig>, Vec<String>, Option<String>, u32, u32) + 'static,
) {
    let dialog = make_dialog(parent, "Markdown Configuration");
    let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
    vbox.set_margin_top(16);
    vbox.set_margin_bottom(16);
    vbox.set_margin_start(16);
    vbox.set_margin_end(16);

    let name_entry = add_field(&vbox, "Name:", panel_name, "Markdown");
    let file_entry = add_field(&vbox, "File:", file, "path/to/file.md");

    let browse_btn = gtk4::Button::with_label("Browse...");
    browse_btn.add_css_class("flat");
    browse_btn.set_halign(gtk4::Align::Start);
    let fe = file_entry.clone();
    let d = dialog.clone();
    browse_btn.connect_clicked(move |_| {
        let file_dialog = gtk4::FileDialog::builder()
            .title("Select Markdown File")
            .modal(true)
            .build();
        let filter = gtk4::FileFilter::new();
        filter.set_name(Some("Markdown files"));
        filter.add_pattern("*.md");
        filter.add_pattern("*.markdown");
        let filters = gtk4::gio::ListStore::new::<gtk4::FileFilter>();
        filters.append(&filter);
        file_dialog.set_filters(Some(&filters));

        let fe2 = fe.clone();
        file_dialog.open(Some(&d), gtk4::gio::Cancellable::NONE, move |result| {
            if let Ok(file) = result {
                if let Some(path) = file.path() {
                    fe2.set_text(&path.to_string_lossy());
                }
            }
        });
    });
    vbox.append(&browse_btn);

    let (mw_spin, mh_spin) = add_min_size_fields(&vbox, min_width, min_height);

    add_buttons(&vbox, &dialog, move || {
        let name = name_entry.text().to_string();
        let file = file_entry.text().to_string();
        on_done(name, PanelType::Markdown { file }, None, None, vec![], None, mw_spin.value() as u32, mh_spin.value() as u32);
    });

    dialog.set_child(Some(&vbox));
    dialog.present();
}

fn show_browser_config(
    parent: &impl IsA<gtk4::Window>,
    panel_name: &str,
    url: &str,
    min_width: u32,
    min_height: u32,
    on_done: impl Fn(String, PanelType, Option<String>, Option<SshConfig>, Vec<String>, Option<String>, u32, u32) + 'static,
) {
    let dialog = make_dialog(parent, "Browser Configuration");
    let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
    vbox.set_margin_top(16);
    vbox.set_margin_bottom(16);
    vbox.set_margin_start(16);
    vbox.set_margin_end(16);

    let name_entry = add_field(&vbox, "Name:", panel_name, "Browser");
    let url_entry = add_field(&vbox, "URL:", url, "https://example.com");

    let (mw_spin, mh_spin) = add_min_size_fields(&vbox, min_width, min_height);

    add_buttons(&vbox, &dialog, move || {
        let name = name_entry.text().to_string();
        let url = url_entry.text().to_string();
        on_done(name, PanelType::Browser { url }, None, None, vec![], None, mw_spin.value() as u32, mh_spin.value() as u32);
    });

    dialog.set_child(Some(&vbox));
    dialog.present();
}

fn show_code_editor_config(
    parent: &impl IsA<gtk4::Window>,
    panel_name: &str,
    root_dir: &str,
    existing_ssh: Option<&pax_core::workspace::SshConfig>,
    existing_remote_path: Option<&str>,
    min_width: u32,
    min_height: u32,
    saved_ssh: Rc<RefCell<Vec<NamedSshConfig>>>,
    on_done: impl Fn(String, PanelType, Option<String>, Option<SshConfig>, Vec<String>, Option<String>, u32, u32) + 'static,
) {
    let dialog = make_dialog(parent, "Code Editor Configuration");
    let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
    vbox.set_margin_top(16);
    vbox.set_margin_bottom(16);
    vbox.set_margin_start(16);
    vbox.set_margin_end(16);

    let name_entry = add_field(&vbox, "Name:", panel_name, "Code Editor");

    // Project dir with inline browse button
    let dir_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    let dir_label = gtk4::Label::new(Some("Project dir:"));
    dir_label.set_width_chars(15);
    dir_label.set_halign(gtk4::Align::Start);
    let dir_entry = gtk4::Entry::new();
    dir_entry.set_text(root_dir);
    dir_entry.set_placeholder_text(Some("/path/to/project"));
    dir_entry.set_hexpand(true);
    let browse_btn = gtk4::Button::from_icon_name("folder-open-symbolic");
    browse_btn.add_css_class("flat");
    browse_btn.set_tooltip_text(Some("Browse..."));
    dir_row.append(&dir_label);
    dir_row.append(&dir_entry);
    dir_row.append(&browse_btn);
    vbox.append(&dir_row);

    {
        let de = dir_entry.clone();
        let d = dialog.clone();
        browse_btn.connect_clicked(move |_| {
            let file_dialog = gtk4::FileDialog::builder()
                .title("Select Project Directory")
                .modal(true)
                .build();

            let de2 = de.clone();
            file_dialog.select_folder(Some(&d), gtk4::gio::Cancellable::NONE, move |result| {
                if let Ok(file) = result {
                    if let Some(path) = file.path() {
                        de2.set_text(&path.to_string_lossy());
                    }
                }
            });
        });
    }

    // ── SSH / Remote section ──
    vbox.append(&gtk4::Separator::new(gtk4::Orientation::Horizontal));

    let has_ssh = existing_ssh.is_some();
    let ssh_toggle = gtk4::Switch::new();
    ssh_toggle.set_active(has_ssh);
    ssh_toggle.set_valign(gtk4::Align::Center);

    let ssh_header_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    ssh_header_row.set_margin_top(8);
    ssh_header_row.set_margin_bottom(4);
    let ssh_header = gtk4::Label::new(Some("Remote (SSH)"));
    ssh_header.add_css_class("heading");
    ssh_header.set_halign(gtk4::Align::Start);
    ssh_header.set_hexpand(true);
    ssh_header_row.append(&ssh_header);
    ssh_header_row.append(&ssh_toggle);
    vbox.append(&ssh_header_row);

    let ssh_fields = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
    ssh_fields.set_margin_start(0);
    ssh_fields.set_visible(has_ssh);

    let ssh_hint = gtk4::Label::new(Some("Edit remote files via SSH. Requires: ssh + sshpass (for password auth)."));
    ssh_hint.add_css_class("dim-label");
    ssh_hint.add_css_class("caption");
    ssh_hint.set_halign(gtk4::Align::Start);
    ssh_hint.set_margin_bottom(4);
    ssh_fields.append(&ssh_hint);

    let ssh_host_entry = add_field(&ssh_fields, "SSH Host:",
        existing_ssh.map(|s| s.host.as_str()).unwrap_or(""), "server.example.com");
    let ssh_user_entry = add_field(&ssh_fields, "User:",
        existing_ssh.and_then(|s| s.user.as_deref()).unwrap_or(""), "root");
    let ssh_pass_entry = {
        let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        let label = gtk4::Label::new(Some("Password:"));
        label.set_width_chars(15);
        label.set_halign(gtk4::Align::Start);
        let entry = gtk4::PasswordEntry::new();
        entry.set_show_peek_icon(true);
        entry.set_hexpand(true);
        if let Some(p) = existing_ssh.and_then(|s| s.password.as_deref()) {
            entry.set_text(p);
        }
        row.append(&label);
        row.append(&entry);
        ssh_fields.append(&row);
        entry
    };
    let ssh_key_entry = add_field(&ssh_fields, "Identity file:",
        existing_ssh.and_then(|s| s.identity_file.as_deref()).unwrap_or(""), "~/.ssh/id_rsa");
    let ssh_port_entry = add_field(&ssh_fields, "Port:",
        &existing_ssh.map(|s| s.port.to_string()).unwrap_or_else(|| "22".to_string()), "22");

    // Remote path with browse button
    let remote_path_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    let remote_path_label = gtk4::Label::new(Some("Remote path:"));
    remote_path_label.set_width_chars(15);
    remote_path_label.set_halign(gtk4::Align::Start);
    let remote_path_entry = gtk4::Entry::new();
    remote_path_entry.set_placeholder_text(Some("/home/user/project"));
    if let Some(rp) = existing_remote_path {
        remote_path_entry.set_text(rp);
    }
    remote_path_entry.set_hexpand(true);
    let remote_browse_btn = gtk4::Button::from_icon_name("folder-open-symbolic");
    remote_browse_btn.add_css_class("flat");
    remote_browse_btn.set_tooltip_text(Some("Browse remote directories"));
    remote_browse_btn.set_sensitive(has_ssh);
    remote_path_row.append(&remote_path_label);
    remote_path_row.append(&remote_path_entry);
    remote_path_row.append(&remote_browse_btn);
    ssh_fields.append(&remote_path_row);

    // Enable browse button when host is filled
    {
        let btn = remote_browse_btn.clone();
        let host = ssh_host_entry.clone();
        ssh_host_entry.connect_changed(move |_| {
            btn.set_sensitive(!host.text().is_empty());
        });
    }

    // Browse remote directories via SSH
    {
        let host_e = ssh_host_entry.clone();
        let user_e = ssh_user_entry.clone();
        let pass_e = ssh_pass_entry.clone();
        let key_e = ssh_key_entry.clone();
        let port_e = ssh_port_entry.clone();
        let path_e = remote_path_entry.clone();
        remote_browse_btn.connect_clicked(move |btn| {
            let host = host_e.text().to_string();
            let user = user_e.text().to_string();
            let user = if user.is_empty() { "root".to_string() } else { user };
            let password = pass_e.text().to_string();
            let key = key_e.text().to_string();
            let port = port_e.text().to_string();
            let current_path = path_e.text().to_string();
            let start_path = if current_path.is_empty() { "/".to_string() } else { current_path };

            let path_entry = path_e.clone();
            if let Some(win) = btn.root().and_then(|r| r.downcast::<gtk4::Window>().ok()) {
                show_remote_browse_dialog(&win, &host, &user, &password, &key, &port, &start_path, move |selected| {
                    path_entry.set_text(&selected);
                });
            }
        });
    }

    // Save/Load SSH config buttons
    add_ssh_save_load_buttons(
        &ssh_fields, &saved_ssh,
        &ssh_host_entry, &ssh_port_entry, &ssh_user_entry,
        &ssh_pass_entry, &ssh_key_entry,
    );

    vbox.append(&ssh_fields);

    // Toggle SSH fields visibility
    {
        let fields = ssh_fields.clone();
        ssh_toggle.connect_state_set(move |_, active| {
            fields.set_visible(active);
            gtk4::glib::Propagation::Proceed
        });
    }

    vbox.append(&gtk4::Separator::new(gtk4::Orientation::Horizontal));

    let (mw_spin, mh_spin) = add_min_size_fields(&vbox, min_width, min_height);

    add_buttons(&vbox, &dialog, move || {
        let name = name_entry.text().to_string();
        let root_dir = dir_entry.text().to_string();
        let root_dir = if root_dir.is_empty() { ".".to_string() } else { root_dir };

        // Build SSH config only if toggle is enabled AND host is set
        let ssh_enabled = ssh_toggle.is_active();
        let host_text = ssh_host_entry.text().to_string();
        let ssh = if ssh_enabled && !host_text.trim().is_empty() {
            let port_text = ssh_port_entry.text().to_string();
            let port: u16 = port_text.trim().parse().unwrap_or(22);
            let user = {
                let u = ssh_user_entry.text().to_string();
                if u.trim().is_empty() { None } else { Some(u) }
            };
            let password = {
                let p = ssh_pass_entry.text().to_string();
                if p.is_empty() { None } else { Some(p) }
            };
            let identity = {
                let k = ssh_key_entry.text().to_string();
                if k.trim().is_empty() { None } else { Some(k) }
            };
            Some(pax_core::workspace::SshConfig {
                host: host_text,
                port,
                user,
                password,
                identity_file: identity,
                tmux_session: None,
            })
        } else {
            None
        };

        let remote_path = {
            let rp = remote_path_entry.text().to_string();
            if rp.trim().is_empty() { None } else { Some(rp) }
        };

        on_done(name, PanelType::CodeEditor { root_dir, ssh, remote_path }, None, None, vec![], None, mw_spin.value() as u32, mh_spin.value() as u32);
    });

    dialog.set_child(Some(&vbox));
    dialog.present();
}

/// Show a dialog to browse remote directories via SSH.
/// Add Save/Load SSH config buttons to an SSH config section.
fn add_ssh_save_load_buttons(
    container: &gtk4::Box,
    saved_ssh: &Rc<RefCell<Vec<NamedSshConfig>>>,
    host_entry: &gtk4::Entry,
    port_entry: &gtk4::Entry,
    user_entry: &gtk4::Entry,
    pass_entry: &gtk4::PasswordEntry,
    key_entry: &gtk4::Entry,
) {
    let btn_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    btn_row.set_margin_top(4);
    btn_row.set_halign(gtk4::Align::End);

    // Load saved config
    let load_btn = gtk4::Button::new();
    load_btn.set_icon_name("document-open-symbolic");
    load_btn.set_label("Load");
    load_btn.add_css_class("flat");
    load_btn.set_tooltip_text(Some("Load a saved SSH configuration"));
    {
        let saved = saved_ssh.clone();
        let he = host_entry.clone();
        let pe = port_entry.clone();
        let ue = user_entry.clone();
        let pwe = pass_entry.clone();
        let ke = key_entry.clone();
        load_btn.connect_clicked(move |btn| {
            let configs = saved.borrow().clone();
            if configs.is_empty() { return; }

            let dialog = gtk4::Window::builder()
                .title("Saved SSH Configs")
                .modal(true)
                .default_width(400)
                .default_height(300)
                .build();
            if let Some(win) = btn.root().and_then(|r| r.downcast::<gtk4::Window>().ok()) {
                dialog.set_transient_for(Some(&win));
            }

            let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
            let list_box = gtk4::ListBox::new();
            list_box.set_selection_mode(gtk4::SelectionMode::Single);
            let scroll = gtk4::ScrolledWindow::new();
            scroll.set_child(Some(&list_box));
            scroll.set_vexpand(true);

            let saved_rc = saved.clone();
            let populate = {
                let lb = list_box.clone();
                let saved = saved.clone();
                Rc::new(move || {
                    while let Some(child) = lb.first_child() { lb.remove(&child); }
                    for cfg in saved.borrow().iter() {
                        let row_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
                        row_box.set_margin_start(8);
                        row_box.set_margin_end(8);
                        row_box.set_margin_top(6);
                        row_box.set_margin_bottom(6);

                        let info = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
                        info.set_hexpand(true);
                        let name_label = gtk4::Label::new(Some(&cfg.name));
                        name_label.add_css_class("heading");
                        name_label.set_halign(gtk4::Align::Start);
                        info.append(&name_label);

                        let details = format!("{}@{}:{} {}",
                            cfg.config.user.as_deref().unwrap_or("root"),
                            cfg.config.host,
                            cfg.config.port,
                            if cfg.config.identity_file.is_some() { "🔑" } else if cfg.config.password.is_some() { "🔒" } else { "" }
                        );
                        let detail_label = gtk4::Label::new(Some(&details));
                        detail_label.add_css_class("dim-label");
                        detail_label.add_css_class("caption");
                        detail_label.set_halign(gtk4::Align::Start);
                        info.append(&detail_label);
                        row_box.append(&info);

                        // Delete button
                        let del_btn = gtk4::Button::from_icon_name("user-trash-symbolic");
                        del_btn.add_css_class("flat");
                        del_btn.set_tooltip_text(Some("Delete this config"));
                        let name = cfg.name.clone();
                        let saved_del = saved.clone();
                        del_btn.connect_clicked(move |_| {
                            saved_del.borrow_mut().retain(|c| c.name != name);
                        });
                        row_box.append(&del_btn);

                        let row = gtk4::ListBoxRow::new();
                        row.set_child(Some(&row_box));
                        row.set_widget_name(&cfg.name);
                        lb.append(&row);
                    }
                })
            };
            populate();

            // Refresh list when items are deleted
            let populate_ref = populate.clone();
            let saved_for_poll = saved.clone();
            let lb_for_poll = list_box.clone();
            let last_count = Rc::new(std::cell::Cell::new(configs.len()));
            gtk4::glib::timeout_add_local(std::time::Duration::from_millis(200), move || {
                let current = saved_for_poll.borrow().len();
                if current != last_count.get() {
                    last_count.set(current);
                    populate_ref();
                }
                if lb_for_poll.parent().is_none() {
                    return gtk4::glib::ControlFlow::Break;
                }
                gtk4::glib::ControlFlow::Continue
            });

            vbox.append(&scroll);

            // Select button
            let btn_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
            btn_row.set_halign(gtk4::Align::End);
            btn_row.set_margin_start(12);
            btn_row.set_margin_end(12);
            btn_row.set_margin_top(4);
            btn_row.set_margin_bottom(8);
            let cancel_btn = gtk4::Button::with_label("Cancel");
            let select_btn = gtk4::Button::with_label("Select");
            select_btn.add_css_class("suggested-action");
            btn_row.append(&cancel_btn);
            btn_row.append(&select_btn);
            vbox.append(&btn_row);

            {
                let d = dialog.clone();
                cancel_btn.connect_clicked(move |_| { d.close(); });
            }
            {
                let d = dialog.clone();
                let he = he.clone();
                let pe = pe.clone();
                let ue = ue.clone();
                let pwe = pwe.clone();
                let ke = ke.clone();
                let saved = saved_rc;
                list_box.connect_row_activated(move |_, row| {
                    let name = row.widget_name();
                    if let Some(cfg) = saved.borrow().iter().find(|c| c.name == name.as_str()) {
                        he.set_text(&cfg.config.host);
                        pe.set_text(&cfg.config.port.to_string());
                        ue.set_text(cfg.config.user.as_deref().unwrap_or(""));
                        pwe.set_text(cfg.config.password.as_deref().unwrap_or(""));
                        ke.set_text(cfg.config.identity_file.as_deref().unwrap_or(""));
                    }
                    d.close();
                });
                let d2 = dialog.clone();
                select_btn.connect_clicked(move |_| {
                    // Simulate activating the selected row
                    d2.close();
                });
            }

            dialog.set_child(Some(&vbox));
            dialog.present();
        });
    }
    btn_row.append(&load_btn);

    // Save current config
    let save_btn = gtk4::Button::new();
    save_btn.set_icon_name("document-save-symbolic");
    save_btn.set_label("Save");
    save_btn.add_css_class("flat");
    save_btn.set_tooltip_text(Some("Save this SSH configuration for reuse"));
    {
        let saved = saved_ssh.clone();
        let he = host_entry.clone();
        let pe = port_entry.clone();
        let ue = user_entry.clone();
        let pwe = pass_entry.clone();
        let ke = key_entry.clone();
        save_btn.connect_clicked(move |btn| {
            let host = he.text().to_string();
            if host.trim().is_empty() { return; }

            // Show name input dialog
            let dialog = gtk4::Window::builder()
                .title("Save SSH Config")
                .modal(true)
                .default_width(300)
                .default_height(80)
                .build();
            if let Some(win) = btn.root().and_then(|r| r.downcast::<gtk4::Window>().ok()) {
                dialog.set_transient_for(Some(&win));
            }
            let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
            vbox.set_margin_top(12);
            vbox.set_margin_bottom(12);
            vbox.set_margin_start(12);
            vbox.set_margin_end(12);
            let entry = gtk4::Entry::new();
            let default_name = format!("{}@{}", ue.text(), host);
            entry.set_text(&default_name);
            entry.set_placeholder_text(Some("Config name"));
            vbox.append(&entry);
            let ok_btn = gtk4::Button::with_label("Save");
            ok_btn.add_css_class("suggested-action");

            let saved_c = saved.clone();
            let d = dialog.clone();
            let port_text = pe.text().to_string();
            let user_text = ue.text().to_string();
            let pass_text = pwe.text().to_string();
            let key_text = ke.text().to_string();
            ok_btn.connect_clicked(move |_| {
                let name = entry.text().to_string();
                if name.trim().is_empty() { return; }
                let config = SshConfig {
                    host: host.clone(),
                    port: port_text.parse().unwrap_or(22),
                    user: if user_text.is_empty() { None } else { Some(user_text.clone()) },
                    password: if pass_text.is_empty() { None } else { Some(pass_text.clone()) },
                    identity_file: if key_text.is_empty() { None } else { Some(key_text.clone()) },
                    tmux_session: None,
                };
                let mut saved = saved_c.borrow_mut();
                // Replace if same name exists
                saved.retain(|c| c.name != name);
                saved.push(NamedSshConfig { name, config });
                d.close();
            });
            vbox.append(&ok_btn);
            dialog.set_child(Some(&vbox));
            dialog.present();
        });
    }
    btn_row.append(&save_btn);

    container.append(&btn_row);
}

fn show_remote_browse_dialog(
    parent: &gtk4::Window,
    host: &str, user: &str, password: &str, identity_file: &str, port: &str,
    start_path: &str,
    on_select: impl Fn(String) + 'static,
) {
    use std::rc::Rc;
    use std::cell::RefCell;

    let dialog = gtk4::Window::builder()
        .title("Browse Remote Directory")
        .transient_for(parent)
        .modal(true)
        .default_width(450)
        .default_height(400)
        .build();

    let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 0);

    // Header: up button + current path
    let header = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    header.set_margin_start(12);
    header.set_margin_end(12);
    header.set_margin_top(8);
    header.set_margin_bottom(4);

    let up_btn = gtk4::Button::from_icon_name("go-up-symbolic");
    up_btn.add_css_class("flat");
    up_btn.set_tooltip_text(Some("Go up"));
    header.append(&up_btn);

    let path_label = gtk4::Label::new(Some(start_path));
    path_label.add_css_class("heading");
    path_label.set_halign(gtk4::Align::Start);
    path_label.set_hexpand(true);
    path_label.set_ellipsize(gtk4::pango::EllipsizeMode::Start);
    header.append(&path_label);

    vbox.append(&header);
    vbox.append(&gtk4::Separator::new(gtk4::Orientation::Horizontal));

    let list_box = gtk4::ListBox::new();
    list_box.set_selection_mode(gtk4::SelectionMode::Single);
    let scroll = gtk4::ScrolledWindow::new();
    scroll.set_child(Some(&list_box));
    scroll.set_vexpand(true);
    vbox.append(&scroll);

    let status_label = gtk4::Label::new(Some("Loading..."));
    status_label.add_css_class("dim-label");
    status_label.set_margin_top(4);
    status_label.set_margin_bottom(4);
    vbox.append(&status_label);

    let btn_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    btn_row.set_halign(gtk4::Align::End);
    btn_row.set_margin_start(12);
    btn_row.set_margin_end(12);
    btn_row.set_margin_top(4);
    btn_row.set_margin_bottom(8);

    let cancel_btn = gtk4::Button::with_label("Cancel");
    let select_btn = gtk4::Button::with_label("Select");
    select_btn.add_css_class("suggested-action");
    btn_row.append(&cancel_btn);
    btn_row.append(&select_btn);
    vbox.append(&btn_row);

    dialog.set_child(Some(&vbox));

    let ssh_host = host.to_string();
    let ssh_user = user.to_string();
    let ssh_pass = password.to_string();
    let ssh_key = identity_file.to_string();
    let ssh_port = port.to_string();
    let current_path: Rc<RefCell<String>> = Rc::new(RefCell::new(start_path.to_string()));

    // List remote directories via SSH
    let list_remote_dirs = {
        let host = ssh_host.clone();
        let user = ssh_user.clone();
        let pass = ssh_pass.clone();
        let key = ssh_key.clone();
        let port = ssh_port.clone();
        move |path: &str| -> Result<Vec<String>, String> {
            let cmd_str = format!("ls -1ap '{}' 2>/dev/null | grep '/$'", path);
            let mut cmd = if !pass.is_empty() {
                let mut c = std::process::Command::new("sshpass");
                c.args(["-p", &pass, "ssh"]);
                c
            } else {
                std::process::Command::new("ssh")
            };
            cmd.args(["-o", "StrictHostKeyChecking=no", "-o", "ConnectTimeout=5"]);
            if !key.is_empty() {
                cmd.args(["-i", &key]);
            }
            cmd.args(["-p", &port, &format!("{}@{}", user, host), &cmd_str]);

            match cmd.output() {
                Ok(output) if output.status.success() => {
                    let stdout = String::from_utf8_lossy(&output.stdout);
                    let dirs: Vec<String> = stdout.lines()
                        .filter(|l| !l.is_empty() && *l != "./" && *l != "../")
                        .map(|l| l.trim_end_matches('/').to_string())
                        .collect();
                    Ok(dirs)
                }
                Ok(output) => Err(String::from_utf8_lossy(&output.stderr).trim().to_string()),
                Err(e) => Err(format!("SSH failed: {}", e)),
            }
        }
    };

    // Populate list for a given path (async — runs SSH in background thread)
    let populate = {
        let lb = list_box.clone();
        let pl = path_label.clone();
        let sl = status_label.clone();
        let cp = current_path.clone();
        let list_fn = std::sync::Arc::new(list_remote_dirs);
        move |path: &str| {
            *cp.borrow_mut() = path.to_string();
            pl.set_text(path);
            while let Some(child) = lb.first_child() {
                lb.remove(&child);
            }
            sl.set_text("Connecting...");
            sl.set_visible(true);

            let path_owned = path.to_string();
            let list_fn = list_fn.clone();
            let lb = lb.clone();
            let sl = sl.clone();
            let result_slot = std::sync::Arc::new(std::sync::Mutex::new(None::<Result<Vec<String>, String>>));

            // Run SSH in background thread
            let slot = result_slot.clone();
            std::thread::spawn(move || {
                let result = list_fn(&path_owned);
                *slot.lock().unwrap() = Some(result);
            });

            // Poll for result on main thread
            let slot = result_slot;
            gtk4::glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
                let ready = slot.lock().unwrap().is_some();
                if !ready {
                    return gtk4::glib::ControlFlow::Continue;
                }
                let result = slot.lock().unwrap().take().unwrap();
                match result {
                    Ok(dirs) => {
                        if dirs.is_empty() {
                            sl.set_text("No subdirectories");
                        } else {
                            sl.set_visible(false);
                            for dir in &dirs {
                                let row_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
                                row_box.set_margin_start(8);
                                row_box.set_margin_end(8);
                                row_box.set_margin_top(3);
                                row_box.set_margin_bottom(3);
                                let icon = gtk4::Image::from_icon_name("folder-symbolic");
                                icon.set_pixel_size(16);
                                row_box.append(&icon);
                                let label = gtk4::Label::new(Some(dir));
                                label.set_halign(gtk4::Align::Start);
                                row_box.append(&label);
                                let row = gtk4::ListBoxRow::new();
                                row.set_child(Some(&row_box));
                                row.set_widget_name(dir);
                                lb.append(&row);
                            }
                        }
                    }
                    Err(e) => {
                        sl.set_text(&format!("Error: {}", e.chars().take(100).collect::<String>()));
                    }
                }
                gtk4::glib::ControlFlow::Break
            });
        }
    };

    let populate_rc = Rc::new(populate);

    // Initial load (deferred to next idle so dialog is visible first)
    {
        let p = populate_rc.clone();
        let sp = start_path.to_string();
        gtk4::glib::idle_add_local_once(move || { p(&sp); });
    }

    // Double-click directory to navigate into it
    {
        let p = populate_rc.clone();
        let cp = current_path.clone();
        list_box.connect_row_activated(move |_, row| {
            let dir_name = row.widget_name();
            let current = cp.borrow().clone();
            let new_path = if current.ends_with('/') {
                format!("{}{}", current, dir_name)
            } else {
                format!("{}/{}", current, dir_name)
            };
            p(&new_path);
        });
    }

    // Up button
    {
        let p = populate_rc.clone();
        let cp = current_path.clone();
        up_btn.connect_clicked(move |_| {
            let current = cp.borrow().clone();
            if current != "/" {
                let parent = std::path::Path::new(&current)
                    .parent()
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_else(|| "/".to_string());
                p(&parent);
            }
        });
    }

    // Cancel
    {
        let d = dialog.clone();
        cancel_btn.connect_clicked(move |_| { d.close(); });
    }

    // Select current path
    {
        let d = dialog.clone();
        let cp = current_path;
        let on_sel = Rc::new(on_select);
        select_btn.connect_clicked(move |_| {
            let path = cp.borrow().clone();
            on_sel(path);
            d.close();
        });
    }

    dialog.present();
}
