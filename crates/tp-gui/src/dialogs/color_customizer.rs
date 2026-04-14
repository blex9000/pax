use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use gtk4::prelude::*;

use crate::theme::Theme;

struct ColorToken {
    css_name: &'static str,
    label: &'static str,
}

const BG_TOKENS: &[ColorToken] = &[
    ColorToken { css_name: "bg_window", label: "Window" },
    ColorToken { css_name: "bg_chrome", label: "Toolbars, Headers & Dialogs" },
    ColorToken { css_name: "bg_surface", label: "Content (Editor, Terminal, Forms)" },
    ColorToken { css_name: "bg_elevated", label: "Popovers, Cards & Sidebars" },
];

const TEXT_TOKENS: &[ColorToken] = &[
    ColorToken { css_name: "fg_ui", label: "UI Chrome (Labels, Icons, Buttons)" },
    ColorToken { css_name: "fg_content", label: "Content (Editor, File Tree, Forms)" },
];

const ACCENT_TOKENS: &[ColorToken] = &[
    ColorToken { css_name: "accent", label: "Hover & Focus Highlights" },
    ColorToken { css_name: "accent_bg", label: "Selection & Active Backgrounds" },
    ColorToken { css_name: "accent_fg", label: "Text on Selections & Buttons" },
];

const BORDER_TOKENS: &[ColorToken] = &[
    ColorToken { css_name: "border_soft", label: "Internal (Popups, Editor Dividers)" },
    ColorToken { css_name: "border_hard", label: "Structural (Panels, Tabs, Header)" },
];

const GROUPS: &[(&str, &[ColorToken])] = &[
    ("Backgrounds", BG_TOKENS),
    ("Text", TEXT_TOKENS),
    ("Accents", ACCENT_TOKENS),
    ("Borders", BORDER_TOKENS),
];

/// Try to parse a hex color string (`#rrggbb` or `#rrggbbaa`) into an RGBA.
fn hex_to_rgba(hex: &str) -> Option<gtk4::gdk::RGBA> {
    let hex = hex.trim().trim_start_matches('#');
    let (r, g, b, a) = match hex.len() {
        6 => {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            (r, g, b, 255u8)
        }
        8 => {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            let a = u8::from_str_radix(&hex[6..8], 16).ok()?;
            (r, g, b, a)
        }
        _ => return None,
    };
    Some(gtk4::gdk::RGBA::new(
        r as f32 / 255.0,
        g as f32 / 255.0,
        b as f32 / 255.0,
        a as f32 / 255.0,
    ))
}

fn rgba_to_hex(c: &gtk4::gdk::RGBA) -> String {
    format!(
        "#{:02x}{:02x}{:02x}",
        (c.red() * 255.0) as u8,
        (c.green() * 255.0) as u8,
        (c.blue() * 255.0) as u8,
    )
}

/// Open the color customizer dialog for tweaking individual theme CSS tokens.
pub fn show_color_customizer_dialog(parent: &impl IsA<gtk4::Window>) {
    let theme = crate::theme::current_theme();
    let css = theme.css_overrides();

    let dialog = gtk4::Window::builder()
        .title("Customize Theme Colors")
        .transient_for(parent)
        .modal(true)
        .default_width(420)
        .default_height(540)
        .build();
    crate::theme::configure_dialog_window(&dialog);

    // Closing via the X button (without Save) reverts to the base theme
    // so the app never stays in a half-customized state.
    let theme_for_close = theme;
    dialog.connect_close_request(move |_| {
        crate::app::apply_theme(theme_for_close);
        gtk4::glib::Propagation::Proceed
    });

    // Load any previously saved overrides so the pickers start at the
    // user's last saved state (not the base theme defaults).
    let saved = load_custom_colors(theme).unwrap_or_default();
    let overrides: Rc<RefCell<HashMap<String, String>>> =
        Rc::new(RefCell::new(saved.clone()));

    let scroll = gtk4::ScrolledWindow::new();
    scroll.set_vexpand(true);

    let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    vbox.set_margin_top(12);
    vbox.set_margin_bottom(12);
    vbox.set_margin_start(16);
    vbox.set_margin_end(16);

    for &(group_name, tokens) in GROUPS {
        let section = gtk4::Label::new(Some(group_name));
        section.add_css_class("title-4");
        section.set_halign(gtk4::Align::Start);
        section.set_margin_top(12);
        section.set_margin_bottom(4);
        vbox.append(&section);

        for token in tokens {
            let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 10);
            row.set_margin_top(3);
            row.set_margin_bottom(3);

            let lbl = gtk4::Label::new(Some(token.label));
            lbl.set_halign(gtk4::Align::Start);
            lbl.set_hexpand(true);
            lbl.set_xalign(0.0);
            row.append(&lbl);

            // Use the saved override value if present, otherwise fall back
            // to the base theme's CSS value.
            let initial_hex = saved
                .get(token.css_name)
                .cloned()
                .or_else(|| crate::theme::parse_define_color(css, token.css_name));
            let initial_rgba = initial_hex
                .as_deref()
                .and_then(hex_to_rgba)
                .unwrap_or_else(|| gtk4::gdk::RGBA::new(0.5, 0.5, 0.5, 1.0));

            let color_dialog = gtk4::ColorDialog::new();
            let btn = gtk4::ColorDialogButton::new(Some(color_dialog));
            btn.set_rgba(&initial_rgba);
            btn.set_valign(gtk4::Align::Center);

            let overrides_ref = overrides.clone();
            let token_name = token.css_name.to_string();
            let theme_copy = theme;
            btn.connect_rgba_notify(move |b| {
                let rgba = b.rgba();
                let hex = rgba_to_hex(&rgba);
                overrides_ref.borrow_mut().insert(token_name.clone(), hex);
                crate::app::apply_theme_with_overrides(theme_copy, &overrides_ref.borrow());
            });

            row.append(&btn);
            vbox.append(&row);
        }
    }

    scroll.set_child(Some(&vbox));

    let outer = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    outer.append(&scroll);

    // Button bar
    let btn_bar = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    btn_bar.set_halign(gtk4::Align::End);
    btn_bar.set_margin_top(8);
    btn_bar.set_margin_bottom(12);
    btn_bar.set_margin_end(16);

    let reset_btn = gtk4::Button::with_label("Reset");
    reset_btn.add_css_class("flat");
    let theme_for_reset = theme;
    let d = dialog.clone();
    reset_btn.connect_clicked(move |_| {
        clear_custom_colors();
        crate::app::apply_theme(theme_for_reset);
        d.close();
    });

    let save_btn = gtk4::Button::with_label("Save");
    save_btn.add_css_class("suggested-action");
    let overrides_for_save = overrides.clone();
    let theme_for_save = theme;
    let d2 = dialog.clone();
    save_btn.connect_clicked(move |_| {
        save_custom_colors(theme_for_save, &overrides_for_save.borrow());
        d2.close();
    });

    let cancel_btn = gtk4::Button::with_label("Cancel");
    cancel_btn.add_css_class("flat");
    let theme_for_cancel = theme;
    let d3 = dialog.clone();
    cancel_btn.connect_clicked(move |_| {
        crate::app::apply_theme(theme_for_cancel);
        d3.close();
    });

    btn_bar.append(&cancel_btn);
    btn_bar.append(&reset_btn);
    btn_bar.append(&save_btn);
    outer.append(&btn_bar);

    dialog.set_child(Some(&outer));
    dialog.present();
}

fn save_custom_colors(theme: Theme, overrides: &HashMap<String, String>) {
    if overrides.is_empty() {
        clear_custom_colors();
        return;
    }
    let db_path = pax_db::Database::default_path();
    let Ok(db) = pax_db::Database::open(&db_path) else {
        return;
    };
    let payload = serde_json::json!({
        "base": theme.to_id(),
        "overrides": overrides,
    });
    let _ = db.set_app_preference("custom-theme-colors", &payload.to_string());
}

pub(crate) fn load_custom_colors(theme: Theme) -> Option<HashMap<String, String>> {
    let db_path = pax_db::Database::default_path();
    let db = pax_db::Database::open(&db_path).ok()?;
    let json = db.get_app_preference("custom-theme-colors").ok()??;
    let v: serde_json::Value = serde_json::from_str(&json).ok()?;
    let base = v.get("base")?.as_str()?;
    if base != theme.to_id() {
        return None;
    }
    let map = v.get("overrides")?.as_object()?;
    let mut out = HashMap::new();
    for (k, val) in map {
        if let Some(s) = val.as_str() {
            out.insert(k.clone(), s.to_string());
        }
    }
    if out.is_empty() { None } else { Some(out) }
}

fn clear_custom_colors() {
    let db_path = pax_db::Database::default_path();
    let Ok(db) = pax_db::Database::open(&db_path) else {
        return;
    };
    let _ = db.set_app_preference("custom-theme-colors", "");
}
