use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Borders, Paragraph, Widget, Wrap};
use ratatui::text::{Line, Span};

/// Simple markdown renderer for notes panel.
pub struct MarkdownPanel {
    content: String,
    scroll: u16,
}

impl MarkdownPanel {
    pub fn new(content: String) -> Self {
        Self { content, scroll: 0 }
    }

    pub fn set_content(&mut self, content: String) {
        self.content = content;
        self.scroll = 0;
    }

    pub fn scroll_down(&mut self) {
        self.scroll = self.scroll.saturating_add(1);
    }

    pub fn scroll_up(&mut self) {
        self.scroll = self.scroll.saturating_sub(1);
    }

    pub fn render(&self, area: Rect, buf: &mut Buffer, focused: bool) {
        let border_color = if focused { Color::Cyan } else { Color::DarkGray };
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border_color))
            .title("Notes");

        let inner = block.inner(area);
        block.render(area, buf);

        let lines = render_markdown_lines(&self.content);
        let paragraph = Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .scroll((self.scroll, 0));
        paragraph.render(inner, buf);
    }
}

/// Simple markdown line renderer (headers, bold, code).
fn render_markdown_lines(content: &str) -> Vec<Line<'static>> {
    content
        .lines()
        .map(|line| {
            if line.starts_with("### ") {
                Line::from(Span::styled(
                    line[4..].to_string(),
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ))
            } else if line.starts_with("## ") {
                Line::from(Span::styled(
                    line[3..].to_string(),
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                ))
            } else if line.starts_with("# ") {
                Line::from(Span::styled(
                    line[2..].to_string(),
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ))
            } else if line.starts_with("```") {
                Line::from(Span::styled(
                    line.to_string(),
                    Style::default().fg(Color::DarkGray),
                ))
            } else if line.starts_with("- ") || line.starts_with("* ") {
                Line::from(vec![
                    Span::styled("  • ", Style::default().fg(Color::Yellow)),
                    Span::raw(line[2..].to_string()),
                ])
            } else {
                Line::from(line.to_string())
            }
        })
        .collect()
}
