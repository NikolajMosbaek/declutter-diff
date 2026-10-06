mod common;

use std::sync::Mutex;

use common::git;
use declutter::review::{ChangeStatus, FileChange, FileReview, Layers};
use declutter::store::{Note, NoteSide, NoteStore};
use declutter::tui::{App, draw};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use tempfile::TempDir;

fn note(path: &str, side: NoteSide, line: usize, text: &str) -> Note {
    Note {
        path: path.into(),
        side,
        line,
        code: "let a = 2".into(),
        text: text.into(),
    }
}

#[test]
fn notes_survive_reopening_and_one_line_holds_one_note() {
    let dir = TempDir::new().expect("temp dir");
    let path = dir.path().join("notes.json");
    let mut store = NoteStore::at(path.clone());
    store
        .set(note("a.swift", NoteSide::New, 3, "first"))
        .expect("save");
    store
        .set(note("a.swift", NoteSide::New, 3, "second"))
        .expect("save");
    store
        .set(note("a.swift", NoteSide::Old, 3, "on the removed line"))
        .expect("save");

    let reopened = NoteStore::at(path.clone());
    assert_eq!(reopened.notes().len(), 2);
    assert_eq!(
        reopened
            .find("a.swift", NoteSide::New, 3)
            .map(|n| n.text.as_str()),
        Some("second")
    );

    let mut store = reopened;
    store
        .set(note("a.swift", NoteSide::New, 3, "  "))
        .expect("save");
    assert_eq!(
        NoteStore::at(path).notes().len(),
        1,
        "an empty note removes the note"
    );
}

#[test]
fn notes_become_one_prompt() {
    let mut store = NoteStore::in_memory();
    store
        .set(note("b.swift", NoteSide::Old, 9, "why was this removed?"))
        .expect("save");
    store
        .set(note("a.swift", NoteSide::New, 3, "use a named constant"))
        .expect("save");

    assert_eq!(
        store.prompt(),
        "Please address these review comments. Line numbers refer to the version under review.\n\
         \n1. `a.swift:3`: use a named constant\n   ```\n   let a = 2\n   ```\n\
         \n2. `b.swift` (removed line 9): why was this removed?\n   ```\n   let a = 2\n   ```\n"
    );
}

static COPIED: Mutex<String> = Mutex::new(String::new());

fn fake_clipboard(text: &str) -> bool {
    *COPIED.lock().expect("lock") = text.to_string();
    true
}

#[test]
fn m_notes_the_cursor_line_and_e_exports_every_note() {
    let file = FileReview::new(FileChange {
        path: "rate.ts".to_string(),
        old_path: None,
        status: ChangeStatus::Modified,
        old: Some("const rate = 5;\n".to_string()),
        new: Some("const rate = 0.05;\n".to_string()),
        binary: false,
    });
    let mut app = App::new(vec![file], Layers::default());
    app.clipboard = fake_clipboard;
    let key = |code| KeyEvent::from(code);

    app.handle_key(key(KeyCode::Right));
    // Line 0 is the hunk header, 1 the removed line, 2 the added one.
    app.handle_key(key(KeyCode::Down));
    app.handle_key(key(KeyCode::Down));
    app.handle_key(key(KeyCode::Char('m')));
    for c in "use a fraction".chars() {
        app.handle_key(key(KeyCode::Char(c)));
    }
    app.handle_key(key(KeyCode::Enter));

    assert_eq!(
        app.notes.notes(),
        [Note {
            path: "rate.ts".into(),
            side: NoteSide::New,
            line: 1,
            code: "const rate = 0.05;".into(),
            text: "use a fraction".into(),
        }]
    );
    let mut terminal = Terminal::new(TestBackend::new(110, 14)).expect("terminal");
    terminal.draw(|frame| draw(frame, &mut app)).expect("draw");
    let screen: String = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|c| c.symbol())
        .collect();
    assert!(screen.contains("✎ use a fraction"), "{screen}");

    app.handle_key(key(KeyCode::Up));
    app.handle_key(key(KeyCode::Char('m')));
    app.handle_key(key(KeyCode::Char('?')));
    app.handle_key(key(KeyCode::Enter));
    let removed = app
        .notes
        .find("rate.ts", NoteSide::Old, 1)
        .expect("note on the removed line");
    assert_eq!(
        (removed.code.as_str(), removed.text.as_str()),
        ("const rate = 5;", "?")
    );

    app.handle_key(key(KeyCode::Char('E')));
    assert!(
        COPIED
            .lock()
            .expect("lock")
            .contains("`rate.ts:1`: use a fraction")
    );
}

#[test]
fn the_notes_command_prints_and_clears_the_prompt() {
    let dir = TempDir::new().expect("temp dir");
    git(dir.path(), &["init", "-q"]);
    NoteStore::open(dir.path())
        .set(note("a.swift", NoteSide::New, 3, "rename"))
        .expect("save");
    let run = |args: &[&str]| {
        let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_declutter"));
        command.args(args).current_dir(dir.path());
        let output = common::isolated(command);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).into_owned()
    };

    assert!(run(&["notes"]).contains("1. `a.swift:3`: rename"));
    assert_eq!(run(&["notes", "--clear"]), "Cleared 1 note.\n");
    assert!(run(&["notes"]).starts_with("No review notes."));
}
