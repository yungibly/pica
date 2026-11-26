//! Following links through the real key and pointer paths. No test here
//! starts a browser: each installs an opener that only records.
use super::*;
use crossterm::event::{KeyEvent, MouseEvent};
use ratatui::{Terminal, backend::TestBackend, buffer::Buffer};
use std::{cell::RefCell, fs};

thread_local! {
    static OPENED: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

fn record(address: &str) -> io::Result<()> {
    OPENED.with_borrow_mut(|opened| opened.push(address.to_owned()));
    Ok(())
}

/// What the opener was handed since the last call.
fn opened() -> Vec<String> {
    OPENED.with_borrow_mut(std::mem::take)
}

fn key(app: &mut Workspace, code: KeyCode, modifiers: KeyModifiers) {
    app.handle_event(Event::Key(KeyEvent::new(code, modifiers)));
}

fn follow(app: &mut Workspace) {
    key(app, KeyCode::Char('l'), KeyModifiers::ALT);
}

fn draw(app: &mut Workspace) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    terminal.backend().buffer().clone()
}

fn status(app: &mut Workspace) -> String {
    let buffer = draw(app);
    (0..100).map(|x| buffer[(x, 23)].symbol()).collect()
}

/// A workspace on `name` holding `source`, in a folder of its own, with
/// the recording opener.
fn workspace(name: &str, source: &str) -> (tempfile::TempDir, Workspace) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join(name);
    fs::write(&path, source).unwrap();
    let mut app = Workspace::launch(vec![Launch::Path(path, None)]).unwrap();
    app.opener = record;
    opened();
    draw(&mut app);
    (directory, app)
}

fn caret(app: &mut Workspace, needle: &str, inside: usize) {
    let offset = app.editor().document.text().find(needle).unwrap() + inside;
    app.editor_mut().document.set_caret(offset).unwrap();
    draw(app);
}

/// The screen cell where `needle` is drawn, in the document area.
fn cell(buffer: &Buffer, needle: &str) -> (u16, u16) {
    for y in 0..23 {
        let row: Vec<&str> = (0..100).map(|x| buffer[(x, y)].symbol()).collect();
        let line: String = row.concat();
        if let Some(at) = line.find(needle) {
            // Every cell before it holds one character here.
            return (line[..at].chars().count() as u16, y);
        }
    }
    let screen: Vec<String> = (0..24)
        .map(|y| {
            (0..100)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect();
    panic!("{needle:?} is not on screen:\n{}", screen.join("\n"));
}

fn click(app: &mut Workspace, (column, row): (u16, u16), modifiers: KeyModifiers) {
    for kind in [
        MouseEventKind::Down(MouseButton::Left),
        MouseEventKind::Up(MouseButton::Left),
    ] {
        app.handle_event(Event::Mouse(MouseEvent {
            kind,
            column,
            row,
            modifiers,
        }));
    }
}

const NOTES: &str = "# Notes\n\nSee [the site](https://example.com/docs?q=1&r=2#part) and [a reference][ref], or <https://auto.example/path>.\n\nWrite to [me](mailto:me@example.com?subject=Hi) or visit https://bare.example/page, soon.\n\n[![CI](https://ci.example/badge.svg)](https://ci.example/runs) and ![shot](https://img.example/shot.png)\n\nNever: [script](javascript:alert(1)) [local file](file:///etc/passwd) [ftp](ftp://example.com/x)\n\nPlain words here.\n\n[ref]: https://ref.example/target\n";

#[test]
fn the_command_hands_web_and_mail_links_to_the_opener_only_when_asked() {
    let (_directory, mut app) = workspace("notes.md", NOTES);
    // Drawing, moving through links, and clicking them open nothing.
    for (needle, inside) in [("the site", 2), ("auto.example", 3), ("bare.example", 1)] {
        caret(&mut app, needle, inside);
        key(&mut app, KeyCode::Right, KeyModifiers::NONE);
        key(&mut app, KeyCode::Up, KeyModifiers::NONE);
        draw(&mut app);
    }
    assert_eq!(opened(), Vec::<String>::new());
    // Alt+L follows the link under the caret, wherever in it the caret is.
    for (needle, inside, address) in [
        ("the site", 2, "https://example.com/docs?q=1&r=2#part"),
        ("[the site]", 0, "https://example.com/docs?q=1&r=2#part"),
        (
            "example.com/docs",
            4,
            "https://example.com/docs?q=1&r=2#part",
        ),
        ("a reference", 3, "https://ref.example/target"),
        ("auto.example", 3, "https://auto.example/path"),
        ("[me]", 1, "mailto:me@example.com?subject=Hi"),
        ("bare.example", 1, "https://bare.example/page"),
        // Just after an address still counts: the caret has only passed it.
        (", soon", 0, "https://bare.example/page"),
        // A badge follows its link, not its picture.
        ("CI", 1, "https://ci.example/runs"),
        ("shot]", 1, "https://img.example/shot.png"),
    ] {
        caret(&mut app, needle, inside);
        follow(&mut app);
        assert_eq!(opened(), [address], "{needle}");
        let status = status(&mut app);
        assert!(status.contains("Opening "), "{needle}: {status}");
        assert_eq!(app.tabs.len(), 1);
    }
    // The palette lists the command with its key and runs the same thing.
    caret(&mut app, "the site", 2);
    key(&mut app, KeyCode::Char('e'), KeyModifiers::CONTROL);
    app.handle_event(Event::Paste("open link".into()));
    let listed = crate::simulation::snapshot(&mut app, 100, 24).unwrap();
    assert!(listed.contains("Open link under caret"), "{listed}");
    assert!(listed.contains("Alt+L"), "{listed}");
    key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(opened(), ["https://example.com/docs?q=1&r=2#part"]);
    // Away from any link the command says so and opens nothing.
    caret(&mut app, "Plain words", 3);
    follow(&mut app);
    assert!(status(&mut app).contains("No link under the caret"));
    // Other schemes are refused with the reason, whatever the label says.
    for (needle, reason) in [
        ("script", "javascript: links are not opened"),
        ("local file", "file: links are not opened"),
        ("ftp]", "ftp: links are not opened"),
    ] {
        caret(&mut app, needle, 1);
        follow(&mut app);
        let status = status(&mut app);
        assert!(status.contains(reason), "{needle}: {status}");
    }
    assert_eq!(opened(), Vec::<String>::new());
    assert_eq!(app.editor().document.text(), NOTES);
    assert!(!app.editor().document.is_dirty());
}

#[test]
fn addresses_written_out_in_code_and_plain_text_open_too() {
    let source = "// See https://docs.example/api (v2), or mailto:dev@example.com.\nlet url = \"http://localhost:8080/health\";\nlet not_a_link = 1;\n";
    let (_directory, mut app) = workspace("main.rs", source);
    for (needle, address) in [
        ("docs.example", "https://docs.example/api"),
        ("dev@", "mailto:dev@example.com"),
        ("localhost", "http://localhost:8080/health"),
    ] {
        caret(&mut app, needle, 2);
        follow(&mut app);
        assert_eq!(opened(), [address], "{needle}");
    }
    caret(&mut app, "not_a_link", 2);
    follow(&mut app);
    assert_eq!(opened(), Vec::<String>::new());
    assert!(status(&mut app).contains("No link under the caret"));
    // Where a Markdown link ends and an address begins, the caret is on the
    // address it is about to enter, not the link it has just left.
    let (_directory, mut app) = workspace(
        "joined.md",
        "x\n\n[first](https://first.example)https://second.example/x done\n",
    );
    caret(&mut app, "https://second", 0);
    follow(&mut app);
    assert_eq!(opened(), ["https://second.example/x"]);
    caret(&mut app, "https://second", 0);
    key(&mut app, KeyCode::Left, KeyModifiers::NONE);
    follow(&mut app);
    assert_eq!(opened(), ["https://first.example"]);
}

#[test]
fn ctrl_click_follows_the_link_drawn_under_the_pointer() {
    let (_directory, mut app) = workspace("notes.md", NOTES);
    // The caret rests at the end, so the links above it are rendered.
    let end = NOTES.len();
    app.editor_mut().document.set_caret(end).unwrap();
    let buffer = draw(&mut app);
    let site = cell(&buffer, "the site");
    // A plain click only places the caret, which shows the link's source.
    click(&mut app, (site.0 + 2, site.1), KeyModifiers::NONE);
    assert_eq!(opened(), Vec::<String>::new());
    assert_eq!(
        app.editor().document.selection().head,
        NOTES.find("the site").unwrap() + 2
    );
    app.editor_mut().document.set_caret(end).unwrap();
    let buffer = draw(&mut app);
    // Ctrl+click follows it and leaves the caret where it was.
    for (label, address) in [
        ("the site", "https://example.com/docs?q=1&r=2#part"),
        ("a reference", "https://ref.example/target"),
        ("https://auto.example/path", "https://auto.example/path"),
        ("https://bare.example/page", "https://bare.example/page"),
        ("CI", "https://ci.example/runs"),
        ("shot", "https://img.example/shot.png"),
    ] {
        let at = cell(&buffer, label);
        click(&mut app, (at.0 + 1, at.1), KeyModifiers::CONTROL);
        assert_eq!(opened(), [address], "{label}");
        assert_eq!(app.editor().document.selection().head, end, "{label}");
        assert!(status(&mut app).contains("Opening "), "{label}");
    }
    // Only a cell holding the link's own text counts: not the comma after
    // an address, the blank past the end of its row, or ordinary words.
    let buffer = draw(&mut app);
    let bare = cell(&buffer, "https://bare.example/page");
    let after = bare.0 + "https://bare.example/page".len() as u16;
    click(&mut app, (after, bare.1), KeyModifiers::CONTROL);
    assert_eq!(opened(), Vec::<String>::new());
    app.editor_mut().document.set_caret(end).unwrap();
    let buffer = draw(&mut app);
    let shot = cell(&buffer, "shot");
    click(&mut app, (shot.0 + 30, shot.1), KeyModifiers::CONTROL);
    let buffer = draw(&mut app);
    let words = cell(&buffer, "Plain words");
    click(&mut app, (words.0 + 2, words.1), KeyModifiers::CONTROL);
    assert_eq!(opened(), Vec::<String>::new());
    // There, Ctrl+click is an ordinary click.
    assert_eq!(
        app.editor().document.selection().head,
        NOTES.find("Plain words").unwrap() + 2
    );
    // A refused link is refused from the pointer as well.
    app.editor_mut().document.set_caret(end).unwrap();
    let buffer = draw(&mut app);
    let script = cell(&buffer, "script");
    click(&mut app, (script.0 + 1, script.1), KeyModifiers::CONTROL);
    assert_eq!(opened(), Vec::<String>::new());
    assert!(status(&mut app).contains("javascript: links are not opened"));
    // In source view the click works on the link's source text.
    key(&mut app, KeyCode::Home, KeyModifiers::CONTROL);
    key(&mut app, KeyCode::Char('r'), KeyModifiers::CONTROL);
    let buffer = draw(&mut app);
    let source = cell(&buffer, "[the site]");
    click(&mut app, (source.0 + 3, source.1), KeyModifiers::CONTROL);
    assert_eq!(opened(), ["https://example.com/docs?q=1&r=2#part"]);
    assert_eq!(app.editor().document.text(), NOTES);
}

#[test]
fn links_to_local_files_open_tabs_and_fragments_find_headings() {
    let source = "# Index\n\n[other](other.md) [part](other.md#second-part) [again](./other.md#second-part-1) [spaced](my%20notes.md)\n\n[folder](sub) [missing](missing.md) [picture](shot.png) [up](#index) [nowhere](#no-such-heading)\n\n## Deep Heading!\n\n[deep](#deep-heading)\n";
    let (directory, mut app) = workspace("index.md", source);
    let folder = directory.path();
    fs::write(
        folder.join("other.md"),
        "# Other\n\ntext\n\n## Second part\n\nfirst\n\n## Second part\n\nagain\n",
    )
    .unwrap();
    fs::write(folder.join("my notes.md"), "spaced out\n").unwrap();
    fs::create_dir(folder.join("sub")).unwrap();
    fs::write(folder.join("sub/inner.md"), "inner\n").unwrap();
    fs::write(folder.join("shot.png"), b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR").unwrap();
    let other = fs::read_to_string(folder.join("other.md")).unwrap();

    // A relative link opens beside the document, in a tab of its own.
    caret(&mut app, "[other]", 2);
    follow(&mut app);
    draw(&mut app);
    assert_eq!((app.tabs.len(), app.active), (2, 1));
    assert_eq!(app.editor().document.text(), other);
    assert_eq!(app.editor().path(), Some(folder.join("other.md").as_path()));
    // Following it again, or with a fragment, uses the tab already open
    // and goes to the heading the fragment names.
    key(&mut app, KeyCode::Char(','), KeyModifiers::ALT);
    caret(&mut app, "[part]", 2);
    follow(&mut app);
    draw(&mut app);
    assert_eq!((app.tabs.len(), app.active), (2, 1));
    assert_eq!(
        app.editor().document.selection().head,
        other.find("## Second part").unwrap()
    );
    key(&mut app, KeyCode::Char(','), KeyModifiers::ALT);
    caret(&mut app, "[again]", 2);
    follow(&mut app);
    draw(&mut app);
    assert_eq!(
        app.editor().document.selection().head,
        other.rfind("## Second part").unwrap()
    );
    // Percent-encoded names decode, as a browser reads them.
    key(&mut app, KeyCode::Char(','), KeyModifiers::ALT);
    caret(&mut app, "[spaced]", 2);
    follow(&mut app);
    draw(&mut app);
    assert_eq!((app.tabs.len(), app.active), (3, 2));
    assert_eq!(app.editor().document.text(), "spaced out\n");
    key(&mut app, KeyCode::Char('.'), KeyModifiers::ALT);
    assert_eq!(app.active, 0);

    // A fragment alone goes to a heading of this document.
    caret(&mut app, "[deep]", 2);
    follow(&mut app);
    draw(&mut app);
    assert_eq!(
        app.editor().document.selection().head,
        source.find("## Deep Heading!").unwrap()
    );
    caret(&mut app, "[up]", 1);
    follow(&mut app);
    draw(&mut app);
    assert_eq!(app.editor().document.selection().head, 0);
    caret(&mut app, "[nowhere]", 2);
    follow(&mut app);
    assert!(status(&mut app).contains("No heading here matches #no-such-heading"));

    // A missing file is never created, and a file that is not text is not
    // opened.
    caret(&mut app, "[missing]", 2);
    follow(&mut app);
    let status_row = status(&mut app);
    assert!(
        status_row.contains("The linked file does not exist"),
        "{status_row}"
    );
    assert!(!folder.join("missing.md").exists());
    caret(&mut app, "[picture]", 2);
    follow(&mut app);
    let status_row = status(&mut app);
    assert!(
        status_row.contains("Cannot open the linked file"),
        "{status_row}"
    );
    assert_eq!((app.tabs.len(), app.active), (3, 0));
    // A folder opens the file browser there.
    caret(&mut app, "[folder]", 2);
    follow(&mut app);
    assert!(app.browser.is_some());
    let listing = crate::simulation::snapshot(&mut app, 100, 24).unwrap();
    assert!(listing.contains("inner.md"), "{listing}");
    key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    assert!(app.browser.is_none());

    assert_eq!(
        opened(),
        Vec::<String>::new(),
        "no local link reaches the opener"
    );
    assert_eq!(app.tabs[0].editor.document.text(), source);
    assert_eq!(fs::read_to_string(folder.join("other.md")).unwrap(), other);
}

#[cfg(unix)]
#[test]
fn links_never_read_anything_but_regular_files() {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let source = "[pipe](pipe) [device](/dev/null) [loop](loop.md)\n";
    let (directory, mut app) = workspace("index.md", source);
    let pipe = directory.path().join("pipe");
    let name = CString::new(pipe.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    std::os::unix::fs::symlink("loop.md", directory.path().join("loop.md")).unwrap();
    // A pipe would block forever if it were opened for reading.
    for needle in ["[pipe]", "[device]", "[loop]"] {
        caret(&mut app, needle, 2);
        follow(&mut app);
        let status_row = status(&mut app);
        assert!(
            status_row.contains("The link does not lead to a file")
                || status_row.contains("Cannot open the linked file"),
            "{needle}: {status_row}"
        );
        assert_eq!(app.tabs.len(), 1, "{needle}");
    }
    assert_eq!(opened(), Vec::<String>::new());
}

#[test]
fn the_status_line_says_where_the_link_under_the_caret_leads() {
    let long = format!("https://example.com/{}", "segment/".repeat(20));
    let source = format!(
        "# Notes\n\nSee [the site](https://example.com/docs) and [a reference][ref] and [far]({long}).\n\nPlain words here.\n\n[ref]: https://ref.example/target\n"
    );
    let (_directory, mut app) = workspace("notes.md", &source);
    let plain = status(&mut app);
    assert!(
        plain.contains("Ctrl+G help") && !plain.contains('→'),
        "{plain}"
    );
    caret(&mut app, "the site", 2);
    let row = status(&mut app);
    assert!(row.contains("→ https://example.com/docs"), "{row}");
    assert!(row.contains("Alt+L opens"), "{row}");
    assert!(row.contains("notes.md") && row.contains("Ln 3"), "{row}");
    // A reference link's destination is somewhere else in the file.
    caret(&mut app, "a reference", 2);
    let row = status(&mut app);
    assert!(row.contains("→ https://ref.example/target"), "{row}");
    // A long destination is cut short, never run over the document's name.
    caret(&mut app, "[far]", 2);
    let row = status(&mut app);
    assert!(
        row.contains("→ https://example.com/segment/") && row.contains('…'),
        "{row}"
    );
    assert!(
        row.contains("notes.md") && row.trim_end().ends_with("Alt+L opens"),
        "{row}"
    );
    // Off the link the usual status returns, and a notice still wins.
    caret(&mut app, "Plain words", 2);
    assert_eq!(
        status(&mut app),
        plain.replace("Ln 1, Col 1", "Ln 5, Col 3")
    );
    caret(&mut app, "the site", 2);
    follow(&mut app);
    let row = status(&mut app);
    assert!(
        row.contains("Opening https://example.com/docs") && !row.contains('→'),
        "{row}"
    );
    assert_eq!(opened(), ["https://example.com/docs"]);
    // The key named is the one that works now.
    let (bindings, problems) = bindings::Bindings::parse("alt+o = open-link\n");
    assert!(problems.is_empty(), "{problems:?}");
    app.bindings = bindings;
    key(&mut app, KeyCode::Left, KeyModifiers::NONE);
    let row = status(&mut app);
    assert!(
        row.contains("Alt+O opens") && !row.contains("Alt+L"),
        "{row}"
    );
    key(&mut app, KeyCode::Char('o'), KeyModifiers::ALT);
    assert_eq!(opened(), ["https://example.com/docs"]);
    // Destinations never put control characters on screen.
    let hostile = "[x](<https://example.com/\u{1b}[31mred>)\n";
    let (_directory, mut app) = workspace("hostile.md", hostile);
    caret(&mut app, "[x]", 1);
    let row = status(&mut app);
    assert!(!row.contains('\u{1b}'), "{row:?}");
    follow(&mut app);
    assert_eq!(opened(), Vec::<String>::new());
    assert!(status(&mut app).contains("control characters"));
}

#[test]
fn a_failing_opener_is_reported_and_changes_nothing() {
    let (_directory, mut app) = workspace("notes.md", NOTES);
    app.opener = |_| Err(io::Error::other("xdg-open is not installed"));
    caret(&mut app, "the site", 2);
    follow(&mut app);
    let row = status(&mut app);
    assert!(
        row.contains("Could not open the link: xdg-open is not installed"),
        "{row}"
    );
    assert_eq!(app.editor().document.text(), NOTES);
    // Without a test's own opener, tests can never start a real one.
    let fresh = Workspace::open(None).unwrap();
    assert!((fresh.opener)("https://example.com").is_err());
}
