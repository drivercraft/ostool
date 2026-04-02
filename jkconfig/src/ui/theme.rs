use ratatui::style::{Color, Modifier, Style};

#[derive(Debug, Clone, Copy)]
pub struct Theme {
    pub background: Color,
    pub panel_border: Color,
    pub panel_border_active: Color,
    pub text: Color,
    pub text_muted: Color,
    pub accent: Color,
    pub warning: Color,
    pub error: Color,
    pub success: Color,
    pub selection_bg: Color,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            background: Color::Rgb(11, 18, 24),
            panel_border: Color::Rgb(66, 86, 99),
            panel_border_active: Color::Rgb(86, 169, 199),
            text: Color::Rgb(234, 237, 239),
            text_muted: Color::Rgb(150, 163, 171),
            accent: Color::Rgb(86, 169, 199),
            warning: Color::Rgb(220, 162, 60),
            error: Color::Rgb(214, 92, 92),
            success: Color::Rgb(93, 183, 116),
            selection_bg: Color::Rgb(31, 50, 63),
        }
    }
}

impl Theme {
    pub fn text(self) -> Style {
        Style::default().fg(self.text).bg(self.background)
    }

    pub fn muted(self) -> Style {
        Style::default().fg(self.text_muted).bg(self.background)
    }

    pub fn accent(self) -> Style {
        Style::default()
            .fg(self.accent)
            .bg(self.background)
            .add_modifier(Modifier::BOLD)
    }

    pub fn active_border(self) -> Style {
        Style::default().fg(self.panel_border_active)
    }

    pub fn passive_border(self) -> Style {
        Style::default().fg(self.panel_border)
    }

    pub fn selected_row(self) -> Style {
        Style::default().fg(self.text).bg(self.selection_bg)
    }

    pub fn required(self) -> Style {
        Style::default()
            .fg(self.error)
            .bg(self.background)
            .add_modifier(Modifier::BOLD)
    }

    pub fn required_dim(self) -> Style {
        Style::default().fg(self.error).bg(self.background)
    }
}
