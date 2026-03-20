use gtk4::prelude::*;

/// Status bar at the bottom of the window.
#[derive(Debug)]
pub struct StatusBar {
    container: gtk4::Box,
    mode_label: gtk4::Label,
    panel_label: gtk4::Label,
    workspace_label: gtk4::Label,
    message_label: gtk4::Label,
    hints_label: gtk4::Label,
}

impl StatusBar {
    pub fn new(workspace_name: &str) -> Self {
        let container = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        container.add_css_class("status-bar");

        let mode_label = gtk4::Label::new(Some(" NORMAL "));
        mode_label.add_css_class("status-mode");
        container.append(&mode_label);

        let panel_label = gtk4::Label::new(None);
        container.append(&panel_label);

        let workspace_label = gtk4::Label::new(Some(workspace_name));
        workspace_label.set_opacity(0.6);
        container.append(&workspace_label);

        let message_label = gtk4::Label::new(None);
        message_label.set_hexpand(true);
        message_label.set_halign(gtk4::Align::Start);
        container.append(&message_label);

        let hints_label = gtk4::Label::new(Some(
            "C-q:quit  C-n/p:focus  C-z:zoom  C-b:broadcast  C-k:palette",
        ));
        hints_label.set_opacity(0.5);
        hints_label.set_halign(gtk4::Align::End);
        container.append(&hints_label);

        Self {
            container,
            mode_label,
            panel_label,
            workspace_label,
            message_label,
            hints_label,
        }
    }

    pub fn widget(&self) -> &gtk4::Box {
        &self.container
    }

    pub fn set_mode(&self, mode: &str) {
        self.mode_label.set_text(&format!(" {} ", mode));
    }

    pub fn set_panel(&self, panel_id: &str) {
        self.panel_label.set_text(&format!("[{}]", panel_id));
    }

    pub fn set_message(&self, msg: &str) {
        self.message_label.set_text(msg);
    }

    pub fn clear_message(&self) {
        self.message_label.set_text("");
    }
}
