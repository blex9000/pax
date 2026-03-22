use gtk4::prelude::*;

use tp_core::workspace::PanelType;

/// Callback: (name, panel_type, cwd, startup_commands, before_close, min_width, min_height)
pub type ConfigDoneCallback = dyn Fn(String, PanelType, Option<String>, Vec<String>, Option<String>, u32, u32) + 'static;

/// Show a configuration dialog for the given panel type.
pub fn show_panel_config_dialog(
    parent: &impl IsA<gtk4::Window>,
    panel_name: &str,
    panel_type: &PanelType,
    cwd: Option<&str>,
    startup_commands: &[String],
    before_close: Option<&str>,
    min_width: u32,
    min_height: u32,
    on_done: impl Fn(String, PanelType, Option<String>, Vec<String>, Option<String>, u32, u32) + 'static,
) {
    match panel_type {
        PanelType::Terminal => show_terminal_config(parent, panel_name, cwd, startup_commands, before_close, min_width, min_height, on_done),
        PanelType::Ssh { host, port, user, identity_file } => {
            show_ssh_config(parent, panel_name, host, *port, user.as_deref(), identity_file.as_deref(), min_width, min_height, on_done)
        }
        PanelType::RemoteTmux { host, session, user } => {
            show_tmux_config(parent, panel_name, host, session, user.as_deref(), min_width, min_height, on_done)
        }
        PanelType::Markdown { file } => show_markdown_config(parent, panel_name, file, min_width, min_height, on_done),
        PanelType::Browser { url } => show_browser_config(parent, panel_name, url, min_width, min_height, on_done),
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
    mode_inline: gtk4::CheckButton,
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
        mode_inline,
        file_entry,
        script_view,
    }
}

/// Extract the full script text from a TextView buffer, keeping shebang.
fn get_script_lines(view: &gtk4::TextView) -> Vec<String> {
    let buf = view.buffer();
    let text = buf.text(&buf.start_iter(), &buf.end_iter(), false);
    let text = text.to_string();
    if text.trim().is_empty() {
        return vec![];
    }
    // Return as a single element — the full script
    vec![text]
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
    startup_commands: &[String],
    before_close: Option<&str>,
    min_width: u32,
    min_height: u32,
    on_done: impl Fn(String, PanelType, Option<String>, Vec<String>, Option<String>, u32, u32) + 'static,
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
    add_buttons(&vbox, &dialog, move || {
        let name = ne.text().to_string();
        let cwd_text = ce.text().to_string();
        let cwd = if cwd_text.trim().is_empty() { None } else { Some(cwd_text) };
        let selected = id.selected() as usize;
        let interpreter = interps.get(selected).cloned().unwrap_or_else(|| "/bin/bash".to_string());
        let mw = mw_spin.value() as u32;
        let mh = mh_spin.value() as u32;

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
            on_done(name, PanelType::Terminal, cwd, vec![], before_close, mw, mh);
            return;
        }

        let cmds = script_editor.get_script();
        if cmds.is_empty() {
            on_done(name, PanelType::Terminal, cwd, vec![], before_close, mw, mh);
            return;
        }

        // For file mode, prepend interpreter info
        let first = &cmds[0];
        if first.starts_with("file:") {
            let path = first.trim_start_matches("file:");
            on_done(name, PanelType::Terminal, cwd, vec![format!("file:{}:{}", interpreter, path)], before_close, mw, mh);
        } else {
            let script = if first.starts_with("#!") {
                let rest = first.lines().skip(1).collect::<Vec<_>>().join("\n");
                format!("#!{}\n{}", interpreter, rest)
            } else {
                format!("#!{}\n{}", interpreter, first.clone())
            };
            on_done(name, PanelType::Terminal, cwd, vec![script], before_close, mw, mh);
        }
    });

    dialog.set_child(Some(&vbox));
    dialog.present();
}

fn show_ssh_config(
    parent: &impl IsA<gtk4::Window>,
    panel_name: &str,
    host: &str,
    port: u16,
    user: Option<&str>,
    identity_file: Option<&str>,
    min_width: u32,
    min_height: u32,
    on_done: impl Fn(String, PanelType, Option<String>, Vec<String>, Option<String>, u32, u32) + 'static,
) {
    let dialog = make_dialog(parent, "SSH Configuration");
    let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
    vbox.set_margin_top(16);
    vbox.set_margin_bottom(16);
    vbox.set_margin_start(16);
    vbox.set_margin_end(16);

    let name_entry = add_field(&vbox, "Name:", panel_name, "SSH Terminal");
    let host_entry = add_field(&vbox, "Host:", host, "hostname or IP");
    let port_entry = add_field(&vbox, "Port:", &port.to_string(), "22");
    let user_entry = add_field(&vbox, "User:", user.unwrap_or(""), "username");
    let id_entry = add_field(&vbox, "Identity file:", identity_file.unwrap_or(""), "~/.ssh/id_rsa");

    let (mw_spin, mh_spin) = add_min_size_fields(&vbox, min_width, min_height);

    add_buttons(&vbox, &dialog, move || {
        let name = name_entry.text().to_string();
        let host = host_entry.text().to_string();
        let port = port_entry.text().parse::<u16>().unwrap_or(22);
        let user = if user_entry.text().is_empty() { None } else { Some(user_entry.text().to_string()) };
        let identity = if id_entry.text().is_empty() { None } else { Some(id_entry.text().to_string()) };
        on_done(name, PanelType::Ssh { host, port, user, identity_file: identity }, None, vec![], None, mw_spin.value() as u32, mh_spin.value() as u32);
    });

    dialog.set_child(Some(&vbox));
    dialog.present();
}

fn show_tmux_config(
    parent: &impl IsA<gtk4::Window>,
    panel_name: &str,
    host: &str,
    session: &str,
    user: Option<&str>,
    min_width: u32,
    min_height: u32,
    on_done: impl Fn(String, PanelType, Option<String>, Vec<String>, Option<String>, u32, u32) + 'static,
) {
    let dialog = make_dialog(parent, "Remote Tmux Configuration");
    let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
    vbox.set_margin_top(16);
    vbox.set_margin_bottom(16);
    vbox.set_margin_start(16);
    vbox.set_margin_end(16);

    let name_entry = add_field(&vbox, "Name:", panel_name, "Remote Tmux");
    let host_entry = add_field(&vbox, "Host:", host, "hostname or IP");
    let session_entry = add_field(&vbox, "Session:", session, "main");
    let user_entry = add_field(&vbox, "User:", user.unwrap_or(""), "username");

    let (mw_spin, mh_spin) = add_min_size_fields(&vbox, min_width, min_height);

    add_buttons(&vbox, &dialog, move || {
        let name = name_entry.text().to_string();
        let host = host_entry.text().to_string();
        let session = session_entry.text().to_string();
        let user = if user_entry.text().is_empty() { None } else { Some(user_entry.text().to_string()) };
        on_done(name, PanelType::RemoteTmux { host, session, user }, None, vec![], None, mw_spin.value() as u32, mh_spin.value() as u32);
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
    on_done: impl Fn(String, PanelType, Option<String>, Vec<String>, Option<String>, u32, u32) + 'static,
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
        on_done(name, PanelType::Markdown { file }, None, vec![], None, mw_spin.value() as u32, mh_spin.value() as u32);
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
    on_done: impl Fn(String, PanelType, Option<String>, Vec<String>, Option<String>, u32, u32) + 'static,
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
        on_done(name, PanelType::Browser { url }, None, vec![], None, mw_spin.value() as u32, mh_spin.value() as u32);
    });

    dialog.set_child(Some(&vbox));
    dialog.present();
}
