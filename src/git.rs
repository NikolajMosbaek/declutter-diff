use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

use crate::review::{ChangeStatus, FileChange};

/// One side of a comparison.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Side {
    Rev(String),
    Index,
    Worktree,
}

/// What to compare, parsed from the command line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RangeSpec {
    pub old: String,
    pub new: Side,
    /// `a...b`: compare `b` against the merge base of `a` and `b`.
    pub merge_base: bool,
}

impl RangeSpec {
    /// No argument: `HEAD` against the working tree, untracked files included.
    /// `rev`: `rev` against the working tree. `a..b`, `a...b`: as in `git diff`.
    /// `staged`: against the index instead of the working tree.
    pub fn parse(arg: Option<&str>, staged: bool) -> Result<RangeSpec> {
        let rev = |s: &str| {
            if s.is_empty() {
                "HEAD".to_string()
            } else {
                s.to_string()
            }
        };
        let spec = match arg {
            Some(arg) if arg.contains("..") => {
                if staged {
                    bail!("--staged takes a single revision, not a range");
                }
                let (merge_base, (a, b)) = match arg.split_once("...") {
                    Some(parts) => (true, parts),
                    None => (false, arg.split_once("..").unwrap_or((arg, ""))),
                };
                RangeSpec {
                    old: rev(a),
                    new: Side::Rev(rev(b)),
                    merge_base,
                }
            }
            other => RangeSpec {
                old: rev(other.unwrap_or("")),
                new: if staged { Side::Index } else { Side::Worktree },
                merge_base: false,
            },
        };
        Ok(spec)
    }
}

/// The top-level directory of the working tree containing `dir`.
pub fn repo_root(dir: &Path) -> Result<PathBuf> {
    let top = git(dir, &["rev-parse", "--show-toplevel"]).context("not inside a git repository")?;
    Ok(PathBuf::from(String::from_utf8(top)?.trim_end()))
}

/// Reads every changed file in the repository containing `dir`.
pub fn load(dir: &Path, spec: &RangeSpec) -> Result<Vec<FileChange>> {
    let root = repo_root(dir)?;

    let old_rev = if spec.merge_base {
        // Against the working tree or the index, the branch being measured is HEAD's.
        let tip = match &spec.new {
            Side::Rev(new_rev) => new_rev.as_str(),
            Side::Index | Side::Worktree => "HEAD",
        };
        String::from_utf8(git(&root, &["merge-base", &spec.old, tip])?)?
            .trim()
            .to_string()
    } else {
        spec.old.clone()
    };

    let mut args = vec![
        "diff",
        "--name-status",
        "-z",
        "-M",
        "--no-color",
        "--no-ext-diff",
    ];
    if spec.new == Side::Index {
        args.push("--cached");
    }
    args.push(&old_rev);
    if let Side::Rev(new_rev) = &spec.new {
        args.push(new_rev);
    }
    args.push("--");
    let listing = git(&root, &args)?;

    let mut tokens = listing
        .split(|byte| *byte == 0)
        .filter(|token| !token.is_empty())
        .map(|token| String::from_utf8_lossy(token).into_owned());
    let mut changes = Vec::new();
    while let Some(code) = tokens.next() {
        let first = tokens
            .next()
            .context("unexpected `git diff --name-status` output")?;
        let (status, old_path, path) = match code.chars().next() {
            Some('R' | 'C') => {
                let second = tokens
                    .next()
                    .context("unexpected `git diff --name-status` output")?;
                (ChangeStatus::Renamed, Some(first), second)
            }
            Some('A') => (ChangeStatus::Added, None, first),
            Some('D') => (ChangeStatus::Deleted, None, first),
            _ => (ChangeStatus::Modified, None, first),
        };

        let old = match status {
            ChangeStatus::Added => None,
            _ => {
                let old_path = old_path.as_deref().unwrap_or(&path);
                Some(git(&root, &["show", &format!("{old_rev}:{old_path}")])?)
            }
        };
        let new = match status {
            ChangeStatus::Deleted => None,
            _ => Some(read_side(&root, &spec.new, &path)?),
        };
        changes.push(file_change(path, old_path, status, old, new));
    }

    if spec.new == Side::Worktree {
        let untracked = git(&root, &["ls-files", "--others", "--exclude-standard", "-z"])?;
        for path in untracked.split(|byte| *byte == 0).filter(|p| !p.is_empty()) {
            let path = String::from_utf8_lossy(path).into_owned();
            if !root.join(&path).is_file() {
                continue;
            }
            let new = read_side(&root, &Side::Worktree, &path)?;
            changes.push(file_change(
                path,
                None,
                ChangeStatus::Added,
                None,
                Some(new),
            ));
        }
    }

    changes.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(changes)
}

fn read_side(root: &Path, side: &Side, path: &str) -> Result<Vec<u8>> {
    match side {
        Side::Rev(rev) => git(root, &["show", &format!("{rev}:{path}")]),
        Side::Index => git(root, &["show", &format!(":{path}")]),
        Side::Worktree => {
            let full = root.join(path);
            // Git stores a symlink as its target path; a dangling link must not fail the run.
            if full
                .symlink_metadata()
                .is_ok_and(|meta| meta.file_type().is_symlink())
            {
                return Ok(fs::read_link(&full)?
                    .to_string_lossy()
                    .into_owned()
                    .into_bytes());
            }
            fs::read(&full).with_context(|| format!("reading {path}"))
        }
    }
}

fn file_change(
    path: String,
    old_path: Option<String>,
    status: ChangeStatus,
    old: Option<Vec<u8>>,
    new: Option<Vec<u8>>,
) -> FileChange {
    let (old, old_binary) = decode(old);
    let (new, new_binary) = decode(new);
    let binary = old_binary || new_binary;
    FileChange {
        path,
        old_path,
        status,
        old: if binary { None } else { old },
        new: if binary { None } else { new },
        binary,
    }
}

fn decode(bytes: Option<Vec<u8>>) -> (Option<String>, bool) {
    let Some(bytes) = bytes else {
        return (None, false);
    };
    if bytes.iter().take(8000).any(|byte| *byte == 0) {
        return (None, true);
    }
    match String::from_utf8(bytes) {
        Ok(text) => (Some(text), false),
        Err(_) => (None, true),
    }
}

pub(crate) fn git(dir: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .context("failed to run git")?;
    if !output.status.success() {
        bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(output.stdout)
}
