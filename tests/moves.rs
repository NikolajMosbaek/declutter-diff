use declutter::moves::{Direction, detect};
use declutter::render::plain;
use declutter::review::{ChangeStatus, DiffModes, FileChange, FileReview, Layers};
use declutter::tui::{App, draw};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyCode, KeyEvent};

fn review(path: &str, old: &str, new: &str) -> FileReview {
    FileReview::new(FileChange {
        path: path.to_string(),
        old_path: None,
        status: ChangeStatus::Modified,
        old: Some(old.to_string()),
        new: Some(new.to_string()),
        binary: false,
    })
}

const BLOCK: &str = "func validate(cart: Cart) -> Bool {\n    guard cart.items.count > 0 else { return false }\n    return cart.total > 0\n}\n";

/// Cart.swift loses `validate`; Checkout.swift gains it, indented inside a type.
fn moved_across_files() -> Vec<FileReview> {
    let indented: String = BLOCK.lines().map(|line| format!("    {line}\n")).collect();
    vec![
        review(
            "Cart.swift",
            &format!("let a = 1\n{BLOCK}let b = 2\n"),
            "let a = 1\nlet b = 2\n",
        ),
        review(
            "Checkout.swift",
            "struct Checkout {\n}\n",
            &format!("struct Checkout {{\n{indented}}}\n"),
        ),
    ]
}

#[test]
fn a_block_moved_to_another_file_is_found_even_when_reindented() {
    let files = moved_across_files();
    let refs: Vec<&FileReview> = files.iter().collect();
    let moves = detect(&refs, DiffModes::SHOWN);

    let away: Vec<_> = moves
        .iter()
        .filter(|(_, m)| m.direction == Direction::To)
        .collect();
    let here: Vec<_> = moves
        .iter()
        .filter(|(_, m)| m.direction == Direction::From)
        .collect();
    assert_eq!((away.len(), here.len()), (4, 4));
    let start = moves
        .values()
        .find(|m| m.starts_block && m.direction == Direction::To)
        .expect("block start");
    assert_eq!(
        start.describe("Cart.swift"),
        "⇄ 4 lines moved to Checkout.swift:2"
    );
    let start = moves
        .values()
        .find(|m| m.starts_block && m.direction == Direction::From)
        .expect("block start");
    assert_eq!(
        start.describe("Checkout.swift"),
        "⇄ 4 lines moved from Cart.swift:2"
    );
}

#[test]
fn a_block_moved_within_a_file_points_at_the_line() {
    let old = format!(
        "{BLOCK}let a = 1\nlet b = 2\nlet c = 3\nlet d = 4\nlet e = 5\nlet f = 6\nlet g = 7\n"
    );
    let new = format!(
        "let a = 1\nlet b = 2\nlet c = 3\nlet d = 4\nlet e = 5\nlet f = 6\nlet g = 7\n{BLOCK}"
    );
    let file = review("Cart.swift", &old, &new);
    let moves = detect(&[&file], DiffModes::SHOWN);

    let start = moves
        .values()
        .find(|m| m.starts_block && m.direction == Direction::To)
        .expect("block start");
    assert_eq!(start.describe("Cart.swift"), "⇄ 4 lines moved to line 8");
}

#[test]
fn short_or_trivial_runs_are_not_moves() {
    let stay = "let a = 0\nlet b = 0\nlet c = 0\nlet d = 0\n";
    let two_lines = review(
        "a.swift",
        &format!("let x = compute(1)\nlet y = compute(2)\n{stay}"),
        &format!("{stay}let x = compute(1)\nlet y = compute(2)\n"),
    );
    let braces = review(
        "b.swift",
        &format!("}}\n}}\n}}\n{stay}"),
        &format!("{stay}}}\n}}\n}}\n"),
    );

    let changed = |file: &FileReview| {
        file.view(DiffModes::SHOWN)
            .hunks
            .iter()
            .map(|h| h.changed().count())
            .sum::<usize>()
    };
    assert_eq!(
        changed(&two_lines),
        4,
        "the two lines are removed and re-added"
    );
    assert_eq!(changed(&braces), 6, "the braces are removed and re-added");
    assert!(detect(&[&two_lines], DiffModes::SHOWN).is_empty());
    assert!(detect(&[&braces], DiffModes::SHOWN).is_empty());
}

#[test]
fn printed_output_marks_where_a_block_went() {
    let text = plain(&moved_across_files(), Layers::default());

    assert!(
        text.contains("⇄ 4 lines moved to Checkout.swift:2\n-func validate"),
        "{text}"
    );
    assert!(
        text.contains("⇄ 4 lines moved from Cart.swift:2\n+    func validate"),
        "{text}"
    );
}

#[test]
fn v_collapses_moved_blocks_to_their_marker() {
    let mut app = App::new(moved_across_files(), Layers::default());
    let screen = |app: &mut App| {
        let mut terminal = Terminal::new(TestBackend::new(120, 20)).expect("terminal");
        terminal.draw(|frame| draw(frame, app)).expect("draw");
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect::<String>()
    };

    let expanded = screen(&mut app);
    assert!(
        expanded.contains("⇄ 4 lines moved to Checkout.swift:2"),
        "{expanded}"
    );
    assert!(expanded.contains("- func validate"), "{expanded}");
    assert!(expanded.contains("1 moved block"), "{expanded}");

    app.handle_key(KeyEvent::from(KeyCode::Char('v')));
    let collapsed = screen(&mut app);
    assert!(
        collapsed.contains("⇄ 4 lines moved to Checkout.swift:2  (v to expand)"),
        "{collapsed}"
    );
    assert!(!collapsed.contains("- func validate"), "{collapsed}");
}
