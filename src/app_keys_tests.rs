//! The micro-aligned keymap and everyday editing, through the real key path.
use super::*;
use crate::clipboard::tests::{Fake, State};
use ratatui::{Terminal, backend::TestBackend};
use std::sync::{Arc, Mutex};

fn key(app: &mut App, code: KeyCode, modifiers: KeyModifiers) {
    app.handle_event(Event::Key(KeyEvent::new(code, modifiers)));
}

fn ctrl(app: &mut App, c: char) {
    key(app, KeyCode::Char(c), KeyModifiers::CONTROL);
}

fn alt(app: &mut App, c: char) {
    key(app, KeyCode::Char(c), KeyModifiers::ALT);
}

fn draw(app: &mut App, width: u16, height: u16) -> Terminal<TestBackend> {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    terminal
}

/// A code or text buffer named like `name`, with the caret at `caret`.
fn code(name: &str, source: &str, caret: usize) -> App {
    let mut app = App::new(source.into(), None, false);
    app.language = Language::for_path(std::path::Path::new(name));
    app.document.set_caret(caret).unwrap();
    app
}

fn markdown(source: &str, caret: usize) -> App {
    let mut app = App::new(source.into(), None, true);
    app.document.set_caret(caret).unwrap();
    app
}

fn clipboard(app: &mut App) -> Arc<Mutex<State>> {
    let state = Arc::new(Mutex::new(State::default()));
    app.clipboard = Clipboard::with_backend(Box::new(Fake(state.clone())));
    state
}

fn enter(app: &mut App) {
    key(app, KeyCode::Enter, KeyModifiers::NONE);
}

#[test]
fn enter_auto_indents_code_with_tabs_spaces_and_crlf() {
    let mut app = code("main.rs", "fn main() {\r\n\tif ok {", 21);
    enter(&mut app);
    assert_eq!(app.document.text(), "fn main() {\r\n\tif ok {\r\n\t");
    let mut app = code("tool.py", "def run():\n    return 1", 23);
    enter(&mut app);
    assert_eq!(app.document.text(), "def run():\n    return 1\n    ");
    // Leaving a whitespace-only line clears it, as micro does.
    enter(&mut app);
    assert_eq!(app.document.text(), "def run():\n    return 1\n\n    ");
    ctrl(&mut app, 'z');
    assert_eq!(app.document.text(), "def run():\n    return 1\n    ");
    // Plain text files indent too; Alt+Enter is a plain newline.
    let mut app = code("notes.txt", "  - item", 8);
    enter(&mut app);
    assert_eq!(app.document.text(), "  - item\n  ");
    let mut app = code("notes.txt", "  item", 6);
    key(&mut app, KeyCode::Enter, KeyModifiers::ALT);
    assert_eq!(app.document.text(), "  item\n");
}

#[test]
fn enter_auto_indents_markdown_fences_and_still_continues_lists() {
    let source = "```rust\nfn main() {\n    run();";
    let mut app = markdown(source, source.len());
    enter(&mut app);
    assert_eq!(app.document.text(), format!("{source}\n    "));
    let mut app = markdown("  - item", 8);
    enter(&mut app);
    assert_eq!(app.document.text(), "  - item\n  - ");
    let mut app = markdown("- [x] done", 10);
    enter(&mut app);
    assert_eq!(app.document.text(), "- [x] done\n- [ ] ");
    let mut app = markdown("   indented prose", 17);
    enter(&mut app);
    assert_eq!(app.document.text(), "   indented prose\n   ");
}

#[test]
fn smart_home_alternates_and_shift_home_extends() {
    let mut app = code("main.rs", "fn a() {\n    body();\n}", 17);
    draw(&mut app, 60, 10);
    key(&mut app, KeyCode::Home, KeyModifiers::NONE);
    assert_eq!(app.document.selection(), Selection::caret(13));
    key(&mut app, KeyCode::Home, KeyModifiers::NONE);
    assert_eq!(app.document.selection(), Selection::caret(9));
    key(&mut app, KeyCode::Home, KeyModifiers::NONE);
    assert_eq!(app.document.selection(), Selection::caret(13));
    app.document.set_caret(19).unwrap();
    key(&mut app, KeyCode::Home, KeyModifiers::SHIFT);
    assert_eq!(
        app.document.selection(),
        Selection {
            anchor: 19,
            head: 13
        }
    );
    key(&mut app, KeyCode::Home, KeyModifiers::SHIFT);
    assert_eq!(
        app.document.selection(),
        Selection {
            anchor: 19,
            head: 9
        }
    );
    // Live Markdown works on source lines too.
    let mut app = markdown("intro\n  - item", 14);
    draw(&mut app, 60, 10);
    key(&mut app, KeyCode::Home, KeyModifiers::NONE);
    assert_eq!(app.document.selection(), Selection::caret(8));
    assert_eq!(app.document.revision(), 0);
}

#[test]
fn ctrl_k_cuts_lines_and_consecutive_cuts_collect() {
    let mut app = code("list.txt", "one\ntwo\nthree\nfour\n", 0);
    let state = clipboard(&mut app);
    ctrl(&mut app, 'k');
    ctrl(&mut app, 'k');
    assert_eq!(app.document.text(), "three\nfour\n");
    assert_eq!(state.lock().unwrap().text, "one\ntwo\n");
    assert!(app.message.contains("Cut 2 lines"), "{}", app.message);
    // Releases and redraws do not end a run of cuts.
    app.handle_event(Event::Key(KeyEvent::new_with_kind(
        KeyCode::Char('k'),
        KeyModifiers::CONTROL,
        KeyEventKind::Release,
    )));
    app.handle_event(Event::Resize(60, 10));
    ctrl(&mut app, 'k');
    assert_eq!(state.lock().unwrap().text, "one\ntwo\nthree\n");
    // Any other key starts a fresh clipboard entry.
    key(&mut app, KeyCode::Down, KeyModifiers::NONE);
    key(&mut app, KeyCode::Up, KeyModifiers::NONE);
    ctrl(&mut app, 'k');
    assert_eq!(state.lock().unwrap().text, "four\n");
    assert_eq!(app.document.text(), "");
    // Each cut is its own undo step.
    for expected in [
        "four\n",
        "three\nfour\n",
        "two\nthree\nfour\n",
        "one\ntwo\nthree\nfour\n",
    ] {
        ctrl(&mut app, 'z');
        assert_eq!(app.document.text(), expected);
    }
    ctrl(&mut app, 'v');
    assert!(app.document.text().starts_with("four\n"));
}

#[test]
fn ctrl_k_cuts_selected_lines_and_stops_at_the_empty_end() {
    let mut app = code("a.txt", "a\nb\nc", 0);
    let state = clipboard(&mut app);
    app.document
        .set_selection(Selection { anchor: 3, head: 0 })
        .unwrap();
    ctrl(&mut app, 'k');
    assert_eq!(app.document.text(), "c");
    assert_eq!(state.lock().unwrap().text, "a\nb\n");
    ctrl(&mut app, 'k');
    assert_eq!(app.document.text(), "");
    assert_eq!(state.lock().unwrap().text, "a\nb\nc");
    let revision = app.document.revision();
    ctrl(&mut app, 'k');
    assert_eq!(app.document.revision(), revision);
    assert_eq!(state.lock().unwrap().text, "a\nb\nc");
}

#[test]
fn ctrl_d_duplicates_lines_or_selection() {
    let mut app = code("main.rs", "let a = 1;\nlet b = 2;", 3);
    ctrl(&mut app, 'd');
    assert_eq!(app.document.text(), "let a = 1;\nlet a = 1;\nlet b = 2;");
    app.document
        .set_selection(Selection { anchor: 0, head: 3 })
        .unwrap();
    ctrl(&mut app, 'd');
    assert_eq!(app.document.text(), "letlet a = 1;\nlet a = 1;\nlet b = 2;");
    assert_eq!(app.document.selection(), Selection { anchor: 3, head: 6 });
    ctrl(&mut app, 'z');
    ctrl(&mut app, 'z');
    assert_eq!(app.document.text(), "let a = 1;\nlet b = 2;");
}

#[test]
fn comment_keys_include_legacy_ctrl_slash_bytes_and_alt_slash() {
    let events = [
        KeyEvent::new(KeyCode::Char('/'), KeyModifiers::CONTROL),
        // Crossterm reads the legacy 0x1F byte as Ctrl+7.
        KeyEvent::new(KeyCode::Char('7'), KeyModifiers::CONTROL),
        KeyEvent::new(KeyCode::Char('_'), KeyModifiers::CONTROL),
        KeyEvent::new(KeyCode::Char('/'), KeyModifiers::ALT),
    ];
    for event in events {
        let mut app = code("main.rs", "    let x = 1;", 6);
        app.handle_event(Event::Key(event));
        assert_eq!(app.document.text(), "    // let x = 1;", "{event:?}");
        app.handle_event(Event::Key(event));
        assert_eq!(app.document.text(), "    let x = 1;", "{event:?}");
    }
}

#[test]
fn comments_follow_each_language() {
    for (name, source, expected) in [
        ("tool.py", "x = 1", "# x = 1"),
        ("query.sql", "select 1", "-- select 1"),
        ("init.lua", "x()", "-- x()"),
        ("page.html", "<p>", "<!-- <p> -->"),
        ("site.css", "a {}", "/* a {} */"),
        ("conf.yaml", "a: 1", "# a: 1"),
        ("Makefile", "all:", "# all:"),
        ("config", "key value", "# key value"),
        ("data.json", "1", "// 1"),
    ] {
        let mut app = code(name, source, 0);
        ctrl(&mut app, '/');
        assert_eq!(app.document.text(), expected, "{name}");
    }
    let mut app = code("change.diff", "+added", 0);
    ctrl(&mut app, '/');
    assert_eq!(app.document.text(), "+added");
    assert!(app.message.contains("No comment syntax"));
}

#[test]
fn markdown_comments_use_html_except_in_fences_and_front_matter() {
    let mut app = markdown("# Title\nprose", 9);
    ctrl(&mut app, '/');
    assert_eq!(app.document.text(), "# Title\n<!-- prose -->");
    let source = "intro\n\n```python\nx = 1\n```\n";
    let mut app = markdown(source, source.find("x =").unwrap());
    ctrl(&mut app, '/');
    assert_eq!(app.document.text(), "intro\n\n```python\n# x = 1\n```\n");
    let source = "---\ntitle: x\n---\nbody\n";
    let mut app = markdown(source, 5);
    ctrl(&mut app, '/');
    assert_eq!(app.document.text(), "---\n# title: x\n---\nbody\n");
    let source = "```\nplain\n```\n";
    let mut app = markdown(source, 5);
    ctrl(&mut app, '/');
    assert_eq!(app.document.text(), source);
}

#[test]
fn comment_blocks_are_one_undo_step() {
    let source = "a = 1\n  b = 2\n";
    let mut app = code("x.py", source, 0);
    app.document
        .set_selection(Selection {
            anchor: 0,
            head: source.len(),
        })
        .unwrap();
    ctrl(&mut app, '/');
    assert_eq!(app.document.text(), "# a = 1\n#   b = 2\n");
    ctrl(&mut app, 'z');
    assert_eq!(app.document.text(), source);
    assert!(!app.document.can_undo());
}

#[test]
fn ctrl_r_toggles_line_numbers_in_code_and_the_view_in_markdown() {
    let mut app = code("main.rs", "fn main() {}\n", 0);
    draw(&mut app, 60, 10);
    assert!(app.gutter_digits(60) > 0);
    let numbered = app.viewport.x;
    ctrl(&mut app, 'r');
    assert!(!app.ruler);
    assert_eq!(app.gutter_digits(60), 0);
    draw(&mut app, 60, 10);
    assert!(app.viewport.x < numbered);
    ctrl(&mut app, 'r');
    assert!(app.ruler);
    let mut app = markdown("# Notes\n", 0);
    ctrl(&mut app, 'r');
    assert!(!app.live);
    assert!(app.ruler);
    assert!(app.gutter_digits(60) > 0);
    ctrl(&mut app, 'r');
    assert!(app.live);
    // Inside Find, Ctrl+R still expands to Replace.
    ctrl(&mut app, 'f');
    ctrl(&mut app, 'r');
    assert_eq!(app.overlay, Overlay::Search { replace: true });
    assert!(app.live);
    assert_eq!(app.document.revision(), 0);
}

#[test]
fn find_next_and_previous_reuse_the_last_query_after_find_closes() {
    let mut app = code("a.txt", "cat dog cat dog cat", 0);
    // Without any query, Ctrl+N opens Find.
    ctrl(&mut app, 'n');
    assert_eq!(app.overlay, Overlay::Search { replace: false });
    app.handle_event(Event::Paste("cat".into()));
    assert_eq!(app.document.selection(), Selection { anchor: 0, head: 3 });
    key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    ctrl(&mut app, 'n');
    assert_eq!(app.overlay, Overlay::None);
    assert_eq!(
        app.document.selection(),
        Selection {
            anchor: 8,
            head: 11
        }
    );
    ctrl(&mut app, 'p');
    assert_eq!(app.document.selection(), Selection { anchor: 0, head: 3 });
    ctrl(&mut app, 'p');
    assert_eq!(
        app.document.selection(),
        Selection {
            anchor: 16,
            head: 19
        }
    );
    assert!(app.message.contains("wrapped"), "{}", app.message);
    // Switching tabs clears Find, but the query survives for Ctrl+N.
    app.deactivate();
    app.document.set_caret(1).unwrap();
    ctrl(&mut app, 'n');
    assert_eq!(
        app.document.selection(),
        Selection {
            anchor: 8,
            head: 11
        }
    );
    // Inside Find, Ctrl+N and Ctrl+P step through matches too.
    ctrl(&mut app, 'f');
    ctrl(&mut app, 'n');
    assert_eq!(
        app.document.selection(),
        Selection {
            anchor: 16,
            head: 19
        }
    );
    ctrl(&mut app, 'p');
    assert_eq!(
        app.document.selection(),
        Selection {
            anchor: 8,
            head: 11
        }
    );
    assert_eq!(app.document.revision(), 0);
    let mut app = code("a.txt", "nothing here", 0);
    app.last_query = "absent".into();
    ctrl(&mut app, 'n');
    assert!(app.message.contains("No matches"), "{}", app.message);
}

#[test]
fn help_find_and_task_keys_follow_the_micro_aligned_map() {
    let mut app = markdown("- [ ] task", 6);
    ctrl(&mut app, 'g');
    assert_eq!(app.overlay, Overlay::Help);
    ctrl(&mut app, 'g');
    assert_eq!(app.overlay, Overlay::None);
    // Ctrl+W is nano's "Where Is": a silent alias for Find.
    ctrl(&mut app, 'w');
    assert_eq!(app.overlay, Overlay::Search { replace: false });
    key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    // Ctrl+T no longer toggles tasks; Alt+T does.
    ctrl(&mut app, 't');
    assert_eq!(app.document.text(), "- [ ] task");
    alt(&mut app, 't');
    assert_eq!(app.document.text(), "- [x] task");
    // Bold is Markdown only.
    let mut app = code("main.rs", "word", 0);
    app.document.select_all();
    ctrl(&mut app, 'b');
    alt(&mut app, 't');
    assert_eq!(app.document.text(), "word");
    // Legacy terminals send Ctrl+] as Ctrl+5.
    ctrl(&mut app, '5');
    assert_eq!(app.document.text(), "    word");
}

#[test]
fn empty_document_hint_names_primary_keys_only() {
    let mut app = App::open(None).unwrap();
    let screen = crate::simulation::snapshot(&mut app, 80, 24).unwrap();
    for hint in ["Ctrl+E", "Ctrl+O", "Ctrl+G"] {
        assert!(screen.contains(hint), "{hint}: {screen}");
    }
    for alias in ["F1", "F2"] {
        assert!(!screen.contains(alias), "{alias}: {screen}");
    }
}
