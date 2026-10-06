use declutter::diff::{Hunk, RowKind};
use declutter::project::LayerMode;
use declutter::review::{ChangeStatus, DiffModes, FileChange, FileReview};

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

fn formatting(mode: LayerMode) -> DiffModes {
    DiffModes {
        formatting: mode,
        ..DiffModes::SHOWN
    }
}

fn changes(file: &FileReview, modes: DiffModes) -> Vec<String> {
    file.view(modes)
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

#[test]
fn reindenting_and_respacing_are_formatting() {
    let file = review(
        "Cart.swift",
        "func total() -> Int {\nreturn a+b\n}\n",
        "func total() -> Int {\n    return a + b\n}\n",
    );

    assert_eq!(changes(&file, DiffModes::SHOWN).len(), 2);
    assert!(file.view(formatting(LayerMode::Hidden)).hunks.is_empty());
    assert_eq!(file.view(formatting(LayerMode::Hidden)).hidden_hunks, 1);
}

#[test]
fn rewrapping_a_call_is_formatting_and_the_new_layout_stays_as_context() {
    let file = review(
        "cart.ts",
        "start();\ncharge(cart, card, amount);\nfinish();\nconst total = 1;\n",
        "start();\ncharge(\n  cart,\n  card,\n  amount\n);\nfinish();\nconst total = 2;\n",
    );

    let hidden = file.view(formatting(LayerMode::Hidden));
    assert_eq!(
        changes(&file, formatting(LayerMode::Hidden)),
        ["-const total = 1;", "+const total = 2;"]
    );
    let context: Vec<&str> = hidden.hunks[0]
        .rows
        .iter()
        .filter(|row| row.kind == RowKind::Context)
        .map(|row| row.text.as_str())
        .collect();
    assert!(context.contains(&"  amount"), "{context:?}");
}

#[test]
fn a_trailing_comma_is_not_formatting() {
    let file = review("cart.ts", "charge(a, b);\n", "charge(a, b,);\n");

    assert_eq!(changes(&file, formatting(LayerMode::Hidden)).len(), 2);
}

#[test]
fn indentation_is_syntax_in_python() {
    let file = review(
        "loop.py",
        "for x in xs:\n    a(x)\n    b(x)\n",
        "for x in xs:\n    a(x)\nb(x)\n",
    );

    assert_eq!(
        changes(&file, formatting(LayerMode::Hidden)),
        ["-    b(x)", "+b(x)"]
    );
}

#[test]
fn spacing_inside_a_string_is_not_formatting() {
    let file = review("greet.swift", "let s = \"a b\"\n", "let s = \"ab\"\n");

    assert_eq!(changes(&file, formatting(LayerMode::Hidden)).len(), 2);
}

#[test]
fn formatting_pairs_inside_a_mixed_change_are_told_apart() {
    let file = review(
        "rates.swift",
        "let a=1\nlet b = 2\n",
        "let a = 1\nlet b = 3\n",
    );

    assert_eq!(
        changes(&file, formatting(LayerMode::Hidden)),
        ["-let b = 2", "+let b = 3"]
    );
    assert_eq!(
        changes(&file, formatting(LayerMode::Only)),
        ["-let a=1", "+let a = 1"]
    );
}

#[test]
fn added_blank_lines_are_formatting() {
    let file = review(
        "a.swift",
        "let a = 1\nlet b = 2\n",
        "let a = 1\n\n\nlet b = 2\n",
    );

    assert!(file.view(formatting(LayerMode::Hidden)).hunks.is_empty());
}
