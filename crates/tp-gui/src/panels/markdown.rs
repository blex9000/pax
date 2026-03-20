use gtk4::prelude::*;

use super::PanelBackend;

/// Markdown viewer panel using GTK4 TextView.
#[derive(Debug)]
pub struct MarkdownPanel {
    scrolled: gtk4::ScrolledWindow,
    text_view: gtk4::TextView,
    widget: gtk4::Widget,
    file_path: String,
}

impl MarkdownPanel {
    pub fn new(file_path: &str) -> Self {
        let text_view = gtk4::TextView::new();
        text_view.set_editable(false);
        text_view.set_cursor_visible(false);
        text_view.set_wrap_mode(gtk4::WrapMode::Word);
        text_view.set_left_margin(12);
        text_view.set_right_margin(12);
        text_view.set_top_margin(8);
        text_view.set_bottom_margin(8);

        // Add CSS class for styling
        text_view.add_css_class("markdown-panel");

        let scrolled = gtk4::ScrolledWindow::new();
        scrolled.set_child(Some(&text_view));
        scrolled.set_vexpand(true);
        scrolled.set_hexpand(true);

        let widget = scrolled.clone().upcast::<gtk4::Widget>();

        let mut panel = Self {
            scrolled,
            text_view,
            widget,
            file_path: file_path.to_string(),
        };

        panel.load_file();
        panel
    }

    fn load_file(&mut self) {
        let content = match std::fs::read_to_string(&self.file_path) {
            Ok(c) => c,
            Err(e) => format!("Error loading {}: {}", self.file_path, e),
        };
        self.render_markdown(&content);
    }

    fn render_markdown(&self, content: &str) {
        let buffer = self.text_view.buffer();
        buffer.set_text("");

        // Create text tags for styling
        let tag_table = buffer.tag_table();

        let make_tag = |name: &str, size: i32, bold: bool| {
            let tag = gtk4::TextTag::new(Some(name));
            tag.set_size_points(size as f64);
            if bold {
                tag.set_weight(700);
            }
            tag_table.add(&tag);
            tag
        };

        let _h1_tag = make_tag("h1", 20, true);
        let _h2_tag = make_tag("h2", 16, true);
        let _h3_tag = make_tag("h3", 14, true);
        let _bold_tag = make_tag("bold", 11, true);

        let code_tag = gtk4::TextTag::new(Some("code"));
        code_tag.set_family(Some("monospace"));
        tag_table.add(&code_tag);

        let mut iter = buffer.end_iter();

        for line in content.lines() {
            if line.starts_with("### ") {
                buffer.insert_with_tags_by_name(&mut iter, &line[4..], &["h3"]);
                buffer.insert(&mut iter, "\n");
            } else if line.starts_with("## ") {
                buffer.insert_with_tags_by_name(&mut iter, &line[3..], &["h2"]);
                buffer.insert(&mut iter, "\n");
            } else if line.starts_with("# ") {
                buffer.insert_with_tags_by_name(&mut iter, &line[2..], &["h1"]);
                buffer.insert(&mut iter, "\n");
            } else if line.starts_with("```") {
                buffer.insert_with_tags_by_name(&mut iter, line, &["code"]);
                buffer.insert(&mut iter, "\n");
            } else if line.starts_with("- ") || line.starts_with("* ") {
                buffer.insert(&mut iter, &format!("  • {}\n", &line[2..]));
            } else {
                buffer.insert(&mut iter, &format!("{}\n", line));
            }
        }
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
