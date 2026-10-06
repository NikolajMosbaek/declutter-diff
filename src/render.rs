use std::fmt::Write;

use crate::diff::RowKind;
use crate::moves::detect;
use crate::project::LayerMode;
use crate::review::{Detection, DiffModes, FileReview, Layers, Summary};

/// The decluttered diff as plain text. Line numbers in hunk headers refer to the
/// original files, so the output is for reading, not for `git apply`.
pub fn plain(files: &[FileReview], layers: Layers) -> String {
    let mut out = String::new();
    let listed: Vec<&FileReview> = files
        .iter()
        .filter(|file| file.is_visible(layers.tests))
        .collect();
    let moves = detect(&listed, layers.into());
    for (file_index, file) in listed.iter().enumerate() {
        let view = file.view(layers);
        let _ = writeln!(out, "{} [{}]", file_title(file), file.tag());
        if file.detection == Detection::Binary {
            let _ = writeln!(out, "  (binary file not shown)");
        } else if view.hunks.is_empty() {
            let _ = writeln!(out, "  ({})", empty_message(file, layers.into()));
        }
        for (hunk_index, hunk) in view.hunks.iter().enumerate() {
            let _ = writeln!(out, "{}", hunk.header());
            for (row_index, row) in hunk.rows.iter().enumerate() {
                if let Some(moved) = moves.get(&(file_index, hunk_index, row_index))
                    && moved.starts_block
                {
                    let _ = writeln!(out, "{}", moved.describe(&file.path));
                }
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
    let _ = writeln!(out, "{}", Summary::new(files, layers).status_line(layers));
    out
}

pub fn file_title(file: &FileReview) -> String {
    match &file.old_path {
        Some(old) => format!("{} {old} → {}", file.status.letter(), file.path),
        None => format!("{} {}", file.status.letter(), file.path),
    }
}

/// Why a file with changes shows nothing under these layers.
pub fn empty_message(file: &FileReview, modes: DiffModes) -> String {
    let hidden = file.view(modes).hidden_hunks;
    let adjective = modes.adjective();
    match modes.outcome().0 {
        _ if file.total_hunks() == 0 => "no textual changes".to_string(),
        LayerMode::Hidden => format!(
            "{adjective}-only changes: {hidden} hunk{} hidden",
            if hidden == 1 { "" } else { "s" }
        ),
        LayerMode::Only => format!("no {adjective} changes"),
        LayerMode::Shown => "no changes".to_string(),
    }
}
