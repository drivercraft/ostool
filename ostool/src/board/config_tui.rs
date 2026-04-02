use std::{
    io::{self, Stdout},
    path::PathBuf,
};

use anyhow::{Context, bail};
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};

use crate::board::global_config::{BoardGlobalConfig, LoadedBoardGlobalConfig};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActiveField {
    ServerIp,
    Port,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EditorOutcome {
    Saved,
    Cancelled,
}

#[derive(Debug, Clone)]
struct BoardConfigForm {
    path: PathBuf,
    server_ip: String,
    port: String,
    active: ActiveField,
    error: Option<String>,
}

impl BoardConfigForm {
    fn new(path: PathBuf, config: BoardGlobalConfig) -> Self {
        Self {
            path,
            server_ip: config.server_ip,
            port: config.port.to_string(),
            active: ActiveField::ServerIp,
            error: None,
        }
    }

    fn from_loaded_config(config: LoadedBoardGlobalConfig) -> Self {
        Self::new(config.path, config.board)
    }

    fn handle_key(&mut self, key: KeyEvent) -> anyhow::Result<Option<EditorOutcome>> {
        match key.code {
            KeyCode::Esc => return Ok(Some(EditorOutcome::Cancelled)),
            KeyCode::Tab | KeyCode::Down | KeyCode::Right => {
                self.focus_next();
                self.error = None;
            }
            KeyCode::BackTab | KeyCode::Up | KeyCode::Left => {
                self.focus_prev();
                self.error = None;
            }
            KeyCode::Backspace => {
                self.active_value_mut().pop();
                self.error = None;
            }
            KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.save()?;
                return Ok(Some(EditorOutcome::Saved));
            }
            KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.push_char(ch);
                self.error = None;
            }
            _ => {}
        }
        Ok(None)
    }

    fn save(&mut self) -> anyhow::Result<()> {
        let config = self.validate()?;
        LoadedBoardGlobalConfig {
            path: self.path.clone(),
            board: config,
            created: false,
        }
        .save()?;
        self.error = None;
        Ok(())
    }

    fn validate(&self) -> anyhow::Result<BoardGlobalConfig> {
        let server_ip = self.server_ip.trim().to_string();
        if server_ip.is_empty() {
            bail!("server_ip must not be empty");
        }
        let port: u16 = self
            .port
            .trim()
            .parse()
            .context("port must be a valid integer")?;
        if port == 0 {
            bail!("port must be in 1..=65535");
        }
        Ok(BoardGlobalConfig { server_ip, port })
    }

    fn push_char(&mut self, ch: char) {
        match self.active {
            ActiveField::ServerIp => self.server_ip.push(ch),
            ActiveField::Port if ch.is_ascii_digit() => self.port.push(ch),
            ActiveField::Port => {}
        }
    }

    fn active_value_mut(&mut self) -> &mut String {
        match self.active {
            ActiveField::ServerIp => &mut self.server_ip,
            ActiveField::Port => &mut self.port,
        }
    }

    fn focus_next(&mut self) {
        self.active = match self.active {
            ActiveField::ServerIp => ActiveField::Port,
            ActiveField::Port => ActiveField::ServerIp,
        };
    }

    fn focus_prev(&mut self) {
        self.focus_next();
    }
}

pub fn run_board_config_tui() -> anyhow::Result<()> {
    let loaded = LoadedBoardGlobalConfig::load_or_create()?;
    let saved_path = loaded.path.clone();
    let mut form = BoardConfigForm::from_loaded_config(loaded);

    let mut terminal = setup_terminal()?;
    let run_result = run_loop(&mut terminal, &mut form);
    let cleanup_result = restore_terminal(&mut terminal);

    let outcome = run_result?;
    cleanup_result?;

    if outcome == EditorOutcome::Saved {
        println!("Saved board config: {}", saved_path.display());
    }

    Ok(())
}

fn setup_terminal() -> anyhow::Result<Terminal<CrosstermBackend<Stdout>>> {
    enable_raw_mode().context("failed to enable raw mode")?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen).context("failed to enter alternate screen")?;
    let backend = CrosstermBackend::new(stdout);
    Terminal::new(backend).context("failed to create terminal")
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> anyhow::Result<()> {
    disable_raw_mode().context("failed to disable raw mode")?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)
        .context("failed to leave alternate screen")?;
    terminal.show_cursor().context("failed to show cursor")?;
    Ok(())
}

fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    form: &mut BoardConfigForm,
) -> anyhow::Result<EditorOutcome> {
    loop {
        terminal
            .draw(|frame| draw_form(frame, form))
            .context("failed to draw board config tui")?;

        match event::read().context("failed to read terminal event")? {
            Event::Key(key) if key.kind == KeyEventKind::Press => match form.handle_key(key) {
                Ok(Some(outcome)) => return Ok(outcome),
                Ok(None) => {}
                Err(err) => form.error = Some(err.to_string()),
            },
            _ => {}
        }
    }
}

fn draw_form(frame: &mut ratatui::Frame<'_>, form: &BoardConfigForm) {
    let area = frame.area();
    frame.render_widget(
        Block::default().style(Style::default().bg(Color::Rgb(11, 19, 26))),
        area,
    );

    let outer = centered_rect(area);
    let block = Block::default()
        .title(Line::from(vec![Span::styled(
            " OSTool Board Config ",
            Style::default().fg(Color::Cyan),
        )]))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Gray));
    let inner = block.inner(outer);
    frame.render_widget(block, outer);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(2),
            Constraint::Length(2),
        ])
        .margin(1)
        .split(inner);

    let title =
        Paragraph::new("Configure the default ostool-server connection for board commands.")
            .style(Style::default().fg(Color::White));
    frame.render_widget(title, chunks[0]);

    draw_input(
        frame,
        chunks[1],
        "server_ip",
        &form.server_ip,
        form.active == ActiveField::ServerIp,
    );
    draw_input(
        frame,
        chunks[2],
        "port",
        &form.port,
        form.active == ActiveField::Port,
    );

    let error = form.error.as_deref().unwrap_or(" ");
    let error_widget = Paragraph::new(error)
        .style(Style::default().fg(Color::LightRed))
        .wrap(Wrap { trim: true });
    frame.render_widget(error_widget, chunks[3]);

    let footer = Paragraph::new(Line::from(vec![
        Span::styled("Tab/↑↓", Style::default().fg(Color::Yellow)),
        Span::raw(" switch  "),
        Span::styled(
            "Ctrl+S",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" save  "),
        Span::styled("Esc", Style::default().fg(Color::Magenta)),
        Span::raw(" cancel"),
    ]))
    .style(Style::default().fg(Color::Gray));
    frame.render_widget(footer, chunks[4]);
}

fn draw_input(frame: &mut ratatui::Frame<'_>, area: Rect, label: &str, value: &str, active: bool) {
    let border_style = if active {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let title_style = if active {
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Gray)
    };

    let paragraph = Paragraph::new(value.to_string())
        .block(
            Block::default()
                .title(Span::styled(format!(" {label} "), title_style))
                .borders(Borders::ALL)
                .border_style(border_style),
        )
        .style(Style::default().fg(Color::White));
    frame.render_widget(paragraph, area);
}

fn centered_rect(area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(18),
            Constraint::Length(14),
            Constraint::Percentage(18),
        ])
        .split(area);
    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(15),
            Constraint::Min(56),
            Constraint::Percentage(15),
        ])
        .split(vertical[1]);
    horizontal[1]
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use tempfile::tempdir;

    use super::BoardConfigForm;
    use crate::board::global_config::BoardGlobalConfig;

    #[test]
    fn form_initializes_from_existing_config() {
        let form = BoardConfigForm::new(
            PathBuf::from("/tmp/config.toml"),
            BoardGlobalConfig {
                server_ip: "10.0.0.2".into(),
                port: 9000,
            },
        );

        assert_eq!(form.server_ip, "10.0.0.2");
        assert_eq!(form.port, "9000");
    }

    #[test]
    fn save_persists_valid_values() {
        let temp = tempdir().unwrap();
        let path = temp.path().join(".ostool/config.toml");
        let mut form = BoardConfigForm::new(path.clone(), BoardGlobalConfig::default());
        form.server_ip = "10.0.0.2".into();
        form.port = "9000".into();

        form.save().unwrap();

        let content = std::fs::read_to_string(path).unwrap();
        assert!(content.contains("server_ip = \"10.0.0.2\""));
        assert!(content.contains("port = 9000"));
    }

    #[test]
    fn save_rejects_empty_server_ip() {
        let temp = tempdir().unwrap();
        let path = temp.path().join(".ostool/config.toml");
        let mut form = BoardConfigForm::new(path.clone(), BoardGlobalConfig::default());
        form.server_ip = "   ".into();

        let err = form.save().unwrap_err();
        assert!(err.to_string().contains("server_ip"));
        assert!(!path.exists());
    }

    #[test]
    fn save_rejects_invalid_port() {
        let temp = tempdir().unwrap();
        let path = temp.path().join(".ostool/config.toml");
        let mut form = BoardConfigForm::new(path.clone(), BoardGlobalConfig::default());
        form.port = "70000".into();

        let err = form.save().unwrap_err();
        assert!(err.to_string().contains("port"));
        assert!(!path.exists());
    }
}
