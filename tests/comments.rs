use declutter::classify::classify;
use declutter::diff::{Hunk, RowKind};
use declutter::lang::Lang;
use declutter::project::{LayerMode, project};
use declutter::review::{ChangeStatus, Detection, FileChange, FileReview, Layers, Summary};

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

/// The changed rows of a view as `-text` / `+text`.
fn changes(file: &FileReview, mode: LayerMode) -> Vec<String> {
    file.view(mode)
        .hunks
        .iter()
        .flat_map(Hunk::changed)
        .map(|row| {
            let sign = if row.kind == RowKind::Removed {
                '-'
            } else {
                '+'
            };
            format!("{sign}{}", row.text)
        })
        .collect()
}

fn hidden_projection(lang: Lang, src: &str) -> Vec<String> {
    let comments = classify(lang, src).expect("parser runs").comments;
    project(src, &comments, LayerMode::Hidden).lines
}

#[test]
fn comment_only_change_disappears_when_comments_are_hidden() {
    let file = review(
        "calc.py",
        "def total(xs):\n    # add them up\n    return sum(xs)\n",
        "def total(xs):\n    # Sum every element of the list.\n    return sum(xs)\n",
    );

    assert_eq!(file.view(LayerMode::Shown).hunks.len(), 1);
    assert!(file.view(LayerMode::Hidden).hunks.is_empty());
    assert_eq!(file.view(LayerMode::Hidden).hidden_hunks, 1);
}

#[test]
fn changed_line_with_new_trailing_comment_shows_only_the_code_change() {
    let file = review(
        "config.ts",
        "const retries = 1;\n",
        "const retries = 3; // be patient\n",
    );

    assert_eq!(
        changes(&file, LayerMode::Hidden),
        ["-const retries = 1;", "+const retries = 3;"]
    );
}

#[test]
fn trailing_comment_added_to_unchanged_code_is_hidden() {
    let file = review("main.ts", "start();\n", "start(); // kick off the loop\n");

    assert!(file.view(LayerMode::Hidden).hunks.is_empty());
}

#[test]
fn block_comment_spanning_a_hunk_boundary_keeps_original_line_numbers() {
    let old = "func setup() {}\n/* One\n two\n three\n four\n five\n six\n seven\n eight */\nfunc value() -> Int { 1 }\n";
    let new = "func setup() {}\n/* One\n two\n three\n four\n five\n six\n seven\n eight, revised */\nfunc value() -> Int { 2 }\n";
    let file = review("Value.swift", old, new);

    let hunks = &file.view(LayerMode::Hidden).hunks;
    assert_eq!(hunks.len(), 1);
    let rows: Vec<_> = hunks[0]
        .rows
        .iter()
        .map(|row| (row.kind, row.old_line, row.new_line, row.text.as_str()))
        .collect();
    assert_eq!(
        rows,
        [
            (RowKind::Context, Some(1), Some(1), "func setup() {}"),
            (
                RowKind::Removed,
                Some(10),
                None,
                "func value() -> Int { 1 }"
            ),
            (RowKind::Added, None, Some(10), "func value() -> Int { 2 }"),
        ]
    );
}

#[test]
fn code_sharing_lines_with_a_block_comment_is_kept() {
    let lines = hidden_projection(
        Lang::TypeScript,
        "const a = 1; /* note\n   more */ const b = 2;\n",
    );

    assert_eq!(lines, ["const a = 1;", "const b = 2;"]);
}

#[test]
fn inline_block_comment_does_not_leave_a_double_space() {
    let lines = hidden_projection(Lang::TypeScript, "call(a, /* the b */ b);\n");

    assert_eq!(lines, ["call(a, b);"]);
}

#[test]
fn python_docstrings_are_part_of_the_comment_layer() {
    let src = "\"\"\"Module doc.\"\"\"\n\nclass Cart:\n    \"\"\"A cart.\"\"\"\n\n    def add(self, item):\n        \"\"\"Add an item.\n\n        Keeps order.\n        \"\"\"\n        self.items.append(item)\n";

    assert_eq!(
        hidden_projection(Lang::Python, src),
        [
            "",
            "class Cart:",
            "",
            "    def add(self, item):",
            "        self.items.append(item)"
        ]
    );
}

#[test]
fn strings_that_are_not_docstrings_are_code() {
    let src =
        "x = \"\"\"not a docstring\"\"\"\ndef f():\n    run()\n    \"\"\"also not one\"\"\"\n";

    assert_eq!(
        hidden_projection(Lang::Python, src),
        [
            "x = \"\"\"not a docstring\"\"\"",
            "def f():",
            "    run()",
            "    \"\"\"also not one\"\"\""
        ]
    );
}

#[test]
fn comment_markers_inside_strings_are_code() {
    let ts = review(
        "api.ts",
        "const url = \"http://a.example\";\n",
        "const url = \"http://b.example\";\n",
    );
    let py = review("tag.py", "tag = \"# one\"\n", "tag = \"# two\"\n");

    assert_eq!(
        changes(&ts, LayerMode::Hidden),
        [
            "-const url = \"http://a.example\";",
            "+const url = \"http://b.example\";"
        ]
    );
    assert_eq!(
        changes(&py, LayerMode::Hidden),
        ["-tag = \"# one\"", "+tag = \"# two\""]
    );
}

#[test]
fn swift_doc_comments_and_nested_block_comments_are_comments() {
    let src =
        "/// Adds one.\n/* outer /* inner */ still outer */\nfunc inc(_ x: Int) -> Int { x + 1 }\n";

    assert_eq!(
        hidden_projection(Lang::Swift, src),
        ["func inc(_ x: Int) -> Int { x + 1 }"]
    );
}

#[test]
fn blank_lines_added_with_a_comment_are_hidden_with_it() {
    let file = review(
        "steps.py",
        "a = 1\nb = 2\n",
        "a = 1\n\n# b depends on a\nb = 2\n",
    );

    assert!(file.view(LayerMode::Hidden).hunks.is_empty());
    assert_eq!(file.view(LayerMode::Hidden).hidden_hunks, 1);
}

#[test]
fn comments_only_shows_just_the_comment_changes() {
    let file = review(
        "rate.ts",
        "// Rate in percent.\nconst rate = 5; // default\n",
        "// Rate as a fraction.\nconst rate = 0.05; // default\n",
    );

    assert_eq!(
        changes(&file, LayerMode::Only),
        ["-// Rate in percent.", "+// Rate as a fraction."]
    );
}

#[test]
fn code_only_change_is_hidden_in_comments_only_mode() {
    let file = review(
        "sum.ts",
        "// Sum.\nconst n = 1;\n",
        "// Sum.\nconst n = 2;\n",
    );

    assert!(file.view(LayerMode::Only).hunks.is_empty());
    assert_eq!(file.view(LayerMode::Only).hidden_hunks, 1);
}

#[test]
fn file_with_syntax_errors_is_marked_partial_and_still_decluttered() {
    let file = review(
        "Broken.swift",
        "// Old note.\nfunc broken( {\n",
        "// New note.\nfunc broken( {\n",
    );

    assert_eq!(file.detection, Detection::Partial(Lang::Swift));
    assert!(file.view(LayerMode::Hidden).hunks.is_empty());
}

#[test]
fn unsupported_file_type_hides_nothing() {
    let file = review("notes.txt", "# heading\n", "# new heading\n");

    assert_eq!(file.detection, Detection::Unsupported);
    assert_eq!(
        file.view(LayerMode::Hidden).hunks,
        file.view(LayerMode::Shown).hunks
    );
}

#[test]
fn status_line_reports_what_is_hidden() {
    let files = [
        review("a.py", "# one\nx = 1\n", "# two\nx = 1\n"),
        review("b.py", "y = 1\n", "y = 2\n"),
        review("c.txt", "a\n", "b\n"),
    ];

    assert_eq!(
        Summary::new(&files, Layers::default()).status_line(Layers::default()),
        "comments: hidden · showing 2 of 3 hunks · 1 comment-only hunk hidden · 1 comment-only file · no grammar for 1 file"
    );
}

#[test]
fn blank_lines_are_layout_when_comments_are_hidden() {
    let file = review(
        "cart.py",
        "class Cart:\n    def total(self):\n        return 0\n",
        "class Cart:\n    \"\"\"A cart.\"\"\"\n\n    def total(self):\n        return 1\n",
    );

    assert_eq!(
        changes(&file, LayerMode::Hidden),
        ["-        return 0", "+        return 1"]
    );
}
