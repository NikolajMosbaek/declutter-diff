use declutter::review::{ChangeStatus, FileChange, FileReview};
use declutter::store::{Added, End, Note, NoteSide, NoteSide::*, NoteStore};

const OLD: &str = "export function applyDiscount(cart, percent) {\n  const factor = 1 - percent / 100;\n  return { ...cart, total: cart.total * factor };\n}\n";
const NEW: &str = "export function applyDiscount(cart, percent) {\n  if (percent < 0 || percent > 100) {\n    // A discount outside 0-100 is a bug upstream.\n    throw new RangeError(\"percent must be 0-100\");\n  }\n  const factor = 1 - percent / 100;\n  const total = Math.round(cart.total * factor * 100) / 100;\n  return { ...cart, total };\n}\n";

fn cart() -> FileReview {
    FileReview::new(FileChange {
        path: "cart.ts".to_string(),
        old_path: None,
        status: ChangeStatus::Modified,
        old: Some(OLD.to_string()),
        new: Some(NEW.to_string()),
        binary: false,
    })
}

fn block(
    first: End,
    last: End,
    spans: Option<((usize, usize), (usize, usize))>,
    text: &str,
) -> Note {
    Note {
        path: "a.ts".into(),
        side: first.0,
        line: first.1,
        end_side: (last.0 != first.0).then_some(last.0),
        end_line: (last != first).then_some(last.1),
        old_lines: spans.map(|(old, _)| old),
        new_lines: spans.map(|(_, new)| new),
        code: "x".into(),
        text: text.into(),
        review: None,
        draft: false,
    }
}

fn line(side: NoteSide, n: usize, text: &str) -> Note {
    block((side, n), (side, n), None, text)
}

fn texts(found: Option<&Note>) -> Option<&str> {
    found.map(|n| n.text.as_str())
}

#[test]
fn old_notes_without_block_fields_still_load() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("notes.json");
    let json =
        r#"[{"path":"a.ts","side":"new","line":3,"code":"let a = 2","text":"hi","draft":true}]"#;
    std::fs::write(&path, json).unwrap();
    let mut store = NoteStore::at(path.clone());
    let found = store.find("a.ts", New, 3).expect("loads").clone();
    assert_eq!(found.text, "hi");
    store.set(found).unwrap();
    let written = std::fs::read_to_string(&path).unwrap();
    for field in ["end_line", "end_side", "old_lines", "new_lines"] {
        assert!(!written.contains(field), "{field} in {written}");
    }
}

#[test]
fn a_block_is_found_from_any_line_it_covers_and_a_line_note_wins() {
    let mut store = NoteStore::in_memory();
    store
        .set(block((New, 11), (New, 13), None, "block"))
        .unwrap();
    store.set(line(New, 12, "line")).unwrap();
    assert_eq!(texts(store.at_line("a.ts", New, 11)), Some("block"));
    assert_eq!(texts(store.at_line("a.ts", New, 12)), Some("line"));
    assert_eq!(store.at_line("a.ts", New, 14), None);
    assert_eq!(store.find("a.ts", New, 11), None);
    assert!(store.find_block("a.ts", (New, 11), (New, 13)).is_some());
    assert!(store.find_block("a.ts", (New, 12), (New, 12)).is_some());
}

#[test]
fn the_smallest_block_covering_a_line_wins() {
    let mut store = NoteStore::in_memory();
    store
        .set(block((New, 10), (New, 20), None, "outer"))
        .unwrap();
    store
        .set(block((New, 11), (New, 13), None, "inner"))
        .unwrap();
    assert_eq!(texts(store.at_line("a.ts", New, 12)), Some("inner"));
    assert_eq!(texts(store.at_line("a.ts", New, 15)), Some("outer"));
}

#[test]
fn a_mixed_block_covers_each_side_by_its_own_span() {
    let note = block((Old, 3), (New, 8), Some(((3, 3), (7, 8))), "m");
    assert!(note.covers(Old, 3) && note.covers(New, 7) && note.covers(New, 8));
    assert!(!note.covers(New, 6) && !note.covers(Old, 4));
    assert_eq!(note.size(), 3);
    assert!(note.is_block());
    assert_eq!(note.first(), (Old, 3));
    assert_eq!(note.last(), (New, 8));
}

#[test]
fn one_range_holds_one_note_and_another_range_is_another_note() {
    let mut store = NoteStore::in_memory();
    store.set(block((New, 11), (New, 13), None, "a")).unwrap();
    store.set(block((New, 11), (New, 13), None, "b")).unwrap();
    assert_eq!(store.notes().len(), 1);
    assert_eq!(store.notes()[0].text, "b");
    store.set(block((New, 11), (New, 14), None, "c")).unwrap();
    assert_eq!(store.notes().len(), 2);
    let added = store.add(block((New, 11), (New, 13), None, "d")).unwrap();
    assert_eq!(added, Added::Appended);
    assert_eq!(
        texts(store.find_block("a.ts", (New, 11), (New, 13))),
        Some("b\n\nd")
    );
}

#[test]
fn blocks_name_their_lines() {
    let cases = [
        (line(New, 9, "t"), "`a.ts:9`", "", "Line 9"),
        (
            block((New, 11), (New, 13), None, "t"),
            "`a.ts:11–13`",
            "11–13",
            "Lines 11–13",
        ),
        (
            block((Old, 11), (Old, 13), None, "t"),
            "`a.ts` (removed lines 11–13)",
            "removed 11–13",
            "Removed lines 11–13",
        ),
        (
            block((Old, 3), (New, 8), Some(((3, 3), (7, 8))), "t"),
            "`a.ts` (removed line 3 to line 8)",
            "removed 3 to 8",
            "Removed line 3 to line 8",
        ),
        (
            block((New, 6), (Old, 3), Some(((3, 3), (6, 6))), "t"),
            "`a.ts` (line 6 to removed line 3)",
            "6 to removed 3",
            "Line 6 to removed line 3",
        ),
    ];
    for (note, place, label, lines_text) in cases {
        assert_eq!(note.place(), place);
        assert_eq!(note.label(), label);
        assert_eq!(note.lines_text(), lines_text);
    }
}

#[test]
fn the_prompt_quotes_every_line_of_a_block() {
    let mut store = NoteStore::in_memory();
    let mut note = block((New, 11), (New, 13), None, "why?");
    note.code = "  if x {\n    y()\n  }".into();
    store.set(note).unwrap();
    assert_eq!(
        store.prompt(),
        "Please address these review comments. Line numbers refer to the version under review.\n\
         \n1. `a.ts:11–13`: why?\n   ```\n     if x {\n       y()\n     }\n   ```\n"
    );
}

#[test]
fn a_block_that_ends_before_it_starts_has_no_size_to_underflow() {
    assert_eq!(block((New, 9), (New, 5), None, "t").size(), 0);
}

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
    assert_eq!(
        (note.end_side, note.end_line),
        (Some(NoteSide::New), Some(8))
    );
    assert_eq!(
        (note.old_lines, note.new_lines),
        (Some((3, 3)), Some((7, 8)))
    );
    assert_eq!(
        note.code,
        "-  return { ...cart, total: cart.total * factor };\n+  const total = Math.round(cart.total * factor * 100) / 100;\n+  return { ...cart, total };"
    );
}

#[test]
fn a_block_from_a_context_line_down_into_a_removed_one_records_both_sides() {
    let note = Note::block(&cart(), (NoteSide::New, 6), (NoteSide::Old, 3), "x");
    assert_eq!(
        (note.end_side, note.end_line),
        (Some(NoteSide::Old), Some(3))
    );
    assert_eq!(
        (note.old_lines, note.new_lines),
        (Some((3, 3)), Some((6, 6)))
    );
    assert_eq!(note.place(), "`cart.ts` (line 6 to removed line 3)");
}

#[test]
fn a_block_end_outside_the_diff_quotes_the_first_ends_side() {
    let note = Note::block(&cart(), (NoteSide::New, 8), (NoteSide::New, 40), "x");
    assert_eq!(note.code, "  return { ...cart, total };\n}");
    assert_eq!((note.old_lines, note.new_lines), (None, None));
}
