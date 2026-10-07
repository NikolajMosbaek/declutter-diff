use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value, json};

use crate::git::git;
use crate::pr::{PullRequest, Repo};
use crate::store::{Link, Note, NoteSide, NoteStore};

/// Azure DevOps' resource id, for asking `az` for a token that reaches it.
const AZURE_DEVOPS: &str = "499b84ac-1321-427f-aa17-267ca6975798";

/// Posts notes to a pull request, each as a comment on its line (or on the change as a
/// whole), and says where each one landed — or why it didn't.
pub trait Poster {
    fn post(&self, pr: &PullRequest, notes: &[Note]) -> Vec<Result<Link>>;
}

/// Posts with the `az` or `gh` CLI, as whoever they are signed in as.
pub struct CliPoster {
    /// The repository the PR was fetched into, for its head commit.
    pub dir: PathBuf,
}

impl Poster for CliPoster {
    fn post(&self, pr: &PullRequest, notes: &[Note]) -> Vec<Result<Link>> {
        match &pr.repo {
            // Azure DevOps has no batch: one thread per note.
            Repo::AzureDevOps { .. } => notes
                .iter()
                .map(|note| {
                    let (url, body) = azure_thread(pr, note);
                    let thread = run(
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
                        None,
                    )?;
                    let id = id_of(&thread);
                    Ok(Link {
                        url: azure_link(pr, &id),
                        id,
                    })
                })
                .collect(),
            Repo::GitHub { .. } => {
                let head = match head_commit(&self.dir, pr) {
                    Ok(head) => head,
                    Err(error) => {
                        return notes.iter().map(|_| Err(anyhow!("{error:#}"))).collect();
                    }
                };
                // One review, so the author hears about it once. GitHub refuses the
                // whole review if any line is outside its diff; then each note goes on
                // its own, on the file when its line is refused.
                let (args, body) = github_review(pr, notes, &head);
                if let Ok(review) = run("gh", &args, Some(&body.to_string())) {
                    let link = github_link(&review);
                    return notes.iter().map(|_| Ok(link.clone())).collect();
                }
                notes
                    .iter()
                    .map(|note| {
                        let comment = if note.is_general() {
                            run("gh", &github_issue_comment(pr, note), None)?
                        } else {
                            run("gh", &github_comment(pr, note, &head, false), None).or_else(
                                |_| run("gh", &github_comment(pr, note, &head, true), None),
                            )?
                        };
                        Ok(github_link(&comment))
                    })
                    .collect()
            }
        }
    }
}

fn head_commit(dir: &Path, pr: &PullRequest) -> Result<String> {
    let out = git(dir, &["rev-parse", &pr.head_ref()])?;
    Ok(String::from_utf8(out)?.trim().to_string())
}

/// Runs a CLI, optionally feeding it `input`, and reads its JSON answer.
fn run<S: AsRef<std::ffi::OsStr>>(program: &str, args: &[S], input: Option<&str>) -> Result<Value> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("failed to run `{program}`"))?;
    if let (Some(input), Some(mut stdin)) = (input, child.stdin.take()) {
        stdin.write_all(input.as_bytes())?;
    }
    let output = child.wait_with_output()?;
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
    Ok(serde_json::from_slice(&output.stdout).unwrap_or(Value::Null))
}

/// The `id` of a created thread, comment or review, as text.
fn id_of(created: &Value) -> String {
    match &created["id"] {
        Value::Number(id) => id.to_string(),
        Value::String(id) => id.clone(),
        _ => String::new(),
    }
}

fn github_link(created: &Value) -> Link {
    Link {
        id: id_of(created),
        url: created["html_url"].as_str().unwrap_or_default().to_string(),
    }
}

/// The page of an Azure DevOps pull request, opened at one of its threads.
pub fn azure_link(pr: &PullRequest, thread: &str) -> String {
    let Repo::AzureDevOps { org, project, name } = &pr.repo else {
        unreachable!("an Azure DevOps link for a GitHub PR");
    };
    format!(
        "https://dev.azure.com/{}/{}/_git/{}/pullrequest/{}?discussionId={thread}",
        encode(org),
        encode(project),
        encode(name),
        pr.number
    )
}

/// The Azure DevOps request that opens a thread on the note's line: its URL and body.
/// A removed line is anchored on the left (target) side of the diff; a note on the
/// change as a whole has no anchor.
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
    let comments = json!([{ "parentCommentId": 0, "content": note.text.trim(), "commentType": 1 }]);
    if note.is_general() {
        return (url, json!({ "comments": comments, "status": "active" }));
    }
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
        "comments": comments,
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

/// The `gh api` arguments and JSON body for one review holding every note: the note on
/// the change as a whole as its text, the others as comments on their lines.
pub fn github_review(pr: &PullRequest, notes: &[Note], head: &str) -> (Vec<String>, Value) {
    let Repo::GitHub { owner, name } = &pr.repo else {
        unreachable!("a GitHub review for an Azure DevOps PR");
    };
    let args = [
        "api",
        "--method",
        "POST",
        &format!("repos/{owner}/{name}/pulls/{}/reviews", pr.number),
        "--input",
        "-",
    ]
    .map(String::from)
    .to_vec();
    let mut body = json!({ "commit_id": head, "event": "COMMENT" });
    let general: Vec<&str> = notes
        .iter()
        .filter(|note| note.is_general())
        .map(|note| note.text.trim())
        .collect();
    if !general.is_empty() {
        body["body"] = json!(general.join("\n\n"));
    }
    body["comments"] = notes
        .iter()
        .filter(|note| !note.is_general())
        .map(|note| {
            json!({
                "path": note.path,
                "line": note.line,
                "side": match note.side {
                    NoteSide::New => "RIGHT",
                    NoteSide::Old => "LEFT",
                },
                "body": note.text.trim(),
            })
        })
        .collect();
    (args, body)
}

/// The `gh api` arguments for a comment on the pull request itself.
fn github_issue_comment(pr: &PullRequest, note: &Note) -> Vec<String> {
    let Repo::GitHub { owner, name } = &pr.repo else {
        unreachable!("a GitHub comment for an Azure DevOps PR");
    };
    [
        "api".to_string(),
        "--method".to_string(),
        "POST".to_string(),
        format!("repos/{owner}/{name}/issues/{}/comments", pr.number),
        "-f".to_string(),
        format!("body={}", note.text.trim()),
    ]
    .to_vec()
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
/// be posted, asks — unless `yes` already said so — and posts only on an explicit yes.
/// Drafts nobody has opened are never posted. Posted notes leave the store for the
/// posted log; notes that fail stay, so nothing is posted twice and nothing is lost.
pub fn offer_upload(
    store: &mut NoteStore,
    pr: &PullRequest,
    poster: &dyn Poster,
    yes: bool,
    input: &mut impl BufRead,
    out: &mut impl Write,
) -> Result<()> {
    let (drafts, notes): (Vec<Note>, Vec<Note>) = store
        .notes()
        .into_iter()
        .cloned()
        .partition(|note| note.draft);
    let held = match drafts.len() {
        0 => String::new(),
        1 => "1 draft note not opened — kept".to_string(),
        n => format!("{n} draft notes not opened — kept"),
    };
    if notes.is_empty() {
        if !drafts.is_empty() {
            writeln!(
                out,
                "\n{held}. Open a draft with m in the viewer (P for the note on the whole \
                 change) to make it yours; then it can be posted."
            )?;
        }
        return Ok(());
    }
    let plural = if notes.len() == 1 { "" } else { "s" };
    writeln!(
        out,
        "\nYou left {} note{plural} in this review:",
        notes.len()
    )?;
    for note in &notes {
        let mut lines = note.text.trim().lines();
        let first = lines.next().unwrap_or_default();
        let more = if lines.next().is_some() { " …" } else { "" };
        writeln!(out, "  {}  {first}{more}", note.place())?;
    }
    if !drafts.is_empty() {
        writeln!(out, "  ({held})")?;
    }
    if yes {
        writeln!(
            out,
            "Posting {} comment{plural} to PR {}.",
            notes.len(),
            pr.number
        )?;
    } else {
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
    }

    let mut posted = Vec::new();
    for (note, result) in notes.iter().zip(poster.post(pr, &notes)) {
        match result {
            Ok(link) => posted.push((note.clone(), link)),
            Err(error) => writeln!(out, "  could not post {}: {error:#}", note.place())?,
        }
    }
    store.record_posted(&posted)?;
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
