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
    whole_document_moves_from(0, text, markdown, live, width, delta, steps)
}

fn whole_document_moves_from(
    from: usize,
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
    let (mut head, mut affinity, mut preferred) = (from, Affinity::Downstream, None);
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

/// Every visible glyph has the style whole-document layout gives it.
fn styles_match_whole_layout(app: &App) {
    let text = app.document.text();
    let parsed = Parsed::markdown(text, app.document.revision());
    let whole = Projection::build(
        text,
        &parsed,
        app.document.selection(),
        usize::from(app.viewport.width),
        app.live_view(),
    );
    let expected: std::collections::HashMap<_, _> = whole
        .rows(0..whole.row_count())
        .iter()
        .flat_map(|row| &row.glyphs)
        .filter(|glyph| !glyph.source.is_empty())
        .map(|glyph| (glyph.source.start, glyph.style))
        .collect();
    let height = usize::from(app.viewport.height);
    for row in app.projection.rows(app.scroll..app.scroll + height) {
        for glyph in row.glyphs.iter().filter(|glyph| !glyph.source.is_empty()) {
            assert_eq!(
                Some(&glyph.style),
                expected.get(&glyph.source.start),
                "{:?} at {}",
                glyph.text,
                glyph.source.start
            );
        }
    }
}

#[test]
fn code_blocks_highlight_by_window_as_whole_documents_do() {
    let small = "```python\ndef f(x):\n    return \"s\" # note\n```\n\n";
    let large: String = (0..1200)
        .map(|n| format!("    let value_{n} = \"text {n}\"; // note\n"))
        .collect();
    let text = format!(
        "# Title\n\n{small}> ```rust\n> /* open\n> still */ fn f() {{}}\n> ```\n\n\
         ```rust\n{large}```\n\n{small}after\n"
    );
    assert!(text.len() > LARGE_FENCE * 4);
    let mut app = App::new(text.clone(), None, true);
    draw(&mut app, 80, 24);
    styles_match_whole_layout(&app);
    for fraction in [1, 2, 3, 5, 8, 9] {
        let offset = app
            .document
            .floor_grapheme_boundary(text.len() * fraction / 10);
        app.jump_to_source(offset);
        draw(&mut app, 80, 24);
        styles_match_whole_layout(&app);
        // Edits inside the block keep earlier saved lexer states.
        for c in "/* x".chars() {
            press(&mut app, KeyCode::Char(c), KeyModifiers::NONE);
            draw(&mut app, 80, 24);
            styles_match_whole_layout(&app);
        }
        for _ in 0..4 {
            press(&mut app, KeyCode::Backspace, KeyModifiers::NONE);
        }
        draw(&mut app, 80, 24);
        styles_match_whole_layout(&app);
    }
    assert_eq!(app.document.text(), text);
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
    // Reaching the end of the line lays it out once; its row starts let
    // later windows resume near the caret.
    let line_end = text.len() - "\nafter\n".len() + 20;
    app.document.set_caret(line_end).unwrap();
    app.jump_to_source(line_end);
    draw(&mut app, 80, 24);
    assert!(caret_on_screen(&app));
    for c in "tail".chars() {
        press(&mut app, KeyCode::Char(c), KeyModifiers::NONE);
        draw(&mut app, 80, 24);
        assert!(caret_on_screen(&app) && bounded(&app));
        assert!(app.projection.first_row() > 0);
    }
    // Moving through the line agrees with laying out the whole document.
    let text = app.document.text().to_owned();
    let from = app.document.selection().head;
    let columns = usize::from(app.viewport.width);
    let height = usize::from(app.viewport.height) as isize;
    let paged = windowed_moves(&mut app, KeyCode::PageUp, 12);
    assert_eq!(
        paged,
        whole_document_moves_from(from, &text, false, false, columns, -height, 12)
    );
    let from = app.document.selection().head;
    let down = windowed_moves(&mut app, KeyCode::Down, 30);
    assert_eq!(
        down,
        whole_document_moves_from(from, &text, false, false, columns, 1, 30)
    );
    // Typing in the middle of the line keeps the rows above it.
    press(&mut app, KeyCode::Char('y'), KeyModifiers::NONE);
    draw(&mut app, 80, 24);
    assert!(caret_on_screen(&app) && bounded(&app));
    // The view and the line number gutter stay on the long line.
    assert_eq!(app.anchor_at(app.scroll).line, 1);
    assert_eq!(
        app.document.text().len(),
        text.len() + 1,
        "only the typed character changed"
    );
}

/// Every row the window holds begins, ends, and hangs where laying out the
/// whole document would put it.
fn rows_match_whole_layout(app: &App) {
    let text = app.document.text();
    let parsed = Parsed::new(text, MarkdownSnapshot::default(), false, None);
    let whole = Projection::build(
        text,
        &parsed,
        app.document.selection(),
        usize::from(app.viewport.width),
        false,
    );
    let shape = |row: &crate::projection::VisualRow| {
        let first = row.glyphs.iter().find(|glyph| !glyph.source.is_empty());
        (row.end, first.map(|glyph| glyph.column), row.line.is_some())
    };
    let expected: std::collections::HashMap<_, _> = whole
        .rows(0..whole.row_count())
        .iter()
        .map(|row| (row.start, shape(row)))
        .collect();
    let window = app.projection.rows(0..app.projection.row_count());
    for (index, row) in window.iter().enumerate() {
        assert_eq!(
            expected.get(&row.start),
            Some(&shape(row)),
            "window row {index} of {} starting at {}",
            window.len(),
            row.start
        );
    }
}

#[test]
fn edits_deep_in_a_long_wrapped_line_keep_its_rows_where_whole_layout_breaks_them() {
    // Words of many lengths, so rows break at different columns and an
    // edit can pull a word back to the row above or push one down.
    let mut line = String::from("    ");
    for n in 0..9_000 {
        line.push_str(&"w".repeat(1 + (n * 7) % 13));
        line.push(' ');
    }
    let text = format!("before\n{line}\nafter\n");
    let mut app = App::new(text.clone(), None, false);
    draw(&mut app, 80, 24);
    let line_start = text.find('\n').unwrap() + 1;
    // Lay the line out once to its end, so later windows resume inside it.
    app.jump_to_source(line_start + line.len());
    draw(&mut app, 80, 24);
    rows_match_whole_layout(&app);
    let deep = line_start + line.len() * 2 / 3;
    app.jump_to_source(deep);
    draw(&mut app, 80, 24);
    assert!(
        app.projection.first_row() > 0,
        "the window resumes mid-line"
    );
    rows_match_whole_layout(&app);
    let mut resumed = 0;
    for step in 0..60 {
        // Put the caret at, just after, or just before the start of its
        // row, or of a neighbouring row: where a row's first word decides
        // whether it stays or moves.
        let (row, _) = app.caret_position();
        let target = (row + step % 3).min(app.projection.row_count() - 1);
        let start = app.projection.row(target).start;
        let offset = match step % 4 {
            0 => start,
            1 => start + 1,
            2 => start.saturating_sub(1),
            _ => start + 3,
        };
        let offset = app.document.floor_grapheme_boundary(offset);
        app.document.set_caret(offset).unwrap();
        match step % 5 {
            0 => press(&mut app, KeyCode::Backspace, KeyModifiers::NONE),
            1 => press(&mut app, KeyCode::Char('q'), KeyModifiers::NONE),
            2 => press(&mut app, KeyCode::Char(' '), KeyModifiers::NONE),
            3 => press(&mut app, KeyCode::Delete, KeyModifiers::NONE),
            _ => app.handle_event(Event::Paste("pasted words that span rows ".repeat(9))),
        }
        draw(&mut app, 80, 24);
        assert!(caret_on_screen(&app) && bounded(&app), "step {step}");
        rows_match_whole_layout(&app);
        resumed += usize::from(app.projection.first_row() > 0);
        // Rows far below the edit moved with it: a window there must not
        // begin from where they used to start. It is laid out from the
        // edit instead, once.
        if step % 6 == 5 {
            let head = app.document.selection().head;
            let ahead = if step % 12 == 5 { 9_000 } else { 2_500 };
            app.jump_to_source(head + ahead);
            draw(&mut app, 80, 24);
            assert!(caret_on_screen(&app), "step {step}");
            rows_match_whole_layout(&app);
            app.jump_to_source(head.saturating_sub(1_200).max(line_start));
            draw(&mut app, 80, 24);
            rows_match_whole_layout(&app);
        }
    }
    assert!(
        resumed >= 50,
        "edits resumed inside the line {resumed} times"
    );
    // Undo walks the same edits back through the same rows.
    for _ in 0..20 {
        press(&mut app, KeyCode::Char('z'), KeyModifiers::CONTROL);
        draw(&mut app, 80, 24);
        assert!(caret_on_screen(&app));
        rows_match_whole_layout(&app);
    }
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
        std::hint::black_box(pica::markdown::analyze(&text, 0));
    });
    let snapshot = pica::markdown::analyze(&text, 0);
    time("parsed", &mut || {
        std::hint::black_box(Parsed::new(&text, snapshot.clone(), true, None));
    });
    time("parsed with snapshot", &mut || {
        std::hint::black_box(Parsed::markdown(&text, 0));
    });
    time("pulldown", &mut || {
        let input = pica::markdown::parser_input(&text);
        let parser = pulldown_cmark::Parser::new_ext(&input, pica::markdown::options());
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
    // NOWRAP=1 measures the unwrapped view.
    app.set_wrap(std::env::var("NOWRAP").is_err());
    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let lines = app.editor().document.line_count();
    let fraction: usize = std::env::var("AT").map_or(2, |at| at.parse().unwrap());
    // AT=0..4 picks a line a quarter apart; AT=5 is the end of the text.
    let length = app.editor().document.text().len();
    let middle = if fraction > 4 {
        length
    } else {
        app.editor()
            .document
            .line_start(lines * fraction / 4)
            .min(length)
    };
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
