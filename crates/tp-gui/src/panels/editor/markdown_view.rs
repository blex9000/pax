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

    // Toolbar with linked Rendered/Source toggle buttons.
    let bar = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
    bar.add_css_class("editor-markdown-toolbar");
    bar.add_css_class("linked");
    bar.set_margin_start(TOOLBAR_MARGIN);
    bar.set_margin_end(TOOLBAR_MARGIN);
    bar.set_margin_top(TOOLBAR_MARGIN);
    bar.set_margin_bottom(TOOLBAR_MARGIN);

    let rendered_btn = gtk4::ToggleButton::with_label("Rendered");
    rendered_btn.set_active(true);
    let source_btn = gtk4::ToggleButton::with_label("Source");
    source_btn.set_group(Some(&rendered_btn));
    bar.append(&rendered_btn);
    bar.append(&source_btn);

    {
        let stack = inner_stack.clone();
        let rv = rendered_view.clone();
        let buf = buffer.clone();
        let mode_c = mode.clone();
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
        });
    }
    {
        let stack = inner_stack.clone();
        let mode_c = mode.clone();
        source_btn.connect_toggled(move |btn| {
            if !btn.is_active() {
                return;
            }
            stack.set_visible_child_name("source");
            mode_c.set(MarkdownMode::Source);
        });
    }

    let outer = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    outer.set_vexpand(true);
    outer.set_hexpand(true);
    outer.append(&bar);
    outer.append(&inner_stack);

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
