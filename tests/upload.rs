use std::cell::RefCell;

use anyhow::{Result, bail};
use declutter::pr::{PullRequest, Repo};
use declutter::store::{Note, NoteSide, NoteStore};
use declutter::upload::{Poster, azure_thread, github_comment, offer_upload};
use serde_json::json;
use tempfile::TempDir;

fn azure_pr() -> PullRequest {
    PullRequest {
        repo: Repo::AzureDevOps {
            org: "Contoso".into(),
            project: "Mobile Apps".into(),
            name: "ios".into(),
        },
        number: 42,
    }
}

fn github_pr() -> PullRequest {
    PullRequest {
        repo: Repo::GitHub {
            owner: "acme".into(),
            name: "shop".into(),
        },
        number: 7,
    }
}

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
fn an_azure_thread_is_anchored_with_offsets_on_the_right_side() {
    let (url, body) = azure_thread(
        &azure_pr(),
        &note("src/Cart.swift", NoteSide::New, 12, " rename "),
    );

    assert_eq!(
        url,
        "https://dev.azure.com/Contoso/Mobile%20Apps/_apis/git/repositories/ios/pullRequests/42/threads?api-version=7.1"
    );
    assert_eq!(
        body,
        json!({
            "comments": [{ "parentCommentId": 0, "content": "rename", "commentType": 1 }],
            "status": "active",
            "threadContext": {
                "filePath": "/src/Cart.swift",
                "rightFileStart": { "line": 12, "offset": 1 },
                "rightFileEnd": { "line": 12, "offset": 2 },
            },
        })
    );
}

#[test]
fn a_note_on_a_removed_line_goes_on_the_left_side() {
    let (_, body) = azure_thread(&azure_pr(), &note("a.swift", NoteSide::Old, 3, "why?"));
    let context = &body["threadContext"];

    assert_eq!(context["leftFileStart"], json!({ "line": 3, "offset": 1 }));
    assert!(context.get("rightFileStart").is_none());

    let args = github_comment(
        &github_pr(),
        &note("a.swift", NoteSide::Old, 3, "why?"),
        "abc123",
        false,
    );
    assert!(args.contains(&"side=LEFT".to_string()), "{args:?}");
}

#[test]
fn a_github_comment_targets_the_head_commit_and_falls_back_to_the_file() {
    let on_line = github_comment(
        &github_pr(),
        &note("a.swift", NoteSide::New, 9, "nit"),
        "abc123",
        false,
    );
    assert_eq!(
        on_line,
        [
            "api",
            "--method",
            "POST",
            "repos/acme/shop/pulls/7/comments",
            "-f",
            "commit_id=abc123",
            "-f",
            "path=a.swift",
            "-F",
            "line=9",
            "-f",
            "side=RIGHT",
            "-f",
            "body=nit",
        ]
    );

    let on_file = github_comment(
        &github_pr(),
        &note("a.swift", NoteSide::New, 9, "nit"),
        "abc123",
        true,
    );
    assert!(on_file.contains(&"subject_type=file".to_string()));
    assert!(
        on_file.contains(&"body=Line 9: nit".to_string()),
        "{on_file:?}"
    );
}

/// Records posts, failing the note whose text is "fail".
#[derive(Default)]
struct Recorder {
    posted: RefCell<Vec<String>>,
}

impl Poster for Recorder {
    fn post(&self, _: &PullRequest, note: &Note) -> Result<()> {
        if note.text == "fail" {
            bail!("403 forbidden");
        }
        self.posted.borrow_mut().push(note.text.clone());
        Ok(())
    }
}

fn store_with(dir: &TempDir, review: &str, texts: &[&str]) -> NoteStore {
    let mut store = NoteStore::at(dir.path().join("notes.json")).scoped(review);
    for (line, text) in texts.iter().enumerate() {
        store
            .set(note("a.swift", NoteSide::New, line + 1, text))
            .expect("save");
    }
    store
}

fn answer(store: &mut NoteStore, poster: &Recorder, reply: &str) -> String {
    let mut out = Vec::new();
    offer_upload(store, &azure_pr(), poster, &mut reply.as_bytes(), &mut out).expect("offer");
    String::from_utf8(out).expect("utf8")
}

#[test]
fn nothing_is_posted_without_an_explicit_yes() {
    let dir = TempDir::new().expect("temp dir");
    let mut store = store_with(&dir, "PR 42", &["rename", "split"]);
    let poster = Recorder::default();

    for reply in ["\n", "n\n", "nope\n", ""] {
        let out = answer(&mut store, &poster, reply);
        assert!(out.contains("Post 2 comments to PR 42? [y/N]"), "{out}");
        assert!(
            out.contains("`a.swift:1`  rename"),
            "the notes are listed first: {out}"
        );
        assert!(out.contains("Not posted"), "{out}");
    }
    assert!(poster.posted.borrow().is_empty());
    assert_eq!(store.notes().len(), 2);
}

#[test]
fn yes_posts_every_note_and_keeps_only_the_ones_that_failed() {
    let dir = TempDir::new().expect("temp dir");
    let mut store = store_with(&dir, "PR 42", &["rename", "fail", "split"]);
    let poster = Recorder::default();

    let out = answer(&mut store, &poster, "y\n");

    assert_eq!(poster.posted.borrow().as_slice(), ["rename", "split"]);
    assert!(
        out.contains("could not post `a.swift:2`: 403 forbidden"),
        "{out}"
    );
    assert!(
        out.contains("Posted 2 of 3 comments to PR 42; the 1 that failed are kept."),
        "{out}"
    );
    let left: Vec<String> = NoteStore::at(dir.path().join("notes.json"))
        .notes()
        .iter()
        .map(|n| n.text.clone())
        .collect();
    assert_eq!(left, ["fail"], "posted notes leave the store on disk too");
}

#[test]
fn notes_from_another_review_are_not_offered() {
    let dir = TempDir::new().expect("temp dir");
    drop(store_with(
        &dir,
        "origin/main...feature",
        &["from another review"],
    ));
    let mut store = store_with(&dir, "PR 42", &[]);
    let poster = Recorder::default();

    let out = answer(&mut store, &poster, "y\n");

    assert!(
        out.is_empty(),
        "no notes in this review, so no question: {out}"
    );
    assert_eq!(
        NoteStore::at(dir.path().join("notes.json")).notes().len(),
        1
    );
}
