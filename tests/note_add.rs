mod common;

use std::path::Path;
use std::process::Command;

use common::{git, isolated, write};
use declutter::review::{ChangeStatus, FileChange, FileReview};
use declutter::store::{Note, NoteSide};
use serde_json::{Value, json};
use tempfile::TempDir;

fn numbered(lines: std::ops::RangeInclusive<usize>, changed: Option<usize>) -> String {
    lines
        .map(|n| match changed {
            Some(c) if c == n => format!("const v{n} = {n} * 2;\n"),
            _ => format!("const v{n} = {n};\n"),
        })
        .collect()
}

fn change() -> Vec<FileReview> {
    vec![FileReview::new(FileChange {
        path: "src/rate.ts".to_string(),
        old_path: None,
        status: ChangeStatus::Modified,
        old: Some(numbered(1..=30, None)),
        new: Some(numbered(1..=30, Some(15))),
        binary: false,
    })]
}

#[test]
fn a_note_goes_on_a_line_of_the_diff_and_takes_its_code() {
    let note = Note::on_line(&change(), "src/rate.ts", NoteSide::New, 15, "why double?")
        .expect("in the diff");
    assert_eq!(note.code, "const v15 = 15 * 2;");
    assert!(note.draft);

    let context = Note::on_line(&change(), "src/rate.ts", NoteSide::New, 12, "context")
        .expect("context lines are in the diff too");
    assert_eq!(context.code, "const v12 = 12;");

    let removed = Note::on_line(&change(), "src/rate.ts", NoteSide::Old, 15, "was fine")
        .expect("the removed line");
    assert_eq!(removed.code, "const v15 = 15;");
}

#[test]
fn a_line_outside_the_diff_is_refused_with_the_lines_that_are_in_it() {
    let error = Note::on_line(&change(), "src/rate.ts", NoteSide::New, 25, "x")
        .expect_err("line 25 is far from the change");
    assert_eq!(
        error.to_string(),
        "`src/rate.ts:25` is not in the diff; the lines in it are 12–18"
    );

    let error = Note::on_line(&change(), "src/rate.ts", NoteSide::Old, 14, "x")
        .expect_err("line 14 was not removed");
    assert_eq!(
        error.to_string(),
        "removed line 14 of `src/rate.ts` is not in the diff; the removed lines in it are 15"
    );

    let error =
        Note::on_line(&change(), "src/other.ts", NoteSide::New, 1, "x").expect_err("not changed");
    assert_eq!(
        error.to_string(),
        "`src/other.ts` is not part of this change"
    );
}

/// A repository on branch `feature`, which changes line 15 of src/rate.ts.
fn repo() -> TempDir {
    let dir = TempDir::new().expect("temp dir");
    let root = dir.path();
    git(root, &["init", "-q", "-b", "main"]);
    write(root, "src/rate.ts", &numbered(1..=30, None));
    git(root, &["add", "."]);
    git(root, &["commit", "-q", "-m", "base"]);
    git(root, &["checkout", "-q", "-b", "feature"]);
    write(root, "src/rate.ts", &numbered(1..=30, Some(15)));
    git(root, &["commit", "-q", "-am", "double"]);
    dir
}

fn declutter(dir: &Path, args: &[&str], stdin: Option<&str>) -> (bool, String, String) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_declutter"));
    command.args(args).current_dir(dir);
    let output = match stdin {
        None => isolated(command),
        Some(text) => {
            let file = dir.join(".git/stdin.json");
            std::fs::write(&file, text).expect("stdin file");
            command.stdin(std::fs::File::open(&file).expect("open"));
            isolated(command)
        }
    };
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

fn listed(dir: &Path, target: &[&str]) -> Value {
    let mut args = vec!["notes"];
    args.extend(target);
    args.push("--json");
    let (ok, out, err) = declutter(dir, &args, None);
    assert!(ok, "{err}");
    serde_json::from_str(&out).expect("json")
}

#[test]
fn notes_add_puts_draft_notes_on_the_review_and_lists_them_as_json() {
    let dir = repo();

    let (ok, out, err) = declutter(
        dir.path(),
        &[
            "notes",
            "add",
            "feature",
            "--path",
            "src/rate.ts",
            "--line",
            "15",
            "--text",
            "Why double?",
        ],
        None,
    );
    assert!(ok, "{err}");
    assert_eq!(out, "Added 1 draft note to main...feature.\n");

    let (ok, _, err) = declutter(
        dir.path(),
        &["notes", "add", "feature", "--json", "-"],
        Some(
            r#"[{"path": "src/rate.ts", "line": 15, "text": "And untested."},
                {"path": "src/rate.ts", "line": 15, "side": "old", "text": "Was fine."},
                {"text": "No test covers the new rate."}]"#,
        ),
    );
    assert!(ok, "{err}");

    let review = listed(dir.path(), &["feature"]);
    assert_eq!(review["posted"], json!([]));
    let notes = review["notes"].as_array().expect("notes");
    assert_eq!(notes.len(), 3, "{review:#}");
    assert_eq!(notes[0]["text"], "No test covers the new rate.");
    assert_eq!(notes[0]["path"], "");
    assert_eq!(
        (&notes[1]["side"], &notes[1]["code"], &notes[1]["draft"]),
        (&json!("new"), &json!("const v15 = 15 * 2;"), &json!(true))
    );
    assert_eq!(notes[1]["text"], "Why double?\n\nAnd untested.");
    assert_eq!(notes[2]["side"], "old");

    assert_eq!(
        listed(dir.path(), &[])["notes"].as_array().map(Vec::len),
        Some(3),
        "without a target, every review's notes"
    );
    assert_eq!(listed(dir.path(), &["HEAD~1..HEAD"])["notes"], json!([]));
}

#[test]
fn notes_add_refuses_every_bad_entry_and_adds_none() {
    let dir = repo();

    let (ok, _, err) = declutter(
        dir.path(),
        &["notes", "add", "--json", "-"],
        Some(
            r#"[{"path": "src/rate.ts", "line": 15, "text": "fine"},
                {"path": "src/rate.ts", "line": 29, "text": "too far"},
                {"path": "README.md", "line": 1, "text": "untouched"}]"#,
        ),
    );
    assert!(!ok);
    assert!(
        err.contains("note 2: `src/rate.ts:29` is not in the diff; the lines in it are 12–18"),
        "{err}"
    );
    assert!(
        err.contains("note 3: `README.md` is not part of this change"),
        "{err}"
    );
    assert!(err.contains("no notes were added"), "{err}");
    assert_eq!(listed(dir.path(), &[])["notes"], json!([]));
}
