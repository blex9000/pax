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
    /// user clicks its close button. When `on_click` is `Some`, the
    /// toast body (outside the close button) is clickable and invokes
    /// the callback — used to jump to the owning note panel.
    /// `workspace_name`, if present, is rendered as a small subtitle
    /// under the note title so the user can tell where the note lives
    /// (important when the note is in a different workspace than the
    /// current one).
    pub fn push(
        self: &Rc<Self>,
        title: &str,
        body: &str,
        workspace_name: Option<&str>,
        on_click: Option<Rc<dyn Fn()>>,
    ) {
        let toast = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
        toast.add_css_class("alert-toast");
        if on_click.is_some() {
            toast.add_css_class("alert-toast-clickable");
        }
        toast.set_width_request(TOAST_WIDTH_PX);

        let header = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);

        let icon = gtk4::Image::from_icon_name(TOAST_ICON_NAME);
        icon.add_css_class("alert-toast-icon");
        icon.set_pixel_size(16);
        header.append(&icon);

        let title_stack = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        title_stack.set_hexpand(true);
        title_stack.set_valign(gtk4::Align::Center);

        let title_label = gtk4::Label::new(Some(title));
        title_label.add_css_class("heading");
        title_label.add_css_class("alert-toast-title");
        title_label.set_halign(gtk4::Align::Start);
        title_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        title_stack.append(&title_label);

        if let Some(ws) = workspace_name {
            let ws_label = gtk4::Label::new(Some(ws));
            ws_label.add_css_class("caption");
            ws_label.add_css_class("alert-toast-workspace");
            ws_label.set_halign(gtk4::Align::Start);
            ws_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
            title_stack.append(&ws_label);
        }

        header.append(&title_stack);

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

        // Click on the toast body (outside the close button, which
        // claims its own event sequence) invokes the on_click callback.
        // Bubble phase so the close button wins when it's the target.
        if let Some(on_click) = on_click {
            let gesture = gtk4::GestureClick::new();
            gesture.set_button(gtk4::gdk::BUTTON_PRIMARY);
            gesture.connect_released(move |g, _, _, _| {
                g.set_state(gtk4::EventSequenceState::Claimed);
                on_click();
            });
            toast.add_controller(gesture);
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
/// silently if no tray exists (e.g. headless tests). `on_click` is
/// invoked when the user clicks the toast body (outside its close
/// button) — see `AlertTray::push`.
pub fn emit(title: &str, body: &str, workspace_name: Option<&str>, on_click: Option<Rc<dyn Fn()>>) {
    REGISTERED_TRAY.with(|cell| {
        if let Some(tray) = cell.borrow().as_ref() {
            tray.push(title, body, workspace_name, on_click);
        }
    });
}

thread_local! {
    /// Callback invoked when the user clicks an alert toast for a
    /// specific note. Registered by the app once the workspace view is
    /// available; the scheduler only knows note ids and dispatches
    /// through here so concerns stay separated.
    static NOTE_CLICK_HANDLER: RefCell<Option<Rc<dyn Fn(i64)>>> = RefCell::new(None);
}

/// Register the click-to-focus handler. The app installs this after
/// building the workspace view; it receives the note id and is
/// expected to navigate / prompt as appropriate.
pub fn register_note_click_handler(handler: Rc<dyn Fn(i64)>) {
    NOTE_CLICK_HANDLER.with(|c| *c.borrow_mut() = Some(handler));
}

/// Dispatch a note-click event to the registered handler, if any.
/// No-op when nothing is registered (e.g. during startup before the
/// workspace view exists).
pub fn dispatch_note_click(note_id: i64) {
    let handler = NOTE_CLICK_HANDLER.with(|c| c.borrow().clone());
    if let Some(h) = handler {
        h(note_id);
    }
}
