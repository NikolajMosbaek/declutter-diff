use std::collections::HashSet;

use crate::classify::classify;
use crate::diff::{Hunk, RowKind, diff};
use crate::lang::Lang;
use crate::project::{CommentMode, project};

/// Unchanged lines shown around each change.
pub const CONTEXT: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeStatus {
    Added,
    Deleted,
    Modified,
    Renamed,
}

impl ChangeStatus {
    pub fn letter(self) -> char {
        match self {
            ChangeStatus::Added => 'A',
            ChangeStatus::Deleted => 'D',
            ChangeStatus::Modified => 'M',
            ChangeStatus::Renamed => 'R',
        }
    }
}

/// One changed file, both versions in full.
#[derive(Debug, Clone)]
pub struct FileChange {
    pub path: String,
    pub old_path: Option<String>,
    pub status: ChangeStatus,
    pub old: Option<String>,
    pub new: Option<String>,
    pub binary: bool,
}

/// How well declutter could find the comments in a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Detection {
    Parsed(Lang),
    /// Parsed with syntax errors; comments near an error may be missed.
    Partial(Lang),
    /// No grammar for this file type, so nothing is hidden.
    Unsupported,
    Binary,
}

impl Detection {
    pub fn label(self) -> String {
        match self {
            Detection::Parsed(lang) => lang.name().to_string(),
            Detection::Partial(lang) => format!("{}, partial parse", lang.name()),
            Detection::Unsupported => "comments not detected".to_string(),
            Detection::Binary => "binary".to_string(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ModeView {
    pub hunks: Vec<Hunk>,
    /// Hunks of the full diff that have no visible change in this mode.
    pub hidden_hunks: usize,
}

/// A changed file, diffed once per comment mode.
#[derive(Debug, Clone)]
pub struct FileReview {
    pub path: String,
    pub old_path: Option<String>,
    pub status: ChangeStatus,
    pub detection: Detection,
    views: [ModeView; 3],
}

impl FileReview {
    pub fn new(change: FileChange) -> FileReview {
        let old = change.old.as_deref().unwrap_or("");
        let new = change.new.as_deref().unwrap_or("");

        let (detection, old_comments, new_comments) = if change.binary {
            (Detection::Binary, Vec::new(), Vec::new())
        } else {
            match Lang::from_path(&change.path)
                .and_then(|lang| Some((lang, classify(lang, old)?, classify(lang, new)?)))
            {
                Some((lang, old_c, new_c)) => {
                    let detection = if old_c.partial || new_c.partial {
                        Detection::Partial(lang)
                    } else {
                        Detection::Parsed(lang)
                    };
                    (detection, old_c.comments, new_c.comments)
                }
                None => (Detection::Unsupported, Vec::new(), Vec::new()),
            }
        };

        let hunks_for = |mode: CommentMode| {
            let mut hunks = diff(
                &project(old, &old_comments, mode),
                &project(new, &new_comments, mode),
                CONTEXT,
            );
            if mode == CommentMode::Hidden {
                // Added or removed blank lines are layout: they mostly travel with a
                // comment, and they never change what the code does.
                for hunk in &mut hunks {
                    hunk.rows
                        .retain(|row| row.kind == RowKind::Context || !row.text.trim().is_empty());
                }
                hunks.retain(|hunk| hunk.changed().next().is_some());
            }
            hunks
        };

        let full = hunks_for(CommentMode::Shown);
        let views = CommentMode::ALL.map(|mode| {
            let hunks = if mode == CommentMode::Shown {
                full.clone()
            } else {
                hunks_for(mode)
            };
            ModeView {
                hidden_hunks: count_hidden(&full, &hunks),
                hunks,
            }
        });

        FileReview {
            path: change.path,
            old_path: change.old_path,
            status: change.status,
            detection,
            views,
        }
    }

    pub fn view(&self, mode: CommentMode) -> &ModeView {
        &self.views[mode.index()]
    }

    pub fn total_hunks(&self) -> usize {
        self.view(CommentMode::Shown).hunks.len()
    }
}

/// Counts hunks of the full diff none of whose changed lines are still changed in `visible`.
fn count_hidden(full: &[Hunk], visible: &[Hunk]) -> usize {
    let key = |kind: RowKind, old: Option<usize>, new: Option<usize>| match kind {
        RowKind::Removed => old.map(|line| (false, line)),
        _ => new.map(|line| (true, line)),
    };
    let still_changed: HashSet<(bool, usize)> = visible
        .iter()
        .flat_map(Hunk::changed)
        .filter_map(|row| key(row.kind, row.old_line, row.new_line))
        .collect();
    full.iter()
        .filter(|hunk| {
            !hunk.changed().any(|row| {
                key(row.kind, row.old_line, row.new_line)
                    .is_some_and(|k| still_changed.contains(&k))
            })
        })
        .count()
}

/// Totals across all files for one mode.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Summary {
    pub files: usize,
    pub total_hunks: usize,
    pub visible_hunks: usize,
    pub hidden_hunks: usize,
    /// Files with changes, none of them visible in this mode.
    pub fully_hidden_files: usize,
    pub unsupported_files: usize,
    pub partial_files: usize,
}

impl Summary {
    pub fn new(files: &[FileReview], mode: CommentMode) -> Summary {
        let mut summary = Summary {
            files: files.len(),
            ..Summary::default()
        };
        for file in files {
            let view = file.view(mode);
            summary.total_hunks += file.total_hunks();
            summary.visible_hunks += view.hunks.len();
            summary.hidden_hunks += view.hidden_hunks;
            if file.total_hunks() > 0 && view.hunks.is_empty() {
                summary.fully_hidden_files += 1;
            }
            match file.detection {
                Detection::Unsupported => summary.unsupported_files += 1,
                Detection::Partial(_) => summary.partial_files += 1,
                _ => {}
            }
        }
        summary
    }

    pub fn status_line(&self, mode: CommentMode) -> String {
        let mut parts = vec![format!("comments: {}", mode.label())];
        match mode {
            CommentMode::Shown => parts.push(format!(
                "{} {} in {} {}",
                self.total_hunks,
                plural(self.total_hunks, "hunk"),
                self.files,
                plural(self.files, "file")
            )),
            CommentMode::Hidden => {
                parts.push(format!(
                    "showing {} of {} hunks",
                    self.visible_hunks, self.total_hunks
                ));
                parts.push(format!(
                    "{} comment-only {} hidden",
                    self.hidden_hunks,
                    plural(self.hidden_hunks, "hunk")
                ));
                if self.fully_hidden_files > 0 {
                    parts.push(format!(
                        "{} comment-only {}",
                        self.fully_hidden_files,
                        plural(self.fully_hidden_files, "file")
                    ));
                }
            }
            CommentMode::Only => {
                parts.push(format!(
                    "{} {} with comment changes",
                    self.visible_hunks,
                    plural(self.visible_hunks, "hunk")
                ));
                parts.push(format!(
                    "{} code-only {} hidden",
                    self.hidden_hunks,
                    plural(self.hidden_hunks, "hunk")
                ));
            }
        }
        if self.unsupported_files > 0 {
            parts.push(format!(
                "comments not detected in {} {}",
                self.unsupported_files,
                plural(self.unsupported_files, "file")
            ));
        }
        if self.partial_files > 0 {
            parts.push(format!(
                "{} {} partially parsed",
                self.partial_files,
                plural(self.partial_files, "file")
            ));
        }
        parts.join(" · ")
    }
}

fn plural(count: usize, word: &str) -> String {
    if count == 1 {
        word.to_string()
    } else {
        format!("{word}s")
    }
}
