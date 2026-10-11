//! One terminal owner, shared by all Relay interaction flows.
use crate::{Error, Result};
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    execute,
};
use ratatui::{
    DefaultTerminal,
    layout::{Constraint, Layout},
    style::{Color, Modifier, Style},
    widgets::{Block, List, ListState, Paragraph, Wrap},
};
use std::{
    future::Future,
    io::{self, IsTerminal},
    time::Duration,
};

static ACTIVE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
pub(crate) fn is_active() -> bool {
    ACTIVE.load(std::sync::atomic::Ordering::Relaxed)
}

pub enum Choice {
    Pick(usize),
    Back,
    Refresh,
    Rename(usize),
    Delete(usize),
}
pub struct Screen {
    terminal: DefaultTerminal,
    interrupt: tokio::signal::unix::Signal,
    terminate: tokio::signal::unix::Signal,
    hangup: tokio::signal::unix::Signal,
}
impl Screen {
    pub fn new() -> Result<Self> {
        if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
            return Err(Error::new(
                "TERMINAL_REQUIRED",
                "open a terminal for interactive setup; use --help for explicit commands and daemon for foreground service operation",
            ));
        }
        let interrupt = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())
            .map_err(io_error)?;
        let terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .map_err(io_error)?;
        let hangup = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::hangup())
            .map_err(io_error)?;
        let terminal = ratatui::try_init().map_err(io_error)?;
        let screen = Self {
            terminal,
            interrupt,
            terminate,
            hangup,
        };
        execute!(io::stdout(), event::EnableBracketedPaste).map_err(io_error)?;
        ACTIVE.store(true, std::sync::atomic::Ordering::Relaxed);
        Ok(screen)
    }
    pub fn render(
        &mut self,
        title: &str,
        rows: &[String],
        selected: usize,
        detail: &str,
        footer: &str,
    ) -> Result<()> {
        self.terminal
            .draw(|frame| {
                let area = frame.area();
                if area.height < 8 || area.width < 30 {
                    frame.render_widget(
                        Paragraph::new("Enlarge terminal (30 x 8 minimum). Esc exits."),
                        area,
                    );
                    return;
                }
                let parts = Layout::vertical([
                    Constraint::Length(3),
                    Constraint::Min(1),
                    Constraint::Length(3),
                    Constraint::Length(2),
                ])
                .split(area);
                frame.render_widget(
                    Paragraph::new(title)
                        .style(
                            Style::default()
                                .fg(Color::Cyan)
                                .add_modifier(Modifier::BOLD),
                        )
                        .block(Block::bordered().title(" Herdr Relay ")),
                    parts[0],
                );
                if rows.is_empty() {
                    frame
                        .render_widget(Paragraph::new(detail).wrap(Wrap { trim: false }), parts[1]);
                } else {
                    let mut state = ListState::default()
                        .with_selected(Some(selected.min(rows.len().saturating_sub(1))));
                    frame.render_stateful_widget(
                        List::new(rows.to_vec())
                            .highlight_symbol("> ")
                            .highlight_style(
                                Style::default()
                                    .fg(Color::Cyan)
                                    .add_modifier(Modifier::BOLD),
                            ),
                        parts[1],
                        &mut state,
                    );
                    frame
                        .render_widget(Paragraph::new(detail).wrap(Wrap { trim: false }), parts[2]);
                }
                frame.render_widget(
                    Paragraph::new(footer)
                        .style(Style::default().fg(Color::DarkGray))
                        .wrap(Wrap { trim: true }),
                    parts[3],
                );
            })
            .map_err(io_error)?;
        Ok(())
    }
    pub async fn event(&mut self) -> Result<Event> {
        loop {
            if event::poll(Duration::ZERO).map_err(io_error)? {
                let event = event::read().map_err(io_error)?;
                if let Event::Key(key) = event {
                    if key.kind == KeyEventKind::Release {
                        continue;
                    }
                    if key.code == KeyCode::Char('c')
                        && key.modifiers.contains(KeyModifiers::CONTROL)
                    {
                        return Err(cancelled());
                    }
                }
                return Ok(event);
            }
            tokio::select! {
                _ = self.interrupt.recv() => return Err(cancelled()),
                _ = self.terminate.recv() => return Err(cancelled()),
                _ = self.hangup.recv() => return Err(cancelled()),
                _ = tokio::time::sleep(Duration::from_millis(30)) => {},
            }
        }
    }
    pub async fn message(&mut self, title: &str, message: &str) -> Result<()> {
        loop {
            self.render(title, &[], 0, message, "Enter / Esc  Back    Ctrl-C  Exit")?;
            if let Event::Key(KeyEvent {
                code: KeyCode::Enter | KeyCode::Esc | KeyCode::Char('q'),
                ..
            }) = self.event().await?
            {
                return Ok(());
            }
        }
    }
    pub async fn help(&mut self, extra: &str) -> Result<()> {
        self.message("Help", &format!("Up/Down or j/k: move\nEnter: choose / apply\nSpace: toggle a session in serve\n/: search lists    r: refresh\nEsc: back    Ctrl-C: exit\n?: help (F1 also works)\n\n{extra}\n\nRelay yields the terminal to Herdr while connected.\nDetach from Herdr with Ctrl-B, release, then Q.\nExiting setup does not stop the background service.")).await
    }
    pub async fn input(&mut self, title: &str, hint: &str, hidden: bool) -> Result<Option<String>> {
        let mut value = String::new();
        loop {
            let shown = if hidden {
                format!("{} characters pasted", value.len())
            } else {
                crate::names::display(&value)
            };
            self.render(
                title,
                &[],
                0,
                &format!("{hint}\n\n{shown}"),
                "Enter  Continue    Esc  Back    F1  Help",
            )?;
            match self.event().await? {
                Event::Paste(text) => {
                    let text = text.trim();
                    if value.len() + text.len() <= 8192 {
                        value.push_str(text);
                    } else {
                        self.message("Input too long", "Paste a value no longer than 8192 bytes.")
                            .await?;
                    }
                }
                Event::Key(key) => match key.code {
                    KeyCode::Esc => return Ok(None),
                    KeyCode::Enter if !value.trim().is_empty() => {
                        return Ok(Some(value.trim().into()));
                    }
                    KeyCode::Backspace => {
                        value.pop();
                    }
                    KeyCode::F(1) => {
                        self.help("Paste a complete invitation link. Pairing input stays hidden.")
                            .await?
                    }
                    KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        value.clear()
                    }
                    KeyCode::Char(c)
                        if !key.modifiers.contains(KeyModifiers::CONTROL)
                            && value.len() + c.len_utf8() <= 8192 =>
                    {
                        value.push(c)
                    }
                    _ => {}
                },
                _ => {}
            }
        }
    }
    pub async fn choose(
        &mut self,
        title: &str,
        rows: &[String],
        detail: &str,
        aliases: bool,
    ) -> Result<Choice> {
        self.choose_live(title, rows, detail, aliases, None).await
    }
    pub async fn choose_live(
        &mut self,
        title: &str,
        rows: &[String],
        detail: &str,
        aliases: bool,
        mut updates: Option<&mut tokio::sync::mpsc::UnboundedReceiver<(usize, String)>>,
    ) -> Result<Choice> {
        let mut rows = rows.to_vec();
        let mut selected: usize = 0;
        let mut filter = String::new();
        let mut searching = false;
        loop {
            let indices: Vec<_> = rows
                .iter()
                .enumerate()
                .filter(|(_, s)| s.to_lowercase().contains(&filter.to_lowercase()))
                .map(|(i, _)| i)
                .collect();
            selected = selected.min(indices.len().saturating_sub(1));
            let filtered: Vec<_> = indices.iter().map(|i| rows[*i].clone()).collect();
            self.render(
                title,
                &filtered,
                selected,
                &format!("{detail}\nSearch: {filter}"),
                if aliases {
                    "Enter  Select    / Search    r Refresh    a Rename    d Remove    ? Help    Esc Back"
                } else {
                    "Enter  Select    / Search    r Refresh    ? Help    Esc Back"
                },
            )?;
            let event = if let Some(receiver) = updates.as_mut() {
                tokio::select! {
                    event = self.event() => event?,
                    update = receiver.recv() => {
                        if let Some((index, row)) = update { if let Some(slot) = rows.get_mut(index) { *slot = row; } }
                        else { updates = None; }
                        continue;
                    },
                }
            } else {
                self.event().await?
            };
            if let Event::Key(key) = event {
                if searching {
                    match key.code {
                        KeyCode::Esc => {
                            searching = false;
                            filter.clear();
                        }
                        KeyCode::Enter => searching = false,
                        KeyCode::Backspace => {
                            filter.pop();
                        }
                        KeyCode::Char(c) => filter.push(c),
                        _ => {}
                    }
                    selected = 0;
                    continue;
                }
                match key.code {
                    KeyCode::Esc | KeyCode::Char('q') => return Ok(Choice::Back),
                    KeyCode::Up | KeyCode::Char('k') => selected = selected.saturating_sub(1),
                    KeyCode::Down | KeyCode::Char('j') => {
                        selected = (selected + 1).min(indices.len().saturating_sub(1))
                    }
                    KeyCode::Char('/') => searching = true,
                    KeyCode::Char('r') => return Ok(Choice::Refresh),
                    KeyCode::Char('a') if aliases && !indices.is_empty() => {
                        return Ok(Choice::Rename(indices[selected]));
                    }
                    KeyCode::Char('d') | KeyCode::Delete if aliases && !indices.is_empty() => {
                        return Ok(Choice::Delete(indices[selected]));
                    }
                    KeyCode::Enter if !indices.is_empty() => {
                        return Ok(Choice::Pick(indices[selected]));
                    }
                    KeyCode::Char('?') | KeyCode::F(1) => self.help(detail).await?,
                    _ => {}
                }
            }
        }
    }
    pub async fn wait<T>(
        &mut self,
        title: &str,
        future: impl Future<Output = Result<T>>,
    ) -> Result<T> {
        tokio::pin!(future);
        loop {
            self.render(title, &[], 0, "Working…", "Esc  Cancel    Ctrl-C  Exit")?;
            tokio::select! {
                result = &mut future => return result,
                event = self.event() => {
                    if matches!(event?, Event::Key(KeyEvent { code: KeyCode::Esc, .. })) { return Err(Error::new("BACK", "cancelled")); }
                },
            }
        }
    }
}
impl Drop for Screen {
    fn drop(&mut self) {
        ACTIVE.store(false, std::sync::atomic::Ordering::Relaxed);
        let _ = execute!(io::stdout(), event::DisableBracketedPaste);
        ratatui::restore();
    }
}
pub fn cancelled() -> Error {
    Error::new("CANCELLED", "cancelled")
}
fn io_error(e: io::Error) -> Error {
    Error::new("TERMINAL_IO", e.to_string())
}
