use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

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
        let common = git(
            dir,
            &["rev-parse", "--path-format=absolute", "--git-common-dir"],
        )
        .ok()
        .and_then(|out| String::from_utf8(out).ok());
        match common {
            Some(common) => {
                ReviewStore::at(PathBuf::from(common.trim()).join("declutter/reviewed.tsv"))
            }
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
        let dir = path
            .parent()
            .context("review store path has no directory")?;
        fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        let tmp = path.with_extension("tsv.tmp");
        fs::write(&tmp, lines.join("\n") + "\n")
            .with_context(|| format!("writing {}", tmp.display()))?;
        fs::rename(&tmp, path).with_context(|| format!("writing {}", path.display()))
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
