mod common;

use std::cell::RefCell;

use anyhow::Result;
use common::{git, write};
use declutter::git::{Side, load};
use declutter::pr::{PrLookup, PrRefs, PullRequest, Repo, parse_pr_url, parse_remote_url, resolve};
use tempfile::TempDir;

fn github(owner: &str, name: &str) -> Repo {
    Repo::GitHub {
        owner: owner.into(),
        name: name.into(),
    }
}

fn azure(org: &str, project: &str, name: &str) -> Repo {
    Repo::AzureDevOps {
        org: org.into(),
        project: project.into(),
        name: name.into(),
    }
}

#[test]
fn pull_request_urls_are_parsed() {
    let cases = [
        (
            "https://github.com/acme/shop/pull/42",
            github("acme", "shop"),
            42,
        ),
        (
            "https://github.com/acme/shop/pull/42/files?w=1",
            github("acme", "shop"),
            42,
        ),
        (
            "https://dev.azure.com/Contoso/Mobile%20Apps/_git/ios/pullrequest/115115",
            azure("Contoso", "Mobile Apps", "ios"),
            115115,
        ),
        (
            "https://contoso.visualstudio.com/Mobile/_git/ios/pullrequest/7?_a=files",
            azure("contoso", "Mobile", "ios"),
            7,
        ),
    ];
    for (url, repo, number) in cases {
        assert_eq!(
            parse_pr_url(url),
            Some(PullRequest { repo, number }),
            "{url}"
        );
    }
    assert_eq!(parse_pr_url("https://github.com/acme/shop/issues/42"), None);
    assert_eq!(parse_pr_url("https://example.com/acme/shop/pull/42"), None);
}

#[test]
fn remote_urls_are_parsed() {
    let cases = [
        ("https://github.com/acme/shop.git", github("acme", "shop")),
        ("git@github.com:acme/shop.git", github("acme", "shop")),
        ("ssh://git@github.com/acme/shop", github("acme", "shop")),
        (
            "https://Contoso@dev.azure.com/Contoso/Mobile/_git/ios",
            azure("Contoso", "Mobile", "ios"),
        ),
        (
            "git@ssh.dev.azure.com:v3/Contoso/Mobile/ios",
            azure("Contoso", "Mobile", "ios"),
        ),
        (
            "https://contoso.visualstudio.com/DefaultCollection/Mobile/_git/ios",
            azure("contoso", "Mobile", "ios"),
        ),
        (
            "contoso@vs-ssh.visualstudio.com:v3/contoso/Mobile/ios",
            azure("contoso", "Mobile", "ios"),
        ),
    ];
    for (url, repo) in cases {
        assert_eq!(parse_remote_url(url), Some(repo), "{url}");
    }
    assert_eq!(parse_remote_url("/tmp/some/repo"), None);
}

/// Answers every lookup with fixed refs and records what was asked.
struct Stub {
    refs: PrRefs,
    asked: RefCell<Vec<PullRequest>>,
}

impl PrLookup for Stub {
    fn refs(&self, pr: &PullRequest) -> Result<PrRefs> {
        self.asked.borrow_mut().push(pr.clone());
        Ok(self.refs.clone())
    }
}

/// An "upstream" repository standing in for github.com/acme/shop, with PR 7 published
/// under refs/pull/7/head, and a clone whose origin URL is the GitHub one.
fn upstream_and_clone() -> (TempDir, TempDir) {
    let upstream = TempDir::new().expect("temp dir");
    let up = upstream.path();
    git(up, &["init", "-q", "-b", "main"]);
    write(up, "cart.py", "total = 1\n");
    git(up, &["add", "."]);
    git(up, &["commit", "-q", "-m", "base"]);
    git(up, &["checkout", "-q", "-b", "feature"]);
    write(up, "cart.py", "# Doubled.\ntotal = 2\n");
    git(up, &["commit", "-q", "-am", "feature"]);
    git(up, &["update-ref", "refs/pull/7/head", "feature"]);
    git(up, &["checkout", "-q", "main"]);
    write(up, "readme.md", "later work on main\n");
    git(up, &["add", "."]);
    git(up, &["commit", "-q", "-m", "main moves on"]);

    let clone = TempDir::new().expect("temp dir");
    let dir = clone.path();
    git(dir, &["init", "-q", "-b", "main"]);
    git(
        dir,
        &[
            "remote",
            "add",
            "origin",
            "https://github.com/acme/shop.git",
        ],
    );
    let upstream_url = format!("file://{}", up.display());
    git(
        dir,
        &[
            "config",
            &format!("url.{upstream_url}.insteadOf"),
            "https://github.com/acme/shop.git",
        ],
    );
    (upstream, clone)
}

#[test]
fn a_pr_number_is_fetched_from_origin_and_compared_against_its_merge_base() {
    let (_upstream, clone) = upstream_and_clone();
    let stub = Stub {
        refs: PrRefs {
            base: "refs/heads/main".into(),
            head: "refs/pull/7/head".into(),
        },
        asked: RefCell::new(Vec::new()),
    };

    let spec = resolve(clone.path(), "7", &stub).expect("resolve PR");

    assert_eq!(
        stub.asked.borrow().as_slice(),
        [PullRequest {
            repo: github("acme", "shop"),
            number: 7
        }]
    );
    assert_eq!(spec.new, Side::Rev("refs/declutter/pr/7/head".into()));
    let changes = load(clone.path(), &spec).expect("load PR changes");
    let paths: Vec<_> = changes.iter().map(|c| c.path.as_str()).collect();
    // readme.md landed on main after the PR branched off, so it is not part of the PR.
    assert_eq!(paths, ["cart.py"]);
    assert!(
        git(clone.path(), &["branch", "--list"]).is_empty(),
        "no local branch is created"
    );
}

#[test]
fn a_pr_url_for_a_repository_without_a_matching_remote_is_refused() {
    let (_upstream, clone) = upstream_and_clone();
    let stub = Stub {
        refs: PrRefs {
            base: "refs/heads/main".into(),
            head: "refs/pull/7/head".into(),
        },
        asked: RefCell::new(Vec::new()),
    };

    let error = resolve(clone.path(), "https://github.com/other/repo/pull/7", &stub)
        .expect_err("no remote points at other/repo");

    assert!(
        error.to_string().contains("github.com/other/repo"),
        "{error}"
    );
    assert!(stub.asked.borrow().is_empty());
}
