use std::path::Path;

use anyhow::{Context, Result, bail};

use crate::git::{RangeSpec, Side, git};
use crate::pr::{PrLookup, PullRequest, parse_pr_url, pr_number, resolve};

/// What to review, and how to say it in the viewer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub spec: RangeSpec,
    pub label: String,
    /// The pull request, when the target is one.
    pub pr: Option<PullRequest>,
}

impl Target {
    /// Names this review, for scoping notes to it.
    pub fn review_key(&self) -> String {
        match &self.pr {
            Some(pr) => pr.review_key(),
            None => self.label.clone(),
        }
    }
}

/// Works out what the command-line arguments ask to review:
///
/// - nothing: on a branch other than the main one, everything that branch changes —
///   its commits since it split off from the main branch (or `base`), plus uncommitted
///   and untracked work; on the main branch, just the uncommitted work;
/// - a GitHub or Azure DevOps PR URL, or `pr <url | number>`: that pull request;
/// - a branch: that branch against the default branch (`base` overrides it), from the
///   point where it branched off — a branch only on `origin` is fetched first;
/// - `a..b` / `a...b`: as in `git diff`;
/// - any other revision: that revision against the working tree.
pub fn resolve_target(
    dir: &Path,
    positionals: &[String],
    staged: bool,
    base: Option<&str>,
    lookup: &dyn PrLookup,
) -> Result<Target> {
    let pr = |target: &str| -> Result<Target> {
        if staged {
            bail!("--staged cannot be combined with a pull request");
        }
        let label = match pr_number(target) {
            Some(number) => format!("PR {number}"),
            None => format!("PR {target}"),
        };
        let (spec, pr) = resolve(dir, target, lookup)?;
        Ok(Target {
            spec,
            label,
            pr: Some(pr),
        })
    };
    let worktree = if staged { "index" } else { "working tree" };

    match positionals {
        [command, target] if command == "pr" => pr(target),
        [command] if command == "pr" => bail!("`pr` needs a pull-request URL or number"),
        [url] if parse_pr_url(url).is_some() => pr(url),
        [] => {
            let branch = current_branch(dir);
            let base = match base {
                Some(base) => Some(base.to_string()),
                None => default_branch(dir).filter(|main| {
                    branch
                        .as_deref()
                        .is_some_and(|branch| short_name(main) != branch)
                }),
            };
            match base {
                Some(base) => current_branch_against(dir, branch.as_deref(), &base, staged),
                None => Ok(Target {
                    spec: RangeSpec::parse(None, staged)?,
                    label: format!("HEAD → {worktree}"),
                    pr: None,
                }),
            }
        }
        [range] if range.contains("..") => Ok(Target {
            spec: RangeSpec::parse(Some(range), staged)?,
            label: range.clone(),
            pr: None,
        }),
        [name] => match branch(dir, name)? {
            Some(branch) => {
                let base = match base {
                    Some(base) => base.to_string(),
                    None => default_branch(dir)
                        .context("can't tell which branch is the main one; pass it with --base")?,
                };
                // The main branch itself: there is nothing to compare it with but your work.
                if short_name(&branch) == short_name(&base) {
                    return Ok(Target {
                        spec: RangeSpec::parse(Some(name), staged)?,
                        label: format!("{name} → {worktree}"),
                        pr: None,
                    });
                }
                branch_against(dir, &branch, &base, staged)
            }
            None => Ok(Target {
                spec: RangeSpec::parse(Some(name), staged)?,
                label: format!("{name} → {worktree}"),
                pr: None,
            }),
        },
        _ => bail!("give one branch, revision, range or PR"),
    }
}

/// The current branch's commits since it split off from `base`, plus what is not
/// committed yet (staged only, with `staged`).
fn current_branch_against(
    dir: &Path,
    branch: Option<&str>,
    base: &str,
    staged: bool,
) -> Result<Target> {
    if !exists(dir, &format!("{base}^{{commit}}")) {
        bail!("no branch or revision named `{base}` to compare against");
    }
    let (side, uncommitted) = if staged {
        (Side::Index, "staged")
    } else {
        (Side::Worktree, "uncommitted")
    };
    Ok(Target {
        spec: RangeSpec {
            old: base.to_string(),
            new: side,
            merge_base: true,
        },
        label: format!("{base}...{} + {uncommitted}", branch.unwrap_or("HEAD")),
        pr: None,
    })
}

fn branch_against(dir: &Path, branch: &str, base: &str, staged: bool) -> Result<Target> {
    if staged {
        bail!("--staged compares against the index, so it takes a revision, not a branch");
    }
    if !exists(dir, &format!("{base}^{{commit}}")) {
        bail!("no branch or revision named `{base}` to compare against");
    }
    Ok(Target {
        spec: RangeSpec {
            old: base.to_string(),
            new: Side::Rev(branch.to_string()),
            merge_base: true,
        },
        label: format!("{base}...{branch}"),
        pr: None,
    })
}

/// The branch `name` refers to — local first, then on `origin` — fetching it from
/// `origin` when it is nowhere else. `None` when `name` is a revision but not a branch.
fn branch(dir: &Path, name: &str) -> Result<Option<String>> {
    let has_ref = |full: &str| git(dir, &["show-ref", "--verify", "--quiet", full]).is_ok();
    if has_ref(&format!("refs/heads/{name}")) {
        return Ok(Some(name.to_string()));
    }
    if has_ref(&format!("refs/remotes/origin/{name}")) {
        return Ok(Some(format!("origin/{name}")));
    }
    if has_ref(&format!("refs/remotes/{name}")) {
        return Ok(Some(name.to_string()));
    }
    if exists(dir, &format!("{name}^{{commit}}")) {
        return Ok(None);
    }
    let is_branch_name = git(dir, &["check-ref-format", "--branch", name]).is_ok();
    let fetched = is_branch_name
        && git(
            dir,
            &[
                "fetch",
                "--quiet",
                "--no-tags",
                "origin",
                &format!("+refs/heads/{name}:refs/remotes/origin/{name}"),
            ],
        )
        .is_ok();
    if fetched {
        return Ok(Some(format!("origin/{name}")));
    }
    bail!("no branch or revision named `{name}`, here or on origin")
}

/// The branch reviews are compared with: what `origin/HEAD` points at, else the first
/// of `origin/main`, `origin/master`, `main`, `master` that exists.
pub fn default_branch(dir: &Path) -> Option<String> {
    if let Ok(out) = git(
        dir,
        &["symbolic-ref", "--quiet", "refs/remotes/origin/HEAD"],
    ) && let Ok(full) = String::from_utf8(out)
        && let Some(short) = full.trim().strip_prefix("refs/remotes/")
    {
        return Some(short.to_string());
    }
    ["origin/main", "origin/master", "main", "master"]
        .into_iter()
        .find(|name| exists(dir, &format!("{name}^{{commit}}")))
        .map(str::to_string)
}

/// The branch HEAD is on, if any.
pub fn current_branch(dir: &Path) -> Option<String> {
    let out = git(dir, &["symbolic-ref", "--quiet", "--short", "HEAD"]).ok()?;
    Some(String::from_utf8(out).ok()?.trim().to_string())
}

fn exists(dir: &Path, revision: &str) -> bool {
    git(dir, &["rev-parse", "--verify", "--quiet", revision]).is_ok()
}

fn short_name(branch: &str) -> &str {
    branch.strip_prefix("origin/").unwrap_or(branch)
}
