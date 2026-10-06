use std::ops::Range;

use tree_sitter::{Node, Parser};

use crate::lang::Lang;

/// The comment layer of one version of a file.
#[derive(Debug, Clone, Default)]
pub struct Classified {
    /// Byte ranges of comments and docstrings, in document order, non-overlapping.
    pub comments: Vec<Range<usize>>,
    /// The parser recovered from syntax errors. Comments are still collected,
    /// but some may be missed or misplaced near the error.
    pub partial: bool,
}

/// Finds every comment in `src`. Returns `None` only if the parser could not run at all.
pub fn classify(lang: Lang, src: &str) -> Option<Classified> {
    let mut parser = Parser::new();
    parser.set_language(&lang.grammar()).ok()?;
    let tree = parser.parse(src, None)?;

    // Comments are tree-sitter "extras": they can appear at any depth, so walk every node.
    let mut comments = Vec::new();
    let mut cursor = tree.walk();
    'walk: loop {
        let node = cursor.node();
        let is_comment =
            lang.is_comment(node.kind()) || (lang == Lang::Python && is_docstring(node));
        if is_comment {
            comments.push(node.byte_range());
        } else if cursor.goto_first_child() {
            continue;
        }
        while !cursor.goto_next_sibling() {
            if !cursor.goto_parent() {
                break 'walk;
            }
        }
    }

    Some(Classified {
        comments,
        partial: tree.root_node().has_error(),
    })
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
