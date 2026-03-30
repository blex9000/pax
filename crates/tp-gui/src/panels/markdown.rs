use gtk4::prelude::*;
use gtk4::glib;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use super::PanelBackend;

#[derive(Debug, Clone, Copy, PartialEq)]
enum Mode { Render, Edit }

/// Markdown viewer/editor panel with formatting toolbar, auto-save, and file watching.
#[derive(Debug)]
pub struct MarkdownPanel {
    widget: gtk4::Widget,
    text_view: gtk4::TextView,
    #[allow(dead_code)]
    file_path: String,
}

impl MarkdownPanel {
    pub fn new(file_path: &str) -> Self {
        let container = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        let mode = Rc::new(Cell::new(Mode::Render));
        let content = Rc::new(RefCell::new(String::new()));
        let modified = Rc::new(Cell::new(false));

        // ── Main toolbar ─────────────────────────────────────────────────
        let toolbar = gtk4::Box::new(gtk4::Orientation::Horizontal, 2);
        toolbar.add_css_class("markdown-toolbar");
        toolbar.set_margin_start(4);
        toolbar.set_margin_end(4);
        toolbar.set_margin_top(2);
        toolbar.set_margin_bottom(2);

        let render_btn = gtk4::ToggleButton::new();
        render_btn.set_icon_name("emblem-documents-symbolic");
        render_btn.set_active(true);
        render_btn.add_css_class("flat");
        render_btn.set_tooltip_text(Some("Render view"));

        let edit_btn = gtk4::ToggleButton::new();
        edit_btn.set_icon_name("document-edit-symbolic");
        edit_btn.add_css_class("flat");
        edit_btn.set_tooltip_text(Some("Edit mode"));
        edit_btn.set_group(Some(&render_btn));

        let save_indicator = gtk4::Image::from_icon_name("media-floppy-symbolic");
        save_indicator.set_visible(false);
        save_indicator.set_tooltip_text(Some("Unsaved changes"));
        save_indicator.add_css_class("dirty-indicator");

        let reload_btn = gtk4::Button::new();
        reload_btn.set_icon_name("view-refresh-symbolic");
        reload_btn.add_css_class("flat");
        reload_btn.set_tooltip_text(Some("Reload from disk"));

        let file_label = gtk4::Label::new(Some(file_path));
        file_label.add_css_class("dim-label");
        file_label.add_css_class("caption");
        file_label.set_hexpand(true);
        file_label.set_halign(gtk4::Align::End);
        file_label.set_ellipsize(gtk4::pango::EllipsizeMode::Start);

        toolbar.append(&render_btn);
        toolbar.append(&edit_btn);
        toolbar.append(&gtk4::Separator::new(gtk4::Orientation::Vertical));
        toolbar.append(&save_indicator);
        toolbar.append(&reload_btn);
        toolbar.append(&file_label);
        container.append(&toolbar);

        // ── Formatting toolbar (visible only in edit mode) ───────────────
        let fmt_bar = gtk4::Box::new(gtk4::Orientation::Horizontal, 1);
        fmt_bar.add_css_class("markdown-toolbar");
        fmt_bar.set_margin_start(4);
        fmt_bar.set_margin_end(4);
        fmt_bar.set_visible(false);

        let fmt_buttons: Vec<(&str, &str, &str)> = vec![
            ("format-text-bold-symbolic", "Bold (**text**)", "**"),
            ("format-text-italic-symbolic", "Italic (*text*)", "*"),
            ("format-text-strikethrough-symbolic", "Strikethrough (~~text~~)", "~~"),
            ("accessories-text-editor-symbolic", "Inline code (`code`)", "`"),
        ];

        let text_view_for_fmt = Rc::new(RefCell::new(None::<gtk4::TextView>));

        for (icon, tooltip, marker) in &fmt_buttons {
            let btn = gtk4::Button::new();
            btn.set_icon_name(icon);
            btn.add_css_class("flat");
            btn.set_tooltip_text(Some(tooltip));
            let m = marker.to_string();
            let tv_ref = text_view_for_fmt.clone();
            btn.connect_clicked(move |_| {
                if let Some(ref tv) = *tv_ref.borrow() {
                    wrap_selection(tv, &m);
                }
            });
            fmt_bar.append(&btn);
        }

        fmt_bar.append(&gtk4::Separator::new(gtk4::Orientation::Vertical));

        // Heading buttons
        for (level, label) in &[(1, "H1"), (2, "H2"), (3, "H3")] {
            let btn = gtk4::Button::with_label(label);
            btn.add_css_class("flat");
            btn.set_tooltip_text(Some(&format!("Heading {}", level)));
            let prefix = "#".repeat(*level);
            let tv_ref = text_view_for_fmt.clone();
            btn.connect_clicked(move |_| {
                if let Some(ref tv) = *tv_ref.borrow() {
                    prepend_line(tv, &format!("{} ", prefix));
                }
            });
            fmt_bar.append(&btn);
        }

        fmt_bar.append(&gtk4::Separator::new(gtk4::Orientation::Vertical));

        // Special inserts
        let special = vec![
            ("view-list-symbolic", "Bullet list", "- "),
            ("view-list-ordered-symbolic", "Numbered list", "1. "),
            ("mail-attachment-symbolic", "Link", "[text](url)"),
            ("insert-image-symbolic", "Image", "![alt](url)"),
            ("view-grid-symbolic", "Table", "| Col 1 | Col 2 |\n|--------|--------|\n| data   | data   |"),
            ("format-text-blockquote-symbolic", "Blockquote", "> "),
            ("view-more-horizontal-symbolic", "Horizontal rule", "\n---\n"),
            ("utilities-terminal-symbolic", "Code block", "```\n\n```"),
        ];

        for (icon, tooltip, insert_text) in &special {
            let btn = gtk4::Button::new();
            btn.set_icon_name(icon);
            btn.add_css_class("flat");
            btn.set_tooltip_text(Some(tooltip));
            let text = insert_text.to_string();
            let tv_ref = text_view_for_fmt.clone();
            btn.connect_clicked(move |_| {
                if let Some(ref tv) = *tv_ref.borrow() {
                    insert_at_cursor(tv, &text);
                }
            });
            fmt_bar.append(&btn);
        }

        container.append(&fmt_bar);

        // ── Text view ────────────────────────────────────────────────────
        let text_view = gtk4::TextView::new();
        text_view.set_editable(false);
        text_view.set_cursor_visible(false);
        text_view.set_wrap_mode(gtk4::WrapMode::Word);
        text_view.set_left_margin(12);
        text_view.set_right_margin(12);
        text_view.set_top_margin(8);
        text_view.set_bottom_margin(8);
        text_view.add_css_class("markdown-panel");

        *text_view_for_fmt.borrow_mut() = Some(text_view.clone());

        let scrolled = gtk4::ScrolledWindow::new();
        scrolled.set_child(Some(&text_view));
        scrolled.set_vexpand(true);
        scrolled.set_hexpand(true);
        container.append(&scrolled);

        let widget = container.upcast::<gtk4::Widget>();

        // Load initial content
        let initial = std::fs::read_to_string(file_path)
            .unwrap_or_else(|e| format!("Error loading {}: {}", file_path, e));
        *content.borrow_mut() = initial.clone();
        render_markdown_to_view(&text_view, &initial);

        // ── Render button ────────────────────────────────────────────────
        {
            let tv = text_view.clone();
            let ct = content.clone();
            let m = mode.clone();
            let fb = fmt_bar.clone();
            let mod_flag = modified.clone();
            let fp = file_path.to_string();
            let si = save_indicator.clone();
            render_btn.connect_toggled(move |btn| {
                if !btn.is_active() { return; }
                // If coming from edit, save content from buffer
                if m.get() == Mode::Edit {
                    let buf = tv.buffer();
                    let text = buf.text(&buf.start_iter(), &buf.end_iter(), false).to_string();
                    *ct.borrow_mut() = text.clone();
                    // Auto-save
                    if mod_flag.get() {
                        let _ = std::fs::write(&fp, &text);
                        mod_flag.set(false);
                        si.set_visible(false);
                    }
                }
                m.set(Mode::Render);
                tv.set_editable(false);
                tv.set_cursor_visible(false);
                fb.set_visible(false);
                render_markdown_to_view(&tv, &ct.borrow());
            });
        }

        // ── Edit button ──────────────────────────────────────────────────
        {
            let tv = text_view.clone();
            let ct = content.clone();
            let m = mode.clone();
            let fb = fmt_bar.clone();
            edit_btn.connect_toggled(move |btn| {
                if !btn.is_active() { return; }
                m.set(Mode::Edit);
                tv.set_editable(true);
                tv.set_cursor_visible(true);
                fb.set_visible(true);
                tv.buffer().set_text(&ct.borrow());
            });
        }

        // ── Track modifications in edit mode ─────────────────────────────
        {
            let mod_flag = modified.clone();
            let si = save_indicator.clone();
            let m = mode.clone();
            text_view.buffer().connect_changed(move |_| {
                if m.get() == Mode::Edit && !mod_flag.get() {
                    mod_flag.set(true);
                    si.set_visible(true);
                }
            });
        }

        // ── Reload button ────────────────────────────────────────────────
        {
            let fp = file_path.to_string();
            let ct = content.clone();
            let tv = text_view.clone();
            let m = mode.clone();
            let mod_flag = modified.clone();
            let si = save_indicator.clone();
            reload_btn.connect_clicked(move |_| {
                if let Ok(text) = std::fs::read_to_string(&fp) {
                    *ct.borrow_mut() = text.clone();
                    mod_flag.set(false);
                    si.set_visible(false);
                    if m.get() == Mode::Render {
                        render_markdown_to_view(&tv, &text);
                    } else {
                        tv.buffer().set_text(&text);
                    }
                }
            });
        }

        // ── File watch (poll every 2s, only in render mode) ──────────────
        {
            let fp = file_path.to_string();
            let ct = content.clone();
            let tv = text_view.clone();
            let m = mode.clone();
            let last_mtime = Rc::new(Cell::new(get_mtime(file_path)));

            glib::timeout_add_local(std::time::Duration::from_secs(2), move || {
                if m.get() == Mode::Edit { return glib::ControlFlow::Continue; }
                let mtime = get_mtime(&fp);
                if mtime != last_mtime.get() {
                    last_mtime.set(mtime);
                    if let Ok(text) = std::fs::read_to_string(&fp) {
                        *ct.borrow_mut() = text.clone();
                        render_markdown_to_view(&tv, &text);
                    }
                }
                glib::ControlFlow::Continue
            });
        }

        Self { widget, text_view, file_path: file_path.to_string() }
    }

    pub fn reload(&mut self) {
        if let Ok(text) = std::fs::read_to_string(&self.file_path) {
            render_markdown_to_view(&self.text_view, &text);
        }
    }
}

impl PanelBackend for MarkdownPanel {
    fn panel_type(&self) -> &str { "markdown" }
    fn widget(&self) -> &gtk4::Widget { &self.widget }
    fn on_focus(&self) { self.text_view.grab_focus(); }
}

// ── Formatting helpers ───────────────────────────────────────────────────────

fn wrap_selection(tv: &gtk4::TextView, marker: &str) {
    let buf = tv.buffer();
    if let Some((start, end)) = buf.selection_bounds() {
        let text = buf.text(&start, &end, false).to_string();
        let replacement = format!("{}{}{}", marker, text, marker);
        buf.delete(&mut start.clone(), &mut end.clone());
        buf.insert(&mut buf.iter_at_offset(start.offset()), &replacement);
    } else {
        let mut iter = buf.iter_at_mark(&buf.get_insert());
        buf.insert(&mut iter, &format!("{}text{}", marker, marker));
    }
}

fn prepend_line(tv: &gtk4::TextView, prefix: &str) {
    let buf = tv.buffer();
    let mut iter = buf.iter_at_mark(&buf.get_insert());
    iter.set_line_offset(0);
    buf.insert(&mut iter, prefix);
}

fn insert_at_cursor(tv: &gtk4::TextView, text: &str) {
    let buf = tv.buffer();
    let mut iter = buf.iter_at_mark(&buf.get_insert());
    buf.insert(&mut iter, text);
}

fn get_mtime(path: &str) -> u64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .map(|t| t.duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs())
        .unwrap_or(0)
}

// ── Markdown rendering ───────────────────────────────────────────────────────

fn render_markdown_to_view(text_view: &gtk4::TextView, content: &str) {
    let buffer = text_view.buffer();
    buffer.set_text("");
    let tag_table = buffer.tag_table();

    let ensure_tag = |name: &str, setup: &dyn Fn(&gtk4::TextTag)| {
        if tag_table.lookup(name).is_none() {
            let tag = gtk4::TextTag::new(Some(name));
            setup(&tag);
            tag_table.add(&tag);
        }
    };

    ensure_tag("h1", &|t| { t.set_size_points(20.0); t.set_weight(700); });
    ensure_tag("h2", &|t| { t.set_size_points(16.0); t.set_weight(700); });
    ensure_tag("h3", &|t| { t.set_size_points(14.0); t.set_weight(700); });
    ensure_tag("bold", &|t| { t.set_weight(700); });
    ensure_tag("italic", &|t| { t.set_style(gtk4::pango::Style::Italic); });
    ensure_tag("strike", &|t| { t.set_strikethrough(true); });
    ensure_tag("code", &|t| { t.set_family(Some("monospace")); });
    ensure_tag("code_block", &|t| {
        t.set_family(Some("monospace"));
        t.set_paragraph_background(Some("#2a2a2a"));
        t.set_left_margin(20);
    });
    ensure_tag("link", &|t| {
        t.set_foreground(Some("#5588ff"));
        t.set_underline(gtk4::pango::Underline::Single);
    });
    ensure_tag("bullet", &|t| { t.set_left_margin(20); });
    ensure_tag("blockquote", &|t| {
        t.set_left_margin(20);
        t.set_style(gtk4::pango::Style::Italic);
        t.set_foreground(Some("#888888"));
    });
    ensure_tag("separator", &|t| {
        t.set_foreground(Some("#666666"));
        t.set_size_points(6.0);
    });

    let mut iter = buffer.end_iter();
    let mut in_code_block = false;

    for line in content.lines() {
        if line.starts_with("```") {
            in_code_block = !in_code_block;
            if in_code_block {
                let lang = line.trim_start_matches('`').trim();
                if !lang.is_empty() {
                    buffer.insert_with_tags_by_name(&mut iter, &format!("─── {} ───\n", lang), &["separator"]);
                }
            } else {
                buffer.insert_with_tags_by_name(&mut iter, "───────\n", &["separator"]);
            }
            continue;
        }
        if in_code_block {
            buffer.insert_with_tags_by_name(&mut iter, &format!("{}\n", line), &["code_block"]);
            continue;
        }
        if line.starts_with("### ") {
            buffer.insert_with_tags_by_name(&mut iter, &format!("{}\n", &line[4..]), &["h3"]);
        } else if line.starts_with("## ") {
            buffer.insert_with_tags_by_name(&mut iter, &format!("{}\n", &line[3..]), &["h2"]);
        } else if line.starts_with("# ") {
            buffer.insert_with_tags_by_name(&mut iter, &format!("{}\n", &line[2..]), &["h1"]);
        } else if line.starts_with("---") || line.starts_with("***") || line.starts_with("___") {
            buffer.insert_with_tags_by_name(&mut iter, "────────────────────────────────\n", &["separator"]);
        } else if line.starts_with("- ") || line.starts_with("* ") {
            buffer.insert_with_tags_by_name(&mut iter, &format!("  • {}\n", &line[2..]), &["bullet"]);
        } else if line.starts_with("> ") {
            buffer.insert_with_tags_by_name(&mut iter, &format!("│ {}\n", &line[2..]), &["blockquote"]);
        } else {
            render_inline(&buffer, &mut iter, line);
            buffer.insert(&mut iter, "\n");
        }
    }
}

fn render_inline(buffer: &gtk4::TextBuffer, iter: &mut gtk4::TextIter, text: &str) {
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    let mut i = 0;
    let mut plain = String::new();

    while i < len {
        // ~~strikethrough~~
        if i + 1 < len && chars[i] == '~' && chars[i + 1] == '~' {
            flush_plain(buffer, iter, &mut plain);
            i += 2;
            let start = i;
            while i + 1 < len && !(chars[i] == '~' && chars[i + 1] == '~') { i += 1; }
            let t: String = chars[start..i].iter().collect();
            buffer.insert_with_tags_by_name(iter, &t, &["strike"]);
            if i + 1 < len { i += 2; }
        }
        // **bold**
        else if i + 1 < len && chars[i] == '*' && chars[i + 1] == '*' {
            flush_plain(buffer, iter, &mut plain);
            i += 2;
            let start = i;
            while i + 1 < len && !(chars[i] == '*' && chars[i + 1] == '*') { i += 1; }
            let t: String = chars[start..i].iter().collect();
            buffer.insert_with_tags_by_name(iter, &t, &["bold"]);
            if i + 1 < len { i += 2; }
        }
        // *italic*
        else if chars[i] == '*' && (i + 1 >= len || chars[i + 1] != '*') {
            flush_plain(buffer, iter, &mut plain);
            i += 1;
            let start = i;
            while i < len && chars[i] != '*' { i += 1; }
            let t: String = chars[start..i].iter().collect();
            buffer.insert_with_tags_by_name(iter, &t, &["italic"]);
            if i < len { i += 1; }
        }
        // `code`
        else if chars[i] == '`' {
            flush_plain(buffer, iter, &mut plain);
            i += 1;
            let start = i;
            while i < len && chars[i] != '`' { i += 1; }
            let t: String = chars[start..i].iter().collect();
            buffer.insert_with_tags_by_name(iter, &t, &["code"]);
            if i < len { i += 1; }
        }
        // [link](url)
        else if chars[i] == '[' {
            flush_plain(buffer, iter, &mut plain);
            i += 1;
            let start = i;
            while i < len && chars[i] != ']' { i += 1; }
            let link_text: String = chars[start..i].iter().collect();
            if i + 1 < len && chars[i] == ']' && chars[i + 1] == '(' {
                i += 2;
                while i < len && chars[i] != ')' { i += 1; }
                if i < len { i += 1; }
            } else if i < len { i += 1; }
            buffer.insert_with_tags_by_name(iter, &link_text, &["link"]);
        }
        // ![image](url)
        else if chars[i] == '!' && i + 1 < len && chars[i + 1] == '[' {
            flush_plain(buffer, iter, &mut plain);
            i += 2;
            let start = i;
            while i < len && chars[i] != ']' { i += 1; }
            let alt: String = chars[start..i].iter().collect();
            if i + 1 < len && chars[i] == ']' && chars[i + 1] == '(' {
                i += 2;
                while i < len && chars[i] != ')' { i += 1; }
                if i < len { i += 1; }
            } else if i < len { i += 1; }
            buffer.insert_with_tags_by_name(iter, &format!("[img: {}]", alt), &["link"]);
        }
        else {
            plain.push(chars[i]);
            i += 1;
        }
    }
    flush_plain(buffer, iter, &mut plain);
}

fn flush_plain(buffer: &gtk4::TextBuffer, iter: &mut gtk4::TextIter, plain: &mut String) {
    if !plain.is_empty() {
        buffer.insert(iter, plain);
        plain.clear();
    }
}
