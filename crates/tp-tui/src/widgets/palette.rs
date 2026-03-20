use fuzzy_matcher::skim::SkimMatcherV2;
use fuzzy_matcher::FuzzyMatcher;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Widget};

/// Command palette with fuzzy search.
pub struct CommandPalette {
    pub input: String,
    pub items: Vec<PaletteItem>,
    pub filtered: Vec<usize>,
    pub selected: usize,
    pub visible: bool,
    matcher: SkimMatcherV2,
}

#[derive(Clone)]
pub struct PaletteItem {
    pub label: String,
    pub description: String,
    pub action: PaletteAction,
}

#[derive(Clone, Debug)]
pub enum PaletteAction {
    FocusPanel(String),
    ToggleZoom,
    ToggleBroadcast(String),
    ToggleRecording(String),
    SshConnect(String),
    Custom(String),
}

impl CommandPalette {
    pub fn new(items: Vec<PaletteItem>) -> Self {
        let filtered: Vec<usize> = (0..items.len()).collect();
        Self {
            input: String::new(),
            items,
            filtered,
            selected: 0,
            visible: false,
            matcher: SkimMatcherV2::default(),
        }
    }

    pub fn toggle(&mut self) {
        self.visible = !self.visible;
        if self.visible {
            self.input.clear();
            self.selected = 0;
            self.update_filter();
        }
    }

    pub fn type_char(&mut self, c: char) {
        self.input.push(c);
        self.selected = 0;
        self.update_filter();
    }

    pub fn backspace(&mut self) {
        self.input.pop();
        self.selected = 0;
        self.update_filter();
    }

    pub fn move_up(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
        }
    }

    pub fn move_down(&mut self) {
        if self.selected + 1 < self.filtered.len() {
            self.selected += 1;
        }
    }

    pub fn confirm(&mut self) -> Option<PaletteAction> {
        let action = self
            .filtered
            .get(self.selected)
            .map(|&i| self.items[i].action.clone());
        self.visible = false;
        action
    }

    fn update_filter(&mut self) {
        if self.input.is_empty() {
            self.filtered = (0..self.items.len()).collect();
        } else {
            let mut scored: Vec<(usize, i64)> = self
                .items
                .iter()
                .enumerate()
                .filter_map(|(i, item)| {
                    self.matcher
                        .fuzzy_match(&item.label, &self.input)
                        .map(|score| (i, score))
                })
                .collect();
            scored.sort_by(|a, b| b.1.cmp(&a.1));
            self.filtered = scored.into_iter().map(|(i, _)| i).collect();
        }
    }

    /// Render the palette as a centered popup.
    pub fn render(&self, area: Rect, buf: &mut Buffer) {
        if !self.visible {
            return;
        }

        // Centered popup: 60% width, up to 15 items + 3 for borders/input
        let w = (area.width as f64 * 0.6).min(80.0) as u16;
        let h = (self.filtered.len() as u16 + 3).min(18).min(area.height);
        let x = area.x + (area.width.saturating_sub(w)) / 2;
        let y = area.y + (area.height.saturating_sub(h)) / 3;
        let popup = Rect::new(x, y, w, h);

        // Clear background
        Clear.render(popup, buf);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan))
            .title(" Command Palette ");

        let inner = block.inner(popup);
        block.render(popup, buf);

        if inner.height == 0 {
            return;
        }

        // Input line
        let input_line = Line::from(vec![
            Span::styled("> ", Style::default().fg(Color::Cyan)),
            Span::raw(&self.input),
            Span::styled("█", Style::default().fg(Color::White)),
        ]);
        Paragraph::new(input_line).render(
            Rect::new(inner.x, inner.y, inner.width, 1),
            buf,
        );

        // Items
        if inner.height > 1 {
            let list_area = Rect::new(
                inner.x,
                inner.y + 1,
                inner.width,
                inner.height - 1,
            );

            for (vi, &idx) in self.filtered.iter().enumerate() {
                if vi as u16 >= list_area.height {
                    break;
                }
                let item = &self.items[idx];
                let style = if vi == self.selected {
                    Style::default()
                        .bg(Color::DarkGray)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };
                let line = format!("{:width$}", item.label, width = list_area.width as usize);
                buf.set_string(list_area.x, list_area.y + vi as u16, &line, style);
            }
        }
    }
}
