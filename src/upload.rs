use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

use crate::git::git;
use crate::pr::{PullRequest, Repo};
use crate::store::{Note, NoteSide, NoteStore};

/// Azure DevOps' resource id, for asking `az` for a token that reaches it.
const AZURE_DEVOPS: &str = "499b84ac-1321-427f-aa17-267ca6975798";

/// Posts one note to a pull request as a comment on its line.
pub trait Poster {
    fn post(&self, pr: &PullRequest, note: &Note) -> Result<()>;
}

/// Posts with the `az` or `gh` CLI, as whoever they are signed in as.
pub struct CliPoster {
    /// The repository the PR was fetched into, for its head commit.
    pub dir: PathBuf,
}

impl Poster for CliPoster {
    fn post(&self, pr: &PullRequest, note: &Note) -> Result<()> {
        match &pr.repo {
            Repo::AzureDevOps { .. } => {
                let (url, body) = azure_thread(pr, note);
                run(
                    "az",
                    &[
                        "rest",
                        "--method",
                        "post",
                        "--uri",
                        &url,
                        "--resource",
                        AZURE_DEVOPS,
                        "--headers",
                        "Content-Type=application/json",
                        "--body",
                        &body.to_string(),
                    ],
                )
            }
            Repo::GitHub { .. } => {
                let head = head_commit(&self.dir, pr)?;
                let on_line = github_comment(pr, note, &head, false);
                // GitHub only takes line comments inside its own hunks; elsewhere the
                // note goes on the file, naming its line.
                run("gh", &on_line).or_else(|_| run("gh", &github_comment(pr, note, &head, true)))
            }
        }
    }
}

fn head_commit(dir: &Path, pr: &PullRequest) -> Result<String> {
    let out = git(dir, &["rev-parse", &pr.head_ref()])?;
    Ok(String::from_utf8(out)?.trim().to_string())
}

fn run<S: AsRef<std::ffi::OsStr>>(program: &str, args: &[S]) -> Result<()> {
    let output = Command::new(program)
        .args(args)
        .output()
        .with_context(|| format!("failed to run `{program}`"))?;
    if !output.status.success() {
        bail!(
            "{}",
            String::from_utf8_lossy(&output.stderr)
                .lines()
                .find(|line| !line.trim().is_empty())
                .unwrap_or("failed")
                .trim()
        );
    }
    Ok(())
}

/// The Azure DevOps request that opens a thread on the note's line: its URL and body.
/// A removed line is anchored on the left (target) side of the diff.
pub fn azure_thread(pr: &PullRequest, note: &Note) -> (String, Value) {
    let Repo::AzureDevOps { org, project, name } = &pr.repo else {
        unreachable!("an Azure DevOps thread for a GitHub PR");
    };
    let url = format!(
        "https://dev.azure.com/{}/{}/_apis/git/repositories/{}/pullRequests/{}/threads?api-version=7.1",
        encode(org),
        encode(project),
        encode(name),
        pr.number
    );
    // A line-only anchor is rejected: the offsets are required, and 1 → 2 marks the
    // start of the line.
    let start = json!({ "line": note.line, "offset": 1 });
    let end = json!({ "line": note.line, "offset": 2 });
    let mut context = json!({ "filePath": format!("/{}", note.path) });
    let (start_key, end_key) = match note.side {
        NoteSide::New => ("rightFileStart", "rightFileEnd"),
        NoteSide::Old => ("leftFileStart", "leftFileEnd"),
    };
    context[start_key] = start;
    context[end_key] = end;
    let body = json!({
        "comments": [{ "parentCommentId": 0, "content": note.text.trim(), "commentType": 1 }],
        "status": "active",
        "threadContext": context,
    });
    (url, body)
}

/// The `gh api` arguments that comment on the note's line at the PR's head commit, or,
/// with `on_file`, on the file as a whole with the line named in the text.
pub fn github_comment(pr: &PullRequest, note: &Note, head: &str, on_file: bool) -> Vec<String> {
    let Repo::GitHub { owner, name } = &pr.repo else {
        unreachable!("a GitHub comment for an Azure DevOps PR");
    };
    let mut args = vec![
        "api".to_string(),
        "--method".to_string(),
        "POST".to_string(),
        format!("repos/{owner}/{name}/pulls/{}/comments", pr.number),
        "-f".to_string(),
        format!("commit_id={head}"),
        "-f".to_string(),
        format!("path={}", note.path),
    ];
    if on_file {
        let which = match note.side {
            NoteSide::New => "Line",
            NoteSide::Old => "Removed line",
        };
        args.extend([
            "-f".to_string(),
            "subject_type=file".to_string(),
            "-f".to_string(),
            format!("body={which} {}: {}", note.line, note.text.trim()),
        ]);
    } else {
        let side = match note.side {
            NoteSide::New => "RIGHT",
            NoteSide::Old => "LEFT",
        };
        args.extend([
            "-F".to_string(),
            format!("line={}", note.line),
            "-f".to_string(),
            format!("side={side}"),
            "-f".to_string(),
            format!("body={}", note.text.trim()),
        ]);
    }
    args
}

fn encode(segment: &str) -> String {
    segment
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (byte as char).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

/// After a review of a pull request, offers to post its notes there. Lists what would
/// be posted, asks, and posts only on an explicit yes. Posted notes leave the store;
/// notes that fail stay, so nothing is posted twice and nothing is lost.
pub fn offer_upload(
    store: &mut NoteStore,
    pr: &PullRequest,
    poster: &dyn Poster,
    input: &mut impl BufRead,
    out: &mut impl Write,
) -> Result<()> {
    let notes: Vec<Note> = store.notes().into_iter().cloned().collect();
    if notes.is_empty() {
        return Ok(());
    }
    let plural = if notes.len() == 1 { "" } else { "s" };
    writeln!(
        out,
        "\nYou left {} note{plural} in this review:",
        notes.len()
    )?;
    for note in &notes {
        writeln!(out, "  {}  {}", note.place(), note.text.trim())?;
    }
    write!(
        out,
        "Post {} comment{plural} to PR {}? [y/N] ",
        notes.len(),
        pr.number
    )?;
    out.flush()?;
    let mut answer = String::new();
    input.read_line(&mut answer)?;
    if !matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
        writeln!(out, "Not posted. The notes are kept for next time.")?;
        return Ok(());
    }

    let mut posted = Vec::new();
    for note in &notes {
        match poster.post(pr, note) {
            Ok(()) => posted.push(note.clone()),
            Err(error) => writeln!(out, "  could not post {}: {error:#}", note.place())?,
        }
    }
    store.remove(&posted)?;
    let failed = notes.len() - posted.len();
    match failed {
        0 => writeln!(
            out,
            "Posted {} comment{plural} to PR {}.",
            posted.len(),
            pr.number
        )?,
        _ => writeln!(
            out,
            "Posted {} of {} comments to PR {}; the {failed} that failed are kept.",
            posted.len(),
            notes.len(),
            pr.number
        )?,
    }
    Ok(())
}
