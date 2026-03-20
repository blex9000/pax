use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Borders, Widget};

/// State for a terminal panel: holds the vt100 parser.
pub struct TerminalPanel {
    parser: vt100::Parser,
    pub alert_border_color: Option<Color>,
}

impl TerminalPanel {
    pub fn new(rows: u16, cols: u16) -> Self {
        Self {
            parser: vt100::Parser::new(rows, cols, 0),
            alert_border_color: None,
        }
    }

    /// Feed raw PTY output into the vt100 parser.
    pub fn feed(&mut self, data: &[u8]) {
        self.parser.process(data);
    }

    /// Resize the virtual terminal.
    pub fn resize(&mut self, rows: u16, cols: u16) {
        self.parser.set_size(rows, cols);
    }

    /// Get the screen contents for rendering.
    pub fn screen(&self) -> &vt100::Screen {
        self.parser.screen()
    }

    /// Render the terminal into a ratatui buffer area.
    pub fn render(&self, area: Rect, buf: &mut Buffer, title: &str, focused: bool) {
        let border_color = if let Some(c) = self.alert_border_color {
            c
        } else if focused {
            Color::Cyan
        } else {
            Color::DarkGray
        };

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border_color))
            .title(title);

        let inner = block.inner(area);
        block.render(area, buf);

        // Render vt100 screen cells into ratatui buffer
        let screen = self.parser.screen();
        for row in 0..inner.height {
            for col in 0..inner.width {
                let cell = screen.cell(row, col);
                if let Some(cell) = cell {
                    let buf_x = inner.x + col;
                    let buf_y = inner.y + row;

                    if buf_x < buf.area().width && buf_y < buf.area().height {
                        let fg = vt100_color_to_ratatui(cell.fgcolor());
                        let bg = vt100_color_to_ratatui(cell.bgcolor());

                        let mut style = Style::default().fg(fg).bg(bg);
                        if cell.bold() {
                            style = style.add_modifier(Modifier::BOLD);
                        }
                        if cell.underline() {
                            style = style.add_modifier(Modifier::UNDERLINED);
                        }
                        if cell.inverse() {
                            style = style.add_modifier(Modifier::REVERSED);
                        }

                        let ch = cell.contents();
                        let display_char = if ch.is_empty() { " " } else { &ch };
                        buf.set_string(buf_x, buf_y, display_char, style);
                    }
                }
            }
        }

        // Render cursor if focused
        if focused {
            let cursor = screen.cursor_position();
            let cx = inner.x + cursor.1;
            let cy = inner.y + cursor.0;
            if cx < inner.x + inner.width && cy < inner.y + inner.height {
                if let Some(existing) = buf.cell_mut((cx, cy)) {
                    existing.set_style(
                        Style::default()
                            .fg(Color::Black)
                            .bg(Color::White),
                    );
                }
            }
        }
    }
}

fn vt100_color_to_ratatui(color: vt100::Color) -> Color {
    match color {
        vt100::Color::Default => Color::Reset,
        vt100::Color::Idx(i) => Color::Indexed(i),
        vt100::Color::Rgb(r, g, b) => Color::Rgb(r, g, b),
    }
}
