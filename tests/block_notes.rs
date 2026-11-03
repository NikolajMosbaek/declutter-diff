use declutter::review::Layers;
use declutter::review::{ChangeStatus, DiffModes, FileChange, FileReview};
use declutter::store::{Added, End, Note, NoteSide, NoteSide::*, NoteStore, ReviewStore};
use declutter::tui::{App, Input, draw};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::style::Color;

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
    let note = Note::block(
        &cart(),
        DiffModes::SHOWN,
        (NoteSide::New, 2),
        (NoteSide::New, 5),
        "x",
    );
    assert_eq!(note.code.lines().count(), 4);
    assert!(note.code.contains("// A discount outside 0-100"));
    assert!(!note.draft);
}

#[test]
fn a_block_from_a_removed_line_into_added_ones_records_both_sides() {
    let note = Note::block(
        &cart(),
        DiffModes::SHOWN,
        (NoteSide::Old, 3),
        (NoteSide::New, 8),
        "x",
    );
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
    let note = Note::block(
        &cart(),
        DiffModes::SHOWN,
        (NoteSide::New, 6),
        (NoteSide::Old, 3),
        "x",
    );
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
    let note = Note::block(
        &cart(),
        DiffModes::SHOWN,
        (NoteSide::New, 8),
        (NoteSide::New, 40),
        "x",
    );
    assert_eq!(note.code, "  return { ...cart, total };\n}");
    assert_eq!((note.old_lines, note.new_lines), (None, None));
}

#[test]
fn a_block_runs_from_first_to_last_in_diff_order_whichever_way_it_is_given() {
    let down = Note::block(
        &cart(),
        DiffModes::SHOWN,
        (NoteSide::New, 6),
        (NoteSide::Old, 3),
        "x",
    );
    let up = Note::block(
        &cart(),
        DiffModes::SHOWN,
        (NoteSide::Old, 3),
        (NoteSide::New, 6),
        "x",
    );
    assert_eq!(down, up);
    assert_eq!(
        (down.first(), down.last()),
        ((NoteSide::New, 6), (NoteSide::Old, 3))
    );

    let forward = Note::block(
        &cart(),
        DiffModes::SHOWN,
        (NoteSide::New, 7),
        (NoteSide::New, 8),
        "x",
    );
    let backward = Note::block(
        &cart(),
        DiffModes::SHOWN,
        (NoteSide::New, 8),
        (NoteSide::New, 7),
        "x",
    );
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
        ..Note::block(&cart(), DiffModes::SHOWN, first, last, text)
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

/// A second file whose changed rows (new lines 2–4) cover the cart's block start (New, 2).
fn two_files() -> App {
    let other = FileReview::new(FileChange {
        path: "other.ts".to_string(),
        old_path: None,
        status: ChangeStatus::Modified,
        old: Some("l1\nl2\nl3\nl4\nl5\n".to_string()),
        new: Some("l1\nAA\nBB\nCC\nl5\n".to_string()),
        binary: false,
    });
    let mut app = App::with_stores(
        vec![cart(), other],
        Layers::default(),
        ReviewStore::in_memory(),
        NoteStore::in_memory(),
    );
    press(&mut app, &[KeyCode::Right]);
    app
}

#[test]
fn a_selection_never_follows_the_reviewer_into_another_file() {
    let routes: [(&str, &[KeyCode]); 4] = [
        ("}", &[KeyCode::Char('}')]),
        ("]", &[KeyCode::Char(']'), KeyCode::Char(']')]),
        ("n", &[KeyCode::Char('n')]),
        (
            "search",
            &[
                KeyCode::Char('/'),
                KeyCode::Char('B'),
                KeyCode::Char('B'),
                KeyCode::Enter,
            ],
        ),
    ];
    for (name, keys) in routes {
        let mut app = two_files();
        if name == "n" {
            // A search made first, so that `n` has somewhere to go.
            press(
                &mut app,
                &[KeyCode::Char('/'), KeyCode::Char('B'), KeyCode::Enter],
            );
            press(&mut app, &[KeyCode::Char('g')]);
            press(&mut app, &[KeyCode::Char('}'), KeyCode::Char('{')]);
        }
        press(&mut app, &[v()]);
        assert!(screen(&mut app, 120, 20).contains("-- BLOCK"), "{name}");
        press(&mut app, keys);
        assert_eq!(app.selected, 1, "{name}: did not reach the second file");
        assert!(!screen(&mut app, 120, 20).contains("-- BLOCK"), "{name}");
        press(&mut app, &[j()]);
        assert!(!screen(&mut app, 120, 20).contains("-- BLOCK"), "{name}");
        type_note(&mut app, "q");
        assert!(
            app.notes.notes().iter().all(|n| !n.is_block()),
            "{name}: {:?}",
            app.notes.notes()
        );
    }
}

#[test]
fn a_message_does_not_hide_the_block_status() {
    let mut app = cart_app(NoteStore::in_memory());
    press(&mut app, &[v(), KeyCode::Char('n')]);
    let text = screen(&mut app, 120, 20);
    assert!(text.contains("-- BLOCK  cart.ts:2 (1 line)"), "{text}");
    assert!(text.contains("press / to search"), "{text}");
}

#[test]
fn a_one_line_selection_with_the_cursor_on_a_moved_marker_still_saves() {
    let block = "fn validate(a: i32) -> bool {\n    a > 0\n}\nfn other() {}\n";
    let fill = "f1\nf2\nf3\nf4\nf5\n";
    let old = format!("k1\nk2\n{block}{fill}");
    let new = format!("k1\nk2\n{fill}{block}");
    let file = FileReview::new(FileChange {
        path: "m.rs".to_string(),
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
    press(&mut app, &[KeyCode::Right, KeyCode::Home, j(), j()]);
    assert!(screen(&mut app, 120, 20).contains("k2"));
    press(&mut app, &[v(), j()]);
    let text = screen(&mut app, 120, 20);
    assert!(text.contains("moved"), "{text}");
    assert!(text.contains("-- BLOCK  m.rs:2 (1 line)"), "{text}");
    type_note(&mut app, "lost?");
    let all = app.notes.notes();
    assert_eq!(all.len(), 1);
    assert_eq!((all[0].first(), all[0].text.as_str()), ((New, 2), "lost?"));
}

fn p_app(old: &str, new: &str) -> App {
    let file = FileReview::new(FileChange {
        path: "p.ts".to_string(),
        old_path: None,
        status: ChangeStatus::Modified,
        old: Some(old.to_string()),
        new: Some(new.to_string()),
        binary: false,
    });
    let mut app = App::with_stores(
        vec![file],
        Layers::default(),
        ReviewStore::in_memory(),
        NoteStore::in_memory(),
    );
    press(&mut app, &[KeyCode::Right]);
    app
}

/// The status line's `-- BLOCK …` text while selecting.
fn block_status(app: &mut App) -> String {
    let text = screen(app, 120, 20);
    let line = text.lines().find(|line| line.contains("-- BLOCK"));
    line.unwrap_or_else(|| panic!("no block status in\n{text}"))
        .trim()
        .to_string()
}

#[test]
fn a_block_from_a_removed_line_to_context_past_hidden_comments_is_saved_as_selected() {
    // Old 2–9 removed; four comment lines (hidden) then `b`: old 14, new 6, which the
    // decluttered view shows as context and the full diff does not.
    let old = "const a = 1;\nconst r1 = 1;\nconst r2 = 2;\nconst r3 = 3;\nconst r4 = 4;\nconst r5 = 5;\nconst r6 = 6;\nconst r7 = 7;\nconst r8 = 8;\n// c1\n// c2\n// c3\n// c4\nconst b = 1;\nconst z = 1;\n";
    let new = "const a = 1;\n// c1\n// c2\n// c3\n// c4\nconst b = 1;\nconst z = 1;\n";
    let mut app = p_app(old, new);
    assert_eq!(app.cursor, 2);
    press(&mut app, &[j(), j(), j(), j(), j(), j(), j()]);
    let rows = pane(&mut app);
    assert!(rows[9].contains("9       - const r8"), "{}", rows[9]);
    assert!(rows[10].contains("14     6   const b"), "{}", rows[10]);
    press(&mut app, &[v(), j()]);
    assert_eq!(
        block_status(&mut app),
        "-- BLOCK  p.ts (removed line 9 to line 6) (2 lines) · j/k extend · m note · Esc cancel --"
    );
    type_note(&mut app, "probe");
    let all = app.notes.notes();
    assert_eq!(all.len(), 1);
    let note = all[0];
    assert_eq!((note.first(), note.last()), ((Old, 9), (New, 6)));
    assert_eq!((note.end_side, note.end_line), (Some(New), Some(6)));
    assert_eq!(
        (note.old_lines, note.new_lines),
        (Some((9, 9)), Some((6, 6)))
    );
    assert_eq!(note.code, "-const r8 = 8;\n const b = 1;");
    assert_eq!(note.size(), 2);
    assert!(!note.draft);
    assert_eq!(note.place(), "`p.ts` (removed line 9 to line 6)");

    let rows = pane(&mut app);
    assert!(rows[8].contains("8       - const r7"), "{}", rows[8]);
    assert!(rows[9].contains("9       -▎const r8"), "{}", rows[9]);
    assert!(rows[10].contains("14     6  ▎const b"), "{}", rows[10]);
    assert!(
        rows[11].contains("✎ removed 9 to 6 · probe"),
        "{}",
        rows[11]
    );
    assert!(!rows[12].contains('▎'), "{}", rows[12]);
    assert_eq!(
        app.notes.at_line("p.ts", New, 6).map(|n| n.text.as_str()),
        Some("probe")
    );
}

#[test]
fn a_block_from_a_change_to_context_past_a_hidden_comment_keeps_both_sides() {
    // A one-line change, one comment line (hidden), then code: the decluttered hunk
    // ends on new line 6, which the full diff does not reach.
    let old = "const a = 1;\nconst x = 1;\n// note\nconst b = 1;\nconst c = 1;\nconst d = 1;\nconst e = 1;\n";
    let new = "const a = 1;\nconst x = 2;\n// note\nconst b = 1;\nconst c = 1;\nconst d = 1;\nconst e = 1;\n";
    let mut app = p_app(old, new);
    assert_eq!(app.cursor, 2);
    press(&mut app, &[v(), KeyCode::Char('G')]);
    assert_eq!(app.cursor, 6);
    assert_eq!(
        block_status(&mut app),
        "-- BLOCK  p.ts (removed line 2 to line 6) (5 lines) · j/k extend · m note · Esc cancel --"
    );
    type_note(&mut app, "probe");
    let all = app.notes.notes();
    assert_eq!(all.len(), 1);
    let note = all[0];
    assert_eq!((note.first(), note.last()), ((Old, 2), (New, 6)));
    assert_eq!((note.end_side, note.end_line), (Some(New), Some(6)));
    assert_eq!(
        (note.old_lines, note.new_lines),
        (Some((2, 2)), Some((2, 6)))
    );
    assert_eq!(
        note.code,
        "-const x = 1;\n+const x = 2;\n const b = 1;\n const c = 1;\n const d = 1;"
    );

    let rows = pane(&mut app);
    assert!(!rows[1].contains('▎'), "{}", rows[1]);
    assert!(rows[2].contains("2       -▎const x = 1"), "{}", rows[2]);
    assert!(rows[3].contains("2 +▎const x = 2"), "{}", rows[3]);
    for (row, text) in [(4, "const b"), (5, "const c"), (6, "const d")] {
        assert!(rows[row].contains(&format!("▎{text}")), "{}", rows[row]);
    }
    assert!(rows[7].contains("✎ removed 2 to 6 · probe"), "{}", rows[7]);
}

#[test]
fn a_block_from_context_above_a_hidden_comment_down_to_a_removed_line_covers_that_line() {
    let old = "const p = 1;\nconst q = 1;\nconst r = 1;\n// note\nconst x = 1;\nconst z = 1;\n";
    let new = "const p = 1;\nconst q = 1;\nconst r = 1;\n// note\nconst x = 2;\nconst z = 1;\n";
    let mut app = p_app(old, new);
    press(&mut app, &[KeyCode::Char('g'), j(), v(), j(), j(), j()]);
    assert_eq!(app.cursor, 4);
    assert_eq!(
        block_status(&mut app),
        "-- BLOCK  p.ts (line 1 to removed line 5) (4 lines) · j/k extend · m note · Esc cancel --"
    );
    type_note(&mut app, "zz");
    let all = app.notes.notes();
    assert_eq!(all.len(), 1);
    let note = all[0];
    assert_eq!((note.first(), note.last()), ((New, 1), (Old, 5)));
    assert_eq!((note.end_side, note.end_line), (Some(Old), Some(5)));
    assert_eq!(
        (note.old_lines, note.new_lines),
        (Some((5, 5)), Some((1, 3)))
    );
    assert_eq!(
        note.code,
        " const p = 1;\n const q = 1;\n const r = 1;\n-const x = 1;"
    );

    let rows = pane(&mut app);
    for (row, text) in [(1, "const p"), (2, "const q"), (3, "const r")] {
        assert!(rows[row].contains(&format!("▎{text}")), "{}", rows[row]);
    }
    assert!(rows[4].contains("5       -▎const x = 1"), "{}", rows[4]);
    assert!(rows[5].contains("✎ 1 to removed 5 · zz"), "{}", rows[5]);
    assert!(rows[6].contains("5 + const x = 2"), "{}", rows[6]);
    assert!(!rows[6].contains('▎'), "{}", rows[6]);
}

#[test]
fn a_block_with_an_end_in_neither_diff_never_runs_backwards_or_loses_a_side() {
    let backwards = Note::block(&cart(), DiffModes::SHOWN, (New, 40), (New, 8), "x");
    assert_eq!((backwards.first(), backwards.last()), ((New, 8), (New, 40)));
    assert_eq!(backwards.code, "  return { ...cart, total };\n}");

    let across = Note::block(&cart(), DiffModes::SHOWN, (Old, 3), (New, 40), "x");
    assert_eq!((across.first(), across.last()), ((Old, 3), (New, 40)));
    assert_eq!(across.end_side, Some(New));
    assert_eq!(
        (across.old_lines, across.new_lines),
        (Some((3, 3)), Some((40, 40)))
    );
    assert!(across.covers(Old, 3) && across.covers(New, 40));
    assert!(!across.covers(New, 3) && !across.covers(Old, 40));
}

#[test]
fn a_block_note_sits_under_its_last_row_still_shown_when_moved_rows_collapse() {
    let block = "fn validate(a: i32) -> bool {\n    a > 0\n}\nfn other() {}\n";
    let fill = "f1\nf2\nf3\nf4\nf5\n";
    let file = FileReview::new(FileChange {
        path: "m.rs".to_string(),
        old_path: None,
        status: ChangeStatus::Modified,
        old: Some(format!("k1\nk2\n{block}{fill}")),
        new: Some(format!("k1\nk2\n{fill}{block}")),
        binary: false,
    });
    let mut notes = NoteStore::in_memory();
    let note = Note::block(&file, DiffModes::SHOWN, (New, 2), (Old, 6), "x");
    assert_eq!(
        (note.old_lines, note.new_lines),
        (Some((3, 6)), Some((2, 2)))
    );
    notes.set(note).expect("seed");
    let on_k2 = Note {
        path: "m.rs".into(),
        ..line(New, 2, "on k2")
    };
    notes.set(on_k2).expect("seed");
    let mut app = App::with_stores(
        vec![file],
        Layers::default(),
        ReviewStore::in_memory(),
        notes,
    );
    press(&mut app, &[KeyCode::Right, KeyCode::Char('g')]);
    let rows = pane(&mut app);
    let at = |rows: &[String], text: &str| {
        rows.iter()
            .position(|row| row.contains(text))
            .unwrap_or_else(|| panic!("{text} in {rows:#?}"))
    };
    assert_eq!(
        at(&rows, "✎ 2 to removed 6 · x"),
        at(&rows, "6       -▎fn other") + 1
    );
    press(&mut app, &[KeyCode::Char('M')]);
    let rows = pane(&mut app);
    assert!(rows[2].contains("2     2  ▎k2"), "{}", rows[2]);
    assert!(rows[3].contains("✎ 2 to removed 6 · x"), "{}", rows[3]);
    assert!(rows[4].contains("✎ on k2"), "{}", rows[4]);
    assert!(rows[5].contains("lines moved to line 8"), "{}", rows[5]);
}

#[test]
fn a_blank_last_line_counts_in_the_block_status() {
    let old = "function a() {\n}\n";
    let new = "function a() {\n  let x = 1;\n\n  let y = 2;\n}\n";
    let mut app = p_app(old, new);
    // With comments hidden, the blank line goes with them; show it.
    press(&mut app, &[KeyCode::Char('c')]);
    assert_eq!(app.cursor, 2);
    press(&mut app, &[j(), v()]);
    let lone = block_status(&mut app);
    press(&mut app, &[KeyCode::Esc, KeyCode::Char('k'), v(), j()]);
    let ending = block_status(&mut app);
    assert_eq!(
        (lone.as_str(), ending.as_str()),
        (
            "-- BLOCK  p.ts:3 (1 line) · j/k extend · m note · Esc cancel --",
            "-- BLOCK  p.ts:2–3 (2 lines) · j/k extend · m note · Esc cancel --"
        )
    );
}

#[test]
fn the_prompt_quotes_a_blank_last_line_of_a_block() {
    let mut store = NoteStore::in_memory();
    let mut note = block((New, 11), (New, 13), None, "why?");
    note.code = "  if x {\n    y()\n".into();
    store.set(note).unwrap();
    assert_eq!(
        store.prompt(),
        "Please address these review comments. Line numbers refer to the version under review.\n\
         \n1. `a.ts:11–13`: why?\n   ```\n     if x {\n       y()\n   \n   ```\n"
    );
}

/// The colour of the gutter's mark on each pane line from the hunk header down, where
/// it has one: cyan for the cursor, yellow for the rest of a selection.
fn gutter(app: &mut App) -> Vec<Option<Color>> {
    let (width, height) = (120, 20);
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
    terminal.draw(|frame| draw(frame, app)).expect("draw");
    let buffer = terminal.backend().buffer();
    let row = |y: u16| {
        (0..width)
            .map(|x| buffer[(x, y)].symbol())
            .collect::<String>()
    };
    let header = (0..height)
        .find(|&y| row(y).contains("@@ -1 +1 @@"))
        .expect("hunk header");
    // The gutter is the column just left of the header's first `@`.
    let at = (0..width)
        .find(|&x| buffer[(x, header)].symbol() == "@")
        .expect("@")
        - 1;
    (header..height)
        .map(|y| {
            let cell = &buffer[(at, y)];
            (cell.symbol() == "▌").then_some(cell.fg)
        })
        .collect()
}

#[test]
fn a_selection_up_onto_a_note_line_highlights_the_row_the_note_sits_under() {
    let mut app = cart_app(seeded((New, 2), (New, 2), "first", false));
    let rows = pane(&mut app);
    assert!(rows[3].contains("✎ first"), "{}", rows[3]);
    press(&mut app, &[j(), j()]);
    assert_eq!(app.cursor, 4);
    press(&mut app, &[v(), KeyCode::Char('k')]);
    assert_eq!(app.cursor, 3);
    assert!(
        block_status(&mut app).starts_with("-- BLOCK  cart.ts:2–4 (3 lines) ·"),
        "{}",
        block_status(&mut app)
    );
    let marks = gutter(&mut app);
    assert_eq!(
        marks[1..=5],
        [
            None,
            Some(Color::Yellow),
            Some(Color::Cyan),
            Some(Color::Yellow),
            None
        ]
    );
    type_note(&mut app, "block");
    assert_eq!(
        texts(app.notes.find_block("cart.ts", (New, 2), (New, 4))),
        Some("block")
    );
}
