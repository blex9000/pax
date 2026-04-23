//! Single-note card widget. Displays the note's markdown (rendered by
//! default), tag chips and alert badge, and exposes inline text editing.
//!
//! Card is a factory: `build_note_card` returns the root widget plus the
//! wiring. The caller supplies closures for save / delete / severity-cycle
//! so the card stays ignorant of persistence details.

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

/// Build the widget tree for a single note card.
pub fn build_note_card(note: &WorkspaceNote, actions: NoteCardActions) -> gtk4::Widget {
    let card = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
    card.add_css_class("note-card");
    card.add_css_class(&severity_class(&note.severity));
    card.set_margin_top(4);
    card.set_margin_bottom(4);
    card.set_margin_start(6);
    card.set_margin_end(6);

    // Content area: a Stack with rendered view and editable source view.
    let content_stack = gtk4::Stack::new();
    content_stack.set_transition_type(gtk4::StackTransitionType::Crossfade);
    content_stack.set_transition_duration(120);

    let rendered_view = gtk4::TextView::new();
    rendered_view.set_editable(false);
    rendered_view.set_cursor_visible(false);
    rendered_view.set_wrap_mode(gtk4::WrapMode::WordChar);
    rendered_view.set_left_margin(8);
    rendered_view.set_right_margin(8);
    rendered_view.set_top_margin(4);
    rendered_view.set_bottom_margin(4);
    rendered_view.add_css_class("note-card-rendered");
    crate::markdown_render::render_markdown_to_view(&rendered_view, &note.text);

    let source_view = gtk4::TextView::new();
    source_view.set_wrap_mode(gtk4::WrapMode::WordChar);
    source_view.set_left_margin(8);
    source_view.set_right_margin(8);
    source_view.set_top_margin(4);
    source_view.set_bottom_margin(4);
    source_view.set_monospace(true);
    source_view.add_css_class("note-card-source");
    source_view.buffer().set_text(&note.text);

    content_stack.add_named(&rendered_view, Some("rendered"));
    content_stack.add_named(&source_view, Some("source"));
    content_stack.set_visible_child_name("rendered");
    card.append(&content_stack);

    // Footer: tag chips + alert badge + action buttons.
    let footer = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    footer.set_margin_start(4);
    footer.set_margin_end(4);
    footer.set_margin_top(2);

    let tags_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
    for tag in &note.tags {
        let chip = gtk4::Label::new(Some(tag));
        chip.add_css_class("tag-chip");
        tags_box.append(&chip);
    }
    footer.append(&tags_box);

    if let Some(alert_at) = note.alert_at {
        let badge = gtk4::Label::new(Some(&format_alert_badge(alert_at, note.alert_fired_at)));
        badge.add_css_class("alert-badge");
        footer.append(&badge);
    }

    let spacer = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    footer.append(&spacer);

    // Severity cycle button: small label-only toggle letting the user flip
    // the severity without opening the full-editor dialog. The advanced
    // dialog (tags, alert) is wired separately.
    let severity_btn = gtk4::Button::with_label(severity_label(&note.severity));
    severity_btn.add_css_class("flat");
    severity_btn.add_css_class("severity-toggle");
    let cycle_cb = Rc::new(actions.on_cycle_severity);
    severity_btn.connect_clicked(move |_| cycle_cb());
    footer.append(&severity_btn);

    let edit_btn = gtk4::Button::from_icon_name("document-edit-symbolic");
    edit_btn.add_css_class("flat");
    edit_btn.set_tooltip_text(Some("Edit text"));
    footer.append(&edit_btn);

    let save_btn = gtk4::Button::from_icon_name("document-save-symbolic");
    save_btn.add_css_class("flat");
    save_btn.set_tooltip_text(Some("Save"));
    save_btn.set_visible(false);
    footer.append(&save_btn);

    let cancel_btn = gtk4::Button::from_icon_name("process-stop-symbolic");
    cancel_btn.add_css_class("flat");
    cancel_btn.set_tooltip_text(Some("Cancel"));
    cancel_btn.set_visible(false);
    footer.append(&cancel_btn);

    let delete_btn = gtk4::Button::from_icon_name("user-trash-symbolic");
    delete_btn.add_css_class("flat");
    delete_btn.add_css_class("destructive-action");
    delete_btn.set_tooltip_text(Some("Delete"));
    footer.append(&delete_btn);

    card.append(&footer);

    // ── Mode toggling ───────────────────────────────────────────────────
    //
    // Enter edit: show source view, swap Edit button for Save/Cancel.
    // Save: persist new text (caller reloads the list, so the card will
    // be rebuilt with fresh content).
    // Cancel: restore the source buffer to the original text and return
    // to rendered mode.
    let original_text = Rc::new(note.text.clone());
    let on_save_text = Rc::new(actions.on_save_text);

    {
        let stack = content_stack.clone();
        let source = source_view.clone();
        let edit = edit_btn.clone();
        let save = save_btn.clone();
        let cancel = cancel_btn.clone();
        edit_btn.connect_clicked(move |_| {
            stack.set_visible_child_name("source");
            source.grab_focus();
            edit.set_visible(false);
            save.set_visible(true);
            cancel.set_visible(true);
        });
    }
    {
        let stack = content_stack.clone();
        let source = source_view.clone();
        let edit = edit_btn.clone();
        let save = save_btn.clone();
        let cancel = cancel_btn.clone();
        let on_save = on_save_text.clone();
        save_btn.connect_clicked(move |_| {
            let buf = source.buffer();
            let text = buf.text(&buf.start_iter(), &buf.end_iter(), false).to_string();
            on_save(&text);
            stack.set_visible_child_name("rendered");
            edit.set_visible(true);
            save.set_visible(false);
            cancel.set_visible(false);
        });
    }
    {
        let stack = content_stack.clone();
        let source = source_view.clone();
        let edit = edit_btn.clone();
        let save = save_btn.clone();
        let cancel = cancel_btn.clone();
        let orig = original_text.clone();
        cancel_btn.connect_clicked(move |_| {
            source.buffer().set_text(&orig);
            stack.set_visible_child_name("rendered");
            edit.set_visible(true);
            save.set_visible(false);
            cancel.set_visible(false);
        });
    }

    let on_delete = Rc::new(actions.on_delete);
    delete_btn.connect_clicked(move |_| on_delete());

    // Double-click on the rendered body opens the advanced editor dialog.
    // We intentionally attach to the rendered view only — double-clicking
    // inside the source view should just select a word like a normal text
    // editor.
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

fn severity_label(severity: &str) -> &'static str {
    match severity {
        SEVERITY_WARNING => "warning",
        SEVERITY_IMPORTANT => "important",
        SEVERITY_INFO => "info",
        _ => "info",
    }
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
