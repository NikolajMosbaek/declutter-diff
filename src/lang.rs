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
