//! Image tab: metadata header + Picture + zoom controls.
//!
//! Supports the raster formats GTK knows about (via its built-in pixbuf
//! loaders) plus SVG through librsvg. First pass is local-filesystem only
//! — remote (SSH) backends decline image previews in `open_image_file`.

use gtk4::prelude::*;
use std::cell::Cell;
use std::path::Path;
use std::rc::Rc;

use super::tab_content::ImageTab;

pub const IMAGE_EXTS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "webp", "bmp", "ico", "svg",
];

const ZOOM_MIN: f64 = 0.1;
const ZOOM_MAX: f64 = 10.0;
pub(crate) const ZOOM_STEP: f64 = 1.25;
const HEADER_MARGIN: i32 = 6;

pub fn build_image_tab(path: &Path) -> ImageTab {
    let picture = gtk4::Picture::for_filename(path);
    picture.set_content_fit(gtk4::ContentFit::Contain);

    let paintable = picture.paintable();
    let natural_width = paintable
        .as_ref()
        .map(|p| p.intrinsic_width())
        .unwrap_or(0);
    let natural_height = paintable
        .as_ref()
        .map(|p| p.intrinsic_height())
        .unwrap_or(0);

    let size_bytes = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    let format = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_uppercase())
        .unwrap_or_else(|| "?".into());
    let meta_text = format!(
        "{}×{} · {} · {}",
        natural_width,
        natural_height,
        human_size(size_bytes),
        format
    );

    // Header strip: metadata on the left, zoom controls on the right.
    let header = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    header.add_css_class("image-header");
    header.set_margin_start(HEADER_MARGIN);
    header.set_margin_end(HEADER_MARGIN);
    header.set_margin_top(HEADER_MARGIN);
    header.set_margin_bottom(HEADER_MARGIN);

    let meta_label = gtk4::Label::new(Some(&meta_text));
    meta_label.set_halign(gtk4::Align::Start);
    meta_label.set_hexpand(true);
    meta_label.add_css_class("dim-label");
    header.append(&meta_label);

    let zoom = Rc::new(Cell::new(1.0_f64));
    let minus_btn = gtk4::Button::from_icon_name("zoom-out-symbolic");
    minus_btn.add_css_class("flat");
    minus_btn.set_tooltip_text(Some("Zoom out (Ctrl+-)"));
    let reset_btn = gtk4::Button::with_label("100%");
    reset_btn.add_css_class("flat");
    reset_btn.set_tooltip_text(Some("Reset zoom (Ctrl+0)"));
    let plus_btn = gtk4::Button::from_icon_name("zoom-in-symbolic");
    plus_btn.add_css_class("flat");
    plus_btn.set_tooltip_text(Some("Zoom in (Ctrl+=)"));
    header.append(&minus_btn);
    header.append(&reset_btn);
    header.append(&plus_btn);

    // Scrollable area so big images + high zoom levels can pan.
    let scroll = gtk4::ScrolledWindow::new();
    scroll.set_child(Some(&picture));
    scroll.set_vexpand(true);
    scroll.set_hexpand(true);

    let outer = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    outer.set_vexpand(true);
    outer.set_hexpand(true);
    outer.append(&header);
    outer.append(&scroll);

    let tab = ImageTab {
        picture: picture.clone(),
        natural_width,
        natural_height,
        zoom: zoom.clone(),
        reset_button: reset_btn.clone(),
        outer: outer.upcast::<gtk4::Widget>(),
    };

    // Button callbacks delegate to the free functions so keyboard
    // shortcuts (wired in Task 6) and the in-header buttons produce the
    // same side effects.
    {
        let tab_c = tab.clone();
        minus_btn.connect_clicked(move |_| zoom_out(&tab_c));
    }
    {
        let tab_c = tab.clone();
        plus_btn.connect_clicked(move |_| zoom_in(&tab_c));
    }
    {
        let tab_c = tab.clone();
        reset_btn.connect_clicked(move |_| zoom_reset(&tab_c));
    }

    // Apply initial zoom so the size request + "100%" label are consistent
    // even before the user touches a control.
    set_zoom(&tab, 1.0);

    tab
}

pub fn zoom_in(tab: &ImageTab) {
    set_zoom(tab, (tab.zoom.get() * ZOOM_STEP).min(ZOOM_MAX));
}

pub fn zoom_out(tab: &ImageTab) {
    set_zoom(tab, (tab.zoom.get() / ZOOM_STEP).max(ZOOM_MIN));
}

pub fn zoom_reset(tab: &ImageTab) {
    set_zoom(tab, 1.0);
}

fn set_zoom(tab: &ImageTab, z: f64) {
    tab.zoom.set(z);
    let w = (tab.natural_width as f64 * z) as i32;
    let h = (tab.natural_height as f64 * z) as i32;
    tab.picture.set_size_request(w, h);
    tab.reset_button
        .set_label(&format!("{}%", (z * 100.0).round() as i32));
}

fn human_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.2} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}
