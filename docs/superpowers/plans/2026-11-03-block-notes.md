# Block Notes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** One review note on a block of lines: selected with `V` in the viewer or given as a range by `declutter notes add`, drawn beside those lines, quoted whole in exported prompts, and posted as one comment that highlights the range on Azure DevOps and GitHub.

**Architecture:** `Note` keeps `side`/`line` as the block's first line and gains optional fields for its last line and, for blocks that take in removed rows along with others, the span of lines on each side — so "does this note cover this row" is a pure method and posting never needs the diff. A new `src/block.rs` builds block notes from a file's full diff (every layer shown). The viewer adds a selection held in file terms, draws a `▎` beside covered rows and the note under the last visible one; the posters add range anchors.

**Tech Stack:** Rust 2024 (rust-version 1.90), ratatui, serde/serde_json, `similar` diffs. No new dependencies.

**Spec:** `docs/superpowers/specs/2026-11-03-block-notes-design.md` — read it before any task.

## Global Constraints

- No new dependencies. Edition 2024, rust-version 1.90.
- `cargo test` and `cargo clippy --all-targets` pass with zero warnings after every task. If `cargo` is not on `PATH`: `export PATH="$HOME/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$PATH"`.
- Every range shown to a person uses an en dash, U+2013: `11–13`. Input accepts `-` and `–`.
- Old `notes.json` / `posted.jsonl` (no block fields) load unchanged; a single-line note serialises exactly as before (every new field is `#[serde(default, skip_serializing_if = "Option::is_none")]`).
- A one-line block **is** a line note: any constructor given first == last returns `end_line: None` (and no other block field).
- Single-line notes look and behave exactly as today, including the viewer storing the decluttered row text as their `code`. Only blocks go through `src/block.rs`.
- Status line while selecting, exactly: `-- BLOCK  cart.ts:11–13 (3 lines) · j/k extend · m note · Esc cancel --` (two spaces after `BLOCK`; the place is `Note::place()` without backticks).
- In-diff note prefix: `✎ 11–13 · ` and, for a draft, `✎ draft · 11–13 · `. Single-line notes keep `✎ ` / `✎ draft · `.
- Refusals keep today's wording: `` `src/rate.ts:5–25` is not in the diff; the lines in it are 2–8, 22–28 ``; existing single-line refusal tests must pass unchanged.
- GitHub file-fallback text: `Lines 11–13: …`, `Removed lines 11–13: …`, `Removed line 3 to line 8: …`; `start_line`/`start_side` only when the note is a block.
- Azure DevOps end offset of a block = the last line's `chars().count() + 1`, at least 2.
- Drafts are never posted. Nothing is posted to a live host by the builder.
- Commits: conventional prefix (`feat:`, `docs:`), message ends with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

## Review Focus

1. A `notes.json` written by 0.1.4 loads, draws and lists exactly as before, and re-saving it adds no fields — test in Task 1.
2. A block whose last line is hidden by a layer still shows its note, under the last visible line it covers — test in Task 4.
3. Reopening a block note from a middle line and pressing Enter keeps its range (it must not collapse onto the cursor's line) — test in Task 4.
4. A selection in progress when the reviewer leaves the file (`}`, `]` into the next file, Tab) or toggles a layer is cancelled, never saved onto another file — test in Task 5.
5. `--line 15-15`, and `V` then `m` without moving, give a note identical to `--line 15` / `m` — tests in Tasks 2 and 5.

## Shared test fixture

Tasks 4 and 5 use this file (`cart.ts`, TypeScript). With `Layers::default()` (comments hidden) the viewer shows, by pane line:

| pane line | row |
|---|---|
| 0 | `@@ -1 +1 @@` |
| 1 | context 1/1 `export function applyDiscount(cart, percent) {` |
| 2 | added 2 `  if (percent < 0 \|\| percent > 100) {` ← cursor starts here |
| 3 | added 4 `    throw new RangeError("percent must be 0-100");` |
| 4 | added 5 `  }` |
| 5 | context 2/6 `  const factor = 1 - percent / 100;` |
| 6 | removed 3 `  return { ...cart, total: cart.total * factor };` |
| 7 | added 7 `  const total = Math.round(cart.total * factor * 100) / 100;` |
| 8 | added 8 `  return { ...cart, total };` |
| 9 | context 4/9 `}` |

New line 3, `    // A discount outside 0-100 is a bug upstream.`, is hidden. Put in `tests/block_notes.rs`:

```rust
const OLD: &str = "export function applyDiscount(cart, percent) {\n  const factor = 1 - percent / 100;\n  return { ...cart, total: cart.total * factor };\n}\n";
const NEW: &str = "export function applyDiscount(cart, percent) {\n  if (percent < 0 || percent > 100) {\n    // A discount outside 0-100 is a bug upstream.\n    throw new RangeError(\"percent must be 0-100\");\n  }\n  const factor = 1 - percent / 100;\n  const total = Math.round(cart.total * factor * 100) / 100;\n  return { ...cart, total };\n}\n";

fn cart() -> FileReview  // FileReview::new(FileChange { path: "cart.ts", Modified, old: OLD, new: NEW, .. })
fn cart_app(notes: NoteStore) -> App  // App::with_stores(vec![cart()], Layers::default(), ReviewStore::in_memory(), notes)
```

`cart_app` asserts the table above against `screen(...)` (pane line 2 contains `2 +  if (percent`, line 6 contains `3       -  return`) so a diff-alignment surprise fails in the helper, not mid-keystroke.

---

### Task 1: The block note model and its lookups

**Files:**
- Modify: `src/store.rs` (`Note`, `NoteStore::{find, set, add, prompt}`, `same_place`, `place`)
- Modify (struct literals gain the new fields as `None`): `src/store.rs:282,377,429`, `src/tui.rs:674,687`, `tests/notes.rs:15,113,181,235`, `tests/upload.rs:34,248`
- Create: `tests/block_notes.rs`

**Interfaces:**
- Produces:
  - `pub type End = (NoteSide, usize);` in `store.rs`
  - `Note` fields, after `line`: `end_side: Option<NoteSide>` (only when it differs from `side`), `end_line: Option<usize>`, `old_lines: Option<(usize, usize)>`, `new_lines: Option<(usize, usize)>` (both spans set only when the block takes in removed rows along with others)
  - `Note::first(&self) -> End`, `Note::last(&self) -> End`, `Note::is_block(&self) -> bool`
  - `Note::span(&self, side: NoteSide) -> Option<(usize, usize)>` — the stored span when set; else, for the note's own side, `(line, end_line.unwrap_or(line))`; else `None`
  - `Note::covers(&self, side: NoteSide, line: usize) -> bool`
  - `Note::size(&self) -> usize` — lines across both spans
  - `Note::label(&self) -> String` — in-diff range: `11–13`, `removed 11–13`, `removed 3 to 8`; empty for a line note
  - `Note::lines_text(&self) -> String` — `Line 9`, `Removed line 9`, `Lines 11–13`, `Removed lines 11–13`, `Removed line 3 to line 8`
  - `Note::place(&self)` extended: `` `a.ts:11–13` ``, `` `a.ts` (removed lines 11–13) ``, `` `a.ts` (removed line 3 to line 8) ``
  - `NoteStore::find(path, side, line)` — single-line notes only (unchanged signature)
  - `NoteStore::find_block(&self, path: &str, first: End, last: End) -> Option<&Note>` — exact range, line notes included
  - `NoteStore::at_line(&self, path: &str, side: NoteSide, line: usize) -> Option<&Note>` — the line's own note, else the smallest (`size()`) block covering it, ties to the earlier in the store

- [ ] **Step 1: Write the failing tests** in `tests/block_notes.rs`

```rust
fn block(first: End, last: End, spans: Option<((usize, usize), (usize, usize))>, text: &str) -> Note
// a Note on "a.ts" with end_side/end_line from `last`, old_lines/new_lines from `spans`, code "x"

#[test]
fn old_notes_without_block_fields_still_load() {
    // write this 0.1.4 notes.json, open it, find it, set it again, re-read the file
    let json = r#"[{"path":"a.ts","side":"new","line":3,"code":"let a = 2","text":"hi","draft":true}]"#;
    // find("a.ts", New, 3) is Some with text "hi"; after set(), the file contains no "end_line",
    // "end_side", "old_lines" or "new_lines"
}

#[test]
fn a_block_is_found_from_any_line_it_covers_and_a_line_note_wins() {
    // store: block (New,11)–(New,13) "block"; line note New 12 "line"
    assert_eq!(store.at_line("a.ts", New, 11).map(|n| n.text.as_str()), Some("block"));
    assert_eq!(store.at_line("a.ts", New, 12).map(|n| n.text.as_str()), Some("line"));
    assert_eq!(store.at_line("a.ts", New, 14), None);
    assert_eq!(store.find("a.ts", New, 11), None);
    assert!(store.find_block("a.ts", (New, 11), (New, 13)).is_some());
}

#[test]
fn the_smallest_block_covering_a_line_wins() {
    // blocks 10–20 "outer" and 11–13 "inner": at_line(12) is "inner", at_line(15) is "outer"
}

#[test]
fn a_mixed_block_covers_each_side_by_its_own_span() {
    // block (Old,3)–(New,8), spans old (3,3) new (7,8)
    // covers(Old,3) && covers(New,7) && covers(New,8); !covers(New,6) && !covers(Old,4)
}

#[test]
fn one_range_holds_one_note_and_another_range_is_another_note() {
    // set block 11–13 "a", set block 11–13 "b" → 1 note, text "b";
    // set block 11–14 "c" → 2 notes; add(block 11–13 "d") → Added::Appended, text "b\n\nd"
}

#[test]
fn blocks_name_their_lines() {
    // (note, place, label, lines_text)
    // line New 9:            "`a.ts:9`", "", "Line 9"
    // block New 11–13:       "`a.ts:11–13`", "11–13", "Lines 11–13"
    // block Old 11–13:       "`a.ts` (removed lines 11–13)", "removed 11–13", "Removed lines 11–13"
    // block (Old,3)–(New,8): "`a.ts` (removed line 3 to line 8)", "removed 3 to 8", "Removed line 3 to line 8"
}

#[test]
fn the_prompt_quotes_every_line_of_a_block() {
    // block New 11–13 with code "  if x {\n    y()\n  }" and text "why?"
    assert_eq!(store.prompt(),
        "Please address these review comments. Line numbers refer to the version under review.\n\
         \n1. `a.ts:11–13`: why?\n   ```\n     if x {\n       y()\n     }\n   ```\n");
}
```

- [ ] **Step 2: Run them; they fail to compile** — `cargo test --test block_notes` → errors naming `End`, `end_line`, `at_line`.

- [ ] **Step 3: Implement the Interfaces above in `src/store.rs`.** `same_place` compares review, path, `first()` and `last()`. In `prompt()`, a line note keeps `code.trim()`; a block indents each `trim_end()`ed line of `code` by three spaces. Add the four new fields as `None` at every struct literal listed under Files.

- [ ] **Step 4: Run** `cargo test` → all pass, including the untouched `tests/notes.rs` and `tests/upload.rs`.

- [ ] **Step 5: Commit** — `feat: notes that cover a block of lines, in the store`

---

### Task 2: Building a block note from the diff

**Files:**
- Create: `src/block.rs` (and `pub mod block;` in `src/lib.rs`)
- Modify: `src/review.rs` (source accessors), `src/store.rs` (`Note::on_line` becomes a wrapper; `ranges()` becomes `pub(crate)`)
- Test: `tests/note_add.rs`, `tests/block_notes.rs`

**Interfaces:**
- Consumes: Task 1's `End` and `Note` fields.
- Produces:
  - `FileReview::source(&self, old: bool) -> &str` in `review.rs` — the old or new version's text
  - `impl Note` in `block.rs`: `pub fn block(file: &FileReview, first: End, last: End, text: impl Into<String>) -> Note` — never fails; not a draft
  - `impl Note` in `block.rs`: `pub fn on_block(files: &[FileReview], path: &str, side: NoteSide, first: usize, last: usize, text: impl Into<String>) -> Result<Note>` — validated; a draft
  - `Note::on_line(files, path, side, line, text)` = `Note::on_block(files, path, side, line, line, text)`

What a block covers (both constructors): take the rows of `file.view(DiffModes::SHOWN)` in order across hunks; an end matches a row when `Old` ↔ a `Removed` row's `old_line`, `New` ↔ an `Added`/`Context` row's `new_line`. The block is the rows from the first end's row to the last end's row.
- If those rows mix `Removed` with others: `code` is each row as `-text` / `+text` / ` text` joined by `\n`; `old_lines` = min/max `old_line` of the removed rows; `new_lines` = min/max `new_line` of the others.
- Otherwise: `code` is the side's source lines `first..=last` (so lines between, hidden or not, are quoted); no spans.
- If an end matches no row (only possible from the viewer, whose decluttered context can lie outside the full diff): treat the block as single-side on the first end's side, `code` from that side's source lines `first..=last`. `Note::block` never panics.
- first == last: a line note — `end_line`, `end_side` and spans `None`, `code` the line's text.

`on_block` refuses, in this order, with `ranges()` for the listed lines:
- `first > last`: `` `src/rate.ts:16–14` ends before it starts ``
- an end not on that side in the diff, or the ends in different hunks: today's message with the range as the place — `` `src/rate.ts:5–25` is not in the diff; the lines in it are 2–8, 22–28 ``; removed side: `removed lines 15–16 of `src/rate.ts` are not in the diff; the removed lines in it are 15`
- a single line: today's messages, byte for byte.

- [ ] **Step 1: Write the failing tests.** In `tests/note_add.rs` (fixture `change()`: 30 lines, line 15 changed, hunk 12–18), plus a `two_hunks()` fixture with lines 5 and 25 changed (`const v5 = 5 * 2;`, `const v25 = 25 * 2;`):

```rust
#[test]
fn a_block_takes_every_line_between_its_ends() {
    let note = Note::on_block(&change(), "src/rate.ts", NoteSide::New, 12, 14, "x").expect("in the diff");
    assert_eq!((note.end_line, note.old_lines, note.new_lines), (Some(14), None, None));
    assert_eq!(note.code, "const v12 = 12;\nconst v13 = 13;\nconst v14 = 14;");
    assert!(note.draft);
}

#[test]
fn a_block_across_the_change_quotes_it_as_a_diff() {
    let note = Note::on_block(&change(), "src/rate.ts", NoteSide::New, 14, 16, "x").expect("in the diff");
    assert_eq!((note.old_lines, note.new_lines), (Some((15, 15)), Some((14, 16))));
    assert_eq!(note.code, " const v14 = 14;\n-const v15 = 15;\n+const v15 = 15 * 2;\n const v16 = 16;");
}

#[test]
fn a_one_line_block_is_a_line_note() {
    let note = Note::on_block(&change(), "src/rate.ts", NoteSide::New, 15, 15, "x").unwrap();
    assert_eq!((note.end_side, note.end_line, note.old_lines, note.new_lines), (None, None, None, None));
    assert_eq!(note.code, "const v15 = 15 * 2;");
}

#[test]
fn a_block_outside_one_hunk_is_refused_with_the_lines_in_the_diff() {
    // on_block(two_hunks(), New, 5, 25) → "`src/rate.ts:5–25` is not in the diff; the lines in it are 2–8, 22–28"
    // on_block(change(), New, 16, 14) → "`src/rate.ts:16–14` ends before it starts"
    // on_block(change(), Old, 15, 16) → "removed lines 15–16 of `src/rate.ts` are not in the diff; the removed lines in it are 15"
}
```

In `tests/block_notes.rs`:

```rust
#[test]
fn a_block_from_the_viewer_quotes_lines_hidden_by_a_layer() {
    let note = Note::block(&cart(), (NoteSide::New, 2), (NoteSide::New, 5), "x");
    assert_eq!(note.code.lines().count(), 4);
    assert!(note.code.contains("// A discount outside 0-100"));
    assert!(!note.draft);
}

#[test]
fn a_block_from_a_removed_line_into_added_ones_records_both_sides() {
    let note = Note::block(&cart(), (NoteSide::Old, 3), (NoteSide::New, 8), "x");
    assert_eq!((note.end_side, note.end_line), (Some(NoteSide::New), Some(8)));
    assert_eq!((note.old_lines, note.new_lines), (Some((3, 3)), Some((7, 8))));
    assert_eq!(note.code, "-  return { ...cart, total: cart.total * factor };\n+  const total = Math.round(cart.total * factor * 100) / 100;\n+  return { ...cart, total };");
}
```

- [ ] **Step 2: Run** `cargo test --test note_add --test block_notes` → fails to compile (`on_block`, `block` missing).
- [ ] **Step 3: Implement** the Interfaces and rules above.
- [ ] **Step 4: Run** `cargo test` → all pass, including the existing `a_line_outside_the_diff_is_refused_with_the_lines_that_are_in_it`.
- [ ] **Step 5: Commit** — `feat: build a block note from the full diff`

---

### Task 3: Ranges from the command line

**Files:**
- Modify: `src/main.rs` (`NoteFlags`, `NoteEntry`, `parse_args`, `add_notes`, `USAGE`)
- Modify: `README.md` ("Notes from a coding agent")
- Test: `tests/note_add.rs`

**Interfaces:**
- Consumes: `Note::on_block` (Task 2).
- Produces: `NoteFlags.end_line: Option<usize>`; `NoteEntry.end_line: Option<usize>` (`#[serde(default)]`). `--line` takes `N`, `N-M` or `N–M`; anything else fails with `--line needs a line number, or a range such as 11-13`. An entry with `end_line` but no `line` falls into the existing "give both a path and a line…" error.

- [ ] **Step 1: Write the failing test** (uses the file's `repo()`, `declutter()`, `listed()`):

```rust
#[test]
fn notes_add_takes_a_range_of_lines() {
    let dir = repo();
    let (ok, out, err) = declutter(dir.path(),
        &["notes", "add", "feature", "--path", "src/rate.ts", "--line", "12-14", "--text", "Three lines."], None);
    assert!(ok, "{err}");
    assert_eq!(out, "Added 1 draft note to main...feature.\n");

    let (ok, out, err) = declutter(dir.path(), &["notes", "add", "feature", "--json", "-"],
        Some(r#"[{"path": "src/rate.ts", "line": 12, "end_line": 14, "text": "Same lines."}]"#));
    assert!(ok, "{err}");
    assert_eq!(out, "Added 1 draft note to main...feature (1 under a note already on its line).\n");

    let notes = listed(dir.path(), &["feature"])["notes"].clone();
    assert_eq!(notes.as_array().map(Vec::len), Some(1));
    assert_eq!((&notes[0]["line"], &notes[0]["end_line"]), (&json!(12), &json!(14)));

    let (ok, _, err) = declutter(dir.path(),
        &["notes", "add", "feature", "--path", "src/rate.ts", "--line", "15–15", "--text", "One."], None);
    assert!(ok, "{err}");
    assert!(listed(dir.path(), &["feature"])["notes"][1].get("end_line").is_none(),
        "a one-line range is a line note");

    let (ok, _, err) = declutter(dir.path(),
        &["notes", "add", "--path", "src/rate.ts", "--line", "12-", "--text", "x"], None);
    assert!(!ok);
    assert!(err.contains("--line needs a line number, or a range such as 11-13"), "{err}");
}
```

- [ ] **Step 2: Run** `cargo test --test note_add notes_add_takes_a_range_of_lines` → FAIL (`--line needs a line number`).
- [ ] **Step 3: Implement.** In `USAGE`: `declutter notes add [TARGET] [--path FILE --line N[-M] [--old]] --text TEXT`; in the `notes add` paragraph say `--line 11-13` notes a block and `--json` entries take an optional `"end_line"`. README: add a `--line 11-13` example line and `"end_line"` to the `--json` comment.
- [ ] **Step 4: Run** `cargo test` → all pass.
- [ ] **Step 5: Commit** — `feat: notes add takes a range of lines`

---

### Task 4: The viewer draws block notes and reopens them

**Files:**
- Modify: `src/tui.rs` (`row_line`, `diff_view_of`, `note_lines`, `open_note`, `save_note`, the editor title in `draw`, new `App` field)
- Test: `tests/block_notes.rs`

**Interfaces:**
- Consumes: `Note::{covers, label, is_block, first, last, place}`, `NoteStore::{at_line, find_block}` (Task 1), `Note::block` (Task 2).
- Produces: private `App.editing: Option<(End, End)>` — the range the open note editor writes to. `open_note` sets it; `save_note` reads it. `Input` keeps its shape (`tests/navigation.rs:155` matches `Input::Note(String::new())`).

Rules:
- **Bar.** A row whose anchor some *block* note of this file covers draws `▎` in yellow in place of the space after its `+`/`-`/` ` sign (`row_line` gains a `bar: bool` parameter), so code columns do not move.
- **Placement.** Each note is drawn under the last row the note covers *among the rows actually pushed* (collapsed moved rows skipped). Work it out in a first pass over the rows, then draw while pushing; notes under the same row in store order. A note covering no shown row is not drawn (as a note on a hidden line is today).
- **Label.** `note_lines` puts `label() + " · "` after the marker for a block.
- **Plain `m`** on a row: `at_line` for its anchor. A block found → editor prefilled, `editing` = its `first()`/`last()`. Otherwise today's behaviour, `editing` = (anchor, anchor).
- **Saving** with `editing` first == last: today's single-line code path, unchanged. Otherwise `Note::block(file, first, last, text)` stored with `draft: false` through `NoteStore::set`.
- **Editor title** for a block: ` Note on {place without backticks} `; line notes unchanged.

- [ ] **Step 1: Write the failing tests** in `tests/block_notes.rs` (seed with `Note::block(&cart(), …)`; `screen(app, 120, 20)` as in `tests/notes.rs`):

```rust
#[test]
fn a_block_note_has_a_bar_beside_its_lines_and_sits_under_the_last() {
    // seed block (New,2)–(New,5) "Should 100% be allowed?"
    // rows with "2 +▎  if (percent", "4 +▎    throw", "5 +▎  }"; row "6    const factor" has no ▎
    // the row right after "5 +▎  }" contains "✎ 2–5 · Should 100% be allowed?"
}

#[test]
fn the_note_sits_under_the_last_visible_line_when_a_layer_hides_the_end() {
    // seed block (New,2)–(New,3) "x": line 3 is a hidden comment
    // the row right after "2 +▎  if (percent" contains "✎ 2–3 · x"
}

#[test]
fn a_draft_block_is_marked_as_one() {
    // seed a draft block (New,2)–(New,5) "x" → screen contains "✎ draft · 2–5 · x"
}

#[test]
fn m_on_any_line_of_a_block_reopens_it_and_saving_keeps_its_range() {
    // seed a draft block (New,2)–(New,5) "Check the bounds."
    // Right, Down (pane line 3: new line 4), 'm' → app.input == Some(Input::Note("Check the bounds.".into()))
    // screen contains "Note on cart.ts:2–5"; Enter
    // app.notes.notes().len() == 1; that note: last() == (New, 5), !draft, text unchanged
}
```

- [ ] **Step 2: Run** `cargo test --test block_notes` → the four new tests FAIL (no `▎`, no label).
- [ ] **Step 3: Implement** the rules above.
- [ ] **Step 4: Run** `cargo test` → all pass; the existing note tests in `tests/notes.rs` still draw `✎ use a fraction` unchanged.
- [ ] **Step 5: Commit** — `feat: draw block notes beside their lines and reopen them from any`

---

### Task 5: Selecting a block with `V`

**Files:**
- Modify: `src/tui.rs` (`DiffView`, `handle_key`, `draw`, `KEYS`, new `App` field), `src/main.rs` (`USAGE` keys), `README.md` (keys table)
- Test: `tests/block_notes.rs`

**Interfaces:**
- Consumes: `App.editing` and the save path (Task 4), `Note::block`, `Note::place`, `NoteStore::find_block`.
- Produces: private `App.selection: Option<End>` — where the block started, in file terms; `DiffView.hunk_of: Vec<Option<usize>>` — each pane line's hunk index (`None` for headers and blank lines).

Rules:
- `V` in the diff on a line with an anchor starts a selection there; from the file list it first moves into the diff, as `m` does. On a line without an anchor: message `put the cursor on a code line to select from it`. `V` again cancels.
- While selecting, these keys move the cursor and extend the block: `j k ↑ ↓ Space b PageUp PageDown d u g G Home End ] [ n N /` (and typing into the search). `m` opens the editor on the block. `Esc` cancels the selection only (focus stays in the diff). Every other key cancels the selection, then does what it always does (so `c`, `M`, `}`, Tab, `←`, `q` all cancel).
- After every key while selecting: if the file changed, focus left the diff, or the start no longer has a pane line in the view, cancel; otherwise clamp the cursor to the pane lines of the start's hunk (`hunk_of`).
- The block is from the start's pane line to the cursor's, either direction; its ends are those lines' anchors in pane order.
- Draw: lines between the start and the cursor get a yellow `▌` in the gutter (the cursor keeps its cyan `▌`). The status line shows `-- BLOCK  {place} ({n} lines) · j/k extend · m note · Esc cancel --` (`(1 line)` for one), from `Note::block(file, first, last, "")`: `place()` without backticks, `n` = its `code` line count.
- `m`: `editing` = (first, last); editor prefilled from `find_block` when that exact range has a note; selection cleared. A one-line selection is a line note (Task 4's single-line path).
- `KEYS` → Review group gains `("V", "select lines for one note: j/k extend, m note")`. `USAGE` keys: `m  V  E  o     note a line, or a block (V, then j/k, m); copy all notes as a prompt; open in $EDITOR`. README keys table gains a `V` row.

- [ ] **Step 1: Write the failing tests** in `tests/block_notes.rs`:

```rust
#[test]
fn v_then_j_j_then_m_notes_the_block_hidden_lines_included() {
    // Right, 'V', 'j', 'j' → app.cursor == 4;
    // screen contains "-- BLOCK  cart.ts:2–5 (4 lines) · j/k extend · m note · Esc cancel --"
    // 'm', type "Bounds?", Enter → one note: first() (New,2), last() (New,5), 4 code lines, !draft
}

#[test]
fn a_block_from_a_removed_line_into_added_ones_covers_both() {
    // Right, 'j' ×4 (pane line 6), 'V', 'j', 'j', 'm', "x", Enter
    // note: first() (Old,3), last() (New,8), old_lines Some((3,3)), new_lines Some((7,8))
}

#[test]
fn esc_cancels_the_selection_and_leaving_the_file_or_a_layer_key_does_too() {
    // Right, 'V', 'j', Esc → status has no "-- BLOCK", app.focus == Focus::Diff, app.cursor == 3
    // two-file app (cart.ts + a second changed file): Right, 'V', '}' → no "-- BLOCK"
    // Right, 'V', Tab → no "-- BLOCK"; Right, 'V', 'c' → no "-- BLOCK"; no note was saved by any of these
}

#[test]
fn a_selection_stays_inside_its_hunk() {
    // file with lines 5 and 25 changed (two hunks; pane: 0 header, 1–8 hunk one, 9 blank, 10 header, 11–18)
    // Right (cursor 4, removed line 5), 'V', 'G' → app.cursor == 8
}

#[test]
fn v_then_m_on_one_line_inside_a_block_makes_a_line_note() {
    // seed block (New,2)–(New,5); Right, Down, 'V', 'm', "x", Enter
    // 2 notes; app.notes.find("cart.ts", New, 4) is Some with text "x"
}
```

- [ ] **Step 2: Run** `cargo test --test block_notes` → the five new tests FAIL.
- [ ] **Step 3: Implement** the rules above.
- [ ] **Step 4: Run** `cargo test` → all pass, including `tests/navigation.rs` (Esc outside a selection still leaves the diff).
- [ ] **Step 5: Commit** — `feat: select a block of lines with V and note it`

---

### Task 6: Posting a block note

**Files:**
- Modify: `src/upload.rs` (`azure_thread`, new `azure_thread_right_only`, `CliPoster::post`, `github_comment`, `github_review`, `offer_upload`), `src/store.rs` (`Link`)
- Modify: `prod.md` ("Built since v0", decisions)
- Test: `tests/upload.rs`; `Link { … }` literals there and in `tests/notes.rs` gain `..Default::default()`

**Interfaces:**
- Consumes: `Note::{is_block, first, last, span, lines_text, place}` (Task 1).
- Produces:
  - `Link.remark: Option<String>` with `#[serde(skip)]` — a note to print after posting
  - `azure_thread(pr, note) -> (String, Value)`: a block on one side anchors `{side}FileStart {line: first, offset: 1}` → `{side}FileEnd {line: last, offset: last line's chars + 1, at least 2}` (`right` for new, `left` for old; the last line is the last line of `code`). A mixed block sets both ranges from `span(Old)` and `span(New)`; each end offset comes from the last `-` row (left) or the last `+`/` ` row (right) of `code`, marker stripped. Line notes and the whole-change note are unchanged.
  - `pub fn azure_thread_right_only(pr: &PullRequest, note: &Note) -> Option<(String, Value)>` — `Some` only for a mixed block: the same thread with only the right range.
  - `CliPoster`: when a mixed block's thread fails and `azure_thread_right_only` is `Some`, post that; on success set `remark = Some("Azure DevOps highlights {a}–{b} only")` (`{a}` alone when a == b) from `span(New)`.
  - GitHub: a block adds `start_line` = `first().1` and `start_side` (`LEFT` for old, `RIGHT` for new) to its review comment; `line`/`side` are `last()`'s. `github_comment` on a line inserts `-F start_line=…`, `-f start_side=…` before `-F line=…`; on the file, the body is `{lines_text()}: {text}`.
  - `offer_upload`: after posting, for each posted note with a remark print `  {place}: {remark}`, before the summary line.

- [ ] **Step 1: Write the failing tests** in `tests/upload.rs`:

```rust
fn block(first: End, last: End, spans: Option<((usize, usize), (usize, usize))>, code: &str, text: &str) -> Note

#[test]
fn an_azure_thread_on_a_block_spans_its_lines() {
    // block (New,11)–(New,13), code "a\nbb\nccc", text "x"
    // threadContext == {"filePath": "/src/Cart.swift", "rightFileStart": {"line": 11, "offset": 1},
    //                   "rightFileEnd": {"line": 13, "offset": 4}}
    // block (Old,11)–(Old,13), same code → leftFileStart/leftFileEnd, same numbers
}

#[test]
fn a_mixed_block_sends_both_sides_and_can_fall_back_to_the_right() {
    // block (Old,3)–(New,8), spans ((3,3),(7,8)),
    // code "-  return a;\n+  const t = 1;\n+  return t;"
    // threadContext: leftFileStart {3,1}, leftFileEnd {3,12}, rightFileStart {7,1}, rightFileEnd {8,12}
    // azure_thread_right_only → threadContext has only filePath, rightFileStart {7,1}, rightFileEnd {8,12}
    // azure_thread_right_only on block (New,11)–(New,13) → None
}

#[test]
fn github_comments_on_a_block_start_at_its_first_line() {
    // github_review with the mixed block → comments[0] ==
    //   {"path": "a.ts", "start_line": 3, "start_side": "LEFT", "line": 8, "side": "RIGHT", "body": "x"}
    // github_comment(on_file = false) contains, in order, "start_line=3", "start_side=LEFT", "line=8", "side=RIGHT"
    // github_comment(on_file = true) on block (New,11)–(New,13) contains "body=Lines 11–13: x";
    //   on the mixed block, "body=Removed line 3 to line 8: x"
}

#[test]
fn a_thread_narrowed_to_one_side_is_reported_after_posting() {
    // a Poster that returns Link { remark: Some("Azure DevOps highlights 7–8 only".into()), ..Default::default() }
    // offer_upload(.., yes = true, ..) on the mixed block, opened → output contains
    //   "  `a.ts` (removed line 3 to line 8): Azure DevOps highlights 7–8 only\n" before "Posted 1 comment"
}
```

- [ ] **Step 2: Run** `cargo test --test upload` → the four new tests FAIL.
- [ ] **Step 3: Implement** the Interfaces above. In `prod.md`, add to "Built since v0": notes on a block of lines (`V` in the viewer, `--line 11-13` / `"end_line"` for agents), posted as one comment covering the range; add a decision bullet: a block is every diff row between its ends, and if Azure DevOps refuses a thread on both sides it is posted on the right side and the reviewer is told.
- [ ] **Step 4: Run** `cargo test` and `cargo clippy --all-targets` → all pass, zero warnings.
- [ ] **Step 5: Commit** — `feat: post a block note as one comment over its lines`

---

## After the last task

Hand back to the user for the live spot-check the spec asks for (one block note on an Azure DevOps PR, one on GitHub, including a block from removed lines into added ones). Nothing is posted by the builder.
