use declutter::review::Layers;
use declutter::review::{ChangeStatus, FileChange, FileReview};
use declutter::store::{Added, End, Note, NoteSide, NoteSide::*, NoteStore, ReviewStore};
use declutter::tui::{App, Input, draw};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

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

#[test]
fn a_block_runs_from_first_to_last_in_diff_order_whichever_way_it_is_given() {
    let down = Note::block(&cart(), (NoteSide::New, 6), (NoteSide::Old, 3), "x");
    let up = Note::block(&cart(), (NoteSide::Old, 3), (NoteSide::New, 6), "x");
    assert_eq!(down, up);
    assert_eq!(
        (down.first(), down.last()),
        ((NoteSide::New, 6), (NoteSide::Old, 3))
    );

    let forward = Note::block(&cart(), (NoteSide::New, 7), (NoteSide::New, 8), "x");
    let backward = Note::block(&cart(), (NoteSide::New, 8), (NoteSide::New, 7), "x");
    assert_eq!(forward, backward);
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn screen(app: &mut App, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
    terminal.draw(|frame| draw(frame, app)).expect("draw");
    let buffer = terminal.backend().buffer();
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
                + "\n"
        })
        .collect()
}

/// The diff pane's rows as drawn, from the hunk header down (pane line 0 first).
fn pane(app: &mut App) -> Vec<String> {
    let text = screen(app, 120, 20);
    let rows: Vec<String> = text.lines().map(str::to_string).collect();
    let header = rows
        .iter()
        .position(|row| row.contains("@@ -1 +1 @@"))
        .expect("hunk header");
    rows[header..].to_vec()
}

fn cart_app(notes: NoteStore) -> App {
    let new_app = |notes| {
        let mut app = App::with_stores(
            vec![cart()],
            Layers::default(),
            ReviewStore::in_memory(),
            notes,
        );
        app.handle_key(key(KeyCode::Right));
        app
    };
    // The fixture's pane lines, checked without notes so none sits between rows.
    let rows = pane(&mut new_app(NoteStore::in_memory()));
    assert!(rows[2].contains("2 +   if (percent"), "{}", rows[2]);
    assert!(rows[6].contains("3       -   return"), "{}", rows[6]);
    new_app(notes)
}

fn seeded(first: End, last: End, text: &str, draft: bool) -> NoteStore {
    let mut notes = NoteStore::in_memory();
    let note = Note {
        draft,
        ..Note::block(&cart(), first, last, text)
    };
    notes.set(note).expect("seed");
    notes
}

#[test]
fn a_block_note_has_a_bar_beside_its_lines_and_sits_under_the_last() {
    let notes = seeded((New, 2), (New, 5), "Should 100% be allowed?", false);
    let mut app = App::with_stores(
        vec![cart()],
        Layers::default(),
        ReviewStore::in_memory(),
        notes,
    );
    app.handle_key(key(KeyCode::Right));
    let rows = pane(&mut app);
    assert!(rows[2].contains("2 +▎  if (percent"), "{}", rows[2]);
    assert!(rows[3].contains("4 +▎    throw"), "{}", rows[3]);
    assert!(rows[4].contains("5 +▎  }"), "{}", rows[4]);
    assert!(
        rows[5].contains("✎ 2–5 · Should 100% be allowed?"),
        "{}",
        rows[5]
    );
    assert!(rows[6].contains("const factor"), "{}", rows[6]);
    assert!(!rows[6].contains('▎'), "{}", rows[6]);
    assert!(!rows[1].contains('▎'), "{}", rows[1]);
}

#[test]
fn the_note_sits_under_the_last_visible_line_when_a_layer_hides_the_end() {
    let notes = seeded((New, 2), (New, 3), "x", false);
    let mut app = App::with_stores(
        vec![cart()],
        Layers::default(),
        ReviewStore::in_memory(),
        notes,
    );
    app.handle_key(key(KeyCode::Right));
    let rows = pane(&mut app);
    assert!(rows[2].contains("2 +▎  if (percent"), "{}", rows[2]);
    assert!(rows[3].contains("✎ 2–3 · x"), "{}", rows[3]);
}

#[test]
fn a_draft_block_is_marked_as_one() {
    let notes = seeded((New, 2), (New, 5), "x", true);
    let mut app = cart_app(notes);
    assert!(screen(&mut app, 120, 20).contains("✎ draft · 2–5 · x"));
}

#[test]
fn m_on_any_line_of_a_block_reopens_it_and_saving_keeps_its_range() {
    let notes = seeded((New, 2), (New, 5), "Check the bounds.", true);
    let mut app = cart_app(notes);
    app.handle_key(key(KeyCode::Down));
    app.handle_key(key(KeyCode::Char('m')));
    assert_eq!(app.input, Some(Input::Note("Check the bounds.".into())));
    assert!(screen(&mut app, 120, 20).contains("Note on cart.ts:2–5"));
    app.handle_key(key(KeyCode::Enter));
    let all = app.notes.notes();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].first(), (New, 2));
    assert_eq!(all[0].last(), (New, 5));
    assert!(!all[0].draft);
    assert_eq!(all[0].text, "Check the bounds.");
}

fn press(app: &mut App, keys: &[KeyCode]) {
    for &code in keys {
        app.handle_key(key(code));
    }
}

fn type_note(app: &mut App, text: &str) {
    app.handle_key(key(KeyCode::Char('m')));
    for c in text.chars() {
        app.handle_key(key(KeyCode::Char(c)));
    }
    app.handle_key(key(KeyCode::Enter));
}

fn v() -> KeyCode {
    KeyCode::Char('V')
}

fn j() -> KeyCode {
    KeyCode::Char('j')
}

#[test]
fn v_then_j_j_then_m_notes_the_block_hidden_lines_included() {
    let mut app = cart_app(NoteStore::in_memory());
    press(&mut app, &[v(), j(), j()]);
    assert_eq!(app.cursor, 4);
    assert!(
        screen(&mut app, 120, 20)
            .contains("-- BLOCK  cart.ts:2–5 (4 lines) · j/k extend · m note · Esc cancel --")
    );
    type_note(&mut app, "Bounds?");
    let all = app.notes.notes();
    assert_eq!(all.len(), 1);
    assert_eq!((all[0].first(), all[0].last()), ((New, 2), (New, 5)));
    assert_eq!(all[0].code.lines().count(), 4);
    assert!(!all[0].draft);
    assert!(!screen(&mut app, 120, 20).contains("-- BLOCK"));
}

#[test]
fn a_block_from_a_removed_line_into_added_ones_covers_both() {
    let mut app = cart_app(NoteStore::in_memory());
    press(&mut app, &[j(), j(), j(), j()]);
    assert_eq!(app.cursor, 6);
    press(&mut app, &[v(), j(), j()]);
    type_note(&mut app, "x");
    let all = app.notes.notes();
    assert_eq!(all.len(), 1);
    assert_eq!((all[0].first(), all[0].last()), ((Old, 3), (New, 8)));
    assert_eq!(
        (all[0].old_lines, all[0].new_lines),
        (Some((3, 3)), Some((7, 8)))
    );
}

#[test]
fn esc_cancels_the_selection_and_leaving_the_file_or_a_layer_key_does_too() {
    let blocking = |app: &mut App| screen(app, 120, 20).contains("-- BLOCK");
    let mut app = cart_app(NoteStore::in_memory());
    press(&mut app, &[v(), j()]);
    assert!(blocking(&mut app));
    press(&mut app, &[KeyCode::Esc]);
    assert!(!blocking(&mut app));
    assert_eq!(app.focus, declutter::tui::Focus::Diff);
    assert_eq!(app.cursor, 3);

    let other = FileReview::new(FileChange {
        path: "other.ts".to_string(),
        old_path: None,
        status: ChangeStatus::Modified,
        old: Some("a\n".to_string()),
        new: Some("b\n".to_string()),
        binary: false,
    });
    let mut two = App::with_stores(
        vec![cart(), other],
        Layers::default(),
        ReviewStore::in_memory(),
        NoteStore::in_memory(),
    );
    press(&mut two, &[KeyCode::Right, v()]);
    assert!(blocking(&mut two));
    press(&mut two, &[KeyCode::Char('}')]);
    assert!(!blocking(&mut two));
    press(&mut two, &[KeyCode::Char('m'), KeyCode::Enter]);
    assert!(two.notes.notes().iter().all(|n| n.path == "other.ts"));

    for cancel in [KeyCode::Tab, KeyCode::Char('c')] {
        let mut app = cart_app(NoteStore::in_memory());
        press(&mut app, &[v(), j()]);
        assert!(blocking(&mut app));
        press(&mut app, &[cancel]);
        assert!(!blocking(&mut app));
        assert!(app.notes.notes().is_empty());
    }
}

#[test]
fn a_selection_stays_inside_its_hunk() {
    let old: String = (1..=30).map(|n| format!("line {n}\n")).collect();
    let new = old
        .replace("line 5\n", "five\n")
        .replace("line 25\n", "25!\n");
    let file = FileReview::new(FileChange {
        path: "long.ts".to_string(),
        old_path: None,
        status: ChangeStatus::Modified,
        old: Some(old),
        new: Some(new),
        binary: false,
    });
    let mut app = App::with_stores(
        vec![file],
        Layers::default(),
        ReviewStore::in_memory(),
        NoteStore::in_memory(),
    );
    press(&mut app, &[KeyCode::Right]);
    assert_eq!(app.cursor, 4);
    press(&mut app, &[v(), KeyCode::Char('G')]);
    assert_eq!(app.cursor, 8);
    press(&mut app, &[KeyCode::Char('g')]);
    assert_eq!(app.cursor, 1);
}

#[test]
fn v_then_m_on_one_line_inside_a_block_makes_a_line_note() {
    let notes = seeded((New, 2), (New, 5), "block", false);
    let mut app = cart_app(notes);
    press(&mut app, &[KeyCode::Down, v()]);
    assert!(screen(&mut app, 120, 20).contains("(1 line)"));
    type_note(&mut app, "x");
    assert_eq!(app.notes.notes().len(), 2);
    assert_eq!(texts(app.notes.find("cart.ts", New, 4)), Some("x"));
}

#[test]
fn esc_in_the_editor_forgets_the_block_it_was_for() {
    let mut app = cart_app(NoteStore::in_memory());
    press(&mut app, &[v(), j(), KeyCode::Char('m'), KeyCode::Esc]);
    assert_eq!(app.input, None);
    press(&mut app, &[j()]);
    type_note(&mut app, "x");
    let all = app.notes.notes();
    assert_eq!(all.len(), 1);
    assert!(!all[0].is_block());
    assert_eq!(all[0].first(), (New, 5));
}

#[test]
fn v_on_a_header_says_where_to_put_the_cursor_and_v_again_cancels() {
    let mut app = cart_app(NoteStore::in_memory());
    press(&mut app, &[KeyCode::Home, v()]);
    assert!(screen(&mut app, 120, 20).contains("put the cursor on a code line to select from it"));
    press(&mut app, &[j(), v(), v()]);
    assert!(!screen(&mut app, 120, 20).contains("-- BLOCK"));
}

#[test]
fn v_from_the_file_list_moves_into_the_diff_and_selects() {
    let mut app = App::with_stores(
        vec![cart()],
        Layers::default(),
        ReviewStore::in_memory(),
        NoteStore::in_memory(),
    );
    press(&mut app, &[v()]);
    assert_eq!(app.focus, declutter::tui::Focus::Diff);
    assert!(screen(&mut app, 120, 20).contains("-- BLOCK  cart.ts:2 (1 line)"));
}
