//! Single-note card widget. Displays the note's title and markdown
//! (rendered by default), tag chips and alert badge; inline text editing
//! and action buttons reveal on hover so the card surface stays calm.

use gtk4::prelude::*;
use std::rc::Rc;

use pax_db::workspace_notes::{
    SEVERITY_IMPORTANT, SEVERITY_INFO, SEVERITY_WARNING, WorkspaceNote,
};

/// Closures wired by the caller (the list view) so the card can propagate
/// user intent without knowing about the database.
pub struct NoteCardActions {
    pub on_save_text: Box<dyn Fn(&str)>,
    pub on_delete: Box<dyn Fn()>,
    pub on_cycle_severity: Box<dyn Fn()>,
    /// Double-click on the body opens the full editor dialog (tags,
    /// severity, alert).
    pub on_open_editor: Box<dyn Fn()>,
}

const SEVERITY_CLASS_PREFIX: &str = "note-card--";
const NOTE_PREVIEW_MAX_CHARS: usize = 600;

/// Build the widget tree for a single note card.
pub fn build_note_card(note: &WorkspaceNote, actions: NoteCardActions) -> gtk4::Widget {
    let card = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
    card.add_css_class("note-card");
    card.add_css_class(&severity_class(&note.severity));
    card.set_margin_top(2);
    card.set_margin_bottom(2);
    card.set_margin_start(2);
    card.set_margin_end(2);

    // ── Header: severity dot + title + alert badge + actions ───────────
    let header = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    header.set_valign(gtk4::Align::Center);

    let severity_dot = build_severity_dot(&note.severity);
    header.append(&severity_dot);

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
    title_label.set_hexpand(true);
    title_label.set_xalign(0.0);
    header.append(&title_label);

    if let Some(alert_at) = note.alert_at {
        let badge = gtk4::Label::new(Some(&format_alert_badge(alert_at, note.alert_fired_at)));
        badge.add_css_class("alert-badge");
        header.append(&badge);
    }

    // Action row (hidden unless the card is hovered — handled via CSS).
    let actions_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 2);
    actions_box.add_css_class("note-card-actions");

    let edit_btn = gtk4::Button::from_icon_name("document-edit-symbolic");
    edit_btn.add_css_class("flat");
    edit_btn.add_css_class("note-card-action");
    edit_btn.set_tooltip_text(Some("Edit text"));
    actions_box.append(&edit_btn);

    let save_btn = gtk4::Button::from_icon_name("document-save-symbolic");
    save_btn.add_css_class("flat");
    save_btn.add_css_class("note-card-action");
    save_btn.set_tooltip_text(Some("Save"));
    save_btn.set_visible(false);
    actions_box.append(&save_btn);

    let cancel_btn = gtk4::Button::from_icon_name("process-stop-symbolic");
    cancel_btn.add_css_class("flat");
    cancel_btn.add_css_class("note-card-action");
    cancel_btn.set_tooltip_text(Some("Cancel"));
    cancel_btn.set_visible(false);
    actions_box.append(&cancel_btn);

    let delete_btn = gtk4::Button::from_icon_name("user-trash-symbolic");
    delete_btn.add_css_class("flat");
    delete_btn.add_css_class("note-card-action");
    delete_btn.add_css_class("note-card-action-danger");
    delete_btn.set_tooltip_text(Some("Delete"));
    actions_box.append(&delete_btn);

    header.append(&actions_box);
    card.append(&header);

    // ── Body: rendered ↔ source stack ──────────────────────────────────
    let content_stack = gtk4::Stack::new();
    content_stack.set_transition_type(gtk4::StackTransitionType::Crossfade);
    content_stack.set_transition_duration(120);

    let rendered_view = gtk4::TextView::new();
    rendered_view.set_editable(false);
    rendered_view.set_cursor_visible(false);
    rendered_view.set_wrap_mode(gtk4::WrapMode::WordChar);
    rendered_view.set_left_margin(6);
    rendered_view.set_right_margin(6);
    rendered_view.set_top_margin(2);
    rendered_view.set_bottom_margin(2);
    rendered_view.add_css_class("note-card-rendered");
    crate::markdown_render::render_markdown_to_view(
        &rendered_view,
        &truncate_preview(&note.text),
    );

    let source_view = gtk4::TextView::new();
    source_view.set_wrap_mode(gtk4::WrapMode::WordChar);
    source_view.set_left_margin(6);
    source_view.set_right_margin(6);
    source_view.set_top_margin(2);
    source_view.set_bottom_margin(2);
    source_view.set_monospace(true);
    source_view.add_css_class("note-card-source");
    source_view.buffer().set_text(&note.text);

    content_stack.add_named(&rendered_view, Some("rendered"));
    content_stack.add_named(&source_view, Some("source"));
    content_stack.set_visible_child_name("rendered");
    card.append(&content_stack);

    // ── Footer: tag chips (only when present) ──────────────────────────
    if !note.tags.is_empty() {
        let tags_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
        tags_box.set_margin_top(2);
        for tag in &note.tags {
            let chip = gtk4::Label::new(Some(tag));
            chip.add_css_class("tag-chip");
            tags_box.append(&chip);
        }
        card.append(&tags_box);
    }

    // ── Wiring ─────────────────────────────────────────────────────────
    let original_text = Rc::new(note.text.clone());
    let on_save_text = Rc::new(actions.on_save_text);

    {
        let stack = content_stack.clone();
        let source = source_view.clone();
        let edit = edit_btn.clone();
        let save = save_btn.clone();
        let cancel = cancel_btn.clone();
        let delete = delete_btn.clone();
        let actions_box = actions_box.clone();
        edit_btn.connect_clicked(move |_| {
            stack.set_visible_child_name("source");
            source.grab_focus();
            edit.set_visible(false);
            delete.set_visible(false);
            save.set_visible(true);
            cancel.set_visible(true);
            // While editing, the actions row is always visible regardless
            // of hover — the user is engaged with the card.
            actions_box.add_css_class("note-card-actions--editing");
        });
    }
    {
        let stack = content_stack.clone();
        let source = source_view.clone();
        let edit = edit_btn.clone();
        let save = save_btn.clone();
        let cancel = cancel_btn.clone();
        let delete = delete_btn.clone();
        let actions_box = actions_box.clone();
        let on_save = on_save_text.clone();
        save_btn.connect_clicked(move |_| {
            let buf = source.buffer();
            let text = buf.text(&buf.start_iter(), &buf.end_iter(), false).to_string();
            on_save(&text);
            stack.set_visible_child_name("rendered");
            edit.set_visible(true);
            delete.set_visible(true);
            save.set_visible(false);
            cancel.set_visible(false);
            actions_box.remove_css_class("note-card-actions--editing");
        });
    }
    {
        let stack = content_stack.clone();
        let source = source_view.clone();
        let edit = edit_btn.clone();
        let save = save_btn.clone();
        let cancel = cancel_btn.clone();
        let delete = delete_btn.clone();
        let actions_box = actions_box.clone();
        let orig = original_text.clone();
        cancel_btn.connect_clicked(move |_| {
            source.buffer().set_text(&orig);
            stack.set_visible_child_name("rendered");
            edit.set_visible(true);
            delete.set_visible(true);
            save.set_visible(false);
            cancel.set_visible(false);
            actions_box.remove_css_class("note-card-actions--editing");
        });
    }

    let on_delete = Rc::new(actions.on_delete);
    delete_btn.connect_clicked(move |_| on_delete());

    // Severity cycle: click the dot to cycle info → warning → important.
    let on_cycle_severity = Rc::new(actions.on_cycle_severity);
    let severity_click = gtk4::GestureClick::new();
    severity_click.set_button(gtk4::gdk::BUTTON_PRIMARY);
    {
        let cycle = on_cycle_severity.clone();
        severity_click.connect_released(move |_, _, _, _| cycle());
    }
    severity_dot.add_controller(severity_click);

    // Double-click anywhere in the header or body opens the advanced
    // editor. We attach to the rendered body (not the source view, so
    // word-selection still works there).
    let on_open_editor = Rc::new(actions.on_open_editor);
    let dbl_click = gtk4::GestureClick::new();
    dbl_click.set_button(gtk4::gdk::BUTTON_PRIMARY);
    {
        let open = on_open_editor.clone();
        dbl_click.connect_pressed(move |_, n_press, _, _| {
            if n_press == 2 {
                open();
            }
        });
    }
    rendered_view.add_controller(dbl_click);

    // Double-click on the title opens the editor too (same gesture).
    let title_dbl = gtk4::GestureClick::new();
    title_dbl.set_button(gtk4::gdk::BUTTON_PRIMARY);
    {
        let open = on_open_editor.clone();
        title_dbl.connect_pressed(move |_, n_press, _, _| {
            if n_press == 2 {
                open();
            }
        });
    }
    title_label.add_controller(title_dbl);

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
    // Make the dot accept click gestures.
    dot.upcast()
}

/// Format a scheduled-alert timestamp. Keeps it compact: today → "HH:MM",
/// within 7 days → "Wed HH:MM", otherwise ISO date + time.
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
