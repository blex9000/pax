//! Modal dialog for creating or editing a note with all fields exposed:
//! markdown source, comma-separated tags, severity, and an optional
//! scheduled alert. Inline card edit stays as a quick text-only path;
//! this dialog is the canonical way to change tags / severity / alert.

use std::rc::Rc;

use gtk4::prelude::*;

use pax_db::workspace_notes::{WorkspaceNote, SEVERITY_IMPORTANT, SEVERITY_INFO, SEVERITY_WARNING};

const SEVERITY_DISPLAY: &[(&str, &str)] = &[
    (SEVERITY_INFO, "Info"),
    (SEVERITY_WARNING, "Warning"),
    (SEVERITY_IMPORTANT, "Important"),
];

const DEFAULT_ALERT_HOUR: i32 = 9;
const DEFAULT_ALERT_MINUTE: i32 = 0;

/// Values captured from the dialog on save. The caller (list.rs) is
/// responsible for persisting — the dialog doesn't touch the database.
#[derive(Debug, Clone)]
pub struct NoteDraft {
    pub title: String,
    pub text: String,
    pub tags: Vec<String>,
    pub severity: String,
    pub alert_at: Option<i64>,
}

/// Open the dialog. `initial` supplies the starting values (use defaults
/// for a new note); `on_save` is invoked with the captured draft when the
/// user confirms.
pub fn open_note_dialog(
    parent: &gtk4::Window,
    title: &str,
    initial: NoteDraft,
    on_save: Rc<dyn Fn(NoteDraft)>,
) {
    let dialog = gtk4::Window::builder()
        .title(title)
        .transient_for(parent)
        .modal(true)
        .default_width(520)
        .default_height(560)
        .build();
    crate::theme::configure_dialog_window(&dialog);

    let root = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
    root.set_margin_top(12);
    root.set_margin_bottom(12);
    root.set_margin_start(14);
    root.set_margin_end(14);

    // Title.
    let title_label = gtk4::Label::new(Some("Title"));
    title_label.set_halign(gtk4::Align::Start);
    title_label.add_css_class("dim-label");
    root.append(&title_label);

    let title_entry = gtk4::Entry::new();
    title_entry.set_placeholder_text(Some("Untitled"));
    title_entry.set_text(&initial.title);
    root.append(&title_entry);

    // Markdown text.
    let text_label = gtk4::Label::new(Some("Text (markdown)"));
    text_label.set_halign(gtk4::Align::Start);
    text_label.add_css_class("dim-label");
    text_label.set_margin_top(4);
    root.append(&text_label);

    let text_scroll = gtk4::ScrolledWindow::new();
    text_scroll.set_min_content_height(180);
    text_scroll.set_vexpand(true);
    let text_view = gtk4::TextView::new();
    text_view.set_wrap_mode(gtk4::WrapMode::WordChar);
    text_view.set_monospace(true);
    text_view.set_left_margin(6);
    text_view.set_right_margin(6);
    text_view.set_top_margin(4);
    text_view.set_bottom_margin(4);
    text_view.buffer().set_text(&initial.text);
    text_scroll.set_child(Some(&text_view));
    root.append(&text_scroll);

    // Tags (comma-separated entry).
    let tags_label = gtk4::Label::new(Some("Tags (comma-separated)"));
    tags_label.set_halign(gtk4::Align::Start);
    tags_label.add_css_class("dim-label");
    tags_label.set_margin_top(4);
    root.append(&tags_label);

    let tags_entry = gtk4::Entry::new();
    tags_entry.set_placeholder_text(Some("todo, urgent, api"));
    tags_entry.set_text(&initial.tags.join(", "));
    root.append(&tags_entry);

    // Severity dropdown.
    let severity_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);
    severity_row.set_margin_top(4);
    let severity_label = gtk4::Label::new(Some("Severity"));
    severity_label.set_halign(gtk4::Align::Start);
    severity_label.add_css_class("dim-label");
    severity_label.set_width_chars(10);
    severity_row.append(&severity_label);

    let severity_display_labels: Vec<&str> =
        SEVERITY_DISPLAY.iter().map(|(_, label)| *label).collect();
    let severity_dropdown = gtk4::DropDown::from_strings(&severity_display_labels);
    let initial_idx = SEVERITY_DISPLAY
        .iter()
        .position(|(id, _)| *id == initial.severity)
        .unwrap_or(0);
    severity_dropdown.set_selected(initial_idx as u32);
    severity_row.append(&severity_dropdown);
    root.append(&severity_row);

    // Scheduled alert.
    let alert_toggle = gtk4::CheckButton::with_label("Schedule an alert");
    alert_toggle.set_margin_top(6);
    root.append(&alert_toggle);

    let alert_box = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
    alert_box.set_margin_start(22);
    alert_box.set_visible(false);

    let calendar = gtk4::Calendar::new();
    alert_box.append(&calendar);

    let time_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    time_row.append(&gtk4::Label::new(Some("Time:")));
    let hour_spin = gtk4::SpinButton::with_range(0.0, 23.0, 1.0);
    hour_spin.set_digits(0);
    hour_spin.set_orientation(gtk4::Orientation::Horizontal);
    hour_spin.set_wrap(true);
    let minute_spin = gtk4::SpinButton::with_range(0.0, 59.0, 1.0);
    minute_spin.set_digits(0);
    minute_spin.set_orientation(gtk4::Orientation::Horizontal);
    minute_spin.set_wrap(true);
    time_row.append(&hour_spin);
    time_row.append(&gtk4::Label::new(Some(":")));
    time_row.append(&minute_spin);
    alert_box.append(&time_row);

    let alert_warning = gtk4::Label::new(Some("⚠ Alert time is in the past"));
    alert_warning.add_css_class("error");
    alert_warning.set_halign(gtk4::Align::Start);
    alert_warning.set_visible(false);
    alert_box.append(&alert_warning);

    root.append(&alert_box);

    // Seed calendar/time from initial.alert_at if present.
    if let Some(ts) = initial.alert_at {
        apply_alert_ts_to_widgets(ts, &calendar, &hour_spin, &minute_spin);
        alert_toggle.set_active(true);
        alert_box.set_visible(true);
    } else {
        // Default pick: today at 09:00 if/when the user opts in.
        hour_spin.set_value(DEFAULT_ALERT_HOUR as f64);
        minute_spin.set_value(DEFAULT_ALERT_MINUTE as f64);
    }

    // Action row — constructed before the validity wiring below so the
    // save button is available to enable/disable on validation changes.
    let btn_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    btn_row.set_halign(gtk4::Align::End);
    btn_row.set_margin_top(8);
    let cancel_btn = gtk4::Button::with_label("Cancel");
    let save_btn = gtk4::Button::with_label("Save");
    save_btn.add_css_class("suggested-action");
    btn_row.append(&cancel_btn);
    btn_row.append(&save_btn);
    root.append(&btn_row);

    // Validation: if the alert toggle is on and the composed timestamp is
    // before "now", disable Save and show the warning. Re-evaluated on
    // every relevant change (toggle, calendar day, hour, minute).
    let update_alert_validity: Rc<dyn Fn()> = {
        let alert_toggle = alert_toggle.clone();
        let calendar = calendar.clone();
        let hour_spin = hour_spin.clone();
        let minute_spin = minute_spin.clone();
        let alert_warning = alert_warning.clone();
        let save_btn = save_btn.clone();
        Rc::new(move || {
            if !alert_toggle.is_active() {
                alert_warning.set_visible(false);
                save_btn.set_sensitive(true);
                return;
            }
            let now = chrono::Local::now().timestamp();
            let in_past = match compose_alert_ts(&calendar, &hour_spin, &minute_spin) {
                Some(ts) => ts <= now,
                None => true, // invalid date composition — also refuse to save
            };
            alert_warning.set_visible(in_past);
            save_btn.set_sensitive(!in_past);
        })
    };
    update_alert_validity();

    {
        let alert_box = alert_box.clone();
        let update = update_alert_validity.clone();
        alert_toggle.connect_toggled(move |t| {
            alert_box.set_visible(t.is_active());
            update();
        });
    }
    {
        let update = update_alert_validity.clone();
        calendar.connect_day_selected(move |_| update());
    }
    {
        let update = update_alert_validity.clone();
        hour_spin.connect_value_changed(move |_| update());
    }
    {
        let update = update_alert_validity.clone();
        minute_spin.connect_value_changed(move |_| update());
    }

    dialog.set_child(Some(&root));

    {
        let d = dialog.clone();
        cancel_btn.connect_clicked(move |_| d.close());
    }
    {
        let d = dialog.clone();
        let title_entry = title_entry.clone();
        let text_view = text_view.clone();
        let tags_entry = tags_entry.clone();
        let severity_dropdown = severity_dropdown.clone();
        let alert_toggle = alert_toggle.clone();
        let calendar = calendar.clone();
        let hour_spin = hour_spin.clone();
        let minute_spin = minute_spin.clone();
        let on_save = on_save.clone();
        save_btn.connect_clicked(move |_| {
            let draft = capture_draft(
                &title_entry,
                &text_view,
                &tags_entry,
                &severity_dropdown,
                &alert_toggle,
                &calendar,
                &hour_spin,
                &minute_spin,
            );
            d.close();
            // Defer the persist-and-reload to the next main loop tick so
            // the dialog finishes closing (and GSK finishes rendering its
            // current frame) before we tear down / rebuild the note list
            // beneath it. Running synchronously after d.close() was
            // causing a SIGSEGV inside libgallium / gsk_renderer_render
            // when widgets were freed while the previous frame was still
            // in flight.
            let on_save = on_save.clone();
            gtk4::glib::idle_add_local_once(move || {
                on_save(draft);
            });
        });
    }

    dialog.present();
    if initial.title.is_empty() {
        title_entry.grab_focus();
    } else {
        text_view.grab_focus();
    }
}

#[allow(clippy::too_many_arguments)]
fn capture_draft(
    title_entry: &gtk4::Entry,
    text_view: &gtk4::TextView,
    tags_entry: &gtk4::Entry,
    severity_dropdown: &gtk4::DropDown,
    alert_toggle: &gtk4::CheckButton,
    calendar: &gtk4::Calendar,
    hour_spin: &gtk4::SpinButton,
    minute_spin: &gtk4::SpinButton,
) -> NoteDraft {
    let title = title_entry.text().trim().to_string();

    let buf = text_view.buffer();
    let text = buf
        .text(&buf.start_iter(), &buf.end_iter(), false)
        .to_string();

    let tags: Vec<String> = tags_entry
        .text()
        .split(',')
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .collect();

    let sev_idx = severity_dropdown.selected() as usize;
    let severity = SEVERITY_DISPLAY
        .get(sev_idx)
        .map(|(id, _)| (*id).to_string())
        .unwrap_or_else(|| SEVERITY_INFO.to_string());

    let alert_at = if alert_toggle.is_active() {
        compose_alert_ts(calendar, hour_spin, minute_spin)
    } else {
        None
    };

    NoteDraft {
        title,
        text,
        tags,
        severity,
        alert_at,
    }
}

fn apply_alert_ts_to_widgets(
    ts: i64,
    calendar: &gtk4::Calendar,
    hour_spin: &gtk4::SpinButton,
    minute_spin: &gtk4::SpinButton,
) {
    use chrono::{Datelike, Local, TimeZone, Timelike};
    let Some(dt) = Local.timestamp_opt(ts, 0).single() else {
        return;
    };
    // `calendar` uses 0-indexed months.
    if let Some(dt_with_zero_month) = gtk4::glib::DateTime::from_local(
        dt.year(),
        dt.month() as i32,
        dt.day() as i32,
        dt.hour() as i32,
        dt.minute() as i32,
        0.0,
    )
    .ok()
    {
        calendar.select_day(&dt_with_zero_month);
    }
    hour_spin.set_value(dt.hour() as f64);
    minute_spin.set_value(dt.minute() as f64);
}

fn compose_alert_ts(
    calendar: &gtk4::Calendar,
    hour_spin: &gtk4::SpinButton,
    minute_spin: &gtk4::SpinButton,
) -> Option<i64> {
    use chrono::{Local, TimeZone};
    let date = calendar.date();
    let year = date.year();
    let month = date.month();
    let day = date.day_of_month();
    let hour = hour_spin.value_as_int();
    let minute = minute_spin.value_as_int();
    let naive = chrono::NaiveDate::from_ymd_opt(year, month as u32, day as u32)?.and_hms_opt(
        hour as u32,
        minute as u32,
        0,
    )?;
    Local
        .from_local_datetime(&naive)
        .single()
        .map(|dt| dt.timestamp())
}

/// Convenience: build a NoteDraft from an existing stored note.
pub fn draft_from_note(note: &WorkspaceNote) -> NoteDraft {
    NoteDraft {
        title: note.title.clone(),
        text: note.text.clone(),
        tags: note.tags.clone(),
        severity: note.severity.clone(),
        alert_at: note.alert_at,
    }
}

/// Convenience: defaults for a new note.
pub fn draft_default() -> NoteDraft {
    NoteDraft {
        title: String::new(),
        text: String::new(),
        tags: Vec::new(),
        severity: SEVERITY_INFO.to_string(),
        alert_at: None,
    }
}
