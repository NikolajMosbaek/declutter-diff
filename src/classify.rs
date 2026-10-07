use std::ops::Range;

use tree_sitter::{Node, Parser};

use crate::lang::{Lang, parsable};

/// The span layers of one version of a file. Each is a list of byte ranges in
/// document order, non-overlapping within the layer.
#[derive(Debug, Clone, Default)]
pub struct Classified {
    /// Comments and docstrings.
    pub comments: Vec<Range<usize>>,
    /// Import statements (and re-exports, which are imports in disguise).
    pub imports: Vec<Range<usize>>,
    /// Statements that only log: `print(…)`, `console.log(…)`, `logger.debug(…)`.
    pub logging: Vec<Range<usize>>,
    /// Test code inside a source file — in Rust, a `#[cfg(test)]` module or a `#[test]`
    /// function, attributes included. Whole test files are found by path instead.
    pub tests: Vec<Range<usize>>,
    /// One-based line of the first syntax error the parser recovered from. Layers are
    /// still collected, but some may be missed or misplaced near it.
    pub error_line: Option<usize>,
}

/// Finds every comment, import, logging statement and test block in `src`. Returns `None` only if
/// the parser could not run at all.
pub fn classify(lang: Lang, src: &str) -> Option<Classified> {
    let mut parser = Parser::new();
    parser.set_language(&lang.grammar()).ok()?;
    let tree = parser.parse(parsable(lang, src).as_ref(), None)?;

    // Comments are tree-sitter "extras": they can appear at any depth, so walk every node.
    let mut classified = Classified::default();
    let mut cursor = tree.walk();
    'walk: loop {
        let node = cursor.node();
        let test_block = (lang == Lang::Rust)
            .then(|| rust_test_block(node, src))
            .flatten();
        let layer = if let Some(block) = &test_block {
            classified.tests.push(block.clone());
            None
        } else if lang.is_comment(node.kind()) || (lang == Lang::Python && is_docstring(node)) {
            Some(&mut classified.comments)
        } else if is_import(lang, node) {
            Some(&mut classified.imports)
        } else if is_logging(lang, node, src) {
            Some(&mut classified.logging)
        } else {
            None
        };
        if let Some(layer) = layer {
            layer.push(node.byte_range());
        } else if test_block.is_none() && cursor.goto_first_child() {
            continue;
        }
        while !cursor.goto_next_sibling() {
            if !cursor.goto_parent() {
                break 'walk;
            }
        }
    }

    classified.error_line = first_error(tree.root_node()).map(|node| node.start_position().row + 1);
    Some(classified)
}

/// A string literal on its own as the first statement of a module, class or function body.
fn is_docstring(node: Node) -> bool {
    if node.kind() != "expression_statement" || node.named_child_count() != 1 {
        return false;
    }
    let Some(expr) = node.named_child(0) else {
        return false;
    };
    if !matches!(expr.kind(), "string" | "concatenated_string") {
        return false;
    }
    let Some(parent) = node.parent() else {
        return false;
    };
    let is_body = match parent.kind() {
        "module" => true,
        "block" => parent.parent().is_some_and(|owner| {
            matches!(owner.kind(), "function_definition" | "class_definition")
        }),
        _ => false,
    };
    if !is_body {
        return false;
    }
    let mut cursor = parent.walk();
    let first_statement = parent
        .named_children(&mut cursor)
        .find(|child| child.kind() != "comment");
    first_statement.is_some_and(|first| first.id() == node.id())
}

fn is_import(lang: Lang, node: Node) -> bool {
    match lang {
        Lang::Swift | Lang::Go => node.kind() == "import_declaration",
        Lang::TypeScript | Lang::Tsx | Lang::JavaScript => {
            node.kind() == "import_statement"
                || (node.kind() == "export_statement"
                    && node.child_by_field_name("source").is_some())
        }
        Lang::Python => matches!(
            node.kind(),
            "import_statement" | "import_from_statement" | "future_import_statement"
        ),
        Lang::Rust => matches!(node.kind(), "use_declaration" | "extern_crate_declaration"),
        Lang::Kotlin => node.kind() == "import",
    }
}

/// A Rust item marked as test code, from its first attribute to its end: a module under
/// `#[cfg(test)]`, or a function under `#[test]`, `#[tokio::test]`, `#[rstest]`,
/// `#[test_case(…)]` and the like.
fn rust_test_block(node: Node, src: &str) -> Option<Range<usize>> {
    if !matches!(node.kind(), "mod_item" | "function_item") {
        return None;
    }
    let mut start = None;
    let mut is_test = false;
    let mut sibling = node.prev_sibling();
    while let Some(previous) = sibling {
        match previous.kind() {
            "attribute_item" => {
                start = Some(previous.start_byte());
                is_test |= src
                    .get(previous.byte_range())
                    .is_some_and(is_test_attribute);
            }
            "line_comment" | "block_comment" => {}
            _ => break,
        }
        sibling = previous.prev_sibling();
    }
    is_test.then(|| start.unwrap_or(node.start_byte())..node.end_byte())
}

fn is_test_attribute(attribute: &str) -> bool {
    let inner = attribute
        .trim()
        .trim_start_matches("#[")
        .trim_end_matches(']')
        .trim();
    let words = |text: &str| -> Vec<String> {
        text.split(|c: char| !(c.is_alphanumeric() || c == '_'))
            .filter(|word| !word.is_empty())
            .map(str::to_string)
            .collect()
    };
    if inner.starts_with("cfg") {
        return words(inner).iter().any(|word| word == "test");
    }
    let path = inner.split('(').next().unwrap_or(inner);
    let last = path.rsplit("::").next().unwrap_or(path).trim();
    last == "test" || last == "rstest" || last.starts_with("test_")
}

/// A statement that is nothing but a call to a logging function.
fn is_logging(lang: Lang, node: Node, src: &str) -> bool {
    let call = match lang {
        // Swift and Kotlin have no expression statements: a call sits directly in the
        // statement list.
        Lang::Swift | Lang::Kotlin => (node.kind() == "call_expression"
            && node
                .parent()
                .is_some_and(|p| matches!(p.kind(), "statements" | "source_file" | "block")))
        .then_some(node),
        _ => (node.kind() == "expression_statement")
            .then(|| node.named_child(0))
            .flatten()
            .filter(|child| {
                matches!(
                    child.kind(),
                    "call_expression" | "call" | "macro_invocation"
                )
            }),
    };
    let Some(call) = call else {
        return false;
    };
    let callee = match (lang, call.kind()) {
        (Lang::Swift | Lang::Kotlin, _) => call.named_child(0),
        (_, "macro_invocation") => call.child_by_field_name("macro"),
        _ => call.child_by_field_name("function"),
    };
    callee
        .and_then(|callee| src.get(callee.byte_range()))
        .is_some_and(|callee| is_logging_callee(lang, callee))
}

fn is_logging_callee(lang: Lang, callee: &str) -> bool {
    let callee: String = callee.chars().filter(|c| !c.is_whitespace()).collect();
    let segments: Vec<&str> = callee
        .split(['.', '?', '!', ':'])
        .filter(|s| !s.is_empty())
        .collect();
    let Some((&method, receivers)) = segments.split_last() else {
        return false;
    };
    if receivers.is_empty() {
        return match lang {
            Lang::Swift => matches!(method, "print" | "debugPrint" | "NSLog" | "os_log" | "dump"),
            Lang::Python => method == "print",
            Lang::Go | Lang::Kotlin => matches!(method, "print" | "println"),
            Lang::Rust => matches!(
                method,
                "println"
                    | "eprintln"
                    | "print"
                    | "eprint"
                    | "dbg"
                    | "trace"
                    | "debug"
                    | "info"
                    | "warn"
                    | "error"
            ),
            _ => false,
        };
    }
    let receiver = receivers
        .last()
        .map(|receiver| receiver.to_ascii_lowercase())
        .unwrap_or_default();
    let level = method.to_ascii_lowercase();
    match lang {
        // `fmt.Fprint*` writes to a writer (perhaps a response), and `log.Fatal*` and
        // `log.Panic*` end the program: those are code, not logging.
        Lang::Go if receiver == "fmt" => matches!(method, "Print" | "Println" | "Printf"),
        Lang::Go if receiver == "log" => method.starts_with("Print"),
        Lang::Go if level.starts_with("fatal") || level.starts_with("panic") => false,
        // Android's `Log.d(…)` and Timber's `Timber.w(…)` use one-letter levels.
        Lang::Kotlin if receiver == "log" || receiver == "timber" => {
            matches!(method, "d" | "i" | "w" | "e" | "v" | "wtf") || is_level(&level)
        }
        _ => is_level(&level) && receivers.iter().any(|receiver| is_logger(receiver)),
    }
}

/// A logging level, including Go's formatted variants (`Infof`, `Debugw`).
fn is_level(level: &str) -> bool {
    const LEVELS: [&str; 13] = [
        "log",
        "debug",
        "info",
        "notice",
        "warn",
        "warning",
        "error",
        "fault",
        "trace",
        "critical",
        "exception",
        "verbose",
        "fatal",
    ];
    LEVELS.contains(&level)
        || level
            .strip_suffix(['f', 'w'])
            .is_some_and(|stem| LEVELS.contains(&stem))
}

fn is_logger(receiver: &str) -> bool {
    let receiver = receiver.to_ascii_lowercase();
    matches!(
        receiver.as_str(),
        "console" | "logging" | "log" | "tracing" | "slog" | "timber"
    ) || receiver.ends_with("logger")
}

fn first_error(root: Node) -> Option<Node> {
    if !root.has_error() {
        return None;
    }
    // Go as deep as the error goes: the parser often wraps everything from the start of
    // the file in one ERROR node, and the line that matters is the innermost one.
    let mut node = root;
    'descend: loop {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.has_error() || child.is_missing() {
                node = child;
                continue 'descend;
            }
        }
        return Some(node);
    }
}
