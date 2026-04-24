//! In-app alert toasts anchored top-right of the main window.
//!
//! Shown in addition to the OS desktop notification for every fired
//! scheduled alert. Unlike desktop notifications, these stay visible
//! until the user dismisses them (they never auto-hide) and stack if
//! multiple alerts are in flight.

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::prelude::*;

/// Max width of a single toast in the tray. Narrow enough to fit a
/// workspace sidebar, wide enough for two lines of preview.
const TOAST_WIDTH_PX: i32 = 320;
/// Gap between stacked toasts (same variable used on the container Box
/// to keep toasts visually distinct but grouped).
const TOAST_STACK_GAP_PX: i32 = 6;
/// Icon shown on the leading edge of each toast. Matches the panel
/// registry's icon for the "note" panel type so the alert visually
/// reads as "a note is calling you".
const TOAST_ICON_NAME: &str = "text-editor-symbolic";

thread_local! {
    /// Global singleton — registered once when the main window builds
    /// its overlay. The alert scheduler (no access to the window) looks
    /// it up here when emitting a toast. Single-threaded because GTK
    /// runs on one thread.
    static REGISTERED_TRAY: RefCell<Option<Rc<AlertTray>>> = RefCell::new(None);
}

/// The tray widget: a vertical Box into which toast widgets are
/// prepended (newest on top). Anchored in a GtkOverlay at top-right.
pub struct AlertTray {
    container: gtk4::Box,
}

impl AlertTray {
    pub fn new() -> Rc<Self> {
        let container = gtk4::Box::new(gtk4::Orientation::Vertical, TOAST_STACK_GAP_PX);
        container.add_css_class("alert-tray");
        container.set_halign(gtk4::Align::End);
        container.set_valign(gtk4::Align::Start);
        container.set_margin_top(12);
        container.set_margin_end(12);
        // The tray itself is invisible when empty; each toast supplies
        // its own background. `set_can_target(false)` on the empty box
        // avoids stealing pointer events from the content below, but
        // we still want it enabled so the toasts can be clicked — the
        // container is cheap to overlap.
        Rc::new(Self { container })
    }

    pub fn widget(&self) -> &gtk4::Widget {
        self.container.upcast_ref()
    }

    /// Push a new toast onto the top of the stack. Persists until the
    /// user clicks its close button.
    pub fn push(self: &Rc<Self>, title: &str, body: &str) {
        let toast = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
        toast.add_css_class("alert-toast");
        toast.set_width_request(TOAST_WIDTH_PX);

        let header = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);

        let icon = gtk4::Image::from_icon_name(TOAST_ICON_NAME);
        icon.add_css_class("alert-toast-icon");
        icon.set_pixel_size(16);
        header.append(&icon);

        let title_label = gtk4::Label::new(Some(title));
        title_label.add_css_class("heading");
        title_label.add_css_class("alert-toast-title");
        title_label.set_halign(gtk4::Align::Start);
        title_label.set_hexpand(true);
        title_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        header.append(&title_label);

        let close_btn = gtk4::Button::from_icon_name("window-close-symbolic");
        close_btn.add_css_class("flat");
        close_btn.add_css_class("alert-toast-close");
        close_btn.set_tooltip_text(Some("Dismiss"));
        header.append(&close_btn);

        let body_label = gtk4::Label::new(Some(body));
        body_label.add_css_class("alert-toast-body");
        body_label.set_halign(gtk4::Align::Start);
        body_label.set_xalign(0.0);
        body_label.set_wrap(true);
        body_label.set_wrap_mode(gtk4::pango::WrapMode::WordChar);

        toast.append(&header);
        toast.append(&body_label);

        {
            let container = self.container.clone();
            let toast_for_remove = toast.clone();
            close_btn.connect_clicked(move |_| {
                container.remove(&toast_for_remove);
            });
        }

        // Newest on top.
        self.container.prepend(&toast);
    }
}

/// Register a tray as the global sink for in-app alerts. Called once
/// when the main window builds its overlay.
pub fn register_global(tray: Rc<AlertTray>) {
    REGISTERED_TRAY.with(|cell| *cell.borrow_mut() = Some(tray));
}

/// Push an alert into whichever tray is currently registered. Drops
/// silently if no tray exists (e.g. headless tests).
pub fn emit(title: &str, body: &str) {
    REGISTERED_TRAY.with(|cell| {
        if let Some(tray) = cell.borrow().as_ref() {
            tray.push(title, body);
        }
    });
}
