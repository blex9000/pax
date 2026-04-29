//! Markdown tab: Rendered / Source toggle in one tab.
//!
//! Lives as a child of the editor's content_stack under the name `tab-{id}`,
//! alongside the shared source editor. Rendered mode uses the shared markdown
//! renderer in `crate::markdown_render`; source mode uses `sourceview5::View`
//! with the markdown language applied.

use gtk4::prelude::*;
use sourceview5::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use super::tab_content::{MarkdownMode, MarkdownTab};

const RENDERED_MARGIN: i32 = 12;
const TOOLBAR_MARGIN: i32 = 4;
const EDITOR_LEFT_MARGIN: i32 = 6;
const EDITOR_TOP_MARGIN: i32 = 3;
const RIGHT_MARGIN_POSITION: u32 = 120;
const TAB_WIDTH: u32 = 4;
/// Horizontal padding around the label text inside the Rendered/Source
/// toggle buttons. Without this the labels hug the button border and read
/// as cramped next to the linked button group's outer chrome.
const MODE_BUTTON_PAD_PX: i32 = 10;
/// Note mark attribute values for the markdown tab's source view. Kept
/// close to the source-tab constants in editor_tabs but duplicated here
/// to avoid leaking a module boundary just for numbers.
const MD_NOTE_MARK_ICON: &str = "user-bookmarks-symbolic";
const MD_NOTE_MARK_R: f32 = 0.96;
const MD_NOTE_MARK_G: f32 = 0.78;
const MD_NOTE_MARK_B: f32 = 0.25;
const MD_NOTE_MARK_A: f32 = 0.25;
const MD_NOTE_MARK_PRIORITY: i32 = 10;

pub fn build_markdown_tab(content: &str) -> MarkdownTab {
    let mode = Rc::new(Cell::new(MarkdownMode::Rendered));
    let saved_content = Rc::new(RefCell::new(content.to_string()));

    // Source view (markdown language highlighting).
    let buffer = sourceview5::Buffer::new(None::<&gtk4::TextTagTable>);
    buffer.set_text(content);
    let lang_manager = sourceview5::LanguageManager::default();
    if let Some(lang) = lang_manager.language("markdown") {
        buffer.set_language(Some(&lang));
    }
    buffer.set_highlight_syntax(true);
    crate::theme::register_sourceview_buffer(&buffer);
    buffer.set_enable_undo(false);
    buffer.set_enable_undo(true);

    let source_view = sourceview5::View::with_buffer(&buffer);
    source_view.add_css_class("editor-code-view");
    source_view.set_show_line_numbers(true);
    source_view.set_show_line_marks(true);
    source_view.set_auto_indent(true);
    source_view.set_tab_width(TAB_WIDTH);
    source_view.set_wrap_mode(gtk4::WrapMode::WordChar);
    source_view.set_left_margin(EDITOR_LEFT_MARGIN);
    source_view.set_top_margin(EDITOR_TOP_MARGIN);
    source_view.set_monospace(true);
    source_view.set_show_right_margin(true);
    source_view.set_right_margin_position(RIGHT_MARGIN_POSITION);

    // Same note-mark category as source tabs so the gutter bookmark
    // icon renders for markdown too.
    {
        let attrs = sourceview5::MarkAttributes::new();
        attrs.set_icon_name(MD_NOTE_MARK_ICON);
        attrs.set_background(&gtk4::gdk::RGBA::new(
            MD_NOTE_MARK_R,
            MD_NOTE_MARK_G,
            MD_NOTE_MARK_B,
            MD_NOTE_MARK_A,
        ));
        source_view.set_mark_attributes(
            crate::panels::editor::notes_state::NOTE_MARK_CATEGORY,
            &attrs,
            MD_NOTE_MARK_PRIORITY,
        );
    }

    let source_scroll = gtk4::ScrolledWindow::new();
    source_scroll.set_child(Some(&source_view));
    source_scroll.set_vexpand(true);
    source_scroll.set_hexpand(true);

    // Side ruler with amber dots for notes. Lives at the outer level so
    // it's visible in BOTH Rendered and Source modes (the dots are at
    // source-line proportions regardless of which view is shown). Hidden
    // by default — `update` toggles visibility when lines are present.
    let notes_ruler = Rc::new(
        crate::panels::editor::notes_ruler::NotesRuler::new(source_view.clone()),
    );
    notes_ruler.widget.set_visible(false);

    // Rendered view (read-only TextView populated by the shared markdown renderer).
    let rendered_view = gtk4::TextView::new();
    rendered_view.set_editable(false);
    rendered_view.set_cursor_visible(false);
    // Read-only render — see panels/markdown.rs note: focusing the
    // TextView triggers Viewport scroll-to-focus, which snaps to top
    // on every click when the view sits inside the Overlay used for
    // the blockquote bar.
    rendered_view.set_can_focus(false);
    rendered_view.set_wrap_mode(gtk4::WrapMode::WordChar);
    rendered_view.set_left_margin(RENDERED_MARGIN);
    rendered_view.set_right_margin(RENDERED_MARGIN);
    rendered_view.set_top_margin(RENDERED_MARGIN);
    rendered_view.set_bottom_margin(RENDERED_MARGIN);
    rendered_view.add_css_class("editor-markdown-rendered");
    crate::markdown_render::render_markdown_to_view(&rendered_view, content);
    let rendered_scroll = gtk4::ScrolledWindow::new();
    rendered_scroll.set_child(Some(&rendered_view));
    rendered_scroll.set_vexpand(true);
    rendered_scroll.set_hexpand(true);
    crate::markdown_render::attach_blockquote_bar_overlay(&rendered_scroll, &rendered_view);

    let inner_stack = gtk4::Stack::new();
    inner_stack.add_named(&rendered_scroll, Some("rendered"));
    inner_stack.add_named(&source_scroll, Some("source"));
    inner_stack.set_visible_child_name("rendered");
    inner_stack.set_vexpand(true);
    inner_stack.set_hexpand(true);

    // Toolbar row 1: Rendered / Source mode toggle, right-aligned.
    let mode_bar = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
    mode_bar.add_css_class("editor-markdown-toolbar");
    mode_bar.set_margin_start(TOOLBAR_MARGIN);
    mode_bar.set_margin_end(TOOLBAR_MARGIN);
    mode_bar.set_margin_top(TOOLBAR_MARGIN);
    mode_bar.set_margin_bottom(TOOLBAR_MARGIN);

    // Spacer pushes the linked toggle pair to the right edge.
    let mode_spacer = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
    mode_spacer.set_hexpand(true);
    mode_bar.append(&mode_spacer);

    // Export PDF button — runs gtk's PrintOperation in Export mode
    // against the current buffer contents (works in both Rendered and
    // Source modes since the source of truth is the buffer text).
    let export_pdf_btn = gtk4::Button::from_icon_name("document-save-as-symbolic");
    export_pdf_btn.add_css_class("flat");
    export_pdf_btn.set_tooltip_text(Some("Export to PDF"));
    export_pdf_btn.set_margin_end(8);
    {
        let buf = buffer.clone();
        let parent_widget: gtk4::Widget = export_pdf_btn.clone().upcast();
        export_pdf_btn.connect_clicked(move |_| {
            let text = buf
                .text(&buf.start_iter(), &buf.end_iter(), false)
                .to_string();
            let parent = parent_widget
                .root()
                .and_then(|r| r.downcast::<gtk4::Window>().ok());
            if let Some(win) = parent.as_ref() {
                crate::markdown_export::export_markdown_to_pdf(win, &text, "document.pdf");
            }
        });
    }
    mode_bar.append(&export_pdf_btn);

    let mode_toggles = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
    mode_toggles.add_css_class("linked");
    mode_toggles.set_halign(gtk4::Align::End);

    // Build the toggle buttons with a padded label so the hit area is
    // comfortable rather than hugging the text.
    let rendered_btn = gtk4::ToggleButton::new();
    rendered_btn.set_child(Some(&padded_button_label("Rendered")));
    rendered_btn.set_active(true);
    let source_btn = gtk4::ToggleButton::new();
    source_btn.set_child(Some(&padded_button_label("Source")));
    source_btn.set_group(Some(&rendered_btn));
    mode_toggles.append(&rendered_btn);
    mode_toggles.append(&source_btn);
    mode_bar.append(&mode_toggles);

    // Toolbar row 2: formatting buttons for Source mode. Hidden in Rendered
    // mode. Mirrors the buttons available in the standalone Markdown Panel.
    let fmt_bar = build_formatting_bar(&buffer);

    {
        let stack = inner_stack.clone();
        let rv = rendered_view.clone();
        let buf = buffer.clone();
        let mode_c = mode.clone();
        let fmt_bar_c = fmt_bar.clone();
        let src_scroll = source_scroll.clone();
        let ren_scroll = rendered_scroll.clone();
        rendered_btn.connect_toggled(move |btn| {
            if !btn.is_active() {
                return;
            }
            // Preserve scroll: read fraction from source (departing), re-render,
            // then apply to rendered (arriving) after the layout pass.
            let frac = scroll_fraction(&src_scroll);
            let text = buf
                .text(&buf.start_iter(), &buf.end_iter(), false)
                .to_string();
            crate::markdown_render::render_markdown_to_view(&rv, &text);
            stack.set_visible_child_name("rendered");
            mode_c.set(MarkdownMode::Rendered);
            fmt_bar_c.set_visible(false);
            let target = ren_scroll.clone();
            gtk4::glib::idle_add_local_once(move || set_scroll_fraction(&target, frac));
        });
    }
    {
        let stack = inner_stack.clone();
        let mode_c = mode.clone();
        let fmt_bar_c = fmt_bar.clone();
        let src_scroll = source_scroll.clone();
        let ren_scroll = rendered_scroll.clone();
        source_btn.connect_toggled(move |btn| {
            if !btn.is_active() {
                return;
            }
            let frac = scroll_fraction(&ren_scroll);
            stack.set_visible_child_name("source");
            mode_c.set(MarkdownMode::Source);
            fmt_bar_c.set_visible(true);
            let target = src_scroll.clone();
            gtk4::glib::idle_add_local_once(move || set_scroll_fraction(&target, frac));
        });
    }

    // Horizontal row: inner_stack fills, notes_ruler sits on the right.
    // Keeping the ruler outside the stack means it stays visible when
    // the user toggles between Rendered and Source.
    let content_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
    content_row.set_vexpand(true);
    content_row.set_hexpand(true);
    content_row.append(&inner_stack);
    content_row.append(&notes_ruler.widget);

    let outer = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    outer.set_vexpand(true);
    outer.set_hexpand(true);
    outer.append(&mode_bar);
    outer.append(&fmt_bar);
    outer.append(&content_row);

    // Re-render on theme change so theme-reactive colors in the renderer
    // (code block background) take effect without restarting the app.
    {
        let buf = buffer.clone();
        let rv = rendered_view.clone();
        let mode_c = mode.clone();
        crate::theme::register_theme_observer(Rc::new(move || {
            if mode_c.get() == MarkdownMode::Rendered {
                let text = buf
                    .text(&buf.start_iter(), &buf.end_iter(), false)
                    .to_string();
                crate::markdown_render::render_markdown_to_view(&rv, &text);
            }
        }));
    }

    MarkdownTab {
        buffer,
        source_view,
        rendered_view,
        inner_stack,
        mode,
        modified: false,
        saved_content,
        outer: outer.upcast::<gtk4::Widget>(),
        rendered_btn,
        source_btn,
        rendered_scroll,
        source_scroll,
        notes: crate::panels::editor::notes_state::NotesState::new(),
        notes_ruler,
    }
}

/// Flip the tab between Rendered and Source modes. Drives the toggle
/// buttons directly so all wired side effects (re-rendering, toolbar
/// visibility, mode cell update, scroll preservation) fire through the
/// existing `connect_toggled` handlers.
pub fn toggle_mode(tab: &MarkdownTab) {
    match tab.mode.get() {
        MarkdownMode::Rendered => tab.source_btn.set_active(true),
        MarkdownMode::Source => tab.rendered_btn.set_active(true),
    }
}

fn padded_button_label(text: &str) -> gtk4::Label {
    let label = gtk4::Label::new(Some(text));
    label.set_margin_start(MODE_BUTTON_PAD_PX);
    label.set_margin_end(MODE_BUTTON_PAD_PX);
    label
}

fn scroll_fraction(scroll: &gtk4::ScrolledWindow) -> f64 {
    let v = scroll.vadjustment();
    let range = v.upper() - v.page_size() - v.lower();
    if range <= 0.0 {
        0.0
    } else {
        ((v.value() - v.lower()) / range).clamp(0.0, 1.0)
    }
}

fn set_scroll_fraction(scroll: &gtk4::ScrolledWindow, frac: f64) {
    let v = scroll.vadjustment();
    let range = (v.upper() - v.page_size() - v.lower()).max(0.0);
    v.set_value(v.lower() + range * frac.clamp(0.0, 1.0));
}

/// Formatting buttons for Source mode: bold / italic / code, H1-H3,
/// list / link / code-block. Mirrors the standalone Markdown panel.
fn build_formatting_bar(buffer: &sourceview5::Buffer) -> gtk4::Box {
    let fmt_bar = gtk4::Box::new(gtk4::Orientation::Horizontal, 2);
    fmt_bar.add_css_class("markdown-toolbar");
    fmt_bar.add_css_class("editor-markdown-toolbar");
    fmt_bar.set_margin_start(TOOLBAR_MARGIN);
    fmt_bar.set_margin_end(TOOLBAR_MARGIN);
    fmt_bar.set_margin_bottom(TOOLBAR_MARGIN);
    fmt_bar.set_visible(false);

    // Left-side spacer so the formatting buttons sit right-aligned,
    // matching the Rendered/Source toggle above them.
    let spacer = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    fmt_bar.append(&spacer);

    let buf: gtk4::TextBuffer = buffer.clone().upcast();

    // Inline-wrap markers.
    for (icon, tooltip, marker) in &[
        ("format-text-bold-symbolic", "Bold", "**"),
        ("format-text-italic-symbolic", "Italic", "*"),
        ("accessories-text-editor-symbolic", "Code", "`"),
    ] {
        let btn = gtk4::Button::from_icon_name(icon);
        btn.add_css_class("flat");
        btn.set_tooltip_text(Some(tooltip));
        let m = marker.to_string();
        let b = buf.clone();
        btn.connect_clicked(move |_| wrap_selection(&b, &m));
        fmt_bar.append(&btn);
    }
    fmt_bar.append(&gtk4::Separator::new(gtk4::Orientation::Vertical));

    // Heading levels.
    for (level, text) in &[(1, "H1"), (2, "H2"), (3, "H3")] {
        let btn = gtk4::Button::with_label(text);
        btn.add_css_class("flat");
        let prefix = "#".repeat(*level);
        let b = buf.clone();
        btn.connect_clicked(move |_| prepend_line(&b, &format!("{} ", prefix)));
        fmt_bar.append(&btn);
    }
    fmt_bar.append(&gtk4::Separator::new(gtk4::Orientation::Vertical));

    // Insert-at-cursor.
    for (icon, tooltip, text) in &[
        ("view-list-symbolic", "List", "- "),
        ("mail-attachment-symbolic", "Link", "[text](url)"),
        ("utilities-terminal-symbolic", "Code block", "```\n\n```"),
        (
            "view-grid-symbolic",
            "Table",
            "| Column 1 | Column 2 | Column 3 |\n|----------|----------|----------|\n| cell     | cell     | cell     |\n",
        ),
    ] {
        let btn = gtk4::Button::from_icon_name(icon);
        btn.add_css_class("flat");
        btn.set_tooltip_text(Some(tooltip));
        let t = text.to_string();
        let b = buf.clone();
        btn.connect_clicked(move |_| insert_at_cursor(&b, &t));
        fmt_bar.append(&btn);
    }

    fmt_bar
}

fn wrap_selection(buf: &gtk4::TextBuffer, marker: &str) {
    if let Some((start, end)) = buf.selection_bounds() {
        let text = buf.text(&start, &end, false).to_string();
        buf.delete(&mut start.clone(), &mut end.clone());
        buf.insert(
            &mut buf.iter_at_offset(start.offset()),
            &format!("{}{}{}", marker, text, marker),
        );
    } else {
        buf.insert(
            &mut buf.iter_at_mark(&buf.get_insert()),
            &format!("{}text{}", marker, marker),
        );
    }
}

fn prepend_line(buf: &gtk4::TextBuffer, prefix: &str) {
    let mut iter = buf.iter_at_mark(&buf.get_insert());
    iter.set_line_offset(0);
    buf.insert(&mut iter, prefix);
}

fn insert_at_cursor(buf: &gtk4::TextBuffer, text: &str) {
    buf.insert(&mut buf.iter_at_mark(&buf.get_insert()), text);
}

/// Apply external file content to a markdown tab: update the saved snapshot,
/// replace the source buffer, clear undo, and re-render the rendered view if
/// currently in Rendered mode. The connect_changed handler wired in
/// editor_tabs.rs sees `current == saved` and clears the dirty flag.
pub fn reload_from_disk(tab: &MarkdownTab, content: &str) {
    *tab.saved_content.borrow_mut() = content.to_string();
    tab.buffer.set_text(content);
    tab.buffer.set_enable_undo(false);
    tab.buffer.set_enable_undo(true);
    if tab.mode.get() == MarkdownMode::Rendered {
        crate::markdown_render::render_markdown_to_view(&tab.rendered_view, content);
    }
}
