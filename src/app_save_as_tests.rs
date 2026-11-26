//! Save As through the editor's own event path, against real files.
use super::*;
use ratatui::{Terminal, backend::TestBackend};
use std::fs;

fn key(app: &mut App, code: KeyCode, modifiers: KeyModifiers) {
    app.handle_event(Event::Key(KeyEvent::new(code, modifiers)));
}
fn press(app: &mut App, code: KeyCode) {
    key(app, code, KeyModifiers::NONE);
}
fn type_text(app: &mut App, text: &str) {
    for c in text.chars() {
        press(app, KeyCode::Char(c));
    }
}
fn paste(app: &mut App, text: impl Into<String>) {
    app.handle_event(Event::Paste(text.into()));
}
/// The whole screen as text, one row per line.
fn screen(app: &mut App, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let buffer = terminal.backend().buffer();
    (0..height)
        .map(|y| {
            let row: String = (0..width).map(|x| buffer[(x, y)].symbol()).collect();
            format!("{}\n", row.trim_end())
        })
        .collect()
}
fn field(app: &App) -> &str {
    app.save_as.path.text()
}
/// An untitled document holding `text`, with Save As open on `path`.
fn saving(text: &str, path: &Path) -> App {
    let mut app = App::new(String::new(), None, true);
    paste(&mut app, text);
    press(&mut app, KeyCode::F(4));
    paste(&mut app, path.display().to_string());
    app
}
/// A scratch folder whose own name cannot be mistaken for a temporary file's.
fn scratch() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("pica-save-")
        .tempdir()
        .unwrap()
}
/// Nothing a user should ever have to read.
fn assert_plain(text: &str) {
    for raw in ["os error", ".tmp", "at path \"", "(os"] {
        assert!(!text.contains(raw), "{raw:?} in {text}");
    }
}

#[test]
fn save_as_starts_from_the_documents_path_with_its_name_selected() {
    let dir = scratch();
    let notes = dir.path().join("notes.md");
    fs::write(&notes, "notes\n").unwrap();
    let mut app = App::open(Some(notes.clone())).unwrap();
    press(&mut app, KeyCode::F(4));
    assert_eq!(field(&app), notes.display().to_string());
    assert_eq!(app.save_as.path.selected_text(), "notes.md");
    let text = screen(&mut app, 100, 24);
    assert!(
        text.contains("Save as") && text.contains("notes.md"),
        "{text}"
    );
    // Typing renames; the folder stays.
    type_text(&mut app, "plan.md");
    let plan = dir.path().join("plan.md");
    assert_eq!(field(&app), plan.display().to_string());
    press(&mut app, KeyCode::Enter);
    assert_eq!(fs::read_to_string(&plan).unwrap(), "notes\n");
    assert_eq!(app.path(), Some(plan.as_path()));
    assert_eq!(fs::read_to_string(&notes).unwrap(), "notes\n");
    assert!(!app.saving_as() && !app.document.is_dirty());
    assert!(app.message.starts_with("Saved ") && !app.message_is_error);

    // An untitled document starts in the working folder, ready for a name.
    let mut app = App::new(String::new(), None, true);
    press(&mut app, KeyCode::F(4));
    let folder = crate::paths::folder_prompt(&std::env::current_dir().unwrap());
    assert_eq!(field(&app), folder);
    assert!(folder.ends_with('/'));
    assert_eq!(app.save_as.path.selection(), Selection::caret(folder.len()));

    // A recovered copy starts from the file it came from.
    let mut app = App::recovered("draft".into(), Selection::caret(0), true, true);
    app.suggest_save_path(Some(notes.clone()));
    key(&mut app, KeyCode::Char('s'), KeyModifiers::CONTROL);
    assert_eq!(field(&app), notes.display().to_string());
    assert_eq!(app.save_as.path.selected_text(), "notes.md");
}

#[test]
fn the_path_field_edits_like_a_field_and_takes_whole_paths() {
    let dir = scratch();
    let notes = dir.path().join("notes.md");
    fs::write(&notes, "notes\n").unwrap();
    let folder = format!("{}/", dir.path().display());
    let mut app = App::open(Some(notes)).unwrap();
    press(&mut app, KeyCode::F(4));
    // A path from the root, pasted or typed over the selected name,
    // replaces the folder too instead of landing after it.
    paste(&mut app, "/elsewhere/other.md\n");
    assert_eq!(field(&app), "/elsewhere/other.md");
    key(&mut app, KeyCode::Char('z'), KeyModifiers::CONTROL);
    assert_eq!(field(&app), format!("{folder}notes.md"));
    press(&mut app, KeyCode::End);
    for _ in 0.."notes.md".len() {
        press(&mut app, KeyCode::Backspace);
    }
    assert_eq!(field(&app), folder);
    // Ctrl+Backspace and Ctrl+Left work by words, as in the document.
    type_text(&mut app, "two words");
    key(&mut app, KeyCode::Backspace, KeyModifiers::CONTROL);
    assert_eq!(field(&app), format!("{folder}two "));
    key(&mut app, KeyCode::Left, KeyModifiers::CONTROL);
    type_text(&mut app, "one ");
    assert_eq!(field(&app), format!("{folder}one two "));
    key(&mut app, KeyCode::Char('z'), KeyModifiers::CONTROL);
    key(&mut app, KeyCode::Char('a'), KeyModifiers::CONTROL);
    press(&mut app, KeyCode::Delete);
    paste(&mut app, folder.clone());
    assert_eq!(field(&app), folder);
    // Typed after a folder, with nothing selected, a slash is the one
    // already there and a tilde is a character: Tab completes a folder
    // with its slash, and people type the slash anyway.
    type_text(&mut app, "/");
    assert_eq!(field(&app), folder);
    type_text(&mut app, "~x/");
    assert_eq!(field(&app), format!("{folder}~x/"));
    type_text(&mut app, "/");
    assert_eq!(field(&app), format!("{folder}~x/"));
    // Typed over a selection, they begin a new path.
    press(&mut app, KeyCode::Esc);
    press(&mut app, KeyCode::F(4));
    assert_eq!(app.save_as.path.selected_text(), "notes.md");
    type_text(&mut app, "~");
    assert_eq!(field(&app), "~");
    type_text(&mut app, "/x");
    assert_eq!(field(&app), "~/x");
    key(&mut app, KeyCode::Char('a'), KeyModifiers::CONTROL);
    type_text(&mut app, "/etc");
    assert_eq!(field(&app), "/etc");
    // Caret keys, selection, and Ctrl+U.
    press(&mut app, KeyCode::Home);
    key(&mut app, KeyCode::Right, KeyModifiers::SHIFT);
    assert_eq!(app.save_as.path.selected_text(), "/");
    key(&mut app, KeyCode::Char('a'), KeyModifiers::CONTROL);
    type_text(&mut app, "name with spaces.md");
    assert_eq!(field(&app), "name with spaces.md");
    key(&mut app, KeyCode::Char('u'), KeyModifiers::CONTROL);
    assert_eq!(field(&app), "");
    // Enter on an empty field does nothing; control characters never enter.
    press(&mut app, KeyCode::Enter);
    paste(&mut app, "a\u{1b}b\tc\0");
    assert_eq!(field(&app), "abc");
    assert!(app.saving_as());
    assert_eq!(app.document.text(), "notes\n");
    assert!(!app.document.can_undo());
}

#[test]
fn saving_over_an_existing_file_asks_first_and_replaces_only_on_y() {
    let dir = scratch();
    let other = dir.path().join("other.md");
    fs::write(&other, "what was there\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&other, fs::Permissions::from_mode(0o640)).unwrap();
    }
    let mut app = saving("my text\n", &other);
    press(&mut app, KeyCode::Enter);
    assert!(app.saving_as() && app.document.is_dirty());
    assert_eq!(fs::read_to_string(&other).unwrap(), "what was there\n");
    // An unseen question cannot be answered.
    press(&mut app, KeyCode::Char('y'));
    assert_eq!(fs::read_to_string(&other).unwrap(), "what was there\n");
    let text = screen(&mut app, 100, 24);
    assert!(text.contains("Replace file?"), "{text}");
    assert!(text.contains("other.md already exists."), "{text}");
    assert!(text.contains(" Y  Replace   N  Choose another name   Esc  Cancel"));
    // Nothing but a plain, deliberate Y replaces it.
    for (code, modifiers) in [
        (KeyCode::Enter, KeyModifiers::NONE),
        (KeyCode::Char('x'), KeyModifiers::NONE),
        (KeyCode::Char('y'), KeyModifiers::CONTROL),
        (KeyCode::Char('y'), KeyModifiers::ALT),
    ] {
        key(&mut app, code, modifiers);
    }
    app.handle_event(Event::Key(KeyEvent::new_with_kind(
        KeyCode::Char('y'),
        KeyModifiers::NONE,
        KeyEventKind::Repeat,
    )));
    paste(&mut app, "y");
    // Nor does one in a window too small to show the question.
    screen(&mut app, 30, 6);
    press(&mut app, KeyCode::Char('y'));
    screen(&mut app, 100, 24);
    app.handle_event(Event::Resize(30, 6));
    press(&mut app, KeyCode::Char('y'));
    assert_eq!(fs::read_to_string(&other).unwrap(), "what was there\n");
    assert!(app.saving_as() && app.document.is_dirty());
    // N goes back to the name.
    screen(&mut app, 100, 24);
    press(&mut app, KeyCode::Char('n'));
    assert!(app.saving_as());
    assert_eq!(field(&app), other.display().to_string());
    assert!(!screen(&mut app, 100, 24).contains("Replace file?"));
    // Y, seen and plain, replaces it and keeps the file's permissions.
    press(&mut app, KeyCode::Enter);
    screen(&mut app, 100, 24);
    key(&mut app, KeyCode::Char('Y'), KeyModifiers::SHIFT);
    assert_eq!(fs::read_to_string(&other).unwrap(), "my text\n");
    assert_eq!(app.path(), Some(other.as_path()));
    assert!(!app.saving_as() && !app.document.is_dirty());
    assert!(app.message.starts_with("Saved "));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(&other).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o640);
    }
    // It is now this document's file: Ctrl+S saves to it.
    paste(&mut app, "more ");
    key(&mut app, KeyCode::Char('s'), KeyModifiers::CONTROL);
    assert_eq!(fs::read_to_string(&other).unwrap(), "my text\nmore ");
}

#[test]
fn esc_cancels_the_question_and_a_file_that_changed_again_is_not_replaced() {
    let dir = scratch();
    let other = dir.path().join("other.md");
    fs::write(&other, "old").unwrap();
    let mut app = saving("mine", &other);
    press(&mut app, KeyCode::Enter);
    screen(&mut app, 100, 24);
    press(&mut app, KeyCode::Esc);
    assert!(!app.saving_as());
    assert_eq!(app.overlay, Overlay::None);
    press(&mut app, KeyCode::Char('y'));
    assert_eq!(fs::read_to_string(&other).unwrap(), "old");
    assert_eq!(app.document.text(), "miney");
    // Between the question and its answer, someone else writes the file.
    let mut app = saving("mine", &other);
    press(&mut app, KeyCode::Enter);
    screen(&mut app, 100, 24);
    fs::write(&other, "newer than the question").unwrap();
    press(&mut app, KeyCode::Char('y'));
    assert_eq!(
        fs::read_to_string(&other).unwrap(),
        "newer than the question"
    );
    assert!(app.saving_as() && app.document.is_dirty());
    let text = screen(&mut app, 100, 24);
    assert!(
        text.contains("changed on disk; nothing was replaced"),
        "{text}"
    );
    assert_plain(&text);
    // Asking again is asking about the newer file.
    press(&mut app, KeyCode::Enter);
    screen(&mut app, 100, 24);
    press(&mut app, KeyCode::Char('y'));
    assert_eq!(fs::read_to_string(&other).unwrap(), "mine");
}

#[cfg(unix)]
#[test]
fn save_as_still_refuses_what_a_save_refuses() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let dir = scratch();
    let path = |name: &str| dir.path().join(name);
    fs::write(path("linked.md"), "two names").unwrap();
    fs::hard_link(path("linked.md"), path("second-name.md")).unwrap();
    fs::write(path("locked.md"), "locked").unwrap();
    fs::set_permissions(path("locked.md"), fs::Permissions::from_mode(0o444)).unwrap();
    fs::create_dir(path("folder")).unwrap();
    symlink(path("nowhere.md"), path("dangling.md")).unwrap();
    assert!(
        std::process::Command::new("mkfifo")
            .arg(path("pipe"))
            .status()
            .unwrap()
            .success()
    );
    let root = unsafe { libc::geteuid() } == 0;
    for (name, reason) in [
        ("linked.md", "other hard links"),
        ("locked.md", "read-only"),
        ("folder", "That is a folder"),
        ("dangling.md", "does not exist"),
        ("pipe", "Only regular files"),
    ] {
        if root && name == "locked.md" {
            continue;
        }
        let mut app = saving("mine", &path(name));
        press(&mut app, KeyCode::Enter);
        let text = screen(&mut app, 100, 24);
        assert!(text.contains(reason), "{name}: {text}");
        assert!(!text.contains("Replace file?"), "{name}: {text}");
        assert_plain(&text);
        // No question was asked, so Y is only a letter in the name.
        press(&mut app, KeyCode::Char('y'));
        assert!(app.saving_as() && app.document.is_dirty(), "{name}");
        assert!(field(&app).ends_with('y'), "{name}");
    }
    assert_eq!(fs::read_to_string(path("linked.md")).unwrap(), "two names");
    assert_eq!(fs::read_to_string(path("locked.md")).unwrap(), "locked");
    assert!(fs::read_dir(path("folder")).unwrap().next().is_none());
    assert!(!path("nowhere.md").exists());
    // A folder gains its slash, ready for a name inside it.
    let mut app = saving("mine", &path("folder"));
    press(&mut app, KeyCode::Enter);
    assert_eq!(field(&app), format!("{}/", path("folder").display()));
    type_text(&mut app, "inside.md");
    press(&mut app, KeyCode::Enter);
    assert_eq!(
        fs::read_to_string(path("folder/inside.md")).unwrap(),
        "mine"
    );

    // A link to a file is followed, as when opening one: the question
    // names where it leads, and the link itself is kept.
    fs::write(path("real.md"), "real").unwrap();
    symlink(path("real.md"), path("alias.md")).unwrap();
    let mut app = saving("mine", &path("alias.md"));
    press(&mut app, KeyCode::Enter);
    let text = screen(&mut app, 100, 24);
    assert!(text.contains("alias.md is a link to"), "{text}");
    press(&mut app, KeyCode::Char('y'));
    assert_eq!(fs::read_to_string(path("real.md")).unwrap(), "mine");
    assert!(
        fs::symlink_metadata(path("alias.md"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(app.path(), Some(path("alias.md").as_path()));
}

#[test]
fn save_as_onto_its_own_file_saves_and_asks_only_when_the_disk_version_changed() {
    let dir = scratch();
    let notes = dir.path().join("notes.md");
    fs::write(&notes, "base").unwrap();
    let mut app = App::open(Some(notes.clone())).unwrap();
    paste(&mut app, "mine ");
    press(&mut app, KeyCode::F(4));
    // The name is already there: Enter is an ordinary save.
    press(&mut app, KeyCode::Enter);
    assert_eq!(fs::read_to_string(&notes).unwrap(), "mine base");
    assert!(!app.saving_as() && !app.document.is_dirty());
    // With a newer version on disk, a plain save refuses and says where to
    // go; Save As then asks, and Y keeps this document's text.
    paste(&mut app, "more ");
    fs::write(&notes, "theirs").unwrap();
    key(&mut app, KeyCode::Char('s'), KeyModifiers::CONTROL);
    assert!(app.message_is_error);
    assert_eq!(
        app.message,
        "The file changed on disk. Your edits are intact · Ctrl+E: Save as"
    );
    assert_eq!(fs::read_to_string(&notes).unwrap(), "theirs");
    press(&mut app, KeyCode::F(4));
    press(&mut app, KeyCode::Enter);
    assert!(app.saving_as());
    assert_eq!(fs::read_to_string(&notes).unwrap(), "theirs");
    let text = screen(&mut app, 100, 24);
    assert!(
        text.contains("notes.md changed on disk after you opened it."),
        "{text}"
    );
    assert!(text.contains("Replace the version on disk with this document?"));
    press(&mut app, KeyCode::Char('y'));
    assert_eq!(fs::read_to_string(&notes).unwrap(), "mine more base");
    assert!(!app.saving_as() && !app.document.is_dirty());
    assert_eq!(app.path(), Some(notes.as_path()));
    assert!(!app.check_external_change());
    paste(&mut app, "last ");
    key(&mut app, KeyCode::Char('s'), KeyModifiers::CONTROL);
    assert!(fs::read_to_string(&notes).unwrap().contains("last "));
}

#[test]
fn a_missing_folder_is_offered_and_created_only_on_y() {
    let dir = scratch();
    let target = dir.path().join("notes/2026/new.md");
    let mut app = saving("draft\r\n", &target);
    press(&mut app, KeyCode::Enter);
    assert!(app.saving_as());
    let text = screen(&mut app, 100, 24);
    assert!(text.contains("Create folder?"), "{text}");
    assert!(text.contains("notes/2026 does not exist."), "{text}");
    assert!(text.contains("Create it and save new.md there?"), "{text}");
    assert!(text.contains(" Y  Create and save   N  Choose another name"));
    assert_plain(&text);
    press(&mut app, KeyCode::Char('n'));
    assert!(!dir.path().join("notes").exists());
    assert_eq!(field(&app), target.display().to_string());
    press(&mut app, KeyCode::Enter);
    screen(&mut app, 100, 24);
    press(&mut app, KeyCode::Char('y'));
    assert_eq!(fs::read(&target).unwrap(), b"draft\r\n");
    assert_eq!(app.path(), Some(target.as_path()));
    assert!(!app.saving_as() && !app.document.is_dirty());
    // A file where a folder should be cannot be fixed by creating one.
    let blocked = dir.path().join("notes/2026/new.md/inside.md");
    let mut app = saving("draft", &blocked);
    press(&mut app, KeyCode::Enter);
    let text = screen(&mut app, 100, 24);
    assert!(
        text.contains("Part of that path is a file, not a folder"),
        "{text}"
    );
    assert!(!text.contains("Create folder?"));
    assert_plain(&text);
    assert_eq!(fs::read(&target).unwrap(), b"draft\r\n");
}

#[test]
fn tab_completes_the_path_and_lists_what_it_cannot_decide() {
    let dir = scratch();
    fs::create_dir(dir.path().join("notes")).unwrap();
    fs::write(dir.path().join("notes.md"), "").unwrap();
    fs::write(dir.path().join("plan.md"), "").unwrap();
    let root = dir.path().display().to_string();
    let mut app = saving("text", &dir.path().join("pl"));
    press(&mut app, KeyCode::Tab);
    assert_eq!(field(&app), format!("{root}/plan.md"));
    let text = screen(&mut app, 100, 24);
    assert!(text.contains("Tab completes names."), "{text}");
    assert!(
        text.contains(" ⏎  Save   Tab  Complete   Esc  Cancel"),
        "{text}"
    );
    key(&mut app, KeyCode::Char('a'), KeyModifiers::CONTROL);
    paste(&mut app, format!("{root}/no"));
    press(&mut app, KeyCode::Tab);
    assert_eq!(field(&app), format!("{root}/notes"));
    let text = screen(&mut app, 100, 24);
    assert!(text.contains("notes/  notes.md"), "{text}");
    // Typing on narrows it; the list goes once the path changes.
    type_text(&mut app, "/");
    assert!(!screen(&mut app, 100, 24).contains("notes/  notes.md"));
    type_text(&mut app, "zz");
    press(&mut app, KeyCode::Tab);
    assert_eq!(field(&app), format!("{root}/notes/zz"));
    assert!(screen(&mut app, 100, 24).contains("No name here starts like that."));
    // Tab never saves, and the caret ends after what it filled in.
    assert!(app.saving_as() && app.document.is_dirty());
    press(&mut app, KeyCode::Backspace);
    press(&mut app, KeyCode::Backspace);
    type_text(&mut app, "new.md");
    press(&mut app, KeyCode::Enter);
    assert_eq!(
        fs::read_to_string(dir.path().join("notes/new.md")).unwrap(),
        "text"
    );
}

#[test]
fn save_as_problems_stay_in_the_dialog_and_leave_with_it() {
    let dir = scratch();
    let mut app = saving("text", dir.path());
    press(&mut app, KeyCode::Enter);
    let text = screen(&mut app, 100, 24);
    assert!(text.contains("That is a folder."), "{text}");
    // The status line under the dialog keeps naming the document.
    assert!(
        text.lines().last().unwrap().starts_with(" untitled ●"),
        "{text}"
    );
    assert!(app.message.is_empty());
    // Editing the path answers the problem.
    type_text(&mut app, "x");
    assert!(!screen(&mut app, 100, 24).contains("That is a folder."));
    press(&mut app, KeyCode::Backspace);
    press(&mut app, KeyCode::Enter);
    press(&mut app, KeyCode::Esc);
    let text = screen(&mut app, 100, 24);
    assert!(!text.contains("folder"), "{text}");
    assert!(app.message.is_empty() && !app.message_is_error);
    // Opened again, the dialog starts clean.
    press(&mut app, KeyCode::F(4));
    assert!(!screen(&mut app, 100, 24).contains("That is a folder."));
}

#[test]
fn saving_under_another_kind_of_name_changes_the_documents_kind() {
    let dir = scratch();
    let script = dir.path().join("tool.py");
    let mut app = saving("print('hi')\n", &script);
    assert!(app.is_markdown());
    press(&mut app, KeyCode::Enter);
    assert!(!app.is_markdown());
    assert_eq!(
        app.language().map(|language| language.name()),
        Some("Python")
    );
    // Past the "Saved" notice, the status line names the new kind.
    press(&mut app, KeyCode::Right);
    let text = screen(&mut app, 100, 24);
    assert!(text.lines().last().unwrap().contains("python"), "{text}");
    // And back: the same text under a Markdown name is Markdown again.
    let notes = dir.path().join("notes.md");
    press(&mut app, KeyCode::F(4));
    type_text(&mut app, "notes.md");
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.path(), Some(notes.as_path()));
    assert!(app.is_markdown() && app.language().is_none());
}

#[test]
fn a_failed_save_says_so_in_a_sentence() {
    let dir = scratch();
    let folder = dir.path().join("gone");
    fs::create_dir(&folder).unwrap();
    let notes = folder.join("notes.md");
    fs::write(&notes, "base").unwrap();
    let mut app = App::open(Some(notes.clone())).unwrap();
    paste(&mut app, "mine ");
    // The folder disappears under the open file.
    fs::remove_dir_all(&folder).unwrap();
    key(&mut app, KeyCode::Char('s'), KeyModifiers::CONTROL);
    assert!(app.message_is_error);
    assert_eq!(
        app.message,
        "The file was removed from disk. Your edits are intact · Ctrl+E: Save as"
    );
    assert_plain(&app.message);
    // Save As offers to put the folder back.
    press(&mut app, KeyCode::F(4));
    press(&mut app, KeyCode::Enter);
    let text = screen(&mut app, 100, 24);
    assert!(text.contains("Create folder?"), "{text}");
    press(&mut app, KeyCode::Char('y'));
    assert_eq!(fs::read_to_string(&notes).unwrap(), "mine base");
    assert!(!app.document.is_dirty());
}

#[test]
fn a_question_cut_short_by_a_small_window_cannot_be_answered() {
    let dir = scratch();
    let other = dir.path().join("other.md");
    fs::write(&other, "what was there\n").unwrap();
    let mut app = saving("my text\n", &other);
    press(&mut app, KeyCode::Enter);
    // Each of these windows is big enough for a dialog, but not for all of
    // this question: the warning, or one of the three keys, would be cut
    // off. Nothing of it is offered, and Y replaces nothing.
    for (width, height) in [(44, 9), (44, 11), (50, 10), (58, 9)] {
        let text = screen(&mut app, width, height);
        assert!(
            text.contains("Resize to confirm"),
            "{width}x{height}: {text}"
        );
        assert!(!text.contains("Replace"), "{width}x{height}: {text}");
        press(&mut app, KeyCode::Char('y'));
        assert_eq!(fs::read_to_string(&other).unwrap(), "what was there\n");
        assert!(app.saving_as() && app.document.is_dirty());
    }
    // With room, all of it shows, the keys on rows of their own where the
    // window is narrow, and only then does Y replace the file.
    let text = screen(&mut app, 44, 12);
    for part in [
        "Replace file?",
        "other.md already exists.",
        "Replace it with this document?",
        "will be lost.",
        " Y  Replace",
        " N  Choose another name",
        " Esc  Cancel",
    ] {
        assert!(text.contains(part), "{part:?} in {text}");
    }
    assert_eq!(fs::read_to_string(&other).unwrap(), "what was there\n");
    press(&mut app, KeyCode::Char('y'));
    assert_eq!(fs::read_to_string(&other).unwrap(), "my text\n");
    assert!(!app.saving_as() && !app.document.is_dirty());
}
