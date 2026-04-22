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
    source_view.set_auto_indent(true);
    source_view.set_tab_width(TAB_WIDTH);
    source_view.set_wrap_mode(gtk4::WrapMode::WordChar);
    source_view.set_left_margin(EDITOR_LEFT_MARGIN);
    source_view.set_top_margin(EDITOR_TOP_MARGIN);
    source_view.set_monospace(true);
    source_view.set_show_right_margin(true);
    source_view.set_right_margin_position(RIGHT_MARGIN_POSITION);
    let source_scroll = gtk4::ScrolledWindow::new();
    source_scroll.set_child(Some(&source_view));
    source_scroll.set_vexpand(true);
    source_scroll.set_hexpand(true);

    // Rendered view (read-only TextView populated by the shared markdown renderer).
    let rendered_view = gtk4::TextView::new();
    rendered_view.set_editable(false);
    rendered_view.set_cursor_visible(false);
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

    let inner_stack = gtk4::Stack::new();
    inner_stack.add_named(&rendered_scroll, Some("rendered"));
    inner_stack.add_named(&source_scroll, Some("source"));
    inner_stack.set_visible_child_name("rendered");
    inner_stack.set_vexpand(true);
    inner_stack.set_hexpand(true);

    // Toolbar row 1: Rendered / Source mode toggle.
    let mode_bar = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
    mode_bar.add_css_class("editor-markdown-toolbar");
    mode_bar.add_css_class("linked");
    mode_bar.set_margin_start(TOOLBAR_MARGIN);
    mode_bar.set_margin_end(TOOLBAR_MARGIN);
    mode_bar.set_margin_top(TOOLBAR_MARGIN);
    mode_bar.set_margin_bottom(TOOLBAR_MARGIN);

    let rendered_btn = gtk4::ToggleButton::with_label("Rendered");
    rendered_btn.set_active(true);
    let source_btn = gtk4::ToggleButton::with_label("Source");
    source_btn.set_group(Some(&rendered_btn));
    mode_bar.append(&rendered_btn);
    mode_bar.append(&source_btn);

    // Toolbar row 2: formatting buttons for Source mode. Hidden in Rendered
    // mode. Mirrors the buttons available in the standalone Markdown Panel.
    let fmt_bar = build_formatting_bar(&buffer);

    {
        let stack = inner_stack.clone();
        let rv = rendered_view.clone();
        let buf = buffer.clone();
        let mode_c = mode.clone();
        let fmt_bar_c = fmt_bar.clone();
        rendered_btn.connect_toggled(move |btn| {
            if !btn.is_active() {
                return;
            }
            // Re-render from the current buffer content (dirty OK). Matches
            // the standalone Markdown panel's behavior.
            let text = buf
                .text(&buf.start_iter(), &buf.end_iter(), false)
                .to_string();
            crate::markdown_render::render_markdown_to_view(&rv, &text);
            stack.set_visible_child_name("rendered");
            mode_c.set(MarkdownMode::Rendered);
            fmt_bar_c.set_visible(false);
        });
    }
    {
        let stack = inner_stack.clone();
        let mode_c = mode.clone();
        let fmt_bar_c = fmt_bar.clone();
        source_btn.connect_toggled(move |btn| {
            if !btn.is_active() {
                return;
            }
            stack.set_visible_child_name("source");
            mode_c.set(MarkdownMode::Source);
            fmt_bar_c.set_visible(true);
        });
    }

    let outer = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    outer.set_vexpand(true);
    outer.set_hexpand(true);
    outer.append(&mode_bar);
    outer.append(&fmt_bar);
    outer.append(&inner_stack);

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
    }
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
