use std::io::IsTerminal;
use std::path::Path;
use std::process::ExitCode;

use anyhow::{Context, Result, anyhow, bail};
use declutter::git::{load, repo_root};
use declutter::pr::CliLookup;
use declutter::project::LayerMode;
use declutter::review::{FileReview, Layers};
use declutter::store::{Added, Note, NoteSide, NoteStore, ReviewStore};
use declutter::target::{Target, resolve_target};
use declutter::upload::{CliPoster, offer_upload};
use declutter::{render, tui};
use serde::Deserialize;
use serde_json::json;

const USAGE: &str = "\
declutter — review a diff with comments and tests shown, hidden, or on their own

USAGE:
    declutter [OPTIONS]                   the branch you are on vs main, uncommitted work included
                                          (on main itself: just the uncommitted work)
    declutter [OPTIONS] <PR URL>          a GitHub or Azure DevOps pull request
    declutter [OPTIONS] pr <NUMBER>       a pull request in the repository origin points at
    declutter [OPTIONS] <BRANCH>          a branch vs the main branch, from where it split off
    declutter [OPTIONS] <A..B | A...B>    as in git diff
    declutter [OPTIONS] <REVISION>        a revision vs the working tree
    declutter notes [TARGET] [--json | --clear]
    declutter notes add [TARGET] [--path FILE --line N [--old]] --text TEXT
    declutter notes add [TARGET] --json FILE
    declutter notes post [TARGET] [--yes]

    A pull request is fetched into refs/declutter/pr/<n>/ (no local branch is touched) and
    shown as its PR page shows it; if the remote publishes no merge ref for it, the `gh` or
    `az` CLI is asked for its branches. A branch that exists only on origin is fetched. The
    main branch is what origin/HEAD points at (else main or master); --base overrides it.
    `notes` prints the review notes left with `m` as one prompt for a coding agent
    (TARGET, written as for the viewer, limits it to that review); `--json` prints them
    with what was already posted; `--clear` deletes them. After reviewing a pull request,
    declutter lists the notes you left and asks before posting them to the PR as line
    comments (via `az` / `gh`).

    `notes add` adds draft notes from outside the viewer, e.g. from a coding agent: on a
    line of the diff (`--old` for a removed line), or, without --path and --line, on the
    change as a whole. `--json` reads [{\"path\", \"line\", \"side\": \"old\"|\"new\", \"text\"}, …]
    from FILE (`-` for stdin), adding all or, if any is refused, none. A note on an
    already noted line goes under it. Drafts are never posted until opened with `m`.
    `notes post` offers to post a pull request's notes without opening the viewer;
    `--yes` posts without asking (drafts still stay).

OPTIONS:
    --staged             Compare against the index instead of the working tree
    --base <BRANCH>      Compare against BRANCH instead of the main branch
    --comments <MODE>    Start comments in MODE: hidden (default), only, shown
    --tests <MODE>       Start tests in MODE: shown (default), hidden, only
    --imports <MODE>     Start imports in MODE: shown (default), hidden, only
    --logging <MODE>     Start logging statements in MODE: shown (default), hidden, only
    --formatting <MODE>  Start whitespace-only changes in MODE: shown (default), hidden, only
    -p, --print          Print the diff instead of opening the viewer
    -n, --line-numbers   Print the diff with each row's old and new line numbers
    --no-syntax          Start with syntax colouring off (s toggles it)
    -h, --help           Show this help
    -V, --version        Show the version

KEYS (viewer; press ? for the full list):
    ↑ ↓  j k    move through the files, or the diff's line cursor
    ← →  Tab    switch between the file list and the diff
    ] [         next / previous change, on into the next file
    } {         next / previous file
    Space  b    page down / up; d u half a page; g G top / bottom
    /  n  N     search; next / previous match
    r           mark the file reviewed and go to the next one
    m  E  o     note a line; copy all notes as a prompt; open in $EDITOR
    c t i l f   hide / show comments, tests, imports, logging, formatting
    C T I L F   show only that layer (again to go back)
    a           all filters off, and back on
    M           collapse moved blocks
    s           syntax colouring on / off
    q           quit
";

/// The flags of `notes add`.
#[derive(Default)]
struct NoteFlags {
    path: Option<String>,
    line: Option<usize>,
    old: bool,
    text: Option<String>,
    json_file: Option<String>,
}

impl NoteFlags {
    fn any(&self) -> bool {
        self.path.is_some()
            || self.line.is_some()
            || self.old
            || self.text.is_some()
            || self.json_file.is_some()
    }
}

/// One note for `notes add --json`; without a path and line it is on the whole change.
#[derive(Deserialize)]
struct NoteEntry {
    path: Option<String>,
    line: Option<usize>,
    side: Option<NoteSide>,
    text: String,
}

struct Args {
    positionals: Vec<String>,
    base: Option<String>,
    staged: bool,
    layers: Layers,
    print: bool,
    line_numbers: bool,
    clear: bool,
    json: bool,
    yes: bool,
    note: NoteFlags,
    syntax: bool,
}

fn parse_args(raw: impl Iterator<Item = String>) -> Result<Option<Args>> {
    let mut args = Args {
        positionals: Vec::new(),
        base: None,
        staged: false,
        layers: Layers::default(),
        print: false,
        line_numbers: false,
        clear: false,
        json: false,
        yes: false,
        note: NoteFlags::default(),
        syntax: true,
    };
    let mut raw = raw.peekable();
    while let Some(arg) = raw.next() {
        let (flag, inline) = match arg.split_once('=') {
            Some((flag, value)) if flag.starts_with("--") => {
                (flag.to_string(), Some(value.to_string()))
            }
            _ => (arg.clone(), None),
        };
        match flag.as_str() {
            "-h" | "--help" => {
                print!("{USAGE}");
                return Ok(None);
            }
            "-V" | "--version" => {
                println!("declutter {}", env!("CARGO_PKG_VERSION"));
                return Ok(None);
            }
            "--staged" | "--cached" => args.staged = true,
            "-p" | "--print" => args.print = true,
            "-n" | "--line-numbers" => {
                args.print = true;
                args.line_numbers = true;
            }
            "--clear" => args.clear = true,
            "--json" if args.positionals.get(1).is_some_and(|word| word == "add") => {
                let Some(value) = inline.or_else(|| raw.next()) else {
                    bail!("--json needs a file, or - for stdin");
                };
                args.note.json_file = Some(value);
            }
            "--json" => args.json = true,
            "--old" => args.note.old = true,
            "-y" | "--yes" => args.yes = true,
            "--path" | "--text" => {
                let Some(value) = inline.or_else(|| raw.next()) else {
                    bail!("{flag} needs a value");
                };
                match flag.as_str() {
                    "--path" => args.note.path = Some(value),
                    _ => args.note.text = Some(value),
                }
            }
            "--line" => {
                let value = inline.or_else(|| raw.next()).unwrap_or_default();
                let Ok(line) = value.parse::<usize>() else {
                    bail!("--line needs a line number");
                };
                args.note.line = Some(line);
            }
            "--no-syntax" => args.syntax = false,
            "--base" => {
                let Some(value) = inline.or_else(|| raw.next()) else {
                    bail!("--base needs a branch");
                };
                args.base = Some(value);
            }
            "--comments" | "--tests" | "--imports" | "--logging" | "--formatting" => {
                let Some(value) = inline.or_else(|| raw.next()) else {
                    bail!("{flag} needs a value: hidden, only or shown");
                };
                let Some(mode) = LayerMode::parse(&value) else {
                    bail!("unknown {flag} value `{value}`: use hidden, only or shown");
                };
                match flag.as_str() {
                    "--comments" => args.layers.comments = mode,
                    "--tests" => args.layers.tests = mode,
                    "--imports" => args.layers.imports = mode,
                    "--logging" => args.layers.logging = mode,
                    _ => args.layers.formatting = mode,
                }
            }
            _ if arg.starts_with('-') => bail!("unknown option `{arg}` (see --help)"),
            _ => args.positionals.push(arg),
        }
    }
    Ok(Some(args))
}

fn run() -> Result<()> {
    let Some(args) = parse_args(std::env::args().skip(1))? else {
        return Ok(());
    };
    let dir = std::env::current_dir()?;
    let subcommand = |words: &[&str]| {
        words
            .iter()
            .zip(&args.positionals)
            .all(|(word, given)| word == given)
            && args.positionals.len() >= words.len()
    };
    if subcommand(&["notes", "add"]) {
        return add_notes(&dir, &args.positionals[2..], &args);
    }
    if args.note.any() {
        bail!("--path, --line, --old and --text only apply to `declutter notes add`");
    }
    if subcommand(&["notes", "post"]) {
        return post_notes(&dir, &args.positionals[2..], &args);
    }
    if args.yes {
        bail!("--yes only applies to `declutter notes post`");
    }
    if subcommand(&["notes"]) {
        return list_notes(&dir, &args.positionals[1..], &args);
    }
    if args.clear || args.json {
        bail!("--clear and --json only apply to `declutter notes`");
    }
    let target = target(&dir, &args.positionals, &args)?;
    let files: Vec<FileReview> = load(&dir, &target.spec)?
        .into_iter()
        .map(FileReview::new)
        .collect();

    if args.print {
        if args.line_numbers {
            print!("{}", render::plain_numbered(&files, args.layers));
        } else {
            print!("{}", render::plain(&files, args.layers));
        }
        return Ok(());
    }
    if files.is_empty() {
        println!("No changes in {}.", target.label);
        return Ok(());
    }
    if !std::io::stdout().is_terminal() {
        bail!("stdout is not a terminal; use --print for text output");
    }
    let notes = NoteStore::open(&dir).scoped(target.review_key());
    let mut notes = tui::run(
        files,
        args.layers,
        ReviewStore::open(&dir),
        notes,
        repo_root(&dir)?,
        target.label.clone(),
        args.syntax,
    )?;
    if let Some(pr) = &target.pr
        && std::io::stdin().is_terminal()
    {
        let poster = CliPoster { dir: dir.clone() };
        offer_upload(
            &mut notes,
            pr,
            &poster,
            false,
            &mut std::io::stdin().lock(),
            &mut std::io::stdout(),
        )?;
    }
    Ok(())
}

fn target(dir: &Path, positionals: &[String], args: &Args) -> Result<Target> {
    resolve_target(
        dir,
        positionals,
        args.staged,
        args.base.as_deref(),
        &CliLookup,
    )
}

/// `declutter notes`: the notes as a prompt or as JSON, of every review or of one.
fn list_notes(dir: &Path, positionals: &[String], args: &Args) -> Result<()> {
    let mut notes = NoteStore::open(dir);
    if !positionals.is_empty() {
        notes = notes.scoped(target(dir, positionals, args)?.review_key());
    }
    if args.clear {
        let count = notes.notes().len();
        notes.clear()?;
        println!("Cleared {count} note{}.", if count == 1 { "" } else { "s" });
    } else if args.json {
        let listing = json!({ "notes": notes.notes(), "posted": notes.posted() });
        println!("{}", serde_json::to_string_pretty(&listing)?);
    } else if notes.notes().is_empty() {
        println!("No review notes. Press m on a diff line in the viewer to add one.");
    } else {
        print!("{}", notes.prompt());
    }
    Ok(())
}

/// `declutter notes post`: the end-of-review offer to post, without the review.
fn post_notes(dir: &Path, positionals: &[String], args: &Args) -> Result<()> {
    let target = target(dir, positionals, args)?;
    let Some(pr) = &target.pr else {
        bail!(
            "{} is not a pull request; `notes post` takes a PR URL or `pr <number>`",
            target.label
        );
    };
    let mut notes = NoteStore::open(dir).scoped(target.review_key());
    if notes.notes().is_empty() {
        println!("No notes to post in {}.", target.label);
        return Ok(());
    }
    offer_upload(
        &mut notes,
        pr,
        &CliPoster {
            dir: dir.to_path_buf(),
        },
        args.yes,
        &mut std::io::stdin().lock(),
        &mut std::io::stdout(),
    )
}

/// `declutter notes add`: draft notes from outside the viewer, all or none.
fn add_notes(dir: &Path, positionals: &[String], args: &Args) -> Result<()> {
    let flags = &args.note;
    let entries: Vec<NoteEntry> = match &flags.json_file {
        Some(file) => {
            if flags.path.is_some() || flags.line.is_some() || flags.old || flags.text.is_some() {
                bail!(
                    "--json takes the notes from FILE; leave out --path, --line, --old and --text"
                );
            }
            let text = if file == "-" {
                std::io::read_to_string(std::io::stdin())?
            } else {
                std::fs::read_to_string(file).with_context(|| format!("reading {file}"))?
            };
            serde_json::from_str(&text).context(
                r#"--json expects [{"path": …, "line": …, "side": "old"|"new", "text": …}, …]"#,
            )?
        }
        None => {
            let Some(text) = flags.text.clone() else {
                bail!("`notes add` needs --text, or --json FILE");
            };
            vec![NoteEntry {
                path: flags.path.clone(),
                line: flags.line,
                side: flags.old.then_some(NoteSide::Old),
                text,
            }]
        }
    };
    if entries.is_empty() {
        bail!("no notes to add");
    }

    let target = target(dir, positionals, args)?;
    let files: Vec<FileReview> = load(dir, &target.spec)?
        .into_iter()
        .map(FileReview::new)
        .collect();
    let mut notes = Vec::new();
    let mut problems = Vec::new();
    for (i, entry) in entries.into_iter().enumerate() {
        let note = match (entry.path, entry.line) {
            _ if entry.text.trim().is_empty() => Err(anyhow!("the text is empty")),
            (None, None) => Ok(Note::on_pull_request(entry.text.trim())),
            (Some(path), Some(line)) => Note::on_line(
                &files,
                &path,
                entry.side.unwrap_or(NoteSide::New),
                line,
                entry.text,
            ),
            _ => Err(anyhow!(
                "give both a path and a line, or neither for a note on the whole change"
            )),
        };
        match note {
            Ok(note) => notes.push(note),
            Err(error) if flags.json_file.is_some() => {
                problems.push(format!("note {}: {error}", i + 1))
            }
            Err(error) => problems.push(error.to_string()),
        }
    }
    if !problems.is_empty() {
        bail!("{}\nno notes were added", problems.join("\n"));
    }

    let mut store = NoteStore::open(dir).scoped(target.review_key());
    let (mut appended, mut repeated) = (0, 0);
    for note in &notes {
        match store.add(note.clone())? {
            Added::New => {}
            Added::Appended => appended += 1,
            Added::Duplicate => repeated += 1,
        }
    }
    let count = notes.len() - repeated;
    let mut extra = Vec::new();
    if appended > 0 {
        extra.push(format!("{appended} under a note already on its line"));
    }
    if repeated > 0 {
        extra.push(format!("{repeated} already there"));
    }
    println!(
        "Added {count} draft note{} to {}{}.",
        if count == 1 { "" } else { "s" },
        target.label,
        if extra.is_empty() {
            String::new()
        } else {
            format!(" ({})", extra.join(", "))
        }
    );
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("declutter: {error:#}");
            ExitCode::from(2)
        }
    }
}
