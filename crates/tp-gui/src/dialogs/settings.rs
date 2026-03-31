use gtk4::prelude::*;

use crate::theme::Theme;

/// Settings that can be changed in the dialog.
#[derive(Debug, Clone)]
pub struct AppSettings {
    pub workspace_name: String,
    pub theme: Theme,
    pub default_shell: String,
    pub scrollback_lines: usize,
    pub output_retention_days: Option<u32>,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            workspace_name: "untitled".to_string(),
            theme: Theme::System,
            default_shell: std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string()),
            scrollback_lines: 10_000,
            output_retention_days: None,
        }
    }
}

pub fn show_settings_dialog(
    parent: &impl IsA<gtk4::Window>,
    current: &AppSettings,
    on_apply: impl Fn(AppSettings) + 'static,
) {
    let dialog = gtk4::Window::builder()
        .title("Settings")
        .transient_for(parent)
        .modal(true)
        .default_width(500)
        .default_height(500)
        .build();

    let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    vbox.set_margin_top(16);
    vbox.set_margin_bottom(16);
    vbox.set_margin_start(20);
    vbox.set_margin_end(20);

    // ── Workspace ─────────────────────────────────────

    let section_label = gtk4::Label::new(Some("Workspace"));
    section_label.add_css_class("title-4");
    section_label.set_halign(gtk4::Align::Start);
    section_label.set_margin_bottom(8);
    vbox.append(&section_label);

    // Workspace name
    let name_row = make_row("Name");
    let name_entry = gtk4::Entry::new();
    name_entry.set_text(&current.workspace_name);
    name_entry.set_hexpand(true);
    name_row.append(&name_entry);
    vbox.append(&name_row);

    add_separator(&vbox);

    // ── Appearance ──────────────────────────────────────

    let section_label2 = gtk4::Label::new(Some("Appearance"));
    section_label2.add_css_class("title-4");
    section_label2.set_halign(gtk4::Align::Start);
    section_label2.set_margin_bottom(8);
    vbox.append(&section_label2);

    // Theme
    let theme_row = make_row("Theme");
    let theme_dropdown = gtk4::DropDown::from_strings(
        &Theme::all().iter().map(|t| t.label()).collect::<Vec<_>>(),
    );
    let current_idx = Theme::all().iter().position(|t| *t == current.theme).unwrap_or(0);
    theme_dropdown.set_selected(current_idx as u32);
    theme_row.append(&theme_dropdown);
    vbox.append(&theme_row);

    add_separator(&vbox);

    // ── Terminal ────────────────────────────────────────

    let section_label3 = gtk4::Label::new(Some("Terminal"));
    section_label3.add_css_class("title-4");
    section_label3.set_halign(gtk4::Align::Start);
    section_label3.set_margin_bottom(8);
    vbox.append(&section_label3);

    // Default shell
    let shell_row = make_row("Default shell");
    let shell_entry = gtk4::Entry::new();
    shell_entry.set_text(&current.default_shell);
    shell_entry.set_hexpand(true);
    shell_row.append(&shell_entry);
    vbox.append(&shell_row);

    // Scrollback lines
    let scroll_row = make_row("Scrollback lines");
    let scroll_spin = gtk4::SpinButton::with_range(100.0, 1_000_000.0, 1000.0);
    scroll_spin.set_value(current.scrollback_lines as f64);
    scroll_row.append(&scroll_spin);
    vbox.append(&scroll_row);

    add_separator(&vbox);

    // ── Data ───────────────────────────────────────────

    let section_label4 = gtk4::Label::new(Some("Data"));
    section_label4.add_css_class("title-4");
    section_label4.set_halign(gtk4::Align::Start);
    section_label4.set_margin_bottom(8);
    vbox.append(&section_label4);

    // Output retention
    let retention_row = make_row("Output retention (days)");
    let retention_spin = gtk4::SpinButton::with_range(0.0, 365.0, 1.0);
    retention_spin.set_value(current.output_retention_days.unwrap_or(0) as f64);
    retention_spin.set_tooltip_text(Some("0 = keep forever"));
    retention_row.append(&retention_spin);
    vbox.append(&retention_row);

    // DB path (read-only info)
    let db_path = pax_db::Database::default_path();
    let db_row = make_row("Database");
    let db_label = gtk4::Label::new(Some(&db_path.to_string_lossy()));
    db_label.add_css_class("dim-label");
    db_label.add_css_class("caption");
    db_label.set_ellipsize(gtk4::pango::EllipsizeMode::Start);
    db_label.set_tooltip_text(Some(&db_path.to_string_lossy()));
    db_label.set_hexpand(true);
    db_label.set_halign(gtk4::Align::Start);
    db_row.append(&db_label);
    vbox.append(&db_row);

    // Log path (read-only info)
    let log_path = db_path.parent().unwrap_or(std::path::Path::new("/tmp")).join("pax.log");
    let log_row = make_row("Log file");
    let log_label = gtk4::Label::new(Some(&log_path.to_string_lossy()));
    log_label.add_css_class("dim-label");
    log_label.add_css_class("caption");
    log_label.set_ellipsize(gtk4::pango::EllipsizeMode::Start);
    log_label.set_tooltip_text(Some(&log_path.to_string_lossy()));
    log_label.set_hexpand(true);
    log_label.set_halign(gtk4::Align::Start);
    log_row.append(&log_label);
    vbox.append(&log_row);

    // ── Buttons ────────────────────────────────────────

    let btn_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    btn_box.set_halign(gtk4::Align::End);
    btn_box.set_margin_top(20);

    let cancel_btn = gtk4::Button::with_label("Cancel");
    cancel_btn.add_css_class("flat");
    let d = dialog.clone();
    cancel_btn.connect_clicked(move |_| d.close());

    let apply_btn = gtk4::Button::with_label("Apply");
    apply_btn.add_css_class("suggested-action");

    let d = dialog.clone();
    let ne = name_entry.clone();
    let td = theme_dropdown.clone();
    let se = shell_entry.clone();
    let ss = scroll_spin.clone();
    let rs = retention_spin.clone();
    apply_btn.connect_clicked(move |_| {
        let theme_idx = td.selected() as usize;
        let theme = Theme::all().get(theme_idx).copied().unwrap_or(Theme::System);
        let retention = rs.value() as u32;

        on_apply(AppSettings {
            workspace_name: ne.text().to_string(),
            theme,
            default_shell: se.text().to_string(),
            scrollback_lines: ss.value() as usize,
            output_retention_days: if retention == 0 { None } else { Some(retention) },
        });
        d.close();
    });

    btn_box.append(&cancel_btn);
    btn_box.append(&apply_btn);
    vbox.append(&btn_box);

    dialog.set_child(Some(&vbox));
    dialog.present();
}

fn make_row(label: &str) -> gtk4::Box {
    let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);
    row.set_margin_top(4);
    row.set_margin_bottom(4);
    let lbl = gtk4::Label::new(Some(label));
    lbl.set_halign(gtk4::Align::Start);
    lbl.set_xalign(0.0);
    lbl.set_width_chars(20);
    row.append(&lbl);
    row
}

fn add_separator(vbox: &gtk4::Box) {
    let sep = gtk4::Separator::new(gtk4::Orientation::Horizontal);
    sep.set_margin_top(12);
    sep.set_margin_bottom(12);
    vbox.append(&sep);
}
