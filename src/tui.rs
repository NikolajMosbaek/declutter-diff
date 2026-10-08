use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::ops::Range;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use anyhow::Result;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Flex, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, List, ListItem, ListState, Paragraph};
use ratatui::{DefaultTerminal, Frame};

use crate::diff::{Row, RowKind};
use crate::editor::editor_command;
use crate::moves::{Direction, Moves, detect};
use crate::palette::Palette;
use crate::project::LayerMode;
use crate::render::{empty_message, file_title};
use crate::review::{ChangeStatus, Detection, FileReview, Layers, Summary};
use crate::store::{Note, NoteSide, NoteStore, ReviewStore};

/// Key help, most-used first: a narrow terminal cuts the end off. `?` shows them all.
const HELP: &str = " ↑↓ move  ←→ pane  ]/[ change  }/{ file  / search  r reviewed  m note  c t i l f layers  a all  ? help  q quit";

/// A titled group of (keys, action) pairs on the `?` screen.
type KeyGroup = (&'static str, &'static [(&'static str, &'static str)]);

/// Every key, for the `?` screen, as two columns of groups.
const KEYS: [&[KeyGroup]; 2] = [
    &[
        (
            "Move",
            &[
                ("↑ ↓  j k", "move: files, or the diff cursor"),
                ("← →  Tab ⏎", "switch pane"),
                ("] [", "next / previous change (on across files)"),
                ("} {", "next / previous file"),
                ("Space  b", "page down / up"),
                ("d  u", "half a page down / up"),
                ("g  G", "top / bottom (in files: first / last)"),
                ("/  n  N", "search; next / previous match"),
            ],
        ),
        (
            "Other",
            &[
                ("Esc", "back: leave the diff, cancel, clear search"),
                ("?", "this help"),
                ("q", "quit"),
            ],
        ),
    ],
    &[
        (
            "Review",
            &[
                ("r", "mark reviewed, go to the next file"),
                ("m", "note the line under the cursor"),
                ("P", "note on the change as a whole"),
                ("Alt+⏎", "new line, while writing a note"),
                ("E", "copy all notes as one prompt"),
                ("o", "open in $EDITOR at the line"),
            ],
        ),
        (
            "Layers: key hides / shows, Shift = only",
            &[
                ("c  C", "comments"),
                ("t  T", "tests"),
                ("i  I", "imports"),
                ("l  L", "logging"),
                ("f  F", "formatting-only changes"),
                ("a", "all filters off / back on"),
                ("M", "collapse moved blocks"),
                ("s", "syntax colouring on / off"),
            ],
        ),
    ],
];

/// The pane the arrow keys act on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Files,
    Diff,
}

/// Text being typed: a note (on the cursor's line, or `General`ly on the change as a
/// whole), or a search in the status bar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Input {
    Note(String),
    General(String),
    Search(String),
}

/// The layers, as keys for remembering what a layer was before *only*.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Layer {
    Comments,
    Tests,
    Imports,
    Logging,
    Formatting,
}

/// The line of a file a diff line stands for, so notes can be attached to it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Anchor {
    side: NoteSide,
    line: usize,
    code: String,
}

/// The diff pane's content for one file.
struct DiffView {
    lines: Vec<Line<'static>>,
    /// For each line, the file line it stands for.
    anchors: Vec<Option<Anchor>>,
    /// For each line that shows code, that code, for search.
    texts: Vec<Option<String>>,
    /// The first changed line of each hunk.
    changes: Vec<usize>,
}

/// Where the cursor is in file terms, so it can be found again after the view changes.
#[derive(Debug, Clone, Copy)]
struct Place {
    side: NoteSide,
    line: usize,
    /// The cursor's row on screen, kept so the view does not jump.
    row: usize,
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
    /// The prompt being typed, if one is open.
    pub input: Option<Input>,
    /// Where typing goes in the input, in characters from its start.
    input_cursor: usize,
    /// The last search, highlighted in the diff until cleared with Esc.
    pub search: Option<String>,
    pub show_help: bool,
    /// A one-off message for the status bar, cleared by the next key.
    pub message: Option<String>,
    pub clipboard: Clipboard,
    /// Moved blocks among the listed files, keyed by (visible position, hunk, row).
    moves: Moves,
    /// Show each moved block as its one-line marker only.
    pub collapse_moves: bool,
    /// What each layer was before it was switched to *only*, to switch back to.
    before_only: HashMap<Layer, LayerMode>,
    /// The layers as they were when `a` showed everything, for `a` to bring back.
    saved_layers: Option<Layers>,
    /// The working tree's top-level directory; changed paths are relative to it.
    pub root: PathBuf,
    /// A file and line to open in the editor, for the run loop to carry out.
    pub open_request: Option<(PathBuf, usize)>,
    /// What is being reviewed, shown over the file list: "origin/main...feature", "PR 7".
    pub title: String,
    pub palette: Palette,
    /// Colour code by syntax; off leaves only the diff's own colouring.
    pub syntax: bool,
    pub quit: bool,
    /// Height of the diff pane at the last draw, for paging and keeping the cursor visible.
    diff_height: usize,
    /// Width of the diff pane's text at the last draw, for wrapping notes.
    diff_width: usize,
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
            input_cursor: 0,
            search: None,
            show_help: false,
            message: None,
            clipboard: system_clipboard,
            moves: Moves::new(),
            collapse_moves: false,
            before_only: HashMap::new(),
            saved_layers: None,
            root: PathBuf::from("."),
            open_request: None,
            title: String::new(),
            palette: Palette::TRUE_COLOR,
            syntax: true,
            quit: false,
            diff_height: 20,
            diff_width: 80,
        };
        app.refilter();
        app.refresh_moves();
        app.go_to_first_change();
        app
    }

    /// The file under the cursor, if any file is listed.
    pub fn current(&self) -> Option<&FileReview> {
        self.visible
            .get(self.selected)
            .map(|&index| &self.files[index])
    }

    /// Rebuilds the visible list after the test layer changed, keeping the selection on
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
                self.go_to_first_change();
            }
        }
    }

    /// Puts the cursor on the current file's first change.
    fn go_to_first_change(&mut self) {
        self.cursor = self.diff_view().changes.first().copied().unwrap_or(0);
        self.scroll = self.cursor.saturating_sub(1);
    }

    /// Re-finds moved blocks: what counts as moved depends on the layers and the files listed.
    fn refresh_moves(&mut self) {
        let listed: Vec<&FileReview> = self.visible.iter().map(|&i| &self.files[i]).collect();
        self.moves = detect(&listed, self.layers.into());
    }

    /// Number of moved blocks among the listed files.
    pub fn moved_blocks(&self) -> usize {
        self.moves
            .values()
            .filter(|moved| moved.starts_block && moved.direction == Direction::To)
            .count()
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        if self.show_help {
            self.show_help = false;
            return;
        }
        if self.input.is_some() {
            self.handle_input_key(key);
            return;
        }
        self.message = None;
        let half_page = (self.diff_height / 2).max(1);
        let page = self.diff_height.saturating_sub(1).max(1);
        match key.code {
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => self.quit = true,
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Char('?') => self.show_help = true,
            KeyCode::Esc => match self.focus {
                Focus::Diff => self.focus = Focus::Files,
                Focus::Files => self.search = None,
            },

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
            KeyCode::Char(']') => self.next_change(),
            KeyCode::Char('[') => self.previous_change(),
            KeyCode::Char('}') => self.select(self.selected.saturating_add(1)),
            KeyCode::Char('{') => self.select(self.selected.saturating_sub(1)),
            KeyCode::Char(' ') | KeyCode::PageDown => self.page(page as isize),
            KeyCode::Char('b') | KeyCode::PageUp => self.page(-(page as isize)),
            KeyCode::Char('d') => self.page(half_page as isize),
            KeyCode::Char('u') => self.page(-(half_page as isize)),
            KeyCode::Char('g') | KeyCode::Home => match self.focus {
                Focus::Files => self.select(0),
                Focus::Diff => self.move_cursor(0),
            },
            KeyCode::Char('G') | KeyCode::End => match self.focus {
                Focus::Files => self.select(usize::MAX),
                Focus::Diff => self.move_cursor(usize::MAX),
            },
            KeyCode::Char('/') => self.start_input(Input::Search(String::new())),
            KeyCode::Char('n') => self.find(true),
            KeyCode::Char('N') => self.find(false),

            KeyCode::Char('r') => self.toggle_reviewed(),
            KeyCode::Char('m') => self.open_note(),
            KeyCode::Char('P') => self.open_general_note(),
            KeyCode::Char('E') => self.export_notes(),
            KeyCode::Char('o') => self.request_open(),

            KeyCode::Char('c') => self.toggle_hidden(Layer::Comments),
            KeyCode::Char('C') => self.toggle_only(Layer::Comments),
            KeyCode::Char('t') => self.toggle_hidden(Layer::Tests),
            KeyCode::Char('T') => self.toggle_only(Layer::Tests),
            KeyCode::Char('i') => self.toggle_hidden(Layer::Imports),
            KeyCode::Char('I') => self.toggle_only(Layer::Imports),
            KeyCode::Char('l') => self.toggle_hidden(Layer::Logging),
            KeyCode::Char('L') => self.toggle_only(Layer::Logging),
            KeyCode::Char('f') => self.toggle_hidden(Layer::Formatting),
            KeyCode::Char('F') => self.toggle_only(Layer::Formatting),
            KeyCode::Char('s') => self.syntax = !self.syntax,
            KeyCode::Char('a') => self.toggle_all(),
            KeyCode::Char('M') => {
                let place = self.place();
                self.collapse_moves = !self.collapse_moves;
                self.restore(place);
            }
            _ => {}
        }
    }

    fn handle_input_key(&mut self, key: KeyEvent) {
        let Some(input) = self.input.as_mut() else {
            return;
        };
        let is_note = !matches!(input, Input::Search(_));
        let text = match input {
            Input::Note(text) | Input::General(text) | Input::Search(text) => text,
        };
        let chars = text.chars().count();
        self.input_cursor = self.input_cursor.min(chars);
        let at = |cursor: usize| {
            text.char_indices()
                .nth(cursor)
                .map_or(text.len(), |(i, _)| i)
        };
        match key.code {
            KeyCode::Esc => self.input = None,
            // Terminals send Enter for Shift+Enter; Alt+Enter is the one that comes through.
            KeyCode::Enter
                if is_note
                    && key
                        .modifiers
                        .intersects(KeyModifiers::ALT | KeyModifiers::SHIFT) =>
            {
                text.insert(at(self.input_cursor), '\n');
                self.input_cursor += 1;
            }
            KeyCode::Enter => match self.input.take() {
                Some(Input::Note(text)) => self.save_note(text),
                Some(Input::General(text)) => self.save_general_note(text),
                Some(Input::Search(query)) => {
                    if !query.is_empty() {
                        self.search = Some(query);
                    }
                    self.find(true);
                }
                None => {}
            },
            KeyCode::Left => self.input_cursor = self.input_cursor.saturating_sub(1),
            KeyCode::Right => self.input_cursor = (self.input_cursor + 1).min(chars),
            KeyCode::Home => self.input_cursor = 0,
            KeyCode::End => self.input_cursor = chars,
            KeyCode::Backspace if self.input_cursor > 0 => {
                self.input_cursor -= 1;
                text.remove(at(self.input_cursor));
            }
            KeyCode::Delete if self.input_cursor < chars => {
                text.remove(at(self.input_cursor));
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                text.insert(at(self.input_cursor), c);
                self.input_cursor += 1;
            }
            _ => {}
        }
    }

    /// Opens `input` with the cursor at the end of its text.
    fn start_input(&mut self, input: Input) {
        self.input_cursor = match &input {
            Input::Note(text) | Input::General(text) | Input::Search(text) => text.chars().count(),
        };
        self.input = Some(input);
    }

    fn layer_mut(&mut self, layer: Layer) -> &mut LayerMode {
        match layer {
            Layer::Comments => &mut self.layers.comments,
            Layer::Tests => &mut self.layers.tests,
            Layer::Imports => &mut self.layers.imports,
            Layer::Logging => &mut self.layers.logging,
            Layer::Formatting => &mut self.layers.formatting,
        }
    }

    /// Lower-case layer key: hidden ↔ shown; from *only*, back to shown.
    fn toggle_hidden(&mut self, layer: Layer) {
        let before = self.where_am_i();
        let mode = self.layer_mut(layer);
        *mode = match *mode {
            LayerMode::Shown => LayerMode::Hidden,
            LayerMode::Hidden | LayerMode::Only => LayerMode::Shown,
        };
        self.layers_changed(layer, before);
    }

    /// Shifted layer key: the layer alone; pressed again, back to what it was.
    fn toggle_only(&mut self, layer: Layer) {
        let before = self.where_am_i();
        let current = *self.layer_mut(layer);
        let next = if current == LayerMode::Only {
            self.before_only.remove(&layer).unwrap_or(LayerMode::Shown)
        } else {
            self.before_only.insert(layer, current);
            LayerMode::Only
        };
        *self.layer_mut(layer) = next;
        self.layers_changed(layer, before);
    }

    /// `a`: with any layer filtering, shows everything and remembers how it was; pressed
    /// again, puts it back. With nothing to put back, hides every layer.
    fn toggle_all(&mut self) {
        let before = self.where_am_i();
        let all = |mode| Layers {
            comments: mode,
            tests: mode,
            imports: mode,
            logging: mode,
            formatting: mode,
        };
        let (next, message) = if self.layers != all(LayerMode::Shown) {
            self.saved_layers = Some(self.layers);
            (
                all(LayerMode::Shown),
                "all layers shown · a puts the filters back",
            )
        } else {
            match self.saved_layers.take() {
                Some(saved) => (saved, "filters back on"),
                None => (all(LayerMode::Hidden), "every layer hidden"),
            }
        };
        self.layers = next;
        self.before_only.clear();
        self.layers_changed(Layer::Tests, before);
        self.message = Some(message.to_string());
    }

    /// The selected file and the cursor's place in it, taken before a layer changes.
    fn where_am_i(&self) -> (Option<usize>, Option<Place>) {
        (self.visible.get(self.selected).copied(), self.place())
    }

    /// Re-derives everything a layer change affects, keeping the cursor on the same line
    /// of the same file where that line is still shown.
    fn layers_changed(&mut self, layer: Layer, (file, place): (Option<usize>, Option<Place>)) {
        if layer == Layer::Tests {
            self.refilter();
        }
        self.refresh_moves();
        if self.visible.get(self.selected).copied() == file {
            self.restore(place);
        }
    }

    /// The cursor's position in file terms: its own line, or the nearest line above it.
    fn place(&self) -> Option<Place> {
        let view = self.diff_view();
        let at = self.cursor.min(view.anchors.len().saturating_sub(1));
        let anchor = (0..=at)
            .rev()
            .chain(at + 1..view.anchors.len())
            .find_map(|i| view.anchors.get(i)?.as_ref())?;
        Some(Place {
            side: anchor.side,
            line: anchor.line,
            row: self.cursor.saturating_sub(self.scroll),
        })
    }

    /// Puts the cursor back on `place` — the same line if still shown, otherwise the
    /// closest one — at the same height on screen.
    fn restore(&mut self, place: Option<Place>) {
        let Some(place) = place else {
            self.go_to_first_change();
            return;
        };
        let view = self.diff_view();
        let closest = view
            .anchors
            .iter()
            .enumerate()
            .filter_map(|(i, anchor)| Some((i, anchor.as_ref()?)))
            .min_by_key(|(_, anchor)| {
                let other_side = usize::from(anchor.side != place.side);
                (anchor.line.abs_diff(place.line), other_side)
            });
        match closest {
            Some((index, _)) => {
                self.cursor = index;
                self.scroll = index.saturating_sub(place.row);
            }
            None => self.go_to_first_change(),
        }
    }

    /// Moves the cursor to the next change, on into the next file that has one.
    fn next_change(&mut self) {
        if let Some(&line) = self
            .diff_view()
            .changes
            .iter()
            .find(|&&line| line > self.cursor)
        {
            self.jump_to(line);
            return;
        }
        let next = (self.selected + 1..self.visible.len())
            .find(|&position| !self.diff_view_of(position).changes.is_empty());
        match next {
            Some(position) => self.select(position),
            None => self.message = Some("no more changes".to_string()),
        }
    }

    /// Moves the cursor to the previous change, back into the previous file with one.
    fn previous_change(&mut self) {
        let view = self.diff_view();
        if let Some(&line) = view.changes.iter().rev().find(|&&line| line < self.cursor) {
            self.jump_to(line);
            return;
        }
        let previous = (0..self.selected)
            .rev()
            .find(|&position| !self.diff_view_of(position).changes.is_empty());
        match previous {
            Some(position) => {
                self.select(position);
                if let Some(&last) = self.diff_view().changes.last() {
                    self.jump_to(last);
                }
            }
            None => self.message = Some("no earlier changes".to_string()),
        }
    }

    /// Moves the cursor to `line`, showing the line above it for context.
    fn jump_to(&mut self, line: usize) {
        self.cursor = line;
        self.scroll = line.saturating_sub(1);
    }

    fn page(&mut self, by: isize) {
        self.scroll = self.scroll.saturating_add_signed(by);
        self.move_cursor(self.cursor.saturating_add_signed(by));
    }

    /// Jumps to the next (or previous) line matching the search, across the listed
    /// files, wrapping around at the ends.
    fn find(&mut self, forward: bool) {
        let Some(query) = self.search.clone() else {
            self.message = Some("press / to search".to_string());
            return;
        };
        let matches: Vec<(usize, usize)> = (0..self.visible.len())
            .flat_map(|position| {
                let view = self.diff_view_of(position);
                view.texts
                    .iter()
                    .enumerate()
                    .filter(|(_, text)| {
                        text.as_deref()
                            .is_some_and(|t| !find_all(t, &query).is_empty())
                    })
                    .map(|(line, _)| (position, line))
                    .collect::<Vec<_>>()
            })
            .collect();
        if matches.is_empty() {
            self.message = Some(format!("no match for “{query}”"));
            return;
        }
        let here = (self.selected, self.cursor);
        let index = if forward {
            matches.iter().position(|&m| m > here).unwrap_or(0)
        } else {
            matches
                .iter()
                .rposition(|&m| m < here)
                .unwrap_or(matches.len() - 1)
        };
        let (position, line) = matches[index];
        self.select(position);
        self.focus = Focus::Diff;
        self.cursor = line;
        self.scroll = line.saturating_sub(self.diff_height / 2);
        self.message = Some(format!(
            "match {} of {} for “{query}”",
            index + 1,
            matches.len()
        ));
    }

    /// Opens the note editor on the cursor's line, pre-filled with any note already
    /// there. From the file list it moves into the diff first.
    fn open_note(&mut self) {
        if self.focus == Focus::Files {
            self.focus = Focus::Diff;
            self.go_to_first_change();
        }
        let Some((path, anchor)) = self.cursor_anchor() else {
            self.message = Some("put the cursor on a code line to leave a note".to_string());
            return;
        };
        let existing = self.notes.find(&path, anchor.side, anchor.line);
        self.start_input(Input::Note(
            existing.map(|note| note.text.clone()).unwrap_or_default(),
        ));
    }

    /// Opens the editor on the note about the change as a whole.
    fn open_general_note(&mut self) {
        let existing = self.notes.general().map(|note| note.text.clone());
        self.start_input(Input::General(existing.unwrap_or_default()));
    }

    fn save_general_note(&mut self, text: String) {
        let note = Note {
            draft: false,
            ..Note::on_pull_request(text)
        };
        if let Err(error) = self.notes.set(note) {
            self.message = Some(format!("could not save the note: {error:#}"));
        }
    }

    fn save_note(&mut self, text: String) {
        let Some((path, anchor)) = self.cursor_anchor() else {
            return;
        };
        let note = Note {
            path,
            side: anchor.side,
            line: anchor.line,
            code: anchor.code,
            text,
            review: None,
            draft: false,
        };
        if let Err(error) = self.notes.set(note) {
            self.message = Some(format!("could not save the note: {error:#}"));
        }
    }

    /// Copies all notes as one prompt to the clipboard and saves it next to the notes.
    fn export_notes(&mut self) {
        let all = self.notes.notes();
        let count = all.iter().filter(|note| !note.draft).count();
        if count == 0 {
            self.message = Some(if all.is_empty() {
                "no notes yet: press m on a diff line to add one".to_string()
            } else {
                "only drafts so far: open one with m to include it".to_string()
            });
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

    /// Asks for the current file to be opened at the cursor's line, or, from the file
    /// list, at its first change. A removed line opens at the nearest line still there.
    fn request_open(&mut self) {
        let Some(file) = self.current() else {
            return;
        };
        if file.status == ChangeStatus::Deleted {
            self.message = Some("the file was deleted; there is nothing to open".to_string());
            return;
        }
        let path = self.root.join(&file.path);
        let view = self.diff_view();
        let new_line = |anchor: &Option<Anchor>| {
            anchor
                .as_ref()
                .filter(|anchor| anchor.side == NoteSide::New)
                .map(|anchor| anchor.line)
        };
        let from = match self.focus {
            Focus::Diff => self.cursor,
            Focus::Files => view.changes.first().copied().unwrap_or(0),
        }
        .min(view.anchors.len());
        let line = view.anchors[from..]
            .iter()
            .find_map(new_line)
            .or_else(|| view.anchors[..from].iter().rev().find_map(new_line))
            .unwrap_or(1);
        self.open_request = Some((path, line));
    }

    fn cursor_anchor(&self) -> Option<(String, Anchor)> {
        let path = self.current()?.path.clone();
        let anchor = self
            .diff_view()
            .anchors
            .into_iter()
            .nth(self.cursor)
            .flatten()?;
        Some((path, anchor))
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
            self.go_to_first_change();
        }
    }

    fn move_cursor(&mut self, cursor: usize) {
        let last = self.diff_view().lines.len().saturating_sub(1);
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

    fn diff_view(&self) -> DiffView {
        self.diff_view_of(self.selected)
    }

    /// The diff pane's content for the file at `position` in the visible list.
    fn diff_view_of(&self, position: usize) -> DiffView {
        let message = |text: String| DiffView {
            lines: vec![Line::from(text).dim()],
            anchors: vec![None],
            texts: vec![None],
            changes: Vec::new(),
        };
        let Some(file) = self.visible.get(position).map(|&index| &self.files[index]) else {
            return message(match (self.files.is_empty(), self.layers.tests) {
                (true, _) => "No changes.".to_string(),
                (false, LayerMode::Hidden) => {
                    "Every changed file is a test file. Press t to show tests.".to_string()
                }
                (false, _) => "No test files changed. Press T to show the other files.".to_string(),
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

        let mut out = DiffView {
            lines: Vec::new(),
            anchors: Vec::new(),
            texts: Vec::new(),
            changes: Vec::new(),
        };
        let push = |out: &mut DiffView,
                    line: Line<'static>,
                    anchor: Option<Anchor>,
                    text: Option<String>| {
            out.lines.push(line);
            out.anchors.push(anchor);
            out.texts.push(text);
        };
        for (hunk_index, hunk) in view.hunks.iter().enumerate() {
            if hunk_index > 0 {
                push(&mut out, Line::from(""), None, None);
            }
            push(
                &mut out,
                Line::from(hunk.header()).fg(Color::Cyan),
                None,
                None,
            );
            let mut first_change = None;
            for (row_index, row) in hunk.rows.iter().enumerate() {
                let moved = self.moves.get(&(position, hunk_index, row_index));
                if let Some(moved) = moved
                    && moved.starts_block
                {
                    let marker = moved.describe(&file.path);
                    let hint = if self.collapse_moves {
                        "  (M to expand)"
                    } else {
                        ""
                    };
                    if self.collapse_moves && row.kind != RowKind::Context {
                        first_change.get_or_insert(out.lines.len());
                    }
                    push(
                        &mut out,
                        Line::from(format!("              {marker}{hint}"))
                            .fg(Color::Cyan)
                            .italic(),
                        None,
                        None,
                    );
                }
                if moved.is_some() && self.collapse_moves {
                    continue;
                }
                if row.kind != RowKind::Context {
                    first_change.get_or_insert(out.lines.len());
                }
                let anchor = anchor_of(row);
                push(
                    &mut out,
                    row_line(
                        row,
                        moved.is_some(),
                        self.search.as_deref(),
                        &self.palette,
                        self.syntax,
                    ),
                    anchor.clone(),
                    Some(row.text.clone()),
                );
                if let Some(anchor) = anchor
                    && let Some(note) = self.notes.find(&file.path, anchor.side, anchor.line)
                {
                    for line in note_lines(note, self.diff_width) {
                        push(&mut out, line, Some(anchor.clone()), None);
                    }
                }
            }
            out.changes.extend(first_change);
        }
        out
    }
}

/// How far a note is indented under its line.
const NOTE_INDENT: usize = 14;

/// A note under its line: its text wrapped to the pane, a draft marked as one.
fn note_lines(note: &Note, width: usize) -> Vec<Line<'static>> {
    let marker = if note.draft { "✎ draft · " } else { "✎ " };
    let width = width.saturating_sub(NOTE_INDENT + 2).max(20);
    let style = if note.draft {
        Style::new().fg(Color::Yellow).italic()
    } else {
        Style::new().fg(Color::Yellow).bold()
    };
    let mut first = true;
    wrap(&format!("{marker}{}", note.text.trim()), width)
        .into_iter()
        .map(|text| {
            let indent = if first { NOTE_INDENT } else { NOTE_INDENT + 2 };
            first = false;
            Line::from(format!("{}{text}", " ".repeat(indent))).style(style)
        })
        .collect()
}

/// `text` broken into rows of at most `width` characters, at spaces where it can be,
/// keeping its own line breaks.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut rows = Vec::new();
    for line in text.split('\n') {
        let mut row = String::new();
        for word in line.split(' ') {
            let row_len = row.chars().count();
            let word_len = word.chars().count();
            if row_len > 0 && row_len + 1 + word_len > width {
                rows.push(std::mem::take(&mut row));
            } else if row_len > 0 {
                row.push(' ');
            }
            row.push_str(word);
            while row.chars().count() > width {
                let rest: String = row.chars().skip(width).collect();
                rows.push(row.chars().take(width).collect());
                row = rest;
            }
        }
        rows.push(row);
    }
    rows
}

/// Byte ranges of `query` in `text`; case-insensitive unless the query has a capital.
fn find_all(text: &str, query: &str) -> Vec<Range<usize>> {
    if query.is_empty() {
        return Vec::new();
    }
    let sensitive = query.chars().any(char::is_uppercase);
    let (haystack, needle) = if sensitive {
        (text.to_string(), query.to_string())
    } else {
        (text.to_lowercase(), query.to_lowercase())
    };
    // Lower-casing can change byte lengths outside ASCII; fall back to exact matching then.
    if haystack.len() != text.len() {
        return text
            .match_indices(query)
            .map(|(at, found)| at..at + found.len())
            .collect();
    }
    haystack
        .match_indices(&needle)
        .map(|(at, found)| at..at + found.len())
        .collect()
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

/// One diff row: line numbers, sign, and the code coloured by syntax (when `syntax` is
/// on and the file has a grammar), changed words and search matches.
fn row_line(
    row: &Row,
    moved: bool,
    search: Option<&str>,
    palette: &Palette,
    syntax: bool,
) -> Line<'static> {
    let number = |n: Option<usize>| n.map_or("     ".to_string(), |n| format!("{n:>5}"));
    let (sign, sign_colour, line, emphasis) = match (row.kind, moved) {
        (RowKind::Context, _) => (' ', Color::Reset, Style::new(), Style::new()),
        // Moved lines get their own tint: nothing about them changed but their place.
        (RowKind::Removed, true) => (
            '-',
            palette.moved_sign,
            palette.moved_away,
            palette.moved_away,
        ),
        (RowKind::Added, true) => (
            '+',
            palette.moved_sign,
            palette.moved_here,
            palette.moved_here,
        ),
        (RowKind::Removed, false) => (
            '-',
            palette.removed_sign,
            palette.removed,
            palette.removed_emphasis,
        ),
        (RowKind::Added, false) => (
            '+',
            palette.added_sign,
            palette.added,
            palette.added_emphasis,
        ),
    };
    let coloured = syntax && !row.syntax.is_empty();
    // Without syntax colours, colour the changed text itself so the change stands out.
    let style = if coloured || row.kind == RowKind::Context {
        line
    } else {
        line.fg(sign_colour)
    };
    let syntax_marks = row
        .syntax
        .iter()
        .filter(|_| coloured)
        .filter_map(|(range, class)| Some((range.clone(), palette.syntax(*class)?)));
    let marks: Vec<(Range<usize>, Style)> = syntax_marks
        .chain(row.emphasis.iter().map(|range| (range.clone(), emphasis)))
        .chain(
            search
                .map(|query| find_all(&row.text, query))
                .unwrap_or_default()
                .into_iter()
                .map(|range| (range, palette.search)),
        )
        .collect();
    let mut spans = vec![
        Span::styled(
            format!("{} {} ", number(row.old_line), number(row.new_line)),
            Style::new().add_modifier(Modifier::DIM),
        ),
        Span::styled(
            format!("{sign} "),
            line.fg(sign_colour).add_modifier(Modifier::BOLD),
        ),
    ];
    spans.extend(styled_segments(&row.text, style, &marks));
    Line::from(spans)
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
            let noted = app
                .notes
                .notes()
                .iter()
                .filter(|note| note.path == file.path)
                .count();
            let noted = match noted {
                0 => String::new(),
                n => format!(" ✎{n}"),
            };
            let item = ListItem::new(format!("{mark}{} ({visible}){noted}", file_title(file)));
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
            .block(pane(
                if app.title.is_empty() {
                    " Files ".to_string()
                } else {
                    format!(" {} ", app.title)
                },
                app.focus == Focus::Files,
            ))
            .highlight_style(Style::new().add_modifier(Modifier::REVERSED)),
        files_area,
        &mut list_state,
    );

    app.diff_height = diff_area.height.saturating_sub(2) as usize;
    // Inside the borders, less the cursor's gutter.
    app.diff_width = diff_area.width.saturating_sub(3) as usize;
    let lines = app.diff_view().lines;
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
        (Some(Input::Note(_) | Input::General(_)), _) => {
            "Enter save · Alt+Enter new line · ← → Home End move · Esc cancel · empty removes"
                .to_string()
        }
        (Some(Input::Search(query)), _) => format!(
            "/{}   (Enter search · Esc cancel)",
            with_cursor(query, app.input_cursor)
        ),
        (None, Some(message)) => message.clone(),
        (None, None) => format!(
            "{} · {reviewed}/{listed} reviewed{}",
            Summary::new(&app.files, app.layers).status_line(app.layers),
            notes_status(&app.notes)
                + &match (app.moved_blocks(), app.collapse_moves) {
                    (0, _) => String::new(),
                    (n, collapsed) => format!(
                        " · {n} moved block{}{}",
                        if n == 1 { "" } else { "s" },
                        if collapsed { " (collapsed)" } else { "" }
                    ),
                }
        ),
    };
    frame.render_widget(
        Paragraph::new(format!(" {summary}")).style(Style::new().add_modifier(Modifier::REVERSED)),
        status,
    );
    frame.render_widget(Paragraph::new(HELP).dim(), help);
    if let Some(Input::Note(text) | Input::General(text)) = &app.input {
        let title = match &app.input {
            Some(Input::General(_)) => " Note on the change as a whole ".to_string(),
            _ => match app.cursor_anchor() {
                Some((path, anchor)) if anchor.side == NoteSide::Old => {
                    format!(" Note on {path}, removed line {} ", anchor.line)
                }
                Some((path, anchor)) => format!(" Note on {path}:{} ", anchor.line),
                None => " Note ".to_string(),
            },
        };
        let width = diff_area.width.saturating_sub(4) as usize;
        let rows = wrap(&with_cursor(text, app.input_cursor), width);
        let height = (rows.len() as u16 + 2).min(diff_area.height);
        // Right under the cursor's line, where the eye already is — over the note being
        // edited — else above it, else at the bottom of the pane.
        let top = diff_area.y + 1;
        let bottom = diff_area.bottom().saturating_sub(1);
        let line = top + app.cursor.saturating_sub(app.scroll).min(u16::MAX as usize) as u16;
        let y = if line + 1 + height <= bottom + 1 {
            line + 1
        } else if line >= top + height {
            line - height
        } else {
            diff_area.bottom().saturating_sub(height)
        };
        let area = Rect {
            y,
            height,
            ..diff_area
        };
        let block = pane(title, true);
        let inner = block.inner(area);
        frame.render_widget(Clear, area);
        frame.render_widget(block, area);
        let text: Vec<Line> = rows.into_iter().map(Line::from).collect();
        frame.render_widget(
            Paragraph::new(text).fg(Color::Yellow),
            Rect {
                x: inner.x + 1,
                width: inner.width.saturating_sub(1),
                ..inner
            },
        );
    }
    if app.show_help {
        draw_help(frame);
    }
}

/// The `?` screen: every key, grouped in two columns, in a box over the viewer.
fn draw_help(frame: &mut Frame) {
    let columns: Vec<Vec<Line>> = KEYS
        .iter()
        .map(|groups| {
            let mut lines = Vec::new();
            for (group, keys) in groups.iter() {
                if !lines.is_empty() {
                    lines.push(Line::from(""));
                }
                lines.push(Line::from(*group).bold().fg(Color::Cyan));
                for (key, action) in *keys {
                    lines.push(Line::from(vec![
                        Span::styled(format!("  {key:<12}"), Style::new().bold()),
                        Span::raw(*action),
                    ]));
                }
            }
            lines
        })
        .collect();
    let widths: Vec<u16> = columns
        .iter()
        .map(|lines| lines.iter().map(Line::width).max().unwrap_or(0) as u16)
        .collect();
    let rows = columns.iter().map(Vec::len).max().unwrap_or(0) as u16;
    // Both columns, a three-column gap, and the border with a column of padding.
    let area = centered(frame.area(), widths.iter().sum::<u16>() + 7, rows + 2);
    frame.render_widget(Clear, area);
    let block = pane(" Keys · any key closes ".to_string(), true);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let [left, right] =
        Layout::horizontal([Constraint::Length(widths[0] + 4), Constraint::Min(0)]).areas(inner);
    let [left_column, right_column] = [left, right].map(|area| Rect {
        x: area.x + 1,
        width: area.width.saturating_sub(1),
        ..area
    });
    let mut columns = columns.into_iter();
    for area in [left_column, right_column] {
        frame.render_widget(Paragraph::new(columns.next().unwrap_or_default()), area);
    }
}

/// `text` with a block cursor drawn `cursor` characters in.
fn with_cursor(text: &str, cursor: usize) -> String {
    let at = text
        .char_indices()
        .nth(cursor)
        .map_or(text.len(), |(i, _)| i);
    format!("{}█{}", &text[..at], &text[at..])
}

/// " · 3 notes (2 drafts)", and a pointer to a draft note on the whole change.
fn notes_status(notes: &NoteStore) -> String {
    let all = notes.notes();
    let plural = |n: usize| if n == 1 { "" } else { "s" };
    let drafts = all.iter().filter(|note| note.draft).count();
    let mut status = match (all.len(), drafts) {
        (0, _) => String::new(),
        (n, 0) => format!(" · {n} note{}", plural(n)),
        (n, d) => format!(" · {n} note{} ({d} draft{})", plural(n), plural(d)),
    };
    if notes.general().is_some_and(|note| note.draft) {
        status.push_str(" · P: draft on the whole change");
    }
    status
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let [area] = Layout::horizontal([Constraint::Length(width.min(area.width))])
        .flex(Flex::Center)
        .areas(area);
    let [area] = Layout::vertical([Constraint::Length(height.min(area.height))])
        .flex(Flex::Center)
        .areas(area);
    area
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
    root: PathBuf,
    title: String,
    syntax: bool,
) -> Result<NoteStore> {
    let mut app = App::with_stores(files, layers, store, notes);
    app.root = root;
    app.title = title;
    app.palette = Palette::detect();
    app.syntax = syntax;
    ratatui::run(|terminal: &mut DefaultTerminal| -> Result<()> {
        while !app.quit {
            terminal.draw(|frame| draw(frame, &mut app))?;
            if let Event::Key(key) = event::read()?
                && key.kind == KeyEventKind::Press
            {
                app.handle_key(key);
            }
            if let Some((path, line)) = app.open_request.take() {
                app.message = open_in_editor(terminal, &path, line).err();
            }
        }
        Ok(())
    })?;
    Ok(app.notes)
}

/// Opens `path` at `line` in `$VISUAL` / `$EDITOR`. A terminal editor takes over the
/// screen until it exits; anything else is launched alongside the viewer.
fn open_in_editor(
    terminal: &mut DefaultTerminal,
    path: &std::path::Path,
    line: usize,
) -> Result<(), String> {
    let editor = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .ok()
        .filter(|value| !value.trim().is_empty());
    let command = editor_command(editor.as_deref(), path, line);
    let mut process = Command::new(&command.program);
    process.args(&command.args);
    let failed = |error: std::io::Error| format!("could not start `{}`: {error}", command.program);
    if command.in_terminal {
        ratatui::restore();
        let status = process.status();
        *terminal = ratatui::init();
        status.map_err(failed)?;
    } else {
        process
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(failed)?;
    }
    Ok(())
}
