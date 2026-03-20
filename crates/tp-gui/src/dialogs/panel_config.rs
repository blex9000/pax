use gtk4::prelude::*;

use tp_core::workspace::PanelType;

/// Callback: (name, panel_type, startup_commands)
pub type ConfigDoneCallback = dyn Fn(String, PanelType, Vec<String>) + 'static;

/// Show a configuration dialog for the given panel type.
pub fn show_panel_config_dialog(
    parent: &impl IsA<gtk4::Window>,
    panel_name: &str,
    panel_type: &PanelType,
    startup_commands: &[String],
    on_done: impl Fn(String, PanelType, Vec<String>) + 'static,
) {
    match panel_type {
        PanelType::Terminal => show_terminal_config(parent, panel_name, startup_commands, on_done),
        PanelType::Ssh { host, port, user, identity_file } => {
            show_ssh_config(parent, panel_name, host, *port, user.as_deref(), identity_file.as_deref(), on_done)
        }
        PanelType::RemoteTmux { host, session, user } => {
            show_tmux_config(parent, panel_name, host, session, user.as_deref(), on_done)
        }
        PanelType::Markdown { file } => show_markdown_config(parent, panel_name, file, on_done),
        PanelType::Browser { url } => show_browser_config(parent, panel_name, url, on_done),
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

/// Build the startup script editor widget.
/// Returns the TextView that contains the script.
fn add_script_editor(
    vbox: &gtk4::Box,
    dialog: &gtk4::Window,
    existing_commands: &[String],
) -> gtk4::TextView {
    let label = gtk4::Label::new(Some("Startup script:"));
    label.set_halign(gtk4::Align::Start);
    vbox.append(&label);

    let script_view = gtk4::TextView::new();
    script_view.set_monospace(true);
    script_view.set_wrap_mode(gtk4::WrapMode::Word);
    script_view.set_left_margin(8);
    script_view.set_top_margin(4);

    // Prepopulate
    let initial_text = if existing_commands.is_empty() {
        "#!/bin/bash\necho \"Hello World\"\n".to_string()
    } else {
        existing_commands.join("\n")
    };
    script_view.buffer().set_text(&initial_text);

    let scroll = gtk4::ScrolledWindow::new();
    scroll.set_child(Some(&script_view));
    scroll.set_min_content_height(120);
    scroll.set_vexpand(true);
    vbox.append(&scroll);

    // Browse button to load script from file
    let browse_btn = gtk4::Button::with_label("Load from file…");
    browse_btn.add_css_class("flat");
    browse_btn.set_halign(gtk4::Align::Start);

    let sv = script_view.clone();
    let d = dialog.clone();
    browse_btn.connect_clicked(move |_| {
        let file_dialog = gtk4::FileDialog::builder()
            .title("Select Script")
            .modal(true)
            .build();
        let filter = gtk4::FileFilter::new();
        filter.set_name(Some("Scripts"));
        filter.add_pattern("*.sh");
        filter.add_pattern("*.bash");
        filter.add_mime_type("application/x-shellscript");
        let all = gtk4::FileFilter::new();
        all.set_name(Some("All files"));
        all.add_pattern("*");
        let filters = gtk4::gio::ListStore::new::<gtk4::FileFilter>();
        filters.append(&filter);
        filters.append(&all);
        file_dialog.set_filters(Some(&filters));

        let sv2 = sv.clone();
        file_dialog.open(Some(&d), gtk4::gio::Cancellable::NONE, move |result| {
            if let Ok(file) = result {
                if let Some(path) = file.path() {
                    if let Ok(content) = std::fs::read_to_string(&path) {
                        sv2.buffer().set_text(&content);
                    }
                }
            }
        });
    });
    vbox.append(&browse_btn);

    script_view
}

/// Extract script lines from a TextView buffer.
fn get_script_lines(view: &gtk4::TextView) -> Vec<String> {
    let buf = view.buffer();
    let text = buf.text(&buf.start_iter(), &buf.end_iter(), false);
    let text = text.to_string();
    if text.trim().is_empty() {
        return vec![];
    }
    text.lines()
        .filter(|l| !l.starts_with("#!")) // Skip shebang
        .map(|l| l.to_string())
        .collect()
}

fn show_terminal_config(
    parent: &impl IsA<gtk4::Window>,
    panel_name: &str,
    startup_commands: &[String],
    on_done: impl Fn(String, PanelType, Vec<String>) + 'static,
) {
    let dialog = make_dialog(parent, "Terminal Configuration");
    let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
    vbox.set_margin_top(16);
    vbox.set_margin_bottom(16);
    vbox.set_margin_start(16);
    vbox.set_margin_end(16);

    let name_entry = add_field(&vbox, "Name:", panel_name, "Terminal");
    let script_view = add_script_editor(&vbox, &dialog, startup_commands);

    let ne = name_entry.clone();
    let sv = script_view.clone();
    add_buttons(&vbox, &dialog, move || {
        let name = ne.text().to_string();
        let cmds = get_script_lines(&sv);
        on_done(name, PanelType::Terminal, cmds);
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
    on_done: impl Fn(String, PanelType, Vec<String>) + 'static,
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

    add_buttons(&vbox, &dialog, move || {
        let name = name_entry.text().to_string();
        let host = host_entry.text().to_string();
        let port = port_entry.text().parse::<u16>().unwrap_or(22);
        let user = if user_entry.text().is_empty() { None } else { Some(user_entry.text().to_string()) };
        let identity = if id_entry.text().is_empty() { None } else { Some(id_entry.text().to_string()) };
        on_done(name, PanelType::Ssh { host, port, user, identity_file: identity }, vec![]);
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
    on_done: impl Fn(String, PanelType, Vec<String>) + 'static,
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

    add_buttons(&vbox, &dialog, move || {
        let name = name_entry.text().to_string();
        let host = host_entry.text().to_string();
        let session = session_entry.text().to_string();
        let user = if user_entry.text().is_empty() { None } else { Some(user_entry.text().to_string()) };
        on_done(name, PanelType::RemoteTmux { host, session, user }, vec![]);
    });

    dialog.set_child(Some(&vbox));
    dialog.present();
}

fn show_markdown_config(
    parent: &impl IsA<gtk4::Window>,
    panel_name: &str,
    file: &str,
    on_done: impl Fn(String, PanelType, Vec<String>) + 'static,
) {
    let dialog = make_dialog(parent, "Markdown Configuration");
    let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
    vbox.set_margin_top(16);
    vbox.set_margin_bottom(16);
    vbox.set_margin_start(16);
    vbox.set_margin_end(16);

    let name_entry = add_field(&vbox, "Name:", panel_name, "Markdown");
    let file_entry = add_field(&vbox, "File:", file, "path/to/file.md");

    let browse_btn = gtk4::Button::with_label("Browse…");
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

    add_buttons(&vbox, &dialog, move || {
        let name = name_entry.text().to_string();
        let file = file_entry.text().to_string();
        on_done(name, PanelType::Markdown { file }, vec![]);
    });

    dialog.set_child(Some(&vbox));
    dialog.present();
}

fn show_browser_config(
    parent: &impl IsA<gtk4::Window>,
    panel_name: &str,
    url: &str,
    on_done: impl Fn(String, PanelType, Vec<String>) + 'static,
) {
    let dialog = make_dialog(parent, "Browser Configuration");
    let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
    vbox.set_margin_top(16);
    vbox.set_margin_bottom(16);
    vbox.set_margin_start(16);
    vbox.set_margin_end(16);

    let name_entry = add_field(&vbox, "Name:", panel_name, "Browser");
    let url_entry = add_field(&vbox, "URL:", url, "https://example.com");

    add_buttons(&vbox, &dialog, move || {
        let name = name_entry.text().to_string();
        let url = url_entry.text().to_string();
        on_done(name, PanelType::Browser { url }, vec![]);
    });

    dialog.set_child(Some(&vbox));
    dialog.present();
}
