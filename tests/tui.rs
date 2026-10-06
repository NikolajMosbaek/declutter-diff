use declutter::review::{ChangeStatus, FileChange, FileReview, Layers};
use declutter::tui::{App, draw};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyCode, KeyEvent};

fn app() -> App {
    let file = FileReview::new(FileChange {
        path: "rate.ts".to_string(),
        old_path: None,
        status: ChangeStatus::Modified,
        old: Some("// Percent.\nconst rate = 5;\n".to_string()),
        new: Some("// Fraction.\nconst rate = 0.05;\n".to_string()),
        binary: false,
    });
    App::new(vec![file], Layers::default())
}

fn screen(app: &mut App) -> String {
    let mut terminal = Terminal::new(TestBackend::new(110, 16)).expect("test terminal");
    terminal.draw(|frame| draw(frame, app)).expect("draw");
    let buffer = terminal.backend().buffer();
    let width = buffer.area.width as usize;
    buffer
        .content()
        .chunks(width)
        .map(|row| {
            row.iter()
                .map(|cell| cell.symbol())
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn c_hides_and_shows_comments_and_shift_c_shows_them_alone() {
    let mut app = app();
    let press = |app: &mut App, c| app.handle_key(KeyEvent::from(KeyCode::Char(c)));

    let hidden = screen(&mut app);
    assert!(hidden.contains("comments: hidden"), "{hidden}");
    assert!(hidden.contains("+ const rate = 0.05;"), "{hidden}");
    assert!(!hidden.contains("Fraction"), "{hidden}");

    press(&mut app, 'c');
    let shown = screen(&mut app);
    assert!(shown.contains("all layers shown"), "{shown}");
    assert!(
        shown.contains("+ // Fraction.") && shown.contains("+ const rate = 0.05;"),
        "{shown}"
    );

    press(&mut app, 'c');
    assert!(screen(&mut app).contains("comments: hidden"));

    press(&mut app, 'C');
    let only = screen(&mut app);
    assert!(only.contains("comments: only"), "{only}");
    assert!(only.contains("+ // Fraction."), "{only}");
    assert!(!only.contains("0.05"), "{only}");

    press(&mut app, 'C');
    assert!(
        screen(&mut app).contains("comments: hidden"),
        "pressing C again goes back to how comments were"
    );
}

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

#[test]
fn arrow_keys_move_through_the_file_list_until_the_diff_has_focus() {
    let long_old: String = (0..60).map(|i| format!("let v{i} = {i}\n")).collect();
    let long_new = long_old
        .replace("= 0\n", "= 100\n")
        .replace("= 59\n", "= 159\n");
    let mut app = App::new(
        vec![
            change("A.swift", "let a = 1\n", "let a = 2\n"),
            change("B.swift", &long_old, &long_new),
        ],
        Layers::default(),
    );
    screen(&mut app);

    app.handle_key(KeyEvent::from(KeyCode::Down));
    assert_eq!(app.selected, 1);
    assert!(screen(&mut app).contains(" B.swift [Swift] "));

    app.handle_key(KeyEvent::from(KeyCode::Right));
    let first_change = app.cursor;
    app.handle_key(KeyEvent::from(KeyCode::Down));
    assert_eq!(app.selected, 1);
    assert_eq!(app.cursor, first_change + 1);

    app.handle_key(KeyEvent::from(KeyCode::Left));
    app.handle_key(KeyEvent::from(KeyCode::Up));
    assert_eq!(app.selected, 0);
}
