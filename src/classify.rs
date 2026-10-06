use std::ops::Range;

use tree_sitter::{Node, Parser};

use crate::lang::Lang;

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
    /// The parser recovered from syntax errors. Comments are still collected,
    /// but some may be missed or misplaced near the error.
    pub partial: bool,
}

/// Finds every comment, import and logging statement in `src`. Returns `None` only if
/// the parser could not run at all.
pub fn classify(lang: Lang, src: &str) -> Option<Classified> {
    let mut parser = Parser::new();
    parser.set_language(&lang.grammar()).ok()?;
    let tree = parser.parse(src, None)?;

    // Comments are tree-sitter "extras": they can appear at any depth, so walk every node.
    let mut classified = Classified::default();
    let mut cursor = tree.walk();
    'walk: loop {
        let node = cursor.node();
        let layer = if lang.is_comment(node.kind()) || (lang == Lang::Python && is_docstring(node))
        {
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
        } else if cursor.goto_first_child() {
            continue;
        }
        while !cursor.goto_next_sibling() {
            if !cursor.goto_parent() {
                break 'walk;
            }
        }
    }

    classified.partial = tree.root_node().has_error();
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
        Lang::Swift => node.kind() == "import_declaration",
        Lang::TypeScript | Lang::Tsx | Lang::JavaScript => {
            node.kind() == "import_statement"
                || (node.kind() == "export_statement"
                    && node.child_by_field_name("source").is_some())
        }
        Lang::Python => matches!(
            node.kind(),
            "import_statement" | "import_from_statement" | "future_import_statement"
        ),
    }
}

/// A statement that is nothing but a call to a logging function.
fn is_logging(lang: Lang, node: Node, src: &str) -> bool {
    let call = match lang {
        // Swift has no expression statements: a call sits directly in the statement list.
        Lang::Swift => (node.kind() == "call_expression"
            && node
                .parent()
                .is_some_and(|p| matches!(p.kind(), "statements" | "source_file")))
        .then_some(node),
        _ => (node.kind() == "expression_statement")
            .then(|| node.named_child(0))
            .flatten()
            .filter(|child| matches!(child.kind(), "call_expression" | "call")),
    };
    let Some(call) = call else {
        return false;
    };
    let callee = match lang {
        Lang::Swift => call.named_child(0),
        _ => call.child_by_field_name("function"),
    };
    callee
        .and_then(|callee| src.get(callee.byte_range()))
        .is_some_and(|callee| is_logging_callee(lang, callee))
}

fn is_logging_callee(lang: Lang, callee: &str) -> bool {
    let callee: String = callee.chars().filter(|c| !c.is_whitespace()).collect();
    let segments: Vec<&str> = callee
        .split(['.', '?', '!'])
        .filter(|s| !s.is_empty())
        .collect();
    let Some((&method, receivers)) = segments.split_last() else {
        return false;
    };
    if receivers.is_empty() {
        return match lang {
            Lang::Swift => matches!(method, "print" | "debugPrint" | "NSLog" | "os_log" | "dump"),
            Lang::Python => method == "print",
            _ => false,
        };
    }
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
    LEVELS.contains(&method)
        && receivers.iter().any(|receiver| {
            let receiver = receiver.to_ascii_lowercase();
            receiver == "console"
                || receiver == "logging"
                || receiver == "log"
                || receiver.ends_with("logger")
        })
}
