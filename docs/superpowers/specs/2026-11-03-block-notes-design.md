# Block notes: one note on several lines

Date: 2026-11-03 · Status: approved

## Why

Today a note sits on exactly one line (`m`), or on the change as a whole (`P`). Review comments
are often about a few lines together — a guard clause, a loop, a replaced expression — and
pinning them to one of those lines makes the comment on the PR page point at the wrong thing.
The reviewer wants to mark the lines the comment is about, and have the posted comment highlight
that whole range on GitHub and Azure DevOps.

Coding agents add notes too (`declutter notes add`), and their findings often cover several
lines; they should be able to add block notes the same way.

## Goals

- Select a block of lines in the diff and leave one note on it.
- The note shows in the diff covering those lines, and reopens from any of them.
- Posting turns it into one comment that highlights the whole range (an Azure DevOps thread, a
  GitHub multi-line comment).
- `declutter notes add` accepts a line range, on the command line and in `--json`.
- Exported prompts (`E`, `declutter notes`) quote the whole block.

## Non-goals

- File-level notes and free-standing notes. The user dropped these for now; `P` covers the
  change as a whole.
- Blocks across files, or across hunks (see "Calls made").
- Changing how single-line notes look or behave.
- Updating `~/.claude/skills/review-declutter` to seed ranges. It lives outside this repo; it is
  a follow-up once this ships (it maps one `line` per finding and compares `code` to one line).

## Anti-goals

- A posted comment that highlights different lines from the ones the reviewer selected,
  without the reviewer being able to tell.
- Breaking existing `notes.json`, `posted.jsonl`, the `--json` input format, or the refusal
  wording agents already parse (`… is not in the diff; the lines in it are 12–18`).
- Posting drafts. Block drafts follow the existing rule: never posted until opened with `m`.

## Decided by the user

**Selecting.** In the diff, `V` starts a block on the cursor's line; `j`/`k` grow or shrink it
(vim's linewise visual mode: the start stays put, the cursor moves, the block is everything
between). Selected lines are highlighted and the status line shows the range:

```
-- BLOCK  cart.ts:11–13 (3 lines) · j/k extend · m note · Esc cancel --
```

`Esc` cancels the selection (before Esc's current meaning of leaving the diff). A block stays
inside one file.

**Writing.** `m` with a block selected opens the same note editor as today, titled for the range
(` Note on cart.ts:11–13 `). Saving creates one note covering the block; saving it empty deletes
it, as now. The selection ends when the editor opens.

**Showing — option A.** A yellow `▎` beside every line of the block, and the note under the
block's last line, its text prefixed with the range. Single-line notes stay exactly as they are
(no range prefix) — matching them is why A was chosen.

```
   10    10   export function applyDiscount(cart: Cart, percent: number): Cart {
         11 +▎  if (percent < 0 || percent > 100) {
         12 +▎    throw new RangeError("percent must be 0–100");
         13 +▎  }
              ✎ 11–13 · Should 100% be allowed? This lets a free cart through.
   11    14     const factor = 1 - percent / 100;
```

The bar takes the column of the space after `+`/`-`/` `, so code does not shift. Drafts keep
today's dimmed italic style: `✎ draft · 11–13 · …`.

**Reopening.** `m` on any line of the block opens its note.

**Hidden lines.** The block covers the real file's lines from its first to its last, including
lines hidden by a layer. Selecting 11–18 with comments hidden makes a note on 11–18, and that is
what the PR page highlights. When layers hide some of a block's lines, the bar marks the visible
ones and the note sits under the last visible one; when all are hidden, the note is hidden the way
a note on a hidden line is today.

**Exporting.** `E` and `declutter notes` show the note as `` `cart.ts:11–13` `` followed by every
line of the block (hidden ones included), not just one.

**Posting.** One comment per block note, highlighting the range: an Azure DevOps thread anchored
to the range, a GitHub multi-line comment.

**Agents.** `declutter notes add` takes a range (below), so review findings that cover several
lines arrive as block drafts.

## Blocks that take in removed lines along with others

**Decided by the user: allowed.**

**What a block is** (my call): every diff row from its first row to its last, in diff order, as
the diff shows them with every layer shown — what you see selected is what the note covers. Each
end is a (side, line); unchanged lines count as new-side, as today.

In a replace hunk the natural block runs from a removed line into the added lines that replace
it, or from an unchanged line above a replaced section to one below it, taking in removed and
added rows on the way. A note today has one side; such blocks need both. GitHub supports them
directly (`start_side` / `side`, each end its own side). Azure DevOps takes a left range and a
right range in one thread context; whether it accepts both at once is unverified, and the user
will check it live after the build (see "Posting" for how the code copes either way).

## Design

### Data model (`src/store.rs`)

`Note` keeps `side` and `line` as the block's **first** line and gains two optional fields for
its last line:

```rust
/// The block's last line, when the note covers more than one.
#[serde(default, skip_serializing_if = "Option::is_none")]
pub end_line: Option<usize>,
/// The last line's side, when it differs from the first's (a block from removed lines
/// into added ones).
#[serde(default, skip_serializing_if = "Option::is_none")]
pub end_side: Option<NoteSide>,
```

A missing `end_line` means a single-line note, so old `notes.json` and `posted.jsonl` load
unchanged and single-line notes serialise exactly as before. Helpers for "last line" (defaulting
to `side`/`line`) and "covers (side, line)" keep the rest of the code from touching the options.

- **`code`** holds every row of the block from the diff with every layer shown, joined with
  `\n`. When the block takes in removed rows along with others, each row keeps its `-`/`+`/` `
  marker so the export reads as a diff; otherwise plain lines, as today.
- **Per-side spans.** Azure DevOps needs, for a mixed block, the span of its removed rows (old
  numbering) and the span of its other rows (new numbering) — values the two ends alone do not
  give, and `declutter notes post` does not load the diff. Record both spans on the note when it
  is made (additive, optional fields; showing up in `notes --json` is fine), or recompute them at
  posting time from the diff — builder's choice. Either way, old notes must still load.
- **Coverage** (which rows get the `▎`, and which rows `m` reopens the note from) is every row
  between the two ends in diff order, so it is worked out in the viewer from the diff's rows, not
  from line numbers alone.
- **Identity** (`same_place`): review, path, and both ends. Two notes with the same start and
  different ends are different notes.
- **`find`** stays exact (by both ends). The viewer finds the note covering a row from its
  coverage, for `m` and rendering (rules under "Calls made").
- **`place()`**: `` `cart.ts:11–13` ``; for removed lines `` `cart.ts` (removed lines 11–13) ``;
  across sides, builder's choice of a clear form (e.g. `` `cart.ts` removed line 12 to line 16 ``).
  `place()` already feeds the post listing, the "could not post" errors and the drafts list, so
  they pick this up for free.
- **`prompt()`**: the code fence must indent every code line (today it assumes one).
- Sorting by `(path, line)` stays as is.

### Viewer (`src/tui.rs`)

- New state: the selection's start, held in file terms (side + line, like `Place`) — not as a
  screen row — so a view rebuild cannot silently move it. The block is the rows from the start to
  the cursor, either direction.
- `V` from the file list moves into the diff first, as `m` does.
- Cursor moves while selecting (`j k ↑ ↓`, paging, `g G`, `] [`, search) extend the block but stop
  at the edges of the hunk it started in. Leaving the file (`} {`, Tab, `←`, the file list)
  cancels it. Layer toggles and `M` may cancel it too; builder's choice.
- Selected rows get a highlight distinct from the cursor line; the status line shows the
  `-- BLOCK …` text above in place of the usual help.
- Rendering: for each row, draw the `▎` if any note's block covers that row's (side, line); draw
  a block note under the last visible row of its block.
- Help (`?`) gains `V  select lines for a note`; the `KEYS` section of `--help` gains the same.

### Command line and agents (`src/main.rs`)

- `--line 11-13` (also accept `11–13`; print an en dash). `--old` applies to both ends. A single
  number works as today.
- `--json` entries gain an optional `end_line`: `{"path", "line", "end_line", "side", "text"}`.
  `line` stays a number, so existing input keeps working.
- Agents cannot add a block across sides (my call: their findings cite one side; the viewer
  covers the mixed case).
- Validation, against the diff with every layer shown: `line` and `end_line` must both be diff
  rows on that side, in the same hunk, with `line ≤ end_line` (equal is a single-line note).
  Since a block is the rows between its ends, checking the ends is enough; an `--old` block may
  take in unchanged rows between two removed ones, as a block selected in the viewer would.
  Refusals keep today's wording and list the lines in the diff, e.g.
  `` `cart.ts:11–40` is not in the diff; the lines in it are 11–18, 35–44 ``.
- `notes add` on a block whose exact range already has a note appends under it (today's
  `Added::Appended`); an overlapping but different range is a new note.
- The usage text documents the range and `end_line`.

### Posting (`src/upload.rs`)

- **Azure DevOps.** `rightFileStart` = first line, `rightFileEnd` = last line (left for removed
  lines). The end offset must cover the last line: today's `1 → 2` marks only a line's start, so
  use the last line's length + 1 (from `code`). For a mixed block, send both ranges. If Azure
  DevOps refuses that thread, post it again with the right range only, and say so in the
  posting output (e.g. `` posted `cart.ts:11–16`; Azure DevOps highlights 14–16 only ``) — never
  by adding text to the posted comment.
- **GitHub.** In the review body and in the single-comment fallback, add `start_line` and
  `start_side` when the block has more than one line (GitHub rejects `start_line` equal to
  `line`). `line`/`side` are the last line's. GitHub refuses a range outside one of its hunks;
  the existing fallback then posts on the file, its text starting `Lines 11–13:` (or
  `Removed lines 11–13:`).
- Block ranges have never been posted to a live host. The user spot-checks one block note on a
  real Azure DevOps PR and one on GitHub after the build; the builder posts nothing live.

### Docs

README's notes section, `prod.md` ("Built since v0" and the decisions list), and the help texts.

## Calls made (mine, not the user's)

1. **A block stays inside one hunk**, not just one file. Lines between hunks are not in the
   diff, so GitHub cannot highlight them and the viewer has nowhere to draw them.
2. **Overlapping notes are allowed.** `V…m` always addresses exactly the selected range: it opens
   the note with that exact range, or starts a new one. `V` then `m` on one line makes a
   single-line note, even inside a block. Plain `m` on a line opens that line's own note if it has
   one, else the smallest block covering the line, else starts a single-line note.
3. **Field names** `end_line` / `end_side`, with `line` as the first line — so an agent writes
   `"line": 11, "end_line": 13` in reading order.
4. **Agents add single-side blocks only.**
5. **A block is every diff row between its two ends**, so coverage, the bar and the exported
   code include removed rows between two unchanged ones.
6. **If Azure DevOps refuses both sides, the thread falls back to the right side and the
   reviewer is told** in the posting output; the posted text stays exactly what they wrote.
7. **The viewer does not re-check a reviewer's block against the full diff.** A block selected
   in a decluttered view can, rarely, take in lines outside the full diff's hunks; it is saved as
   selected, and on GitHub falls back to a file comment when posted.

## Left to the builder

- Exact highlight colours for selected rows and the bar; the label form for a block across
  sides.
- Whether layer toggles and `M` keep or cancel a selection.
- Internal structure: helper names, how `DiffView` records block coverage.

## Done when

- Tests cover: selecting and saving a block; reopening from a middle line; overlap rules; hidden
  lines inside a block; old notes loading; `--line N-M` and `end_line` accepted and refused with
  the existing wording; the prompt quoting every line; the Azure DevOps thread body and the GitHub
  review/comment arguments for a block, including the file fallback text and the Azure DevOps
  right-side retry for a mixed block.
- `cargo test` and `cargo clippy` pass.
- Handed to the user for a live spot-check on Azure DevOps and GitHub; nothing posted by the
  builder.
