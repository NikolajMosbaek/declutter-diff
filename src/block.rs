//! Building a note on a block of lines from a file's diff.

use anyhow::{Result, bail};

use crate::diff::{Row, RowKind};
use crate::review::{DiffModes, FileReview, ModeView};
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

/// The rows of `view` from the row `first` points at to the row `last` points at, in
/// diff order, and whether the ends came the other way round; `None` unless both are
/// rows of it.
fn rows_between(view: &ModeView, first: End, last: End) -> Option<(Vec<&Row>, bool)> {
    let rows: Vec<&Row> = view.hunks.iter().flat_map(|hunk| &hunk.rows).collect();
    let start = rows.iter().position(|row| is_at(row, first))?;
    let end = rows.iter().rposition(|row| is_at(row, last))?;
    Some(if start <= end {
        (rows[start..=end].to_vec(), false)
    } else {
        (rows[end..=start].to_vec(), true)
    })
}

impl Note {
    /// A note on the lines from `first` to `last` of `file`, picked in the view of it
    /// that `modes` gives, quoting what they say. The block is the rows between its ends
    /// in the full diff, so lines a layer hides are part of it; when an end is not in the
    /// full diff — a decluttered view shows more context than the full diff — it is the
    /// rows between them in the view they were picked in, so it is saved as selected. A
    /// block that takes in removed lines along with others is quoted as a diff and
    /// records the lines it covers on each side, any other block quotes the version it
    /// is on. Ends given out of diff order are swapped, so the note runs from first to
    /// last as the diff reads. Never fails, and is not a draft; one line is a line note.
    pub fn block(
        file: &FileReview,
        modes: impl Into<DiffModes>,
        first: End,
        last: End,
        text: impl Into<String>,
    ) -> Note {
        let full = file.view(DiffModes::SHOWN);
        let picked = file.view(modes);
        let found = rows_between(&full, first, last).or_else(|| rows_between(&picked, first, last));
        let (first, last, covered) = match found {
            Some((rows, false)) => (first, last, rows),
            Some((rows, true)) => (last, first, rows),
            // In neither diff: keep the ends in reading order when they share a side.
            None if first.0 == last.0 && last.1 < first.1 => (last, first, Vec::new()),
            None => (first, last, Vec::new()),
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
        note.end_side = (last.0 != first.0).then_some(last.0);
        if removed && others {
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
        } else if first.0 != last.0 {
            // Ends on both sides with no rows between them: each side covers its own end,
            // and a new-side line outside the full diff is an unchanged one.
            let quote = |(side, line): End| match side {
                NoteSide::Old => format!("-{}", source_lines(file, side, line, line)),
                NoteSide::New => format!(" {}", source_lines(file, side, line, line)),
            };
            for (side, line) in [first, last] {
                match side {
                    NoteSide::Old => note.old_lines = Some((line, line)),
                    NoteSide::New => note.new_lines = Some((line, line)),
                }
            }
            note.code = format!("{}\n{}", quote(first), quote(last));
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
        let mut note = Note::block(file, DiffModes::SHOWN, (side, first), (side, last), text);
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
