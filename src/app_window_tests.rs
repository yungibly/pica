//! Windowed layout through the editor: scrolling, paging, and editing far
//! from the top must behave as whole-document layout did, at bounded cost.
use super::*;
use crate::projection::TABLE_ROW_LIMIT;
use ratatui::{Terminal, backend::TestBackend};

fn draw(app: &mut App, width: u16, height: u16) -> Terminal<TestBackend> {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    terminal
}

fn press(app: &mut App, code: KeyCode, modifiers: KeyModifiers) {
    app.handle_event(Event::Key(KeyEvent::new(code, modifiers)));
}

fn wheel(app: &mut App, down: bool) {
    app.handle_event(Event::Mouse(MouseEvent {
        kind: if down {
            MouseEventKind::ScrollDown
        } else {
            MouseEventKind::ScrollUp
        },
        column: app.viewport.x,
        row: app.viewport.y,
        modifiers: KeyModifiers::NONE,
    }));
}

/// The caret is drawn inside the viewport.
fn caret_on_screen(app: &App) -> bool {
    app.caret_cell().is_some_and(|(row, _)| {
        row >= app.scroll && row < app.scroll + usize::from(app.viewport.height)
    })
}

/// Rows laid out stay within a few screens, however long the document.
fn bounded(app: &App) -> bool {
    app.projection.row_count() <= 4 * usize::from(app.viewport.height) + 2 * OVERSCAN + 8
}

/// A long Markdown document with wrapping prose, lists, tasks, quotes,
/// tables, fences, and a link whose title hides a line ending.
fn long_markdown(sections: usize) -> String {
    let mut text = String::from("# Title\n\n");
    for n in 0..sections {
        text.push_str(&format!(
            "## Section {n}\n\nParagraph {n} has **bold**, *italic*, `code`, and a \
             [link](https://example.com/{n} \"a title\nacross lines\") in prose long \
             enough to wrap more than once at the widths used here.\n\n\
             - First item {n}\n- [ ] Task {n}\n  - Nested {n}\n\n\
             > Quoted {n} with *emphasis*.\n\n\
             | Name | Value |\n| --- | ---: |\n| alpha | {n} |\n| beta | {} |\n\n\
             ```rust\nfn section_{n}() -> usize {{\n    {n}\n}}\n```\n\n",
            n * 2
        ));
    }
    text
}

/// Caret offsets after each vertical move under whole-document layout,
/// drawn between moves as the editor did before windowing.
fn whole_document_moves(
    text: &str,
    markdown: bool,
    live: bool,
    width: usize,
    delta: isize,
    steps: usize,
) -> Vec<usize> {
    let document = Document::new(text);
    let snapshot = if markdown {
        document.markdown()
    } else {
        MarkdownSnapshot::default()
    };
    let parsed = Parsed::new(text, snapshot, markdown, None);
    let (mut head, mut affinity, mut preferred) = (0, Affinity::Downstream, None);
    let mut moves = Vec::new();
    for _ in 0..steps {
        let projection = Projection::build(text, &parsed, Selection::caret(head), width, live);
        let (row, column) = projection.cursor_with_affinity(head, affinity);
        let column = *preferred.get_or_insert(column);
        let hit = projection.hit(projection.navigable_row(row, delta), column);
        head = document.floor_grapheme_boundary(hit.offset);
        affinity = hit.affinity;
        moves.push(head);
    }
    moves
}

fn windowed_moves(app: &mut App, code: KeyCode, steps: usize) -> Vec<usize> {
    let size = (app.editor_area.width, app.editor_area.height);
    (0..steps)
        .map(|_| {
            press(app, code, KeyModifiers::NONE);
            draw(app, size.0, size.1);
            assert!(
                caret_on_screen(app),
                "caret {} at {:?}, scroll {}, window {}..{} of {} rows",
                app.document.selection().head,
                app.caret_cell(),
                app.scroll,
                app.projection.start(),
                app.projection.end(),
                app.projection.row_count()
            );
            assert!(
                bounded(app) || app.projection.row_count() < TABLE_ROW_LIMIT + 100,
                "{} rows",
                app.projection.row_count()
            );
            app.document.selection().head
        })
        .collect()
}

#[test]
fn vertical_movement_through_a_long_live_document_matches_whole_layout() {
    let text = long_markdown(12);
    for (live, width, height) in [(true, 80, 24), (true, 46, 17), (false, 60, 20)] {
        let mut app = App::new(text.clone(), None, true);
        app.live = live;
        draw(&mut app, width, height);
        let columns = usize::from(app.viewport.width);
        let rows = usize::from(app.viewport.height) as isize;
        // Enough moves to reach the end through wrapped rows and grids.
        let lines = text.lines().count() * 2;
        let down = windowed_moves(&mut app, KeyCode::Down, lines);
        assert_eq!(
            down,
            whole_document_moves(&text, true, live, columns, 1, lines)
        );
        assert_eq!(app.document.selection().head, text.len());
        // Paging from the top crosses window edges a screen at a time.
        press(&mut app, KeyCode::Home, KeyModifiers::CONTROL);
        draw(&mut app, width, height);
        let pages = lines / 10;
        let paged = windowed_moves(&mut app, KeyCode::PageDown, pages);
        assert_eq!(
            paged,
            whole_document_moves(&text, true, live, columns, rows, pages)
        );
        let back: Vec<_> = windowed_moves(&mut app, KeyCode::PageUp, pages);
        assert_eq!(*back.last().unwrap(), 0);
        assert_eq!(app.scroll, 0);
        assert_eq!(app.document.text(), text);
    }
}

#[test]
fn wheel_scrolling_moves_through_the_document_three_rows_at_a_time() {
    let text: String = (1..=3000).map(|n| format!("line {n}\n")).collect();
    let mut app = App::new(text.clone(), None, false);
    draw(&mut app, 60, 20);
    for _ in 0..500 {
        wheel(&mut app, true);
        draw(&mut app, 60, 20);
        assert!(bounded(&app));
    }
    // The caret stays where it was; the view moved 1500 lines.
    assert_eq!(app.document.selection().head, 0);
    assert_eq!(app.projection.row(app.scroll).line, Some(1501));
    let screen = draw(&mut app, 60, 20);
    let first = (0..60)
        .map(|x| {
            screen.backend().buffer()[(x, app.viewport.y)]
                .symbol()
                .to_string()
        })
        .collect::<String>();
    assert!(first.contains("line 1501"), "{first:?}");
    for _ in 0..200 {
        wheel(&mut app, false);
        draw(&mut app, 60, 20);
    }
    assert_eq!(app.projection.row(app.scroll).line, Some(901));
    // A key brings the view back to the caret.
    press(&mut app, KeyCode::Right, KeyModifiers::NONE);
    draw(&mut app, 60, 20);
    assert!(caret_on_screen(&app));
    assert_eq!(app.projection.row(app.scroll).line, Some(1));
    // Wheeling past the end keeps the last page full.
    for _ in 0..1100 {
        wheel(&mut app, true);
        draw(&mut app, 60, 20);
    }
    assert!(app.projection.complete());
    assert_eq!(
        app.scroll,
        app.projection.row_count() - usize::from(app.viewport.height)
    );
}

#[test]
fn editing_at_the_end_of_a_long_file_keeps_the_caret_in_view() {
    let text: String = (1..=5000)
        .map(|n| format!("    let value_{n} = {n}; // a comment\n"))
        .collect();
    let path = std::path::Path::new("long.rs");
    let mut app = App::new(text.clone(), None, false);
    app.language = Language::for_path(path);
    app.refresh_mode();
    draw(&mut app, 80, 24);
    press(&mut app, KeyCode::End, KeyModifiers::CONTROL);
    draw(&mut app, 80, 24);
    assert!(caret_on_screen(&app));
    let height = usize::from(app.viewport.height);
    assert_eq!(app.scroll, app.projection.row_count() - height);
    for c in "fn tail() {}".chars() {
        press(&mut app, KeyCode::Char(c), KeyModifiers::NONE);
        draw(&mut app, 80, 24);
        assert!(caret_on_screen(&app));
    }
    for _ in 0..30 {
        press(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        draw(&mut app, 80, 24);
        assert!(caret_on_screen(&app));
        assert!(bounded(&app));
    }
    for _ in 0..30 {
        press(&mut app, KeyCode::Backspace, KeyModifiers::NONE);
        draw(&mut app, 80, 24);
        assert!(caret_on_screen(&app));
    }
    assert_eq!(app.document.text(), format!("{text}fn tail() {{}}"));
    // Highlighting reaches the end of the file in the window.
    let row = app.caret_cell().unwrap().0;
    let keyword = app
        .projection
        .row(row)
        .glyphs
        .iter()
        .find(|glyph| glyph.text == "f")
        .unwrap();
    assert_ne!(keyword.style.fg, Some(palette().foreground));
    while app.document.can_undo() {
        press(&mut app, KeyCode::Char('z'), KeyModifiers::CONTROL);
        draw(&mut app, 80, 24);
        assert!(caret_on_screen(&app));
    }
    assert_eq!(app.document.text(), text);
}

#[test]
fn a_fence_spanning_the_file_lays_out_only_the_window() {
    let body: String = (0..20_000).map(|n| format!("let x{n} = {n};\n")).collect();
    let text = format!("intro\n\n```rust\n{body}```\n");
    let mut app = App::new(text.clone(), None, true);
    draw(&mut app, 80, 24);
    assert!(bounded(&app));
    press(&mut app, KeyCode::End, KeyModifiers::CONTROL);
    draw(&mut app, 80, 24);
    assert!(caret_on_screen(&app) && bounded(&app));
    press(&mut app, KeyCode::Up, KeyModifiers::NONE);
    press(&mut app, KeyCode::Up, KeyModifiers::NONE);
    draw(&mut app, 80, 24);
    press(&mut app, KeyCode::Char('y'), KeyModifiers::NONE);
    draw(&mut app, 80, 24);
    assert!(caret_on_screen(&app) && bounded(&app));
    let middle = text.find("let x10000 ").unwrap();
    app.jump_to_source(middle);
    draw(&mut app, 80, 24);
    assert!(caret_on_screen(&app) && bounded(&app));
    let reading = usize::from(app.viewport.height) / 3;
    assert_eq!(app.caret_cell().unwrap().0 - app.scroll, reading);
    for _ in 0..50 {
        press(&mut app, KeyCode::PageDown, KeyModifiers::NONE);
        draw(&mut app, 80, 24);
        assert!(caret_on_screen(&app) && bounded(&app));
    }
}

#[test]
fn a_table_over_the_row_limit_stays_source_and_bounded() {
    let rows: String = (0..TABLE_ROW_LIMIT + 50)
        .map(|n| format!("| cell {n} | {n} |\n"))
        .collect();
    let text = format!("intro\n\n| Name | Value |\n| --- | --- |\n{rows}\nafter\n");
    let mut app = App::new(text.clone(), None, true);
    let screen = draw(&mut app, 80, 24);
    let shown: String = (0..24)
        .flat_map(|y| (0..80).map(move |x| (x, y)))
        .map(|cell| screen.backend().buffer()[cell].symbol().to_string())
        .collect();
    assert!(shown.contains("| cell 0 |") && !shown.contains('╭'));
    press(&mut app, KeyCode::End, KeyModifiers::CONTROL);
    draw(&mut app, 80, 24);
    assert!(caret_on_screen(&app) && bounded(&app));
    // A table within the limit still renders as a grid, a window at a time.
    let rows: String = (1..TABLE_ROW_LIMIT)
        .map(|n| format!("| cell {n} | {n} |\n"))
        .collect();
    let text = format!("intro\n\n| Name | Value |\n| --- | --- |\n{rows}\nafter\n");
    let mut app = App::new(text.clone(), None, true);
    let screen = draw(&mut app, 80, 24);
    let shown: String = (0..24)
        .flat_map(|y| (0..80).map(move |x| (x, y)))
        .map(|cell| screen.backend().buffer()[cell].symbol().to_string())
        .collect();
    assert!(shown.contains('╭'));
    let columns = usize::from(app.viewport.width);
    let steps = TABLE_ROW_LIMIT + 10;
    assert_eq!(
        windowed_moves(&mut app, KeyCode::Down, steps),
        whole_document_moves(&text, true, true, columns, 1, steps)
    );
}

#[test]
fn a_very_long_line_lays_out_only_the_rows_shown_from_its_start() {
    let line = "word ".repeat(30_000);
    let text = format!("before\n{line}\nafter\n");
    let mut app = App::new(text.clone(), None, false);
    draw(&mut app, 80, 24);
    assert!(bounded(&app));
    press(&mut app, KeyCode::Down, KeyModifiers::NONE);
    for _ in 0..20 {
        press(&mut app, KeyCode::Char('x'), KeyModifiers::NONE);
        draw(&mut app, 80, 24);
        assert!(caret_on_screen(&app) && bounded(&app));
    }
    // Reaching the end of the line lays out the line up to the caret.
    press(&mut app, KeyCode::End, KeyModifiers::CONTROL);
    draw(&mut app, 80, 24);
    assert!(caret_on_screen(&app));
    press(&mut app, KeyCode::Up, KeyModifiers::NONE);
    draw(&mut app, 80, 24);
    assert!(caret_on_screen(&app));
    assert!(app.document.selection().head > text.len() / 2);
}

#[test]
fn large_files_open_as_plain_source_without_changing_the_live_preference() {
    let mut text = "- item with **bold** text\n".repeat(LARGE_FILE / 20);
    text.push_str("- last");
    let mut app = App::new(text.clone(), None, true);
    assert!(app.large && app.live && !app.live_view());
    assert!(app.message.contains("Large file"));
    let screen = draw(&mut app, 80, 24);
    let first: String = (0..80)
        .map(|x| {
            screen.backend().buffer()[(x, app.viewport.y)]
                .symbol()
                .to_string()
        })
        .collect();
    assert!(first.contains("- item with **bold** text"), "{first:?}");
    assert!(app.parsed.snapshot.blocks.is_empty());
    // Enter is literal: no list continuation from a whole-document parse.
    press(&mut app, KeyCode::End, KeyModifiers::CONTROL);
    press(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    draw(&mut app, 80, 24);
    assert!(app.document.text().ends_with("- last\n"));
    press(&mut app, KeyCode::Char('t'), KeyModifiers::CONTROL);
    assert!(app.document.text().ends_with("- last\n"));
    // Toggling records the preference; the view stays source while large.
    press(&mut app, KeyCode::F(6), KeyModifiers::NONE);
    assert!(!app.live && !app.live_view());
    press(&mut app, KeyCode::F(6), KeyModifiers::NONE);
    assert!(app.live && !app.live_view());
    // Words are counted in pauses; large files show lines instead.
    assert!(app.extent().ends_with("lines"));
    // Shrinking well below the threshold restores the live view.
    press(&mut app, KeyCode::Char('a'), KeyModifiers::CONTROL);
    app.handle_event(Event::Paste("- small\n".into()));
    draw(&mut app, 80, 24);
    assert!(!app.large && app.live_view());
    press(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(app.document.text(), "- small\n\n");
}

#[test]
fn word_counts_wait_for_a_pause_past_a_small_document() {
    let mut app = App::new("one two three".into(), None, true);
    assert_eq!(app.source_word_count(), 3);
    press(&mut app, KeyCode::Char('x'), KeyModifiers::NONE);
    assert_eq!(app.source_word_count(), 3);
    let text = "word ".repeat(EAGER_WORD_COUNT);
    let mut app = App::new(text, None, true);
    assert_eq!(app.source_word_count(), EAGER_WORD_COUNT);
    press(&mut app, KeyCode::End, KeyModifiers::CONTROL);
    app.handle_event(Event::Paste(" more".into()));
    assert_eq!(app.source_word_count(), EAGER_WORD_COUNT);
    assert!(!app.idle());
    app.last_input -= std::time::Duration::from_secs(1);
    assert!(app.idle());
    assert_eq!(app.source_word_count(), EAGER_WORD_COUNT + 1);
    assert!(!app.idle());
}

#[test]
fn enter_reuses_the_drawn_analysis() {
    let mut app = App::new("- one `code` here".into(), None, true);
    draw(&mut app, 80, 24);
    let revision = app.parsed.snapshot.revision;
    assert_eq!(revision, app.document.revision());
    press(&mut app, KeyCode::End, KeyModifiers::CONTROL);
    press(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(app.document.text(), "- one `code` here\n- ");
    // Inside inline code, Enter stays literal using the cached code spans.
    let mut app = App::new("- one `code` here".into(), None, true);
    draw(&mut app, 80, 24);
    app.document.set_caret(9).unwrap();
    press(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(app.document.text(), "- one `co\nde` here");
}

#[test]
#[ignore]
fn probe_parse_costs() {
    use std::time::Instant;
    let path = std::env::var("PROBE").unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    let time = |label: &str, f: &mut dyn FnMut()| {
        f();
        let t = Instant::now();
        for _ in 0..10 {
            f();
        }
        eprintln!("{label}: {:?}", t.elapsed() / 10);
    };
    time("snapshot", &mut || {
        std::hint::black_box(milli::markdown::analyze(&text, 0));
    });
    let snapshot = milli::markdown::analyze(&text, 0);
    time("parsed", &mut || {
        std::hint::black_box(Parsed::new(&text, snapshot.clone(), true, None));
    });
    time("parsed with snapshot", &mut || {
        std::hint::black_box(Parsed::markdown(&text, 0));
    });
    time("pulldown", &mut || {
        let input = milli::markdown::parser_input(&text);
        let parser = pulldown_cmark::Parser::new_ext(&input, milli::markdown::options());
        std::hint::black_box(parser.into_offset_iter().count());
    });
    time("words", &mut || {
        std::hint::black_box(text.unicode_words().count());
    });
}

#[test]
#[ignore]
fn probe_frame_costs() {
    use std::time::Instant;
    let path = std::env::var("PROBE").unwrap();
    let mut app = crate::workspace::Workspace::open(Some(path.clone().into())).unwrap();
    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let lines = app.editor().document.line_count();
    let fraction: usize = std::env::var("AT").map_or(2, |at| at.parse().unwrap());
    let middle = app
        .editor()
        .document
        .line_start(lines * fraction / 4)
        .min(app.editor().document.text().len());
    app.editor_mut().jump_to_source(middle);
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let mut total = Vec::new();
    for round in 0..40 {
        let key = if round % 2 == 0 {
            KeyCode::Char('x')
        } else {
            KeyCode::Backspace
        };
        let t = Instant::now();
        app.handle_event(Event::Key(KeyEvent::new(key, KeyModifiers::NONE)));
        let handled = t.elapsed();
        let editor = app.editor_mut();
        let p = Instant::now();
        editor.ensure_parsed();
        let parsed = p.elapsed();
        let l = Instant::now();
        editor.update_layout();
        let laid = l.elapsed();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        total.push((t.elapsed(), handled, parsed, laid));
    }
    total.sort_by_key(|t| t.0);
    eprintln!(
        "{path}: total p50 {:?} p95 {:?} · handle {:?} parse {:?} layout {:?} rows {}",
        total[20].0,
        total[38].0,
        total[20].1,
        total[20].2,
        total[20].3,
        app.editor().projection.row_count()
    );
}
