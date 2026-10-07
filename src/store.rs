use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::git::git;
use crate::review::FileReview;

/// Which changes the reviewer has marked as reviewed. A mark belongs to a file's exact
/// change — its path and both versions — so it lapses as soon as either version moves,
/// and it carries over between ranges that contain the same change.
///
/// Kept in `<git common dir>/declutter/reviewed.tsv`: private to this clone, shared by its
/// worktrees, never committed.
pub struct ReviewStore {
    path: Option<PathBuf>,
    marks: HashSet<(String, u64)>,
}

impl ReviewStore {
    /// Opens the store of the repository containing `dir`, or an in-memory one if the
    /// repository's git directory can't be found.
    pub fn open(dir: &Path) -> ReviewStore {
        match state_dir(dir) {
            Some(state) => ReviewStore::at(state.join("reviewed.tsv")),
            None => ReviewStore::in_memory(),
        }
    }

    pub fn at(path: PathBuf) -> ReviewStore {
        let marks = fs::read_to_string(&path)
            .unwrap_or_default()
            .lines()
            .filter_map(|line| {
                let (hash, file) = line.split_once('\t')?;
                Some((file.to_string(), u64::from_str_radix(hash, 16).ok()?))
            })
            .collect();
        ReviewStore {
            path: Some(path),
            marks,
        }
    }

    pub fn in_memory() -> ReviewStore {
        ReviewStore {
            path: None,
            marks: HashSet::new(),
        }
    }

    pub fn is_reviewed(&self, file: &FileReview) -> bool {
        self.marks.contains(&key(file))
    }

    pub fn set_reviewed(&mut self, file: &FileReview, reviewed: bool) -> Result<()> {
        if reviewed {
            self.marks.insert(key(file));
        } else {
            self.marks.remove(&key(file));
        }
        self.save()
    }

    fn save(&self) -> Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        let mut lines: Vec<String> = self
            .marks
            .iter()
            .map(|(file, hash)| format!("{hash:016x}\t{file}"))
            .collect();
        lines.sort();
        write_atomically(path, &(lines.join("\n") + "\n"))
    }
}

/// `<git common dir>/declutter`: per-clone state, shared by worktrees, never committed.
pub fn state_dir(dir: &Path) -> Option<PathBuf> {
    let common = git(
        dir,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )
    .ok()?;
    Some(PathBuf::from(String::from_utf8(common).ok()?.trim()).join("declutter"))
}

fn write_atomically(path: &Path, contents: &str) -> Result<()> {
    let dir = path.parent().context("state file has no directory")?;
    fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, contents).with_context(|| format!("writing {}", tmp.display()))?;
    fs::rename(&tmp, path).with_context(|| format!("writing {}", path.display()))
}

/// Which version of the file a note's line number refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NoteSide {
    /// A removed line: the number is in the old version.
    Old,
    New,
}

/// A reviewer's note on one line of a change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Note {
    pub path: String,
    pub side: NoteSide,
    pub line: usize,
    /// The line the note is about, so the note still makes sense once lines move.
    pub code: String,
    pub text: String,
    /// The review the note was left in — a pull request, or a range such as
    /// `origin/main...feature` — so it only shows up, and is only posted, there.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review: Option<String>,
}

/// Review notes, kept in `<git common dir>/declutter/notes.json` until cleared or
/// posted. A store can be scoped to one review; it then reads and writes only that
/// review's notes, and leaves the others alone.
pub struct NoteStore {
    path: Option<PathBuf>,
    notes: Vec<Note>,
    scope: Option<String>,
}

impl NoteStore {
    pub fn open(dir: &Path) -> NoteStore {
        match state_dir(dir) {
            Some(state) => NoteStore::at(state.join("notes.json")),
            None => NoteStore::in_memory(),
        }
    }

    pub fn at(path: PathBuf) -> NoteStore {
        let notes = fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        NoteStore {
            path: Some(path),
            notes,
            scope: None,
        }
    }

    pub fn in_memory() -> NoteStore {
        NoteStore {
            path: None,
            notes: Vec::new(),
            scope: None,
        }
    }

    /// The same store, seeing only the notes of `review`.
    pub fn scoped(mut self, review: impl Into<String>) -> NoteStore {
        self.scope = Some(review.into());
        self
    }

    fn in_scope(&self, note: &Note) -> bool {
        self.scope.is_none() || note.review == self.scope
    }

    pub fn notes(&self) -> Vec<&Note> {
        self.notes
            .iter()
            .filter(|note| self.in_scope(note))
            .collect()
    }

    /// Where exported prompts are written, next to the notes themselves.
    pub fn export_path(&self) -> Option<PathBuf> {
        Some(self.path.as_ref()?.with_file_name("review-notes.md"))
    }

    pub fn find(&self, path: &str, side: NoteSide, line: usize) -> Option<&Note> {
        self.notes.iter().find(|note| {
            self.in_scope(note) && note.path == path && note.side == side && note.line == line
        })
    }

    /// Adds or replaces the note on the same line of this review; an empty text removes it.
    pub fn set(&mut self, mut note: Note) -> Result<()> {
        note.review = self.scope.clone();
        self.notes.retain(|n| {
            !(n.review == note.review
                && n.path == note.path
                && n.side == note.side
                && n.line == note.line)
        });
        if !note.text.trim().is_empty() {
            self.notes.push(note);
            self.notes
                .sort_by(|a, b| (&a.path, a.line).cmp(&(&b.path, b.line)));
        }
        self.save()
    }

    /// Deletes this review's notes (every note, when the store is not scoped).
    pub fn clear(&mut self) -> Result<()> {
        let scope = self.scope.clone();
        self.notes
            .retain(|note| scope.is_some() && note.review != scope);
        self.save()
    }

    /// Deletes exactly these notes, as after posting them.
    pub fn remove(&mut self, gone: &[Note]) -> Result<()> {
        self.notes.retain(|note| !gone.contains(note));
        self.save()
    }

    /// The notes as one prompt to hand to a coding agent.
    pub fn prompt(&self) -> String {
        let mut out = String::from(
            "Please address these review comments. Line numbers refer to the version under review.\n",
        );
        for (i, note) in self.notes().into_iter().enumerate() {
            out.push_str(&format!(
                "\n{}. {}: {}\n   ```\n   {}\n   ```\n",
                i + 1,
                note.place(),
                note.text.trim(),
                note.code.trim()
            ));
        }
        out
    }

    fn save(&self) -> Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        write_atomically(path, &serde_json::to_string_pretty(&self.notes)?)
    }
}

impl Note {
    /// "`Cart.swift:12`", or "`Cart.swift` (removed line 12)".
    pub fn place(&self) -> String {
        match self.side {
            NoteSide::New => format!("`{}:{}`", self.path, self.line),
            NoteSide::Old => format!("`{}` (removed line {})", self.path, self.line),
        }
    }
}

fn key(file: &FileReview) -> (String, u64) {
    (file.path.clone(), file.fingerprint)
}

/// FNV-1a, 64-bit: stable across Rust releases, unlike the standard library's hasher.
pub(crate) fn fingerprint(parts: &[&str]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for (i, part) in parts.iter().enumerate() {
        if i > 0 {
            hash ^= 0xff;
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        for byte in part.bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    hash
}
