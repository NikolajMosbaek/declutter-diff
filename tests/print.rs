use declutter::render::{plain, plain_numbered};
use declutter::review::{ChangeStatus, FileChange, FileReview, Layers};

fn rate_change() -> Vec<FileReview> {
    vec![FileReview::new(FileChange {
        path: "rate.ts".to_string(),
        old_path: None,
        status: ChangeStatus::Modified,
        old: Some("const a = 1;\nconst rate = 5;\nconst b = 2;\n".to_string()),
        new: Some(
            "const a = 1;\n// a fraction, not a percentage\nconst rate = 0.05;\nconst b = 2;\n"
                .to_string(),
        ),
        binary: false,
    })]
}

#[test]
fn line_numbers_name_both_versions_even_with_rows_hidden() {
    let text = plain_numbered(&rate_change(), Layers::default());

    assert!(text.contains("    1     1  const a = 1;\n"), "{text}");
    assert!(text.contains("    2       -const rate = 5;\n"), "{text}");
    assert!(
        text.contains("          3 +const rate = 0.05;\n"),
        "the hidden comment on new line 2 is skipped, not counted: {text}"
    );
    assert!(text.contains("    3     4  const b = 2;\n"), "{text}");
    assert!(!text.contains("fraction"), "{text}");
}

#[test]
fn plain_output_stays_without_numbers() {
    let text = plain(&rate_change(), Layers::default());

    assert!(text.contains("\n+const rate = 0.05;\n"), "{text}");
}
