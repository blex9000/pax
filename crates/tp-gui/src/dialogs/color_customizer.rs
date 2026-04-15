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
    ColorToken { css_name: "bg_window", label: "Window & Chrome" },
    ColorToken { css_name: "bg_surface", label: "Content (Editor, Terminal, Forms)" },
    ColorToken { css_name: "bg_elevated", label: "Cards" },
    ColorToken { css_name: "bg_popover", label: "Popovers & Dropdowns" },
];

const TEXT_TOKENS: &[ColorToken] = &[
    ColorToken { css_name: "fg_ui", label: "UI Chrome (Labels, Icons, Buttons)" },
    ColorToken { css_name: "fg_content", label: "Content (Editor, File Tree, Forms)" },
];

const ACCENT_TOKENS: &[ColorToken] = &[
    ColorToken { css_name: "accent", label: "Accent (Focus, Selection & Active)" },
    ColorToken { css_name: "accent_fg", label: "Text on Accent Backgrounds" },
    ColorToken { css_name: "hover_bg", label: "Hover Background" },
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

/// Parse a CSS color value into an RGBA. Handles hex (#rrggbb), named colors
/// (white, black), and the GTK `alpha(color, opacity)` function.
fn css_value_to_rgba(val: &str) -> Option<gtk4::gdk::RGBA> {
    let val = val.trim();
    // Try direct parse first: handles #hex, rgb(), rgba(), named colors
    if let Ok(rgba) = gtk4::gdk::RGBA::parse(val) {
        return Some(rgba);
    }
    // Handle alpha(color, opacity) — common in border tokens
    if let Some(inner) = val.strip_prefix("alpha(").and_then(|s| s.strip_suffix(')')) {
        let last_comma = inner.rfind(',')?;
        let color_part = inner[..last_comma].trim();
        let alpha_part = inner[last_comma + 1..].trim();
        let mut base = gtk4::gdk::RGBA::parse(color_part).ok()?;
        let alpha: f32 = alpha_part.parse().ok()?;
        base.set_alpha(alpha);
        return Some(base);
    }
    None
}

fn rgba_to_css(c: &gtk4::gdk::RGBA) -> String {
    if (c.alpha() - 1.0).abs() < 0.01 {
        format!(
            "#{:02x}{:02x}{:02x}",
            (c.red() * 255.0) as u8,
            (c.green() * 255.0) as u8,
            (c.blue() * 255.0) as u8,
        )
    } else {
        format!(
            "#{:02x}{:02x}{:02x}{:02x}",
            (c.red() * 255.0) as u8,
            (c.green() * 255.0) as u8,
            (c.blue() * 255.0) as u8,
            (c.alpha() * 255.0) as u8,
        )
    }
}

/// Open the color customizer dialog for tweaking individual theme CSS tokens.
pub fn show_color_customizer_dialog(parent: &impl IsA<gtk4::Window>) {
    let theme = crate::theme::current_theme();
    let css = theme.css_overrides();

    let dialog = gtk4::Window::builder()
        .title("Customize Theme Colors")
        .transient_for(parent)
        .modal(true)
        .default_width(520)
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
                .and_then(css_value_to_rgba)
                .unwrap_or_else(|| gtk4::gdk::RGBA::new(0.5, 0.5, 0.5, 1.0));
            tracing::debug!("color_customizer: {} = {:?} -> rgba({},{},{},{})",
                token.css_name, initial_hex,
                initial_rgba.red(), initial_rgba.green(), initial_rgba.blue(), initial_rgba.alpha());

            let current_rgba = Rc::new(RefCell::new(initial_rgba));

            // Color swatch button
            let btn = gtk4::Button::new();
            btn.set_valign(gtk4::Align::Center);
            btn.set_size_request(28, 20);
            let swatch = gtk4::DrawingArea::new();
            swatch.set_size_request(28, 20);
            let rgba_for_draw = current_rgba.clone();
            swatch.set_draw_func(move |_, cr, w, h| {
                let c = rgba_for_draw.borrow();
                cr.set_source_rgba(c.red() as f64, c.green() as f64, c.blue() as f64, c.alpha() as f64);
                cr.rectangle(0.0, 0.0, w as f64, h as f64);
                let _ = cr.fill();
            });
            btn.set_child(Some(&swatch));

            // Value label showing hex/rgba text
            let value_label = gtk4::Label::new(Some(&rgba_to_css(&initial_rgba)));
            value_label.add_css_class("dim-label");
            value_label.add_css_class("caption");
            value_label.set_width_chars(24);
            value_label.set_xalign(0.0);
            value_label.set_selectable(true);

            // Single click handler: open ColorDialog with current color
            let overrides_ref = overrides.clone();
            let token_name = token.css_name.to_string();
            let theme_copy = theme;
            let current_for_click = current_rgba.clone();
            let swatch_for_click = swatch.clone();
            let dialog_ref = dialog.clone();
            let vlabel = value_label.clone();
            btn.connect_clicked(move |_| {
                let color_dialog = gtk4::ColorDialog::new();
                color_dialog.set_with_alpha(true);
                let rgba_now = *current_for_click.borrow();
                let overrides_c = overrides_ref.clone();
                let token_c = token_name.clone();
                let current_c = current_for_click.clone();
                let swatch_c = swatch_for_click.clone();
                let vlabel_c = vlabel.clone();
                color_dialog.choose_rgba(
                    Some(&dialog_ref),
                    Some(&rgba_now),
                    gtk4::gio::Cancellable::NONE,
                    move |result| {
                        if let Ok(rgba) = result {
                            *current_c.borrow_mut() = rgba;
                            swatch_c.queue_draw();
                            let css_val = rgba_to_css(&rgba);
                            vlabel_c.set_text(&css_val);
                            overrides_c.borrow_mut().insert(token_c.clone(), css_val);
                            crate::app::apply_theme_with_overrides(theme_copy, &overrides_c.borrow());
                        }
                    },
                );
            });

            row.append(&value_label);
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

/// The only token names that may be overridden. Anything else stored in the
/// DB from an earlier session (e.g. old alias names like accent_bg, bg_chrome)
/// is silently dropped so stale overrides cannot break the alias chains.
const VALID_BASE_TOKENS: &[&str] = &[
    "bg_window", "bg_surface", "bg_elevated", "bg_popover",
    "fg_ui", "fg_content",
    "accent", "accent_fg", "hover_bg",
    "border_soft", "border_hard",
];

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
        if VALID_BASE_TOKENS.contains(&k.as_str()) {
            if let Some(s) = val.as_str() {
                out.insert(k.clone(), s.to_string());
            }
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
