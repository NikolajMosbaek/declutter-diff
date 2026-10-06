use declutter::project::CommentMode;
use declutter::review::{ChangeStatus, FileChange, FileReview};
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
    App::new(vec![file], CommentMode::Hidden)
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
fn pressing_c_cycles_through_the_comment_modes() {
    let mut app = app();

    let hidden = screen(&mut app);
    assert!(hidden.contains("comments: hidden"), "{hidden}");
    assert!(hidden.contains("+ const rate = 0.05;"), "{hidden}");
    assert!(!hidden.contains("Fraction"), "{hidden}");

    app.handle_key(KeyEvent::from(KeyCode::Char('c')));
    let only = screen(&mut app);
    assert!(only.contains("comments: only"), "{only}");
    assert!(only.contains("+ // Fraction."), "{only}");
    assert!(!only.contains("0.05"), "{only}");

    app.handle_key(KeyEvent::from(KeyCode::Char('c')));
    let shown = screen(&mut app);
    assert!(shown.contains("comments: shown"), "{shown}");
    assert!(
        shown.contains("+ // Fraction.") && shown.contains("+ const rate = 0.05;"),
        "{shown}"
    );
}
