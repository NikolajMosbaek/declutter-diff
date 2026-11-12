use declutter::markdown_files::is_markdown_file;
use declutter::project::LayerMode;
use declutter::render::plain;
use declutter::review::{ChangeStatus, FileChange, FileReview, Layers, Summary};
use declutter::tui::{App, draw};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
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
        review("README.md", "# Cart\n", "# Cart\n\nTotals a cart.\n"),
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
        review("docs/guide.markdown", "Old.\n", "New.\n"),
    ]
}

fn layers(tests: LayerMode, markdown: LayerMode) -> Layers {
    Layers {
        tests,
        markdown,
        ..Layers::default()
    }
}

fn listed(files: &[FileReview], layers: Layers) -> Vec<&str> {
    files
        .iter()
        .filter(|file| file.is_visible(layers))
        .map(|file| file.path.as_str())
        .collect()
}

fn screen(app: &mut App) -> String {
    let mut terminal = Terminal::new(TestBackend::new(120, 24)).expect("terminal");
    terminal.draw(|frame| draw(frame, app)).expect("draw");
    let buffer = terminal.backend().buffer();
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn markdown_files_are_recognised_by_extension() {
    for path in [
        "README.md",
        "docs/guide.markdown",
        "CHANGELOG.MD",
        "Sources/Cart/Cart.docc/Overview.md",
    ] {
        assert!(is_markdown_file(path), "{path} should be Markdown");
    }
    for path in [
        "Sources/Cart/Cart.swift",
        "docs/md/index.html",
        "README",
        "notes.mdx",
        ".md",
        "src/md.rs",
    ] {
        assert!(!is_markdown_file(path), "{path} should not be Markdown");
    }
}

#[test]
fn hiding_markdown_leaves_markdown_files_out_and_says_how_many() {
    let files = files();
    let layers = layers(LayerMode::Shown, LayerMode::Hidden);

    let summary = Summary::new(&files, layers);
    assert_eq!((summary.files, summary.filtered_files), (2, 2));
    assert_eq!(
        summary.status_line(layers),
        "comments: hidden · markdown: hidden · showing 2 of 2 hunks · 0 comment-only hunks hidden · 2 markdown files hidden"
    );

    let text = plain(&files, layers);
    assert!(text.contains("M Sources/Cart/Cart.swift [Swift]"), "{text}");
    assert!(!text.contains("README.md"), "{text}");
    assert!(!text.contains("guide.markdown"), "{text}");
}

#[test]
fn markdown_only_lists_just_the_markdown_files_with_their_whole_diff() {
    let files = files();
    let layers = layers(LayerMode::Shown, LayerMode::Only);

    assert_eq!(listed(&files, layers), ["README.md", "docs/guide.markdown"]);
    let text = plain(&files, layers);
    assert!(text.contains("+Totals a cart."), "{text}");
    assert!(text.contains(" · 2 non-markdown files hidden · "), "{text}");
}

#[test]
fn hiding_tests_and_markdown_counts_each_left_out_file_once() {
    let mut files = files();
    files.push(review("Tests/README.md", "Old.\n", "New.\n"));
    let layers = layers(LayerMode::Hidden, LayerMode::Hidden);

    assert_eq!(listed(&files, layers), ["Sources/Cart/Cart.swift"]);
    let status = Summary::new(&files, layers).status_line(layers);
    assert!(
        status.contains(" · 2 test files hidden · 2 markdown files hidden"),
        "a Markdown test file counts once, as a test file: {status}"
    );
}

#[test]
fn only_wins_over_hidden_and_two_only_layers_list_both_kinds_of_file() {
    let mut files = files();
    files.push(review("Tests/README.md", "Old.\n", "New.\n"));

    assert_eq!(
        listed(&files, layers(LayerMode::Hidden, LayerMode::Only)),
        ["README.md", "docs/guide.markdown", "Tests/README.md"],
        "with Markdown only, hiding tests does not take the Markdown test file away"
    );
    let tests_only = layers(LayerMode::Only, LayerMode::Hidden);
    assert_eq!(
        listed(&files, tests_only),
        ["Tests/CartTests/CartTests.swift", "Tests/README.md"]
    );
    assert_eq!(
        Summary::new(&files, tests_only).filtered_markdown_files,
        0,
        "the README is left out by tests only, not by hiding Markdown"
    );

    let both = layers(LayerMode::Only, LayerMode::Only);
    assert_eq!(
        listed(&files, both),
        [
            "README.md",
            "Tests/CartTests/CartTests.swift",
            "docs/guide.markdown",
            "Tests/README.md"
        ]
    );
    let text = plain(&files, both);
    assert!(
        text.contains("+Totals a cart."),
        "only tests does not cut a Markdown file's lines: {text}"
    );
    assert!(text.contains(" · 1 other file hidden"), "{text}");
}

#[test]
fn pressing_w_keeps_the_cursor_on_the_same_file_when_it_stays_listed() {
    let mut app = App::new(files(), Layers::default());
    app.handle_key(KeyEvent::from(KeyCode::Down));
    app.handle_key(KeyEvent::from(KeyCode::Down));
    assert_eq!(
        app.current().map(|f| f.path.as_str()),
        Some("Tests/CartTests/CartTests.swift")
    );

    app.handle_key(KeyEvent::from(KeyCode::Char('w')));
    assert_eq!(app.layers.markdown, LayerMode::Hidden);
    assert_eq!(
        app.current().map(|f| f.path.as_str()),
        Some("Tests/CartTests/CartTests.swift")
    );
    assert_eq!(
        app.review_progress().1,
        2,
        "the Markdown files are left out"
    );

    app.handle_key(KeyEvent::from(KeyCode::Char('W')));
    assert_eq!(app.layers.markdown, LayerMode::Only);
    assert_eq!(
        app.review_progress().1,
        2,
        "only the Markdown files are listed"
    );
    assert!(app.current().is_some_and(|f| f.is_markdown));

    app.handle_key(KeyEvent::from(KeyCode::Char('W')));
    assert_eq!(
        app.layers.markdown,
        LayerMode::Hidden,
        "W again goes back to what it was"
    );
}

#[test]
fn an_empty_list_says_which_key_brings_the_files_back() {
    let markdown = vec![review("README.md", "# Cart\n", "# Cart!\n")];
    let mut hidden = App::new(
        markdown.clone(),
        layers(LayerMode::Shown, LayerMode::Hidden),
    );
    assert!(
        screen(&mut hidden)
            .contains("Every changed file is a Markdown file. Press w to show Markdown files."),
        "{}",
        screen(&mut hidden)
    );

    let code = vec![review("Cart.swift", "let a = 1\n", "let a = 2\n")];
    let mut only = App::new(code, layers(LayerMode::Shown, LayerMode::Only));
    assert!(
        screen(&mut only).contains("No Markdown files changed. Press W to show the other files."),
        "{}",
        screen(&mut only)
    );

    let mut both_only = App::new(
        vec![review("Cart.swift", "let a = 1\n", "let a = 2\n")],
        layers(LayerMode::Only, LayerMode::Only),
    );
    assert!(
        screen(&mut both_only)
            .contains("No test or Markdown files changed. Press a to show the other files."),
        "{}",
        screen(&mut both_only)
    );

    let mut tests_hidden = App::new(
        vec![review("Tests/A.swift", "let a = 1\n", "let a = 2\n")],
        layers(LayerMode::Hidden, LayerMode::Shown),
    );
    assert!(
        screen(&mut tests_hidden)
            .contains("Every changed file is a test file. Press t to show tests."),
        "the tests layer keeps its words: {}",
        screen(&mut tests_hidden)
    );
}
