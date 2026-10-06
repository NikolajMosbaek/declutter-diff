use std::fmt::Write;

use crate::diff::RowKind;
use crate::project::LayerMode;
use crate::review::{Detection, FileReview, Layers, SpanModes, Summary};

/// The decluttered diff as plain text. Line numbers in hunk headers refer to the
/// original files, so the output is for reading, not for `git apply`.
pub fn plain(files: &[FileReview], layers: Layers) -> String {
    let mut out = String::new();
    for file in files.iter().filter(|file| file.is_visible(layers.tests)) {
        let view = file.view(layers);
        let _ = writeln!(out, "{} [{}]", file_title(file), file.tag());
        if file.detection == Detection::Binary {
            let _ = writeln!(out, "  (binary file not shown)");
        } else if view.hunks.is_empty() {
            let _ = writeln!(out, "  ({})", empty_message(file, layers.into()));
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
pub fn empty_message(file: &FileReview, modes: SpanModes) -> String {
    let hidden = file.view(modes).hidden_hunks;
    let adjective = modes.adjective();
    match modes.effective().0 {
        _ if file.total_hunks() == 0 => "no textual changes".to_string(),
        LayerMode::Hidden => format!(
            "{adjective}-only changes: {hidden} hunk{} hidden",
            if hidden == 1 { "" } else { "s" }
        ),
        LayerMode::Only => format!("no {adjective} changes"),
        LayerMode::Shown => "no changes".to_string(),
    }
}
