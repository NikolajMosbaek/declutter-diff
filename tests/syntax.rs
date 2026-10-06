use declutter::highlight::{Class, annotate};
use declutter::lang::Lang;
use declutter::project::{LayerMode, project};

fn classes(lang: Lang, src: &str) -> Vec<Vec<(String, Class)>> {
    let mut projection = project(src, &[], LayerMode::Shown);
    annotate(&mut projection, lang);
    projection
        .lines
        .iter()
        .zip(&projection.syntax)
        .map(|(line, spans)| {
            spans
                .iter()
                .map(|(r, c)| (line[r.clone()].to_string(), *c))
                .collect()
        })
        .collect()
}

fn has(lines: &[Vec<(String, Class)>], text: &str, class: Class) -> bool {
    lines
        .iter()
        .flatten()
        .any(|(t, c)| t == text && *c == class)
}

#[test]
fn every_language_gets_keywords_strings_and_numbers() {
    let cases = [
        (Lang::Swift, "let name = \"cart\"\nlet count = 42\n"),
        (
            Lang::TypeScript,
            "const name: string = \"cart\";\nconst count = 42;\n",
        ),
        (
            Lang::Tsx,
            "const view = <b>{\"cart\"}</b>;\nconst count = 42;\n",
        ),
        (
            Lang::Python,
            "def name():\n    return \"cart\" if 42 else None\n",
        ),
    ];
    for (lang, src) in cases {
        let lines = classes(lang, src);
        let keyword = if lang == Lang::Python {
            "def"
        } else if lang == Lang::Swift {
            "let"
        } else {
            "const"
        };
        assert!(has(&lines, keyword, Class::Keyword), "{lang:?}: {lines:?}");
        assert!(
            has(&lines, "\"cart\"", Class::String),
            "{lang:?}: {lines:?}"
        );
        assert!(has(&lines, "42", Class::Number), "{lang:?}: {lines:?}");
    }
}

#[test]
fn a_string_spanning_lines_is_coloured_on_each_line() {
    let lines = classes(Lang::Python, "s = \"\"\"one\ntwo\"\"\"\n");

    assert!(has(&lines, "\"\"\"one", Class::String), "{lines:?}");
    assert!(has(&lines, "two\"\"\"", Class::String), "{lines:?}");
}
