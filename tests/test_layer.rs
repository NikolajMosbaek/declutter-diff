use declutter::project::LayerMode;
use declutter::render::plain;
use declutter::review::{ChangeStatus, FileChange, FileReview, Layers, Summary};
use declutter::test_files::{is_test_file, is_test_path};
use declutter::tui::App;
use ratatui::crossterm::event::{KeyCode, KeyEvent};

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

fn files() -> Vec<FileReview> {
    vec![
        review(
            "Sources/Cart/Cart.swift",
            "let total = 1\n",
            "let total = 2\n",
        ),
        review(
            "Tests/CartTests/CartTests.swift",
            "let expected = 1\n",
            "let expected = 2\n",
        ),
        review("web/src/cart.test.ts", "expect(1);\n", "expect(2);\n"),
    ]
}

fn layers(tests: LayerMode) -> Layers {
    Layers {
        tests,
        ..Layers::default()
    }
}

#[test]
fn test_paths_are_recognised_across_languages() {
    let tests = [
        "Tests/CartTests/CartTests.swift",
        "App/AppTests/LoginTests.swift",
        "App/AppTests/Helpers/Fixtures.swift",
        "Sources/Cart/CartSpec.swift",
        "web/src/cart.test.ts",
        "web/src/Cart.spec.tsx",
        "web/src/__tests__/cart.ts",
        "web/src/__mocks__/api.js",
        "pkg/test_cart.py",
        "pkg/cart_test.py",
        "pkg/conftest.py",
        "tests/fixtures/cart.json",
        "Tests/CartTests/__Snapshots__/CartTests/render.1.png",
    ];
    let code = [
        "Sources/Cart/Cart.swift",
        "Sources/Cart/Latest.swift",
        "Sources/Cart/Contest.swift",
        "Sources/Testing/Harness.swift",
        "web/src/attestation.ts",
        "pkg/testimonials.py",
        "Test.swift",
    ];

    for path in tests {
        assert!(is_test_path(path), "{path} should be a test path");
    }
    for path in code {
        assert!(!is_test_path(path), "{path} should not be a test path");
    }
}

#[test]
fn files_outside_test_directories_count_as_tests_when_they_import_a_test_framework() {
    assert!(is_test_file(
        "Sources/Harness.swift",
        "import Foundation\n@testable import Cart\n"
    ));
    assert!(is_test_file("Sources/Harness.swift", "import Testing\n"));
    assert!(is_test_file("pkg/checks.py", "import pytest\n"));
    assert!(is_test_file(
        "web/src/checks.ts",
        "import { describe } from 'vitest';\n"
    ));
    assert!(!is_test_file(
        "Sources/Cart.swift",
        "import Foundation\n// import XCTest\n"
    ));
    assert!(!is_test_file(
        "web/src/cart.ts",
        "import { total } from './vitest-free';\n"
    ));
}

#[test]
fn hiding_tests_leaves_test_files_out_and_says_how_many() {
    let files = files();
    let layers = layers(LayerMode::Hidden);

    let summary = Summary::new(&files, layers);
    assert_eq!((summary.files, summary.filtered_files), (1, 2));
    assert_eq!(
        summary.status_line(layers),
        "comments: hidden · tests: hidden · showing 1 of 1 hunks · 0 comment-only hunks hidden · 2 test files hidden"
    );

    let text = plain(&files, layers);
    assert!(text.contains("M Sources/Cart/Cart.swift [Swift]"), "{text}");
    assert!(!text.contains("CartTests"), "{text}");
}

#[test]
fn tests_only_lists_just_the_test_files() {
    let files = files();
    let layers = layers(LayerMode::Only);

    let text = plain(&files, layers);
    assert!(
        text.contains("M Tests/CartTests/CartTests.swift [Swift, test]"),
        "{text}"
    );
    assert!(
        text.contains("M web/src/cart.test.ts [TypeScript, test]"),
        "{text}"
    );
    assert!(!text.contains("Sources/Cart/Cart.swift"), "{text}");
    assert!(
        text.trim_end().ends_with("1 non-test file hidden"),
        "{text}"
    );
}

#[test]
fn pressing_t_keeps_the_cursor_on_the_same_file_when_it_stays_listed() {
    let mut app = App::new(files(), Layers::default());
    app.handle_key(KeyEvent::from(KeyCode::Down));
    app.handle_key(KeyEvent::from(KeyCode::Down));
    assert_eq!(
        app.current().map(|f| f.path.as_str()),
        Some("web/src/cart.test.ts")
    );

    app.handle_key(KeyEvent::from(KeyCode::Char('t')));
    assert_eq!(app.layers.tests, LayerMode::Hidden);
    assert_eq!(
        app.current().map(|f| f.path.as_str()),
        Some("Sources/Cart/Cart.swift")
    );

    app.handle_key(KeyEvent::from(KeyCode::Char('t')));
    assert_eq!(app.layers.tests, LayerMode::Only);
    assert_eq!(
        app.current().map(|f| f.path.as_str()),
        Some("Tests/CartTests/CartTests.swift")
    );
    app.handle_key(KeyEvent::from(KeyCode::Down));
    app.handle_key(KeyEvent::from(KeyCode::Char('t')));
    assert_eq!(app.layers.tests, LayerMode::Shown);
    assert_eq!(
        app.current().map(|f| f.path.as_str()),
        Some("web/src/cart.test.ts")
    );
}
