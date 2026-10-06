use anyhow::Result;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, List, ListItem, ListState, Paragraph};
use ratatui::{DefaultTerminal, Frame};

use crate::diff::RowKind;
use crate::project::LayerMode;
use crate::render::{empty_message, file_title};
use crate::review::{Detection, FileReview, Layers, Summary};
use crate::store::ReviewStore;

const HELP: &str = " ↑/↓ move   ←/→ or Tab switch pane   n/p next/prev file   space/PgDn page   g/G top/bottom   r reviewed   c comments   t tests   q quit";

/// The pane the arrow keys act on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Files,
    Diff,
}

pub struct App {
    pub files: Vec<FileReview>,
    pub layers: Layers,
    /// Indices into `files` of the files the test layer currently lists.
    visible: Vec<usize>,
    /// Position in the visible list.
    pub selected: usize,
    pub scroll: usize,
    pub focus: Focus,
    pub store: ReviewStore,
    /// A one-off message for the status bar, cleared by the next key.
    pub message: Option<String>,
    pub quit: bool,
    /// Height of the diff pane at the last draw, for paging and clamping.
    diff_height: usize,
}

impl App {
    pub fn new(files: Vec<FileReview>, layers: Layers) -> App {
        App::with_store(files, layers, ReviewStore::in_memory())
    }

    pub fn with_store(files: Vec<FileReview>, layers: Layers, store: ReviewStore) -> App {
        let mut app = App {
            files,
            layers,
            visible: Vec::new(),
            selected: 0,
            scroll: 0,
            focus: Focus::Files,
            store,
            message: None,
            quit: false,
            diff_height: 20,
        };
        app.refilter();
        app
    }

    /// The file under the cursor, if any file is listed.
    pub fn current(&self) -> Option<&FileReview> {
        self.visible
            .get(self.selected)
            .map(|&index| &self.files[index])
    }

    /// Rebuilds the visible list after the test layer changed, keeping the cursor on
    /// the same file when it is still listed.
    fn refilter(&mut self) {
        let current = self.visible.get(self.selected).copied();
        self.visible = (0..self.files.len())
            .filter(|&index| self.files[index].is_visible(self.layers.tests))
            .collect();
        match current.and_then(|index| self.visible.iter().position(|&i| i == index)) {
            Some(position) => self.selected = position,
            None => {
                self.selected = self.selected.min(self.visible.len().saturating_sub(1));
                self.scroll = 0;
            }
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        let page = (self.diff_height / 2).max(1);
        self.message = None;
        match key.code {
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => self.quit = true,
            KeyCode::Esc if self.focus == Focus::Diff => self.focus = Focus::Files,
            KeyCode::Esc => self.quit = true,
            KeyCode::Char('c') => {
                self.layers.comments = self.layers.comments.next();
                self.scroll = 0;
            }
            KeyCode::Char('r') => self.toggle_reviewed(),
            KeyCode::Char('t') => {
                self.layers.tests = self.layers.tests.next();
                self.refilter();
            }
            KeyCode::Right | KeyCode::Char('l') | KeyCode::Enter => self.focus = Focus::Diff,
            KeyCode::Left | KeyCode::Char('h') => self.focus = Focus::Files,
            KeyCode::Tab | KeyCode::BackTab => {
                self.focus = match self.focus {
                    Focus::Files => Focus::Diff,
                    Focus::Diff => Focus::Files,
                }
            }
            KeyCode::Down | KeyCode::Char('j') => match self.focus {
                Focus::Files => self.select(self.selected.saturating_add(1)),
                Focus::Diff => self.scroll_to(self.scroll.saturating_add(1)),
            },
            KeyCode::Up | KeyCode::Char('k') => match self.focus {
                Focus::Files => self.select(self.selected.saturating_sub(1)),
                Focus::Diff => self.scroll_to(self.scroll.saturating_sub(1)),
            },
            KeyCode::Char('n') | KeyCode::Char('J') => self.select(self.selected.saturating_add(1)),
            KeyCode::Char('p') | KeyCode::Char('K') => self.select(self.selected.saturating_sub(1)),
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

    /// Marks the current file reviewed and moves on to the next unreviewed one, or
    /// clears the mark if it was already set.
    fn toggle_reviewed(&mut self) {
        let Some(&index) = self.visible.get(self.selected) else {
            return;
        };
        let reviewed = !self.store.is_reviewed(&self.files[index]);
        if let Err(error) = self.store.set_reviewed(&self.files[index], reviewed) {
            self.message = Some(format!("could not save review marks: {error:#}"));
        }
        if reviewed
            && let Some(next) = (self.selected + 1..self.visible.len())
                .find(|&position| !self.store.is_reviewed(&self.files[self.visible[position]]))
        {
            self.select(next);
        }
    }

    /// Reviewed and total counts over the listed files.
    pub fn review_progress(&self) -> (usize, usize) {
        let reviewed = self
            .visible
            .iter()
            .filter(|&&index| self.store.is_reviewed(&self.files[index]))
            .count();
        (reviewed, self.visible.len())
    }

    fn select(&mut self, index: usize) {
        let index = index.min(self.visible.len().saturating_sub(1));
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
        let Some(file) = self.current() else {
            let message = match (self.files.is_empty(), self.layers.tests) {
                (true, _) => "No changes.",
                (false, LayerMode::Hidden) => {
                    "Every changed file is a test file. Press t to cycle the test layer."
                }
                (false, _) => "No test files changed. Press t to cycle the test layer.",
            };
            return vec![Line::from(message).dim()];
        };
        let mode = self.layers.comments;
        let view = file.view(mode);
        if file.detection == Detection::Binary {
            return vec![Line::from("Binary file not shown.").dim()];
        }
        if view.hunks.is_empty() {
            return vec![
                Line::from(format!(
                    "No visible changes: {}.",
                    empty_message(file, mode)
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
        .visible
        .iter()
        .map(|&index| {
            let file = &app.files[index];
            let visible = file.view(app.layers.comments).hunks.len();
            let reviewed = app.store.is_reviewed(file);
            let mark = if reviewed { "✓ " } else { "  " };
            let item = ListItem::new(format!("{mark}{} ({visible})", file_title(file)));
            if visible == 0 || reviewed {
                item.dim()
            } else {
                item
            }
        })
        .collect();
    let mut list_state =
        ListState::default().with_selected((!app.visible.is_empty()).then_some(app.selected));
    frame.render_stateful_widget(
        List::new(items)
            .block(pane(" Files ".to_string(), app.focus == Focus::Files))
            .highlight_style(Style::new().add_modifier(Modifier::REVERSED)),
        files_area,
        &mut list_state,
    );

    app.diff_height = diff_area.height.saturating_sub(2) as usize;
    let lines = app.diff_lines();
    app.scroll = app.scroll.min(lines.len().saturating_sub(app.diff_height));
    let title = app
        .current()
        .map(|file| format!(" {} [{}] ", file.path, file.tag()))
        .unwrap_or_default();
    frame.render_widget(
        Paragraph::new(lines)
            .block(pane(title, app.focus == Focus::Diff))
            .scroll((app.scroll.min(u16::MAX as usize) as u16, 0)),
        diff_area,
    );

    let (reviewed, listed) = app.review_progress();
    let summary = match &app.message {
        Some(message) => message.clone(),
        None => format!(
            "{} · {reviewed}/{listed} reviewed",
            Summary::new(&app.files, app.layers).status_line(app.layers)
        ),
    };
    frame.render_widget(
        Paragraph::new(format!(" {summary}")).style(Style::new().add_modifier(Modifier::REVERSED)),
        status,
    );
    frame.render_widget(Paragraph::new(HELP).dim(), help);
}

/// A bordered pane; the focused one is drawn in colour so it is clear where the arrows go.
fn pane(title: String, focused: bool) -> Block<'static> {
    let block = Block::bordered().title(title);
    if focused {
        block
            .border_style(Style::new().fg(Color::Cyan))
            .title_style(Style::new().bold())
    } else {
        block.border_style(Style::new().add_modifier(Modifier::DIM))
    }
}

pub fn run(files: Vec<FileReview>, layers: Layers, store: ReviewStore) -> Result<()> {
    let mut app = App::with_store(files, layers, store);
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
