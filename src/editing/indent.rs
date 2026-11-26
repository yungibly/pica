//! What one level of indentation is. Tab, Shift+Tab and Backspace use it,
//! so a file indented with spaces never gains a tab, nor the reverse.
use unicode_width::UnicodeWidthChar;

use super::{general::content_start, line_range};
use crate::Document;

thread_local! {
    static TAB_WIDTH: std::cell::Cell<usize> = const { std::cell::Cell::new(4) };
}

/// The column step of a tab character, here and in every view: four unless
/// another width is chosen.
pub fn tab_width() -> usize {
    TAB_WIDTH.get()
}

/// Choose the column step of a tab character, from one to eight.
pub fn set_tab_width(width: usize) {
    TAB_WIDTH.set(width.clamp(1, 8));
}

/// Detection reads this many lines; a file settles its style long before.
const SCAN_LINES: usize = 2000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Indent {
    Tabs,
    /// One to eight spaces.
    Spaces(usize),
}

impl Indent {
    pub fn spaces(width: usize) -> Self {
        Self::Spaces(width.clamp(1, 8))
    }

    /// Columns one level spans.
    pub fn width(self) -> usize {
        match self {
            Self::Tabs => tab_width(),
            Self::Spaces(width) => width,
        }
    }

    /// The text of one level.
    pub fn unit(self) -> String {
        match self {
            Self::Tabs => "\t".to_owned(),
            Self::Spaces(width) => " ".repeat(width),
        }
    }

    /// The style `text` already uses, or `None` when nothing in it is
    /// indented. Tabs win when more lines start with one than with spaces.
    /// Otherwise the step is the most common increase from one line to the
    /// next, which hanging and aligned continuation lines rarely outvote;
    /// a tie takes the smaller step. One-column steps are comment
    /// continuations (` * text`) and are not counted, nor is the line after
    /// one ending in a comma or a backslash: that is an argument or a
    /// command carried on, aligned wherever its first half put it.
    pub fn detect(text: &str) -> Option<Self> {
        let (mut tabbed, mut spaced) = (0usize, 0usize);
        let mut steps = [0usize; 9];
        let mut previous = 0;
        let mut carried = false;
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        for line in text.split(['\n', '\r']).take(SCAN_LINES) {
            let content = line.trim_start_matches([' ', '\t']);
            if content.is_empty() {
                continue;
            }
            let continues =
                std::mem::replace(&mut carried, content.trim_end().ends_with([',', '\\']));
            if line.starts_with('\t') {
                tabbed += 1;
                previous = 0;
                continue;
            }
            let width = line.len() - line.trim_start_matches(' ').len();
            if width > previous && (2..=8).contains(&(width - previous)) && !continues {
                steps[width - previous] += 1;
            }
            spaced += usize::from(width >= 2);
            previous = width;
        }
        if tabbed > spaced {
            return Some(Self::Tabs);
        }
        let best = (2..=8).max_by_key(|&step| (steps[step], std::cmp::Reverse(step)))?;
        if steps[best] > 0 {
            Some(Self::Spaces(best))
        } else {
            (tabbed > 0).then_some(Self::Tabs)
        }
    }
}

/// The display column of `offset` on the line whose text begins at `start`.
fn column(source: &str, start: usize, offset: usize) -> usize {
    let tab = tab_width();
    source[start..offset].chars().fold(0, |column, c| match c {
        '\t' => column + tab - column % tab,
        c => column + c.width().unwrap_or(0),
    })
}

/// Tab at a caret, replacing any selection: a tab character, or the spaces
/// that reach the next indent stop.
pub fn insert_tab(document: &mut Document, indent: Indent) -> bool {
    let source = document.text();
    // A caret before a byte order mark types after it; the mark stays first.
    let floor = crate::markdown::bom_len(source);
    let range = document.selection().range();
    let range = range.start.max(floor)..range.end.max(floor);
    let text = match indent {
        Indent::Tabs => "\t".to_owned(),
        Indent::Spaces(width) => {
            let start = content_start(source, line_range(source, range.start).start);
            let column = column(source, start.min(range.start), range.start);
            " ".repeat(width - column % width)
        }
    };
    document
        .replace_range(range, &text)
        .expect("selection boundaries")
}

/// Backspace. With space indentation, a caret in a line's leading spaces
/// removes back to the previous indent stop, undoing what Tab inserted.
pub fn backspace(document: &mut Document, indent: Indent) -> bool {
    let selection = document.selection();
    if let (Indent::Spaces(width), true) = (indent, selection.is_empty()) {
        let source = document.text();
        let head = selection.head;
        let start = content_start(source, line_range(source, head).start);
        if start < head && source[start..head].bytes().all(|byte| byte == b' ') {
            let removed = (head - start - 1) % width + 1;
            return document
                .replace_range(head - removed..head, "")
                .expect("ASCII spaces");
        }
    }
    document.backspace()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detection_follows_the_files_own_lines() {
        for (text, expected) in [
            ("", None),
            ("plain\ntext\n", None),
            (
                "def f():\n    x = 1\n    if x:\n        y = 2\n",
                Some(Indent::Spaces(4)),
            ),
            ("a:\n  b:\n    c: 1\n  d: 2\n", Some(Indent::Spaces(2))),
            (
                "all:\n\tcc main.c\n\nclean:\n\trm -f a.out\n",
                Some(Indent::Tabs),
            ),
            ("\u{feff}a {\r\n   b;\r\n}\r\n", Some(Indent::Spaces(3))),
            // A doc comment's one-column continuation is not a step.
            (
                "/**\n * text\n */\nint main() {\n    return 0;\n}\n",
                Some(Indent::Spaces(4)),
            ),
            // Aligned continuation lines lose to the common step.
            (
                "f(a,\n  b)\nif x:\n    y\n    if z:\n        w\nwhile q:\n    r\n",
                Some(Indent::Spaces(4)),
            ),
            // A lone continuation line is not a level: the file's type decides.
            ("def f(a,\n      b):\n    return a\n", None),
            ("cc -c main.c \\\n   -o main.o\nall:\n", None),
            // Nor does it outvote the lines that are.
            ("call(a,\n     b)\nif x:\n    y\n", Some(Indent::Spaces(4))),
            // A list's items end in commas at one level and still count
            // where the list opens.
            ("x = [\n  1,\n  2,\n]\n", Some(Indent::Spaces(2))),
            // A tie between steps takes the smaller: four can be two levels of two.
            ("a\n  b\nc\n    d\n", Some(Indent::Spaces(2))),
            // Mostly tabs, with one space-aligned line.
            ("a {\n\tb\n\tc\n  d\n}\n", Some(Indent::Tabs)),
            // Mostly spaces, with one stray tab.
            ("a {\n  b\n  c\n\td\n}\n", Some(Indent::Spaces(2))),
            // Only whitespace-only lines are indented: no evidence.
            ("a\n    \nb\n", None),
        ] {
            assert_eq!(Indent::detect(text), expected, "{text:?}");
        }
    }

    #[test]
    fn detection_reads_a_bounded_prefix() {
        let mut text = "x\n".repeat(SCAN_LINES);
        text.push_str("a\n  b\n");
        assert_eq!(Indent::detect(&text), None);
    }

    #[test]
    fn tab_inserts_one_unit_up_to_the_next_stop() {
        for (text, caret, indent, expected) in [
            ("", 0, Indent::Tabs, "\t"),
            ("", 0, Indent::Spaces(4), "    "),
            ("ab", 2, Indent::Spaces(4), "ab  "),
            ("abcd", 4, Indent::Spaces(4), "abcd    "),
            ("  x", 2, Indent::Spaces(2), "    x"),
            ("   x", 3, Indent::Spaces(2), "    x"),
            // A wide character spans two columns; a tab reaches column four.
            ("界", 3, Indent::Spaces(4), "界  "),
            ("\ta", 2, Indent::Spaces(4), "\ta   "),
            ("one\ntwo", 7, Indent::Spaces(4), "one\ntwo "),
            ("ab", 2, Indent::Tabs, "ab\t"),
        ] {
            let mut document = Document::new(text);
            document.set_caret(caret).unwrap();
            assert!(insert_tab(&mut document, indent));
            assert_eq!(document.text(), expected, "{text:?} {indent:?}");
            assert!(document.selection().is_empty());
            assert_eq!(
                document.selection().head,
                caret + expected.len() - text.len()
            );
            document.undo();
            assert_eq!(document.text(), text);
        }
    }

    #[test]
    fn a_chosen_tab_width_moves_the_stops_tabs_reach() {
        assert_eq!(tab_width(), 4);
        for (width, after_tab, level) in
            [(2, "\ta     ", 2), (8, "\ta       ", 8), (4, "\ta   ", 4)]
        {
            set_tab_width(width);
            assert_eq!(Indent::Tabs.width(), level);
            // After a tab and a letter, spaces reach the next eight-column stop.
            let mut document = Document::new("\ta");
            document.set_caret(2).unwrap();
            insert_tab(&mut document, Indent::Spaces(8));
            assert_eq!(document.text(), after_tab, "{width}");
        }
        // Widths outside one to eight are brought into range.
        set_tab_width(0);
        assert_eq!(tab_width(), 1);
        set_tab_width(99);
        assert_eq!(tab_width(), 8);
        set_tab_width(4);
    }

    #[test]
    fn tab_replaces_a_selection_and_stays_after_a_byte_order_mark() {
        let mut document = Document::new("abcdef");
        document
            .set_selection(crate::Selection { anchor: 1, head: 3 })
            .unwrap();
        assert!(insert_tab(&mut document, Indent::Spaces(4)));
        assert_eq!(document.text(), "a   def");
        let mut document = Document::new("\u{feff}x");
        document.set_caret(0).unwrap();
        assert!(insert_tab(&mut document, Indent::Spaces(2)));
        assert_eq!(document.text(), "\u{feff}  x");
    }

    #[test]
    fn backspace_in_leading_spaces_returns_to_the_previous_stop() {
        for (text, caret, indent, expected) in [
            ("        x", 8, Indent::Spaces(4), "    x"),
            ("      x", 6, Indent::Spaces(4), "    x"),
            ("    x", 4, Indent::Spaces(4), "x"),
            ("    x", 2, Indent::Spaces(4), "  x"),
            ("  x", 2, Indent::Spaces(2), "x"),
            ("\u{feff}    x", 7, Indent::Spaces(4), "\u{feff}x"),
            ("a\n    x", 6, Indent::Spaces(4), "a\nx"),
            // Past the indentation, with tabs, or after a tab: one character.
            ("    x", 5, Indent::Spaces(4), "    "),
            ("a   b", 4, Indent::Spaces(4), "a  b"),
            ("        x", 8, Indent::Tabs, "       x"),
            ("\t  x", 3, Indent::Spaces(4), "\t x"),
            ("\t\tx", 2, Indent::Tabs, "\tx"),
        ] {
            let mut document = Document::new(text);
            document.set_caret(caret).unwrap();
            assert!(backspace(&mut document, indent));
            assert_eq!(document.text(), expected, "{text:?} {indent:?}");
            document.undo();
            assert_eq!(document.text(), text);
        }
        let mut document = Document::new("    x");
        document.set_caret(0).unwrap();
        assert!(!backspace(&mut document, Indent::Spaces(4)));
        document.select_all();
        assert!(backspace(&mut document, Indent::Spaces(4)));
        assert_eq!(document.text(), "");
    }
}
