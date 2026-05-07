//! Thin wrapper around `gio::Notification` so callers don't repeat the
//! boilerplate of building / ID-tagging / dispatching a notification.
//!
//! The identifier collapses duplicate notifications in the OS notification
//! centre — scheduled alerts for the same note re-use the same ID so the
//! user sees a single, most-recent toast rather than a stack.

use gtk4::gio;
use gtk4::glib;
use gtk4::prelude::*;
use libadwaita as adw;

/// Send a desktop notification. `id` must be stable per logical event;
/// pass `None` for one-offs (a fresh UUID is allocated).
pub fn send_desktop(app: &adw::Application, id: Option<&str>, title: &str, body: &str) {
    let notif = gio::Notification::new(title);
    notif.set_body(Some(body));
    notif.set_priority(gio::NotificationPriority::Normal);

    let fallback: String;
    let used_id: Option<&str> = match id {
        Some(i) => Some(i),
        None => {
            fallback = format!("pax-{}", glib::uuid_string_random());
            Some(fallback.as_str())
        }
    };

    app.send_notification(used_id, &notif);
}
