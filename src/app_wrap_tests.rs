//! Word wrap off: one row for each line, a view that scrolls sideways with
//! the caret, and geometry that stays exact while it does.
use super::*;
use ratatui::{Terminal, backend::TestBackend, buffer::Buffer};

const LONG: &str = "fn short() {}\n    let value = compute(alpha, beta, gamma, delta, epsilon, zeta, eta, theta, iota, kappa, lambda);\nfn tail() {}\n\tlast\n";

/// Draw a frame and return its cells and where it put the terminal cursor.
fn frame(app: &mut App, width: u16, height: u16) -> (Buffer, (u16, u16)) {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let cursor = terminal.get_cursor_position().unwrap();
    (terminal.backend().buffer().clone(), (cursor.x, cursor.y))
}

fn press(app: &mut App, code: KeyCode, modifiers: KeyModifiers) {
    app.handle_event(Event::Key(KeyEvent::new(code, modifiers)));
}

fn pointer(app: &mut App, kind: MouseEventKind, column: u16, row: u16, modifiers: KeyModifiers) {
    app.handle_event(Event::Mouse(MouseEvent {
        kind,
        column,
        row,
        modifiers,
    }));
}

/// A code buffer with word wrap off.
fn unwrapped(source: &str) -> App {
    let mut app = App::new(source.into(), None, false);
    app.wrap = false;
    app
}

/// The text column's cells on one screen row.
fn shown(buffer: &Buffer, app: &App, screen_row: u16) -> String {
    (app.viewport.x..app.viewport.right())
        .map(|x| buffer[(x, app.viewport.y + screen_row)].symbol())
        .collect()
}

/// The line-number gutter's cells on one screen row.
fn gutter(buffer: &Buffer, app: &App, screen_row: u16) -> String {
    (0..app.viewport.x)
        .map(|x| buffer[(x, app.viewport.y + screen_row)].symbol())
        .collect()
}

#[test]
fn unwrapped_views_keep_each_line_on_one_row_and_mark_what_is_cut_off() {
    let mut wrapped = App::new(LONG.into(), None, false);
    frame(&mut wrapped, 40, 10);
    assert!(wrapped.projection.row_count() > 5, "the long line wraps");

    let mut app = unwrapped(LONG);
    let (buffer, cursor) = frame(&mut app, 40, 10);
    // Four lines and the empty row after the last line ending.
    assert_eq!(app.projection.row_count(), 5);
    for row in 0..5 {
        assert_eq!(app.projection.row(row).line, Some(row + 1));
    }
    assert_eq!(app.side, 0);
    assert_eq!(cursor, (app.viewport.x, app.viewport.y));
    let width = usize::from(app.viewport.width);
    let line = LONG.lines().nth(1).unwrap();
    // The long line shows as far as the last cell, which holds the mark.
    assert_eq!(shown(&buffer, &app, 1), format!("{}›", &line[..width - 1]));
    assert_eq!(shown(&buffer, &app, 0).trim_end(), "fn short() {}");
    assert_eq!(shown(&buffer, &app, 2).trim_end(), "fn tail() {}");
    assert_eq!(shown(&buffer, &app, 3).trim_end(), "    last");
    for row in 0..4 {
        assert_eq!(gutter(&buffer, &app, row), format!("   {} ", row + 1));
    }
    // A line that fits exactly needs no mark: nothing of it is hidden.
    let exact = "x".repeat(width);
    let mut app = unwrapped(&format!("{exact}\nshort\n"));
    let (buffer, _) = frame(&mut app, 40, 10);
    assert_eq!(shown(&buffer, &app, 0), exact);
    // Turning wrap back on lays the same text out in rows again.
    let mut app = unwrapped(LONG);
    frame(&mut app, 40, 10);
    app.wrap = true;
    let (buffer, _) = frame(&mut app, 40, 10);
    assert_eq!(app.projection.row_count(), wrapped.projection.row_count());
    assert!(!shown(&buffer, &app, 1).contains('›'));
}

#[test]
fn the_view_follows_the_caret_sideways_and_comes_back_to_the_line_start() {
    let mut app = unwrapped(LONG);
    let line = LONG.lines().nth(1).unwrap();
    let start = LONG.find(line).unwrap();
    app.document.set_caret(start).unwrap();
    frame(&mut app, 40, 10);
    let width = usize::from(app.viewport.width);
    let y = app.viewport.y + 1;
    let mut previous = 0;
    for column in 1..=line.len() {
        press(&mut app, KeyCode::Right, KeyModifiers::NONE);
        let (buffer, cursor) = frame(&mut app, 40, 10);
        let x = usize::from(cursor.0 - app.viewport.x);
        assert_eq!(cursor.1, y);
        assert_eq!(x, column - app.side, "the caret is drawn on its column");
        // The cell under the caret holds the character after it.
        if column < line.len() {
            assert_eq!(
                buffer[(cursor.0, y)].symbol(),
                &line[column..column + 1],
                "column {column}"
            );
        }
        // While more of the line follows, a few cells of it show after
        // the caret, and the caret never sits on the mark.
        let cut = line.len() > app.side + width;
        assert_eq!(
            shown(&buffer, &app, 1).ends_with('›'),
            cut,
            "column {column}"
        );
        if cut {
            assert!(x + SIDE_MARGIN + 1 < width, "column {column}: caret at {x}");
        }
        // The view moves a cell at a time, and the numbers stay put.
        assert!(app.side >= previous && app.side - previous <= 1);
        previous = app.side;
        assert_eq!(gutter(&buffer, &app, 1).trim_matches(['‹', ' ']), "2");
    }
    // At the end of the line the caret takes the last cell.
    assert_eq!(app.side, line.len() + 1 - width);
    let (buffer, cursor) = frame(&mut app, 40, 10);
    assert_eq!(usize::from(cursor.0 - app.viewport.x), width - 1);
    assert_eq!(
        shown(&buffer, &app, 1).trim_end(),
        &line[app.side..],
        "the end of the line, with no mark after it"
    );
    // Rows with text hidden on the left say so in the blank cell before
    // the text; the empty last row hides nothing.
    for (row, mark) in [(0, "‹"), (1, "‹"), (2, "‹"), (3, "‹"), (4, " ")] {
        assert_eq!(
            buffer[(app.viewport.x - 1, app.viewport.y + row)].symbol(),
            mark,
            "row {row}"
        );
    }
    assert_eq!(shown(&buffer, &app, 0).trim(), "");
    // Typing at the end keeps the caret in the last cell.
    for c in "xyz".chars() {
        press(&mut app, KeyCode::Char(c), KeyModifiers::NONE);
        let (buffer, cursor) = frame(&mut app, 40, 10);
        assert_eq!(usize::from(cursor.0 - app.viewport.x), width - 1);
        assert!(shown(&buffer, &app, 1).trim_end().ends_with(c));
    }
    for _ in 0..3 {
        press(&mut app, KeyCode::Backspace, KeyModifiers::NONE);
    }
    frame(&mut app, 40, 10);
    // Moving left keeps a few cells before the caret in view, a cell at a
    // time, until the caret fits the first screen; then the lines show
    // from their start again.
    let mut previous = app.side;
    for column in (0..line.len()).rev() {
        press(&mut app, KeyCode::Left, KeyModifiers::NONE);
        let (_, cursor) = frame(&mut app, 40, 10);
        let x = usize::from(cursor.0 - app.viewport.x);
        assert_eq!(x, column - app.side);
        assert!(app.side <= previous);
        if app.side > 0 {
            assert!(x >= SIDE_MARGIN, "column {column}: caret at {x}");
            assert!(previous - app.side <= 1);
        } else {
            assert!(column + SIDE_MARGIN + 2 <= width || previous == 0);
        }
        previous = app.side;
    }
    assert_eq!(app.side, 0);
    // End and Home go to the ends of the line, not of a screenful of it.
    press(&mut app, KeyCode::End, KeyModifiers::NONE);
    frame(&mut app, 40, 10);
    assert_eq!(app.document.selection().head, start + line.len());
    assert!(app.side > 0);
    press(&mut app, KeyCode::Home, KeyModifiers::NONE);
    let (buffer, _) = frame(&mut app, 40, 10);
    assert_eq!(app.document.selection().head, start + 4);
    assert_eq!(app.side, 0);
    assert!(shown(&buffer, &app, 1).starts_with("    let value"));
    // Moving down from the end of a long line onto short ones brings the
    // view back too, since their ends fit the first screen.
    press(&mut app, KeyCode::End, KeyModifiers::NONE);
    frame(&mut app, 40, 10);
    press(&mut app, KeyCode::Down, KeyModifiers::NONE);
    let (_, cursor) = frame(&mut app, 40, 10);
    assert_eq!(app.side, 0);
    assert_eq!(cursor, (app.viewport.x + 12, app.viewport.y + 2));
    assert_eq!(app.document.text(), LONG);
}

#[test]
fn clicks_and_drags_land_on_the_text_under_them_in_a_scrolled_view() {
    theme::set_theme(Theme::Dark);
    let mut app = unwrapped(LONG);
    let line = LONG.lines().nth(1).unwrap();
    let start = LONG.find(line).unwrap();
    app.document.set_caret(start + line.len()).unwrap();
    frame(&mut app, 40, 10);
    let side = app.side;
    assert!(side > 20);
    let (x, y) = (app.viewport.x, app.viewport.y + 1);
    let none = KeyModifiers::NONE;
    let left = MouseButton::Left;
    // A click lands on the character drawn in that cell.
    pointer(&mut app, MouseEventKind::Down(left), x + 9, y, none);
    pointer(&mut app, MouseEventKind::Up(left), x + 9, y, none);
    assert_eq!(app.document.selection().head, start + side + 9);
    let (buffer, cursor) = frame(&mut app, 40, 10);
    assert_eq!(
        app.side, side,
        "a click on visible text does not move the view"
    );
    assert_eq!(cursor, (x + 9, y));
    assert_eq!(buffer[(x + 9, y)].symbol(), &line[side + 9..side + 10]);
    // A drag selects the cells it crosses, and they are drawn selected.
    pointer(&mut app, MouseEventKind::Down(left), x + 6, y, none);
    pointer(&mut app, MouseEventKind::Drag(left), x + 15, y, none);
    pointer(&mut app, MouseEventKind::Up(left), x + 15, y, none);
    assert_eq!(
        app.document.selection().range(),
        start + side + 6..start + side + 15
    );
    assert_eq!(app.document.selected_text(), &line[side + 6..side + 15]);
    let (buffer, _) = frame(&mut app, 40, 10);
    assert_eq!(app.side, side);
    let selection = palette().selection;
    for cell in 0..app.viewport.width {
        assert_eq!(
            buffer[(x + cell, y)].bg == selection,
            (6..15).contains(&cell),
            "cell {cell}"
        );
    }
    // Shift+click extends to another row at the scrolled column.
    pointer(
        &mut app,
        MouseEventKind::Down(left),
        x + 2,
        y - 1,
        KeyModifiers::SHIFT,
    );
    pointer(&mut app, MouseEventKind::Up(left), x + 2, y - 1, none);
    // The first line ends before the view begins: the click lands at its end.
    assert_eq!(app.document.selection().head, "fn short() {}".len());
    // The cell holding the right-hand mark still maps to the text under it.
    app.document.set_caret(start).unwrap();
    press(&mut app, KeyCode::Home, none);
    let (buffer, _) = frame(&mut app, 40, 10);
    assert_eq!(app.side, 0);
    let last = app.viewport.right() - 1;
    assert_eq!(buffer[(last, y)].symbol(), "›");
    pointer(&mut app, MouseEventKind::Down(left), last, y, none);
    pointer(&mut app, MouseEventKind::Up(left), last, y, none);
    let width = usize::from(app.viewport.width);
    assert_eq!(app.document.selection().head, start + width - 1);
    assert_eq!(app.document.text(), LONG);
}

#[test]
fn find_scrolls_sideways_to_a_match_and_draws_it_where_it_is() {
    theme::set_theme(Theme::Dark);
    let mut app = unwrapped(LONG);
    frame(&mut app, 40, 12);
    press(&mut app, KeyCode::Char('f'), KeyModifiers::CONTROL);
    app.handle_event(Event::Paste("kappa".into()));
    let (buffer, _) = frame(&mut app, 40, 12);
    let line = LONG.lines().nth(1).unwrap();
    let column = line.find("kappa").unwrap();
    assert!(app.side > 0 && column >= app.side);
    let x = app.viewport.x + (column - app.side) as u16;
    let y = app.viewport.y + 1;
    let found: String = (0..5).map(|cell| buffer[(x + cell, y)].symbol()).collect();
    assert_eq!(found, "kappa");
    for cell in 0..5 {
        assert_eq!(buffer[(x + cell, y)].bg, palette().search_active);
    }
    assert_ne!(buffer[(x - 1, y)].bg, palette().search_active);
    assert_ne!(buffer[(x + 5, y)].bg, palette().search_active);
    press(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    // Going to a line and column brings that column into view as well.
    app.go_to(2, 70);
    let (_, cursor) = frame(&mut app, 40, 12);
    assert_eq!(usize::from(cursor.0 - app.viewport.x), 69 - app.side);
    assert!(app.side > 0);
}

#[test]
fn indent_guides_and_the_current_line_scroll_with_the_text() {
    theme::set_theme(Theme::Dark);
    let deep = " ".repeat(16);
    let source = format!(
        "fn a() {{\n    if b {{\n{deep}call(one, two, three, four, five, six, seven, eight, nine, ten);\n{deep}next();\n    }}\n}}\n"
    );
    let mut app = unwrapped(&source);
    app.guides = true;
    let line = source.lines().nth(2).unwrap();
    let start = source.find(line).unwrap();
    // Far enough along the long line to scroll, not far enough to hide all
    // of the indentation.
    app.document.set_caret(start + 44).unwrap();
    let (buffer, _) = frame(&mut app, 46, 10);
    let side = app.side;
    assert!(side > 0 && side < 12, "side {side}");
    let y = app.viewport.y + 3;
    for column in side..16 {
        let cell = &buffer[(app.viewport.x + (column - side) as u16, y)];
        let guide = column % 4 == 0;
        assert_eq!(
            cell.symbol(),
            if guide { "│" } else { " " },
            "column {column}"
        );
    }
    assert_eq!(
        buffer[(app.viewport.x + (16 - side) as u16, y)].symbol(),
        "n"
    );
    // The caret's line is highlighted across the whole text column.
    let current = app.viewport.y + 2;
    for x in app.viewport.x..app.viewport.right() {
        assert_eq!(buffer[(x, current)].bg, palette().cursorline);
    }
}

#[test]
fn a_sideways_wheel_scrolls_an_unwrapped_view_and_typing_returns_to_the_caret() {
    let mut app = unwrapped(LONG);
    frame(&mut app, 40, 10);
    let (x, y) = (app.viewport.x + 3, app.viewport.y + 1);
    let width = usize::from(app.viewport.width);
    let line = LONG.lines().nth(1).unwrap();
    let none = KeyModifiers::NONE;
    pointer(&mut app, MouseEventKind::ScrollRight, x, y, none);
    let (buffer, _) = frame(&mut app, 40, 10);
    assert_eq!(app.side, SIDE_STEP);
    assert!(shown(&buffer, &app, 1).starts_with(&line[SIDE_STEP..SIDE_STEP + 10]));
    assert_eq!(
        app.document.selection().head,
        0,
        "the caret stays where it was"
    );
    // Shift turns the ordinary wheel sideways.
    pointer(
        &mut app,
        MouseEventKind::ScrollDown,
        x,
        y,
        KeyModifiers::SHIFT,
    );
    frame(&mut app, 40, 10);
    assert_eq!((app.side, app.scroll), (2 * SIDE_STEP, 0));
    // It stops where the longest visible line ends, with room for a caret.
    for _ in 0..40 {
        pointer(&mut app, MouseEventKind::ScrollRight, x, y, none);
    }
    let (buffer, _) = frame(&mut app, 40, 10);
    assert_eq!(app.side, line.len() + 1 - width);
    assert_eq!(shown(&buffer, &app, 1).trim_end(), &line[app.side..]);
    pointer(&mut app, MouseEventKind::ScrollLeft, x, y, none);
    pointer(
        &mut app,
        MouseEventKind::ScrollUp,
        x,
        y,
        KeyModifiers::SHIFT,
    );
    frame(&mut app, 40, 10);
    assert_eq!(app.side, line.len() + 1 - width - 2 * SIDE_STEP);
    // A click in the scrolled view lands under the pointer.
    pointer(
        &mut app,
        MouseEventKind::Down(MouseButton::Left),
        x + 4,
        y,
        none,
    );
    pointer(
        &mut app,
        MouseEventKind::Up(MouseButton::Left),
        x + 4,
        y,
        none,
    );
    let start = LONG.find(line).unwrap();
    assert_eq!(app.document.selection().head, start + app.side + 7);
    // Typing follows the caret again: back at the top, the view returns.
    app.document.set_caret(0).unwrap();
    for _ in 0..40 {
        pointer(&mut app, MouseEventKind::ScrollLeft, x, y, none);
    }
    pointer(&mut app, MouseEventKind::ScrollRight, x, y, none);
    frame(&mut app, 40, 10);
    assert_eq!(app.side, SIDE_STEP);
    press(&mut app, KeyCode::Char('x'), none);
    let (buffer, cursor) = frame(&mut app, 40, 10);
    assert_eq!(app.side, 0);
    assert_eq!(cursor, (app.viewport.x + 1, app.viewport.y));
    assert!(shown(&buffer, &app, 0).starts_with("xfn short"));
    // Where lines wrap there is nowhere to go sideways, and Shift leaves
    // the wheel scrolling down the document.
    let mut wrapped = App::new("line\n".repeat(60), None, false);
    frame(&mut wrapped, 40, 10);
    pointer(&mut wrapped, MouseEventKind::ScrollRight, x, y, none);
    frame(&mut wrapped, 40, 10);
    assert_eq!((wrapped.side, wrapped.scroll), (0, 0));
    pointer(
        &mut wrapped,
        MouseEventKind::ScrollDown,
        x,
        y,
        KeyModifiers::SHIFT,
    );
    frame(&mut wrapped, 40, 10);
    assert_eq!((wrapped.side, wrapped.scroll), (0, 3));
}

#[test]
fn markdown_source_unwraps_while_the_live_view_keeps_wrapping() {
    let prose = "A paragraph of prose with **bold** words that runs well past the edge of a narrow window to its end.";
    let source = format!(
        "# Title\n\n{prose}\n\n```rust\nfn main() {{ let answer = compute(1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13); }}\n```\n"
    );
    let mut app = App::new(source.clone(), None, true);
    app.wrap = false;
    let caret = source.find(prose).unwrap() + prose.len();
    // Rendered Markdown wraps whatever the setting: its prose and its code.
    assert!(app.live_view() && !app.unwrapped());
    app.document.set_caret(source.len()).unwrap();
    let (buffer, _) = frame(&mut app, 50, 20);
    assert!(app.projection.row_count() > source.lines().count());
    assert_eq!(app.side, 0);
    for row in 0..app.viewport.height {
        assert!(!shown(&buffer, &app, row).contains('›'), "row {row}");
    }
    app.document.set_caret(caret).unwrap();
    frame(&mut app, 50, 20);
    assert_eq!(app.side, 0);
    // Its source view keeps each line on a row and follows the caret.
    press(&mut app, KeyCode::Char('r'), KeyModifiers::CONTROL);
    assert!(app.unwrapped());
    let (buffer, cursor) = frame(&mut app, 50, 20);
    assert_eq!(app.projection.row_count(), source.lines().count() + 1);
    let width = usize::from(app.viewport.width);
    assert_eq!(app.side, prose.len() + 1 - width);
    assert_eq!(cursor.0, app.viewport.right() - 1);
    assert_eq!(shown(&buffer, &app, 2).trim_end(), &prose[app.side..]);
    assert_eq!(gutter(&buffer, &app, 2), "   3‹");
    // Back in the live view nothing is scrolled.
    press(&mut app, KeyCode::Char('r'), KeyModifiers::CONTROL);
    let (buffer, _) = frame(&mut app, 50, 20);
    assert_eq!(app.side, 0);
    assert!(shown(&buffer, &app, 0).contains("Title"));
    assert_eq!(app.document.text(), source);
}

#[test]
fn wide_text_and_tabs_scroll_sideways_without_half_glyphs() {
    let line =
        "界面\tab界面界面 middle 界面界面界面界面 text\t界面界面 end of the 界面 line 界面界面界面";
    let source = format!("{line}\nnext\n");
    let mut app = unwrapped(&source);
    app.ruler = false;
    frame(&mut app, 24, 6);
    let width = usize::from(app.viewport.width);
    let steps = line.chars().count();
    let mut sides = std::collections::BTreeSet::new();
    for step in 0..=steps {
        let (buffer, cursor) = frame(&mut app, 24, 6);
        let (row, column) = app.caret_position();
        assert_eq!(row, 0);
        let x = usize::from(cursor.0 - app.viewport.x);
        assert_eq!(x, column - app.side, "step {step}");
        assert!(x < width);
        // Every glyph in view is drawn whole at its own cells; one cut by
        // the left edge leaves blank cells, and none shares the mark's.
        let glyphs = &app.projection.row(0).glyphs;
        let length = row_width(app.projection.row(0));
        let cut = length > app.side + width;
        for glyph in glyphs {
            if glyph.column + glyph.width <= app.side || glyph.column >= app.side + width {
                continue;
            }
            let end = glyph.column + glyph.width - app.side;
            if glyph.column < app.side {
                for cell in 0..end {
                    let symbol = buffer[(app.viewport.x + cell as u16, app.viewport.y)].symbol();
                    assert_eq!(symbol, " ", "step {step}: half of {:?}", glyph.text);
                }
                continue;
            }
            let cell = glyph.column - app.side;
            let last = cut && end >= width;
            let symbol = buffer[(app.viewport.x + cell as u16, app.viewport.y)].symbol();
            // A tab is drawn as the blank cells it spans.
            let text = if glyph.text.trim().is_empty() {
                " "
            } else {
                glyph.text.as_str()
            };
            if end > width || last {
                // Cut by the right edge, or under the mark: not drawn.
                assert!(symbol == " " || symbol == "›", "step {step}: {symbol:?}");
            } else {
                assert_eq!(symbol, text, "step {step}, cell {cell}");
            }
        }
        assert_eq!(
            buffer[(app.viewport.right() - 1, app.viewport.y)].symbol() == "›",
            cut,
            "step {step}"
        );
        sides.insert(app.side);
        if step < steps {
            press(&mut app, KeyCode::Right, KeyModifiers::NONE);
        }
    }
    assert!(sides.len() > 10, "the view scrolled through the line");
    assert_eq!(app.document.selection().head, line.len());
    // Every click lands on a glyph boundary of the text under it.
    for cell in 0..app.viewport.width {
        let (x, y) = (app.viewport.x + cell, app.viewport.y);
        let left = MouseButton::Left;
        pointer(
            &mut app,
            MouseEventKind::Down(left),
            x,
            y,
            KeyModifiers::NONE,
        );
        pointer(&mut app, MouseEventKind::Up(left), x, y, KeyModifiers::NONE);
        let head = app.document.selection().head;
        assert!(source.is_char_boundary(head));
        let glyph = app
            .projection
            .row(0)
            .glyphs
            .iter()
            .find(|glyph| glyph.source.start == head || glyph.source.end == head);
        assert!(glyph.is_some(), "cell {cell} landed at {head}");
        app.document.set_caret(line.len()).unwrap();
        frame(&mut app, 24, 6);
    }
    assert_eq!(app.document.text(), source);
}

#[test]
fn a_line_longer_than_the_unwrapped_width_continues_on_the_next_row() {
    let long = "word ".repeat(UNWRAPPED_WIDTH);
    let source = format!("first\n{long}\nlast\n");
    let mut app = unwrapped(&source);
    frame(&mut app, 60, 12);
    // Five times the width: the line takes that many rows and a little.
    let rows = app.projection.row_count();
    assert!((7..=9).contains(&rows), "{rows} rows");
    assert_eq!(app.projection.row(1).line, Some(2));
    assert_eq!(app.projection.row(2).line, None);
    assert!(row_width(app.projection.row(1)) < UNWRAPPED_WIDTH);
    // The caret can reach the far end of a row and the end of the line.
    app.document.set_caret(6).unwrap();
    press(&mut app, KeyCode::End, KeyModifiers::NONE);
    let (_, cursor) = frame(&mut app, 60, 12);
    let width = usize::from(app.viewport.width);
    assert_eq!(app.side, row_width(app.projection.row(1)) + 1 - width);
    assert_eq!(cursor.1, app.viewport.y + 1);
    app.jump_to_source(6 + long.len());
    let (_, cursor) = frame(&mut app, 60, 12);
    let (row, column) = app.caret_position();
    assert_eq!(usize::from(cursor.0 - app.viewport.x), column - app.side);
    assert_eq!(app.projection.row(row + 1).line, Some(3));
    assert_eq!(app.document.text(), source);
}

#[test]
fn snapshots_render_unwrapped_views_as_the_terminal_shows_them() {
    let mut app = unwrapped(LONG);
    let line = LONG.lines().nth(1).unwrap();
    let start = LONG.find(line).unwrap();
    let screen = crate::simulation::snapshot(&mut app, 40, 8).unwrap();
    let rows: Vec<&str> = screen.lines().collect();
    assert_eq!(rows[1], "   1 fn short() {}");
    assert_eq!(rows[2], format!("   2 {}›", &line[..33]));
    assert_eq!(rows[3], "   3 fn tail() {}");
    assert!(screen.contains("visual row: 1 · column: 1"), "{screen}");
    app.document.set_caret(start + line.len()).unwrap();
    let screen = crate::simulation::snapshot(&mut app, 40, 8).unwrap();
    let rows: Vec<&str> = screen.lines().collect();
    assert_eq!(rows[1], "   1‹");
    assert_eq!(rows[2], format!("   2‹{}", &line[line.len() + 1 - 34..]));
    // The caret's cell in the layout, as before: its line and its column.
    assert!(
        screen.contains(&format!("visual row: 2 · column: {}", line.len() + 1)),
        "{screen}"
    );
}
