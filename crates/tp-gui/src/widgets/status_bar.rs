use gtk4::prelude::*;
use gtk4::glib;

/// Status bar at the bottom of the window.
#[derive(Debug)]
pub struct StatusBar {
    container: gtk4::Box,
    path_label: gtk4::Label,
    message_label: gtk4::Label,
    #[allow(dead_code)]
    ram_label: gtk4::Label,
}

impl StatusBar {
    pub fn new() -> Self {
        let container = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        container.add_css_class("status-bar");

        let path_label = gtk4::Label::new(Some("(unsaved)"));
        path_label.set_halign(gtk4::Align::Start);
        path_label.set_ellipsize(gtk4::pango::EllipsizeMode::Start);
        path_label.set_opacity(0.6);
        path_label.add_css_class("caption");
        container.append(&path_label);

        let message_label = gtk4::Label::new(None);
        message_label.set_hexpand(true);
        message_label.set_halign(gtk4::Align::End);
        message_label.set_opacity(0.7);
        message_label.add_css_class("caption");
        container.append(&message_label);

        let ram_label = gtk4::Label::new(None);
        ram_label.set_halign(gtk4::Align::End);
        ram_label.set_opacity(0.5);
        ram_label.add_css_class("caption");
        container.append(&ram_label);

        // Update RAM usage every 2 seconds
        {
            let label = ram_label.clone();
            // Set initial value
            label.set_text(&format_ram_usage());
            glib::timeout_add_local(std::time::Duration::from_secs(2), move || {
                label.set_text(&format_ram_usage());
                glib::ControlFlow::Continue
            });
        }

        Self {
            container,
            path_label,
            message_label,
            ram_label,
        }
    }

    pub fn widget(&self) -> &gtk4::Box {
        &self.container
    }

    pub fn set_path(&self, path: &str) {
        self.path_label.set_text(path);
        self.path_label.set_tooltip_text(Some(path));
    }

    pub fn set_message(&self, msg: &str) {
        self.message_label.set_text(msg);
    }

    pub fn clear_message(&self) {
        self.message_label.set_text("");
    }

    // Keep for compatibility
    pub fn set_panel(&self, _panel_id: &str) {}
}

/// Read RSS (Resident Set Size) from /proc/self/status and format as MB.
fn format_ram_usage() -> String {
    if let Ok(status) = std::fs::read_to_string("/proc/self/status") {
        for line in status.lines() {
            if line.starts_with("VmRSS:") {
                // Format: "VmRSS:    12345 kB"
                let parts: Vec<&str> = line.split_whitespace().collect();
                if let Some(kb_str) = parts.get(1) {
                    if let Ok(kb) = kb_str.parse::<f64>() {
                        let mb = kb / 1024.0;
                        return format!("{:.1} MB", mb);
                    }
                }
            }
        }
    }
    String::new()
}
