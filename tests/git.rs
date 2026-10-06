mod common;

use std::fs;
use std::path::Path;
use std::process::Command;

use common::{git, isolated, write};

use declutter::git::{RangeSpec, load};
use declutter::project::LayerMode;
use declutter::review::{ChangeStatus, FileReview};
use tempfile::TempDir;

/// A repository with one commit, then a mix of comment-only and code changes on disk.
fn repo() -> TempDir {
    let dir = TempDir::new().expect("temp dir");
    let root = dir.path();
    git(root, &["init", "-q", "-b", "main"]);
    write(root, "notes.py", "# first draft\nvalue = 1\n");
    write(root, "logic.ts", "export const limit = 10;\n");
    write(root, "old.py", "gone = True\n");
    git(root, &["add", "."]);
    git(root, &["commit", "-q", "-m", "initial"]);

    write(root, "notes.py", "# second draft, much longer\nvalue = 1\n");
    write(root, "logic.ts", "export const limit = 20; // raised\n");
    fs::remove_file(root.join("old.py")).expect("remove fixture");
    write(root, "Fresh.swift", "// New file.\nlet ready = true\n");
    dir
}

fn by_path<'a>(files: &'a [FileReview], path: &str) -> &'a FileReview {
    files
        .iter()
        .find(|file| file.path == path)
        .expect("file is listed")
}

fn reviews(root: &Path, spec: RangeSpec) -> Vec<FileReview> {
    load(root, &spec)
        .expect("load changes")
        .into_iter()
        .map(FileReview::new)
        .collect()
}

#[test]
fn working_tree_includes_modified_deleted_and_untracked_files() {
    let dir = repo();
    let files = reviews(dir.path(), RangeSpec::parse(None, false).expect("spec"));

    let listed: Vec<_> = files.iter().map(|f| (f.path.as_str(), f.status)).collect();
    assert_eq!(
        listed,
        [
            ("Fresh.swift", ChangeStatus::Added),
            ("logic.ts", ChangeStatus::Modified),
            ("notes.py", ChangeStatus::Modified),
            ("old.py", ChangeStatus::Deleted),
        ]
    );
    assert!(
        by_path(&files, "notes.py")
            .view(LayerMode::Hidden)
            .hunks
            .is_empty()
    );
    assert_eq!(
        by_path(&files, "logic.ts")
            .view(LayerMode::Hidden)
            .hunks
            .len(),
        1
    );
}

#[test]
fn staged_compares_head_with_the_index_only() {
    let dir = repo();
    git(dir.path(), &["add", "logic.ts"]);
    let files = reviews(dir.path(), RangeSpec::parse(None, true).expect("spec"));

    let listed: Vec<_> = files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(listed, ["logic.ts"]);
}

#[test]
fn revision_range_compares_two_commits_and_follows_renames() {
    let dir = repo();
    let root = dir.path();
    // An unedited move, so git's similarity check sees a rename rather than a delete and an add.
    git(root, &["checkout", "--", "logic.ts"]);
    git(root, &["mv", "logic.ts", "rules.ts"]);
    git(root, &["add", "-A"]);
    git(root, &["commit", "-q", "-m", "second"]);
    let files = reviews(
        root,
        RangeSpec::parse(Some("HEAD~1..HEAD"), false).expect("spec"),
    );

    let rules = by_path(&files, "rules.ts");
    assert_eq!(rules.status, ChangeStatus::Renamed);
    assert_eq!(rules.old_path.as_deref(), Some("logic.ts"));
}

#[test]
fn print_mode_shows_code_changes_and_says_what_is_hidden() {
    let dir = repo();
    let mut command = Command::new(env!("CARGO_BIN_EXE_declutter"));
    command.arg("--print").current_dir(dir.path());
    let output = isolated(command);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(stdout.contains("+export const limit = 20;\n"), "{stdout}");
    assert!(!stdout.contains("raised"), "{stdout}");
    assert!(
        stdout.contains("M notes.py [Python]\n  (comment-only changes: 1 hunk hidden)"),
        "{stdout}"
    );
    assert!(stdout.contains("1 comment-only hunk hidden"), "{stdout}");
}

#[test]
fn viewer_refuses_to_start_without_a_terminal() {
    let dir = repo();
    let mut command = Command::new(env!("CARGO_BIN_EXE_declutter"));
    command.current_dir(dir.path());
    let output = isolated(command);

    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("use --print"));
}
