mod common;

use common::git;
use declutter::review::{ChangeStatus, FileChange, FileReview, Layers};
use declutter::store::ReviewStore;
use declutter::tui::App;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use tempfile::TempDir;

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

#[test]
fn marks_survive_reopening_but_lapse_when_the_change_moves_on() {
    let dir = TempDir::new().expect("temp dir");
    let path = dir.path().join("reviewed.tsv");
    let first = review("cart.py", "a = 1\n", "a = 2\n");
    ReviewStore::at(path.clone())
        .set_reviewed(&first, true)
        .expect("save");

    let reopened = ReviewStore::at(path);
    assert!(reopened.is_reviewed(&review("cart.py", "a = 1\n", "a = 2\n")));
    assert!(!reopened.is_reviewed(&review("cart.py", "a = 1\n", "a = 3\n")));
    assert!(!reopened.is_reviewed(&review("other.py", "a = 1\n", "a = 2\n")));
}

#[test]
fn the_store_lives_in_the_git_directory() {
    let dir = TempDir::new().expect("temp dir");
    git(dir.path(), &["init", "-q"]);
    let file = review("cart.py", "a = 1\n", "a = 2\n");

    ReviewStore::open(dir.path())
        .set_reviewed(&file, true)
        .expect("save");

    assert!(dir.path().join(".git/declutter/reviewed.tsv").is_file());
    assert!(ReviewStore::open(dir.path()).is_reviewed(&file));
}

#[test]
fn r_marks_the_file_and_moves_to_the_next_unreviewed_one() {
    let files = vec![
        review("a.py", "a = 1\n", "a = 2\n"),
        review("b.py", "b = 1\n", "b = 2\n"),
        review("c.py", "c = 1\n", "c = 2\n"),
    ];
    let mut app = App::new(files, Layers::default());
    let r = KeyEvent::from(KeyCode::Char('r'));
    let current = |app: &App| app.current().map(|f| f.path.clone());

    app.handle_key(KeyEvent::from(KeyCode::Down));
    app.handle_key(r);
    assert_eq!(current(&app).as_deref(), Some("c.py"));
    assert_eq!(app.review_progress(), (1, 3));

    app.handle_key(r);
    assert_eq!(
        current(&app).as_deref(),
        Some("c.py"),
        "nothing unreviewed after c.py"
    );
    assert_eq!(app.review_progress(), (2, 3));

    app.handle_key(r);
    assert_eq!(
        app.review_progress(),
        (1, 3),
        "pressing r again clears the mark"
    );
}
