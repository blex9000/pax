//! Single-note card widget.
//!
//! Two visual states:
//!   compact   — one line: [title · preview…] [tag][tag] [alert] [expand] [edit][delete]
//!                Edit / Delete appear only when the row is hovered.
//!                Expand button is shown only when the note has more content
//!                than fits on one line.
//!   expanded  — compact row kept, plus the fully rendered markdown below it.
//!
//! Severity is communicated by a colored left border on the card (info uses
//! the theme accent, warning and important have their own theme tokens).

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
/// A note "has more" (and therefore shows the expand button) when it
/// contains multiple lines or a single line longer than this many chars.
const INLINE_PREVIEW_CHAR_THRESHOLD: usize = 80;

pub fn build_note_card(note: &WorkspaceNote, actions: NoteCardActions) -> gtk4::Widget {
    let card = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
    card.add_css_class("note-card");
    card.add_css_class(&severity_class(&note.severity));

    let on_open_editor = Rc::new(actions.on_open_editor);
    let on_delete = Rc::new(actions.on_delete);
    // Severity cycling lost its dedicated UI (we no longer render the dot)
    // but the callback remains in the public API so the list view can still
    // wire it for future surfaces (keyboard shortcut, dialog, etc.).
    let _ = actions.on_cycle_severity;

    // ── Compact row (always visible) ───────────────────────────────────
    let compact = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    compact.set_valign(gtk4::Align::Center);

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
    compact.append(&title_label);

    let preview_label = gtk4::Label::new(Some(&first_line_preview(&note.text)));
    preview_label.add_css_class("note-card-preview");
    preview_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    preview_label.set_halign(gtk4::Align::Start);
    preview_label.set_xalign(0.0);
    preview_label.set_hexpand(true);
    compact.append(&preview_label);

    for tag in &note.tags {
        let chip = gtk4::Label::new(Some(tag));
        chip.add_css_class("tag-chip");
        compact.append(&chip);
    }

    if let Some(alert_at) = note.alert_at {
        let badge = gtk4::Label::new(Some(&format_alert_badge(alert_at, note.alert_fired_at)));
        badge.add_css_class("alert-badge");
        compact.append(&badge);
    }

    let edit_btn = gtk4::Button::with_label("Edit");
    edit_btn.add_css_class("note-card-action");
    edit_btn.add_css_class("note-card-hover-action");
    edit_btn.set_valign(gtk4::Align::Center);
    {
        let on_open = on_open_editor.clone();
        edit_btn.connect_clicked(move |_| on_open());
    }
    compact.append(&edit_btn);

    let delete_btn = gtk4::Button::with_label("Delete");
    delete_btn.add_css_class("note-card-action");
    delete_btn.add_css_class("note-card-hover-action");
    delete_btn.add_css_class("destructive-action");
    delete_btn.set_valign(gtk4::Align::Center);
    {
        let on_del = on_delete.clone();
        delete_btn.connect_clicked(move |_| on_del());
    }
    compact.append(&delete_btn);

    let has_more = note_has_more_content(&note.text);
    let expand_btn = gtk4::Button::from_icon_name("pan-down-symbolic");
    expand_btn.add_css_class("flat");
    expand_btn.add_css_class("note-card-action");
    expand_btn.add_css_class("note-card-expand");
    expand_btn.set_tooltip_text(Some("Expand"));
    expand_btn.set_valign(gtk4::Align::Center);
    if !has_more {
        expand_btn.set_visible(false);
    }
    compact.append(&expand_btn);

    card.append(&compact);

    // ── Expanded body (wrapped in a Revealer for natural-size reveal) ──
    //
    // A raw `TextView` toggled via `set_visible` measures its natural
    // height against an unconstrained width on the first show, so the
    // first expand grabs the whole scroll area; on subsequent shows the
    // cached width gives the right measurement. `gtk::Revealer` is built
    // for exactly this pattern — it queries the child's natural size
    // at its own allocated width, so the first expand is the right size.
    let rendered_view = gtk4::TextView::new();
    rendered_view.set_editable(false);
    rendered_view.set_cursor_visible(false);
    rendered_view.set_wrap_mode(gtk4::WrapMode::WordChar);
    rendered_view.set_left_margin(0);
    rendered_view.set_right_margin(0);
    rendered_view.set_top_margin(6);
    rendered_view.set_bottom_margin(0);
    rendered_view.set_can_target(false);
    rendered_view.set_vexpand(false);
    rendered_view.set_valign(gtk4::Align::Start);
    rendered_view.add_css_class("note-card-rendered");
    crate::markdown_render::render_markdown_to_view(&rendered_view, &note.text);

    let body_revealer = gtk4::Revealer::new();
    body_revealer.set_transition_type(gtk4::RevealerTransitionType::SlideDown);
    body_revealer.set_transition_duration(120);
    body_revealer.set_reveal_child(false);
    body_revealer.set_child(Some(&rendered_view));
    card.append(&body_revealer);

    {
        let revealer = body_revealer.clone();
        let btn = expand_btn.clone();
        expand_btn.connect_clicked(move |_| {
            let expanded = !revealer.reveals_child();
            revealer.set_reveal_child(expanded);
            btn.set_icon_name(if expanded {
                "pan-up-symbolic"
            } else {
                "pan-down-symbolic"
            });
            btn.set_tooltip_text(Some(if expanded { "Collapse" } else { "Expand" }));
        });
    }

    card.upcast()
}

fn severity_class(severity: &str) -> String {
    let suffix = match severity {
        SEVERITY_WARNING => "warning",
        SEVERITY_IMPORTANT => "important",
        SEVERITY_INFO => "info",
        _ => "info",
    };
    format!("{SEVERITY_CLASS_PREFIX}{suffix}")
}

fn note_has_more_content(text: &str) -> bool {
    text.lines().count() > 1 || text.chars().count() > INLINE_PREVIEW_CHAR_THRESHOLD
}

fn first_line_preview(text: &str) -> String {
    let first = text.lines().next().unwrap_or("").trim();
    if first.chars().count() <= INLINE_PREVIEW_CHAR_THRESHOLD {
        first.to_string()
    } else {
        let truncated: String = first.chars().take(INLINE_PREVIEW_CHAR_THRESHOLD).collect();
        format!("{truncated}…")
    }
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
