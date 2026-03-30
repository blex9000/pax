use gtk4::prelude::*;
use gtk4::glib;
use std::cell::RefCell;
use std::rc::Rc;

use crate::panels::PanelBackend;

/// Actions that can be triggered from panel/tab menus.
#[derive(Debug, Clone, PartialEq)]
pub enum PanelAction {
    SplitH,
    SplitV,
    AddTab,
    Close,
    /// Configure panel settings
    Configure,
    /// Add a new tab to existing Notebook (from tab bar menu)
    AddTabToNotebook,
    /// Remove current tab from Notebook (from tab bar menu)
    RemoveTab,
    /// Toggle zoom/fullscreen
    Zoom,
    /// Toggle sync input
    Sync,
    /// Rename panel (carries new name)
    Rename(String),
    /// Focus this panel
    Focus,
}

/// Callback type for panel menu actions.
pub type PanelActionCallback = Rc<dyn Fn(&str, PanelAction)>;

/// Container widget that hosts a PanelBackend with title bar.
pub struct PanelHost {
    outer: gtk4::Box,
    container: gtk4::Box,
    _title_bar: gtk4::Box,
    type_icon: gtk4::Image,
    title_label: gtk4::Label,
    sync_button: gtk4::Button,
    zoom_button: gtk4::Button,
    menu_button: gtk4::MenuButton,
    footer_bar: gtk4::Box,
    footer_label: gtk4::Label,
    widget: gtk4::Widget,
    panel_id: String,
    backend: RefCell<Option<Box<dyn PanelBackend>>>,
    focused: RefCell<bool>,
    /// Shared callback ref — updated by set_action_callback, read by button handlers.
    action_cb_ref: Rc<RefCell<Option<PanelActionCallback>>>,
    /// Shared sync commit callback — updated by set_sync_commit_callback, read by VTE commit handler.
    #[cfg(feature = "vte")]
    sync_cb_ref: Rc<RefCell<Option<Rc<dyn Fn(&str, &str)>>>>,
    #[cfg(feature = "vte")]
    sync_connected: std::cell::Cell<bool>,
}

impl std::fmt::Debug for PanelHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PanelHost")
            .field("panel_id", &self.panel_id)
            .finish()
    }
}

impl PanelHost {
    pub fn new(panel_id: &str, name: &str, action_cb: Option<PanelActionCallback>) -> Self {
        let container = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        let action_cb_ref: Rc<RefCell<Option<PanelActionCallback>>> = Rc::new(RefCell::new(action_cb.clone()));

        // Title bar
        let title_bar = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
        title_bar.add_css_class("panel-title-bar");

        // Panel type icon
        let type_icon = gtk4::Image::from_icon_name("radio-symbolic"); // default: empty/chooser dot
        type_icon.add_css_class("panel-type-icon");

        // Title: stack with label (view) and entry (edit), double-click to rename
        let title_stack = gtk4::Stack::new();
        title_stack.set_halign(gtk4::Align::Start);
        title_stack.set_hexpand(true);

        let title_label = gtk4::Label::new(Some(name));
        title_label.add_css_class("panel-title");
        title_label.set_halign(gtk4::Align::Start);
        title_stack.add_named(&title_label, Some("label"));

        let title_entry = gtk4::Entry::new();
        title_entry.set_text(name);
        title_entry.add_css_class("panel-title-edit");
        title_stack.add_named(&title_entry, Some("entry"));

        title_stack.set_visible_child_name("label");

        // Double-click on title label → switch to edit mode
        {
            let stack = title_stack.clone();
            let entry = title_entry.clone();
            let label = title_label.clone();
            let gesture = gtk4::GestureClick::new();
            gesture.set_button(1);
            gesture.set_propagation_phase(gtk4::PropagationPhase::Bubble);
            gesture.connect_released(move |g, n_press, _, _| {
                if n_press == 2 {
                    entry.set_text(&label.text());
                    stack.set_visible_child_name("entry");
                    entry.grab_focus();
                    g.set_state(gtk4::EventSequenceState::Claimed);
                }
                // Single clicks: don't claim — let them propagate to window (for fullscreen etc.)
            });
            title_stack.add_controller(gesture);
        }

        // Enter on entry → confirm rename
        {
            let stack = title_stack.clone();
            let label = title_label.clone();
            let cb_ref = action_cb_ref.clone();
            let pid = panel_id.to_string();
            title_entry.connect_activate(move |entry| {
                let new_name = entry.text().to_string();
                if !new_name.trim().is_empty() {
                    label.set_text(&new_name);
                    // Notify via action callback — use Configure with the new name
                    if let Ok(borrowed) = cb_ref.try_borrow() {
                        if let Some(ref cb) = *borrowed {
                            cb(&pid, PanelAction::Rename(new_name));
                        }
                    }
                }
                stack.set_visible_child_name("label");
            });
        }

        // Escape on entry → cancel
        {
            let stack = title_stack.clone();
            let key_ctrl = gtk4::EventControllerKey::new();
            key_ctrl.connect_key_pressed(move |_, key, _, _| {
                if key == gtk4::gdk::Key::Escape {
                    stack.set_visible_child_name("label");
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
            });
            title_entry.add_controller(key_ctrl);
        }

        // Sync button
        let sync_button = gtk4::Button::new();
        sync_button.set_icon_name("media-playlist-consecutive-symbolic");
        sync_button.add_css_class("flat");
        sync_button.add_css_class("panel-action-btn");
        sync_button.set_tooltip_text(Some("Toggle sync input (Ctrl+Shift+S)"));
        {
            let cb_ref = action_cb_ref.clone();
            let pid = panel_id.to_string();
            sync_button.connect_clicked(move |_| {
                if let Ok(borrowed) = cb_ref.try_borrow() {
                    if let Some(ref cb) = *borrowed {
                        cb(&pid, PanelAction::Sync);
                    }
                }
            });
        }

        // Zoom button
        let zoom_button = gtk4::Button::new();
        zoom_button.set_icon_name("view-fullscreen-symbolic");
        zoom_button.add_css_class("flat");
        zoom_button.add_css_class("panel-action-btn");
        zoom_button.set_tooltip_text(Some("Toggle zoom (Ctrl+Z)"));
        {
            let cb_ref = action_cb_ref.clone();
            let pid = panel_id.to_string();
            zoom_button.connect_clicked(move |_| {
                if let Ok(borrowed) = cb_ref.try_borrow() {
                    if let Some(ref cb) = *borrowed {
                        cb(&pid, PanelAction::Zoom);
                    }
                }
            });
        }

        // ⋮ menu button
        let menu_button = gtk4::MenuButton::new();
        menu_button.set_icon_name("view-more-symbolic");
        menu_button.add_css_class("flat");
        menu_button.add_css_class("panel-menu-btn");
        menu_button.set_tooltip_text(Some("Panel actions"));

        // Build popover menu
        let popover = build_panel_menu(panel_id, action_cb);
        menu_button.set_popover(Some(&popover));

        title_bar.append(&type_icon);
        title_bar.append(&title_stack);
        // Spacer pushes buttons to the right (even when title is hidden)
        let spacer = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        title_bar.append(&spacer);
        title_bar.append(&sync_button);
        title_bar.append(&zoom_button);
        title_bar.append(&menu_button);

        // Click on title bar → focus this panel
        {
            let cb_ref = action_cb_ref.clone();
            let pid = panel_id.to_string();
            let gesture = gtk4::GestureClick::new();
            gesture.set_button(1);
            gesture.set_propagation_phase(gtk4::PropagationPhase::Bubble);
            gesture.connect_released(move |_, n_press, _, _| {
                if n_press == 1 {
                    if let Ok(borrowed) = cb_ref.try_borrow() {
                        if let Some(ref cb) = *borrowed {
                            cb(&pid, PanelAction::Focus);
                        }
                    }
                }
            });
            title_bar.add_controller(gesture);
        }

        container.append(&title_bar);

        // Footer bar
        let footer_bar = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
        footer_bar.add_css_class("panel-footer-bar");
        let footer_label = gtk4::Label::new(None);
        footer_label.set_use_markup(true);
        footer_label.add_css_class("panel-footer");
        footer_label.set_halign(gtk4::Align::Start);
        footer_label.set_hexpand(true);
        footer_label.set_ellipsize(gtk4::pango::EllipsizeMode::Middle);
        footer_label.set_margin_start(6);
        footer_bar.append(&footer_label);
        footer_bar.set_visible(false); // Hidden until content is set

        let outer = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        outer.append(&container);
        outer.append(&footer_bar);
        outer.add_css_class("panel-frame");
        outer.set_widget_name(panel_id);
        outer.set_size_request(80, 60);

        // Click anywhere in the panel → focus it
        {
            let cb_ref = action_cb_ref.clone();
            let pid = panel_id.to_string();
            let gesture = gtk4::GestureClick::new();
            gesture.set_button(1);
            gesture.set_propagation_phase(gtk4::PropagationPhase::Capture);
            gesture.connect_pressed(move |_, _, _, _| {
                if let Ok(borrowed) = cb_ref.try_borrow() {
                    if let Some(ref cb) = *borrowed {
                        cb(&pid, PanelAction::Focus);
                    }
                }
                // Don't claim — let the click propagate to the content (VTE, TextView, etc.)
            });
            outer.add_controller(gesture);
        }

        let widget = outer.clone().upcast::<gtk4::Widget>();

        Self {
            outer,
            container,
            _title_bar: title_bar,
            type_icon,
            title_label,
            sync_button,
            zoom_button,
            menu_button,
            footer_bar,
            footer_label,
            widget,
            panel_id: panel_id.to_string(),
            backend: RefCell::new(None),
            focused: RefCell::new(false),
            action_cb_ref,
            #[cfg(feature = "vte")]
            sync_cb_ref: Rc::new(RefCell::new(None)),
            #[cfg(feature = "vte")]
            sync_connected: std::cell::Cell::new(false),
        }
    }

    /// Update the action callback (rebuilds the popover menu; buttons use shared ref automatically).
    pub fn set_action_callback(&self, cb: PanelActionCallback) {
        // Use try_borrow_mut to avoid panic if called during a button click handler
        if let Ok(mut r) = self.action_cb_ref.try_borrow_mut() {
            *r = Some(cb.clone());
        }
        let popover = build_panel_menu(&self.panel_id, Some(cb));
        self.menu_button.set_popover(Some(&popover));
    }

    /// Set the panel backend, placing its widget inside this host.
    /// If a backend is already set, removes the old widget first.
    pub fn set_backend(&self, backend: Box<dyn PanelBackend>) {
        // Remove old backend widget if present
        {
            if let Ok(current) = self.backend.try_borrow() {
                if let Some(ref old) = *current {
                    let old_widget = old.widget().clone();
                    drop(current); // Release borrow before remove
                    self.container.remove(&old_widget);
                }
            }
        }
        self.footer_bar.set_visible(false);

        let panel_widget = backend.widget().clone();
        panel_widget.set_vexpand(true);
        panel_widget.set_hexpand(true);
        self.container.append(&panel_widget);

        // If this is a VTE terminal, connect directory tracking
        #[cfg(feature = "vte")]
        self.setup_vte_directory_tracking(&panel_widget);

        *self.backend.borrow_mut() = Some(backend);
    }

    pub fn widget(&self) -> &gtk4::Widget {
        &self.widget
    }

    pub fn panel_id(&self) -> &str {
        &self.panel_id
    }

    pub fn set_focused(&self, focused: bool) {
        *self.focused.borrow_mut() = focused;
        if focused {
            self.outer.add_css_class("panel-focused");
            self.outer.remove_css_class("panel-unfocused");
            // Visual focus indicator removed — keeping it clean
            if let Some(ref backend) = *self.backend.borrow() {
                backend.on_focus();
            }
        } else {
            self.outer.remove_css_class("panel-focused");
            self.outer.add_css_class("panel-unfocused");
            if let Some(ref backend) = *self.backend.borrow() {
                backend.on_blur();
            }
        }
    }

    pub fn set_alert_border(&self, color: &str) {
        self.outer.remove_css_class("alert-red");
        self.outer.remove_css_class("alert-yellow");
        self.outer.remove_css_class("alert-green");
        self.outer.add_css_class(&format!("alert-{}", color));
    }

    pub fn clear_alert_border(&self) {
        self.outer.remove_css_class("alert-red");
        self.outer.remove_css_class("alert-yellow");
        self.outer.remove_css_class("alert-green");
    }

    pub fn set_title(&self, title: &str) {
        self.title_label.set_text(title);
    }

    /// Hide/show the title name and icon (when panel is inside a tab — already shown there).
    /// Keeps the action buttons (sync, zoom, menu) visible.
    pub fn set_title_visible(&self, visible: bool) {
        self.type_icon.set_visible(visible);
        // Hide the title stack (contains label + entry)
        if let Some(parent) = self.title_label.parent() {
            parent.set_visible(visible);
        }
    }

    /// Update the panel type icon.
    pub fn set_type_icon(&self, panel_type: &str) {
        let icon_name = match panel_type {
            "terminal" => "utilities-terminal-symbolic",
            "markdown" => "document-properties-symbolic",
            "browser" => "web-browser-symbolic",
            _ => "radio-symbolic", // Empty/chooser — dot
        };
        self.type_icon.set_icon_name(Some(icon_name));
    }

    /// Update zoom button visual state and icon.
    pub fn set_zoom_active(&self, active: bool) {
        if active {
            self.zoom_button.set_icon_name("view-restore-symbolic");
            self.zoom_button.add_css_class("zoom-active");
        } else {
            self.zoom_button.set_icon_name("view-fullscreen-symbolic");
            self.zoom_button.remove_css_class("zoom-active");
        }
    }

    /// Update sync button visual state.
    pub fn set_sync_active(&self, active: bool) {
        if active {
            self.sync_button.add_css_class("sync-active");
        } else {
            self.sync_button.remove_css_class("sync-active");
        }
    }

    /// Set the sync commit callback. The VTE commit handler is connected only once;
    /// subsequent calls just update the shared callback ref.
    #[cfg(feature = "vte")]
    pub fn set_sync_commit_callback(&self, cb: Rc<dyn Fn(&str, &str)>, propagating: Rc<std::cell::Cell<bool>>) {
        // Update the shared callback
        *self.sync_cb_ref.borrow_mut() = Some(cb);

        // Connect the VTE handler only once
        if self.sync_connected.get() {
            return;
        }

        use vte4::prelude::*;
        let panel_widget = {
            let backend = self.backend.borrow();
            backend.as_ref().map(|b| b.widget().clone())
        };
        if let Some(widget) = panel_widget {
            if let Ok(vte) = widget.clone().downcast::<vte4::Terminal>() {
                let pid = self.panel_id.clone();
                let flag = propagating;
                let cb_ref = self.sync_cb_ref.clone();
                vte.connect_commit(move |_vte, text, _size| {
                    if flag.get() {
                        return;
                    }
                    if let Ok(borrowed) = cb_ref.try_borrow() {
                        if let Some(ref cb) = *borrowed {
                            flag.set(true);
                            cb(&pid, text);
                            flag.set(false);
                        }
                    }
                });
                self.sync_connected.set(true);
            }
        }
    }

    /// Set footer text (e.g. user@host:directory). Empty string hides the footer.
    pub fn set_footer(&self, text: &str) {
        if text.is_empty() {
            self.footer_bar.set_visible(false);
        } else {
            self.footer_label.set_text(text);
            self.footer_label.set_tooltip_text(Some(text));
            self.footer_bar.set_visible(true);
        }
    }

    /// Connect VTE current-directory-uri signal to update the footer.
    #[cfg(feature = "vte")]
    fn setup_vte_directory_tracking(&self, widget: &gtk4::Widget) {
        use vte4::prelude::*;
        if let Ok(vte) = widget.clone().downcast::<vte4::Terminal>() {
            let footer = self.footer_label.clone();
            let footer_bar = self.footer_bar.clone();
            let user = std::env::var("USER").unwrap_or_default();
            let hostname = std::env::var("HOSTNAME")
                .or_else(|_| std::fs::read_to_string("/etc/hostname").map(|s| s.trim().to_string()))
                .unwrap_or_else(|_| "localhost".to_string());
            vte.connect_current_directory_uri_changed(move |vte| {
                if let Some(uri) = vte.current_directory_uri() {
                    // URI format: file://hostname/path/to/dir
                    // Parse with url crate logic: strip scheme, then hostname
                    let after_scheme = uri.strip_prefix("file://").unwrap_or(&uri);
                    // Find the first '/' after hostname — that starts the absolute path
                    let path = if let Some(slash_pos) = after_scheme.find('/') {
                        &after_scheme[slash_pos..]
                    } else {
                        after_scheme
                    };
                    // URL-decode %XX sequences
                    let path = percent_decode(path);
                    // Abbreviate home dir
                    let home = std::env::var("HOME").unwrap_or_default();
                    let display_path = if !home.is_empty() && path.starts_with(&home) {
                        format!("~{}", &path[home.len()..])
                    } else {
                        path
                    };
                    let plain = format!("{}@{}:{}", user, hostname, display_path);
                    let markup = format!(
                        "<span color='#33cc33'>{}@{}</span>:<span color='#5588ff'>{}</span>",
                        glib::markup_escape_text(&user),
                        glib::markup_escape_text(&hostname),
                        glib::markup_escape_text(&display_path),
                    );
                    footer.set_markup(&markup);
                    footer.set_tooltip_text(Some(&plain));
                    footer_bar.set_visible(true);
                }
            });
        }
    }

    pub fn write_input(&self, data: &[u8]) -> bool {
        if let Some(ref backend) = *self.backend.borrow() {
            backend.write_input(data)
        } else {
            false
        }
    }

    pub fn accepts_input(&self) -> bool {
        if let Some(ref backend) = *self.backend.borrow() {
            backend.accepts_input()
        } else {
            false
        }
    }
}

/// Build the ⋮ popover menu with panel actions.
fn build_panel_menu(panel_id: &str, action_cb: Option<PanelActionCallback>) -> gtk4::Popover {
    let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
    vbox.set_margin_top(4);
    vbox.set_margin_bottom(4);
    vbox.set_margin_start(4);
    vbox.set_margin_end(4);

    let items: Vec<(&str, &str, PanelAction)> = vec![
        ("Configure…", "Panel settings", PanelAction::Configure),
        ("Split Horizontal", "New panel below", PanelAction::SplitH),
        ("Split Vertical", "New panel to the right", PanelAction::SplitV),
        ("Add Tab", "New panel as tab", PanelAction::AddTab),
        ("Close Panel", "Close this panel", PanelAction::Close),
    ];

    let popover = gtk4::Popover::new();
    let pid = panel_id.to_string();

    for (label, tooltip, action) in items {
        let btn = gtk4::Button::new();
        btn.add_css_class("flat");
        btn.set_tooltip_text(Some(tooltip));

        let btn_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        let icon_name = match action {
            PanelAction::Configure => "preferences-system-symbolic",
            PanelAction::SplitH => "view-dual-symbolic",
            PanelAction::SplitV => "view-dual-symbolic",
            PanelAction::AddTab => "tab-new-symbolic",
            PanelAction::Close => "window-close-symbolic",
            PanelAction::AddTabToNotebook => "tab-new-symbolic",
            PanelAction::RemoveTab => "window-close-symbolic",
            PanelAction::Zoom => "view-fullscreen-symbolic",
            PanelAction::Sync => "media-playlist-consecutive-symbolic",
            PanelAction::Rename(_) => "document-edit-symbolic",
            PanelAction::Focus => "radio-symbolic",
        };
        let icon = gtk4::Image::from_icon_name(icon_name);
        let lbl = gtk4::Label::new(Some(label));
        lbl.set_halign(gtk4::Align::Start);
        lbl.set_hexpand(true);

        // Add shortcut hint
        let hint_text = match action {
            PanelAction::Configure => "",
            PanelAction::SplitH => "Ctrl+Shift+H",
            PanelAction::SplitV => "Ctrl+Shift+J",
            PanelAction::AddTab => "Ctrl+Shift+T",
            PanelAction::Close => "Ctrl+Shift+W",
            PanelAction::AddTabToNotebook => "",
            PanelAction::RemoveTab => "",
            PanelAction::Zoom => "Ctrl+Z",
            PanelAction::Sync => "Ctrl+Shift+S",
            PanelAction::Rename(_) => "Dbl-click",
            PanelAction::Focus => "",
        };
        let hint = gtk4::Label::new(Some(hint_text));
        hint.add_css_class("dim-label");
        hint.set_halign(gtk4::Align::End);

        btn_box.append(&icon);
        btn_box.append(&lbl);
        btn_box.append(&hint);
        btn.set_child(Some(&btn_box));

        let cb = action_cb.clone();
        let id = pid.clone();
        let pop = popover.clone();
        let act = action.clone();
        btn.connect_clicked(move |_| {
            pop.popdown(); // Close menu first
            if let Some(ref callback) = cb {
                callback(&id, act.clone());
            }
        });

        vbox.append(&btn);

        // Add separator after Configure
        if action == PanelAction::Configure {
            let sep = gtk4::Separator::new(gtk4::Orientation::Horizontal);
            sep.set_margin_top(4);
            sep.set_margin_bottom(4);
            vbox.append(&sep);
        }
    }

    popover.set_child(Some(&vbox));
    popover
}

/// Decode percent-encoded URI path (e.g. %20 → space).
fn percent_decode(s: &str) -> String {
    let mut result = Vec::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(byte) = u8::from_str_radix(
                std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or(""),
                16,
            ) {
                result.push(byte);
                i += 3;
                continue;
            }
        }
        result.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(result).unwrap_or_else(|_| s.to_string())
}
