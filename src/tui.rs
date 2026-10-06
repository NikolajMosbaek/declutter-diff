use std::fs;
use std::io::Write;
use std::ops::Range;
use std::process::{Command, Stdio};

use anyhow::Result;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, List, ListItem, ListState, Paragraph};
use ratatui::{DefaultTerminal, Frame};

use crate::diff::{Row, RowKind};
use crate::highlight::Class;
use crate::project::LayerMode;
use crate::render::{empty_message, file_title};
use crate::review::{Detection, FileReview, Layers, Summary};
use crate::store::{Note, NoteSide, NoteStore, ReviewStore};

const HELP: &str = " ↑/↓ move   ←/→ switch pane   n/p file   space page   r reviewed   m note   E export notes   c comments   t tests   i imports   l logging   w formatting   q quit";

/// The pane the arrow keys act on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Files,
    Diff,
}

/// The line of a file a diff line stands for, so notes can be attached to it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Anchor {
    side: NoteSide,
    line: usize,
    code: String,
}

/// Copies text to the system clipboard; returns whether it worked.
pub type Clipboard = fn(&str) -> bool;

pub struct App {
    pub files: Vec<FileReview>,
    pub layers: Layers,
    /// Indices into `files` of the files the test layer currently lists.
    visible: Vec<usize>,
    /// Position in the visible list.
    pub selected: usize,
    /// Line of the diff pane the cursor is on.
    pub cursor: usize,
    pub scroll: usize,
    pub focus: Focus,
    pub store: ReviewStore,
    pub notes: NoteStore,
    /// The note being typed, while the note editor is open.
    pub input: Option<String>,
    /// A one-off message for the status bar, cleared by the next key.
    pub message: Option<String>,
    pub clipboard: Clipboard,
    pub quit: bool,
    /// Height of the diff pane at the last draw, for paging and keeping the cursor visible.
    diff_height: usize,
}

impl App {
    pub fn new(files: Vec<FileReview>, layers: Layers) -> App {
        App::with_stores(
            files,
            layers,
            ReviewStore::in_memory(),
            NoteStore::in_memory(),
        )
    }

    pub fn with_stores(
        files: Vec<FileReview>,
        layers: Layers,
        store: ReviewStore,
        notes: NoteStore,
    ) -> App {
        let mut app = App {
            files,
            layers,
            visible: Vec::new(),
            selected: 0,
            cursor: 0,
            scroll: 0,
            focus: Focus::Files,
            store,
            notes,
            input: None,
            message: None,
            clipboard: system_clipboard,
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
                self.reset_diff_position();
            }
        }
    }

    fn reset_diff_position(&mut self) {
        self.cursor = 0;
        self.scroll = 0;
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        if self.input.is_some() {
            self.handle_note_key(key);
            return;
        }
        let page = (self.diff_height / 2).max(1);
        self.message = None;
        match key.code {
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => self.quit = true,
            KeyCode::Esc if self.focus == Focus::Diff => self.focus = Focus::Files,
            KeyCode::Esc => self.quit = true,
            KeyCode::Char('c') => {
                self.layers.comments = self.layers.comments.next();
                self.reset_diff_position();
            }
            KeyCode::Char('r') => self.toggle_reviewed(),
            KeyCode::Char('m') => self.open_note(),
            KeyCode::Char('E') => self.export_notes(),
            KeyCode::Char('t') => {
                self.layers.tests = self.layers.tests.next();
                self.refilter();
            }
            KeyCode::Char('i') => {
                self.layers.imports = self.layers.imports.next();
                self.reset_diff_position();
            }
            KeyCode::Char('l') => {
                self.layers.logging = self.layers.logging.next();
                self.reset_diff_position();
            }
            KeyCode::Char('w') => {
                self.layers.formatting = self.layers.formatting.next();
                self.reset_diff_position();
            }
            KeyCode::Right | KeyCode::Enter => self.focus = Focus::Diff,
            KeyCode::Left => self.focus = Focus::Files,
            KeyCode::Tab | KeyCode::BackTab => {
                self.focus = match self.focus {
                    Focus::Files => Focus::Diff,
                    Focus::Diff => Focus::Files,
                }
            }
            KeyCode::Down | KeyCode::Char('j') => match self.focus {
                Focus::Files => self.select(self.selected.saturating_add(1)),
                Focus::Diff => self.move_cursor(self.cursor.saturating_add(1)),
            },
            KeyCode::Up | KeyCode::Char('k') => match self.focus {
                Focus::Files => self.select(self.selected.saturating_sub(1)),
                Focus::Diff => self.move_cursor(self.cursor.saturating_sub(1)),
            },
            KeyCode::Char('n') | KeyCode::Char('J') => self.select(self.selected.saturating_add(1)),
            KeyCode::Char('p') | KeyCode::Char('K') => self.select(self.selected.saturating_sub(1)),
            KeyCode::Char('d') | KeyCode::Char(' ') | KeyCode::PageDown => {
                self.scroll = self.scroll.saturating_add(page);
                self.move_cursor(self.cursor.saturating_add(page));
            }
            KeyCode::Char('u') | KeyCode::PageUp => {
                self.scroll = self.scroll.saturating_sub(page);
                self.move_cursor(self.cursor.saturating_sub(page));
            }
            KeyCode::Char('g') | KeyCode::Home => self.move_cursor(0),
            KeyCode::Char('G') | KeyCode::End => self.move_cursor(usize::MAX),
            _ => {}
        }
    }

    fn handle_note_key(&mut self, key: KeyEvent) {
        let Some(input) = self.input.as_mut() else {
            return;
        };
        match key.code {
            KeyCode::Esc => self.input = None,
            KeyCode::Enter => self.save_note(),
            KeyCode::Backspace => {
                input.pop();
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => input.push(c),
            _ => {}
        }
    }

    /// Opens the note editor on the cursor's line, pre-filled with any note already there.
    fn open_note(&mut self) {
        if self.focus != Focus::Diff {
            self.message = Some("move into the diff (→) and pick a line to note".to_string());
            return;
        }
        let Some((path, anchor)) = self.cursor_anchor() else {
            self.message = Some("put the cursor on a code line to leave a note".to_string());
            return;
        };
        let existing = self.notes.find(&path, anchor.side, anchor.line);
        self.input = Some(existing.map(|note| note.text.clone()).unwrap_or_default());
    }

    fn save_note(&mut self) {
        let Some(text) = self.input.take() else {
            return;
        };
        let Some((path, anchor)) = self.cursor_anchor() else {
            return;
        };
        let note = Note {
            path,
            side: anchor.side,
            line: anchor.line,
            code: anchor.code,
            text,
        };
        if let Err(error) = self.notes.set(note) {
            self.message = Some(format!("could not save the note: {error:#}"));
        }
    }

    /// Copies all notes as one prompt to the clipboard and saves it next to the notes.
    fn export_notes(&mut self) {
        let count = self.notes.notes().len();
        if count == 0 {
            self.message = Some("no notes yet: press m on a diff line to add one".to_string());
            return;
        }
        let prompt = self.notes.prompt();
        let saved = self
            .notes
            .export_path()
            .filter(|path| fs::write(path, &prompt).is_ok());
        let copied = (self.clipboard)(&prompt);
        let plural = if count == 1 { "" } else { "s" };
        self.message = Some(match (copied, saved) {
            (true, Some(path)) => format!(
                "copied {count} note{plural} to the clipboard (also in {})",
                path.display()
            ),
            (true, None) => format!("copied {count} note{plural} to the clipboard"),
            (false, Some(path)) => format!("saved {count} note{plural} to {}", path.display()),
            (false, None) => "could not copy or save the notes; run `declutter notes`".to_string(),
        });
    }

    fn cursor_anchor(&self) -> Option<(String, Anchor)> {
        let path = self.current()?.path.clone();
        let (_, anchors) = self.diff_view();
        Some((path, anchors.into_iter().nth(self.cursor).flatten()?))
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
            self.reset_diff_position();
        }
    }

    fn move_cursor(&mut self, cursor: usize) {
        let last = self.diff_view().0.len().saturating_sub(1);
        self.cursor = cursor.min(last);
        self.keep_cursor_visible();
    }

    fn keep_cursor_visible(&mut self) {
        let height = self.diff_height.max(1);
        if self.cursor < self.scroll {
            self.scroll = self.cursor;
        } else if self.cursor >= self.scroll + height {
            self.scroll = self.cursor + 1 - height;
        }
    }

    /// The diff pane's lines and, for each, the file line it stands for.
    fn diff_view(&self) -> (Vec<Line<'static>>, Vec<Option<Anchor>>) {
        let message = |text: String| (vec![Line::from(text).dim()], vec![None]);
        let Some(file) = self.current() else {
            return message(match (self.files.is_empty(), self.layers.tests) {
                (true, _) => "No changes.".to_string(),
                (false, LayerMode::Hidden) => {
                    "Every changed file is a test file. Press t to cycle the test layer."
                        .to_string()
                }
                (false, _) => "No test files changed. Press t to cycle the test layer.".to_string(),
            });
        };
        let view = file.view(self.layers);
        if file.detection == Detection::Binary {
            return message("Binary file not shown.".to_string());
        }
        if view.hunks.is_empty() {
            return message(format!(
                "No visible changes: {}.",
                empty_message(file, self.layers.into())
            ));
        }

        let mut lines = Vec::new();
        let mut anchors = Vec::new();
        for (i, hunk) in view.hunks.iter().enumerate() {
            if i > 0 {
                lines.push(Line::from(""));
                anchors.push(None);
            }
            lines.push(Line::from(hunk.header()).fg(Color::Cyan));
            anchors.push(None);
            for row in &hunk.rows {
                let anchor = anchor_of(row);
                lines.push(row_line(row));
                anchors.push(anchor.clone());
                if let Some(anchor) = anchor
                    && let Some(note) = self.notes.find(&file.path, anchor.side, anchor.line)
                {
                    lines.push(
                        Line::from(format!("              ✎ {}", note.text))
                            .fg(Color::Yellow)
                            .bold(),
                    );
                    anchors.push(Some(anchor));
                }
            }
        }
        (lines, anchors)
    }
}

fn anchor_of(row: &Row) -> Option<Anchor> {
    let (side, line) = match row.kind {
        RowKind::Removed => (NoteSide::Old, row.old_line?),
        RowKind::Added | RowKind::Context => (NoteSide::New, row.new_line?),
    };
    Some(Anchor {
        side,
        line,
        code: row.text.clone(),
    })
}

fn row_line(row: &Row) -> Line<'static> {
    let number = |n: Option<usize>| n.map_or("     ".to_string(), |n| format!("{n:>5}"));
    let (sign, line, emphasis) = match row.kind {
        RowKind::Context => (' ', Style::new(), Style::new()),
        RowKind::Removed => ('-', REMOVED, REMOVED_EMPHASIS),
        RowKind::Added => ('+', ADDED, ADDED_EMPHASIS),
    };
    // Without syntax colours, colour the text itself so the change still stands out.
    let style = match row.kind {
        RowKind::Removed if row.syntax.is_empty() => line.fg(Color::Red),
        RowKind::Added if row.syntax.is_empty() => line.fg(Color::Green),
        _ => line,
    };
    let marks: Vec<(Range<usize>, Style)> = row
        .syntax
        .iter()
        .map(|(range, class)| (range.clone(), syntax_style(*class)))
        .chain(row.emphasis.iter().map(|range| (range.clone(), emphasis)))
        .collect();
    let mut spans = vec![
        Span::styled(
            format!("{} {} ", number(row.old_line), number(row.new_line)),
            Style::new().add_modifier(Modifier::DIM),
        ),
        Span::styled(format!("{sign} "), style.fg(sign_colour(row.kind)).bold()),
    ];
    spans.extend(styled_segments(&row.text, style, &marks));
    Line::from(spans)
}

// 256-colour backgrounds, so terminals without true colour (Terminal.app) show them too.
const REMOVED: Style = Style::new().bg(Color::Indexed(52));
const ADDED: Style = Style::new().bg(Color::Indexed(22));
const REMOVED_EMPHASIS: Style = Style::new()
    .bg(Color::Indexed(88))
    .add_modifier(Modifier::BOLD);
const ADDED_EMPHASIS: Style = Style::new()
    .bg(Color::Indexed(28))
    .add_modifier(Modifier::BOLD);

fn sign_colour(kind: RowKind) -> Color {
    match kind {
        RowKind::Removed => Color::Red,
        RowKind::Added => Color::Green,
        RowKind::Context => Color::Reset,
    }
}

/// Named colours, so the viewer follows the terminal's own theme.
fn syntax_style(class: Class) -> Style {
    match class {
        Class::Keyword => Style::new().fg(Color::Magenta),
        Class::String => Style::new().fg(Color::Yellow),
        Class::Comment => Style::new()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::ITALIC),
        Class::Number | Class::Constant => Style::new().fg(Color::Cyan),
        Class::Type => Style::new().fg(Color::LightCyan),
        Class::Function => Style::new().fg(Color::LightBlue),
        Class::Attribute => Style::new().fg(Color::LightMagenta),
    }
}

/// Splits `text` into spans, each styled by `base` patched with every layer covering it.
/// Later layers win where they overlap.
fn styled_segments(
    text: &str,
    base: Style,
    layers: &[(Range<usize>, Style)],
) -> Vec<Span<'static>> {
    let mut cuts: Vec<usize> = layers
        .iter()
        .flat_map(|(range, _)| [range.start, range.end])
        .filter(|&cut| cut < text.len() && text.is_char_boundary(cut))
        .chain([0, text.len()])
        .collect();
    cuts.sort_unstable();
    cuts.dedup();
    cuts.windows(2)
        .map(|pair| {
            let (start, end) = (pair[0], pair[1]);
            let style = layers
                .iter()
                .filter(|(range, _)| range.start <= start && end <= range.end)
                .fold(base, |style, (_, layer)| style.patch(*layer));
            Span::styled(text[start..end].replace('\t', "    "), style)
        })
        .collect()
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
            let visible = file.view(app.layers).hunks.len();
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
    let (lines, _) = app.diff_view();
    app.cursor = app.cursor.min(lines.len().saturating_sub(1));
    app.scroll = app.scroll.min(lines.len().saturating_sub(app.diff_height));
    app.keep_cursor_visible();
    let lines: Vec<Line> = lines
        .into_iter()
        .enumerate()
        .map(|(i, line)| {
            let on_cursor = app.focus == Focus::Diff && i == app.cursor;
            let gutter = if on_cursor {
                Span::styled("▌", Style::new().fg(Color::Cyan))
            } else {
                Span::raw(" ")
            };
            let mut spans = vec![gutter];
            spans.extend(line.spans);
            Line::from(spans).style(line.style)
        })
        .collect();
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
    let summary = match (&app.input, &app.message) {
        (Some(input), _) => format!("note › {input}█   (Enter save · Esc cancel · empty removes)"),
        (None, Some(message)) => message.clone(),
        (None, None) => format!(
            "{} · {reviewed}/{listed} reviewed{}",
            Summary::new(&app.files, app.layers).status_line(app.layers),
            match app.notes.notes().len() {
                0 => String::new(),
                n => format!(" · {n} note{}", if n == 1 { "" } else { "s" }),
            }
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

/// Pipes text into the platform's clipboard tool, whichever is installed.
fn system_clipboard(text: &str) -> bool {
    let tools: [(&str, &[&str]); 3] = [
        ("pbcopy", &[]),
        ("wl-copy", &[]),
        ("xclip", &["-selection", "clipboard"]),
    ];
    tools.iter().any(|(program, args)| {
        let Ok(mut child) = Command::new(program)
            .args(*args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        else {
            return false;
        };
        let written = child
            .stdin
            .take()
            .is_some_and(|mut stdin| stdin.write_all(text.as_bytes()).is_ok());
        child.wait().is_ok_and(|status| status.success()) && written
    })
}

pub fn run(
    files: Vec<FileReview>,
    layers: Layers,
    store: ReviewStore,
    notes: NoteStore,
) -> Result<()> {
    let mut app = App::with_stores(files, layers, store, notes);
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
