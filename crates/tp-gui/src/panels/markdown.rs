use gtk4::prelude::*;
use gtk4::glib;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use super::PanelBackend;

/// Markdown viewer/editor panel with render/raw toggle and file watching.
#[derive(Debug)]
pub struct MarkdownPanel {
    _container: gtk4::Box,
    widget: gtk4::Widget,
    text_view: gtk4::TextView,
    file_path: String,
    _is_raw: Rc<Cell<bool>>,
    _is_editing: Rc<Cell<bool>>,
    content: Rc<RefCell<String>>,
}

impl MarkdownPanel {
    pub fn new(file_path: &str) -> Self {
        let container = gtk4::Box::new(gtk4::Orientation::Vertical, 0);

        // Toolbar
        let toolbar = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
        toolbar.add_css_class("markdown-toolbar");
        toolbar.set_margin_start(4);
        toolbar.set_margin_end(4);
        toolbar.set_margin_top(2);
        toolbar.set_margin_bottom(2);

        let render_btn = gtk4::ToggleButton::with_label("Render");
        render_btn.set_active(true);
        render_btn.add_css_class("flat");
        render_btn.set_tooltip_text(Some("Toggle rendered/raw view"));

        let edit_btn = gtk4::ToggleButton::with_label("Edit");
        edit_btn.add_css_class("flat");
        edit_btn.set_tooltip_text(Some("Toggle edit mode"));

        let save_btn = gtk4::Button::with_label("Save");
        save_btn.add_css_class("flat");
        save_btn.set_tooltip_text(Some("Save file"));
        save_btn.set_sensitive(false);

        let file_label = gtk4::Label::new(Some(file_path));
        file_label.add_css_class("dim-label");
        file_label.add_css_class("caption");
        file_label.set_hexpand(true);
        file_label.set_halign(gtk4::Align::End);
        file_label.set_ellipsize(gtk4::pango::EllipsizeMode::Start);

        toolbar.append(&render_btn);
        toolbar.append(&edit_btn);
        toolbar.append(&save_btn);
        toolbar.append(&file_label);
        container.append(&toolbar);

        // Text view
        let text_view = gtk4::TextView::new();
        text_view.set_editable(false);
        text_view.set_cursor_visible(false);
        text_view.set_wrap_mode(gtk4::WrapMode::Word);
        text_view.set_left_margin(12);
        text_view.set_right_margin(12);
        text_view.set_top_margin(8);
        text_view.set_bottom_margin(8);
        text_view.add_css_class("markdown-panel");

        let scrolled = gtk4::ScrolledWindow::new();
        scrolled.set_child(Some(&text_view));
        scrolled.set_vexpand(true);
        scrolled.set_hexpand(true);
        container.append(&scrolled);

        let widget = container.clone().upcast::<gtk4::Widget>();
        let is_raw = Rc::new(Cell::new(false));
        let is_editing = Rc::new(Cell::new(false));
        let content = Rc::new(RefCell::new(String::new()));

        let mut panel = Self {
            _container: container,
            widget,
            text_view,
            file_path: file_path.to_string(),
            _is_raw: is_raw.clone(),
            _is_editing: is_editing.clone(),
            content: content.clone(),
        };

        panel.load_file();

        // Render toggle
        {
            let tv = panel.text_view.clone();
            let raw = is_raw.clone();
            let ct = content.clone();
            render_btn.connect_toggled(move |btn| {
                raw.set(!btn.is_active());
                let text = ct.borrow().clone();
                if btn.is_active() {
                    render_markdown_to_view(&tv, &text);
                } else {
                    tv.buffer().set_text(&text);
                }
            });
        }

        // Edit toggle
        {
            let tv = panel.text_view.clone();
            let editing = is_editing.clone();
            let raw = is_raw.clone();
            let ct = content.clone();
            let sb = save_btn.clone();
            let rb = render_btn.clone();
            edit_btn.connect_toggled(move |btn| {
                let active = btn.is_active();
                editing.set(active);
                tv.set_editable(active);
                tv.set_cursor_visible(active);
                sb.set_sensitive(active);
                if active {
                    // Switch to raw mode for editing
                    raw.set(true);
                    rb.set_active(false);
                    tv.buffer().set_text(&ct.borrow());
                } else {
                    // Save buffer back to content, switch to render
                    let buf = tv.buffer();
                    let text = buf.text(&buf.start_iter(), &buf.end_iter(), false).to_string();
                    *ct.borrow_mut() = text.clone();
                    raw.set(false);
                    rb.set_active(true);
                    render_markdown_to_view(&tv, &text);
                }
            });
        }

        // Save button
        {
            let fp = panel.file_path.clone();
            let ct = content.clone();
            let tv = panel.text_view.clone();
            save_btn.connect_clicked(move |_| {
                let buf = tv.buffer();
                let text = buf.text(&buf.start_iter(), &buf.end_iter(), false).to_string();
                *ct.borrow_mut() = text.clone();
                if let Err(e) = std::fs::write(&fp, &text) {
                    tracing::error!("Failed to save {}: {}", fp, e);
                }
            });
        }

        // File watch — poll every 2 seconds
        {
            let fp = panel.file_path.clone();
            let ct = content.clone();
            let tv = panel.text_view.clone();
            let raw = is_raw.clone();
            let editing = is_editing.clone();
            let last_modified = Rc::new(Cell::new(get_mtime(&fp)));

            glib::timeout_add_local(std::time::Duration::from_secs(2), move || {
                if editing.get() {
                    return glib::ControlFlow::Continue;
                }
                let mtime = get_mtime(&fp);
                if mtime != last_modified.get() {
                    last_modified.set(mtime);
                    if let Ok(text) = std::fs::read_to_string(&fp) {
                        *ct.borrow_mut() = text.clone();
                        if raw.get() {
                            tv.buffer().set_text(&text);
                        } else {
                            render_markdown_to_view(&tv, &text);
                        }
                    }
                }
                glib::ControlFlow::Continue
            });
        }

        panel
    }

    fn load_file(&mut self) {
        let text = match std::fs::read_to_string(&self.file_path) {
            Ok(c) => c,
            Err(e) => format!("Error loading {}: {}", self.file_path, e),
        };
        *self.content.borrow_mut() = text.clone();
        render_markdown_to_view(&self.text_view, &text);
    }

    pub fn reload(&mut self) {
        self.load_file();
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
        self.text_view.grab_focus();
    }
}

fn get_mtime(path: &str) -> u64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .map(|t| t.duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs())
        .unwrap_or(0)
}

fn render_markdown_to_view(text_view: &gtk4::TextView, content: &str) {
    let buffer = text_view.buffer();
    buffer.set_text("");

    let tag_table = buffer.tag_table();

    // Create tags if they don't exist yet
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
                // Start of code block — show language hint
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
            buffer.insert_with_tags_by_name(&mut iter, &format!("│ {}\n", &line[2..]), &["italic"]);
        } else {
            // Inline formatting: **bold**, *italic*, `code`, [link](url)
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
        if i + 1 < len && chars[i] == '*' && chars[i + 1] == '*' {
            // Bold
            if !plain.is_empty() {
                buffer.insert(iter, &plain);
                plain.clear();
            }
            i += 2;
            let start = i;
            while i + 1 < len && !(chars[i] == '*' && chars[i + 1] == '*') { i += 1; }
            let bold_text: String = chars[start..i].iter().collect();
            buffer.insert_with_tags_by_name(iter, &bold_text, &["bold"]);
            if i + 1 < len { i += 2; } // skip closing **
        } else if chars[i] == '*' && (i + 1 >= len || chars[i + 1] != '*') {
            // Italic
            if !plain.is_empty() {
                buffer.insert(iter, &plain);
                plain.clear();
            }
            i += 1;
            let start = i;
            while i < len && chars[i] != '*' { i += 1; }
            let italic_text: String = chars[start..i].iter().collect();
            buffer.insert_with_tags_by_name(iter, &italic_text, &["italic"]);
            if i < len { i += 1; }
        } else if chars[i] == '`' {
            // Inline code
            if !plain.is_empty() {
                buffer.insert(iter, &plain);
                plain.clear();
            }
            i += 1;
            let start = i;
            while i < len && chars[i] != '`' { i += 1; }
            let code_text: String = chars[start..i].iter().collect();
            buffer.insert_with_tags_by_name(iter, &code_text, &["code"]);
            if i < len { i += 1; }
        } else if chars[i] == '[' {
            // Link: [text](url)
            if !plain.is_empty() {
                buffer.insert(iter, &plain);
                plain.clear();
            }
            i += 1;
            let start = i;
            while i < len && chars[i] != ']' { i += 1; }
            let link_text: String = chars[start..i].iter().collect();
            if i + 1 < len && chars[i] == ']' && chars[i + 1] == '(' {
                i += 2;
                while i < len && chars[i] != ')' { i += 1; }
                if i < len { i += 1; }
            } else if i < len {
                i += 1;
            }
            buffer.insert_with_tags_by_name(iter, &link_text, &["link"]);
        } else {
            plain.push(chars[i]);
            i += 1;
        }
    }

    if !plain.is_empty() {
        buffer.insert(iter, &plain);
    }
}
