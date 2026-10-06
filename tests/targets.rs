mod common;

use std::path::Path;

use anyhow::{Result, bail};
use common::{git, write};
use declutter::git::load;
use declutter::pr::{PrLookup, PrRefs, PullRequest};
use declutter::target::{Target, default_branch, resolve_target};
use tempfile::TempDir;

/// Fails every lookup: proves a PR was read without asking the host's CLI.
struct NoCli;

impl PrLookup for NoCli {
    fn refs(&self, _: &PullRequest) -> Result<PrRefs> {
        bail!("the CLI was asked")
    }
}

const URL: &str = "https://github.com/acme/shop.git";

/// An upstream whose `feature` branch adds b.py while main moves on with c.py, with
/// PR 8 published as its merge ref, and a clone of it that reaches it through `URL`.
fn upstream_and_clone() -> (TempDir, TempDir) {
    let upstream = TempDir::new().expect("temp dir");
    let up = upstream.path();
    git(up, &["init", "-q", "-b", "main"]);
    write(up, "a.py", "a = 1\n");
    git(up, &["add", "."]);
    git(up, &["commit", "-q", "-m", "base"]);
    git(up, &["checkout", "-q", "-b", "feature"]);
    write(up, "b.py", "b = 1\n");
    git(up, &["add", "."]);
    git(up, &["commit", "-q", "-m", "feature"]);
    git(up, &["checkout", "-q", "main"]);
    write(up, "c.py", "c = 1\n");
    git(up, &["add", "."]);
    git(up, &["commit", "-q", "-m", "main moves on"]);
    git(up, &["merge", "-q", "--no-ff", "--no-edit", "feature"]);
    let merge = git(up, &["rev-parse", "HEAD"]);
    git(up, &["update-ref", "refs/pull/8/merge", &merge]);
    git(up, &["reset", "-q", "--hard", "HEAD~1"]);

    let clone = TempDir::new().expect("temp dir");
    let instead_of = format!("url.file://{}.insteadOf", up.display());
    let parent = clone.path().parent().expect("temp parent");
    let name = clone
        .path()
        .file_name()
        .expect("temp name")
        .to_string_lossy()
        .into_owned();
    std::fs::remove_dir(clone.path()).expect("make room for the clone");
    git(
        parent,
        &[
            "-c",
            &format!("{instead_of}={URL}"),
            "clone",
            "-q",
            URL,
            &name,
        ],
    );
    git(clone.path(), &["config", &instead_of, URL]);
    (upstream, clone)
}

fn target(dir: &Path, args: &[&str], base: Option<&str>) -> Result<Target> {
    let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
    resolve_target(dir, &args, false, base, &NoCli)
}

fn paths(dir: &Path, target: &Target) -> Vec<String> {
    load(dir, &target.spec)
        .expect("load")
        .into_iter()
        .map(|change| change.path)
        .collect()
}

#[test]
fn a_branch_is_compared_with_main_from_where_it_split_off() {
    let (_upstream, clone) = upstream_and_clone();
    let dir = clone.path();

    let target = target(dir, &["feature"], None).expect("resolve");
    assert_eq!(target.label, "origin/main...origin/feature");
    assert_eq!(
        paths(dir, &target),
        ["b.py"],
        "c.py landed on main later, so it is not shown"
    );
}

#[test]
fn a_local_branch_wins_over_the_remote_one() {
    let (_upstream, clone) = upstream_and_clone();
    let dir = clone.path();
    git(dir, &["checkout", "-q", "-b", "mine"]);
    write(dir, "d.py", "d = 1\n");
    git(dir, &["add", "."]);
    git(dir, &["commit", "-q", "-m", "mine"]);

    let target = target(dir, &["mine"], None).expect("resolve");
    assert_eq!(target.label, "origin/main...mine");
    assert_eq!(paths(dir, &target), ["d.py"]);
}

#[test]
fn a_branch_only_on_origin_is_fetched_first() {
    let (upstream, clone) = upstream_and_clone();
    let up = upstream.path();
    git(up, &["checkout", "-q", "-b", "later", "feature"]);
    write(up, "e.py", "e = 1\n");
    git(up, &["add", "."]);
    git(up, &["commit", "-q", "-m", "pushed after the clone"]);

    let target = target(clone.path(), &["later"], None).expect("resolve");
    assert_eq!(target.label, "origin/main...origin/later");
    assert_eq!(paths(clone.path(), &target), ["b.py", "e.py"]);
}

#[test]
fn the_main_branch_and_plain_revisions_compare_with_the_working_tree() {
    let (_upstream, clone) = upstream_and_clone();
    let dir = clone.path();
    write(dir, "a.py", "a = 2\n");

    let main = target(dir, &["main"], None).expect("resolve");
    assert_eq!(main.label, "main → working tree");
    assert_eq!(paths(dir, &main), ["a.py"]);

    let revision = target(dir, &["HEAD~1"], None).expect("resolve");
    assert_eq!(revision.label, "HEAD~1 → working tree");
    assert_eq!(paths(dir, &revision), ["a.py", "c.py"]);
}

#[test]
fn base_overrides_the_main_branch_and_alone_compares_the_current_branch() {
    let (_upstream, clone) = upstream_and_clone();
    let dir = clone.path();
    git(
        dir,
        &["checkout", "-q", "-b", "on-feature", "origin/feature"],
    );
    write(dir, "f.py", "f = 1\n");
    git(dir, &["add", "."]);
    git(dir, &["commit", "-q", "-m", "on top of feature"]);

    let against_feature = target(dir, &["on-feature"], Some("origin/feature")).expect("resolve");
    assert_eq!(paths(dir, &against_feature), ["f.py"]);

    let current = target(dir, &[], Some("origin/main")).expect("resolve");
    assert_eq!(current.label, "origin/main...HEAD");
    assert_eq!(paths(dir, &current), ["b.py", "f.py"]);
}

#[test]
fn a_pr_url_is_read_from_its_merge_ref_without_the_cli() {
    let (_upstream, clone) = upstream_and_clone();
    let dir = clone.path();

    let target = target(dir, &["https://github.com/acme/shop/pull/8"], None).expect("resolve");
    assert_eq!(target.label, "PR 8");
    assert_eq!(paths(dir, &target), ["b.py"]);
}

#[test]
fn an_unknown_name_says_so() {
    let (_upstream, clone) = upstream_and_clone();

    let error = target(clone.path(), &["no-such-branch"], None).expect_err("unknown");
    assert!(
        error
            .to_string()
            .contains("no branch or revision named `no-such-branch`"),
        "{error}"
    );
}

#[test]
fn without_origin_the_main_branch_is_main_or_master() {
    let dir = TempDir::new().expect("temp dir");
    git(dir.path(), &["init", "-q", "-b", "master"]);
    write(dir.path(), "a.py", "a = 1\n");
    git(dir.path(), &["add", "."]);
    git(dir.path(), &["commit", "-q", "-m", "base"]);

    assert_eq!(default_branch(dir.path()).as_deref(), Some("master"));
}
