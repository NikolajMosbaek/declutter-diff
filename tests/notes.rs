mod common;

use std::sync::Mutex;

use common::git;
use declutter::review::{ChangeStatus, FileChange, FileReview, Layers};
use declutter::store::{Added, Link, Note, NoteSide, NoteStore};
use declutter::tui::{App, draw};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tempfile::TempDir;

fn note(path: &str, side: NoteSide, line: usize, text: &str) -> Note {
    Note {
        path: path.into(),
        side,
        line,
        code: "let a = 2".into(),
        text: text.into(),
        review: None,
        draft: false,
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
    // Line 0 is the hunk header, 1 the removed line, 2 the added one; the cursor
    // starts on the first change.
    assert_eq!(app.cursor, 1);
    app.handle_key(key(KeyCode::Down));
    app.handle_key(key(KeyCode::Char('m')));
    for c in "use a fraction".chars() {
        app.handle_key(key(KeyCode::Char(c)));
    }
    app.handle_key(key(KeyCode::Enter));

    assert_eq!(
        app.notes.notes(),
        [&Note {
            path: "rate.ts".into(),
            side: NoteSide::New,
            line: 1,
            code: "const rate = 0.05;".into(),
            text: "use a fraction".into(),
            review: None,
            draft: false,
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

fn draft(path: &str, line: usize, text: &str) -> Note {
    Note {
        draft: true,
        ..note(path, NoteSide::New, line, text)
    }
}

#[test]
fn adding_to_a_noted_line_appends_and_makes_it_a_draft_again() {
    let dir = TempDir::new().expect("temp dir");
    let path = dir.path().join("notes.json");
    let mut store = NoteStore::at(path.clone()).scoped("PR 7");
    store
        .set(note("a.swift", NoteSide::New, 3, "mine"))
        .expect("save");

    assert_eq!(
        store
            .add(draft("a.swift", 3, "from the agent"))
            .expect("add"),
        Added::Appended
    );
    assert_eq!(
        store
            .add(draft("a.swift", 3, "from the agent"))
            .expect("add"),
        Added::Duplicate,
        "adding the same text again changes nothing"
    );
    assert_eq!(
        store.add(draft("a.swift", 4, "elsewhere")).expect("add"),
        Added::New
    );

    let reopened = NoteStore::at(path).scoped("PR 7");
    let noted = reopened.find("a.swift", NoteSide::New, 3).expect("note");
    assert_eq!(noted.text, "mine\n\nfrom the agent");
    assert!(noted.draft, "unread text was added");
    assert_eq!(reopened.notes().len(), 2);
}

#[test]
fn a_pull_request_note_has_no_line_and_comes_first_in_the_prompt() {
    let mut store = NoteStore::in_memory();
    store
        .set(note("a.swift", NoteSide::New, 3, "use a constant"))
        .expect("save");
    store
        .add(Note::on_pull_request(
            "Nothing tests the new branch.\nAdd a case for it.",
        ))
        .expect("add");

    let general = store.general().expect("the PR note");
    assert!(general.is_general() && general.draft);
    assert_eq!(
        store.prompt(),
        "Please address these review comments. Line numbers refer to the version under review.\n\
         \n1. On the change as a whole: Nothing tests the new branch.\n   Add a case for it.\n\
         \n2. `a.swift:3`: use a constant\n   ```\n   let a = 2\n   ```\n"
    );
}

#[test]
fn posted_notes_are_logged_with_their_link_and_leave_the_store() {
    let dir = TempDir::new().expect("temp dir");
    let path = dir.path().join("notes.json");
    let mut store = NoteStore::at(path.clone()).scoped("PR 7");
    store
        .set(note("a.swift", NoteSide::New, 3, "rename"))
        .expect("save");
    store
        .set(note("a.swift", NoteSide::New, 4, "split"))
        .expect("save");
    let posted = store.notes()[0].clone();

    store
        .record_posted(&[(
            posted.clone(),
            Link {
                id: "11".into(),
                url: "https://example.com/11".into(),
            },
        )])
        .expect("record");
    NoteStore::at(path.clone())
        .scoped("PR 8")
        .record_posted(&[(note("b.swift", NoteSide::New, 1, "other"), Link::default())])
        .expect("record");

    let reopened = NoteStore::at(path).scoped("PR 7");
    assert_eq!(reopened.notes().len(), 1);
    let log = reopened.posted();
    assert_eq!(log.len(), 1, "only this review's posts: {log:?}");
    assert_eq!(log[0].note.text, "rename");
    assert_eq!(log[0].note.review.as_deref(), Some("PR 7"));
    assert_eq!(log[0].link.url, "https://example.com/11");
    assert!(log[0].posted_at > 1_700_000_000);
}

fn screen(app: &mut App, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
    terminal.draw(|frame| draw(frame, app)).expect("draw");
    let buffer = terminal.backend().buffer();
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
                + "\n"
        })
        .collect()
}

fn rate_app(notes: NoteStore) -> App {
    let file = FileReview::new(FileChange {
        path: "rate.ts".to_string(),
        old_path: None,
        status: ChangeStatus::Modified,
        old: Some("const rate = 5;\n".to_string()),
        new: Some("const rate = 0.05;\n".to_string()),
        binary: false,
    });
    App::with_stores(
        vec![file],
        Layers::default(),
        declutter::store::ReviewStore::in_memory(),
        notes,
    )
}

#[test]
fn a_draft_shows_as_one_until_opened_and_keeps_its_lines() {
    let mut notes = NoteStore::in_memory();
    notes
        .add(draft(
            "rate.ts",
            1,
            "A fraction now, not a percentage.\nEvery caller still passes 5, which is now 500%: \
             multiply by a hundred where it is shown, and divide where it is read.",
        ))
        .expect("add");
    let mut app = rate_app(notes);
    let key = |code| KeyEvent::from(code);

    let shown = screen(&mut app, 150, 16);
    assert!(
        shown.contains("✎ draft · A fraction now, not a percentage."),
        "{shown}"
    );
    assert!(
        shown.contains("│                 Every caller still passes 5"),
        "{shown}"
    );
    assert!(
        shown.contains("│                 shown, and divide where it is read."),
        "wrapped, under the text: {shown}"
    );
    assert!(
        shown.contains("rate.ts (1) ✎1"),
        "the file list marks noted files: {shown}"
    );
    assert!(shown.contains("1 note (1 draft)"), "{shown}");

    app.handle_key(key(KeyCode::Right));
    app.handle_key(key(KeyCode::Down));
    app.handle_key(key(KeyCode::Char('m')));
    app.handle_key(key(KeyCode::Esc));
    assert!(app.notes.notes()[0].draft, "Esc leaves it a draft");

    app.handle_key(key(KeyCode::Char('m')));
    let editing = screen(&mut app, 150, 16);
    assert!(
        editing.contains("Every caller still passes 5"),
        "the editor shows it all: {editing}"
    );
    app.handle_key(key(KeyCode::Enter));
    let opened = app.notes.notes()[0];
    assert!(!opened.draft, "opened and saved, it is the reviewer's");
    assert!(
        opened.text.ends_with("where it is read."),
        "unchanged: {}",
        opened.text
    );
    assert!(screen(&mut app, 150, 16).contains("✎ A fraction now"));
}

#[test]
fn the_note_editor_moves_its_cursor_and_takes_new_lines() {
    let mut app = rate_app(NoteStore::in_memory());
    let key = |code| KeyEvent::from(code);
    app.handle_key(key(KeyCode::Char('m')));
    for c in "why 5?".chars() {
        app.handle_key(key(KeyCode::Char(c)));
    }
    app.handle_key(key(KeyCode::Left));
    app.handle_key(key(KeyCode::Backspace));
    app.handle_key(key(KeyCode::Char('0')));
    app.handle_key(key(KeyCode::Char('.')));
    app.handle_key(key(KeyCode::Char('0')));
    app.handle_key(key(KeyCode::Char('5')));
    app.handle_key(key(KeyCode::End));
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT));
    for c in "It was a percentage.".chars() {
        app.handle_key(key(KeyCode::Char(c)));
    }
    app.handle_key(key(KeyCode::Enter));

    assert_eq!(app.notes.notes()[0].text, "why 0.05?\nIt was a percentage.");
}

#[test]
fn shift_p_notes_the_change_as_a_whole() {
    let mut notes = NoteStore::in_memory();
    notes
        .add(Note::on_pull_request("No test covers the new rate."))
        .expect("add");
    let mut app = rate_app(notes);
    let key = |code| KeyEvent::from(code);

    let shown = screen(&mut app, 150, 16);
    assert!(shown.contains("P: draft on the whole change"), "{shown}");

    app.handle_key(key(KeyCode::Char('P')));
    assert!(
        screen(&mut app, 150, 16).contains("No test covers the new rate."),
        "the draft is shown to be read"
    );
    for c in " Add one.".chars() {
        app.handle_key(key(KeyCode::Char(c)));
    }
    app.handle_key(key(KeyCode::Enter));

    let general = app.notes.general().expect("the note on the whole change");
    assert_eq!(general.text, "No test covers the new rate. Add one.");
    assert!(!general.draft);
}
