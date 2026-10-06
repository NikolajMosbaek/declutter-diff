use std::io::IsTerminal;
use std::process::ExitCode;

use anyhow::{Result, bail};
use declutter::git::{RangeSpec, load};
use declutter::pr::{CliLookup, resolve};
use declutter::project::LayerMode;
use declutter::review::{FileReview, Layers};
use declutter::store::{NoteStore, ReviewStore};
use declutter::{render, tui};

const USAGE: &str = "\
declutter — review a diff with comments and tests shown, hidden, or on their own

USAGE:
    declutter [OPTIONS] [REVISION | A..B | A...B]
    declutter [OPTIONS] pr <URL | NUMBER>
    declutter notes [--clear]

    With no revision, compares HEAD against the working tree, untracked files included.
    `pr` takes a GitHub or Azure DevOps pull-request URL, or a PR number in the repository
    `origin` points at; it fetches the PR (no local branch is touched) and shows what the
    PR page shows. Uses the `gh` or `az` CLI to look the PR up.
    `notes` prints the review notes left with `m` as one prompt for a coding agent;
    `--clear` deletes them.

OPTIONS:
    --staged             Compare against the index instead of the working tree
    --comments <MODE>    Start comments in MODE: hidden (default), only, shown
    --tests <MODE>       Start tests in MODE: shown (default), hidden, only
    --imports <MODE>     Start imports in MODE: shown (default), hidden, only
    --logging <MODE>     Start logging statements in MODE: shown (default), hidden, only
    --formatting <MODE>  Start whitespace-only changes in MODE: shown (default), hidden, only
    -p, --print          Print the diff instead of opening the viewer
    -h, --help           Show this help
    -V, --version        Show the version

KEYS (viewer):
    ↑ / ↓     move through the files, or scroll the diff when it has focus
    → / ←     focus the diff / the file list (also Tab, Enter)
    n / p     next / previous file from either pane
    r         mark the file reviewed and go to the next one (again to unmark)
    m         leave a note on the line under the cursor (in the diff pane)
    E         copy all notes to the clipboard as a prompt for a coding agent
    space     page down; u up; g / G top / bottom
    c         cycle comments: hidden → only → shown
    t         cycle tests: shown → hidden → only
    i / l     cycle imports / logging statements: shown → hidden → only
    w         cycle formatting (whitespace-only) changes: shown → hidden → only
    q         quit
";

struct Args {
    positionals: Vec<String>,
    staged: bool,
    layers: Layers,
    print: bool,
    clear: bool,
}

fn parse_args(raw: impl Iterator<Item = String>) -> Result<Option<Args>> {
    let mut args = Args {
        positionals: Vec::new(),
        staged: false,
        layers: Layers::default(),
        print: false,
        clear: false,
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
    let spec = match args.positionals.as_slice() {
        [pr, target] if pr == "pr" => {
            if args.staged {
                bail!("--staged cannot be combined with `pr`");
            }
            resolve(&dir, target, &CliLookup)?
        }
        [pr] if pr == "pr" => bail!("`pr` needs a pull-request URL or number"),
        [] => RangeSpec::parse(None, args.staged)?,
        [range] => RangeSpec::parse(Some(range), args.staged)?,
        _ => bail!("only one revision or range can be given"),
    };
    let files: Vec<FileReview> = load(&dir, &spec)?
        .into_iter()
        .map(FileReview::new)
        .collect();

    if args.print {
        print!("{}", render::plain(&files, args.layers));
        return Ok(());
    }
    if files.is_empty() {
        println!("No changes.");
        return Ok(());
    }
    if !std::io::stdout().is_terminal() {
        bail!("stdout is not a terminal; use --print for text output");
    }
    tui::run(
        files,
        args.layers,
        ReviewStore::open(&dir),
        NoteStore::open(&dir),
    )
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
