use declutter::diff::inline_changes;
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
    let backgrounds: Vec<(String, Color)> = buffer
        .content()
        .iter()
        .filter(|cell| cell.bg == Color::Indexed(22) || cell.bg == Color::Indexed(52))
        .map(|cell| (cell.symbol().to_string(), cell.bg))
        .collect();
    assert_eq!(
        backgrounds,
        [
            ("1".to_string(), Color::Indexed(52)),
            ("0".to_string(), Color::Indexed(52)),
            ("2".to_string(), Color::Indexed(22)),
            ("0".to_string(), Color::Indexed(22)),
        ]
    );
}
