use std::path::Path;

/// Whether a changed file belongs to the Markdown layer: `.md` or `.markdown`, in any case.
pub fn is_markdown_file(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("md") || extension.eq_ignore_ascii_case("markdown")
        })
}
