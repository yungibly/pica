//! Everyday commands for every file type: auto-indent, smart Home, cutting,
//! duplicating and commenting whole source lines. Each edit is one undo step
//! through `Document::replace_with_selection`.
use std::ops::Range;

use unicode_segmentation::UnicodeSegmentation;

use super::{line_range, preferred_newline};
use crate::{Document, Selection, markdown};

/// The first editable byte of the line starting at `start`; a BOM stays put.
fn content_start(source: &str, start: usize) -> usize {
    if start == 0 {
        markdown::bom_len(source)
    } else {
        start
    }
}

fn indent_len(text: &str) -> usize {
    text.bytes()
        .take_while(|b| matches!(b, b' ' | b'\t'))
        .count()
}

fn ending_len(source: &str, at: usize) -> usize {
    let rest = &source.as_bytes()[at..];
    match rest {
        [b'\r', b'\n', ..] => 2,
        [b'\r' | b'\n', ..] => 1,
        _ => 0,
    }
}

/// Enter with auto-indent, as micro does: the new line repeats the current
/// line's leading spaces and tabs up to the caret, using the local line
/// ending. A line left holding only that whitespace is cleared, so no
/// trailing indentation stays behind. A selection is replaced.
pub fn indent_newline(document: &mut Document) -> bool {
    let source = document.text();
    // A caret before a byte order mark types after it; the mark stays first.
    let floor = markdown::bom_len(source);
    let range = document.selection().range();
    let range = range.start.max(floor)..range.end.max(floor);
    let line = line_range(source, range.start);
    let start = content_start(source, line.start).min(range.start);
    let before = &source[start..range.start];
    let indent = &before[..indent_len(before)];
    let newline = preferred_newline(source, range.start);
    let replaced = if !before.is_empty() && indent.len() == before.len() {
        start..range.end
    } else {
        range
    };
    let text = format!("{newline}{indent}");
    let caret = replaced.start + text.len();
    document
        .replace_with_selection(replaced, &text, Selection::caret(caret))
        .expect("line and selection boundaries")
}

/// Smart Home: the first non-blank character of the caret's source line, or
/// the start of the line when the caret is already there.
pub fn smart_home(source: &str, head: usize) -> usize {
    let line = line_range(source, head);
    let start = content_start(source, line.start);
    if start > line.end {
        return line.end;
    }
    let first = start + indent_len(&source[start..line.end]);
    if head == first { start } else { first }
}

/// Byte offset of a 1-based line and grapheme column. Both clamp: a line past
/// the end goes to the end of the document, a column past the end of its
/// line to the end of that line. CRLF counts as one line break.
pub fn line_column_offset(source: &str, line: usize, column: usize) -> usize {
    let bytes = source.as_bytes();
    let mut start = 0;
    let mut current = 1;
    let mut index = 0;
    while current < line.max(1) {
        match bytes[index..]
            .iter()
            .position(|b| matches!(b, b'\r' | b'\n'))
        {
            Some(found) => {
                index += found;
                index += ending_len(source, index);
                start = index;
                current += 1;
            }
            None => return source.len(),
        }
    }
    let start = content_start(source, start);
    let end = line_range(source, start).end;
    source[start..end]
        .grapheme_indices(true)
        .map(|(offset, _)| start + offset)
        .chain(std::iter::once(end))
        .nth(column.saturating_sub(1))
        .unwrap_or(end)
}

/// The whole source lines touched by a selection, including their endings.
/// A selection that ends at the start of a line leaves that line out.
fn touched_lines(source: &str, selection: Selection) -> Range<usize> {
    let range = selection.range();
    let first = line_range(source, range.start);
    let start = content_start(source, first.start);
    if !selection.is_empty() {
        let last = line_range(source, range.end);
        if last.start == range.end && range.end > start {
            return start..range.end;
        }
        return start..last.end + ending_len(source, last.end);
    }
    start..first.end + ending_len(source, first.end)
}

/// Cut the caret's source line, or every line the selection touches, with its
/// ending. Returns the removed text for the clipboard, or `None` when there is
/// nothing to cut (the empty last line). The caret lands where the lines were.
pub fn cut_lines(document: &mut Document) -> Option<String> {
    let source = document.text();
    let lines = touched_lines(source, document.selection());
    if lines.is_empty() {
        return None;
    }
    let removed = source[lines.clone()].to_owned();
    let caret = lines.start;
    document
        .replace_with_selection(lines, "", Selection::caret(caret))
        .expect("whole lines have grapheme boundaries");
    Some(removed)
}

/// Duplicate the selected text right after itself and select the copy, as
/// micro does; without a selection, duplicate the caret's line below.
pub fn duplicate(document: &mut Document) -> bool {
    let selection = document.selection();
    if selection.is_empty() {
        return document.duplicate_lines_down();
    }
    let range = selection.range();
    let copy = document.text()[range.clone()].to_owned();
    let end = range.end + copy.len();
    let after = if selection.anchor < selection.head {
        Selection {
            anchor: range.end,
            head: end,
        }
    } else {
        Selection {
            anchor: end,
            head: range.end,
        }
    };
    document
        .replace_with_selection(range.end..range.end, &copy, after)
        .expect("selection boundaries")
}

/// Toggle comments on the lines touched by the selection as one undo step.
/// `open` is a line-comment marker such as `//` or `#`; languages without one
/// pass a pair such as `<!--` and `-->`. If every non-blank line is already
/// commented, the markers (and one space beside each) are removed. Otherwise
/// every non-blank line is commented at the block's smallest indentation,
/// so the lines stay aligned. A lone blank line is commented too.
pub fn toggle_comment(document: &mut Document, open: &str, close: &str) -> bool {
    let selection = document.selection();
    let source = document.text();
    let span = touched_lines(source, selection);
    let mut lines = Vec::new();
    let mut at = span.start;
    loop {
        let line = line_range(source, at);
        lines.push(at..line.end);
        let next = line.end + ending_len(source, line.end);
        if next >= span.end || next == line.end {
            break;
        }
        at = next;
    }
    let blank = |line: &Range<usize>| source[line.clone()].trim().is_empty();
    let all_blank = lines.iter().all(blank);
    if all_blank && lines.len() > 1 {
        return false;
    }
    let targets: Vec<_> = lines
        .into_iter()
        .filter(|line| all_blank || !blank(line))
        .collect();
    let commented = |line: &Range<usize>| {
        let body = source[line.clone()]
            .trim_start_matches([' ', '\t'])
            .trim_end();
        body.starts_with(open)
            && (close.is_empty()
                || (body.len() >= open.len() + close.len() && body.ends_with(close)))
    };
    let mut edits: Vec<(Range<usize>, String)> = Vec::new();
    if !all_blank && targets.iter().all(commented) {
        for line in &targets {
            let text = &source[line.clone()];
            let marker = line.start + indent_len(text);
            let mut end = marker + open.len();
            if source[end..line.end].starts_with(' ') {
                end += 1;
            }
            edits.push((marker..end, String::new()));
            if !close.is_empty() {
                let trimmed = line.start + text.trim_end().len();
                let mut start = trimmed - close.len();
                if start > end && source[..start].ends_with(' ') {
                    start -= 1;
                }
                edits.push((start.max(end)..trimmed, String::new()));
            }
        }
    } else {
        let indent = targets
            .iter()
            .map(|line| indent_len(&source[line.clone()]))
            .min()
            .unwrap_or(0);
        for line in &targets {
            let at = line.start + indent.min(line.len());
            edits.push((at..at, format!("{open} ")));
            if !close.is_empty() {
                edits.push((line.end..line.end, format!(" {close}")));
            }
        }
    }
    apply_edits(document, edits)
}

/// Apply sorted, non-overlapping edits as one replacement and carry the
/// selection along. A selection's start stays before text inserted exactly
/// at it, so commenting whole selected lines keeps them selected.
fn apply_edits(document: &mut Document, edits: Vec<(Range<usize>, String)>) -> bool {
    let (Some(first), Some(last)) = (edits.first(), edits.last()) else {
        return false;
    };
    let span = first.0.start..last.0.end;
    let source = document.text();
    let selection = document.selection();
    let map = |offset: usize, before: bool| {
        let mut delta: isize = 0;
        for (range, replacement) in &edits {
            let start = range.start.checked_add_signed(delta).unwrap();
            if offset < range.start || (before && offset == range.start && range.is_empty()) {
                break;
            }
            if offset <= range.end {
                return start + replacement.len();
            }
            delta += replacement.len() as isize - range.len() as isize;
        }
        offset.checked_add_signed(delta).unwrap()
    };
    let range = selection.range();
    let selected = !selection.is_empty();
    let (start, end) = (map(range.start, selected), map(range.end, false));
    let after = if selection.anchor <= selection.head {
        Selection {
            anchor: start,
            head: end,
        }
    } else {
        Selection {
            anchor: end,
            head: start,
        }
    };
    let mut text = String::new();
    let mut cursor = span.start;
    for (range, replacement) in &edits {
        text.push_str(&source[cursor..range.start]);
        text.push_str(replacement);
        cursor = range.end;
    }
    text.push_str(&source[cursor..span.end]);
    document
        .replace_with_selection(span, &text, after)
        .expect("edits at line and marker boundaries")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(source: &str, anchor: usize, head: usize) -> Document {
        let mut document = Document::new(source);
        document.set_selection(Selection { anchor, head }).unwrap();
        document
    }

    fn enter(source: &str, caret: usize) -> Document {
        let mut document = doc(source, caret, caret);
        assert!(indent_newline(&mut document));
        assert!(document.can_undo());
        document
    }

    #[test]
    fn enter_copies_spaces_and_tabs_up_to_the_caret() {
        let document = enter("    let x = 1;", 14);
        assert_eq!(document.text(), "    let x = 1;\n    ");
        assert_eq!(document.selection(), Selection::caret(19));
        let document = enter("\t\tif ok {", 9);
        assert_eq!(document.text(), "\t\tif ok {\n\t\t");
        let document = enter(" \t mixed", 8);
        assert_eq!(document.text(), " \t mixed\n \t ");
        // Splitting a line keeps the indentation and moves the rest down.
        let document = enter("    one two", 7);
        assert_eq!(document.text(), "    one\n     two");
        assert_eq!(document.selection(), Selection::caret(12));
        let document = enter("plain", 5);
        assert_eq!(document.text(), "plain\n");
    }

    #[test]
    fn enter_clears_a_line_left_holding_only_indentation() {
        let document = enter("    ", 4);
        assert_eq!(document.text(), "\n    ");
        // Inside the indentation, only the part before the caret is copied.
        let document = enter("    body", 2);
        assert_eq!(document.text(), "\n    body");
        assert_eq!(document.selection(), Selection::caret(3));
        let document = enter("a\n\t\n", 3);
        assert_eq!(document.text(), "a\n\n\t\n");
        let mut document = enter("  x", 3);
        assert!(indent_newline(&mut document));
        assert_eq!(document.text(), "  x\n\n  ");
        assert!(document.undo());
        assert_eq!(document.text(), "  x\n  ");
        assert!(document.undo());
        assert_eq!(document.text(), "  x");
        // Column zero has nothing to copy or clear.
        let document = enter("  x", 0);
        assert_eq!(document.text(), "\n  x");
    }

    #[test]
    fn enter_keeps_local_endings_bom_and_replaces_selections() {
        let document = enter("  a\r\n  b", 3);
        assert_eq!(document.text(), "  a\r\n  \r\n  b");
        let document = enter("x\r  y", 5);
        assert_eq!(document.text(), "x\r  y\r  ");
        let document = enter("\u{feff}  a", 6);
        assert_eq!(document.text(), "\u{feff}  a\n  ");
        let document = enter("\u{feff}  ", 5);
        assert_eq!(document.text(), "\u{feff}\n  ");
        let document = enter("\u{feff}a", 0);
        assert_eq!(document.text(), "\u{feff}\na");
        let mut document = doc("    alpha beta", 9, 4);
        assert!(indent_newline(&mut document));
        assert_eq!(document.text(), "\n     beta");
        let mut document = doc("  one\n  two", 2, 11);
        assert!(indent_newline(&mut document));
        assert_eq!(document.text(), "\n  ");
        assert!(document.undo());
        assert_eq!(document.text(), "  one\n  two");
    }

    #[test]
    fn smart_home_alternates_between_text_and_line_start() {
        let source = "a\n    indented\n\t\ttabs\n   \n";
        let line = 2;
        assert_eq!(smart_home(source, line + 8), line + 4);
        assert_eq!(smart_home(source, line + 4), line);
        assert_eq!(smart_home(source, line), line + 4);
        assert_eq!(smart_home(source, line + 2), line + 4);
        assert_eq!(smart_home(source, 19), 17);
        assert_eq!(smart_home(source, 17), 15);
        assert_eq!(smart_home(source, 15), 17);
        // A whitespace-only line toggles between its end and its start.
        assert_eq!(smart_home(source, 24), 25);
        assert_eq!(smart_home(source, 25), 22);
        assert_eq!(smart_home(source, 26), 26);
        assert_eq!(smart_home("ab", 1), 0);
        assert_eq!(smart_home("\u{feff}  x", 6), 5);
        assert_eq!(smart_home("\u{feff}  x", 5), 3);
        assert_eq!(smart_home("\u{feff}  x", 0), 5);
        assert_eq!(smart_home("a\r\n  b", 6), 5);
    }

    #[test]
    fn line_and_column_offsets_clamp_and_count_graphemes() {
        let source = "one\r\ntwo\rthree\ne\u{301}x";
        assert_eq!(line_column_offset(source, 1, 1), 0);
        assert_eq!(line_column_offset(source, 2, 1), 5);
        assert_eq!(line_column_offset(source, 2, 3), 7);
        assert_eq!(line_column_offset(source, 2, 99), 8);
        assert_eq!(line_column_offset(source, 3, 1), 9);
        assert_eq!(line_column_offset(source, 4, 2), 18);
        assert_eq!(line_column_offset(source, 0, 0), 0);
        assert_eq!(line_column_offset(source, 99, 1), source.len());
        assert_eq!(line_column_offset("a\n", 2, 5), 2);
        assert_eq!(line_column_offset("\u{feff}ab", 1, 2), 4);
    }

    #[test]
    fn cut_lines_removes_whole_lines_and_each_cut_undoes() {
        let mut document = doc("one\ntwo\r\nthree", 5, 5);
        assert_eq!(cut_lines(&mut document).as_deref(), Some("two\r\n"));
        assert_eq!(document.text(), "one\nthree");
        assert_eq!(document.selection(), Selection::caret(4));
        assert_eq!(cut_lines(&mut document).as_deref(), Some("three"));
        assert_eq!(document.text(), "one\n");
        // The empty last line has nothing to cut.
        assert_eq!(cut_lines(&mut document), None);
        assert_eq!(document.text(), "one\n");
        assert!(document.undo());
        assert_eq!(document.text(), "one\nthree");
        assert!(document.undo());
        assert_eq!(document.text(), "one\ntwo\r\nthree");
        assert!(!document.undo());
    }

    #[test]
    fn cut_lines_takes_every_selected_line_and_keeps_a_bom() {
        let mut document = doc("a\nb\nc\nd", 3, 5);
        assert_eq!(cut_lines(&mut document).as_deref(), Some("b\nc\n"));
        assert_eq!(document.text(), "a\nd");
        // Ending at the start of a line leaves that line alone.
        let mut document = doc("a\nb\nc", 2, 4);
        assert_eq!(cut_lines(&mut document).as_deref(), Some("b\n"));
        assert_eq!(document.text(), "a\nc");
        let mut document = doc("a\nb\nc", 5, 0);
        assert_eq!(cut_lines(&mut document).as_deref(), Some("a\nb\nc"));
        assert_eq!(document.text(), "");
        let mut document = doc("\u{feff}first\nsecond", 0, 0);
        assert_eq!(cut_lines(&mut document).as_deref(), Some("first\n"));
        assert_eq!(document.text(), "\u{feff}second");
        assert_eq!(cut_lines(&mut doc("", 0, 0)), None);
    }

    #[test]
    fn duplicate_copies_lines_or_the_selection() {
        let mut document = doc("one\ntwo", 1, 1);
        assert!(duplicate(&mut document));
        assert_eq!(document.text(), "one\none\ntwo");
        let mut document = doc("say hi", 4, 6);
        assert!(duplicate(&mut document));
        assert_eq!(document.text(), "say hihi");
        assert_eq!(document.selection(), Selection { anchor: 6, head: 8 });
        let mut document = doc("ab\ncd\n", 6, 0);
        assert!(duplicate(&mut document));
        assert_eq!(document.text(), "ab\ncd\nab\ncd\n");
        assert_eq!(
            document.selection(),
            Selection {
                anchor: 12,
                head: 6
            }
        );
        assert!(document.undo());
        assert_eq!(document.text(), "ab\ncd\n");
    }

    fn toggled(source: &str, anchor: usize, head: usize, open: &str, close: &str) -> Document {
        let mut document = doc(source, anchor, head);
        toggle_comment(&mut document, open, close);
        document
    }

    #[test]
    fn comments_toggle_one_line_after_its_indentation() {
        let document = toggled("    let x = 1;", 6, 6, "//", "");
        assert_eq!(document.text(), "    // let x = 1;");
        assert_eq!(document.selection(), Selection::caret(9));
        let mut document = document;
        assert!(toggle_comment(&mut document, "//", ""));
        assert_eq!(document.text(), "    let x = 1;");
        assert_eq!(document.selection(), Selection::caret(6));
        assert_eq!(toggled("\t#x", 3, 3, "#", "").text(), "\tx");
        assert_eq!(toggled("--  y", 0, 0, "--", "").text(), " y");
        assert_eq!(toggled("", 0, 0, "#", "").text(), "# ");
        assert_eq!(toggled("a\r\nb\r\n", 3, 3, "#", "").text(), "a\r\n# b\r\n");
    }

    #[test]
    fn comment_blocks_align_at_the_smallest_indent_and_skip_blank_lines() {
        let source = "fn main() {\n    if ok {\n\n        run();\n    }\n}\n";
        let start = source.find("    if").unwrap();
        let end = source.find("    }").unwrap() + 5;
        let mut document = doc(source, start, end);
        assert!(toggle_comment(&mut document, "//", ""));
        assert_eq!(
            document.text(),
            "fn main() {\n    // if ok {\n\n    //     run();\n    // }\n}\n"
        );
        assert_eq!(document.selection().anchor, start);
        assert!(document.can_undo());
        // Mixed blocks comment every line; fully commented blocks uncomment.
        assert!(toggle_comment(&mut document, "//", ""));
        assert_eq!(document.text(), source);
        let mixed = "# done\nnot yet\n";
        let mut document = doc(mixed, 0, mixed.len());
        assert!(toggle_comment(&mut document, "#", ""));
        assert_eq!(document.text(), "# # done\n# not yet\n");
        assert_eq!(
            document.selection(),
            Selection {
                anchor: 0,
                head: 19
            }
        );
        assert!(document.undo());
        assert_eq!(document.text(), mixed);
        assert!(!toggle_comment(&mut doc("\n\n", 0, 2), "#", ""));
    }

    #[test]
    fn wrapping_comments_use_both_markers() {
        let document = toggled("  <b>hi</b>", 0, 0, "<!--", "-->");
        assert_eq!(document.text(), "  <!-- <b>hi</b> -->");
        let mut document = document;
        assert!(toggle_comment(&mut document, "<!--", "-->"));
        assert_eq!(document.text(), "  <b>hi</b>");
        assert_eq!(toggled("<!--x-->  ", 0, 0, "<!--", "-->").text(), "x  ");
        assert_eq!(
            toggled("a {}\nb {}", 0, 9, "/*", "*/").text(),
            "/* a {} */\n/* b {} */"
        );
        // An opening marker alone is not a comment in a wrapping language.
        assert_eq!(
            toggled("<!-- open", 0, 0, "<!--", "-->").text(),
            "<!-- <!-- open -->"
        );
    }

    #[test]
    fn comments_keep_a_bom_and_whole_line_selections() {
        let document = toggled("\u{feff}x\ny\n", 0, 6, "#", "");
        assert_eq!(document.text(), "\u{feff}# x\n# y\n");
        assert_eq!(
            document.selection(),
            Selection {
                anchor: 0,
                head: 10
            }
        );
        let document = toggled("one\ntwo\n", 8, 0, "//", "");
        assert_eq!(document.text(), "// one\n// two\n");
        assert_eq!(
            document.selection(),
            Selection {
                anchor: 14,
                head: 0
            }
        );
    }
}
