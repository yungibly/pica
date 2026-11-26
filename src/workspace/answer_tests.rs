//! A key that answers a question, held down or repeating, answers only that
//! question. Each test gives every key press its moment.
use super::*;
use ratatui::{Terminal, backend::TestBackend};
use std::fs;

fn draw(app: &mut Workspace) -> String {
    let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let buffer = terminal.backend().buffer();
    (0..24)
        .map(|y| {
            let row: String = (0..100).map(|x| buffer[(x, y)].symbol()).collect();
            format!("{}\n", row.trim_end())
        })
        .collect()
}

/// Press `c` at `start` plus `ms` milliseconds, and draw, as the running
/// editor does after every event.
fn press(app: &mut Workspace, c: char, start: Instant, ms: u64) {
    let event = Event::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
    app.handle_event_at(event, || start + Duration::from_millis(ms));
    draw(app);
}

/// Hold `c` down from `start` plus `ms`: the press, a first repeat half a
/// second later, then one every 30 ms for two seconds. Returns the moment
/// of the last repeat.
fn hold(app: &mut Workspace, c: char, start: Instant, ms: u64) -> u64 {
    press(app, c, start, ms);
    let mut at = ms + 500;
    while at < ms + 2500 {
        press(app, c, start, at);
        at += 30;
    }
    at - 30
}

/// Three unsaved tabs, and "Quit pica (all tabs)" asking about the first.
fn quitting() -> Workspace {
    let mut app = Workspace::open(None).unwrap();
    app.handle_event(Event::Paste("first".into()));
    for text in ["second", "third"] {
        app.new_tab();
        app.handle_event(Event::Paste(text.into()));
    }
    app.run_command(Command::Quit);
    assert!(draw(&mut app).contains("Save changes before quitting?"));
    assert_eq!(app.active, 0);
    app
}

fn unsaved(app: &Workspace) -> Vec<&str> {
    app.tabs
        .iter()
        .filter(|tab| tab.editor.document.is_dirty())
        .map(|tab| tab.editor.document.text())
        .collect()
}

#[test]
fn a_held_n_discards_one_tab_and_asks_about_the_next() {
    let mut app = quitting();
    let start = Instant::now();
    // N held for two and a half seconds: one answer.
    let released = hold(&mut app, 'n', start, 0);
    assert_eq!(app.active, 1, "the question moved on to the second tab");
    assert!(app.pending.is_some() && !app.should_exit);
    assert_eq!(unsaved(&app), ["first", "second", "third"]);
    assert!(draw(&mut app).contains("Save changes before quitting?"));
    // Let go, and pressed again on purpose: the next answer. A press too
    // soon after it is still the same press.
    press(&mut app, 'n', start, released + 1000);
    assert_eq!(app.active, 2);
    assert!(!app.should_exit);
    press(&mut app, 'n', start, released + 1300);
    assert!(
        !app.should_exit,
        "a second press 300 ms on is not an answer"
    );
    press(&mut app, 'n', start, released + 2100);
    assert!(app.should_exit);
}

#[test]
fn presses_that_queued_behind_a_slow_answer_are_still_the_held_key() {
    // Y at the first tab's question, held, and the save it starts takes a
    // second and a half: the answer is read at 0 and done at 1500. The
    // repeats that queued meanwhile then arrive a millisecond apart, and
    // the key goes on repeating.
    let dir = tempfile::tempdir().unwrap();
    let mut app = Workspace::open(Some(dir.path().join("one.md"))).unwrap();
    app.handle_event(Event::Paste("first".into()));
    app.open_path(dir.path().join("two.md")).unwrap();
    app.handle_event(Event::Paste("second".into()));
    app.run_command(Command::Quit);
    assert!(draw(&mut app).contains("Save changes before quitting?"));
    let start = Instant::now();
    let mut times = [0, 1500]
        .map(|ms| start + Duration::from_millis(ms))
        .into_iter();
    let y = Event::Key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
    app.handle_event_at(y, || times.next().unwrap());
    draw(&mut app);
    for queued in 1..=34 {
        press(&mut app, 'y', start, 1500 + queued);
    }
    let mut at = 1560;
    while at < 4000 {
        press(&mut app, 'y', start, at);
        at += 30;
    }
    // One answer: the first file was saved, and the second is still asked
    // about.
    assert_eq!(
        fs::read_to_string(dir.path().join("one.md")).unwrap(),
        "first"
    );
    assert!(!dir.path().join("two.md").exists());
    assert!(app.pending.is_some() && !app.should_exit);
    assert_eq!(unsaved(&app), ["second"]);
    // Let go, then pressed again: the second file is saved and pica quits.
    press(&mut app, 'y', start, at + 400);
    assert_eq!(
        fs::read_to_string(dir.path().join("two.md")).unwrap(),
        "second"
    );
    assert!(app.should_exit);
}

#[test]
fn a_held_n_never_reaches_the_end_of_the_tabs_however_long_it_is_held() {
    let mut app = quitting();
    let start = Instant::now();
    // The same on a keyboard that repeats at once and fast, or slowly.
    for (first, every) in [(250, 20), (660, 40), (120, 150)] {
        let mut app = quitting();
        press(&mut app, 'n', start, 0);
        let mut at = first;
        while at < 6000 {
            press(&mut app, 'n', start, at);
            at += every;
        }
        assert_eq!(
            app.active, 1,
            "first repeat at {first} ms, then every {every}"
        );
        assert!(!app.should_exit);
    }
    // A held Y saves nothing further either: the first tab has no file,
    // so Y opens Save As, and the repeats do not type into its path.
    press(&mut app, 'y', start, 0);
    assert!(app.editor().saving_as());
    let path = app.editor().pending_save_path();
    let mut at = 500;
    while at < 2500 {
        press(&mut app, 'y', start, at);
        at += 30;
    }
    assert!(app.editor().saving_as());
    assert_eq!(app.editor().pending_save_path(), path);
    assert_eq!(unsaved(&app), ["first", "second", "third"]);
}

#[test]
fn a_held_y_replaces_the_file_and_types_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let taken = dir.path().join("taken.md");
    fs::write(&taken, "theirs").unwrap();
    let mut app = Workspace::open(None).unwrap();
    app.handle_event(Event::Paste("mine".into()));
    app.handle_event(Event::Key(KeyEvent::new(KeyCode::F(4), KeyModifiers::NONE)));
    app.handle_event(Event::Paste(taken.display().to_string()));
    app.handle_event(Event::Key(KeyEvent::new(
        KeyCode::Enter,
        KeyModifiers::NONE,
    )));
    assert!(draw(&mut app).contains("Replace file?"));
    let start = Instant::now();
    let released = hold(&mut app, 'y', start, 0);
    assert_eq!(fs::read_to_string(&taken).unwrap(), "mine");
    assert_eq!(app.editor().document.text(), "mine");
    assert!(!app.editor().document.is_dirty() && !app.editor().saving_as());
    // After a pause the key types again.
    press(&mut app, 'y', start, released + 250);
    assert_eq!(app.editor().document.text(), "miney");
    // The other answer: N goes back to the path and types nothing there.
    let other = dir.path().join("other.md");
    fs::write(&other, "other").unwrap();
    app.handle_event(Event::Key(KeyEvent::new(KeyCode::F(4), KeyModifiers::NONE)));
    app.handle_event(Event::Paste(other.display().to_string()));
    app.handle_event(Event::Key(KeyEvent::new(
        KeyCode::Enter,
        KeyModifiers::NONE,
    )));
    assert!(draw(&mut app).contains("Replace file?"));
    let start = Instant::now();
    hold(&mut app, 'n', start, 0);
    assert!(app.editor().saving_as());
    assert_eq!(app.editor().pending_save_path(), Some(other.clone()));
    assert_eq!(fs::read_to_string(&other).unwrap(), "other");
}

#[test]
fn a_held_y_creates_the_folder_and_types_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("new").join("notes.md");
    let mut app = Workspace::open(None).unwrap();
    app.handle_event(Event::Paste("mine".into()));
    app.handle_event(Event::Key(KeyEvent::new(KeyCode::F(4), KeyModifiers::NONE)));
    app.handle_event(Event::Paste(target.display().to_string()));
    app.handle_event(Event::Key(KeyEvent::new(
        KeyCode::Enter,
        KeyModifiers::NONE,
    )));
    assert!(draw(&mut app).contains("Create folder?"));
    hold(&mut app, 'y', Instant::now(), 0);
    assert_eq!(fs::read_to_string(&target).unwrap(), "mine");
    assert_eq!(app.editor().document.text(), "mine");
    assert!(!app.editor().document.is_dirty() && !app.editor().saving_as());
}

#[test]
fn a_held_answer_to_reload_types_nothing_into_the_reloaded_text() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("notes.md");
    fs::write(&file, "base").unwrap();
    let mut app = Workspace::open(Some(file.clone())).unwrap();
    app.handle_event(Event::Paste("mine ".into()));
    fs::write(&file, "disk").unwrap();
    for (answer, text) in [('n', "mine base"), ('y', "disk")] {
        app.run_command(Command::Reload);
        assert!(draw(&mut app).contains("Replace local edits with disk text?"));
        hold(&mut app, answer, Instant::now(), 0);
        assert_eq!(app.editor().document.text(), text, "{answer}");
    }
}

#[test]
fn another_key_or_an_unanswered_press_does_not_hold_anything_back() {
    // A different key ends the hold: N, then Y at once, are two answers.
    let mut app = quitting();
    let start = Instant::now();
    press(&mut app, 'n', start, 0);
    assert_eq!(app.active, 1);
    press(&mut app, 'x', start, 40);
    press(&mut app, 'n', start, 80);
    assert_eq!(app.active, 2, "N after another key is a new press");
    // Shift does not make it another key.
    let shifted = Event::Key(KeyEvent::new(KeyCode::Char('N'), KeyModifiers::SHIFT));
    app.handle_event_at(shifted, || start + Duration::from_millis(120));
    assert!(!app.should_exit);
    // Ctrl does: Ctrl+N right after N is a key of its own.
    let chord = Event::Key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::CONTROL));
    app.handle_event_at(chord, || start + Duration::from_millis(160));
    assert!(app.answered.is_none(), "a chord ends the hold");

    // A press that answered nothing starts no hold: in a window too small
    // to show the question it is ignored, and the next press counts.
    let mut app = quitting();
    let start = Instant::now();
    app.handle_event(Event::Resize(30, 6));
    press_small(&mut app, 'n', start, 0);
    assert_eq!(app.active, 0);
    app.handle_event(Event::Resize(100, 24));
    press(&mut app, 'n', start, 50);
    assert_eq!(app.active, 1);

    // Typing the same letter twice in a document is never held back.
    let mut app = Workspace::open(None).unwrap();
    let start = Instant::now();
    for ms in [0, 10, 20, 30] {
        press(&mut app, 'n', start, ms);
    }
    assert_eq!(app.editor().document.text(), "nnnn");
    // Nor is a key a command sends, right after the same key answered.
    let mut app = quitting();
    press(&mut app, 'n', Instant::now(), 0);
    app.send(KeyCode::Char('n'), KeyModifiers::NONE);
    assert_eq!(app.active, 2);
}

fn press_small(app: &mut Workspace, c: char, start: Instant, ms: u64) {
    let event = Event::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
    app.handle_event_at(event, || start + Duration::from_millis(ms));
    let mut terminal = Terminal::new(TestBackend::new(30, 6)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
}

#[test]
fn a_repeat_the_terminal_names_as_one_never_answers() {
    let mut app = quitting();
    let start = Instant::now();
    // Long after any hold, a reported repeat is still not a press.
    for ms in [0, 2000, 4000] {
        let repeat = Event::Key(KeyEvent::new_with_kind(
            KeyCode::Char('n'),
            KeyModifiers::NONE,
            KeyEventKind::Repeat,
        ));
        app.handle_event_at(repeat, || start + Duration::from_millis(ms));
        assert_eq!(app.active, 0);
        assert_eq!(unsaved(&app), ["first", "second", "third"]);
    }
    press(&mut app, 'n', start, 6000);
    assert_eq!(app.active, 1);
    // And after an answer, reported repeats are dropped like any other.
    for ms in [6500, 6530, 6560, 9000] {
        let repeat = Event::Key(KeyEvent::new_with_kind(
            KeyCode::Char('n'),
            KeyModifiers::NONE,
            KeyEventKind::Repeat,
        ));
        app.handle_event_at(repeat, || start + Duration::from_millis(ms));
    }
    assert_eq!(app.active, 1);
    assert_eq!(unsaved(&app), ["first", "second", "third"]);
}
