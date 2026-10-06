use anyhow::Result;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, List, ListItem, ListState, Paragraph};
use ratatui::{DefaultTerminal, Frame};

use crate::diff::RowKind;
use crate::project::CommentMode;
use crate::render::{empty_message, file_title};
use crate::review::{Detection, FileReview, Summary};

const HELP: &str = " c comments (hidden → only → shown)   n/p file   j/k scroll   d/u page   g/G top/bottom   q quit";

pub struct App {
    pub files: Vec<FileReview>,
    pub mode: CommentMode,
    pub selected: usize,
    pub scroll: usize,
    pub quit: bool,
    /// Height of the diff pane at the last draw, for paging and clamping.
    diff_height: usize,
}

impl App {
    pub fn new(files: Vec<FileReview>, mode: CommentMode) -> App {
        App {
            files,
            mode,
            selected: 0,
            scroll: 0,
            quit: false,
            diff_height: 20,
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        let page = (self.diff_height / 2).max(1);
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => self.quit = true,
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => self.quit = true,
            KeyCode::Char('c') => {
                self.mode = self.mode.next();
                self.scroll = 0;
            }
            KeyCode::Char('n') | KeyCode::Char('J') | KeyCode::Tab | KeyCode::Right => {
                self.select(self.selected.saturating_add(1))
            }
            KeyCode::Char('p') | KeyCode::Char('K') | KeyCode::BackTab | KeyCode::Left => {
                self.select(self.selected.saturating_sub(1))
            }
            KeyCode::Char('j') | KeyCode::Down => self.scroll_to(self.scroll.saturating_add(1)),
            KeyCode::Char('k') | KeyCode::Up => self.scroll_to(self.scroll.saturating_sub(1)),
            KeyCode::Char('d') | KeyCode::Char(' ') | KeyCode::PageDown => {
                self.scroll_to(self.scroll.saturating_add(page))
            }
            KeyCode::Char('u') | KeyCode::PageUp => {
                self.scroll_to(self.scroll.saturating_sub(page))
            }
            KeyCode::Char('g') | KeyCode::Home => self.scroll = 0,
            KeyCode::Char('G') | KeyCode::End => self.scroll_to(usize::MAX),
            _ => {}
        }
    }

    fn select(&mut self, index: usize) {
        let index = index.min(self.files.len().saturating_sub(1));
        if index != self.selected {
            self.selected = index;
            self.scroll = 0;
        }
    }

    fn scroll_to(&mut self, scroll: usize) {
        let max = self.diff_lines().len().saturating_sub(self.diff_height);
        self.scroll = scroll.min(max);
    }

    fn diff_lines(&self) -> Vec<Line<'static>> {
        let Some(file) = self.files.get(self.selected) else {
            return vec![Line::from("No changes.")];
        };
        let view = file.view(self.mode);
        if file.detection == Detection::Binary {
            return vec![Line::from("Binary file not shown.").dim()];
        }
        if view.hunks.is_empty() {
            return vec![
                Line::from(format!(
                    "No visible changes: {}.",
                    empty_message(file, self.mode)
                ))
                .dim(),
            ];
        }

        let mut lines = Vec::new();
        for (i, hunk) in view.hunks.iter().enumerate() {
            if i > 0 {
                lines.push(Line::from(""));
            }
            lines.push(Line::from(hunk.header()).fg(Color::Cyan));
            for row in &hunk.rows {
                let number =
                    |n: Option<usize>| n.map_or("     ".to_string(), |n| format!("{n:>5}"));
                let (sign, style) = match row.kind {
                    RowKind::Context => (' ', Style::new()),
                    RowKind::Removed => ('-', Style::new().fg(Color::Red)),
                    RowKind::Added => ('+', Style::new().fg(Color::Green)),
                };
                lines.push(Line::from(vec![
                    Span::styled(
                        format!("{} {} ", number(row.old_line), number(row.new_line)),
                        Style::new().add_modifier(Modifier::DIM),
                    ),
                    Span::styled(format!("{sign} {}", row.text.replace('\t', "    ")), style),
                ]));
            }
        }
        lines
    }
}

pub fn draw(frame: &mut Frame, app: &mut App) {
    let [main, status, help] = Layout::vertical([
        Constraint::Min(3),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(frame.area());
    let [files_area, diff_area] =
        Layout::horizontal([Constraint::Percentage(30), Constraint::Min(20)]).areas(main);

    let items: Vec<ListItem> = app
        .files
        .iter()
        .map(|file| {
            let visible = file.view(app.mode).hunks.len();
            let item = ListItem::new(format!("{} ({visible})", file_title(file)));
            if visible == 0 { item.dim() } else { item }
        })
        .collect();
    let mut list_state = ListState::default().with_selected(Some(app.selected));
    frame.render_stateful_widget(
        List::new(items)
            .block(Block::bordered().title(" Files "))
            .highlight_style(Style::new().add_modifier(Modifier::REVERSED)),
        files_area,
        &mut list_state,
    );

    app.diff_height = diff_area.height.saturating_sub(2) as usize;
    let lines = app.diff_lines();
    app.scroll = app.scroll.min(lines.len().saturating_sub(app.diff_height));
    let title = app
        .files
        .get(app.selected)
        .map(|file| format!(" {} [{}] ", file.path, file.detection.label()))
        .unwrap_or_default();
    frame.render_widget(
        Paragraph::new(lines)
            .block(Block::bordered().title(title))
            .scroll((app.scroll.min(u16::MAX as usize) as u16, 0)),
        diff_area,
    );

    let summary = Summary::new(&app.files, app.mode).status_line(app.mode);
    frame.render_widget(
        Paragraph::new(format!(" {summary}")).style(Style::new().add_modifier(Modifier::REVERSED)),
        status,
    );
    frame.render_widget(Paragraph::new(HELP).dim(), help);
}

pub fn run(files: Vec<FileReview>, mode: CommentMode) -> Result<()> {
    let mut app = App::new(files, mode);
    ratatui::run(|terminal: &mut DefaultTerminal| -> Result<()> {
        while !app.quit {
            terminal.draw(|frame| draw(frame, &mut app))?;
            if let Event::Key(key) = event::read()?
                && key.kind == KeyEventKind::Press
            {
                app.handle_key(key);
            }
        }
        Ok(())
    })
}
