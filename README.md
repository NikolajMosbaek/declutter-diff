# declutter-diff

Review AI-generated diffs with the comments and tests toggled out — or with nothing *but* them.
See [prod.md](prod.md) for the idea, the design and the roadmap.

## Status

Prototype: two layers.

- **Comments** (and Python docstrings) in Swift, TypeScript/TSX, JavaScript and Python.
- **Tests**, per file: test directories and file-name conventions in any language
  (`Tests/`, `FooTests/`, `__tests__/`, `FooTests.swift`, `foo.test.ts`, `test_foo.py`, …),
  plus Swift, Python and TypeScript files that import a test framework.

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
declutter --print            # print instead of opening the viewer
declutter -p --comments only # print only the comment changes
declutter --tests hidden     # start with test files left out
```

Viewer keys:

| Key | Action |
|-----|--------|
| `↑` / `↓` (or `k` / `j`) | move through the files, or scroll the diff when it has focus |
| `→` / `Enter`, `←` (or `Tab`) | focus the diff, back to the files |
| `n` / `p` | next / previous file from either pane |
| `Space` / `PgDn`, `u` / `PgUp` | page the diff |
| `g` / `G` | top / bottom of the diff |
| `c` | cycle comments: hidden → only → shown |
| `t` | cycle tests: shown → hidden → only |
| `q` | quit (`Esc` leaves the diff first) |

The status line always says what is hidden, e.g.
`comments: hidden · tests: hidden · showing 7 of 12 hunks · 5 comment-only hunks hidden · 4 test files hidden`.

## Test

```sh
cargo test
```
