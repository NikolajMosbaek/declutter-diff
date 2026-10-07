mod common;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

use common::{git, isolated, write};
use serde_json::Value;
use tempfile::TempDir;

const URL: &str = "https://github.com/acme/shop.git";

/// A stand-in for github.com/acme/shop with PR 8 (`feature`, changing cart.py) published
/// as its merge ref, and a clone that reaches it through `URL`.
fn pr_clone() -> (TempDir, TempDir) {
    let upstream = TempDir::new().expect("temp dir");
    let up = upstream.path();
    git(up, &["init", "-q", "-b", "main"]);
    write(up, "cart.py", "total = 1\n");
    git(up, &["add", "."]);
    git(up, &["commit", "-q", "-m", "base"]);
    git(up, &["checkout", "-q", "-b", "feature"]);
    write(up, "cart.py", "total = 2\n");
    git(up, &["commit", "-q", "-am", "double"]);
    git(up, &["checkout", "-q", "main"]);
    git(up, &["merge", "-q", "--no-ff", "--no-edit", "feature"]);
    let merge = git(up, &["rev-parse", "HEAD"]);
    git(up, &["update-ref", "refs/pull/8/merge", &merge]);
    git(up, &["reset", "-q", "--hard", "HEAD~1"]);

    let clone = TempDir::new().expect("temp dir");
    let instead_of = format!("url.file://{}.insteadOf", up.display());
    git(clone.path(), &["init", "-q", "-b", "main"]);
    git(clone.path(), &["remote", "add", "origin", URL]);
    git(clone.path(), &["config", &instead_of, URL]);
    git(clone.path(), &["fetch", "-q", "origin"]);
    (upstream, clone)
}

/// A `gh` that logs its arguments and input, and answers as GitHub does for a review.
fn fake_gh(bin: &Path, log: &Path) {
    let script = format!(
        "#!/bin/sh\necho \"$@\" >> '{log}'\ncat >> '{log}'\necho >> '{log}'\n\
         echo '{{\"id\": 99, \"html_url\": \"https://github.com/acme/shop/pull/8#pullrequestreview-99\"}}'\n",
        log = log.display()
    );
    let gh = bin.join("gh");
    fs::write(&gh, script).expect("write gh");
    fs::set_permissions(&gh, fs::Permissions::from_mode(0o755)).expect("chmod");
}

fn declutter(dir: &Path, bin: &Path, args: &[&str]) -> (bool, String, String) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_declutter"));
    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    command.args(args).current_dir(dir).env("PATH", path);
    let output = isolated(command);
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn notes_post_holds_drafts_and_posts_opened_notes_as_one_review() {
    let (_upstream, clone) = pr_clone();
    let dir = clone.path();
    let bin = TempDir::new().expect("temp dir");
    let log = bin.path().join("gh.log");
    fake_gh(bin.path(), &log);

    let (ok, _, err) = declutter(
        dir,
        bin.path(),
        &[
            "notes", "add", "pr", "8", "--path", "cart.py", "--line", "1", "--text", "Why 2?",
        ],
    );
    assert!(ok, "{err}");
    let (ok, _, err) = declutter(
        dir,
        bin.path(),
        &["notes", "add", "pr", "8", "--text", "Untested."],
    );
    assert!(ok, "{err}");

    let (ok, out, err) = declutter(dir, bin.path(), &["notes", "post", "pr", "8", "--yes"]);
    assert!(ok, "{err}");
    assert!(out.contains("2 draft notes not opened — kept"), "{out}");
    assert!(!log.exists(), "nothing was posted");

    // Opening the notes in the viewer makes them the reviewer's.
    let notes = dir.join(".git/declutter/notes.json");
    let opened = fs::read_to_string(&notes)
        .expect("notes")
        .replace("\"draft\": true", "\"draft\": false");
    fs::write(&notes, opened).expect("open the notes");

    let (ok, out, err) = declutter(dir, bin.path(), &["notes", "post", "pr", "8", "--yes"]);
    assert!(ok, "{err}");
    assert!(out.contains("Posted 2 comments to PR 8."), "{out}");
    let sent = fs::read_to_string(&log).expect("gh was run");
    assert_eq!(
        sent.matches("repos/acme/shop/pulls/8/reviews").count(),
        1,
        "{sent}"
    );
    let body: Value = serde_json::from_str(sent.lines().nth(1).expect("the review")).expect("json");
    assert_eq!(body["body"], "Untested.");
    assert_eq!(body["comments"][0]["body"], "Why 2?");
    assert_eq!(body["comments"][0]["line"], 1);

    let (ok, out, err) = declutter(dir, bin.path(), &["notes", "pr", "8", "--json"]);
    assert!(ok, "{err}");
    let listing: Value = serde_json::from_str(&out).expect("json");
    assert_eq!(listing["notes"], Value::Array(Vec::new()));
    assert_eq!(
        listing["posted"][1]["url"],
        "https://github.com/acme/shop/pull/8#pullrequestreview-99"
    );
    assert_eq!(listing["posted"][1]["text"], "Why 2?");
}

#[test]
fn notes_post_needs_a_pull_request() {
    let (_upstream, clone) = pr_clone();
    let bin = TempDir::new().expect("temp dir");

    let (ok, _, err) = declutter(clone.path(), bin.path(), &["notes", "post", "origin/main"]);

    assert!(!ok);
    assert!(err.contains("is not a pull request"), "{err}");
}
