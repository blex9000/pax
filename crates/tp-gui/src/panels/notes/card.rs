//! Single-note card widget.
//!
//! Layout:
//!   header  : title + [edit] [delete] icon buttons (right-aligned)
//!   body    : rendered markdown preview
//!   footer  : severity dot · tag chips · alert badge

use gtk4::prelude::*;
use std::rc::Rc;

use pax_db::workspace_notes::{
    SEVERITY_IMPORTANT, SEVERITY_INFO, SEVERITY_WARNING, WorkspaceNote,
};

/// Callbacks propagated to the caller (list view); the card itself never
/// touches the database.
pub struct NoteCardActions {
    pub on_delete: Box<dyn Fn()>,
    pub on_cycle_severity: Box<dyn Fn()>,
    /// Opens the full editor dialog (title, tags, severity, alert, text).
    pub on_open_editor: Box<dyn Fn()>,
}

const SEVERITY_CLASS_PREFIX: &str = "note-card--";
const NOTE_PREVIEW_MAX_CHARS: usize = 600;

pub fn build_note_card(note: &WorkspaceNote, actions: NoteCardActions) -> gtk4::Widget {
    let card = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
    card.add_css_class("note-card");
    card.add_css_class(&severity_class(&note.severity));
    card.set_margin_top(4);
    card.set_margin_bottom(4);
    card.set_margin_start(4);
    card.set_margin_end(4);

    let on_open_editor = Rc::new(actions.on_open_editor);
    let on_delete = Rc::new(actions.on_delete);
    let on_cycle_severity = Rc::new(actions.on_cycle_severity);

    // ── Header: title + edit/delete buttons ────────────────────────────
    let header = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    header.set_valign(gtk4::Align::Center);

    let title_label = gtk4::Label::new(None);
    if note.title.trim().is_empty() {
        title_label.set_text("Untitled");
        title_label.add_css_class("dim-label");
    } else {
        title_label.set_text(&note.title);
    }
    title_label.add_css_class("note-card-title");
    title_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    title_label.set_halign(gtk4::Align::Start);
    title_label.set_xalign(0.0);
    title_label.set_hexpand(true);
    header.append(&title_label);

    let edit_btn = gtk4::Button::with_label("Edit");
    edit_btn.add_css_class("note-card-action");
    edit_btn.set_valign(gtk4::Align::Center);
    {
        let on_open = on_open_editor.clone();
        edit_btn.connect_clicked(move |_| {
            tracing::info!("note card: edit clicked");
            on_open();
        });
    }
    header.append(&edit_btn);

    let delete_btn = gtk4::Button::with_label("Delete");
    delete_btn.add_css_class("note-card-action");
    delete_btn.add_css_class("destructive-action");
    delete_btn.set_valign(gtk4::Align::Center);
    {
        let on_del = on_delete.clone();
        delete_btn.connect_clicked(move |_| {
            tracing::info!("note card: delete clicked");
            on_del();
        });
    }
    header.append(&delete_btn);

    card.append(&header);

    // ── Body: rendered markdown ────────────────────────────────────────
    let rendered_view = gtk4::TextView::new();
    rendered_view.set_editable(false);
    rendered_view.set_cursor_visible(false);
    rendered_view.set_wrap_mode(gtk4::WrapMode::WordChar);
    rendered_view.set_left_margin(0);
    rendered_view.set_right_margin(0);
    rendered_view.set_top_margin(0);
    rendered_view.set_bottom_margin(0);
    rendered_view.set_can_target(false);
    rendered_view.add_css_class("note-card-rendered");
    crate::markdown_render::render_markdown_to_view(
        &rendered_view,
        &truncate_preview(&note.text),
    );
    card.append(&rendered_view);

    // ── Footer: severity dot · tags · alert badge ──────────────────────
    let footer = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    footer.set_valign(gtk4::Align::Center);
    footer.set_margin_top(2);

    let severity_dot = build_severity_dot(&note.severity);
    footer.append(&severity_dot);

    for tag in &note.tags {
        let chip = gtk4::Label::new(Some(tag));
        chip.add_css_class("tag-chip");
        footer.append(&chip);
    }

    let spacer = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    footer.append(&spacer);

    if let Some(alert_at) = note.alert_at {
        let badge = gtk4::Label::new(Some(&format_alert_badge(alert_at, note.alert_fired_at)));
        badge.add_css_class("alert-badge");
        footer.append(&badge);
    }

    card.append(&footer);

    // Click the severity dot to cycle info → warning → important.
    {
        let gesture = gtk4::GestureClick::new();
        gesture.set_button(gtk4::gdk::BUTTON_PRIMARY);
        let cycle = on_cycle_severity.clone();
        gesture.connect_released(move |g, _, _, _| {
            g.set_state(gtk4::EventSequenceState::Claimed);
            cycle();
        });
        severity_dot.add_controller(gesture);
    }

    card.upcast()
}

fn severity_class(severity: &str) -> String {
    let suffix = match severity {
        SEVERITY_WARNING => "warning",
        SEVERITY_IMPORTANT => "important",
        _ => "info",
    };
    format!("{SEVERITY_CLASS_PREFIX}{suffix}")
}

fn build_severity_dot(severity: &str) -> gtk4::Widget {
    let dot = gtk4::Label::new(Some("●"));
    dot.add_css_class("note-card-severity-dot");
    dot.add_css_class(&severity_class(severity));
    let tooltip = match severity {
        SEVERITY_WARNING => "Warning · click to cycle",
        SEVERITY_IMPORTANT => "Important · click to cycle",
        SEVERITY_INFO => "Info · click to cycle",
        _ => "Severity · click to cycle",
    };
    dot.set_tooltip_text(Some(tooltip));
    dot.upcast()
}

fn format_alert_badge(alert_at: i64, fired_at: Option<i64>) -> String {
    let prefix = if fired_at.is_some() { "⏰ fired " } else { "⏰ " };
    format!("{prefix}{}", format_timestamp(alert_at))
}

fn format_timestamp(ts: i64) -> String {
    use chrono::{DateTime, Local, TimeZone};
    let Some(dt) = Local.timestamp_opt(ts, 0).single() else {
        return ts.to_string();
    };
    let now: DateTime<Local> = Local::now();
    let diff = dt.signed_duration_since(now);
    let abs_days = diff.num_days().abs();

    if diff.num_days() == 0 && dt.date_naive() == now.date_naive() {
        dt.format("%H:%M").to_string()
    } else if abs_days < 7 {
        dt.format("%a %H:%M").to_string()
    } else {
        dt.format("%Y-%m-%d %H:%M").to_string()
    }
}

fn truncate_preview(text: &str) -> String {
    if text.chars().count() <= NOTE_PREVIEW_MAX_CHARS {
        return text.to_string();
    }
    let truncated: String = text.chars().take(NOTE_PREVIEW_MAX_CHARS).collect();
    format!("{truncated}…")
}
