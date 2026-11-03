use declutter::store::{Added, End, Note, NoteSide, NoteSide::*, NoteStore};

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
