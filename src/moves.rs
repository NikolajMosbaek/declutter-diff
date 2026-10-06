use std::collections::{HashMap, HashSet};

use crate::diff::RowKind;
use crate::review::{DiffModes, FileReview};

/// The shortest run of lines treated as a move; shorter matches are mostly coincidence.
pub const MIN_MOVED_LINES: usize = 3;

/// Which end of a move a row is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// A removed line that reappears elsewhere.
    To,
    /// An added line that was removed elsewhere.
    From,
}

/// A changed row that is part of a block moved within the change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Move {
    pub direction: Direction,
    /// Where the other end of the block is: its file and first line.
    pub path: String,
    pub line: usize,
    /// The first row of the block, where the viewer puts the "moved" marker.
    pub starts_block: bool,
    pub block_len: usize,
}

impl Move {
    /// "5 lines moved to Cart.swift:12", or just the line when the block stayed in `here`.
    pub fn describe(&self, here: &str) -> String {
        let direction = match self.direction {
            Direction::To => "to",
            Direction::From => "from",
        };
        let place = if self.path == here {
            format!("line {}", self.line)
        } else {
            format!("{}:{}", self.path, self.line)
        };
        format!("⇄ {} lines moved {direction} {place}", self.block_len)
    }
}

/// Moved rows, keyed by (index into `files`, hunk, row).
pub type Moves = HashMap<(usize, usize, usize), Move>;

struct Entry {
    key: (usize, usize, usize),
    path: String,
    line: usize,
    /// The line with surrounding whitespace removed, so re-indented moves still match.
    text: String,
}

/// Finds blocks of at least `MIN_MOVED_LINES` removed lines that are added again
/// elsewhere — in the same file or another — under the given layers.
pub fn detect(files: &[&FileReview], modes: DiffModes) -> Moves {
    let mut removed: Vec<Vec<Entry>> = Vec::new();
    let mut added: Vec<Vec<Entry>> = Vec::new();
    for (file_index, file) in files.iter().enumerate() {
        let view = file.view(modes);
        for (hunk_index, hunk) in view.hunks.iter().enumerate() {
            let mut run_kind = RowKind::Context;
            for (row_index, row) in hunk.rows.iter().enumerate() {
                let text = row.text.trim();
                if row.kind != run_kind {
                    run_kind = row.kind;
                    match row.kind {
                        RowKind::Removed => removed.push(Vec::new()),
                        RowKind::Added => added.push(Vec::new()),
                        RowKind::Context => continue,
                    }
                }
                // Blank lines neither count towards a block nor break one.
                if text.is_empty() || row.kind == RowKind::Context {
                    continue;
                }
                let (runs, line) = match row.kind {
                    RowKind::Removed => (&mut removed, row.old_line),
                    _ => (&mut added, row.new_line),
                };
                if let (Some(run), Some(line)) = (runs.last_mut(), line) {
                    run.push(Entry {
                        key: (file_index, hunk_index, row_index),
                        path: file.path.clone(),
                        line,
                        text: text.to_string(),
                    });
                }
            }
        }
    }

    let window = |run: &[Entry], at: usize| -> String {
        run[at..at + MIN_MOVED_LINES]
            .iter()
            .map(|entry| entry.text.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    };
    let mut index: HashMap<String, Vec<(usize, usize)>> = HashMap::new();
    for (run_index, run) in added.iter().enumerate() {
        for at in 0..(run.len() + 1).saturating_sub(MIN_MOVED_LINES) {
            index
                .entry(window(run, at))
                .or_default()
                .push((run_index, at));
        }
    }

    let mut moves = Moves::new();
    let mut claimed: HashSet<(usize, usize)> = HashSet::new();
    for run in &removed {
        let mut at = 0;
        while at + MIN_MOVED_LINES <= run.len() {
            let best = index
                .get(&window(run, at))
                .into_iter()
                .flatten()
                .map(|&(target, start)| {
                    let other = &added[target];
                    let len = (0..)
                        .take_while(|&k| {
                            at + k < run.len()
                                && start + k < other.len()
                                && run[at + k].text == other[start + k].text
                                && !claimed.contains(&(target, start + k))
                        })
                        .count();
                    (len, target, start)
                })
                .max_by_key(|&(len, _, _)| len);
            let Some((len, target, start)) = best.filter(|&(len, _, _)| len >= MIN_MOVED_LINES)
            else {
                at += 1;
                continue;
            };
            let block = &run[at..at + len];
            // Three closing braces in a row are not a move worth pointing at.
            let substance: usize = block
                .iter()
                .map(|entry| entry.text.chars().filter(|c| c.is_alphanumeric()).count())
                .sum();
            if substance < 10 {
                at += 1;
                continue;
            }
            let destination = &added[target][start..start + len];
            for (k, (from, to)) in block.iter().zip(destination).enumerate() {
                claimed.insert((target, start + k));
                moves.insert(
                    from.key,
                    Move {
                        direction: Direction::To,
                        path: destination[0].path.clone(),
                        line: destination[0].line,
                        starts_block: k == 0,
                        block_len: len,
                    },
                );
                moves.insert(
                    to.key,
                    Move {
                        direction: Direction::From,
                        path: block[0].path.clone(),
                        line: block[0].line,
                        starts_block: k == 0,
                        block_len: len,
                    },
                );
            }
            at += len;
        }
    }
    moves
}
