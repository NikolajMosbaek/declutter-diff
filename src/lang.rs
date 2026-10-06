use std::borrow::Cow;
use std::path::Path;

use tree_sitter::Language;

/// A language whose comments declutter can find.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Swift,
    TypeScript,
    Tsx,
    /// Parsed with the TSX grammar, which accepts plain JavaScript and JSX.
    JavaScript,
    Python,
}

impl Lang {
    pub fn from_path(path: &str) -> Option<Lang> {
        let ext = Path::new(path).extension()?.to_str()?;
        match ext {
            "swift" => Some(Lang::Swift),
            "ts" | "mts" | "cts" => Some(Lang::TypeScript),
            "tsx" => Some(Lang::Tsx),
            "js" | "mjs" | "cjs" | "jsx" => Some(Lang::JavaScript),
            "py" | "pyi" => Some(Lang::Python),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Lang::Swift => "Swift",
            Lang::TypeScript => "TypeScript",
            Lang::Tsx => "TSX",
            Lang::JavaScript => "JavaScript",
            Lang::Python => "Python",
        }
    }

    pub fn grammar(self) -> Language {
        match self {
            Lang::Swift => tree_sitter_swift::LANGUAGE.into(),
            Lang::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            Lang::Tsx | Lang::JavaScript => tree_sitter_typescript::LANGUAGE_TSX.into(),
            Lang::Python => tree_sitter_python::LANGUAGE.into(),
        }
    }

    pub fn is_comment(self, kind: &str) -> bool {
        match self {
            Lang::Swift => matches!(kind, "comment" | "multiline_comment"),
            Lang::TypeScript | Lang::Tsx | Lang::JavaScript => {
                matches!(kind, "comment" | "html_comment")
            }
            Lang::Python => kind == "comment",
        }
    }
}

/// The text handed to the parser in place of `src`: always the same length, byte for
/// byte, so every range found in it is a range in `src` too.
///
/// It papers over constructs tree-sitter-swift 0.7 cannot parse, each of which leaves
/// an error that can swallow the rest of a function:
///
/// - the empty tuple `()` used as a value (`.success(())`, `resume(returning: ())`,
///   `value ?? ()`) becomes the empty array literal `[]`, which parses there;
/// - `await` opening an `if`/`while`/`guard` condition (`if await a != b`) is blanked;
/// - `nonisolated(unsafe)` loses its `(unsafe)`.
///
/// None of these touch a comment, an import or a logging call's extent.
pub fn parsable(lang: Lang, src: &str) -> Cow<'_, str> {
    if lang != Lang::Swift {
        return Cow::Borrowed(src);
    }
    let mut out = src.as_bytes().to_vec();
    let mut blank = |range: std::ops::Range<usize>, with: &[u8]| {
        out[range.clone()].copy_from_slice(&with[..range.len()]);
    };

    for (at, _) in src.match_indices("()") {
        let before = src[..at].trim_end();
        let after = src[at + 2..].trim_start();
        let value_position = matches!(before.chars().last(), Some('(' | ',' | ':' | '=' | '?'))
            && !before.ends_with("->")
            && !["->", "throws", "async"]
                .iter()
                .any(|word| after.starts_with(word));
        if value_position {
            blank(at..at + 2, b"[]");
        }
    }
    for (at, _) in src.match_indices("await ") {
        let before = src[..at].trim_end();
        let opens_condition = ["if", "while", "guard"].iter().any(|keyword| {
            before.ends_with(keyword)
                && !before[..before.len() - keyword.len()]
                    .ends_with(|c: char| c.is_alphanumeric() || c == '_')
        });
        if opens_condition {
            blank(at..at + 5, b"     ");
        }
    }
    for (at, _) in src.match_indices("nonisolated(unsafe)") {
        blank(at + 11..at + 19, b"        ");
    }

    if out == src.as_bytes() {
        return Cow::Borrowed(src);
    }
    // Only ASCII bytes were swapped for ASCII bytes, so the text is still valid UTF-8.
    Cow::Owned(String::from_utf8(out).unwrap_or_else(|_| src.to_string()))
}
