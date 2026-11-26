use super::*;
use crate::theme::{Theme, current_theme, set_theme};
use ratatui::{Terminal, backend::TestBackend};
use std::{fs, time::Duration};

fn key(app: &mut Workspace, code: KeyCode, modifiers: KeyModifiers) {
    app.handle_event(Event::Key(KeyEvent::new(code, modifiers)));
}
fn draw(app: &mut Workspace, width: u16, height: u16) {
    Terminal::new(TestBackend::new(width, height))
        .unwrap()
        .draw(|frame| app.draw(frame))
        .unwrap();
}
fn checkpoint(app: &mut Workspace) {
    app.last_tick = Instant::now() - Duration::from_secs(2);
    app.tick();
}
fn configure(app: &mut Workspace, dir: &Path) {
    app.enable_state(&dir.join("config"), &dir.join("state"), false);
}

#[test]
fn theme_preview_cancel_accept_and_restart_preserve_document() {
    let original = current_theme();
    set_theme(Theme::Dark);
    let dir = tempfile::tempdir().unwrap();
    let mut app = Workspace::open(None).unwrap();
    configure(&mut app, dir.path());
    app.editor_mut().document.insert("# source\n");
    let revision = app.editor().document.revision();
    app.open_themes();
    app.handle_event(Event::Paste("Sage Light".into()));
    assert_eq!(current_theme(), Theme::Light);
    assert_eq!(app.editor().document.revision(), revision);
    key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    assert_eq!(current_theme(), Theme::Dark);
    assert!(!dir.path().join("config/settings.conf").exists());
    app.open_themes();
    app.handle_event(Event::Paste("Catppuccin Mocha".into()));
    let selected = current_theme();
    draw(&mut app, 80, 24);
    key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    assert!(app.choice.is_none());
    assert_ne!(selected, Theme::Dark);
    assert_eq!(app.editor().document.text(), "# source\n");
    let mut restarted = Workspace::open(None).unwrap();
    set_theme(Theme::Dark);
    configure(&mut restarted, dir.path());
    assert_eq!(current_theme(), selected);
    set_theme(Theme::Light);
    let mut explicit = Workspace::open(None).unwrap();
    explicit.enable_state(&dir.path().join("config"), &dir.path().join("state"), true);
    assert_eq!(current_theme(), Theme::Light);
    app.finish_state().unwrap();
    restarted.finish_state().unwrap();
    explicit.finish_state().unwrap();
    set_theme(original);
}

#[test]
fn chosen_indentation_applies_at_once_survives_restart_and_can_follow_files_again() {
    use pica::editing::Indent;
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("tool.py");
    fs::write(&file, "def f():\n    x = 1\n").unwrap();
    let mut app = Workspace::open(Some(file.clone())).unwrap();
    configure(&mut app, dir.path());
    assert_eq!(app.editor().indent(), Indent::Spaces(4));
    // The picker opens on the current choice and names this file's own style.
    app.run_command(Command::Indentation);
    draw(&mut app, 80, 24);
    key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    assert!(!dir.path().join("config/settings.conf").exists());
    app.run_command(Command::Indentation);
    draw(&mut app, 80, 24);
    key(&mut app, KeyCode::Down, KeyModifiers::NONE);
    draw(&mut app, 80, 24);
    key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    assert!(app.choice.is_none());
    assert_eq!(app.editor().indent(), Indent::Tabs);
    key(&mut app, KeyCode::Tab, KeyModifiers::NONE);
    assert!(app.editor().document.text().starts_with("\tdef"));
    let saved = fs::read_to_string(dir.path().join("config/settings.conf")).unwrap();
    assert!(saved.contains("indent=tabs\n"), "{saved}");
    app.finish_state().unwrap();

    crate::app::set_indent_preference(None);
    let mut restarted = Workspace::open(Some(file)).unwrap();
    configure(&mut restarted, dir.path());
    assert_eq!(restarted.editor().indent(), Indent::Tabs);
    restarted.run_command(Command::Indentation);
    draw(&mut restarted, 80, 24);
    key(&mut restarted, KeyCode::Up, KeyModifiers::NONE);
    draw(&mut restarted, 80, 24);
    key(&mut restarted, KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(restarted.editor().indent(), Indent::Spaces(4));
    let saved = fs::read_to_string(dir.path().join("config/settings.conf")).unwrap();
    assert!(saved.contains("indent=auto\n"), "{saved}");
    restarted.finish_state().unwrap();
    crate::app::set_indent_preference(None);
}

fn screen(app: &mut Workspace, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let buffer = terminal.backend().buffer();
    (0..height)
        .map(|y| {
            (0..width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
                + "\n"
        })
        .collect()
}

#[test]
fn keys_conf_binds_commands_and_keys_and_the_palette_shows_them() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("config")).unwrap();
    let keys = dir.path().join("config/keys.conf");
    fs::write(
        &keys,
        "# mine\nctrl+j = duplicate\nctrl+d = none\nalt+w = ctrl+a\nctrl+s = find\nctrl+f = save\nalt+q = quit\n",
    )
    .unwrap();
    let before = fs::read(&keys).unwrap();
    let file = dir.path().join("notes.txt");
    fs::write(&file, "one\n").unwrap();
    let mut app = Workspace::open(Some(file.clone())).unwrap();
    configure(&mut app, dir.path());
    let first = screen(&mut app, 80, 24);
    assert!(first.contains(" notes.txt "), "{first}");
    // A command by its name; the default key it replaced; a key by a key.
    key(&mut app, KeyCode::Char('j'), KeyModifiers::CONTROL);
    assert_eq!(app.editor().document.text(), "one\none\n");
    key(&mut app, KeyCode::Char('d'), KeyModifiers::CONTROL);
    assert_eq!(app.editor().document.text(), "one\none\n");
    key(&mut app, KeyCode::Char('w'), KeyModifiers::ALT);
    assert_eq!(app.editor().document.selected_text(), "one\none\n");
    // Two keys that swap commands do not chase each other.
    key(&mut app, KeyCode::Char('s'), KeyModifiers::CONTROL);
    assert!(screen(&mut app, 80, 24).contains(" Find "));
    assert_eq!(fs::read_to_string(&file).unwrap(), "one\n");
    key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    key(&mut app, KeyCode::Char('f'), KeyModifiers::CONTROL);
    assert_eq!(fs::read_to_string(&file).unwrap(), "one\none\n");
    assert!(!app.editor().document.is_dirty());
    // The palette names the user's keys, runs every command whatever its
    // key has become, and types bound letters into its filter.
    key(&mut app, KeyCode::Char('e'), KeyModifiers::CONTROL);
    let palette = screen(&mut app, 80, 30);
    let row = |label: &str| {
        palette
            .lines()
            .find(|line| line.contains(label))
            .unwrap_or_else(|| panic!("{label} in\n{palette}"))
            .to_owned()
    };
    assert!(row("Save document").contains("Ctrl+F"));
    assert!(row("Find in document").contains("Ctrl+S"));
    app.handle_event(Event::Paste("duplicate line or".into()));
    let filtered = screen(&mut app, 80, 30);
    assert!(
        filtered.contains("Ctrl+J") && !filtered.contains("Ctrl+D"),
        "{filtered}"
    );
    key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(app.editor().document.text(), "one\none\none\none\n");
    key(&mut app, KeyCode::Char('z'), KeyModifiers::CONTROL);
    assert!(!app.should_exit);
    key(&mut app, KeyCode::Char('q'), KeyModifiers::ALT);
    assert!(app.should_exit);
    assert_eq!(fs::read(&keys).unwrap(), before);
    app.finish_state().unwrap();
}

#[test]
fn a_keys_conf_with_mistakes_says_so_and_keeps_its_good_lines() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("config")).unwrap();
    fs::write(
        dir.path().join("config/keys.conf"),
        "ctrl+j = duplicate\nctrl+y = dance\nj = save\n",
    )
    .unwrap();
    let mut app = Workspace::open(None).unwrap();
    configure(&mut app, dir.path());
    let shown = screen(&mut app, 100, 12);
    assert!(
        shown.contains("keys.conf line 2: unknown command \"dance\", and 1 more"),
        "{shown}"
    );
    key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    app.editor_mut().document.insert("x");
    key(&mut app, KeyCode::Char('j'), KeyModifiers::CONTROL);
    assert_eq!(app.editor().document.text(), "x\nx");
    // The line that failed leaves its key alone: Ctrl+Y still redoes.
    key(&mut app, KeyCode::Char('z'), KeyModifiers::CONTROL);
    key(&mut app, KeyCode::Char('y'), KeyModifiers::CONTROL);
    assert_eq!(app.editor().document.text(), "x\nx");
    app.finish_state().unwrap();

    // A directory or an oversized file in its place is a problem, not a crash.
    let odd = tempfile::tempdir().unwrap();
    fs::create_dir_all(odd.path().join("config/keys.conf")).unwrap();
    let mut app = Workspace::open(None).unwrap();
    configure(&mut app, odd.path());
    assert!(screen(&mut app, 100, 12).contains("Key bindings unavailable"));
    app.finish_state().unwrap();
}

#[test]
fn bound_keys_stay_out_of_prompts_pickers_and_the_browser() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("config")).unwrap();
    fs::write(dir.path().join("config/keys.conf"), "ctrl+a = duplicate\n").unwrap();
    let mut app = Workspace::open(None).unwrap();
    configure(&mut app, dir.path());
    app.editor_mut().document.insert("x");
    draw(&mut app, 80, 24);
    // In the palette's filter, Ctrl+A is still the field's select all.
    key(&mut app, KeyCode::Char('e'), KeyModifiers::CONTROL);
    app.handle_event(Event::Paste("zzz".into()));
    key(&mut app, KeyCode::Char('a'), KeyModifiers::CONTROL);
    app.handle_event(Event::Paste("undo".into()));
    assert!(screen(&mut app, 80, 24).contains("Undo edit"));
    assert_eq!(app.editor().document.text(), "x");
    key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    // In the Save As prompt it does not duplicate a line either.
    key(&mut app, KeyCode::Char('s'), KeyModifiers::CONTROL);
    draw(&mut app, 80, 24);
    key(&mut app, KeyCode::Char('a'), KeyModifiers::CONTROL);
    assert_eq!(app.editor().document.text(), "x");
    key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    // In the text it is the user's key.
    key(&mut app, KeyCode::Char('a'), KeyModifiers::CONTROL);
    assert_eq!(app.editor().document.text(), "x\nx");
    app.finish_state().unwrap();
}

#[test]
fn find_case_choice_is_saved_when_toggled_and_restored_on_restart() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = Workspace::open(None).unwrap();
    configure(&mut app, dir.path());
    app.editor_mut().document.insert("Pica pica");
    draw(&mut app, 100, 24);
    key(&mut app, KeyCode::Char('f'), KeyModifiers::CONTROL);
    app.handle_event(Event::Paste("pica".into()));
    draw(&mut app, 100, 24);
    assert!(!dir.path().join("config/settings.conf").exists());
    key(&mut app, KeyCode::Char('c'), KeyModifiers::ALT);
    draw(&mut app, 100, 24);
    let saved = fs::read_to_string(dir.path().join("config/settings.conf")).unwrap();
    assert!(saved.contains("find-case=match\n"), "{saved}");
    assert_eq!(app.editor().document.text(), "Pica pica");
    app.finish_state().unwrap();

    crate::search::set_match_case(false);
    let mut restarted = Workspace::open(None).unwrap();
    configure(&mut restarted, dir.path());
    assert!(crate::search::match_case());
    draw(&mut restarted, 100, 24);
    key(&mut restarted, KeyCode::Char('f'), KeyModifiers::CONTROL);
    key(&mut restarted, KeyCode::Char('c'), KeyModifiers::ALT);
    draw(&mut restarted, 100, 24);
    let saved = fs::read_to_string(dir.path().join("config/settings.conf")).unwrap();
    assert!(saved.contains("find-case=ignore\n"), "{saved}");
    restarted.finish_state().unwrap();
    crate::search::set_match_case(false);
}

#[test]
fn icon_preference_survives_restart_without_changing_source_or_selection() {
    use crate::icons::{self, IconSet};
    let original = icons::current();
    icons::set(IconSet::Plain);
    let dir = tempfile::tempdir().unwrap();
    let mut app = Workspace::open(None).unwrap();
    configure(&mut app, dir.path());
    app.editor_mut().document.insert("# A note\n");
    app.editor_mut().document.select_all();
    let selection = app.editor().document.selection();
    let revision = app.editor().document.revision();
    app.run_command(Command::Icons);
    assert_eq!(icons::current(), IconSet::Nerd);
    assert_eq!(app.editor().document.selection(), selection);
    assert_eq!(app.editor().document.revision(), revision);
    let preference = fs::read_to_string(dir.path().join("config/settings.conf")).unwrap();
    assert!(preference.contains("icons=nerd"));
    icons::set(IconSet::Plain);
    let mut restarted = Workspace::open(None).unwrap();
    configure(&mut restarted, dir.path());
    assert_eq!(icons::current(), IconSet::Nerd);
    restarted.run_command(Command::Icons);
    assert_eq!(icons::current(), IconSet::Plain);
    assert_eq!(app.editor().document.text(), "# A note\n");
    key(&mut app, KeyCode::Char('z'), KeyModifiers::CONTROL);
    assert_eq!(app.editor().document.text(), "");
    app.finish_state().unwrap();
    restarted.finish_state().unwrap();
    icons::set(original);
}

#[test]
fn invisible_or_stale_theme_picker_cannot_accept_and_query_never_edits_source() {
    let original = current_theme();
    let mut app = Workspace::open(None).unwrap();
    app.open_themes();
    app.handle_event(Event::Paste("Nord".into()));
    draw(&mut app, 80, 24);
    app.handle_event(Event::Resize(20, 4));
    key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    assert!(app.choice.is_some());
    draw(&mut app, 20, 4);
    key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    assert!(app.choice.is_some());
    key(&mut app, KeyCode::Char('a'), KeyModifiers::CONTROL);
    app.handle_event(Event::Paste("\0\r\n".into()));
    app.handle_event(Event::Resize(80, 24));
    draw(&mut app, 80, 24);
    key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    assert_eq!(current_theme(), original);
    assert!(!app.editor().document.can_undo());
    assert_eq!(app.editor().document.text(), "");
}

#[test]
fn abandoned_edits_recover_as_detached_unsaved_copies_without_overwriting_disk() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("draft.md");
    fs::write(&file, "disk\n").unwrap();
    {
        let mut crashed = Workspace::open(Some(file.clone())).unwrap();
        configure(&mut crashed, dir.path());
        crashed.editor_mut().document.insert("unsaved 界\n");
        checkpoint(&mut crashed);
        fs::write(&file, "newer external source\n").unwrap();
        // Drop intentionally does not perform clean shutdown.
    }
    let mut app = Workspace::open(Some(file.clone())).unwrap();
    configure(&mut app, dir.path());
    app.open_recovery();
    assert!(app.choice.is_some());
    draw(&mut app, 80, 24);
    key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(app.tabs.len(), 2);
    assert_eq!(app.editor().path(), None);
    assert!(app.editor().document.is_dirty());
    assert_eq!(app.editor().document.text(), "unsaved 界\ndisk\n");
    assert_eq!(
        fs::read_to_string(&file).unwrap(),
        "newer external source\n"
    );
    assert!(app.label(app.active).contains("Recovered draft.md"));
    drop(app);
    let mut restarted = Workspace::open(None).unwrap();
    configure(&mut restarted, dir.path());
    restarted.open_recovery();
    assert!(
        restarted.choice.is_some(),
        "recovered buffer was checkpointed before consuming the old copy"
    );
    restarted.choice = None;
    restarted.recover_document(0);
    restarted.request(Intent::Close);
    draw(&mut restarted, 80, 24);
    key(&mut restarted, KeyCode::Char('n'), KeyModifiers::NONE);
    drop(restarted);
    let mut final_session = Workspace::open(None).unwrap();
    configure(&mut final_session, dir.path());
    final_session.open_recovery();
    assert!(
        final_session.choice.is_none(),
        "explicitly discarded recovered copy must not reappear"
    );
    final_session.finish_state().unwrap();
}

/// The status row of a 100×24 frame.
fn status_row(app: &mut Workspace) -> String {
    let snapshot = crate::simulation::snapshot(app, 100, 24).unwrap();
    snapshot.lines().nth(23).unwrap().to_owned()
}

/// Leave `count` unsaved documents behind, as an interrupted session does.
fn abandon(dir: &Path, count: usize) {
    let mut crashed = Workspace::open(None).unwrap();
    configure(&mut crashed, dir);
    for index in 0..count {
        if index > 0 {
            crashed.new_tab();
        }
        crashed.editor_mut().document.insert("unsaved");
    }
    checkpoint(&mut crashed);
    // Drop intentionally does not perform clean shutdown.
}

#[test]
fn recovery_notice_counts_in_plain_words_and_goes_with_the_next_key() {
    for (count, notice) in [
        (
            1,
            " 1 document can be recovered · Ctrl+E → Recover documents",
        ),
        (
            2,
            " 2 documents can be recovered · Ctrl+E → Recover documents",
        ),
    ] {
        let dir = tempfile::tempdir().unwrap();
        abandon(dir.path(), count);
        // A key the editor handles, one only the workspace sees, and typing.
        for (code, modifiers) in [
            (KeyCode::Down, KeyModifiers::NONE),
            (KeyCode::Char('e'), KeyModifiers::CONTROL),
            (KeyCode::Char('x'), KeyModifiers::NONE),
        ] {
            let mut app = Workspace::open(None).unwrap();
            configure(&mut app, dir.path());
            let row = status_row(&mut app);
            assert!(row.starts_with(notice), "{row}");
            assert!(!row.contains("(s)") && !row.contains("Esc"), "{row}");
            // Looking is not a keypress: the notice waits to be read.
            app.handle_event(Event::Resize(100, 24));
            app.tick();
            assert!(status_row(&mut app).starts_with(notice));
            key(&mut app, code, modifiers);
            if modifiers == KeyModifiers::CONTROL {
                assert!(app.palette.is_some());
                key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
            }
            let row = status_row(&mut app);
            assert!(row.starts_with(" untitled"), "{row}");
            assert!(row.contains("Ln 1, Col") && !row.contains("recovered"));
            // The copies are still there to recover.
            app.open_recovery();
            assert!(app.choice.is_some());
        }
    }
}

#[test]
fn a_launch_problem_stays_until_dismissed_and_carries_the_recovery_notice() {
    let dir = tempfile::tempdir().unwrap();
    abandon(dir.path(), 1);
    fs::create_dir_all(dir.path().join("config")).unwrap();
    fs::write(dir.path().join("config/settings.conf"), "not a setting\n").unwrap();
    let mut app = Workspace::open(None).unwrap();
    configure(&mut app, dir.path());
    let problem = status_row(&mut app);
    assert!(
        problem.starts_with(" 1 document can be recovered · Ctrl+E → Recover documents · Pref"),
        "{problem}"
    );
    assert!(problem.trim_end().ends_with("Esc"), "{problem}");
    for code in [KeyCode::Down, KeyCode::Char('x')] {
        key(&mut app, code, KeyModifiers::NONE);
        assert_eq!(status_row(&mut app), problem);
    }
    key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    assert!(status_row(&mut app).starts_with(" untitled ●"));
}

#[test]
fn recovery_answers_are_notices_and_a_recovered_tab_says_so_once() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = Workspace::open(None).unwrap();
    configure(&mut app, dir.path());
    app.open_recovery();
    assert!(app.choice.is_none());
    assert!(status_row(&mut app).starts_with(" No documents to recover"));
    key(&mut app, KeyCode::Right, KeyModifiers::NONE);
    assert!(status_row(&mut app).starts_with(" untitled"));
    drop(app);
    abandon(dir.path(), 1);
    let mut app = Workspace::open(None).unwrap();
    configure(&mut app, dir.path());
    app.recover_document(0);
    let row = status_row(&mut app);
    assert!(row.starts_with(" Recovered as an unsaved copy"), "{row}");
    assert!(!row.contains("Esc"), "{row}");
    key(&mut app, KeyCode::Right, KeyModifiers::NONE);
    assert!(status_row(&mut app).starts_with(" Recovered untitled ●"));
    app.finish_state().unwrap();
}

#[test]
fn a_recovered_copy_can_be_saved_back_over_its_original_after_asking() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("draft.md");
    fs::write(&file, "disk\n").unwrap();
    {
        let mut crashed = Workspace::open(Some(file.clone())).unwrap();
        configure(&mut crashed, dir.path());
        crashed.editor_mut().document.insert("unsaved ");
        checkpoint(&mut crashed);
    }
    let mut app = Workspace::open(None).unwrap();
    configure(&mut app, dir.path());
    app.recover_document(0);
    let row = status_row(&mut app);
    assert!(
        row.starts_with(" Recovered as an unsaved copy · Ctrl+S saves it"),
        "{row}"
    );
    // Ctrl+S opens Save As on the file the copy came from, and replacing
    // that file is asked about like any other.
    key(&mut app, KeyCode::Char('s'), KeyModifiers::CONTROL);
    assert!(app.editor().saving_as());
    assert_eq!(app.editor().pending_save_path(), Some(file.clone()));
    key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(fs::read_to_string(&file).unwrap(), "disk\n");
    let snapshot = crate::simulation::snapshot(&mut app, 100, 24).unwrap();
    assert!(snapshot.contains("draft.md already exists."), "{snapshot}");
    key(&mut app, KeyCode::Char('y'), KeyModifiers::NONE);
    assert_eq!(fs::read_to_string(&file).unwrap(), "unsaved disk\n");
    assert_eq!(app.editor().path(), Some(file.as_path()));
    assert!(!app.editor().document.is_dirty());
    app.finish_state().unwrap();
}

#[test]
fn save_clears_recovery_and_cancelled_quit_retains_all_dirty_tabs() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("draft.md");
    fs::write(&file, "one\n").unwrap();
    {
        let mut app = Workspace::open(Some(file.clone())).unwrap();
        configure(&mut app, dir.path());
        app.editor_mut().document.insert("saved ");
        checkpoint(&mut app);
        key(&mut app, KeyCode::Char('s'), KeyModifiers::CONTROL);
    }
    let mut app = Workspace::open(None).unwrap();
    configure(&mut app, dir.path());
    app.open_recovery();
    assert!(app.choice.is_none());
    app.editor_mut().document.insert("first");
    app.new_tab();
    app.editor_mut().document.insert("second");
    checkpoint(&mut app);
    app.request(Intent::Quit);
    draw(&mut app, 80, 24);
    key(&mut app, KeyCode::Char('n'), KeyModifiers::NONE);
    draw(&mut app, 80, 24);
    key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    assert!(!app.should_exit);
    drop(app);
    let mut recovered = Workspace::open(None).unwrap();
    configure(&mut recovered, dir.path());
    recovered.recover_document(0);
    let first = recovered.editor().document.text().to_owned();
    recovered.recover_document(0);
    let second = recovered.editor().document.text().to_owned();
    assert!(first == "first" && second == "second" || first == "second" && second == "first");
    recovered.finish_state().unwrap();
}

#[test]
fn external_change_checks_run_even_without_persistent_state() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("draft.md");
    fs::write(&file, "before").unwrap();
    let mut app = Workspace::open(Some(file.clone())).unwrap();
    // Written a few seconds ago, so it is whole. The tick loads it into the
    // clean buffer, and says so.
    fs::write(&file, "after").unwrap();
    fs::File::options()
        .write(true)
        .open(&file)
        .unwrap()
        .set_modified(std::time::SystemTime::now() - Duration::from_secs(5))
        .unwrap();
    app.last_tick = Instant::now() - Duration::from_secs(2);
    assert!(app.tick());
    assert_eq!(app.editor().document.text(), "after");
    assert!(status_row(&mut app).starts_with(" Changed on disk: reloaded"));
    // Reload has nothing more to fetch, and undo still has the old text.
    app.run_command(Command::Reload);
    assert_eq!(app.editor().document.text(), "after");
    key(&mut app, KeyCode::Char('z'), KeyModifiers::CONTROL);
    assert_eq!(app.editor().document.text(), "before");
    assert!(app.editor().document.is_dirty());
    // Now there are edits to lose: a further change waits for a decision.
    fs::write(&file, "later").unwrap();
    app.last_tick = Instant::now() - Duration::from_secs(2);
    assert!(app.tick());
    assert_eq!(app.editor().document.text(), "before");
    assert!(status_row(&mut app).starts_with(" Disk file changed; your text is intact"));
}

#[test]
fn empty_unsaved_recovery_stays_dirty_and_can_be_saved() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("removed.md");
    fs::write(&file, "delete me").unwrap();
    {
        let mut app = Workspace::open(Some(file.clone())).unwrap();
        configure(&mut app, dir.path());
        app.editor_mut().document.select_all();
        app.editor_mut().document.insert("");
        checkpoint(&mut app);
    }
    let mut app = Workspace::open(None).unwrap();
    configure(&mut app, dir.path());
    app.recover_document(0);
    assert_eq!(app.editor().document.text(), "");
    assert!(app.editor().document.is_dirty());
    assert_eq!(app.editor().path(), None);
    assert_eq!(fs::read_to_string(file).unwrap(), "delete me");
    app.finish_state().unwrap();
}

#[cfg(unix)]
#[test]
fn control_characters_in_filenames_do_not_prevent_crash_recovery() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("draft\nnotes\t.md");
    fs::write(&file, "disk").unwrap();
    {
        let mut app = Workspace::open(Some(file.clone())).unwrap();
        configure(&mut app, dir.path());
        app.editor_mut().document.insert("local ");
        checkpoint(&mut app);
    }
    let mut app = Workspace::open(None).unwrap();
    configure(&mut app, dir.path());
    app.recover_document(0);
    assert_eq!(app.editor().document.text(), "local disk");
    assert_eq!(
        app.tabs[app.active].recovered.as_ref().unwrap().1,
        Some(file)
    );
    app.finish_state().unwrap();
}

#[test]
fn save_via_cancelled_quit_removes_its_recovery_copy_immediately() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("first.md");
    let second = dir.path().join("second.md");
    fs::write(&first, "first").unwrap();
    fs::write(&second, "second").unwrap();
    {
        let mut app = Workspace::open(Some(first.clone())).unwrap();
        configure(&mut app, dir.path());
        app.editor_mut().document.insert("saved ");
        app.open_path(second).unwrap();
        app.editor_mut().document.insert("unsaved ");
        checkpoint(&mut app);
        app.request(Intent::Quit);
        draw(&mut app, 80, 24);
        key(&mut app, KeyCode::Char('y'), KeyModifiers::NONE);
        draw(&mut app, 80, 24);
        key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
        assert_eq!(fs::read_to_string(first).unwrap(), "saved first");
    }
    let mut app = Workspace::open(None).unwrap();
    configure(&mut app, dir.path());
    app.recover_document(0);
    assert_eq!(app.editor().document.text(), "unsaved second");
    app.open_recovery();
    assert!(app.choice.is_none());
    app.finish_state().unwrap();
}

#[test]
fn markdown_view_choice_is_remembered_without_forcing_other_files() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = Workspace::open(None).unwrap();
    configure(&mut app, dir.path());
    assert!(app.editor().live);
    // F6 stays a silent alias for the view switch whatever the primary key.
    key(&mut app, KeyCode::F(6), KeyModifiers::NONE);
    draw(&mut app, 80, 24);
    assert!(!app.editor().live);
    let saved = fs::read_to_string(dir.path().join("config/settings.conf")).unwrap();
    assert!(saved.contains("markdown-view=source"), "{saved}");
    app.new_tab();
    assert!(
        !app.editor().live,
        "new Markdown opens in the remembered view"
    );
    let mut restarted = Workspace::open(None).unwrap();
    configure(&mut restarted, dir.path());
    assert!(!restarted.editor().live);
    key(&mut restarted, KeyCode::F(6), KeyModifiers::NONE);
    draw(&mut restarted, 80, 24);
    let saved = fs::read_to_string(dir.path().join("config/settings.conf")).unwrap();
    assert!(saved.contains("markdown-view=live"), "{saved}");
    // Opening without a choice saved never rewrites settings.
    let other = tempfile::tempdir().unwrap();
    let mut fresh = Workspace::open(None).unwrap();
    configure(&mut fresh, other.path());
    draw(&mut fresh, 80, 24);
    assert!(fresh.editor().live);
    assert!(!other.path().join("config/settings.conf").exists());
    app.finish_state().unwrap();
    restarted.finish_state().unwrap();
    fresh.finish_state().unwrap();
}
