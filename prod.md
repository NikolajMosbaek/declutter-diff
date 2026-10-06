# declutter-diff

A code-review tool for AI-generated changes that lets you toggle whole categories of code —
comments, tests, imports — in and out of the diff, so you can review the logic first and the
rest second.

## The problem

AI coding agents produce large diffs, and a big share of every diff is not logic:

- **Comments and docstrings** — often several lines per function, restating what the code does.
- **Tests** — frequently as long as the change itself.
- **Boilerplate** — imports, logging, formatting churn.

All of it lands in one interleaved stream. A reviewer who wants to answer "is the logic
right?" has to read past the rest to find it, and the attention spent skimming noise is
attention not spent on the code that can break production. Today's tools can hide whitespace
and filter by file path, but nothing understands that a line is a comment or that a block is a
test.

## The idea

Treat a diff as layers that can be switched on and off:

| Key | Layer | What it covers |
|-----|-------|----------------|
| `c` | Comments | Line and block comments, doc comments, Python docstrings |
| `t` | Tests | Test files (by path convention) and test blocks inside source files |
| `i` | Imports | `import` / `use` / `#include` statements |
| `l` | Logging | Calls to known logging APIs (later; per-language config) |

Toggle comments off and the diff shows only code changes: comment-only hunks disappear,
and a line that changed code *and* a trailing comment shows only the code change. A status
line always says what is hidden — `14 comment-only hunks, 3 test files hidden` — so nothing
is skipped silently.

The inverse is just as useful: **comments only**. AI-written comments are often stale or
describe what the code was meant to do rather than what it does. Reviewing them on their own,
as a separate pass, is fast and catches a class of error that is easy to miss inline.

## How it works

1. **Parse both sides** of each changed file with [tree-sitter](https://tree-sitter.github.io/),
   which has mature grammars for nearly every language.
2. **Classify** nodes into layers using a small tree-sitter query per language. Comments are
   near-trivial (most grammars name them `comment`, `line_comment`, `block_comment`). Tests
   combine path conventions (`*Tests/`, `*_test.go`, `*.spec.ts`, `test_*.py`) with in-file
   queries (Swift `@Test` / `XCTestCase`, Rust `#[cfg(test)] mod tests`, JS `describe`/`it`).
3. **Project** each side: produce a version of the file with the hidden layers removed, while
   keeping a map from every projected line back to its original line.
4. **Diff the projections**, not the originals. This is the important step — hiding lines in a
   normal diff after the fact gets mixed lines and comment-only hunks wrong. Re-diffing the
   projected text makes those cases fall out naturally.
5. **Render** using the line map, so line numbers, "open in editor" and review comments always
   refer to the real file.

The line map (steps 3–5) is where the real difficulty lives; everything else is assembly of
well-understood parts.

## Shape of the product

A **Rust core library** with front-ends on top:

- **TUI (first)** — `declutter main..HEAD`, `declutter --staged`, a file tree plus diff view,
  single-key layer toggles. Built with [ratatui](https://ratatui.rs/). Fits the terminal
  workflow where AI agents already run.
- **`git difftool` mode** — drop-in for existing git workflows.
- **Machine output** — `--hide comments,tests --format json|patch`, so an agent or script can
  consume a decluttered diff.
- **Later:** a browser extension for GitHub / Azure DevOps PR pages, where most review actually
  happens; possibly an editor extension.

Rust because tree-sitter's primary bindings are Rust, the TUI ecosystem is strong, and a
single static binary is easy to install.

## Prototype scope (v0)

The prototype proves the core idea end to end and nothing more:

- Input: a git revision range or the working tree.
- Layers: **comments** only (including docstrings).
- Languages: **Swift, TypeScript, Python** (JavaScript rides along on the TSX grammar).
- Views: unified diff in a TUI with a comments on / off / only toggle and the hidden-count
  status line.
- Correctness tests for the hard cases: comment-only hunks, code + trailing comment on one
  line, multi-line block comments spanning a hunk boundary, docstrings, files with syntax
  errors.

Done when: reviewing a real AI-generated branch with comments toggled off shows only code
changes, with correct line numbers, and toggling back restores the full diff.

### Decisions made while building v0

- **Syntax errors don't disable a file.** tree-sitter recovers from errors, and real-world
  files often contain constructs a grammar doesn't know. Comments are still collected from
  the recovered tree, and the file is marked *partial parse* so the reviewer knows a comment
  near the error might be missed.
- **Blank lines are layout when comments are hidden.** An added docstring or comment usually
  brings a blank line with it; showing that blank line as a change is noise. With comments
  hidden, added or removed blank lines are not shown as changes (context blank lines stay).
- **Unsupported file types hide nothing** and are counted in the status line
  ("comments not detected in N files").
- **Default mode is comments hidden** — that is the point of opening the tool.

## Roadmap after v0

1. Tests layer (path conventions + in-file test blocks).
2. Imports layer; more languages (Go, Rust, Kotlin, Java, C#).
3. Side-by-side view, file tree with per-file hidden counts, "mark file reviewed".
4. `git difftool` integration and JSON/patch output.
5. Per-repo config (`.declutter.toml`): custom test paths, logging APIs, extra layers defined
   as tree-sitter queries.
6. Browser extension for GitHub and Azure DevOps PRs.

## Non-goals

- **Not an AI reviewer.** It does not summarise, judge or comment on code; it changes what a
  human sees.
- **Not a formatter or linter.** It never modifies source files.
- **Not a replacement for reading the hidden layers.** Hiding is a way to order the review,
  not to skip parts of it — which is why hidden counts are always visible.

## Prior art

- [diffsitter](https://github.com/afnanenayet/diffsitter) — tree-sitter AST diff; can exclude
  node kinds via config, but only leaf nodes, with no live toggle and no notion of tests.
- [difftastic](https://github.com/Wilfred/difftastic) — structural diff; excellent at
  formatting noise, no category filtering.
- [tuicr](https://github.com/agavra/tuicr), turboreview — terminal review tools for AI diffs;
  their "comments" are reviewer notes, not code comments.

None combine a structural parse with reviewer-controlled layer toggles.

## Open questions

- Should "comments off" keep doc comments on public API (they are part of the contract)?
  Possibly a separate `d` layer.
- How to present a hunk where the code change is only meaningful with its comment (e.g. a
  `// SAFETY:` justification)? Likely: hide by default, flag specially-marked comments.
- Name of the binary: `declutter` vs `dd` (taken by coreutils) vs something else.
