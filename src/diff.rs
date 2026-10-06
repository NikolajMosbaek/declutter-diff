use std::ops::Range;

use similar::{Algorithm, DiffTag, capture_diff_slices, group_diff_ops};

use crate::highlight::LineClasses;
use crate::project::{LayerMode, Projection};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowKind {
    Context,
    Removed,
    Added,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub kind: RowKind,
    /// One-based line number in the original old file.
    pub old_line: Option<usize>,
    /// One-based line number in the original new file.
    pub new_line: Option<usize>,
    pub text: String,
    /// Byte ranges of `text` that differ from the paired line on the other side.
    pub emphasis: Vec<Range<usize>>,
    pub syntax: LineClasses,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hunk {
    pub rows: Vec<Row>,
}

impl Hunk {
    pub fn changed(&self) -> impl Iterator<Item = &Row> {
        self.rows.iter().filter(|row| row.kind != RowKind::Context)
    }

    pub fn header(&self) -> String {
        let old = self.rows.iter().find_map(|row| row.old_line);
        let new = self.rows.iter().find_map(|row| row.new_line);
        let fmt = |line: Option<usize>| line.map_or("0".to_string(), |n| n.to_string());
        format!("@@ -{} +{} @@", fmt(old), fmt(new))
    }
}

/// Diffs two projections line by line and groups the result into hunks with
/// `context` unchanged lines around each change.
pub fn diff(old: &Projection, new: &Projection, context: usize) -> Vec<Hunk> {
    let ops = capture_diff_slices(Algorithm::Histogram, &old.lines, &new.lines);
    let old_row = |i: usize, kind| Row {
        kind,
        old_line: Some(old.orig[i] + 1),
        new_line: None,
        text: old.lines[i].clone(),
        emphasis: Vec::new(),
        syntax: old.syntax.get(i).cloned().unwrap_or_default(),
    };
    let new_row = |j: usize| Row {
        kind: RowKind::Added,
        old_line: None,
        new_line: Some(new.orig[j] + 1),
        text: new.lines[j].clone(),
        emphasis: Vec::new(),
        syntax: new.syntax.get(j).cloned().unwrap_or_default(),
    };

    group_diff_ops(ops, context)
        .into_iter()
        .map(|group| {
            let mut rows = Vec::new();
            for op in group {
                let (tag, old_range, new_range) = op.as_tag_tuple();
                match tag {
                    DiffTag::Equal => {
                        for (i, j) in old_range.zip(new_range) {
                            rows.push(Row {
                                new_line: Some(new.orig[j] + 1),
                                ..old_row(i, RowKind::Context)
                            });
                        }
                    }
                    DiffTag::Delete => rows.extend(old_range.map(|i| old_row(i, RowKind::Removed))),
                    DiffTag::Insert => rows.extend(new_range.map(new_row)),
                    DiffTag::Replace => {
                        let removed_at = rows.len();
                        let pairs = old_range.len().min(new_range.len());
                        rows.extend(old_range.map(|i| old_row(i, RowKind::Removed)));
                        let added_at = rows.len();
                        rows.extend(new_range.map(new_row));
                        // Lines replaced one-for-one are usually edits of each other.
                        for k in 0..pairs {
                            let (old_text, new_text) =
                                (&rows[removed_at + k].text, &rows[added_at + k].text);
                            if let Some((old_marks, new_marks)) = inline_changes(old_text, new_text)
                            {
                                rows[removed_at + k].emphasis = old_marks;
                                rows[added_at + k].emphasis = new_marks;
                            }
                        }
                    }
                }
            }
            Hunk { rows }
        })
        .collect()
}

/// Byte ranges within a line.
pub type Marks = Vec<Range<usize>>;

/// The parts of two versions of a line that differ, compared token by token, or `None`
/// when the lines have too little in common for highlighting the difference to help.
pub fn inline_changes(old: &str, new: &str) -> Option<(Marks, Marks)> {
    let (old_tokens, new_tokens) = (tokens(old), tokens(new));
    let old_text: Vec<&str> = old_tokens.iter().map(|range| &old[range.clone()]).collect();
    let new_text: Vec<&str> = new_tokens.iter().map(|range| &new[range.clone()]).collect();

    let mut old_marks: Vec<Range<usize>> = Vec::new();
    let mut new_marks: Vec<Range<usize>> = Vec::new();
    for op in capture_diff_slices(Algorithm::Myers, &old_text, &new_text) {
        let (tag, old_range, new_range) = op.as_tag_tuple();
        if tag == DiffTag::Equal {
            continue;
        }
        if let Some(span) = span_of(&old_tokens, old_range) {
            push_merged(&mut old_marks, span);
        }
        if let Some(span) = span_of(&new_tokens, new_range) {
            push_merged(&mut new_marks, span);
        }
    }

    let changed = |marks: &[Range<usize>], line: &str| {
        let line = line.trim();
        let bytes: usize = marks.iter().map(|m| m.len()).sum();
        line.is_empty() || bytes * 10 > line.len() * 6
    };
    if changed(&old_marks, old) && changed(&new_marks, new) {
        return None;
    }
    Some((old_marks, new_marks))
}

/// Splits a line into identifier/number runs, whitespace runs and single punctuation.
pub(crate) fn tokens(line: &str) -> Vec<Range<usize>> {
    let class = |c: char| {
        if c.is_alphanumeric() || c == '_' {
            0
        } else if c.is_whitespace() {
            1
        } else {
            2
        }
    };
    let mut out: Vec<Range<usize>> = Vec::new();
    let mut previous = None;
    for (i, c) in line.char_indices() {
        let kind = class(c);
        match out.last_mut() {
            Some(last) if previous == Some(kind) && kind != 2 => last.end = i + c.len_utf8(),
            _ => out.push(i..i + c.len_utf8()),
        }
        previous = Some(kind);
    }
    out
}

fn span_of(tokens: &[Range<usize>], range: Range<usize>) -> Option<Range<usize>> {
    if range.is_empty() {
        return None;
    }
    Some(tokens[range.start].start..tokens[range.end - 1].end)
}

fn push_merged(marks: &mut Vec<Range<usize>>, span: Range<usize>) {
    match marks.last_mut() {
        Some(last) if last.end >= span.start => last.end = last.end.max(span.end),
        _ => marks.push(span),
    }
}

/// Applies the formatting layer to hunks: finds changes that only move whitespace —
/// re-indenting, re-spacing, re-wrapping a statement over more or fewer lines — and
/// either turns them into context (hidden) or keeps nothing else (only).
pub fn filter_formatting(hunks: &mut Vec<Hunk>, mode: LayerMode, indentation_matters: bool) {
    if mode == LayerMode::Shown {
        return;
    }
    for hunk in hunks.iter_mut() {
        let formatting = formatting_rows(&hunk.rows, indentation_matters);
        let rows = std::mem::take(&mut hunk.rows);
        hunk.rows = rows
            .into_iter()
            .zip(formatting)
            .filter_map(
                |(row, is_formatting)| match (mode, row.kind, is_formatting) {
                    (_, RowKind::Context, _) => Some(row),
                    // The new layout stays visible as context, so the code is not missing.
                    (LayerMode::Hidden, RowKind::Added, true) => Some(Row {
                        kind: RowKind::Context,
                        old_line: None,
                        emphasis: Vec::new(),
                        ..row
                    }),
                    (LayerMode::Hidden, _, true) => None,
                    (LayerMode::Only, _, false) => None,
                    _ => Some(row),
                },
            )
            .collect();
    }
    hunks.retain(|hunk| hunk.changed().next().is_some());
}

/// For each row, whether it belongs to a formatting-only change. A run of changed rows
/// counts as a whole when its removed and added lines hold the same tokens; otherwise
/// lines replaced one-for-one are compared pairwise.
fn formatting_rows(rows: &[Row], indentation_matters: bool) -> Vec<bool> {
    let mut marks = vec![false; rows.len()];
    let mut start = 0;
    while start < rows.len() {
        if rows[start].kind == RowKind::Context {
            start += 1;
            continue;
        }
        let end = (start..rows.len())
            .find(|&i| rows[i].kind == RowKind::Context)
            .unwrap_or(rows.len());
        let removed: Vec<usize> = (start..end)
            .filter(|&i| rows[i].kind == RowKind::Removed)
            .collect();
        let added: Vec<usize> = (start..end)
            .filter(|&i| rows[i].kind == RowKind::Added)
            .collect();
        let stream = |indices: &[usize]| -> Vec<String> {
            indices
                .iter()
                .flat_map(|&i| significant_tokens(&rows[i].text, indentation_matters))
                .collect()
        };
        if !(removed.is_empty() && added.is_empty()) {
            // Equal streams include added or removed blank lines: layout only.
            if stream(&removed) == stream(&added) {
                marks[start..end].iter_mut().for_each(|mark| *mark = true);
            } else if removed.len() == added.len() {
                for (&old, &new) in removed.iter().zip(&added) {
                    if stream(&[old]) == stream(&[new]) {
                        marks[old] = true;
                        marks[new] = true;
                    }
                }
            }
        }
        start = end;
    }
    marks
}

/// The tokens of a line that whitespace changes cannot affect, plus its indentation
/// when indentation is part of the syntax.
fn significant_tokens(line: &str, indentation_matters: bool) -> Vec<String> {
    let mut out = Vec::new();
    if indentation_matters && !line.trim().is_empty() {
        out.push(format!(
            "indent:{}",
            &line[..line.len() - line.trim_start().len()]
        ));
    }
    out.extend(
        tokens(line)
            .into_iter()
            .map(|range| &line[range])
            .filter(|token| !token.trim().is_empty())
            .map(str::to_string),
    );
    out
}
