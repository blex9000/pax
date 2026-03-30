use gtk4::prelude::*;
use gtk4::glib;
use sourceview5::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use super::PanelBackend;

#[derive(Debug, Clone, Copy, PartialEq)]
enum Mode { Render, Edit }

/// Markdown viewer/editor panel with GtkSourceView highlighting.
#[derive(Debug)]
pub struct MarkdownPanel {
    widget: gtk4::Widget,
    render_view: gtk4::TextView,
    source_view: sourceview5::View,
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

        let undo_btn = gtk4::Button::new();
        undo_btn.set_icon_name("edit-undo-symbolic");
        undo_btn.add_css_class("flat");
        undo_btn.set_tooltip_text(Some("Undo"));
        undo_btn.set_sensitive(false);

        let redo_btn = gtk4::Button::new();
        redo_btn.set_icon_name("edit-redo-symbolic");
        redo_btn.add_css_class("flat");
        redo_btn.set_tooltip_text(Some("Redo"));
        redo_btn.set_sensitive(false);

        let save_btn = gtk4::Button::new();
        save_btn.set_icon_name("media-floppy-symbolic");
        save_btn.set_sensitive(false);
        save_btn.set_tooltip_text(Some("Save"));
        save_btn.add_css_class("flat");

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
        toolbar.append(&undo_btn);
        toolbar.append(&redo_btn);
        toolbar.append(&gtk4::Separator::new(gtk4::Orientation::Vertical));
        toolbar.append(&save_btn);
        toolbar.append(&reload_btn);
        toolbar.append(&file_label);
        container.append(&toolbar);

        // ── Formatting toolbar (edit mode only) ──────────────────────────
        let fmt_bar = gtk4::Box::new(gtk4::Orientation::Horizontal, 1);
        fmt_bar.add_css_class("markdown-toolbar");
        fmt_bar.set_margin_start(4);
        fmt_bar.set_margin_end(4);
        fmt_bar.set_visible(false);

        let source_view_for_fmt = Rc::new(RefCell::new(None::<sourceview5::View>));

        let fmt_buttons: Vec<(&str, &str, &str)> = vec![
            ("format-text-bold-symbolic", "Bold (**text**)", "**"),
            ("format-text-italic-symbolic", "Italic (*text*)", "*"),
            ("format-text-strikethrough-symbolic", "Strikethrough (~~text~~)", "~~"),
            ("accessories-text-editor-symbolic", "Inline code (`code`)", "`"),
        ];

        for (icon, tooltip, marker) in &fmt_buttons {
            let btn = gtk4::Button::new();
            btn.set_icon_name(icon);
            btn.add_css_class("flat");
            btn.set_tooltip_text(Some(tooltip));
            let m = marker.to_string();
            let sv_ref = source_view_for_fmt.clone();
            btn.connect_clicked(move |_| {
                if let Some(ref sv) = *sv_ref.borrow() {
                    wrap_selection_sv(sv, &m);
                }
            });
            fmt_bar.append(&btn);
        }

        fmt_bar.append(&gtk4::Separator::new(gtk4::Orientation::Vertical));

        for (level, label) in &[(1, "H1"), (2, "H2"), (3, "H3")] {
            let btn = gtk4::Button::with_label(label);
            btn.add_css_class("flat");
            let prefix = "#".repeat(*level);
            let sv_ref = source_view_for_fmt.clone();
            btn.connect_clicked(move |_| {
                if let Some(ref sv) = *sv_ref.borrow() {
                    prepend_line_sv(sv, &format!("{} ", prefix));
                }
            });
            fmt_bar.append(&btn);
        }

        fmt_bar.append(&gtk4::Separator::new(gtk4::Orientation::Vertical));

        let special = vec![
            ("view-list-symbolic", "Bullet list", "- "),
            ("mail-attachment-symbolic", "Link", "[text](url)"),
            ("insert-image-symbolic", "Image", "![alt](url)"),
            ("view-grid-symbolic", "Table", "| Col 1 | Col 2 |\n|--------|--------|\n| data   | data   |"),
            ("utilities-terminal-symbolic", "Code block", "```\n\n```"),
        ];

        for (icon, tooltip, insert_text) in &special {
            let btn = gtk4::Button::new();
            btn.set_icon_name(icon);
            btn.add_css_class("flat");
            btn.set_tooltip_text(Some(tooltip));
            let text = insert_text.to_string();
            let sv_ref = source_view_for_fmt.clone();
            btn.connect_clicked(move |_| {
                if let Some(ref sv) = *sv_ref.borrow() {
                    insert_at_cursor_sv(sv, &text);
                }
            });
            fmt_bar.append(&btn);
        }

        container.append(&fmt_bar);

        // ── Stack: render view + source view ─────────────────────────────
        let stack = gtk4::Stack::new();

        // Render view (read-only TextView for rendered markdown)
        let render_view = gtk4::TextView::new();
        render_view.set_editable(false);
        render_view.set_cursor_visible(false);
        render_view.set_wrap_mode(gtk4::WrapMode::Word);
        render_view.set_left_margin(12);
        render_view.set_right_margin(12);
        render_view.set_top_margin(8);
        render_view.set_bottom_margin(8);
        render_view.add_css_class("markdown-panel");

        let render_scroll = gtk4::ScrolledWindow::new();
        render_scroll.set_child(Some(&render_view));
        render_scroll.set_vexpand(true);
        render_scroll.set_hexpand(true);
        stack.add_named(&render_scroll, Some("render"));

        // Source view (GtkSourceView with markdown highlighting)
        let source_buffer = sourceview5::Buffer::new(None::<&gtk4::TextTagTable>);
        if let Some(lang) = sourceview5::LanguageManager::default()
            .language("markdown")
        {
            source_buffer.set_language(Some(&lang));
        }
        source_buffer.set_highlight_syntax(true);

        // Use a dark scheme if available
        let scheme_manager = sourceview5::StyleSchemeManager::default();
        if let Some(scheme) = scheme_manager.scheme("Adwaita-dark")
            .or_else(|| scheme_manager.scheme("classic-dark"))
            .or_else(|| scheme_manager.scheme("oblivion"))
        {
            source_buffer.set_style_scheme(Some(&scheme));
        }

        let source_view = sourceview5::View::with_buffer(&source_buffer);
        source_view.set_show_line_numbers(true);
        source_view.set_highlight_current_line(true);
        source_view.set_auto_indent(true);
        source_view.set_indent_on_tab(true);
        source_view.set_tab_width(4);
        source_view.set_wrap_mode(gtk4::WrapMode::Word);
        source_view.set_left_margin(8);
        source_view.set_top_margin(4);
        source_view.set_monospace(true);

        *source_view_for_fmt.borrow_mut() = Some(source_view.clone());

        let source_scroll = gtk4::ScrolledWindow::new();
        source_scroll.set_child(Some(&source_view));
        source_scroll.set_vexpand(true);
        source_scroll.set_hexpand(true);
        stack.add_named(&source_scroll, Some("edit"));

        stack.set_visible_child_name("render");
        container.append(&stack);

        let widget = container.upcast::<gtk4::Widget>();

        // Load initial content
        let initial = std::fs::read_to_string(file_path)
            .unwrap_or_else(|e| format!("Error loading {}: {}", file_path, e));
        *content.borrow_mut() = initial.clone();
        render_markdown_to_view(&render_view, &initial);

        // ── Render button ────────────────────────────────────────────────
        {
            let rv = render_view.clone();
            let ct = content.clone();
            let m = mode.clone();
            let fb = fmt_bar.clone();
            let mod_flag = modified.clone();
            let fp = file_path.to_string();
            let sb = save_btn.clone();
            let st = stack.clone();
            let sv = source_view.clone();
            let ub = undo_btn.clone();
            let rb = redo_btn.clone();
            render_btn.connect_toggled(move |btn| {
                if !btn.is_active() { return; }
                if m.get() == Mode::Edit {
                    let buf = sv.buffer();
                    let text = buf.text(&buf.start_iter(), &buf.end_iter(), false).to_string();
                    *ct.borrow_mut() = text.clone();
                    if mod_flag.get() {
                        let _ = std::fs::write(&fp, &text);
                        mod_flag.set(false);
                        sb.set_sensitive(false);
                    }
                }
                m.set(Mode::Render);
                fb.set_visible(false);
                ub.set_sensitive(false);
                rb.set_sensitive(false);
                st.set_visible_child_name("render");
                render_markdown_to_view(&rv, &ct.borrow());
            });
        }

        // ── Edit button ──────────────────────────────────────────────────
        {
            let ct = content.clone();
            let m = mode.clone();
            let fb = fmt_bar.clone();
            let st = stack.clone();
            let sv = source_view.clone();
            let ub = undo_btn.clone();
            let rb = redo_btn.clone();
            edit_btn.connect_toggled(move |btn| {
                if !btn.is_active() { return; }
                m.set(Mode::Edit);
                fb.set_visible(true);
                let buf = sv.buffer();
                buf.set_text(&ct.borrow());
                buf.set_modified(false);
                ub.set_sensitive(buf.can_undo());
                rb.set_sensitive(buf.can_redo());
                st.set_visible_child_name("edit");
                sv.grab_focus();
            });
        }

        // ── Undo/Redo ────────────────────────────────────────────────────
        {
            let sv = source_view.clone();
            let ub2 = undo_btn.clone();
            let rb2 = redo_btn.clone();
            undo_btn.connect_clicked(move |_| {
                let buf = sv.buffer();
                buf.undo();
                ub2.set_sensitive(buf.can_undo());
                rb2.set_sensitive(buf.can_redo());
            });
        }
        {
            let sv = source_view.clone();
            let ub2 = undo_btn.clone();
            let rb2 = redo_btn.clone();
            redo_btn.connect_clicked(move |_| {
                let buf = sv.buffer();
                buf.redo();
                ub2.set_sensitive(buf.can_undo());
                rb2.set_sensitive(buf.can_redo());
            });
        }

        // ── Track undo/redo state ────────────────────────────────────────
        {
            let ub = undo_btn.clone();
            let m = mode.clone();
            source_buffer.connect_notify_local(Some("can-undo"), move |buf, _| {
                ub.set_sensitive(m.get() == Mode::Edit && buf.can_undo());
            });
        }
        {
            let rb = redo_btn.clone();
            let m = mode.clone();
            source_buffer.connect_notify_local(Some("can-redo"), move |buf, _| {
                rb.set_sensitive(m.get() == Mode::Edit && buf.can_redo());
            });
        }

        // ── Track save state (compare with saved content) ────────────────
        {
            let sb = save_btn.clone();
            let ct = content.clone();
            let m = mode.clone();
            let mod_flag = modified.clone();
            source_buffer.connect_changed(move |buf| {
                if m.get() != Mode::Edit { return; }
                let current = buf.text(&buf.start_iter(), &buf.end_iter(), false).to_string();
                let dirty = current != *ct.borrow();
                mod_flag.set(dirty);
                sb.set_sensitive(dirty);
            });
        }

        // ── Save button ─────────────────────────────────────────────────
        {
            let fp = file_path.to_string();
            let ct = content.clone();
            let sv = source_view.clone();
            let mod_flag = modified.clone();
            let sb2 = save_btn.clone();
            save_btn.connect_clicked(move |_| {
                let buf = sv.buffer();
                let text = buf.text(&buf.start_iter(), &buf.end_iter(), false).to_string();
                *ct.borrow_mut() = text.clone();
                let _ = std::fs::write(&fp, &text);
                mod_flag.set(false);
                sb2.set_sensitive(false);
            });
        }

        // ── Reload button ────────────────────────────────────────────────
        {
            let fp = file_path.to_string();
            let ct = content.clone();
            let rv = render_view.clone();
            let sv = source_view.clone();
            let m = mode.clone();
            let mod_flag = modified.clone();
            let sb = save_btn.clone();
            reload_btn.connect_clicked(move |_| {
                if let Ok(text) = std::fs::read_to_string(&fp) {
                    *ct.borrow_mut() = text.clone();
                    mod_flag.set(false);
                    sb.set_sensitive(false);
                    if m.get() == Mode::Render {
                        render_markdown_to_view(&rv, &text);
                    } else {
                        sv.buffer().set_text(&text);
                    }
                }
            });
        }

        // ── File watch (500ms, render mode only) ─────────────────────────
        {
            let fp = file_path.to_string();
            let ct = content.clone();
            let rv = render_view.clone();
            let m = mode.clone();
            let last_mtime = Rc::new(Cell::new(get_mtime(file_path)));

            glib::timeout_add_local(std::time::Duration::from_millis(500), move || {
                if m.get() == Mode::Edit { return glib::ControlFlow::Continue; }
                let mtime = get_mtime(&fp);
                if mtime != last_mtime.get() {
                    last_mtime.set(mtime);
                    if let Ok(text) = std::fs::read_to_string(&fp) {
                        if text != *ct.borrow() {
                            *ct.borrow_mut() = text.clone();
                            render_markdown_to_view(&rv, &text);
                        }
                    }
                }
                glib::ControlFlow::Continue
            });
        }

        Self {
            widget,
            render_view,
            source_view,
            file_path: file_path.to_string(),
        }
    }

    pub fn reload(&mut self) {
        if let Ok(text) = std::fs::read_to_string(&self.file_path) {
            render_markdown_to_view(&self.render_view, &text);
        }
    }
}

impl PanelBackend for MarkdownPanel {
    fn panel_type(&self) -> &str { "markdown" }
    fn widget(&self) -> &gtk4::Widget { &self.widget }
    fn on_focus(&self) { self.source_view.grab_focus(); }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn wrap_selection_sv(sv: &sourceview5::View, marker: &str) {
    let buf = sv.buffer();
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

fn prepend_line_sv(sv: &sourceview5::View, prefix: &str) {
    let buf = sv.buffer();
    let mut iter = buf.iter_at_mark(&buf.get_insert());
    iter.set_line_offset(0);
    buf.insert(&mut iter, prefix);
}

fn insert_at_cursor_sv(sv: &sourceview5::View, text: &str) {
    let buf = sv.buffer();
    let mut iter = buf.iter_at_mark(&buf.get_insert());
    buf.insert(&mut iter, text);
}

fn get_mtime(path: &str) -> u64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .map(|t| t.duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs())
        .unwrap_or(0)
}

// ── Markdown rendering (for render mode) ─────────────────────────────────────

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
        if i + 1 < len && chars[i] == '~' && chars[i + 1] == '~' {
            flush_plain(buffer, iter, &mut plain);
            i += 2;
            let start = i;
            while i + 1 < len && !(chars[i] == '~' && chars[i + 1] == '~') { i += 1; }
            let t: String = chars[start..i].iter().collect();
            buffer.insert_with_tags_by_name(iter, &t, &["strike"]);
            if i + 1 < len { i += 2; }
        } else if i + 1 < len && chars[i] == '*' && chars[i + 1] == '*' {
            flush_plain(buffer, iter, &mut plain);
            i += 2;
            let start = i;
            while i + 1 < len && !(chars[i] == '*' && chars[i + 1] == '*') { i += 1; }
            let t: String = chars[start..i].iter().collect();
            buffer.insert_with_tags_by_name(iter, &t, &["bold"]);
            if i + 1 < len { i += 2; }
        } else if chars[i] == '*' && (i + 1 >= len || chars[i + 1] != '*') {
            flush_plain(buffer, iter, &mut plain);
            i += 1;
            let start = i;
            while i < len && chars[i] != '*' { i += 1; }
            let t: String = chars[start..i].iter().collect();
            buffer.insert_with_tags_by_name(iter, &t, &["italic"]);
            if i < len { i += 1; }
        } else if chars[i] == '`' {
            flush_plain(buffer, iter, &mut plain);
            i += 1;
            let start = i;
            while i < len && chars[i] != '`' { i += 1; }
            let t: String = chars[start..i].iter().collect();
            buffer.insert_with_tags_by_name(iter, &t, &["code"]);
            if i < len { i += 1; }
        } else if chars[i] == '[' {
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
        } else if chars[i] == '!' && i + 1 < len && chars[i + 1] == '[' {
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
        } else {
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
