use std::cell::{Cell, RefCell};

use anyhow::{Result, bail};
use declutter::pr::{PullRequest, Repo};
use declutter::store::{End, Link, Note, NoteSide, NoteStore};
use declutter::upload::{
    Poster, azure_link, azure_thread, azure_thread_right_only, github_comment, github_review,
    offer_upload, post_azure_thread,
};
use serde_json::{Value, json};
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
        end_side: None,
        end_line: None,
        old_lines: None,
        new_lines: None,
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
    fn post(&self, _: &PullRequest, notes: &[Note]) -> Vec<Result<Link>> {
        notes
            .iter()
            .map(|note| {
                if note.text == "fail" {
                    bail!("403 forbidden");
                }
                self.posted.borrow_mut().push(note.text.clone());
                Ok(Link {
                    id: note.line.to_string(),
                    url: format!("https://example.com/{}", note.line),
                    ..Default::default()
                })
            })
            .collect()
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
    offer_upload(
        store,
        &azure_pr(),
        poster,
        false,
        &mut reply.as_bytes(),
        &mut out,
    )
    .expect("offer");
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

#[test]
fn drafts_are_held_back_until_opened() {
    let dir = TempDir::new().expect("temp dir");
    let mut store = store_with(&dir, "PR 42", &["rename"]);
    store
        .add(Note {
            draft: true,
            ..note("a.swift", NoteSide::New, 5, "from the agent")
        })
        .expect("add");
    store
        .add(Note::on_pull_request("No test covers this."))
        .expect("add");
    let poster = Recorder::default();

    let out = answer(&mut store, &poster, "y\n");

    assert!(out.contains("Post 1 comment to PR 42? [y/N]"), "{out}");
    assert!(
        out.contains("2 draft notes not opened — kept: the change as a whole (P), `a.swift:5`"),
        "{out}"
    );
    assert_eq!(poster.posted.borrow().as_slice(), ["rename"]);
    assert_eq!(store.notes().len(), 2);

    let only_drafts = answer(&mut store, &poster, "y\n");
    assert!(
        !only_drafts.contains("[y/N]"),
        "nothing to ask: {only_drafts}"
    );
    assert!(
        only_drafts.contains("2 draft notes not opened — kept"),
        "{only_drafts}"
    );
}

#[test]
fn yes_up_front_posts_without_asking_and_logs_where_each_note_went() {
    let dir = TempDir::new().expect("temp dir");
    let mut store = store_with(&dir, "PR 42", &["rename", "split"]);
    let poster = Recorder::default();
    let mut out = Vec::new();

    offer_upload(
        &mut store,
        &azure_pr(),
        &poster,
        true,
        &mut "".as_bytes(),
        &mut out,
    )
    .expect("offer");

    let out = String::from_utf8(out).expect("utf8");
    assert!(!out.contains("[y/N]"), "{out}");
    assert!(out.contains("Posted 2 comments to PR 42."), "{out}");
    let log = NoteStore::at(dir.path().join("notes.json"))
        .scoped("PR 42")
        .posted();
    let urls: Vec<&str> = log.iter().map(|p| p.link.url.as_str()).collect();
    assert_eq!(urls, ["https://example.com/1", "https://example.com/2"]);
}

#[test]
fn a_note_on_the_whole_change_is_a_thread_without_a_line() {
    let (_, body) = azure_thread(&azure_pr(), &Note::on_pull_request(" Untested. "));

    assert_eq!(
        body,
        json!({
            "comments": [{ "parentCommentId": 0, "content": "Untested.", "commentType": 1 }],
            "status": "active",
        })
    );
    assert_eq!(
        azure_link(&azure_pr(), "77"),
        "https://dev.azure.com/Contoso/Mobile%20Apps/_git/ios/pullrequest/42?discussionId=77"
    );
}

#[test]
fn github_gets_one_review_with_the_line_notes_as_its_comments() {
    let notes = [
        Note::on_pull_request("Untested.\nAdd a case."),
        note("a.swift", NoteSide::New, 9, "nit"),
        note("a.swift", NoteSide::Old, 3, "why?"),
    ];

    let (args, body) = github_review(&github_pr(), &notes, "abc123");
    let (_, without) = github_review(&github_pr(), &notes[1..], "abc123");
    assert_eq!(
        without["body"], "",
        "a review without a note on the whole change"
    );

    assert_eq!(
        args,
        [
            "api",
            "--method",
            "POST",
            "repos/acme/shop/pulls/7/reviews",
            "--input",
            "-"
        ]
    );
    assert_eq!(
        body,
        json!({
            "commit_id": "abc123",
            "event": "COMMENT",
            "body": "Untested.\nAdd a case.",
            "comments": [
                { "path": "a.swift", "line": 9, "side": "RIGHT", "body": "nit" },
                { "path": "a.swift", "line": 3, "side": "LEFT", "body": "why?" },
            ],
        })
    );
}

type Spans = Option<((usize, usize), (usize, usize))>;

fn block(first: End, last: End, spans: Spans, code: &str, text: &str) -> Note {
    Note {
        end_side: (last.0 != first.0).then_some(last.0),
        end_line: Some(last.1),
        old_lines: spans.map(|(old, _)| old),
        new_lines: spans.map(|(_, new)| new),
        code: code.into(),
        ..note("a.ts", first.0, first.1, text)
    }
}

fn mixed() -> Note {
    block(
        (NoteSide::Old, 3),
        (NoteSide::New, 8),
        Some(((3, 3), (7, 8))),
        "-  return a;\n+  const t = 1;\n+  return t;",
        "x",
    )
}

#[test]
fn an_azure_thread_on_a_block_spans_its_lines() {
    for (side, start, end) in [
        (NoteSide::New, "rightFileStart", "rightFileEnd"),
        (NoteSide::Old, "leftFileStart", "leftFileEnd"),
    ] {
        let mut note = block((side, 11), (side, 13), None, "a\nbb\nccc", "x");
        note.path = "src/Cart.swift".into();
        let (_, body) = azure_thread(&azure_pr(), &note);
        assert_eq!(
            body["threadContext"],
            json!({
                "filePath": "/src/Cart.swift",
                start: { "line": 11, "offset": 1 },
                end: { "line": 13, "offset": 4 },
            })
        );
    }
}

#[test]
fn a_mixed_block_sends_both_sides_and_can_fall_back_to_the_right() {
    let note = mixed();
    let (_, body) = azure_thread(&azure_pr(), &note);
    assert_eq!(
        body["threadContext"],
        json!({
            "filePath": "/a.ts",
            "leftFileStart": { "line": 3, "offset": 1 },
            "leftFileEnd": { "line": 3, "offset": 12 },
            "rightFileStart": { "line": 7, "offset": 1 },
            "rightFileEnd": { "line": 8, "offset": 12 },
        })
    );

    let (_, right) = azure_thread_right_only(&azure_pr(), &note).expect("a mixed block");
    assert_eq!(
        right["threadContext"],
        json!({
            "filePath": "/a.ts",
            "rightFileStart": { "line": 7, "offset": 1 },
            "rightFileEnd": { "line": 8, "offset": 12 },
        })
    );

    let plain = block(
        (NoteSide::New, 11),
        (NoteSide::New, 13),
        None,
        "a\nb\nc",
        "x",
    );
    assert!(azure_thread_right_only(&azure_pr(), &plain).is_none());
}

#[test]
fn github_comments_on_a_block_start_at_its_first_line() {
    let note = mixed();
    let (_, body) = github_review(&github_pr(), std::slice::from_ref(&note), "abc123");
    assert_eq!(
        body["comments"][0],
        json!({
            "path": "a.ts", "start_line": 3, "start_side": "LEFT",
            "line": 8, "side": "RIGHT", "body": "x",
        })
    );

    let args = github_comment(&github_pr(), &note, "abc123", false);
    let at = |needle: &str| {
        args.iter()
            .position(|arg| arg == needle)
            .unwrap_or_else(|| panic!("{needle} in {args:?}"))
    };
    assert!(at("start_line=3") < at("start_side=LEFT"));
    assert!(at("start_side=LEFT") < at("line=8"));
    assert!(at("line=8") < at("side=RIGHT"));

    let range = block(
        (NoteSide::New, 11),
        (NoteSide::New, 13),
        None,
        "a\nb\nc",
        "x",
    );
    let on_file = github_comment(&github_pr(), &range, "abc123", true);
    assert!(
        on_file.contains(&"body=Lines 11\u{2013}13: x".to_string()),
        "{on_file:?}"
    );
    let on_file = github_comment(&github_pr(), &note, "abc123", true);
    assert!(
        on_file.contains(&"body=Removed line 3 to line 8: x".to_string()),
        "{on_file:?}"
    );
}

struct Narrowed;

impl Poster for Narrowed {
    fn post(&self, _: &PullRequest, notes: &[Note]) -> Vec<Result<Link>> {
        notes
            .iter()
            .map(|_| {
                Ok(Link {
                    remark: Some("Azure DevOps highlights 7\u{2013}8 only".into()),
                    ..Default::default()
                })
            })
            .collect()
    }
}

#[test]
fn a_thread_narrowed_to_one_side_is_reported_after_posting() {
    let dir = TempDir::new().expect("temp dir");
    let mut store = NoteStore::at(dir.path().join("notes.json")).scoped("PR 42");
    store.set(mixed()).expect("save");
    let mut out = Vec::new();
    offer_upload(
        &mut store,
        &azure_pr(),
        &Narrowed,
        true,
        &mut "".as_bytes(),
        &mut out,
    )
    .expect("offer");
    let out = String::from_utf8(out).expect("utf8");
    let said = "  `a.ts` (removed line 3 to line 8): Azure DevOps highlights 7\u{2013}8 only\n";
    let summary = out.find("Posted 1 comment").expect("summary");
    assert!(out.find(said).expect(&out) < summary, "{out}");
}

#[test]
fn an_azure_thread_on_a_block_ending_on_a_blank_line_ends_at_offset_two() {
    let note = block(
        (NoteSide::New, 11),
        (NoteSide::New, 13),
        None,
        "fn total() {\n  return t;\n",
        "x",
    );
    let (_, body) = azure_thread(&azure_pr(), &note);
    assert_eq!(
        body["threadContext"]["rightFileEnd"],
        json!({ "line": 13, "offset": 2 })
    );
}

#[test]
fn a_mixed_block_azure_devops_refuses_is_posted_again_on_the_right_side() {
    let sent = RefCell::new(Vec::new());
    let link = post_azure_thread(&azure_pr(), &mixed(), |(_, body): &(String, Value)| {
        sent.borrow_mut().push(body.clone());
        if sent.borrow().len() == 1 {
            bail!("both sides refused");
        }
        Ok(json!({ "id": 77 }))
    })
    .expect("posted on the right side");
    let sent = sent.into_inner();
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[0], azure_thread(&azure_pr(), &mixed()).1);
    assert_eq!(
        sent[1]["threadContext"],
        json!({
            "filePath": "/a.ts",
            "rightFileStart": { "line": 7, "offset": 1 },
            "rightFileEnd": { "line": 8, "offset": 12 },
        })
    );
    assert_eq!(
        link.remark.as_deref(),
        Some("Azure DevOps highlights 7\u{2013}8 only")
    );
    assert_eq!(link.id, "77");
    assert_eq!(link.url, azure_link(&azure_pr(), "77"));

    let calls = Cell::new(0);
    let link = post_azure_thread(&azure_pr(), &mixed(), |_: &(String, Value)| {
        calls.set(calls.get() + 1);
        Ok(json!({ "id": 78 }))
    })
    .expect("posted on both sides");
    assert_eq!((calls.get(), link.remark), (1, None));
}

#[test]
fn a_block_on_one_side_azure_devops_refuses_is_not_posted_again() {
    let plain = block(
        (NoteSide::New, 11),
        (NoteSide::New, 13),
        None,
        "a\nb\nc",
        "x",
    );
    let calls = Cell::new(0);
    let error = post_azure_thread(&azure_pr(), &plain, |_: &(String, Value)| {
        calls.set(calls.get() + 1);
        bail!("refused")
    })
    .expect_err("refused");
    assert_eq!(calls.get(), 1);
    assert_eq!(format!("{error:#}"), "refused");
}

#[test]
fn when_azure_devops_refuses_the_right_side_too_both_refusals_are_told() {
    let error = post_azure_thread(&azure_pr(), &mixed(), |(_, body): &(String, Value)| {
        if body["threadContext"].get("leftFileStart").is_some() {
            bail!("both sides refused")
        }
        bail!("right side refused")
    })
    .expect_err("refused twice");
    assert_eq!(
        format!("{error:#}"),
        "both sides refused; on the right side alone: right side refused"
    );
}
