use std::ops::Range;
use std::sync::OnceLock;

use tree_sitter_highlight::{HighlightConfiguration, HighlightEvent, Highlighter};

use crate::lang::Lang;
use crate::project::Projection;

/// The syntax categories the viewer colours.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    Keyword,
    String,
    Comment,
    Number,
    Constant,
    Type,
    Function,
    Attribute,
}

/// Capture names handed to tree-sitter-highlight; a query capture such as
/// `function.method` resolves to the longest matching prefix here.
const NAMES: [(&str, Class); 14] = [
    ("keyword", Class::Keyword),
    ("include", Class::Keyword),
    ("conditional", Class::Keyword),
    ("repeat", Class::Keyword),
    ("string", Class::String),
    ("comment", Class::Comment),
    ("number", Class::Number),
    ("float", Class::Number),
    ("boolean", Class::Constant),
    ("constant", Class::Constant),
    ("type", Class::Type),
    ("constructor", Class::Type),
    ("function", Class::Function),
    ("attribute", Class::Attribute),
];

/// Syntax classes per line, as byte ranges within that line.
pub type LineClasses = Vec<(Range<usize>, Class)>;

/// Colours a projection by highlighting its own text, so what is coloured is exactly
/// what is shown — comments removed, or only comments — not the original file.
pub fn annotate(projection: &mut Projection, lang: Lang) {
    let Some(config) = config(lang) else {
        return;
    };
    let source = projection.lines.join("\n");
    let mut starts = Vec::with_capacity(projection.lines.len());
    let mut offset = 0;
    for line in &projection.lines {
        starts.push(offset);
        offset += line.len() + 1;
    }

    let mut classes: Vec<LineClasses> = vec![Vec::new(); projection.lines.len()];
    let mut highlighter = Highlighter::new();
    let Ok(events) = highlighter.highlight(config, source.as_bytes(), None, None, |_| None) else {
        return;
    };
    let mut active: Vec<Class> = Vec::new();
    for event in events {
        match event {
            Ok(HighlightEvent::HighlightStart(highlight)) => active.push(NAMES[highlight.0].1),
            Ok(HighlightEvent::HighlightEnd) => {
                active.pop();
            }
            Ok(HighlightEvent::Source { start, end }) => {
                let Some(&class) = active.last() else {
                    continue;
                };
                // Split the span over the lines it crosses.
                let mut line = starts.partition_point(|&s| s <= start).saturating_sub(1);
                let mut from = start;
                while from < end && line < starts.len() {
                    let line_end = starts[line] + projection.lines[line].len();
                    let to = end.min(line_end);
                    if to > from {
                        let span = from - starts[line]..to - starts[line];
                        match classes[line].last_mut() {
                            // Grammars split some tokens (a string and its quotes); join them back.
                            Some((last, last_class))
                                if last.end == span.start && *last_class == class =>
                            {
                                last.end = span.end
                            }
                            _ => classes[line].push((span, class)),
                        }
                    }
                    line += 1;
                    from = starts.get(line).copied().unwrap_or(end);
                }
            }
            Err(_) => return,
        }
    }
    projection.syntax = classes;
}

fn config(lang: Lang) -> Option<&'static HighlightConfiguration> {
    static SWIFT: OnceLock<Option<HighlightConfiguration>> = OnceLock::new();
    static TYPESCRIPT: OnceLock<Option<HighlightConfiguration>> = OnceLock::new();
    static TSX: OnceLock<Option<HighlightConfiguration>> = OnceLock::new();
    static PYTHON: OnceLock<Option<HighlightConfiguration>> = OnceLock::new();

    let build = |query: String| {
        let mut config =
            HighlightConfiguration::new(lang.grammar(), lang.name(), &query, "", "").ok()?;
        let names: Vec<&str> = NAMES.iter().map(|(name, _)| *name).collect();
        config.configure(&names);
        Some(config)
    };
    // The TypeScript queries only add to the JavaScript ones, so both are needed.
    let ecma = || {
        format!(
            "{}\n{}",
            tree_sitter_typescript::HIGHLIGHTS_QUERY,
            tree_sitter_javascript::HIGHLIGHT_QUERY
        )
    };
    let cell = match lang {
        Lang::Swift => &SWIFT,
        Lang::TypeScript => &TYPESCRIPT,
        Lang::Tsx | Lang::JavaScript => &TSX,
        Lang::Python => &PYTHON,
    };
    cell.get_or_init(|| match lang {
        Lang::Swift => build(tree_sitter_swift::HIGHLIGHTS_QUERY.to_string()),
        Lang::TypeScript => build(ecma()),
        Lang::Tsx | Lang::JavaScript => build(format!(
            "{}\n{}",
            ecma(),
            tree_sitter_javascript::JSX_HIGHLIGHT_QUERY
        )),
        Lang::Python => build(tree_sitter_python::HIGHLIGHTS_QUERY.to_string()),
    })
    .as_ref()
}
