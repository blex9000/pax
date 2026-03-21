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
}

/// Callback type for panel menu actions.
pub type PanelActionCallback = Rc<dyn Fn(&str, PanelAction)>;

/// Container widget that hosts a PanelBackend with title bar.
pub struct PanelHost {
    outer: gtk4::Box,
    container: gtk4::Box,
    title_label: gtk4::Label,
    menu_button: gtk4::MenuButton,
    widget: gtk4::Widget,
    panel_id: String,
    backend: RefCell<Option<Box<dyn PanelBackend>>>,
    focused: RefCell<bool>,
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

        // Title bar
        let title_bar = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
        title_bar.add_css_class("panel-title-bar");


        let title_label = gtk4::Label::new(Some(name));
        title_label.add_css_class("panel-title");
        title_label.set_halign(gtk4::Align::Start);
        title_label.set_hexpand(true);

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
        title_bar.append(&menu_button);
        container.append(&title_bar);

        let outer = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        outer.append(&container);
        outer.add_css_class("panel-frame");
        outer.set_widget_name(panel_id);
        outer.set_size_request(80, 60);

        let widget = outer.clone().upcast::<gtk4::Widget>();

        Self {
            outer,
            container,
            title_label,
            menu_button,
            widget,
            panel_id: panel_id.to_string(),
            backend: RefCell::new(None),
            focused: RefCell::new(false),
        }
    }

    /// Update the action callback (rebuilds the popover menu).
    pub fn set_action_callback(&self, cb: PanelActionCallback) {
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
        let panel_widget = backend.widget().clone();
        panel_widget.set_vexpand(true);
        panel_widget.set_hexpand(true);
        self.container.append(&panel_widget);
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
