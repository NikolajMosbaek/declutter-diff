use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::diff::RowKind;
use crate::git::git;
use crate::review::{DiffModes, FileReview};

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

/// A reviewer's note on one line of a change, or — with an empty path — on the change
/// as a whole.
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
    /// Added from outside the viewer (`declutter notes add`) and not yet opened by the
    /// reviewer. Drafts are never posted: opening one with `m` makes it the reviewer's.
    #[serde(default, skip_serializing_if = "is_false")]
    pub draft: bool,
}

fn is_false(value: &bool) -> bool {
    !value
}

/// What [`NoteStore::add`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Added {
    New,
    /// The line already had a note; the text went under it.
    Appended,
    /// The line's note already says this.
    Duplicate,
}

/// Where a posted note ended up on the host.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Link {
    pub id: String,
    pub url: String,
}

/// A note as it was posted, kept in `posted.jsonl` next to the notes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Posted {
    #[serde(flatten)]
    pub note: Note,
    /// Seconds since the Unix epoch.
    pub posted_at: u64,
    #[serde(flatten)]
    pub link: Link,
}

/// Review notes, kept in `<git common dir>/declutter/notes.json` until cleared or
/// posted. A store can be scoped to one review; it then reads and writes only that
/// review's notes, and leaves the others alone.
pub struct NoteStore {
    path: Option<PathBuf>,
    notes: Vec<Note>,
    scope: Option<String>,
    /// What an in-memory store has posted; a stored one keeps it in `posted.jsonl`.
    posted: Vec<Posted>,
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
            posted: Vec::new(),
        }
    }

    pub fn in_memory() -> NoteStore {
        NoteStore {
            path: None,
            notes: Vec::new(),
            scope: None,
            posted: Vec::new(),
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

    /// The note on the change as a whole, if there is one.
    pub fn general(&self) -> Option<&Note> {
        self.find("", NoteSide::New, 0)
    }

    /// Adds or replaces the note on the same line of this review; an empty text removes it.
    pub fn set(&mut self, mut note: Note) -> Result<()> {
        note.review = self.scope.clone();
        self.notes.retain(|n| !n.same_place(&note));
        if !note.text.trim().is_empty() {
            self.notes.push(note);
            self.notes
                .sort_by(|a, b| (&a.path, a.line).cmp(&(&b.path, b.line)));
        }
        self.save()
    }

    /// Adds a note, putting its text under any note already on that line rather than
    /// replacing it. A note that gains text it didn't have takes the new note's draft
    /// state, so text nobody has read is never posted as read.
    pub fn add(&mut self, mut note: Note) -> Result<Added> {
        note.review = self.scope.clone();
        let text = note.text.trim().to_string();
        let added = match self.notes.iter_mut().find(|n| n.same_place(&note)) {
            Some(existing) if existing.text.contains(&text) => return Ok(Added::Duplicate),
            Some(existing) => {
                existing.text = format!("{}\n\n{text}", existing.text.trim_end());
                existing.draft |= note.draft;
                Added::Appended
            }
            None => {
                note.text = text;
                self.notes.push(note);
                self.notes
                    .sort_by(|a, b| (&a.path, a.line).cmp(&(&b.path, b.line)));
                Added::New
            }
        };
        self.save()?;
        Ok(added)
    }

    /// Logs these notes as posted, each with where it landed, and drops them from the store.
    pub fn record_posted(&mut self, posted: &[(Note, Link)]) -> Result<()> {
        let posted_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_secs());
        let records: Vec<Posted> = posted
            .iter()
            .map(|(note, link)| Posted {
                note: Note {
                    review: self.scope.clone().or_else(|| note.review.clone()),
                    ..note.clone()
                },
                posted_at,
                link: link.clone(),
            })
            .collect();
        match self.log_path() {
            Some(log) => {
                let mut lines = String::new();
                for record in &records {
                    lines.push_str(&serde_json::to_string(record)?);
                    lines.push('\n');
                }
                fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&log)
                    .and_then(|mut file| file.write_all(lines.as_bytes()))
                    .with_context(|| format!("writing {}", log.display()))?;
            }
            None => self.posted.extend(records),
        }
        let gone: Vec<Note> = posted.iter().map(|(note, _)| note.clone()).collect();
        self.remove(&gone)
    }

    /// This review's posted notes, oldest first.
    pub fn posted(&self) -> Vec<Posted> {
        let all = match self.log_path() {
            Some(log) => fs::read_to_string(log)
                .unwrap_or_default()
                .lines()
                .filter_map(|line| serde_json::from_str(line).ok())
                .collect(),
            None => self.posted.clone(),
        };
        all.into_iter()
            .filter(|posted: &Posted| self.in_scope(&posted.note))
            .collect()
    }

    fn log_path(&self) -> Option<PathBuf> {
        Some(self.path.as_ref()?.with_file_name("posted.jsonl"))
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

    /// The notes as one prompt to hand to a coding agent. Drafts are left out: nobody
    /// has read them, so they are no one's review comments yet.
    pub fn prompt(&self) -> String {
        let mut out = String::from(
            "Please address these review comments. Line numbers refer to the version under review.\n",
        );
        let opened = self.notes().into_iter().filter(|note| !note.draft);
        for (i, note) in opened.enumerate() {
            let text = note.text.trim().replace('\n', "\n   ");
            if note.is_general() {
                out.push_str(&format!("\n{}. On the change as a whole: {text}\n", i + 1));
            } else {
                out.push_str(&format!(
                    "\n{}. {}: {text}\n   ```\n   {}\n   ```\n",
                    i + 1,
                    note.place(),
                    note.code.trim()
                ));
            }
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
    /// A draft note on the change as a whole rather than on a line.
    pub fn on_pull_request(text: impl Into<String>) -> Note {
        Note {
            path: String::new(),
            side: NoteSide::New,
            line: 0,
            code: String::new(),
            text: text.into(),
            review: None,
            draft: true,
        }
    }

    /// A draft note on `line` of `path`, which must be a row of the change's diff —
    /// with every layer shown — since that is where the viewer can show it. A removed
    /// line is on the old side; added and unchanged lines are on the new side.
    pub fn on_line(
        files: &[FileReview],
        path: &str,
        side: NoteSide,
        line: usize,
        text: impl Into<String>,
    ) -> Result<Note> {
        let Some(file) = files.iter().find(|file| file.path == path) else {
            bail!("`{path}` is not part of this change");
        };
        let view = file.view(DiffModes::SHOWN);
        let rows = view.hunks.iter().flat_map(|hunk| &hunk.rows);
        let lines: Vec<(usize, &str)> = rows
            .filter_map(|row| match (side, row.kind) {
                (NoteSide::Old, RowKind::Removed) => Some((row.old_line?, row.text.as_str())),
                (NoteSide::New, RowKind::Added | RowKind::Context) => {
                    Some((row.new_line?, row.text.as_str()))
                }
                _ => None,
            })
            .collect();
        let Some((_, code)) = lines.iter().find(|(n, _)| *n == line) else {
            let (place, which) = match side {
                NoteSide::New => (format!("`{path}:{line}`"), "the lines"),
                NoteSide::Old => (
                    format!("removed line {line} of `{path}`"),
                    "the removed lines",
                ),
            };
            if lines.is_empty() {
                bail!("{place} is not in the diff, which has no such lines");
            }
            let numbers: Vec<usize> = lines.iter().map(|(n, _)| *n).collect();
            bail!(
                "{place} is not in the diff; {which} in it are {}",
                ranges(&numbers)
            );
        };
        Ok(Note {
            path: path.to_string(),
            side,
            line,
            code: code.to_string(),
            text: text.into().trim().to_string(),
            review: None,
            draft: true,
        })
    }

    pub fn is_general(&self) -> bool {
        self.path.is_empty()
    }

    fn same_place(&self, other: &Note) -> bool {
        self.review == other.review
            && self.path == other.path
            && self.side == other.side
            && self.line == other.line
    }

    /// "`Cart.swift:12`", or "`Cart.swift` (removed line 12)".
    pub fn place(&self) -> String {
        if self.is_general() {
            return "the change as a whole".to_string();
        }
        match self.side {
            NoteSide::New => format!("`{}:{}`", self.path, self.line),
            NoteSide::Old => format!("`{}` (removed line {})", self.path, self.line),
        }
    }
}

/// "3, 12–18, 40–52" for sorted line numbers.
fn ranges(numbers: &[usize]) -> String {
    let mut sorted = numbers.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    let mut parts: Vec<String> = Vec::new();
    let mut start = 0;
    for i in 0..sorted.len() {
        if i + 1 == sorted.len() || sorted[i + 1] != sorted[i] + 1 {
            parts.push(if start == i {
                sorted[i].to_string()
            } else {
                format!("{}–{}", sorted[start], sorted[i])
            });
            start = i + 1;
        }
    }
    parts.join(", ")
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
