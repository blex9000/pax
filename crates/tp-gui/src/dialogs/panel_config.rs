use gtk4::prelude::*;
use std::collections::HashMap;

use tp_core::workspace::PanelType;

/// Show a configuration dialog for the given panel type.
/// Returns the updated PanelType if the user confirms, None if cancelled.
pub fn show_panel_config_dialog(
    parent: &impl IsA<gtk4::Window>,
    panel_name: &str,
    panel_type: &PanelType,
    on_done: impl Fn(String, PanelType) + 'static,
) {
    match panel_type {
        PanelType::Terminal => show_terminal_config(parent, panel_name, on_done),
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

fn make_dialog(parent: &impl IsA<gtk4::Window>, title: &str) -> (gtk4::Window, gtk4::Box, gtk4::Entry) {
    let dialog = gtk4::Window::builder()
        .title(title)
        .transient_for(parent)
        .modal(true)
        .default_width(450)
        .default_height(300)
        .build();

    let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
    vbox.set_margin_top(16);
    vbox.set_margin_bottom(16);
    vbox.set_margin_start(16);
    vbox.set_margin_end(16);

    let name_entry = gtk4::Entry::new();
    name_entry.set_placeholder_text(Some("Panel name"));

    (dialog, vbox, name_entry)
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

fn show_terminal_config(
    parent: &impl IsA<gtk4::Window>,
    panel_name: &str,
    on_done: impl Fn(String, PanelType) + 'static,
) {
    let (dialog, vbox, _) = make_dialog(parent, "Terminal Configuration");

    let name_entry = add_field(&vbox, "Name:", panel_name, "Terminal");

    // Startup commands (multi-line)
    let cmd_label = gtk4::Label::new(Some("Startup commands (one per line):"));
    cmd_label.set_halign(gtk4::Align::Start);
    vbox.append(&cmd_label);

    let cmd_view = gtk4::TextView::new();
    cmd_view.set_monospace(true);
    cmd_view.set_wrap_mode(gtk4::WrapMode::Word);
    let cmd_scroll = gtk4::ScrolledWindow::new();
    cmd_scroll.set_child(Some(&cmd_view));
    cmd_scroll.set_min_content_height(80);
    cmd_scroll.set_vexpand(true);
    vbox.append(&cmd_scroll);

    let ne = name_entry.clone();
    let cv = cmd_view.clone();
    add_buttons(&vbox, &dialog, move || {
        let name = ne.text().to_string();
        // Store startup commands in panel config via callback
        // For now, Terminal type doesn't carry commands in PanelType itself
        on_done(name, PanelType::Terminal);
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
    on_done: impl Fn(String, PanelType) + 'static,
) {
    let (dialog, vbox, _) = make_dialog(parent, "SSH Configuration");

    let name_entry = add_field(&vbox, "Name:", panel_name, "SSH Terminal");
    let host_entry = add_field(&vbox, "Host:", host, "hostname or IP");
    let port_entry = add_field(&vbox, "Port:", &port.to_string(), "22");
    let user_entry = add_field(&vbox, "User:", user.unwrap_or(""), "username");
    let id_entry = add_field(&vbox, "Identity file:", identity_file.unwrap_or(""), "~/.ssh/id_rsa");

    let ne = name_entry.clone();
    let he = host_entry.clone();
    let pe = port_entry.clone();
    let ue = user_entry.clone();
    let ie = id_entry.clone();
    add_buttons(&vbox, &dialog, move || {
        let name = ne.text().to_string();
        let host = he.text().to_string();
        let port = pe.text().parse::<u16>().unwrap_or(22);
        let user = if ue.text().is_empty() { None } else { Some(ue.text().to_string()) };
        let identity = if ie.text().is_empty() { None } else { Some(ie.text().to_string()) };
        on_done(name, PanelType::Ssh { host, port, user, identity_file: identity });
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
    on_done: impl Fn(String, PanelType) + 'static,
) {
    let (dialog, vbox, _) = make_dialog(parent, "Remote Tmux Configuration");

    let name_entry = add_field(&vbox, "Name:", panel_name, "Remote Tmux");
    let host_entry = add_field(&vbox, "Host:", host, "hostname or IP");
    let session_entry = add_field(&vbox, "Session:", session, "main");
    let user_entry = add_field(&vbox, "User:", user.unwrap_or(""), "username");

    let ne = name_entry.clone();
    let he = host_entry.clone();
    let se = session_entry.clone();
    let ue = user_entry.clone();
    add_buttons(&vbox, &dialog, move || {
        let name = ne.text().to_string();
        let host = he.text().to_string();
        let session = se.text().to_string();
        let user = if ue.text().is_empty() { None } else { Some(ue.text().to_string()) };
        on_done(name, PanelType::RemoteTmux { host, session, user });
    });

    dialog.set_child(Some(&vbox));
    dialog.present();
}

fn show_markdown_config(
    parent: &impl IsA<gtk4::Window>,
    panel_name: &str,
    file: &str,
    on_done: impl Fn(String, PanelType) + 'static,
) {
    let (dialog, vbox, _) = make_dialog(parent, "Markdown Configuration");

    let name_entry = add_field(&vbox, "Name:", panel_name, "Markdown");
    let file_entry = add_field(&vbox, "File:", file, "path/to/file.md");

    // Browse button
    let browse_btn = gtk4::Button::with_label("Browse...");
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

    let ne = name_entry.clone();
    let fe = file_entry.clone();
    add_buttons(&vbox, &dialog, move || {
        let name = ne.text().to_string();
        let file = fe.text().to_string();
        on_done(name, PanelType::Markdown { file });
    });

    dialog.set_child(Some(&vbox));
    dialog.present();
}

fn show_browser_config(
    parent: &impl IsA<gtk4::Window>,
    panel_name: &str,
    url: &str,
    on_done: impl Fn(String, PanelType) + 'static,
) {
    let (dialog, vbox, _) = make_dialog(parent, "Browser Configuration");

    let name_entry = add_field(&vbox, "Name:", panel_name, "Browser");
    let url_entry = add_field(&vbox, "URL:", url, "https://example.com");

    let ne = name_entry.clone();
    let ue = url_entry.clone();
    add_buttons(&vbox, &dialog, move || {
        let name = ne.text().to_string();
        let url = ue.text().to_string();
        on_done(name, PanelType::Browser { url });
    });

    dialog.set_child(Some(&vbox));
    dialog.present();
}
