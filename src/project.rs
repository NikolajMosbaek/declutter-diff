use std::ops::Range;

/// What the reviewer sees of the comment layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommentMode {
    Shown,
    Hidden,
    Only,
}

impl CommentMode {
    pub const ALL: [CommentMode; 3] = [CommentMode::Shown, CommentMode::Hidden, CommentMode::Only];

    pub fn next(self) -> CommentMode {
        match self {
            CommentMode::Hidden => CommentMode::Only,
            CommentMode::Only => CommentMode::Shown,
            CommentMode::Shown => CommentMode::Hidden,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            CommentMode::Shown => "shown",
            CommentMode::Hidden => "hidden",
            CommentMode::Only => "only",
        }
    }

    pub fn parse(value: &str) -> Option<CommentMode> {
        CommentMode::ALL
            .into_iter()
            .find(|mode| mode.label() == value)
    }

    pub(crate) fn index(self) -> usize {
        match self {
            CommentMode::Shown => 0,
            CommentMode::Hidden => 1,
            CommentMode::Only => 2,
        }
    }
}

/// A file as seen through a comment mode: the lines that remain, each tied back
/// to the line it came from so line numbers always refer to the real file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Projection {
    pub lines: Vec<String>,
    /// Zero-based original line index of each entry in `lines`.
    pub orig: Vec<usize>,
}

pub fn project(src: &str, comments: &[Range<usize>], mode: CommentMode) -> Projection {
    let mut projection = Projection::default();
    let mut next_comment = 0;
    let mut line_start = 0;

    for (index, raw) in src.split_inclusive('\n').enumerate() {
        let line_end = line_start + raw.len();
        let line = raw.trim_end_matches('\n').trim_end_matches('\r');

        while next_comment < comments.len() && comments[next_comment].end <= line_start {
            next_comment += 1;
        }
        // An empty cut marks a blank line inside a multi-line comment: it belongs to the comment.
        let cuts: Vec<Range<usize>> = comments[next_comment..]
            .iter()
            .take_while(|comment| comment.start < line_end)
            .map(|comment| {
                let end = comment.end.min(line_start + line.len()) - line_start;
                (comment.start.max(line_start) - line_start).min(end)..end
            })
            .collect();

        let kept = match mode {
            CommentMode::Shown => Some(line.to_string()),
            CommentMode::Hidden => without_cuts(line, &cuts),
            CommentMode::Only => only_cuts(line, &cuts),
        };
        if let Some(text) = kept {
            projection.lines.push(text);
            projection.orig.push(index);
        }
        line_start = line_end;
    }
    projection
}

/// The line with its comments removed, or `None` if nothing but comments was on it.
fn without_cuts(line: &str, cuts: &[Range<usize>]) -> Option<String> {
    if cuts.is_empty() {
        return Some(line.to_string());
    }
    let mut out = line[..cuts[0].start].to_string();
    for (cut, next) in cuts.iter().zip(cuts.iter().skip(1)) {
        push_joined(&mut out, &line[cut.end..next.start]);
    }
    push_joined(&mut out, &line[cuts[cuts.len() - 1].end..]);

    let trimmed = out.trim_end();
    (!trimmed.trim_start().is_empty()).then(|| trimmed.to_string())
}

/// Appends the text after a cut without doubling the whitespace around it. When the cut
/// started the line, the line's own indentation went with it, so none is added back.
fn push_joined(out: &mut String, segment: &str) {
    if out.is_empty() || out.ends_with(char::is_whitespace) {
        out.push_str(segment.trim_start());
    } else {
        out.push_str(segment);
    }
}

/// Only the comment text of the line, keeping its indentation, or `None` if it has no comment.
fn only_cuts(line: &str, cuts: &[Range<usize>]) -> Option<String> {
    let first = cuts.first()?;
    let indent = &line[..line.len() - line.trim_start().len()];
    let mut out = if first.start == 0 {
        String::new()
    } else {
        indent.to_string()
    };
    for (i, cut) in cuts.iter().enumerate() {
        let text = &line[cut.clone()];
        if i == 0 && cut.start == 0 {
            out.push_str(text);
        } else {
            if i > 0 {
                out.push(' ');
            }
            out.push_str(text.trim());
        }
    }
    Some(out.trim_end().to_string())
}
