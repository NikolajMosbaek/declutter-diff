use std::io::IsTerminal;
use std::process::ExitCode;

use anyhow::{Result, bail};
use declutter::git::{RangeSpec, load};
use declutter::project::CommentMode;
use declutter::review::FileReview;
use declutter::{render, tui};

const USAGE: &str = "\
declutter — review a diff with comments shown, hidden, or on their own

USAGE:
    declutter [OPTIONS] [REVISION | A..B | A...B]

    With no revision, compares HEAD against the working tree, untracked files included.

OPTIONS:
    --staged             Compare against the index instead of the working tree
    --comments <MODE>    Start in MODE: hidden (default), only, shown
    -p, --print          Print the diff instead of opening the viewer
    -h, --help           Show this help
    -V, --version        Show the version

KEYS (viewer):
    c        cycle comments: hidden → only → shown
    n / p    next / previous file
    j / k    scroll; d / u half a page; g / G top / bottom
    q        quit
";

struct Args {
    range: Option<String>,
    staged: bool,
    mode: CommentMode,
    print: bool,
}

fn parse_args(raw: impl Iterator<Item = String>) -> Result<Option<Args>> {
    let mut args = Args {
        range: None,
        staged: false,
        mode: CommentMode::Hidden,
        print: false,
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
            "--comments" => {
                let Some(value) = inline.or_else(|| raw.next()) else {
                    bail!("--comments needs a value: hidden, only or shown");
                };
                let Some(mode) = CommentMode::parse(&value) else {
                    bail!("unknown --comments value `{value}`: use hidden, only or shown");
                };
                args.mode = mode;
            }
            _ if arg.starts_with('-') => bail!("unknown option `{arg}` (see --help)"),
            _ if args.range.is_some() => bail!("only one revision or range can be given"),
            _ => args.range = Some(arg),
        }
    }
    Ok(Some(args))
}

fn run() -> Result<()> {
    let Some(args) = parse_args(std::env::args().skip(1))? else {
        return Ok(());
    };
    let spec = RangeSpec::parse(args.range.as_deref(), args.staged)?;
    let files: Vec<FileReview> = load(&std::env::current_dir()?, &spec)?
        .into_iter()
        .map(FileReview::new)
        .collect();

    if args.print {
        print!("{}", render::plain(&files, args.mode));
        return Ok(());
    }
    if files.is_empty() {
        println!("No changes.");
        return Ok(());
    }
    if !std::io::stdout().is_terminal() {
        bail!("stdout is not a terminal; use --print for text output");
    }
    tui::run(files, args.mode)
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
