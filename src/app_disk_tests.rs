//! A clean buffer following its file on disk, through the same poll the
//! running editor makes once a second.
use super::*;
use ratatui::{Terminal, backend::TestBackend};
use std::{
    fs,
    path::Path,
    time::{Duration, SystemTime},
};

fn key(app: &mut App, code: KeyCode, modifiers: KeyModifiers) {
    app.handle_event(Event::Key(KeyEvent::new(code, modifiers)));
}
fn press(app: &mut App, code: KeyCode) {
    key(app, code, KeyModifiers::NONE);
}
fn draw(app: &mut App) -> Terminal<TestBackend> {
    let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    terminal
}
/// The row of the 100×24 screen at `y`.
fn row(app: &mut App, y: u16) -> String {
    let terminal = draw(app);
    let buffer = terminal.backend().buffer();
    let text: String = (0..100).map(|x| buffer[(x, y)].symbol()).collect();
    text.trim_end().to_owned()
}
fn status(app: &mut App) -> String {
    row(app, 23)
}
/// A file named `name` holding `source`, open and drawn once.
fn open(name: &str, source: &str) -> (tempfile::TempDir, PathBuf, App) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(name);
    fs::write(&path, source).unwrap();
    let mut app = App::open(Some(path.clone())).unwrap();
    draw(&mut app);
    (dir, path, app)
}
/// Write `text` as another program did a moment ago: whole, and settled.
fn write_settled(path: &Path, text: impl AsRef<[u8]>) {
    fs::write(path, text).unwrap();
    let file = fs::File::options().write(true).open(path).unwrap();
    file.set_modified(SystemTime::now() - Duration::from_secs(5))
        .unwrap();
}
/// The caret's row on screen.
fn caret_row(app: &App) -> usize {
    app.caret_position().0 - app.scroll
}

#[test]
fn a_clean_buffer_follows_its_file_and_says_so_briefly() {
    let (_dir, path, mut app) = open("notes.md", "one\ntwo\n");
    assert!(!app.check_external_change());
    write_settled(&path, "one\ntwo\nthree\n");
    assert!(app.check_external_change());
    assert_eq!(app.document.text(), "one\ntwo\nthree\n");
    assert!(!app.document.is_dirty());
    // A notice, not a problem: it names what happened and how to go back.
    let line = status(&mut app);
    assert!(
        line.starts_with(" Changed on disk: reloaded · Ctrl+Z restores the previous text"),
        "{line}"
    );
    assert!(!app.message_is_error && !line.contains("Esc"), "{line}");
    assert!(!app.check_external_change());
    press(&mut app, KeyCode::Right);
    assert!(status(&mut app).starts_with(" notes.md "));
    // It is the file's own baseline now: a save is a no-op, not a conflict.
    key(&mut app, KeyCode::Char('s'), KeyModifiers::CONTROL);
    assert!(!app.message_is_error);
    // One undo step back, as an edit against the new version on disk.
    key(&mut app, KeyCode::Char('z'), KeyModifiers::CONTROL);
    assert_eq!(app.document.text(), "one\ntwo\n");
    assert!(app.document.is_dirty());
    // With that edit unsaved, a further change is reported and not loaded.
    write_settled(&path, "elsewhere\n");
    assert!(app.check_external_change());
    assert!(app.message_is_error);
    assert!(
        app.message
            .contains("Disk file changed; your text is intact")
    );
    assert_eq!(app.document.text(), "one\ntwo\n");
    assert!(!app.check_external_change());
    // Redone, the buffer is clean again and catches up; the report gives
    // way to the notice.
    key(&mut app, KeyCode::Char('y'), KeyModifiers::CONTROL);
    assert!(app.check_external_change());
    assert_eq!(app.document.text(), "elsewhere\n");
    assert!(!app.message_is_error && app.message.contains("reloaded"));
    key(&mut app, KeyCode::Char('z'), KeyModifiers::CONTROL);
    assert_eq!(app.document.text(), "one\ntwo\nthree\n");
}

#[test]
fn the_readers_place_survives_a_reload() {
    let lines = |range: std::ops::Range<usize>| -> String {
        range.map(|line| format!("line {line}\n")).collect()
    };
    let source = lines(0..200);
    let (_dir, path, mut app) = open("log.txt", &source);
    app.go_to(121, 3);
    draw(&mut app);
    let caret = app.document.selection().head;
    assert_eq!(&source[app.document.line_range(caret)], "line 120");
    let (top, on_screen) = (app.top, caret_row(&app));
    let first_row = row(&mut app, 1);
    assert!(top.line > 100 && on_screen > 0, "{top:?}");

    // A log that grows at its end: nothing on screen moves.
    write_settled(&path, format!("{source}{}", lines(200..230)));
    assert!(app.check_external_change());
    assert_eq!(app.document.line_count(), 231);
    assert_eq!(row(&mut app, 1), first_row);
    assert_eq!(app.document.selection().head, caret);
    assert_eq!((app.top, caret_row(&app)), (top, on_screen));

    // Lines added above: the same lines stay on the same rows, and the
    // caret stays in the text it was in.
    let source = format!("{source}{}", lines(200..230));
    let above = "inserted a\ninserted b\ninserted c\n";
    write_settled(&path, format!("{above}{source}"));
    assert!(app.check_external_change());
    draw(&mut app);
    let text = app.document.text().to_owned();
    let head = app.document.selection().head;
    assert_eq!(head, caret + above.len());
    assert_eq!(&text[app.document.line_range(head)], "line 120");
    assert_eq!(app.top.line, top.line + 3);
    assert_eq!(caret_row(&app), on_screen);
    // The gutter counts three more lines; the text on the row is the same.
    let numbered = |row: &str| {
        let (number, text) = row.trim_start().split_once(' ').unwrap();
        (number.parse::<usize>().unwrap(), text.to_owned())
    };
    let (was, now) = (numbered(&first_row), numbered(&row(&mut app, 1)));
    assert_eq!((was.0 + 3, &was.1), (now.0, &now.1));
    assert!(was.1.starts_with("line 1"), "{was:?}");

    // Lines removed above: back again.
    write_settled(&path, &source);
    assert!(app.check_external_change());
    assert_eq!(app.document.selection().head, caret);
    assert_eq!(row(&mut app, 1), first_row);
    assert_eq!((app.top, caret_row(&app)), (top, on_screen));

    // The caret's own line rewritten: it stays on that line.
    write_settled(
        &path,
        source.replace("line 120\n", "LINE ONE-TWENTY, REWRITTEN\n"),
    );
    assert!(app.check_external_change());
    let head = app.document.selection().head;
    assert_eq!(app.document.line_index(head), 120);
    assert_eq!(row(&mut app, 1), first_row);
    assert_eq!(caret_row(&app), on_screen);

    // A view wheeled away from the caret stays where it was put.
    for _ in 0..4 {
        app.handle_event(Event::Mouse(MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: 20,
            row: 5,
            modifiers: KeyModifiers::NONE,
        }));
    }
    let wheeled = row(&mut app, 1);
    assert_ne!(wheeled, first_row);
    write_settled(&path, format!("{source}appended\n"));
    assert!(app.check_external_change());
    assert_eq!(row(&mut app, 1), wheeled);
    assert_eq!(app.document.line_index(app.document.selection().head), 120);
    // Changed above and below the view at once, the view and the caret
    // still stay with their lines.
    press(&mut app, KeyCode::Up);
    let wheeled = row(&mut app, 1);
    let on_screen = caret_row(&app);
    let both = format!("a new first line\n{source}")
        .replace("line 190\n", "line one-ninety\n")
        .replace("line 3\n", "");
    write_settled(&path, &both);
    assert!(app.check_external_change());
    let head = app.document.selection().head;
    assert_eq!(&both[app.document.line_range(head)], "line 119");
    assert_eq!(numbered(&row(&mut app, 1)).1, numbered(&wheeled).1);
    assert_eq!(caret_row(&app), on_screen);
    // Every reload was one undo step.
    for _ in 0..6 {
        key(&mut app, KeyCode::Char('z'), KeyModifiers::CONTROL);
    }
    assert_eq!(app.document.text(), lines(0..200));
    assert!(!app.document.can_undo());
}

#[test]
fn find_stays_open_across_a_reload_and_dialogs_and_drags_put_it_off() {
    let (_dir, path, mut app) = open("pets.txt", "cat\ndog\ncat\n");
    key(&mut app, KeyCode::Char('f'), KeyModifiers::CONTROL);
    app.handle_event(Event::Paste("cat".into()));
    draw(&mut app);
    assert_eq!(app.search.matches.len(), 2);
    write_settled(&path, "cat\ndog\ncat\ncat\n");
    assert!(app.check_external_change());
    assert!(matches!(app.overlay, Overlay::Search { .. }));
    assert_eq!(app.search.query.text(), "cat");
    draw(&mut app);
    assert_eq!(app.search.matches.len(), 3);
    press(&mut app, KeyCode::Esc);
    // Help and Save As are in front: the reload waits for them to close.
    for open in [KeyCode::F(1), KeyCode::F(4)] {
        press(&mut app, open);
        assert!(app.has_modal());
        let before = app.document.text().to_owned();
        write_settled(&path, format!("{before}more\n"));
        assert!(!app.check_external_change());
        assert!(!app.check_external_change());
        assert_eq!(app.document.text(), before);
        assert!(app.message.is_empty());
        press(&mut app, KeyCode::Esc);
        assert!(app.check_external_change());
        assert_eq!(app.document.text(), format!("{before}more\n"));
    }
    // So does a selection being dragged.
    draw(&mut app);
    let (x, y) = (app.viewport.x + 1, app.viewport.y);
    let mouse = |kind, column| {
        Event::Mouse(MouseEvent {
            kind,
            column,
            row: y,
            modifiers: KeyModifiers::NONE,
        })
    };
    app.handle_event(mouse(MouseEventKind::Down(MouseButton::Left), x));
    app.handle_event(mouse(MouseEventKind::Drag(MouseButton::Left), x + 2));
    write_settled(&path, "dragged over\n");
    assert!(!app.check_external_change());
    app.handle_event(mouse(MouseEventKind::Up(MouseButton::Left), x + 2));
    assert!(app.check_external_change());
    assert_eq!(app.document.text(), "dragged over\n");
}

#[test]
fn a_file_still_being_written_is_left_for_the_next_look() {
    let (_dir, path, mut app) = open("notes.md", "the whole text\n");
    // The file is emptied, and for a moment that is all there is.
    app.settle = Duration::from_secs(3600);
    fs::write(&path, "").unwrap();
    for _ in 0..3 {
        assert!(!app.check_external_change());
    }
    assert_eq!(app.document.text(), "the whole text\n");
    assert!(app.message.is_empty() && !app.document.can_undo());
    // Left alone long enough, whatever is there is the new version.
    fs::write(&path, "the whole new text\n").unwrap();
    let file = fs::File::options().write(true).open(&path).unwrap();
    file.set_modified(SystemTime::now() - Duration::from_secs(7200))
        .unwrap();
    assert!(app.check_external_change());
    assert_eq!(app.document.text(), "the whole new text\n");
    // The running editor waits a tenth of a second.
    assert_eq!(SETTLE, Duration::from_millis(100));
    let (_dir, _path, app) = open("other.md", "");
    assert_eq!(app.settle, SETTLE);
}

#[test]
fn a_version_that_cannot_be_loaded_is_reported_once_and_tried_again_when_it_changes() {
    let (_dir, path, mut app) = open("notes.md", "my text\n");
    for bytes in [vec![0xff, 0xfe, b'x'], b"binary\0data".to_vec()] {
        write_settled(&path, bytes);
        assert!(app.check_external_change());
        assert!(app.message_is_error);
        assert_eq!(
            app.message,
            "On disk the file is no longer text pica can open. Your text is intact"
        );
        assert_eq!(app.document.text(), "my text\n");
        assert!(!app.document.is_dirty() && !app.document.can_undo());
        // Said once: not again every second, and not again once dismissed.
        assert!(!app.check_external_change());
        press(&mut app, KeyCode::Esc);
        assert!(app.message.is_empty());
        assert!(!app.check_external_change());
        assert!(app.message.is_empty());
    }
    // The next version is text again and is loaded.
    write_settled(&path, "text once again\n");
    assert!(app.check_external_change());
    assert_eq!(app.document.text(), "text once again\n");
    assert!(!app.message_is_error && app.message.contains("reloaded"));
    // A problem while the report is showing is replaced by the good news;
    // a problem of another kind is not.
    write_settled(&path, [0xff]);
    assert!(app.check_external_change());
    write_settled(&path, "fine\n");
    assert!(app.check_external_change());
    assert!(!app.message_is_error && app.message.contains("reloaded"));
    app.set_message("Could not save preferences");
    write_settled(&path, "finer\n");
    assert!(app.check_external_change());
    assert_eq!(app.document.text(), "finer\n");
    assert_eq!(app.message, "Could not save preferences");
    press(&mut app, KeyCode::Esc);
    // A file that goes away is reported, as before; when it comes back
    // changed, the clean buffer takes it.
    fs::remove_file(&path).unwrap();
    assert!(app.check_external_change());
    assert!(app.message.contains("missing") && app.message_is_error);
    assert_eq!(app.document.text(), "finer\n");
    write_settled(&path, "back\n");
    assert!(app.check_external_change());
    assert_eq!(app.document.text(), "back\n");
    assert!(!app.message_is_error);
    // Undo history that cannot hold the old text keeps the old text.
    app.document.set_history_limits(pica::HistoryLimits {
        max_entries: 0,
        max_bytes: 0,
    });
    write_settled(&path, "unretainable\n");
    assert!(app.check_external_change());
    assert!(app.message.contains("history limits") && app.message_is_error);
    assert_eq!(app.document.text(), "back\n");
    assert!(!app.check_external_change());
}
