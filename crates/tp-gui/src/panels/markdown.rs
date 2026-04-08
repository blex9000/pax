use gtk4::glib;
use gtk4::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

#[cfg(feature = "sourceview")]
use sourceview5::prelude::*;

use super::PanelBackend;

#[derive(Debug, Clone, Copy, PartialEq)]
enum Mode {
    Render,
    Edit,
}

/// Markdown viewer/editor panel.
/// Uses GtkSourceView 5 for edit mode when available (Linux),
/// falls back to plain TextView on macOS/no-sourceview.
#[derive(Debug)]
pub struct MarkdownPanel {
    widget: gtk4::Widget,
    render_view: gtk4::TextView,
    #[cfg(feature = "sourceview")]
    source_view: sourceview5::View,
    #[cfg(not(feature = "sourceview"))]
    source_view: gtk4::TextView,
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
        undo_btn.set_sensitive(false);

        let redo_btn = gtk4::Button::new();
        redo_btn.set_icon_name("edit-redo-symbolic");
        redo_btn.add_css_class("flat");
        redo_btn.set_sensitive(false);

        let save_btn = gtk4::Button::new();
        save_btn.set_icon_name("media-floppy-symbolic");
        save_btn.set_sensitive(false);
        save_btn.add_css_class("flat");

        let reload_btn = gtk4::Button::new();
        reload_btn.set_icon_name("view-refresh-symbolic");
        reload_btn.add_css_class("flat");

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

        let edit_buf_ref: Rc<RefCell<Option<gtk4::TextBuffer>>> = Rc::new(RefCell::new(None));

        let fmt_items: Vec<(&str, &str, &str)> = vec![
            ("format-text-bold-symbolic", "Bold", "**"),
            ("format-text-italic-symbolic", "Italic", "*"),
            ("accessories-text-editor-symbolic", "Code", "`"),
        ];
        for (icon, tooltip, marker) in &fmt_items {
            let btn = gtk4::Button::new();
            btn.set_icon_name(icon);
            btn.add_css_class("flat");
            btn.set_tooltip_text(Some(tooltip));
            let m = marker.to_string();
            let br = edit_buf_ref.clone();
            btn.connect_clicked(move |_| {
                if let Some(ref buf) = *br.borrow() {
                    wrap_selection_buf(buf, &m);
                }
            });
            fmt_bar.append(&btn);
        }
        fmt_bar.append(&gtk4::Separator::new(gtk4::Orientation::Vertical));
        for (level, label) in &[(1, "H1"), (2, "H2"), (3, "H3")] {
            let btn = gtk4::Button::with_label(label);
            btn.add_css_class("flat");
            let prefix = "#".repeat(*level);
            let br = edit_buf_ref.clone();
            btn.connect_clicked(move |_| {
                if let Some(ref buf) = *br.borrow() {
                    prepend_line_buf(buf, &format!("{} ", prefix));
                }
            });
            fmt_bar.append(&btn);
        }
        fmt_bar.append(&gtk4::Separator::new(gtk4::Orientation::Vertical));
        for (icon, tooltip, text) in &[
            ("view-list-symbolic", "List", "- "),
            ("mail-attachment-symbolic", "Link", "[text](url)"),
            ("utilities-terminal-symbolic", "Code block", "```\n\n```"),
        ] {
            let btn = gtk4::Button::new();
            btn.set_icon_name(icon);
            btn.add_css_class("flat");
            btn.set_tooltip_text(Some(tooltip));
            let t = text.to_string();
            let br = edit_buf_ref.clone();
            btn.connect_clicked(move |_| {
                if let Some(ref buf) = *br.borrow() {
                    insert_at_cursor_buf(buf, &t);
                }
            });
            fmt_bar.append(&btn);
        }
        container.append(&fmt_bar);

        // ── Stack: render + edit ─────────────────────────────────────────
        let stack = gtk4::Stack::new();

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

        // Edit view — sourceview5 or plain TextView
        #[cfg(feature = "sourceview")]
        let (source_view, source_buffer) = {
            let buf = sourceview5::Buffer::new(None::<&gtk4::TextTagTable>);
            if let Some(lang) = sourceview5::LanguageManager::default().language("markdown") {
                buf.set_language(Some(&lang));
            }
            buf.set_highlight_syntax(true);
            crate::theme::register_sourceview_buffer(&buf);
            let sv = sourceview5::View::with_buffer(&buf);
            sv.set_show_line_numbers(true);
            sv.set_highlight_current_line(true);
            sv.set_auto_indent(true);
            sv.set_tab_width(4);
            sv.set_wrap_mode(gtk4::WrapMode::Word);
            sv.set_left_margin(8);
            sv.set_top_margin(4);
            sv.set_monospace(true);
            (sv, buf.upcast::<gtk4::TextBuffer>())
        };

        #[cfg(not(feature = "sourceview"))]
        let (source_view, source_buffer) = {
            let tv = gtk4::TextView::new();
            tv.set_wrap_mode(gtk4::WrapMode::Word);
            tv.set_left_margin(8);
            tv.set_top_margin(4);
            tv.set_monospace(true);
            let buf = tv.buffer();
            buf.set_enable_undo(true);
            (tv, buf)
        };

        *edit_buf_ref.borrow_mut() = Some(source_buffer.clone());

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
            let sbuf = source_buffer.clone();
            let ub = undo_btn.clone();
            let rb = redo_btn.clone();
            render_btn.connect_toggled(move |btn| {
                if !btn.is_active() {
                    return;
                }
                if m.get() == Mode::Edit {
                    let text = sbuf
                        .text(&sbuf.start_iter(), &sbuf.end_iter(), false)
                        .to_string();
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
            let sbuf = source_buffer.clone();
            let sv = source_view.clone();
            let ub = undo_btn.clone();
            let rb = redo_btn.clone();
            edit_btn.connect_toggled(move |btn| {
                if !btn.is_active() {
                    return;
                }
                m.set(Mode::Edit);
                fb.set_visible(true);
                sbuf.set_text(&ct.borrow());
                ub.set_sensitive(sbuf.can_undo());
                rb.set_sensitive(sbuf.can_redo());
                st.set_visible_child_name("edit");
                sv.grab_focus();
            });
        }

        // ── Undo/Redo ────────────────────────────────────────────────────
        {
            let sbuf = source_buffer.clone();
            let ub2 = undo_btn.clone();
            let rb2 = redo_btn.clone();
            undo_btn.connect_clicked(move |_| {
                sbuf.undo();
                ub2.set_sensitive(sbuf.can_undo());
                rb2.set_sensitive(sbuf.can_redo());
            });
        }
        {
            let sbuf = source_buffer.clone();
            let ub2 = undo_btn.clone();
            let rb2 = redo_btn.clone();
            redo_btn.connect_clicked(move |_| {
                sbuf.redo();
                ub2.set_sensitive(sbuf.can_undo());
                rb2.set_sensitive(sbuf.can_redo());
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
                if m.get() != Mode::Edit {
                    return;
                }
                let current = buf
                    .text(&buf.start_iter(), &buf.end_iter(), false)
                    .to_string();
                let dirty = current != *ct.borrow();
                mod_flag.set(dirty);
                sb.set_sensitive(dirty);
            });
        }

        // ── Save button ─────────────────────────────────────────────────
        {
            let fp = file_path.to_string();
            let ct = content.clone();
            let sbuf = source_buffer.clone();
            let mod_flag = modified.clone();
            let sb2 = save_btn.clone();
            save_btn.connect_clicked(move |_| {
                let text = sbuf
                    .text(&sbuf.start_iter(), &sbuf.end_iter(), false)
                    .to_string();
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
            let sbuf = source_buffer.clone();
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
                        sbuf.set_text(&text);
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
                if m.get() == Mode::Edit {
                    return glib::ControlFlow::Continue;
                }
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
    fn panel_type(&self) -> &str {
        "markdown"
    }
    fn widget(&self) -> &gtk4::Widget {
        &self.widget
    }
    fn on_focus(&self) {
        self.source_view.grab_focus();
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn wrap_selection_buf(buf: &gtk4::TextBuffer, marker: &str) {
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

fn prepend_line_buf(buf: &gtk4::TextBuffer, prefix: &str) {
    let mut iter = buf.iter_at_mark(&buf.get_insert());
    iter.set_line_offset(0);
    buf.insert(&mut iter, prefix);
}

fn insert_at_cursor_buf(buf: &gtk4::TextBuffer, text: &str) {
    buf.insert(&mut buf.iter_at_mark(&buf.get_insert()), text);
}

fn get_mtime(path: &str) -> u64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .map(|t| {
            t.duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
        })
        .unwrap_or(0)
}

// ── Markdown rendering (render mode) ─────────────────────────────────────────

fn render_markdown_to_view(tv: &gtk4::TextView, content: &str) {
    let buf = tv.buffer();
    buf.set_text("");
    let tt = buf.tag_table();

    let ensure = |name: &str, f: &dyn Fn(&gtk4::TextTag)| {
        if tt.lookup(name).is_none() {
            let t = gtk4::TextTag::new(Some(name));
            f(&t);
            tt.add(&t);
        }
    };
    ensure("h1", &|t| {
        t.set_size_points(20.0);
        t.set_weight(700);
    });
    ensure("h2", &|t| {
        t.set_size_points(16.0);
        t.set_weight(700);
    });
    ensure("h3", &|t| {
        t.set_size_points(14.0);
        t.set_weight(700);
    });
    ensure("bold", &|t| {
        t.set_weight(700);
    });
    ensure("italic", &|t| {
        t.set_style(gtk4::pango::Style::Italic);
    });
    ensure("strike", &|t| {
        t.set_strikethrough(true);
    });
    ensure("code", &|t| {
        t.set_family(Some("monospace"));
    });
    ensure("code_block", &|t| {
        t.set_family(Some("monospace"));
        t.set_paragraph_background(Some("#2a2a2a"));
        t.set_left_margin(20);
    });
    ensure("link", &|t| {
        t.set_foreground(Some("#5588ff"));
        t.set_underline(gtk4::pango::Underline::Single);
    });
    ensure("bullet", &|t| {
        t.set_left_margin(20);
    });
    ensure("bq", &|t| {
        t.set_left_margin(20);
        t.set_style(gtk4::pango::Style::Italic);
        t.set_foreground(Some("#888888"));
    });
    ensure("sep", &|t| {
        t.set_foreground(Some("#666666"));
        t.set_size_points(6.0);
    });

    let mut it = buf.end_iter();
    let mut in_code = false;
    for line in content.lines() {
        if line.starts_with("```") {
            in_code = !in_code;
            let hint = line.trim_start_matches('`').trim();
            if in_code && !hint.is_empty() {
                buf.insert_with_tags_by_name(&mut it, &format!("─── {} ───\n", hint), &["sep"]);
            } else if !in_code {
                buf.insert_with_tags_by_name(&mut it, "───────\n", &["sep"]);
            }
            continue;
        }
        if in_code {
            buf.insert_with_tags_by_name(&mut it, &format!("{}\n", line), &["code_block"]);
            continue;
        }
        if line.starts_with("### ") {
            buf.insert_with_tags_by_name(&mut it, &format!("{}\n", &line[4..]), &["h3"]);
        } else if line.starts_with("## ") {
            buf.insert_with_tags_by_name(&mut it, &format!("{}\n", &line[3..]), &["h2"]);
        } else if line.starts_with("# ") {
            buf.insert_with_tags_by_name(&mut it, &format!("{}\n", &line[2..]), &["h1"]);
        } else if line.starts_with("---") || line.starts_with("***") {
            buf.insert_with_tags_by_name(&mut it, "────────────────────\n", &["sep"]);
        } else if line.starts_with("- ") || line.starts_with("* ") {
            buf.insert_with_tags_by_name(&mut it, &format!("  • {}\n", &line[2..]), &["bullet"]);
        } else if line.starts_with("> ") {
            buf.insert_with_tags_by_name(&mut it, &format!("│ {}\n", &line[2..]), &["bq"]);
        } else {
            render_inline(&buf, &mut it, line);
            buf.insert(&mut it, "\n");
        }
    }
}

fn render_inline(buf: &gtk4::TextBuffer, it: &mut gtk4::TextIter, text: &str) {
    let c: Vec<char> = text.chars().collect();
    let n = c.len();
    let mut i = 0;
    let mut p = String::new();
    while i < n {
        if i + 1 < n && c[i] == '*' && c[i + 1] == '*' {
            if !p.is_empty() {
                buf.insert(it, &p);
                p.clear();
            }
            i += 2;
            let s = i;
            while i + 1 < n && !(c[i] == '*' && c[i + 1] == '*') {
                i += 1;
            }
            buf.insert_with_tags_by_name(it, &c[s..i].iter().collect::<String>(), &["bold"]);
            if i + 1 < n {
                i += 2;
            }
        } else if c[i] == '*' {
            if !p.is_empty() {
                buf.insert(it, &p);
                p.clear();
            }
            i += 1;
            let s = i;
            while i < n && c[i] != '*' {
                i += 1;
            }
            buf.insert_with_tags_by_name(it, &c[s..i].iter().collect::<String>(), &["italic"]);
            if i < n {
                i += 1;
            }
        } else if c[i] == '`' {
            if !p.is_empty() {
                buf.insert(it, &p);
                p.clear();
            }
            i += 1;
            let s = i;
            while i < n && c[i] != '`' {
                i += 1;
            }
            buf.insert_with_tags_by_name(it, &c[s..i].iter().collect::<String>(), &["code"]);
            if i < n {
                i += 1;
            }
        } else if c[i] == '[' {
            if !p.is_empty() {
                buf.insert(it, &p);
                p.clear();
            }
            i += 1;
            let s = i;
            while i < n && c[i] != ']' {
                i += 1;
            }
            let lt: String = c[s..i].iter().collect();
            if i + 1 < n && c[i] == ']' && c[i + 1] == '(' {
                i += 2;
                while i < n && c[i] != ')' {
                    i += 1;
                }
                if i < n {
                    i += 1;
                }
            } else if i < n {
                i += 1;
            }
            buf.insert_with_tags_by_name(it, &lt, &["link"]);
        } else {
            p.push(c[i]);
            i += 1;
        }
    }
    if !p.is_empty() {
        buf.insert(it, &p);
    }
}
