use gtk4::prelude::*;
use gtk4::glib;
use std::cell::RefCell;
use std::rc::Rc;

use crate::panels::PanelBackend;

/// Actions that can be triggered from panel/tab menus.
#[derive(Debug, Clone, Copy, PartialEq)]
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
}

/// Callback type for panel menu actions.
pub type PanelActionCallback = Rc<dyn Fn(&str, PanelAction)>;

/// Container widget that hosts a PanelBackend with title bar.
pub struct PanelHost {
    outer: gtk4::Box,
    container: gtk4::Box,
    title_label: gtk4::Label,
    sync_button: gtk4::Button,
    _zoom_button: gtk4::Button,
    menu_button: gtk4::MenuButton,
    footer_bar: gtk4::Box,
    footer_label: gtk4::Label,
    widget: gtk4::Widget,
    panel_id: String,
    backend: RefCell<Option<Box<dyn PanelBackend>>>,
    focused: RefCell<bool>,
    /// Shared callback ref — updated by set_action_callback, read by button handlers.
    action_cb_ref: Rc<RefCell<Option<PanelActionCallback>>>,
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

        let title_label = gtk4::Label::new(Some(name));
        title_label.add_css_class("panel-title");
        title_label.set_halign(gtk4::Align::Start);
        title_label.set_hexpand(true);

        // Sync button
        let sync_button = gtk4::Button::new();
        sync_button.set_icon_name("chain-link-symbolic");
        sync_button.add_css_class("flat");
        sync_button.add_css_class("panel-action-btn");
        sync_button.set_tooltip_text(Some("Toggle sync input (Ctrl+Shift+S)"));
        {
            let cb_ref = action_cb_ref.clone();
            let pid = panel_id.to_string();
            sync_button.connect_clicked(move |_| {
                if let Some(ref cb) = *cb_ref.borrow() {
                    cb(&pid, PanelAction::Sync);
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
                if let Some(ref cb) = *cb_ref.borrow() {
                    cb(&pid, PanelAction::Zoom);
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

        title_bar.append(&title_label);
        title_bar.append(&sync_button);
        title_bar.append(&zoom_button);
        title_bar.append(&menu_button);
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

        let widget = outer.clone().upcast::<gtk4::Widget>();

        Self {
            outer,
            container,
            title_label,
            sync_button,
            _zoom_button: zoom_button,
            menu_button,
            footer_bar,
            footer_label,
            widget,
            panel_id: panel_id.to_string(),
            backend: RefCell::new(None),
            focused: RefCell::new(false),
            action_cb_ref,
        }
    }

    /// Update the action callback (rebuilds the popover menu; buttons use shared ref automatically).
    pub fn set_action_callback(&self, cb: PanelActionCallback) {
        *self.action_cb_ref.borrow_mut() = Some(cb.clone());
        let popover = build_panel_menu(&self.panel_id, Some(cb));
        self.menu_button.set_popover(Some(&popover));
    }

    /// Set the panel backend, placing its widget inside this host.
    /// If a backend is already set, removes the old widget first.
    pub fn set_backend(&self, backend: Box<dyn PanelBackend>) {
        // Remove old backend widget if present
        if let Some(ref old) = *self.backend.borrow() {
            let old_widget = old.widget().clone();
            self.container.remove(&old_widget);
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

    /// Update sync button visual state.
    pub fn set_sync_active(&self, active: bool) {
        if active {
            self.sync_button.add_css_class("sync-active");
        } else {
            self.sync_button.remove_css_class("sync-active");
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
            PanelAction::Sync => "chain-link-symbolic",
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
        btn.connect_clicked(move |_| {
            pop.popdown(); // Close menu first
            if let Some(ref callback) = cb {
                callback(&id, action);
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
