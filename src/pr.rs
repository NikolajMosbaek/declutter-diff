use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};
use serde_json::Value;

use crate::git::{RangeSpec, Side, git};

/// A repository on a hosting service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Repo {
    GitHub {
        owner: String,
        name: String,
    },
    AzureDevOps {
        org: String,
        project: String,
        name: String,
    },
}

impl Repo {
    fn same_as(&self, other: &Repo) -> bool {
        let eq = |a: &str, b: &str| a.eq_ignore_ascii_case(b);
        match (self, other) {
            (Repo::GitHub { owner: a, name: b }, Repo::GitHub { owner: c, name: d }) => {
                eq(a, c) && eq(b, d)
            }
            (
                Repo::AzureDevOps {
                    org: a,
                    project: b,
                    name: c,
                },
                Repo::AzureDevOps {
                    org: d,
                    project: e,
                    name: f,
                },
            ) => eq(a, d) && eq(b, e) && eq(c, f),
            _ => false,
        }
    }

    fn display(&self) -> String {
        match self {
            Repo::GitHub { owner, name } => format!("github.com/{owner}/{name}"),
            Repo::AzureDevOps { org, project, name } => {
                format!("dev.azure.com/{org}/{project}/{name}")
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullRequest {
    pub repo: Repo,
    pub number: u64,
}

/// The refs to fetch for a pull request, as full ref names on the remote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrRefs {
    pub base: String,
    pub head: String,
}

/// Asks the hosting service which branches a pull request compares.
pub trait PrLookup {
    fn refs(&self, pr: &PullRequest) -> Result<PrRefs>;
}

/// Looks pull requests up with the `gh` and `az` command-line tools, using whatever
/// account they are logged in with.
pub struct CliLookup;

impl PrLookup for CliLookup {
    fn refs(&self, pr: &PullRequest) -> Result<PrRefs> {
        match &pr.repo {
            Repo::GitHub { owner, name } => {
                let json = run_json(
                    "gh",
                    &[
                        "pr",
                        "view",
                        &pr.number.to_string(),
                        "--repo",
                        &format!("{owner}/{name}"),
                        "--json",
                        "baseRefName",
                    ],
                )?;
                let base = json["baseRefName"]
                    .as_str()
                    .context("`gh pr view` returned no baseRefName")?;
                Ok(PrRefs {
                    base: format!("refs/heads/{base}"),
                    // GitHub publishes every PR head here, forks included.
                    head: format!("refs/pull/{}/head", pr.number),
                })
            }
            Repo::AzureDevOps { org, .. } => {
                let json = run_json(
                    "az",
                    &[
                        "repos",
                        "pr",
                        "show",
                        "--id",
                        &pr.number.to_string(),
                        "--org",
                        &format!("https://dev.azure.com/{org}"),
                        "--detect",
                        "false",
                        "--output",
                        "json",
                    ],
                )?;
                let field = |key: &str| {
                    json[key]
                        .as_str()
                        .map(str::to_string)
                        .with_context(|| format!("`az repos pr show` returned no {key}"))
                };
                Ok(PrRefs {
                    base: field("targetRefName")?,
                    head: field("sourceRefName")?,
                })
            }
        }
    }
}

fn run_json(program: &str, args: &[&str]) -> Result<Value> {
    let output = Command::new(program)
        .args(args)
        .output()
        .with_context(|| format!("failed to run `{program}`; is it installed and logged in?"))?;
    if !output.status.success() {
        let login = if program == "gh" {
            "gh auth login"
        } else {
            "az login"
        };
        bail!(
            "`{program} {}` failed: {}\n(if this is a sign-in problem, run `{login}` and try again)",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    serde_json::from_slice(&output.stdout)
        .with_context(|| format!("`{program}` returned invalid JSON"))
}

/// Parses a pull-request URL from GitHub or Azure DevOps.
pub fn parse_pr_url(url: &str) -> Option<PullRequest> {
    let url = url.split(['?', '#']).next()?.trim_end_matches('/');
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let parts: Vec<String> = rest.split('/').map(percent_decode).collect();
    let parts: Vec<&str> = parts.iter().map(String::as_str).collect();
    match parts.as_slice() {
        ["github.com", owner, name, "pull", number, ..] => Some(PullRequest {
            repo: Repo::GitHub {
                owner: owner.to_string(),
                name: name.to_string(),
            },
            number: number.parse().ok()?,
        }),
        [
            "dev.azure.com",
            org,
            project,
            "_git",
            name,
            "pullrequest",
            number,
            ..,
        ] => Some(PullRequest {
            repo: azure(org, project, name),
            number: number.parse().ok()?,
        }),
        [host, project, "_git", name, "pullrequest", number, ..] => Some(PullRequest {
            repo: azure(host.strip_suffix(".visualstudio.com")?, project, name),
            number: number.parse().ok()?,
        }),
        _ => None,
    }
}

/// Parses a git remote URL that points at GitHub or Azure DevOps.
pub fn parse_remote_url(url: &str) -> Option<Repo> {
    let url = url.trim().trim_end_matches('/');
    let url = url.strip_suffix(".git").unwrap_or(url);
    let path = if let Some(rest) = url.strip_prefix("git@github.com:") {
        format!("github.com/{rest}")
    } else if let Some(rest) = url.strip_prefix("git@ssh.dev.azure.com:v3/") {
        format!("ssh.dev.azure.com/{rest}")
    } else if let Some((_, rest)) = url.split_once("@vs-ssh.visualstudio.com:v3/") {
        format!("ssh.dev.azure.com/{rest}")
    } else {
        let rest = url.split_once("://")?.1;
        // Drop credentials or a username in front of the host.
        let rest = match rest.split_once('@') {
            Some((_, host_and_path)) if !host_and_path.contains('@') => host_and_path,
            _ => rest,
        };
        rest.to_string()
    };
    let parts: Vec<String> = path.split('/').map(percent_decode).collect();
    let parts: Vec<&str> = parts.iter().map(String::as_str).collect();
    match parts.as_slice() {
        ["github.com", owner, name] => Some(Repo::GitHub {
            owner: owner.to_string(),
            name: name.to_string(),
        }),
        ["dev.azure.com", org, project, "_git", name]
        | ["ssh.dev.azure.com", org, project, name] => Some(azure(org, project, name)),
        [host, "DefaultCollection", project, "_git", name] | [host, project, "_git", name] => Some(
            azure(host.strip_suffix(".visualstudio.com")?, project, name),
        ),
        _ => None,
    }
}

fn azure(org: &str, project: &str, name: &str) -> Repo {
    Repo::AzureDevOps {
        org: org.to_string(),
        project: project.to_string(),
        name: name.to_string(),
    }
}

fn percent_decode(segment: &str) -> String {
    let bytes = segment.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        if bytes[i] == b'%'
            && let (Some(hi), Some(lo)) = (
                bytes.get(i + 1).copied().and_then(hex),
                bytes.get(i + 2).copied().and_then(hex),
            )
        {
            out.push((hi * 16 + lo) as u8);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Turns a PR URL or number into a range: fetches the PR's base and head into
/// `refs/declutter/pr/<n>/` (so no local branch is touched) and compares the head
/// against its merge base with the base, as the PR page does.
pub fn resolve(dir: &Path, target: &str, lookup: &dyn PrLookup) -> Result<RangeSpec> {
    let remotes = remotes(dir)?;
    let pr = match target.parse::<u64>() {
        Ok(number) => {
            let (_, repo) = remotes
                .iter()
                .find(|(name, _)| name == "origin")
                .or_else(|| remotes.first())
                .context("no GitHub or Azure DevOps remote found; pass the full PR URL")?;
            PullRequest {
                repo: repo.clone(),
                number,
            }
        }
        Err(_) => parse_pr_url(target).with_context(|| {
            format!("`{target}` is not a PR number or a GitHub / Azure DevOps PR URL")
        })?,
    };
    let remote = remotes
        .iter()
        .filter(|(_, repo)| repo.same_as(&pr.repo))
        .min_by_key(|(name, _)| name != "origin")
        .map(|(name, _)| name.clone())
        .with_context(|| {
            format!(
                "this PR is in {}, but no remote of this repository points there",
                pr.repo.display()
            )
        })?;

    let refs = lookup.refs(&pr)?;
    let local = |side: &str| format!("refs/declutter/pr/{}/{side}", pr.number);
    git(
        dir,
        &[
            "fetch",
            "--quiet",
            "--no-tags",
            &remote,
            &format!("+{}:{}", refs.head, local("head")),
            &format!("+{}:{}", refs.base, local("base")),
        ],
    )
    .with_context(|| format!("fetching PR {} from `{remote}`", pr.number))?;

    Ok(RangeSpec {
        old: local("base"),
        new: Side::Rev(local("head")),
        merge_base: true,
    })
}

/// Remotes whose configured URL is a GitHub or Azure DevOps repository. Reads the raw
/// configured URL, before any `insteadOf` rewriting.
fn remotes(dir: &Path) -> Result<Vec<(String, Repo)>> {
    let output = git(dir, &["config", "--get-regexp", r"^remote\..*\.url$"]).unwrap_or_default();
    Ok(String::from_utf8_lossy(&output)
        .lines()
        .filter_map(|line| {
            let (key, url) = line.split_once(' ')?;
            let name = key.strip_prefix("remote.")?.strip_suffix(".url")?;
            Some((name.to_string(), parse_remote_url(url)?))
        })
        .collect())
}
