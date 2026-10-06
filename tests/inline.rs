use declutter::diff::inline_changes;
use declutter::palette::Palette;
use declutter::review::{ChangeStatus, FileChange, FileReview, Layers};
use declutter::tui::{App, draw};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::style::Color;

fn marked<'a>(line: &'a str, marks: &[std::ops::Range<usize>]) -> Vec<&'a str> {
    marks.iter().map(|range| &line[range.clone()]).collect()
}

#[test]
fn only_the_changed_tokens_of_an_edited_line_are_marked() {
    let (old, new) = (
        "export const limit = 10;",
        "export const limit = 20; // raised",
    );
    let (old_marks, new_marks) = inline_changes(old, new).expect("lines are related");

    assert_eq!(marked(old, &old_marks), ["10"]);
    assert_eq!(marked(new, &new_marks), ["20", " // raised"]);
}

#[test]
fn renamed_identifier_is_marked_as_one_token() {
    let (old, new) = ("let total = sum(items)", "let subtotal = sum(items)");
    let (old_marks, new_marks) = inline_changes(old, new).expect("lines are related");

    assert_eq!(marked(old, &old_marks), ["total"]);
    assert_eq!(marked(new, &new_marks), ["subtotal"]);
}

#[test]
fn unrelated_lines_are_not_marked() {
    assert_eq!(
        inline_changes("return cart.total()", "import Foundation"),
        None
    );
}

#[test]
fn the_viewer_paints_the_changed_tokens_with_a_background() {
    let file = FileReview::new(FileChange {
        path: "limits.ts".to_string(),
        old_path: None,
        status: ChangeStatus::Modified,
        old: Some("export const limit = 10;\n".to_string()),
        new: Some("export const limit = 20;\n".to_string()),
        binary: false,
    });
    let mut app = App::new(vec![file], Layers::default());
    let mut terminal = Terminal::new(TestBackend::new(100, 12)).expect("terminal");
    terminal.draw(|frame| draw(frame, &mut app)).expect("draw");

    let buffer = terminal.backend().buffer();
    let removed = Palette::TRUE_COLOR
        .removed_emphasis
        .bg
        .expect("a background");
    let added = Palette::TRUE_COLOR.added_emphasis.bg.expect("a background");
    let backgrounds: Vec<(String, Color)> = buffer
        .content()
        .iter()
        .filter(|cell| cell.bg == removed || cell.bg == added)
        .map(|cell| (cell.symbol().to_string(), cell.bg))
        .collect();
    assert_eq!(
        backgrounds,
        [
            ("1".to_string(), removed),
            ("0".to_string(), removed),
            ("2".to_string(), added),
            ("0".to_string(), added),
        ]
    );
}

#[test]
fn the_viewer_colours_code_by_syntax() {
    let file = FileReview::new(FileChange {
        path: "limits.ts".to_string(),
        old_path: None,
        status: ChangeStatus::Modified,
        old: Some("let a = 1;\n".to_string()),
        new: Some("const a = 1;\n".to_string()),
        binary: false,
    });
    let mut app = App::new(vec![file], Layers::default());
    let mut terminal = Terminal::new(TestBackend::new(100, 12)).expect("terminal");
    terminal.draw(|frame| draw(frame, &mut app)).expect("draw");

    let buffer = terminal.backend().buffer();
    let row = buffer
        .content()
        .chunks(buffer.area.width as usize)
        .find(|row| {
            row.iter()
                .map(|c| c.symbol())
                .collect::<String>()
                .contains("+ const")
        })
        .expect("added row is drawn");
    let text: String = row.iter().map(|c| c.symbol()).collect();
    let at = text.find("const").expect("keyword drawn");
    let column = text[..at].chars().count();
    assert_eq!(row[column].fg, Palette::TRUE_COLOR.keyword);
}

fn added_row_cells(app: &mut App, needle: &str) -> Vec<ratatui::buffer::Cell> {
    let mut terminal = Terminal::new(TestBackend::new(100, 12)).expect("terminal");
    terminal.draw(|frame| draw(frame, app)).expect("draw");
    let buffer = terminal.backend().buffer();
    let row = buffer
        .content()
        .chunks(buffer.area.width as usize)
        .find(|row| {
            row.iter()
                .map(|c| c.symbol())
                .collect::<String>()
                .contains(needle)
        })
        .expect("row is drawn");
    let text: String = row.iter().map(|c| c.symbol()).collect();
    let column = text[..text.find(needle).expect("found")].chars().count();
    row[column..column + needle.chars().count()].to_vec()
}

fn swift_app() -> App {
    let file = FileReview::new(FileChange {
        path: "Cart.swift".to_string(),
        old_path: None,
        status: ChangeStatus::Modified,
        old: Some("let a = 1\n".to_string()),
        new: Some("let a: Cart = make(\"x\")\n".to_string()),
        binary: false,
    });
    App::new(vec![file], Layers::default())
}

#[test]
fn types_and_calls_keep_the_terminal_text_colour() {
    let mut app = swift_app();
    let palette = Palette::TRUE_COLOR;

    let cells = added_row_cells(&mut app, "let a: Cart = make(\"x\")");
    let colour_of = |at: usize| cells[at].fg;
    assert_eq!(colour_of(0), palette.keyword, "let");
    assert_eq!(colour_of(7), Color::Reset, "the type Cart");
    assert_eq!(colour_of(14), Color::Reset, "the call make");
    assert_eq!(colour_of(19), palette.string, "the string");
}

#[test]
fn s_turns_syntax_colouring_off_and_on() {
    let mut app = swift_app();
    let palette = Palette::TRUE_COLOR;

    app.handle_key(ratatui::crossterm::event::KeyEvent::from(
        ratatui::crossterm::event::KeyCode::Char('s'),
    ));
    let plain = added_row_cells(&mut app, "let a: Cart");
    assert!(
        plain.iter().all(|cell| cell.fg == palette.added_sign),
        "the whole line in the diff colour"
    );

    app.handle_key(ratatui::crossterm::event::KeyEvent::from(
        ratatui::crossterm::event::KeyCode::Char('s'),
    ));
    assert_eq!(added_row_cells(&mut app, "let")[0].fg, palette.keyword);
}

#[test]
fn without_true_colour_changed_lines_get_no_background_only_the_changed_words() {
    let mut app = swift_app();
    app.palette = Palette::COLOR_256;

    let cells = added_row_cells(&mut app, "let a: Cart");
    assert_eq!(cells[0].bg, Color::Reset, "no tint behind the line");
    assert!(
        cells.iter().any(|cell| cell.bg == Color::Indexed(22)),
        "the changed words keep a background"
    );
}
