//! Image tab: metadata header + Picture + zoom controls.
//!
//! Supports the raster formats GTK knows about (via its built-in pixbuf
//! loaders) plus SVG through librsvg. SVG files additionally expose a
//! Rendered / Source toggle so the user can inspect the underlying XML.
//! First pass is local-filesystem only — remote (SSH) backends decline
//! image previews in `open_image_file`.

use gtk4::prelude::*;
use sourceview5::prelude::*;
use std::cell::Cell;
use std::path::Path;
use std::rc::Rc;

use super::tab_content::ImageTab;

pub const IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "bmp", "ico", "svg"];

const ZOOM_MIN: f64 = 0.1;
const ZOOM_MAX: f64 = 10.0;
pub(crate) const ZOOM_STEP: f64 = 1.25;
const HEADER_MARGIN: i32 = 6;
/// Horizontal padding around the Rendered/Source toggle labels — same
/// approach used in the markdown tab's mode bar.
const MODE_BUTTON_PAD_PX: i32 = 10;

pub fn build_image_tab(path: &Path) -> ImageTab {
    let picture = gtk4::Picture::for_filename(path);
    picture.set_content_fit(gtk4::ContentFit::Contain);

    let paintable = picture.paintable();
    let natural_width = paintable.as_ref().map(|p| p.intrinsic_width()).unwrap_or(0);
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

    let is_svg = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.eq_ignore_ascii_case("svg"))
        .unwrap_or(false);

    // Header strip: metadata on the left, optional mode toggle (SVG
    // only), zoom controls on the right.
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

    // Zoom controls.
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
    let rotate_left_btn = gtk4::Button::from_icon_name("object-rotate-left-symbolic");
    rotate_left_btn.add_css_class("flat");
    rotate_left_btn.set_tooltip_text(Some("Rotate left"));
    let rotate_right_btn = gtk4::Button::from_icon_name("object-rotate-right-symbolic");
    rotate_right_btn.add_css_class("flat");
    rotate_right_btn.set_tooltip_text(Some("Rotate right"));

    // Rendered / Source toggle — inserted before the zoom controls so
    // it's on the left of them, visually grouping "view mode" separately
    // from "zoom level".
    let (mode_bar_opt, source_stack_opt) = if is_svg {
        let mode_bar = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        mode_bar.add_css_class("linked");
        let rendered_btn = gtk4::ToggleButton::new();
        rendered_btn.set_child(Some(&padded_label("Rendered")));
        rendered_btn.set_active(true);
        let source_btn = gtk4::ToggleButton::new();
        source_btn.set_child(Some(&padded_label("Source")));
        source_btn.set_group(Some(&rendered_btn));
        mode_bar.append(&rendered_btn);
        mode_bar.append(&source_btn);
        header.append(&mode_bar);
        Some((rendered_btn, source_btn))
    } else {
        None
    }
    .map(|pair| (pair.0, pair.1))
    .unzip();

    header.append(&rotate_left_btn);
    header.append(&rotate_right_btn);
    header.append(&minus_btn);
    header.append(&reset_btn);
    header.append(&plus_btn);

    // Rendered view (Picture inside a ScrolledWindow for pan / zoom).
    let rendered_scroll = gtk4::ScrolledWindow::new();
    rendered_scroll.set_child(Some(&picture));
    rendered_scroll.set_vexpand(true);
    rendered_scroll.set_hexpand(true);

    // For SVG also build a Source view of the XML, wrapped in a Stack.
    let inner_stack = gtk4::Stack::new();
    inner_stack.set_vexpand(true);
    inner_stack.set_hexpand(true);
    inner_stack.add_named(&rendered_scroll, Some("rendered"));

    if is_svg {
        if let Ok(text) = std::fs::read_to_string(path) {
            let text = super::text_content::displayable_gtk_text(&text);
            let buffer = sourceview5::Buffer::new(None::<&gtk4::TextTagTable>);
            buffer.set_text(text.as_ref());
            if let Some(lang) = sourceview5::LanguageManager::default().language("xml") {
                buffer.set_language(Some(&lang));
            }
            buffer.set_highlight_syntax(true);
            crate::theme::register_sourceview_buffer(&buffer);

            let source_view = sourceview5::View::with_buffer(&buffer);
            source_view.add_css_class("editor-code-view");
            source_view.set_editable(false);
            source_view.set_show_line_numbers(true);
            source_view.set_monospace(true);
            source_view.set_wrap_mode(gtk4::WrapMode::WordChar);

            let source_scroll = gtk4::ScrolledWindow::new();
            source_scroll.set_child(Some(&source_view));
            source_scroll.set_vexpand(true);
            source_scroll.set_hexpand(true);
            inner_stack.add_named(&source_scroll, Some("source"));
        }
    }
    inner_stack.set_visible_child_name("rendered");

    let outer = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    outer.set_vexpand(true);
    outer.set_hexpand(true);
    outer.append(&header);
    outer.append(&inner_stack);

    let tab = ImageTab {
        picture: picture.clone(),
        path: Rc::new(std::cell::RefCell::new(path.to_path_buf())),
        natural_width,
        natural_height,
        zoom: zoom.clone(),
        rotation_degrees: Rc::new(Cell::new(0)),
        reset_button: reset_btn.clone(),
        outer: outer.upcast::<gtk4::Widget>(),
    };

    // Wire toggle buttons for SVG. Switching to Source hides the zoom
    // controls (they don't apply to text); switching back reveals them.
    if let (Some(rendered_btn), Some(source_btn)) = (mode_bar_opt, source_stack_opt) {
        let stack_c = inner_stack.clone();
        let rotate_left_c = rotate_left_btn.clone();
        let rotate_right_c = rotate_right_btn.clone();
        let minus_c = minus_btn.clone();
        let reset_c = reset_btn.clone();
        let plus_c = plus_btn.clone();
        rendered_btn.connect_toggled(move |btn| {
            if !btn.is_active() {
                return;
            }
            stack_c.set_visible_child_name("rendered");
            rotate_left_c.set_visible(true);
            rotate_right_c.set_visible(true);
            minus_c.set_visible(true);
            reset_c.set_visible(true);
            plus_c.set_visible(true);
        });
        let stack_c2 = inner_stack.clone();
        let rotate_left_c2 = rotate_left_btn.clone();
        let rotate_right_c2 = rotate_right_btn.clone();
        let minus_c2 = minus_btn.clone();
        let reset_c2 = reset_btn.clone();
        let plus_c2 = plus_btn.clone();
        source_btn.connect_toggled(move |btn| {
            if !btn.is_active() {
                return;
            }
            stack_c2.set_visible_child_name("source");
            rotate_left_c2.set_visible(false);
            rotate_right_c2.set_visible(false);
            minus_c2.set_visible(false);
            reset_c2.set_visible(false);
            plus_c2.set_visible(false);
        });
    }

    // Button callbacks delegate to the free functions so keyboard
    // shortcuts and the in-header buttons produce the same side effects.
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
    {
        let tab_c = tab.clone();
        rotate_left_btn.connect_clicked(move |_| rotate_left(&tab_c));
    }
    {
        let tab_c = tab.clone();
        rotate_right_btn.connect_clicked(move |_| rotate_right(&tab_c));
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

pub fn rotate_left(tab: &ImageTab) {
    set_rotation(tab, tab.rotation_degrees.get() - 90);
}

pub fn rotate_right(tab: &ImageTab) {
    set_rotation(tab, tab.rotation_degrees.get() + 90);
}

fn set_zoom(tab: &ImageTab, z: f64) {
    tab.zoom.set(z);
    apply_size_request(tab);
    tab.reset_button
        .set_label(&format!("{}%", (z * 100.0).round() as i32));
}

fn set_rotation(tab: &ImageTab, degrees: i32) {
    tab.rotation_degrees.set(normalize_rotation(degrees));
    apply_picture_rotation(tab);
    apply_size_request(tab);
}

fn apply_picture_rotation(tab: &ImageTab) {
    let rotation = normalize_rotation(tab.rotation_degrees.get());
    let path = tab.path.borrow().clone();
    if rotation == 0 {
        tab.picture.set_filename(Some(&path));
        return;
    }

    match rotated_pixbuf(&path, rotation) {
        Some(pixbuf) => {
            let texture = gtk4::gdk::Texture::for_pixbuf(&pixbuf);
            tab.picture.set_paintable(Some(&texture));
        }
        None => {
            tracing::warn!(
                "image preview: failed to rotate {}; falling back to original",
                path.display()
            );
            tab.picture.set_filename(Some(&path));
        }
    }
}

fn rotated_pixbuf(path: &Path, rotation: i32) -> Option<gtk4::gdk_pixbuf::Pixbuf> {
    let pixbuf = gtk4::gdk_pixbuf::Pixbuf::from_file(path).ok()?;
    pixbuf.rotate_simple(pixbuf_rotation(rotation))
}

fn pixbuf_rotation(rotation: i32) -> gtk4::gdk_pixbuf::PixbufRotation {
    match normalize_rotation(rotation) {
        90 => gtk4::gdk_pixbuf::PixbufRotation::Clockwise,
        180 => gtk4::gdk_pixbuf::PixbufRotation::Upsidedown,
        270 => gtk4::gdk_pixbuf::PixbufRotation::Counterclockwise,
        _ => gtk4::gdk_pixbuf::PixbufRotation::None,
    }
}

fn apply_size_request(tab: &ImageTab) {
    let (base_w, base_h) = rotated_dimensions(
        tab.natural_width,
        tab.natural_height,
        tab.rotation_degrees.get(),
    );
    tab.picture.set_size_request(
        scaled_dimension(base_w, tab.zoom.get()),
        scaled_dimension(base_h, tab.zoom.get()),
    );
}

fn scaled_dimension(size: i32, zoom: f64) -> i32 {
    if size <= 0 {
        0
    } else {
        (size as f64 * zoom).round().max(1.0) as i32
    }
}

fn rotated_dimensions(width: i32, height: i32, rotation: i32) -> (i32, i32) {
    match normalize_rotation(rotation) {
        90 | 270 => (height, width),
        _ => (width, height),
    }
}

fn normalize_rotation(degrees: i32) -> i32 {
    degrees.rem_euclid(360)
}

fn padded_label(text: &str) -> gtk4::Label {
    let l = gtk4::Label::new(Some(text));
    l.set_margin_start(MODE_BUTTON_PAD_PX);
    l.set_margin_end(MODE_BUTTON_PAD_PX);
    l
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

/// Re-point the tab's `Picture` at the (possibly changed) file on disk.
/// Image tabs have no local-edits concept, so this is always a silent reload.
/// `natural_width`/`natural_height`, zoom, and rotation are intentionally left
/// alone: re-querying the paintable here would race the new pixbuf load.
pub fn reload_from_disk(tab: &ImageTab, path: &std::path::Path) {
    *tab.path.borrow_mut() = path.to_path_buf();
    apply_picture_rotation(tab);
    apply_size_request(tab);
}

#[cfg(test)]
mod tests {
    use super::{normalize_rotation, rotated_dimensions, scaled_dimension};

    #[test]
    fn rotation_degrees_are_normalized_to_quadrants() {
        assert_eq!(normalize_rotation(0), 0);
        assert_eq!(normalize_rotation(90), 90);
        assert_eq!(normalize_rotation(450), 90);
        assert_eq!(normalize_rotation(-90), 270);
    }

    #[test]
    fn rotated_dimensions_swap_for_sideways_orientation() {
        assert_eq!(rotated_dimensions(640, 480, 0), (640, 480));
        assert_eq!(rotated_dimensions(640, 480, 90), (480, 640));
        assert_eq!(rotated_dimensions(640, 480, 180), (640, 480));
        assert_eq!(rotated_dimensions(640, 480, 270), (480, 640));
    }

    #[test]
    fn scaled_dimension_keeps_unknown_size_as_zero() {
        assert_eq!(scaled_dimension(0, 2.0), 0);
        assert_eq!(scaled_dimension(10, 1.25), 13);
    }
}
