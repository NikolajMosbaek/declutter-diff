use std::fmt::Write;

use crate::diff::RowKind;
use crate::project::CommentMode;
use crate::review::{Detection, FileReview, Summary};

/// The decluttered diff as plain text. Line numbers in hunk headers refer to the
/// original files, so the output is for reading, not for `git apply`.
pub fn plain(files: &[FileReview], mode: CommentMode) -> String {
    let mut out = String::new();
    for file in files {
        let view = file.view(mode);
        let _ = writeln!(out, "{} [{}]", file_title(file), file.detection.label());
        if file.detection == Detection::Binary {
            let _ = writeln!(out, "  (binary file not shown)");
        } else if view.hunks.is_empty() {
            let _ = writeln!(out, "  ({})", empty_message(file, mode));
        }
        for hunk in &view.hunks {
            let _ = writeln!(out, "{}", hunk.header());
            for row in &hunk.rows {
                let sign = match row.kind {
                    RowKind::Context => ' ',
                    RowKind::Removed => '-',
                    RowKind::Added => '+',
                };
                let _ = writeln!(out, "{sign}{}", row.text);
            }
        }
        let _ = writeln!(out);
    }
    let _ = writeln!(out, "{}", Summary::new(files, mode).status_line(mode));
    out
}

pub fn file_title(file: &FileReview) -> String {
    match &file.old_path {
        Some(old) => format!("{} {old} → {}", file.status.letter(), file.path),
        None => format!("{} {}", file.status.letter(), file.path),
    }
}

/// Why a file with changes shows nothing in this mode.
pub fn empty_message(file: &FileReview, mode: CommentMode) -> String {
    let hidden = file.view(mode).hidden_hunks;
    match mode {
        _ if file.total_hunks() == 0 => "no textual changes".to_string(),
        CommentMode::Hidden => format!(
            "comment-only changes: {hidden} hunk{} hidden",
            if hidden == 1 { "" } else { "s" }
        ),
        CommentMode::Only => "no comment changes".to_string(),
        CommentMode::Shown => "no changes".to_string(),
    }
}
