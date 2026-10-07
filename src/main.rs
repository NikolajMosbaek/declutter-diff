use std::io::IsTerminal;
use std::process::ExitCode;

use anyhow::{Result, bail};
use declutter::git::{load, repo_root};
use declutter::pr::CliLookup;
use declutter::project::LayerMode;
use declutter::review::{FileReview, Layers};
use declutter::store::{NoteStore, ReviewStore};
use declutter::target::resolve_target;
use declutter::upload::{CliPoster, offer_upload};
use declutter::{render, tui};

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
    declutter notes [--clear]

    A pull request is fetched into refs/declutter/pr/<n>/ (no local branch is touched) and
    shown as its PR page shows it; if the remote publishes no merge ref for it, the `gh` or
    `az` CLI is asked for its branches. A branch that exists only on origin is fetched. The
    main branch is what origin/HEAD points at (else main or master); --base overrides it.
    `notes` prints the review notes left with `m` as one prompt for a coding agent;
    `--clear` deletes them. After reviewing a pull request, declutter lists the notes
    you left and asks before posting them to the PR as line comments (via `az` / `gh`).

OPTIONS:
    --staged             Compare against the index instead of the working tree
    --base <BRANCH>      Compare against BRANCH instead of the main branch
    --comments <MODE>    Start comments in MODE: hidden (default), only, shown
    --tests <MODE>       Start tests in MODE: shown (default), hidden, only
    --imports <MODE>     Start imports in MODE: shown (default), hidden, only
    --logging <MODE>     Start logging statements in MODE: shown (default), hidden, only
    --formatting <MODE>  Start whitespace-only changes in MODE: shown (default), hidden, only
    -p, --print          Print the diff instead of opening the viewer
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

struct Args {
    positionals: Vec<String>,
    base: Option<String>,
    staged: bool,
    layers: Layers,
    print: bool,
    clear: bool,
    syntax: bool,
}

fn parse_args(raw: impl Iterator<Item = String>) -> Result<Option<Args>> {
    let mut args = Args {
        positionals: Vec::new(),
        base: None,
        staged: false,
        layers: Layers::default(),
        print: false,
        clear: false,
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
            "--clear" => args.clear = true,
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
    if args
        .positionals
        .first()
        .is_some_and(|first| first == "notes")
    {
        let mut notes = NoteStore::open(&dir);
        if args.clear {
            let count = notes.notes().len();
            notes.clear()?;
            println!("Cleared {count} note{}.", if count == 1 { "" } else { "s" });
        } else if notes.notes().is_empty() {
            println!("No review notes. Press m on a diff line in the viewer to add one.");
        } else {
            print!("{}", notes.prompt());
        }
        return Ok(());
    }
    if args.clear {
        bail!("--clear only applies to `declutter notes`");
    }
    let target = resolve_target(
        &dir,
        &args.positionals,
        args.staged,
        args.base.as_deref(),
        &CliLookup,
    )?;
    let files: Vec<FileReview> = load(&dir, &target.spec)?
        .into_iter()
        .map(FileReview::new)
        .collect();

    if args.print {
        print!("{}", render::plain(&files, args.layers));
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
            &mut std::io::stdin().lock(),
            &mut std::io::stdout(),
        )?;
    }
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
