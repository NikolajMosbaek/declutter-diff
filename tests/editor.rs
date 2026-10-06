use std::path::{Path, PathBuf};

use declutter::editor::{EditorCommand, editor_command};
use declutter::review::{ChangeStatus, FileChange, FileReview, Layers};
use declutter::tui::App;
use ratatui::crossterm::event::{KeyCode, KeyEvent};

fn command(editor: Option<&str>) -> EditorCommand {
    editor_command(editor, Path::new("/repo/Cart.swift"), 42)
}

fn args(editor: &str) -> Vec<String> {
    command(Some(editor)).args
}

#[test]
fn each_editor_family_gets_its_own_line_syntax() {
    assert_eq!(args("code --wait"), ["--wait", "-g", "/repo/Cart.swift:42"]);
    assert_eq!(args("/usr/local/bin/subl"), ["/repo/Cart.swift:42"]);
    assert_eq!(args("nvim"), ["+42", "/repo/Cart.swift"]);
    assert_eq!(args("xed"), ["--line", "42", "/repo/Cart.swift"]);
    assert_eq!(args("unknown-editor"), ["/repo/Cart.swift"]);
}

#[test]
fn terminal_editors_take_over_the_screen_and_gui_ones_do_not() {
    assert!(command(Some("vim")).in_terminal);
    assert!(command(Some("hx")).in_terminal);
    assert!(!command(Some("code")).in_terminal);
    let opener = command(None);
    assert!(!opener.in_terminal);
    assert_eq!(opener.args, ["/repo/Cart.swift"]);
}

fn change(path: &str, status: ChangeStatus, old: Option<&str>, new: Option<&str>) -> FileReview {
    FileReview::new(FileChange {
        path: path.into(),
        old_path: None,
        status,
        old: old.map(str::to_string),
        new: new.map(str::to_string),
        binary: false,
    })
}

#[test]
fn o_opens_the_cursor_line_and_a_removed_line_opens_where_the_code_continues() {
    let file = change(
        "src/rate.ts",
        ChangeStatus::Modified,
        Some("const a = 1;\nconst rate = 5;\nconst b = 2;\n"),
        Some("const z = 0;\nconst a = 1;\nconst b = 2;\n"),
    );
    let mut app = App::new(vec![file], Layers::default());
    app.root = PathBuf::from("/repo");
    let key = |code| KeyEvent::from(code);

    app.handle_key(key(KeyCode::Char('o')));
    assert_eq!(
        app.open_request.take(),
        Some((PathBuf::from("/repo/src/rate.ts"), 1))
    );

    // Header, added `z`, context `a`, then the removed `rate` (old line 2): it opens at
    // `b`, the next line still in the file (new line 3).
    app.handle_key(key(KeyCode::Right));
    for _ in 0..3 {
        app.handle_key(key(KeyCode::Down));
    }
    app.handle_key(key(KeyCode::Char('o')));
    assert_eq!(
        app.open_request.take(),
        Some((PathBuf::from("/repo/src/rate.ts"), 3))
    );
}

#[test]
fn a_deleted_file_is_not_opened() {
    let file = change(
        "gone.ts",
        ChangeStatus::Deleted,
        Some("const a = 1;\n"),
        None,
    );
    let mut app = App::new(vec![file], Layers::default());

    app.handle_key(KeyEvent::from(KeyCode::Char('o')));

    assert_eq!(app.open_request, None);
    assert!(
        app.message
            .as_deref()
            .is_some_and(|m| m.contains("deleted"))
    );
}
