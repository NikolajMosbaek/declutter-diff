# declutter-diff

Review AI-generated diffs with the comments and tests toggled out — or with nothing *but* them.
See [prod.md](prod.md) for the idea, the design and the roadmap.

## Status

Prototype: five layers, each shown, hidden or shown on its own.

- **Comments** (and Python docstrings) in Swift, TypeScript/TSX, JavaScript and Python.
- **Tests**, per file: test directories and file-name conventions in any language
  (`Tests/`, `FooTests/`, `__tests__/`, `FooTests.swift`, `foo.test.ts`, `test_foo.py`, …),
  plus Swift, Python and TypeScript files that import a test framework.
- **Imports**: `import` statements (and `export … from` re-exports).
- **Logging**: statements that only log — `print(…)`, `NSLog`, `os_log`, `console.*(…)`, and
  `debug/info/warn/error/…` calls on a `logger`, `log`, `logging` or `console` receiver.
- **Formatting**: changes that only move whitespace — re-indenting, re-spacing, re-wrapping
  a statement over more or fewer lines, added blank lines. When hidden, the new layout stays
  visible as context. Indentation counts as code in Python and in files without a grammar.

## Build

```sh
cargo build --release
# binary: target/release/declutter
```

## Use

Run inside a git repository:

```sh
declutter                    # HEAD vs working tree, untracked files included
declutter --staged           # HEAD vs index
declutter main               # main vs working tree
declutter main..feature      # two revisions
declutter main...feature     # feature vs its merge base with main
declutter pr 42               # PR 42 of the repository origin points at
declutter pr <PR URL>         # a GitHub or Azure DevOps pull request
declutter --print            # print instead of opening the viewer
declutter -p --comments only # print only the comment changes
declutter --tests hidden     # start with test files left out
declutter --logging only     # start with only the logging changes
```

Viewer keys:

| Key | Action |
|-----|--------|
| `↑` / `↓` (or `k` / `j`) | move through the files, or move the line cursor when the diff has focus |
| `→` / `Enter`, `←` (or `Tab`) | focus the diff, back to the files |
| `n` / `p` | next / previous file from either pane |
| `Space` / `PgDn`, `u` / `PgUp` | page the diff |
| `g` / `G` | top / bottom of the diff |
| `r` | mark the file reviewed and jump to the next unreviewed one (again to unmark) |
| `m` | leave a note on the line under the cursor (diff pane) |
| `E` | copy all notes to the clipboard as one prompt for a coding agent |
| `c` | cycle comments: hidden → only → shown |
| `t` | cycle tests: shown → hidden → only |
| `i` / `l` | cycle imports / logging: shown → hidden → only |
| `w` | cycle formatting-only changes: shown → hidden → only |
| `v` | collapse moved blocks to their one-line marker, or expand them |
| `o` | open the file in `$VISUAL` / `$EDITOR` at the cursor's line |
| `q` | quit (`Esc` leaves the diff first) |

Layers combine: everything set to *hidden* is cut out together. A layer set to *only*
wins — with comments on *only* you see just the comment changes, whatever else is hidden.

**Moved code.** A block of three or more lines removed in one place and added in another —
same file or a different one, re-indented or not — is drawn in its own tint with a marker
(`⇄ 14 lines moved to Checkout.swift:173`) at each end, in the viewer and in `--print`.

The status line always says what is hidden, e.g.
`comments: hidden · tests: hidden · showing 7 of 12 hunks · 5 comment-only hunks hidden · 4 test files hidden`.

`pr` looks the pull request up with the `gh` (GitHub) or `az` (Azure DevOps) CLI, so
whichever account those are signed in with is used. It fetches the PR's branches into
`refs/declutter/pr/<n>/` — no local branch is created or moved — and shows the PR branch
against its merge base, the same diff the PR page shows.

Review marks are saved per repository in `.git/declutter/reviewed.tsv` (shared by
worktrees, never committed). A mark belongs to the exact change, so a file you reviewed
shows up unreviewed again as soon as a new commit touches it.

Notes are kept in `.git/declutter/notes.json` until you clear them. `E` copies them as
a prompt ("Please address these review comments… 1. `path:line`: note") and also saves it
to `.git/declutter/review-notes.md`; `declutter notes` prints the same prompt and
`declutter notes --clear` deletes the notes.

`o` knows the line syntax of VS Code (and Cursor/Windsurf), Sublime, Zed, Helix, Vim/Neovim,
nano, Emacs, micro, Xcode (`xed`) and JetBrains IDEs; terminal editors take over the screen
until you quit them. With no editor set it uses the system opener. It opens the working-tree
file, which is the reviewed version unless you're reviewing an older range.

## Test

```sh
cargo test
```
