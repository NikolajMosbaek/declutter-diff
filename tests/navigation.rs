use declutter::review::{ChangeStatus, FileChange, FileReview, Layers};
use declutter::tui::{App, Focus, Input, draw};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyCode, KeyEvent};

fn change(path: &str, old: &str, new: &str) -> FileReview {
    FileReview::new(FileChange {
        path: path.to_string(),
        old_path: None,
        status: ChangeStatus::Modified,
        old: Some(old.to_string()),
        new: Some(new.to_string()),
        binary: false,
    })
}

/// Forty lines with `line 5` and `line 30` changed: two hunks far apart.
fn two_hunks(path: &str) -> FileReview {
    let old: String = (1..=40).map(|i| format!("let line{i} = {i}\n")).collect();
    let new = old
        .replace("= 5\n", "= 500\n")
        .replace("= 30\n", "= 3000\n");
    change(path, &old, &new)
}

fn press(app: &mut App, code: KeyCode) {
    app.handle_key(KeyEvent::from(code));
}

fn screen(app: &mut App) -> String {
    let mut terminal = Terminal::new(TestBackend::new(120, 24)).expect("terminal");
    terminal.draw(|frame| draw(frame, app)).expect("draw");
    terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|c| c.symbol())
        .collect()
}

/// The text of the code line under the cursor, as drawn.
fn cursor_line(app: &mut App) -> String {
    let mut terminal = Terminal::new(TestBackend::new(120, 24)).expect("terminal");
    terminal.draw(|frame| draw(frame, app)).expect("draw");
    let buffer = terminal.backend().buffer();
    let width = buffer.area.width as usize;
    buffer
        .content()
        .chunks(width)
        .map(|row| row.iter().map(|c| c.symbol()).collect::<String>())
        .find(|row| row.contains('▌'))
        .unwrap_or_default()
}

#[test]
fn brackets_jump_from_change_to_change_and_on_into_the_next_file() {
    let mut app = App::new(
        vec![two_hunks("A.swift"), two_hunks("B.swift")],
        Layers::default(),
    );
    press(&mut app, KeyCode::Right);
    assert!(cursor_line(&mut app).contains("- let line5 = 5"));

    press(&mut app, KeyCode::Char(']'));
    assert!(cursor_line(&mut app).contains("- let line30 = 30"));

    press(&mut app, KeyCode::Char(']'));
    assert_eq!(app.current().map(|f| f.path.as_str()), Some("B.swift"));
    assert!(cursor_line(&mut app).contains("- let line5 = 5"));

    press(&mut app, KeyCode::Char('['));
    assert_eq!(app.current().map(|f| f.path.as_str()), Some("A.swift"));
    assert!(cursor_line(&mut app).contains("- let line30 = 30"));
}

#[test]
fn braces_move_between_files_from_either_pane() {
    let mut app = App::new(
        vec![two_hunks("A.swift"), two_hunks("B.swift")],
        Layers::default(),
    );
    press(&mut app, KeyCode::Right);
    press(&mut app, KeyCode::Char('}'));
    assert_eq!(app.selected, 1);
    press(&mut app, KeyCode::Char('{'));
    assert_eq!(app.selected, 0);
}

#[test]
fn toggling_a_layer_keeps_the_cursor_on_the_same_line() {
    let comments: String = (1..=12).map(|i| format!("// note {i}\n")).collect();
    let old = format!("{comments}let a = 1\nlet b = 2\nlet c = 3\n");
    let new = format!("{comments}let a = 10\nlet b = 20\nlet c = 30\n");
    let mut app = App::new(vec![change("Rates.swift", &old, &new)], Layers::default());
    press(&mut app, KeyCode::Right);
    for _ in 0..4 {
        press(&mut app, KeyCode::Down);
    }
    assert!(cursor_line(&mut app).contains("+ let b = 20"));

    press(&mut app, KeyCode::Char('c'));
    assert!(
        cursor_line(&mut app).contains("+ let b = 20"),
        "comments shown"
    );
    press(&mut app, KeyCode::Char('c'));
    assert!(
        cursor_line(&mut app).contains("+ let b = 20"),
        "comments hidden again"
    );
}

#[test]
fn escape_goes_back_but_never_quits() {
    let mut app = App::new(vec![two_hunks("A.swift")], Layers::default());
    press(&mut app, KeyCode::Right);
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.focus, Focus::Files);
    press(&mut app, KeyCode::Esc);
    assert!(!app.quit);
    press(&mut app, KeyCode::Char('q'));
    assert!(app.quit);
}

#[test]
fn g_and_shift_g_follow_the_focused_pane() {
    let files = vec![
        two_hunks("A.swift"),
        two_hunks("B.swift"),
        two_hunks("C.swift"),
    ];
    let mut app = App::new(files, Layers::default());
    press(&mut app, KeyCode::Char('G'));
    assert_eq!(app.selected, 2);
    press(&mut app, KeyCode::Char('g'));
    assert_eq!(app.selected, 0);

    press(&mut app, KeyCode::Right);
    press(&mut app, KeyCode::Char('G'));
    assert_eq!(
        app.selected, 0,
        "in the diff, G moves the cursor, not the file"
    );
    assert!(app.cursor > 5);
}

#[test]
fn m_from_the_file_list_notes_the_first_change() {
    let mut app = App::new(vec![two_hunks("A.swift")], Layers::default());
    press(&mut app, KeyCode::Char('m'));

    assert_eq!(app.focus, Focus::Diff);
    assert_eq!(app.input, Some(Input::Note(String::new())));
    assert!(cursor_line(&mut app).contains("- let line5 = 5"));
}

#[test]
fn space_pages_a_screen_and_d_half_a_screen() {
    let old: String = (1..=200).map(|i| format!("let v{i} = {i}\n")).collect();
    let new: String = (1..=200)
        .map(|i| format!("let v{i} = {}\n", i * 2))
        .collect();
    let mut app = App::new(vec![change("Big.swift", &old, &new)], Layers::default());
    screen(&mut app);
    press(&mut app, KeyCode::Right);
    let start = app.cursor;

    press(&mut app, KeyCode::Char('d'));
    let half = app.cursor - start;
    press(&mut app, KeyCode::Char(' '));
    let full = app.cursor - start - half;
    assert!(full >= 2 * half - 1 && half > 0, "half {half}, full {full}");
    press(&mut app, KeyCode::Char('b'));
    assert_eq!(app.cursor - start, half);
}

#[test]
fn question_mark_lists_every_key_and_any_key_closes_it() {
    let mut app = App::new(vec![two_hunks("A.swift")], Layers::default());
    press(&mut app, KeyCode::Char('?'));
    let help = screen(&mut app);
    for key in ["] [", "} {", "/  n  N", "c  C", "f  F", "M ", "Esc", "? "] {
        assert!(help.contains(key), "{key} missing from help");
    }

    press(&mut app, KeyCode::Char('q'));
    assert!(!app.quit, "the key that closes help does nothing else");
    assert!(!app.show_help);
}

#[test]
fn search_finds_matches_across_files_and_wraps() {
    let files = vec![
        change("A.swift", "let total = 1\n", "let total = 2\n"),
        change("B.swift", "let other = 1\n", "let other = 2\n"),
        change("C.swift", "let Total = 1\n", "let subtotal = 3\n"),
    ];
    let mut app = App::new(files, Layers::default());
    press(&mut app, KeyCode::Char('/'));
    for c in "total".chars() {
        press(&mut app, KeyCode::Char(c));
    }
    press(&mut app, KeyCode::Enter);
    let at = |app: &App| app.current().map(|f| f.path.clone()).unwrap_or_default();

    // Matches: A's - and + lines, C's - and + lines; the cursor starts on A's - line.
    assert_eq!(at(&app), "A.swift");
    assert_eq!(app.message.as_deref(), Some("match 2 of 4 for “total”"));
    press(&mut app, KeyCode::Char('n'));
    assert_eq!(at(&app), "C.swift");
    press(&mut app, KeyCode::Char('N'));
    assert_eq!(at(&app), "A.swift");

    press(&mut app, KeyCode::Char('/'));
    for c in "Total".chars() {
        press(&mut app, KeyCode::Char(c));
    }
    press(&mut app, KeyCode::Enter);
    assert_eq!(
        at(&app),
        "C.swift",
        "a capital makes the search case-sensitive"
    );
    assert_eq!(app.message.as_deref(), Some("match 1 of 1 for “Total”"));

    press(&mut app, KeyCode::Char('/'));
    for c in "missing".chars() {
        press(&mut app, KeyCode::Char(c));
    }
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.message.as_deref(), Some("no match for “missing”"));
}
