//! Workspace keys: micro's per-tab Ctrl+Q, the palette and tab switching.
use super::*;
use crossterm::event::KeyEvent;
use palette::Entry;
use ratatui::{Terminal, backend::TestBackend};

fn key(app: &mut Workspace, code: KeyCode, modifiers: KeyModifiers) {
    app.handle_event(Event::Key(KeyEvent::new(code, modifiers)));
}
fn ctrl(app: &mut Workspace, c: char) {
    key(app, KeyCode::Char(c), KeyModifiers::CONTROL);
}
fn plain(app: &mut Workspace, c: char) {
    key(app, KeyCode::Char(c), KeyModifiers::NONE);
}
fn draw(app: &mut Workspace) {
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
}

#[test]
fn ctrl_q_closes_the_current_tab_and_quits_after_the_last() {
    let mut app = Workspace::open(None).unwrap();
    ctrl(&mut app, 't');
    app.handle_event(Event::Paste("draft".into()));
    // A dirty tab asks first; discarding closes only that tab.
    ctrl(&mut app, 'q');
    assert!(app.pending.is_some());
    draw(&mut app);
    plain(&mut app, 'n');
    assert_eq!(app.tabs.len(), 1);
    assert!(!app.should_exit);
    // The last tab, when clean, closes at once and quits.
    ctrl(&mut app, 'q');
    assert!(app.should_exit);
}

#[test]
fn ctrl_q_on_a_dirty_last_tab_can_cancel_discard_or_save() {
    let mut app = Workspace::open(None).unwrap();
    app.handle_event(Event::Paste("keep".into()));
    ctrl(&mut app, 'q');
    draw(&mut app);
    key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    assert!(!app.should_exit);
    assert_eq!(app.editor().document.text(), "keep");
    ctrl(&mut app, 'q');
    draw(&mut app);
    plain(&mut app, 'n');
    assert!(app.should_exit);

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("note.txt");
    std::fs::write(&path, "old").unwrap();
    let mut app = Workspace::open(Some(path.clone())).unwrap();
    ctrl(&mut app, 't');
    // Saving then closing the file's tab leaves the untitled tab open.
    key(&mut app, KeyCode::Char(','), KeyModifiers::ALT);
    app.handle_event(Event::Paste("new ".into()));
    ctrl(&mut app, 'q');
    draw(&mut app);
    plain(&mut app, 'y');
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "new old");
    assert_eq!(app.tabs.len(), 1);
    assert!(!app.should_exit);
    assert!(app.editor().path().is_none());
    ctrl(&mut app, 'q');
    assert!(app.should_exit);
}

#[test]
fn quit_all_stays_in_the_palette_and_checks_every_dirty_tab() {
    let mut app = Workspace::open(None).unwrap();
    app.handle_event(Event::Paste("first".into()));
    ctrl(&mut app, 't');
    app.handle_event(Event::Paste("second".into()));
    ctrl(&mut app, 'e');
    app.handle_event(Event::Paste("quit".into()));
    draw(&mut app);
    key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(app.pending.as_ref().unwrap().intent, Intent::Quit);
    assert_eq!(app.active, 0);
    draw(&mut app);
    plain(&mut app, 'n');
    draw(&mut app);
    plain(&mut app, 'n');
    assert!(app.should_exit);
}

#[test]
fn ctrl_e_opens_commands_and_a_bare_number_goes_to_that_line() {
    let mut app = Workspace::open(None).unwrap();
    app.handle_event(Event::Paste("one\ntwo\nthree\n".into()));
    ctrl(&mut app, 'e');
    let palette = app.palette.as_ref().unwrap();
    assert!(!palette.line_mode);
    app.handle_event(Event::Paste("3".into()));
    assert_eq!(
        app.palette.as_ref().unwrap().entries()[0],
        Entry::Line(3, 1)
    );
    draw(&mut app);
    key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    assert!(app.palette.is_none());
    assert_eq!(app.editor().document.selection().head, 8);
    ctrl(&mut app, 'e');
    app.handle_event(Event::Paste("2:3".into()));
    draw(&mut app);
    key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(app.editor().document.selection().head, 6);
    // Ctrl+L asks for a line, and accepts a column too.
    ctrl(&mut app, 'l');
    assert!(app.palette.as_ref().unwrap().line_mode);
    app.handle_event(Event::Paste("1:3".into()));
    draw(&mut app);
    key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(app.editor().document.selection().head, 2);
    // Ctrl+Q leaves the palette, as at micro's command prompt.
    ctrl(&mut app, 'e');
    ctrl(&mut app, 'q');
    assert!(app.palette.is_none());
    assert!(app.pending.is_none());
    assert!(!app.should_exit);
    assert_eq!(app.editor().document.text(), "one\ntwo\nthree\n");
}

#[test]
fn line_entries_parse_lines_and_columns_strictly() {
    for (text, expected) in [
        ("7", Some((7, 1))),
        (" 12 ", Some((12, 1))),
        ("3:4", Some((3, 4))),
        ("0", None),
        ("3:0", None),
        ("-1", None),
        ("+3", None),
        ("3:", None),
        ("save", None),
        ("", None),
    ] {
        assert_eq!(palette::parse_line(text), expected, "{text:?}");
    }
}

#[test]
fn old_keys_no_longer_open_workspace_commands() {
    let mut app = Workspace::open(None).unwrap();
    app.handle_event(Event::Paste("text".into()));
    // Ctrl+P is Find previous; with no query it opens Find, not commands.
    ctrl(&mut app, 'p');
    assert!(app.palette.is_none());
    key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    // Ctrl+G is help, not Go to line.
    ctrl(&mut app, 'g');
    assert!(app.palette.is_none());
    assert!(app.editor().has_modal());
    key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    // Ctrl+N is Find next, and Ctrl+W is Find; neither touches tabs.
    ctrl(&mut app, 'n');
    key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    ctrl(&mut app, 'w');
    key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    assert_eq!(app.tabs.len(), 1);
    assert!(app.pending.is_none());
    // Function keys stay as silent aliases.
    key(&mut app, KeyCode::F(2), KeyModifiers::NONE);
    assert!(app.palette.is_some());
}

#[test]
fn ctrl_t_and_alt_comma_period_manage_tabs() {
    let mut app = Workspace::open(None).unwrap();
    app.handle_event(Event::Paste("first".into()));
    ctrl(&mut app, 't');
    assert_eq!(app.tabs.len(), 2);
    assert_eq!(app.active, 1);
    assert!(app.editor().is_markdown());
    assert!(app.editor().document.text().is_empty());
    key(&mut app, KeyCode::Char(','), KeyModifiers::ALT);
    assert_eq!(app.active, 0);
    key(&mut app, KeyCode::Char(','), KeyModifiers::ALT);
    assert_eq!(app.active, 1);
    key(&mut app, KeyCode::Char('.'), KeyModifiers::ALT);
    assert_eq!(app.active, 0);
    assert_eq!(app.editor().document.text(), "first");
}

#[test]
fn palette_hints_name_primary_keys_and_never_function_keys() {
    for markdown in [false, true] {
        for command in Command::ALL {
            let hint = command.shortcut(markdown);
            assert!(
                !hint.split(['+', ' ']).any(|part| part.len() > 1
                    && part.starts_with('F')
                    && part[1..].bytes().all(|b| b.is_ascii_digit())),
                "{command:?}: {hint}"
            );
        }
        assert_eq!(Command::View.shortcut(markdown).is_empty(), !markdown);
        assert_eq!(Command::Ruler.shortcut(markdown).is_empty(), markdown);
    }
    for (command, hint) in [
        (Command::New, "Ctrl+T"),
        (Command::Close, "Ctrl+Q"),
        (Command::GoToLine, "Ctrl+L"),
        (Command::Help, "Ctrl+G"),
        (Command::FindNext, "Ctrl+N"),
        (Command::FindPrevious, "Ctrl+P"),
        (Command::CutLine, "Ctrl+K"),
        (Command::Duplicate, "Ctrl+D"),
        (Command::Comment, "Ctrl+/"),
        (Command::Task, "Alt+T"),
        (Command::PreviousTab, "Alt+,"),
        (Command::NextTab, "Alt+."),
        (Command::Quit, ""),
    ] {
        assert_eq!(command.shortcut(true), hint, "{command:?}");
    }
}

#[test]
fn palette_runs_the_new_editing_commands() {
    let mut app = Workspace::open(None).unwrap();
    app.handle_event(Event::Paste("alpha\nbeta".into()));
    let _ = app.editor_mut().document.set_caret(0);
    for (query, expected) in [
        ("duplicate line or", "alpha\nalpha\nbeta"),
        ("toggle comment", "alpha\n<!-- alpha -->\nbeta"),
        ("cut line", "alpha\nbeta"),
    ] {
        ctrl(&mut app, 'e');
        app.handle_event(Event::Paste(query.into()));
        draw(&mut app);
        key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(app.editor().document.text(), expected, "{query}");
    }
    ctrl(&mut app, 'e');
    app.handle_event(Event::Paste("line numbers".into()));
    draw(&mut app);
    key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    assert!(!app.editor().ruler);
    ctrl(&mut app, 'e');
    app.handle_event(Event::Paste("find and replace".into()));
    draw(&mut app);
    key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    assert!(app.editor().workspace_commands_allowed());
    assert!(!app.editor().has_modal());
}

#[test]
fn launch_opens_arguments_in_order_and_shares_duplicate_paths() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("first.md");
    let second = dir.path().join("second.txt");
    std::fs::write(&first, "a\nb\nc\n").unwrap();
    std::fs::write(&second, "x\r\ny\r\n").unwrap();
    let app = Workspace::launch(vec![
        Launch::Path(first.clone(), None),
        Launch::Piped("piped # text\n".into(), Some((1, 3))),
        Launch::Path(second.clone(), Some((2, 2))),
        Launch::Path(dir.path().join("./first.md"), Some((3, 1))),
        Launch::Piped(String::new(), None),
    ])
    .unwrap();
    assert_eq!(app.tabs.len(), 4);
    assert_eq!(app.active, 0);
    assert_eq!(app.tabs[0].editor.document.selection().head, 4);
    assert_eq!(app.tabs[1].editor.document.selection().head, 2);
    assert!(!app.tabs[1].editor.is_markdown());
    assert!(app.tabs[1].editor.document.is_dirty());
    assert!(app.tabs[1].editor.path().is_none());
    assert_eq!(app.name(1), "Untitled 2");
    assert_eq!(app.tabs[2].editor.document.selection().head, 4);
    assert!(!app.tabs[3].editor.document.is_dirty());
    assert!(app.browser.is_none());
    for tab in &app.tabs {
        assert_eq!(tab.editor.document.revision(), 0);
    }
    let app = Workspace::launch(vec![Launch::Path(dir.path().to_owned(), None)]).unwrap();
    assert!(app.browser.is_some());
    assert_eq!(app.tabs.len(), 1);
    assert!(app.editor().is_markdown());
    let bad = dir.path().join("bad.md");
    std::fs::write(&bad, b"\xff").unwrap();
    let error = Workspace::launch(vec![
        Launch::Path(dir.path().join("x.md"), None),
        Launch::Path(bad, None),
    ])
    .err()
    .unwrap();
    assert!(error.to_string().contains("bad.md: "), "{error}");
}
