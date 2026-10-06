# declutter-diff

Review AI-generated diffs with the comments toggled out — or with nothing *but* the comments.
See [prod.md](prod.md) for the idea, the design and the roadmap.

## Status

Prototype (v0): the comment layer, for Swift, TypeScript/TSX, JavaScript and Python.

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
```

Viewer keys:

| Key | Action |
|-----|--------|
| `c` | cycle comments: hidden → only → shown |
| `n` / `p` | next / previous file |
| `j` / `k` | scroll |
| `d` / `u` | half page down / up |
| `g` / `G` | top / bottom |
| `q` | quit |

The status line always says what is hidden, e.g.
`comments: hidden · showing 7 of 12 hunks · 5 comment-only hunks hidden · 2 comment-only files`.

## Test

```sh
cargo test
```
