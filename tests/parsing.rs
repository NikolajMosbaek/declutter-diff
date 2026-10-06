use declutter::classify::classify;
use declutter::lang::{Lang, parsable};
use declutter::review::{ChangeStatus, Detection, FileChange, FileReview};

#[test]
fn swift_the_grammar_cannot_parse_is_stood_in_for_without_moving_any_byte() {
    let cases = [
        (
            "outcome.setValue(.success(()))",
            "outcome.setValue(.success([]))",
        ),
        (
            "continuation.resume(returning: ())",
            "continuation.resume(returning: [])",
        ),
        ("self.value = value ?? ()", "self.value = value ?? []"),
        (
            "if await session.status != .idle {",
            "if       session.status != .idle {",
        ),
        (
            "} else if await !isBackground {",
            "} else if       !isBackground {",
        ),
        (
            "nonisolated(unsafe) var ref: Ref?",
            "nonisolated         var ref: Ref?",
        ),
        // Left alone: calls, declarations, function types, ordinary awaits.
        (
            "func reset() -> () { start() }",
            "func reset() -> () { start() }",
        ),
        ("let done: () -> Void = {}", "let done: () -> Void = {}"),
        ("let v = await load()", "let v = await load()"),
        ("verify(await fetch())", "verify(await fetch())"),
    ];
    for (src, expected) in cases {
        let text = parsable(Lang::Swift, src);
        assert_eq!(text, expected, "{src}");
        assert_eq!(text.len(), src.len());
    }
}

#[test]
fn files_using_those_constructs_parse_cleanly() {
    let src = "func run() async {\n    // Wait for the link.\n    if await session.isReady == false {\n        continuation.resume(returning: ())\n    }\n    outcome.setValue(.success(()))\n}\n";

    let classified = classify(Lang::Swift, src).expect("parses");
    assert_eq!(classified.error_line, None);
    assert_eq!(classified.comments.len(), 1);
}

#[test]
fn a_partial_parse_says_where_it_went_wrong() {
    let file = FileReview::new(FileChange {
        path: "Broken.swift".into(),
        old_path: None,
        status: ChangeStatus::Modified,
        old: Some("let a = 1\n".into()),
        new: Some("let a = 1\nlet b = 2\nfunc broken( {\n".into()),
        binary: false,
    });

    assert_eq!(file.detection, Detection::Partial(Lang::Swift, 3));
    assert_eq!(file.tag(), "Swift, partial parse near line 3");
}
