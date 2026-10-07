use declutter::classify::classify;
use declutter::diff::{Hunk, RowKind};
use declutter::highlight::{Class, annotate, config};
use declutter::lang::Lang;
use declutter::project::{LayerMode, project};
use declutter::review::{ChangeStatus, DiffModes, FileChange, FileReview};
use declutter::test_files::is_test_file;

fn texts<'a>(src: &'a str, spans: &[std::ops::Range<usize>]) -> Vec<&'a str> {
    spans.iter().map(|span| &src[span.clone()]).collect()
}

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

fn tests(mode: LayerMode) -> DiffModes {
    DiffModes {
        tests: mode,
        ..DiffModes::SHOWN
    }
}

#[test]
fn files_are_recognised_by_extension() {
    assert_eq!(Lang::from_path("cmd/main.go"), Some(Lang::Go));
    assert_eq!(Lang::from_path("src/lib.rs"), Some(Lang::Rust));
    assert_eq!(Lang::from_path("app/Cart.kt"), Some(Lang::Kotlin));
    assert_eq!(Lang::from_path("build.gradle.kts"), Some(Lang::Kotlin));
}

#[test]
fn comments_and_doc_comments_are_found() {
    let go = "// Package cart sums.\npackage cart\n/* block */\nfunc A() {} // trailing\n";
    let rust = "//! Crate docs.\n/// Sums.\nfn a() {} // trailing\n/* block */\n";
    let kotlin = "/** Sums. */\nfun a() = 1 // trailing\n";

    let found = |lang, src| texts(src, &classify(lang, src).expect("parses").comments);
    assert_eq!(
        found(Lang::Go, go),
        ["// Package cart sums.", "/* block */", "// trailing"]
    );
    assert_eq!(
        found(Lang::Rust, rust),
        [
            "//! Crate docs.\n",
            "/// Sums.\n",
            "// trailing",
            "/* block */"
        ]
    );
    assert_eq!(found(Lang::Kotlin, kotlin), ["/** Sums. */", "// trailing"]);
}

#[test]
fn imports_are_found() {
    let go = "package cart\n\nimport (\n\t\"fmt\"\n\t\"log\"\n)\nimport \"os\"\n";
    let rust = "use std::fmt;\nuse crate::{a, b};\nextern crate foo;\nfn a() {}\n";
    let kotlin = "package cart\n\nimport kotlin.math.max\nimport java.util.*\n\nfun a() = 1\n";

    let found = |lang, src| texts(src, &classify(lang, src).expect("parses").imports);
    assert_eq!(
        found(Lang::Go, go),
        ["import (\n\t\"fmt\"\n\t\"log\"\n)", "import \"os\""]
    );
    assert_eq!(
        found(Lang::Rust, rust),
        ["use std::fmt;", "use crate::{a, b};", "extern crate foo;"]
    );
    assert_eq!(
        found(Lang::Kotlin, kotlin),
        ["import kotlin.math.max", "import java.util.*"]
    );
}

#[test]
fn go_logging_leaves_writers_and_exits_alone() {
    let src = "package a\nfunc f(w io.Writer) {\n\tfmt.Println(\"x\")\n\tlog.Printf(\"y %d\", 1)\n\tslog.Info(\"z\")\n\tlogger.Infof(\"w %s\", v)\n\tfmt.Fprintf(w, \"body\")\n\tlog.Fatal(\"stop\")\n\tlogger.Fatalf(\"stop\")\n\tprintln(\"dbg\")\n}\n";

    assert_eq!(
        texts(src, &classify(Lang::Go, src).expect("parses").logging),
        [
            "fmt.Println(\"x\")",
            "log.Printf(\"y %d\", 1)",
            "slog.Info(\"z\")",
            "logger.Infof(\"w %s\", v)",
            "println(\"dbg\")"
        ]
    );
}

#[test]
fn rust_logging_macros_are_found_as_statements_only() {
    let src = "fn f() {\n    println!(\"x\");\n    log::info!(\"y\");\n    debug!(\"z {}\", 1);\n    tracing::warn!(a = 1, \"w\");\n    dbg!(v);\n    let v = dbg!(x);\n    assert_eq!(1, 1);\n    write!(out, \"keep\");\n}\n";

    assert_eq!(
        texts(src, &classify(Lang::Rust, src).expect("parses").logging),
        [
            "println!(\"x\");",
            "log::info!(\"y\");",
            "debug!(\"z {}\", 1);",
            "tracing::warn!(a = 1, \"w\");",
            "dbg!(v);"
        ]
    );
}

#[test]
fn kotlin_logging_includes_android_levels_only_on_loggers() {
    let src = "fun f() {\n    println(\"x\")\n    Log.d(\"TAG\", \"y\")\n    Timber.w(\"w\")\n    logger.info { \"z\" }\n    point.e(1)\n    list.add(1)\n}\n";

    assert_eq!(
        texts(src, &classify(Lang::Kotlin, src).expect("parses").logging),
        [
            "println(\"x\")",
            "Log.d(\"TAG\", \"y\")",
            "Timber.w(\"w\")",
            "logger.info { \"z\" }"
        ]
    );
}

#[test]
fn rust_test_blocks_are_found_with_their_attributes() {
    let src = "fn code() {}\n\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn adds() {}\n}\n\n#[test]\n#[ignore]\nfn slow() {}\n\n#[tokio::test]\nasync fn fetches() {}\n\n#[test]\n// Needs the network.\nfn online() {}\n\n#[cfg(all(test, feature = \"x\"))]\nmod more {}\n\n#[cfg(feature = \"testing\")]\nmod not_tests {}\n\n#[derive(Debug)]\nstruct Kept;\n";

    assert_eq!(
        texts(src, &classify(Lang::Rust, src).expect("parses").tests),
        [
            "#[cfg(test)]\nmod tests {\n    #[test]\n    fn adds() {}\n}",
            "#[test]\n#[ignore]\nfn slow() {}",
            "#[tokio::test]\nasync fn fetches() {}",
            "#[test]\n// Needs the network.\nfn online() {}",
            "#[cfg(all(test, feature = \"x\"))]\nmod more {}",
        ]
    );
}

#[test]
fn the_test_layer_cuts_or_keeps_rust_test_blocks_in_a_source_file() {
    let old = "pub fn total() -> u32 {\n    1\n}\n\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn one() {\n        assert_eq!(super::total(), 1);\n    }\n}\n";
    let new = old
        .replace("    1\n}", "    2\n}")
        .replace("total(), 1)", "total(), 2)");
    let file = review("src/cart.rs", old, &new);

    assert!(file.has_test_blocks());
    assert_eq!(
        changes(&file, tests(LayerMode::Hidden)),
        ["-    1", "+    2"]
    );
    assert_eq!(
        changes(&file, tests(LayerMode::Only)),
        [
            "-        assert_eq!(super::total(), 1);",
            "+        assert_eq!(super::total(), 2);"
        ]
    );
    assert!(
        file.is_visible(LayerMode::Only),
        "listed when only tests are shown"
    );
    assert!(
        file.is_visible(LayerMode::Hidden),
        "still listed when tests are hidden"
    );

    let layers = declutter::review::Layers {
        tests: LayerMode::Hidden,
        comments: LayerMode::Shown,
        ..Default::default()
    };
    let status =
        declutter::review::Summary::new(std::slice::from_ref(&file), layers).status_line(layers);
    assert!(
        status.contains("test-only hunks hidden"),
        "with test blocks listed, hidden test hunks are named as such: {status}"
    );
}

#[test]
fn a_whole_test_file_is_shown_whole_when_only_tests_are_shown() {
    let old = "use cart::total;\n\n#[test]\nfn one() {\n    assert_eq!(total(), 1);\n}\n";
    let new = "use cart::{total, Cart};\n\n#[test]\nfn one() {\n    assert_eq!(total(), 2);\n}\n";
    let file = review("tests/cart.rs", old, new);

    assert!(file.is_test);
    assert_eq!(
        changes(&file, tests(LayerMode::Only)),
        [
            "-use cart::total;",
            "+use cart::{total, Cart};",
            "-    assert_eq!(total(), 1);",
            "+    assert_eq!(total(), 2);",
        ]
    );
}

#[test]
fn go_and_kotlin_test_files_are_recognised() {
    assert!(is_test_file("cart/cart_test.go", "package cart\n"));
    assert!(is_test_file(
        "app/src/test/kotlin/CartTest.kt",
        "class CartTest\n"
    ));
    assert!(is_test_file(
        "app/src/main/kotlin/Checks.kt",
        "import org.junit.jupiter.api.Test\n"
    ));
    assert!(!is_test_file(
        "app/src/main/kotlin/Cart.kt",
        "import kotlin.math.max\n"
    ));
    assert!(!is_test_file("cart/cart.go", "package cart\n"));
    assert!(
        !is_test_file("src/test_files.rs", "pub fn a() {}\n"),
        "test_ is pytest's prefix only"
    );
    assert!(is_test_file("pkg/test_cart.py", "x = 1\n"));
}

#[test]
fn every_new_language_is_coloured() {
    let cases = [
        (
            Lang::Go,
            "func a() string { return \"cart\" + 42 }\n",
            "func",
        ),
        (
            Lang::Rust,
            "fn a() -> &'static str { let n = 42; \"cart\" }\n",
            "fn",
        ),
        (
            Lang::Kotlin,
            "fun a(): String { val n = 42; return \"cart\" }\n",
            "fun",
        ),
    ];
    for (lang, src, keyword) in cases {
        assert!(
            config(lang).is_some(),
            "{lang:?} has a highlight query that compiles"
        );
        let mut projection = project(src, &[], LayerMode::Shown);
        annotate(&mut projection, lang);
        let spans: Vec<(String, Class)> = projection.syntax[0]
            .iter()
            .map(|(range, class)| (projection.lines[0][range.clone()].to_string(), *class))
            .collect();
        let has = |text: &str, class| spans.iter().any(|(t, c)| t == text && *c == class);
        assert!(has(keyword, Class::Keyword), "{lang:?}: {spans:?}");
        assert!(has("\"cart\"", Class::String), "{lang:?}: {spans:?}");
        assert!(
            has("42", Class::Number) || has("42", Class::Constant),
            "{lang:?}: {spans:?}"
        );
    }
}

#[test]
fn reindenting_go_is_formatting() {
    let file = review(
        "a.go",
        "func a() {\nreturn\n}\n",
        "func a() {\n\treturn\n}\n",
    );

    assert!(
        file.view(DiffModes {
            formatting: LayerMode::Hidden,
            ..DiffModes::SHOWN
        })
        .hunks
        .is_empty()
    );
}
