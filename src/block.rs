//! Building a note on a block of lines from a file's diff.

use anyhow::{Result, bail};

use crate::diff::{Row, RowKind};
use crate::review::{DiffModes, FileReview};
use crate::store::{End, Note, NoteSide, ranges};

/// Whether `row` is the row `end` points at: a removed row by its old line, an added or
/// unchanged row by its new line.
fn is_at(row: &Row, end: End) -> bool {
    match (end.0, row.kind) {
        (NoteSide::Old, RowKind::Removed) => row.old_line == Some(end.1),
        (NoteSide::New, RowKind::Added | RowKind::Context) => row.new_line == Some(end.1),
        _ => false,
    }
}

/// The lowest and highest of `lines`, if there are any.
fn span(lines: impl Iterator<Item = usize>) -> Option<(usize, usize)> {
    lines.fold(None, |span, n| match span {
        None => Some((n, n)),
        Some((low, high)) => Some((low.min(n), high.max(n))),
    })
}

/// Lines `first..=last` of one version of the file, as they are there.
fn source_lines(file: &FileReview, side: NoteSide, first: usize, last: usize) -> String {
    file.source(side == NoteSide::Old)
        .lines()
        .skip(first.saturating_sub(1))
        .take((last + 1).saturating_sub(first))
        .collect::<Vec<_>>()
        .join("\n")
}

impl Note {
    /// A note on the lines from `first` to `last` of `file`, quoting what they say. The
    /// ends are looked up in the file's full diff; a block that takes in removed lines
    /// along with others is quoted as a diff and records the lines it covers on each
    /// side, any other block quotes the version it is on. An end that is not in the
    /// diff — the viewer can pick one its decluttered view shows but the full diff does
    /// not — makes the block a single-sided one, quoted from the first end's version.
    /// Never fails, and is not a draft; one line is a line note.
    pub fn block(file: &FileReview, first: End, last: End, text: impl Into<String>) -> Note {
        let view = file.view(DiffModes::SHOWN);
        let rows: Vec<&Row> = view.hunks.iter().flat_map(|hunk| &hunk.rows).collect();
        let start = rows.iter().position(|row| is_at(row, first));
        let end = rows.iter().rposition(|row| is_at(row, last));
        let covered = match (start, end) {
            (Some(start), Some(end)) if start <= end => &rows[start..=end],
            _ => &[],
        };

        let removed = covered.iter().any(|row| row.kind == RowKind::Removed);
        let others = covered.iter().any(|row| row.kind != RowKind::Removed);
        let mut note = Note {
            path: file.path.clone(),
            side: first.0,
            line: first.1,
            end_side: None,
            end_line: None,
            old_lines: None,
            new_lines: None,
            code: String::new(),
            text: text.into().trim().to_string(),
            review: None,
            draft: false,
        };
        if first == last {
            note.code = match covered.first() {
                Some(row) => row.text.clone(),
                None => source_lines(file, first.0, first.1, first.1),
            };
            return note;
        }
        note.end_line = Some(last.1);
        if removed && others {
            note.end_side = (last.0 != first.0).then_some(last.0);
            note.old_lines = span(
                covered
                    .iter()
                    .filter(|row| row.kind == RowKind::Removed)
                    .filter_map(|row| row.old_line),
            );
            note.new_lines = span(
                covered
                    .iter()
                    .filter(|row| row.kind != RowKind::Removed)
                    .filter_map(|row| row.new_line),
            );
            note.code = covered
                .iter()
                .map(|row| {
                    let sign = match row.kind {
                        RowKind::Removed => '-',
                        RowKind::Added => '+',
                        RowKind::Context => ' ',
                    };
                    format!("{sign}{}", row.text)
                })
                .collect::<Vec<_>>()
                .join("\n");
        } else {
            note.code = source_lines(file, first.0, first.1, last.1);
        }
        note
    }

    /// A draft note on lines `first` to `last` of `path`, all on `side` and in one hunk
    /// of the change's diff — with every layer shown — since that is where the viewer
    /// can show it. A removed line is on the old side; added and unchanged lines are on
    /// the new side. One line is a line note.
    pub fn on_block(
        files: &[FileReview],
        path: &str,
        side: NoteSide,
        first: usize,
        last: usize,
        text: impl Into<String>,
    ) -> Result<Note> {
        let Some(file) = files.iter().find(|file| file.path == path) else {
            bail!("`{path}` is not part of this change");
        };
        let one = first == last;
        let (subject, which) = match (side, one) {
            (NoteSide::New, true) => (format!("`{path}:{first}` is"), "the lines"),
            (NoteSide::New, false) => (format!("`{path}:{first}–{last}` is"), "the lines"),
            (NoteSide::Old, true) => (
                format!("removed line {first} of `{path}` is"),
                "the removed lines",
            ),
            (NoteSide::Old, false) => (
                format!("removed lines {first}–{last} of `{path}` are"),
                "the removed lines",
            ),
        };
        if first > last {
            bail!("`{path}:{first}–{last}` ends before it starts");
        }
        let view = file.view(DiffModes::SHOWN);
        // Each line on this side, with the hunk it is in.
        let lines: Vec<(usize, usize)> = view
            .hunks
            .iter()
            .enumerate()
            .flat_map(|(i, hunk)| hunk.rows.iter().map(move |row| (i, row)))
            .filter_map(|(i, row)| match (side, row.kind) {
                (NoteSide::Old, RowKind::Removed) => Some((i, row.old_line?)),
                (NoteSide::New, RowKind::Added | RowKind::Context) => Some((i, row.new_line?)),
                _ => None,
            })
            .collect();
        let hunk_of = |line: usize| lines.iter().find(|(_, n)| *n == line).map(|(i, _)| *i);
        let (Some(a), Some(b)) = (hunk_of(first), hunk_of(last)) else {
            return Err(not_in_diff(&subject, which, &lines));
        };
        if a != b {
            return Err(not_in_diff(&subject, which, &lines));
        }
        let mut note = Note::block(file, (side, first), (side, last), text);
        note.draft = true;
        Ok(note)
    }
}

/// The refusal for a range (`subject` ends in its verb) that is not in the diff, naming the lines that are.
fn not_in_diff(subject: &str, which: &str, lines: &[(usize, usize)]) -> anyhow::Error {
    if lines.is_empty() {
        return anyhow::anyhow!("{subject} not in the diff, which has no such lines");
    }
    let numbers: Vec<usize> = lines.iter().map(|(_, n)| *n).collect();
    anyhow::anyhow!(
        "{subject} not in the diff; {which} in it are {}",
        ranges(&numbers)
    )
}
