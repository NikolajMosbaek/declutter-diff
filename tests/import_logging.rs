use declutter::classify::classify;
use declutter::diff::{Hunk, RowKind};
use declutter::lang::Lang;
use declutter::project::LayerMode;
use declutter::review::{ChangeStatus, FileChange, FileReview, Layers, SpanModes, Summary};

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

fn changes(file: &FileReview, modes: SpanModes) -> Vec<String> {
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

fn texts<'a>(src: &'a str, spans: &[std::ops::Range<usize>]) -> Vec<&'a str> {
    spans.iter().map(|span| &src[span.clone()]).collect()
}

fn modes(imports: LayerMode, logging: LayerMode) -> SpanModes {
    SpanModes {
        imports,
        logging,
        ..SpanModes::SHOWN
    }
}

#[test]
fn imports_are_found_in_every_language() {
    let swift = "import Foundation\n@testable import Cart\nlet a = 1\n";
    let ts = "import { a } from './a';\nexport { b } from './b';\nexport const c = 1;\n";
    let py = "import os\nfrom a import b\nfrom __future__ import annotations\nx = 1\n";

    let found = |lang, src| texts(src, &classify(lang, src).expect("parses").imports);
    assert_eq!(
        found(Lang::Swift, swift),
        ["import Foundation", "@testable import Cart"]
    );
    assert_eq!(
        found(Lang::TypeScript, ts),
        ["import { a } from './a';", "export { b } from './b';"]
    );
    assert_eq!(
        found(Lang::Python, py),
        [
            "import os",
            "from a import b",
            "from __future__ import annotations"
        ]
    );
}

#[test]
fn logging_statements_are_found_but_other_calls_are_not() {
    let swift = "func f() {\n    print(\"x\")\n    logger.debug(\"y \\(a)\")\n    Self.logger.info(\"z\")\n    dialog.error(\"keep\")\n    let v = log(2)\n}\n";
    let ts = "function f() {\n  console.log('x');\n  this.logger.warn('y');\n  catalog.error('keep');\n  const v = console.log;\n}\n";
    let py = "def f():\n    print('x')\n    logging.info('y')\n    self.log.debug('z')\n    v = print\n    catalog.error('keep')\n";

    let found = |lang, src| texts(src, &classify(lang, src).expect("parses").logging);
    assert_eq!(
        found(Lang::Swift, swift),
        [
            "print(\"x\")",
            "logger.debug(\"y \\(a)\")",
            "Self.logger.info(\"z\")"
        ]
    );
    assert_eq!(
        found(Lang::TypeScript, ts),
        ["console.log('x');", "this.logger.warn('y');"]
    );
    assert_eq!(
        found(Lang::Python, py),
        ["print('x')", "logging.info('y')", "self.log.debug('z')"]
    );
}

#[test]
fn hiding_logging_shows_only_the_real_change() {
    let file = review(
        "cart.ts",
        "function total(xs) {\n  return sum(xs);\n}\n",
        "function total(xs) {\n  console.log('total', xs);\n  return sum(xs) * 2;\n}\n",
    );

    assert_eq!(
        changes(&file, modes(LayerMode::Shown, LayerMode::Hidden)),
        ["-  return sum(xs);", "+  return sum(xs) * 2;"]
    );
    assert_eq!(
        changes(&file, modes(LayerMode::Shown, LayerMode::Only)),
        ["+  console.log('total', xs);"]
    );
}

#[test]
fn hidden_layers_combine() {
    let file = review(
        "cart.py",
        "import os\nx = 1\n",
        "import os\nimport sys\n# why\nprint(x)\nx = 1\n",
    );

    let all = SpanModes {
        comments: LayerMode::Hidden,
        ..modes(LayerMode::Hidden, LayerMode::Hidden)
    };
    assert!(file.view(all).hunks.is_empty());
    assert_eq!(file.view(all).hidden_hunks, 1);
    assert!(
        !file
            .view(modes(LayerMode::Hidden, LayerMode::Shown))
            .hunks
            .is_empty()
    );

    let layers = Layers {
        imports: LayerMode::Hidden,
        logging: LayerMode::Hidden,
        ..Layers::default()
    };
    assert_eq!(
        Summary::new(std::slice::from_ref(&file), layers).status_line(layers),
        "comments: hidden · imports: hidden · logging: hidden · showing 0 of 1 hunks · 1 comment/import/logging-only hunk hidden · 1 comment/import/logging-only file"
    );
}

#[test]
fn a_layer_set_to_only_wins_over_hidden_ones() {
    let file = review(
        "rate.py",
        "# Percent.\nrate = 5\n",
        "# Fraction.\nprint(rate)\nrate = 0.05\n",
    );
    let comments_only = SpanModes {
        comments: LayerMode::Only,
        ..modes(LayerMode::Shown, LayerMode::Hidden)
    };

    assert_eq!(
        changes(&file, comments_only),
        ["-# Percent.", "+# Fraction."]
    );
}
