//! Tab and Shift+Tab on a Markdown list item: nest it under the item
//! above, or lift it out one level, carrying its own lines along.
use std::ops::Range;

use super::{empty_nested_dash, general::content_start, line_range, replace_many};
use crate::{
    Document,
    markdown::{Block, BlockKind, MarkdownSnapshot},
};

/// Where a plainly indented item's marker starts, and how far past it the
/// item's content begins: the marker and one to four spaces. `None` inside
/// a quote, after another marker on the line, or behind tabs.
fn marker(source: &str, item: &Range<usize>) -> Option<(usize, usize)> {
    let line = line_range(source, item.start);
    let start = content_start(source, line.start);
    let bytes = source.as_bytes();
    let at = start
        + source[start..line.end]
            .bytes()
            .take_while(|b| *b == b' ')
            .count();
    let mut end = at;
    match *bytes.get(end)? {
        b'-' | b'*' | b'+' => end += 1,
        b'0'..=b'9' => {
            while bytes.get(end).is_some_and(u8::is_ascii_digit) {
                end += 1;
            }
            if !matches!(bytes.get(end), Some(b'.' | b')')) {
                return None;
            }
            end += 1;
        }
        _ => return None,
    }
    let spaces = source[end..line.end]
        .bytes()
        .take_while(|b| *b == b' ')
        .count();
    if spaces == 0 && end < line.end {
        return None;
    }
    // Five or more spaces start indented code one column after the marker.
    let spacing = if (1..=4).contains(&spaces) { spaces } else { 1 };
    Some((at, end - at + spacing))
}

/// An ordered marker's digits, its number and its delimiter.
fn ordinal(source: &str, at: usize) -> Option<(Range<usize>, u64, u8)> {
    let digits = source[at..].bytes().take_while(u8::is_ascii_digit).count();
    let number = source[at..at + digits].parse().ok()?;
    let delimiter = *source.as_bytes().get(at + digits)?;
    Some((at..at + digits, number, delimiter))
}

/// Nest the list item on the caret's line under the item before it
/// (`outward` false), or lift it to its parent's level. The item's
/// continuation lines and children move with it.
///
/// A numbered item takes the number its new place calls for, because
/// CommonMark only lets a numbered list start mid-item at 1: nested, it
/// follows the children already there or starts at 1; lifted, it follows
/// its parent. Lifting also turns the item's later siblings into its
/// children, so numbered ones are renumbered to follow on. No other
/// numbers change.
///
/// `None` when this is not such a line: a selection, text outside a list,
/// an item's later lines, a quoted or tab-indented list, or a top-level
/// item being lifted; the caller's ordinary Tab applies. `Some(false)` when
/// it is an item with nothing above to nest under.
pub fn nest_item(
    document: &mut Document,
    cached: Option<&MarkdownSnapshot>,
    outward: bool,
) -> Option<bool> {
    let selection = document.selection();
    if !selection.is_empty() {
        return None;
    }
    let fresh;
    let snapshot = match cached.filter(|snapshot| snapshot.revision == document.revision()) {
        Some(snapshot) => snapshot,
        None => {
            fresh = document.markdown();
            &fresh
        }
    };
    let source = document.text();
    let line = line_range(source, selection.head);
    let empty_dash = empty_nested_dash(source, &line, snapshot);
    let items = || {
        snapshot
            .blocks
            .iter()
            .chain(empty_dash.iter())
            .filter(|block| block.kind == BlockKind::ListItem)
    };
    let mut here =
        items().filter(|block| line_range(source, block.range.start).start == line.start);
    let item = here.next()?;
    // `- - inner` holds two items on one line; leave it to plain Tab.
    if here.next().is_some() {
        return None;
    }
    let (at, _) = marker(source, &item.range)?;
    let start = content_start(source, line.start);
    let item_end = item.range.end.max(line.end);
    // The smallest item around this one.
    let above = items()
        .filter(|block| {
            block.range.start <= item.range.start
                && block.range.end >= item_end
                && block.range != item.range
        })
        .min_by_key(|block| block.range.len());
    // Of the items wholly inside `range`, the outermost that ends last:
    // the last child there.
    let last_in = |range: Range<usize>| {
        items()
            .filter(|block| {
                block.range.start >= range.start
                    && block.range.end <= range.end
                    && block.range != range
            })
            .max_by_key(|block| (block.range.end, block.range.len()))
    };
    // The number after `block`'s, if it is numbered with `delimiter`.
    let after = |block: Option<&Block>, delimiter: u8| {
        block
            .and_then(|block| marker(source, &block.range))
            .and_then(|(at, _)| ordinal(source, at))
            .filter(|(_, _, other)| *other == delimiter)
            .map_or(1, |(_, number, _)| number.saturating_add(1))
    };
    let numbered = ordinal(source, at);
    let (removed, spaces, number, adopted) = if outward {
        let parent = above?;
        let (parent_at, _) = marker(source, &parent.range)?;
        let parent_line = line_range(source, parent.range.start);
        let parent_indent = parent_at - content_start(source, parent_line.start);
        let depth = (at - start).checked_sub(parent_indent).filter(|d| *d > 0)?;
        let number = numbered
            .as_ref()
            .map(|(_, _, delimiter)| after(Some(parent), *delimiter));
        // Later siblings become this item's children; numbered ones follow
        // on from the children it has.
        let mut adopted = Vec::new();
        let mut following: Vec<&Block> = items()
            .filter(|block| block.range.start >= item_end && block.range.end <= parent.range.end)
            .collect();
        following.sort_by_key(|block| (block.range.start, std::cmp::Reverse(block.range.len())));
        let mut next = item_end;
        let mut last = last_in(item.range.start..item_end);
        let mut run: Option<(u8, u64)> = None;
        for sibling in following {
            // Skip the children of a sibling already seen.
            if sibling.range.start < next {
                continue;
            }
            next = sibling.range.end;
            let ordered = marker(source, &sibling.range)
                .and_then(|(sibling_at, _)| ordinal(source, sibling_at));
            let Some((digits, old, delimiter)) = ordered else {
                last = Some(sibling);
                run = None;
                continue;
            };
            let new = match run {
                Some((same, number)) if same == delimiter => number.saturating_add(1),
                Some(_) => 1,
                None => after(last, delimiter),
            };
            run = Some((delimiter, new));
            if new != old {
                adopted.push((digits, new.to_string()));
            }
        }
        (depth, String::new(), number, adopted)
    } else {
        // The nearest earlier item of the same list and level.
        let within = above.map_or(0, |block| block.range.start);
        let sibling = items()
            .filter(|block| {
                block.range.start >= within
                    && block.range.end <= line.start.max(item.range.start)
                    && block.range != item.range
            })
            .max_by_key(|block| (block.range.end, block.range.len()));
        let Some(sibling) = sibling.filter(|block| {
            source[block.range.end.min(line.start)..line.start]
                .bytes()
                .all(|byte| byte.is_ascii_whitespace())
        }) else {
            return Some(false);
        };
        let (sibling_at, width) = marker(source, &sibling.range)?;
        let sibling_line = line_range(source, sibling.range.start);
        // Line up under the sibling's content, from wherever this item sits.
        let target = sibling_at - content_start(source, sibling_line.start) + width;
        let Some(added) = target.checked_sub(at - start).filter(|added| *added > 0) else {
            return Some(false);
        };
        let number = numbered
            .as_ref()
            .map(|(_, _, delimiter)| after(last_in(sibling.range.clone()), *delimiter));
        (0, " ".repeat(added), number, Vec::new())
    };
    let number = number.map(|number| number.to_string());
    let renumbered: Vec<(Range<usize>, String)> = adopted;
    let mut edits: Vec<(Range<usize>, &str)> = Vec::new();
    // The item's first line, then each of its later lines that has text.
    let mut next = line.start;
    while next < item_end {
        let current = line_range(source, next);
        let from = content_start(source, current.start);
        let first = current.start == line.start;
        if first || !source[from..current.end].trim().is_empty() {
            if outward {
                let leading = source[from..current.end]
                    .bytes()
                    .take_while(|byte| *byte == b' ')
                    .count()
                    .min(removed);
                if leading > 0 {
                    edits.push((from..from + leading, ""));
                }
            } else {
                edits.push((from..from, spaces.as_str()));
            }
        }
        if first
            && let (Some((digits, ..)), Some(number)) = (&numbered, &number)
            && source[digits.clone()] != **number
        {
            edits.push((digits.clone(), number.as_str()));
        }
        // Step over this line's ending; CRLF is two bytes.
        next = current.end
            + match source.as_bytes().get(current.end..) {
                Some([b'\r', b'\n', ..]) => 2,
                Some([_, ..]) => 1,
                _ => break,
            };
    }
    for (digits, number) in &renumbered {
        edits.push((digits.clone(), number.as_str()));
    }
    document.break_undo_group();
    Some(replace_many(document, edits))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Selection;

    /// Run on the caret marked `|`; the result is marked the same way.
    fn run(marked: &str, outward: bool) -> (Option<bool>, String) {
        let caret = marked.find('|').expect("caret mark");
        let mut document = Document::new(marked.replacen('|', "", 1));
        document.set_caret(caret).unwrap();
        let outcome = nest_item(&mut document, None, outward);
        let mut text = document.text().to_owned();
        text.insert(document.selection().head, '|');
        (outcome, text)
    }

    #[test]
    fn tab_nests_an_item_under_the_one_before_it() {
        for (before, after) in [
            ("- one\n- tw|o", "- one\n  - tw|o"),
            ("* one\n* |two\n", "* one\n  * |two\n"),
            // A numbered item starts its new list at 1, and returns as the next.
            ("1. one\n2. two|", "1. one\n   1. two|"),
            ("10) ten\n11) next|", "10) ten\n    1) next|"),
            ("- a\n1. b|", "- a\n  1. b|"),
            ("-   wide\n- next|", "-   wide\n    - next|"),
            ("- [ ] a\n- [x] b|", "- [ ] a\n  - [x] b|"),
            // An empty item, as Enter leaves it.
            ("- one\n- |", "- one\n  - |"),
            // A loose list, and CRLF endings.
            ("- a\n\n- b|", "- a\n\n  - b|"),
            ("- a\r\n- b|\r\n  - c\r\n", "- a\r\n  - b|\r\n    - c\r\n"),
            ("\u{feff}- a\n- b|", "\u{feff}- a\n  - b|"),
            // Under the previous item at this level, past its children.
            ("- a\n  - a1\n- b|", "- a\n  - a1\n  - b|"),
            ("- a\n  - a1\n  - a2|", "- a\n  - a1\n    - a2|"),
            // Continuation lines and children travel; blank lines stay empty.
            (
                "- a\n- b|\n  more\n\n  para\n  - b1\n- c\n",
                "- a\n  - b|\n    more\n\n    para\n    - b1\n- c\n",
            ),
        ] {
            let (outcome, text) = run(before, false);
            assert_eq!(outcome, Some(true), "{before:?}");
            assert_eq!(text, after, "{before:?}");
            // Shift+Tab is the inverse.
            let (outcome, text) = run(after, true);
            assert_eq!(outcome, Some(true), "{after:?}");
            assert_eq!(text, before, "{after:?}");
        }
    }

    #[test]
    fn numbered_items_take_the_number_their_new_place_calls_for() {
        for (before, outward, after) in [
            // Nested after numbered children, an item continues them.
            (
                "1. a\n   1. x\n   2. y\n2. b|",
                false,
                "1. a\n   1. x\n   2. y\n   3. b|",
            ),
            // After bulleted children it starts a list of its own.
            ("1. a\n   - x\n2. b|", false, "1. a\n   - x\n   1. b|"),
            ("5. a\n7. b|", false, "5. a\n   1. b|"),
            // Lifted, it follows its parent; a bullet parent starts it at 1.
            ("3. a\n   1. x|", true, "3. a\n4. x|"),
            ("- a\n\n  3. x|", true, "- a\n\n1. x|"),
            // Its later siblings become its children and are numbered on.
            (
                "1. a\n   1. x\n   2. y|\n   3. z\n   4. w\n2. b\n",
                true,
                "1. a\n   1. x\n2. y|\n   1. z\n   2. w\n2. b\n",
            ),
            (
                "1. a\n   1. x|\n      1. kept\n   2. y\n",
                true,
                "1. a\n2. x|\n   1. kept\n   2. y\n",
            ),
            // Bullets and their own children are left as typed.
            (
                "- a\n  - x|\n  - y\n    7. deep\n  3. z\n",
                true,
                "- a\n- x|\n  - y\n    7. deep\n  1. z\n",
            ),
        ] {
            let (outcome, text) = run(before, outward);
            assert_eq!(outcome, Some(true), "{before:?}");
            assert_eq!(text, after, "{before:?}");
        }
    }

    /// Every list keeps its items after a move: nothing turns into
    /// paragraph text because a number could not start a list.
    #[test]
    fn moved_lists_still_parse_as_the_same_items() {
        for (before, outward) in [
            ("1. a\n2. b|\n3. c\n", false),
            ("1. a\n   1. x\n   2. y|\n   3. z\n", true),
            ("- a\n  1. x\n  2. y|\n  3. z\n  - w\n  4. v\n", true),
            ("1. a\n2. b\n   1. c\n3. d|\n", false),
        ] {
            let caret = before.find('|').unwrap();
            let mut document = Document::new(before.replacen('|', "", 1));
            document.set_caret(caret).unwrap();
            let count = |document: &Document| {
                document
                    .markdown()
                    .blocks
                    .iter()
                    .filter(|block| block.kind == BlockKind::ListItem)
                    .count()
            };
            let items = count(&document);
            assert_eq!(nest_item(&mut document, None, outward), Some(true));
            assert_eq!(count(&document), items, "{:?}", document.text());
        }
    }

    #[test]
    fn an_item_with_nothing_above_stays_where_it_is() {
        for marked in [
            "- on|e",
            "- on|e\n- two",
            "intro\n\n- on|e",
            "- a\n\npara\n\n- b|",
            // Already the first child.
            "- a\n  - a1|",
        ] {
            let (outcome, text) = run(marked, false);
            assert_eq!(outcome, Some(false), "{marked:?}");
            assert_eq!(text, marked);
        }
    }

    #[test]
    fn other_lines_keep_ordinary_tab() {
        for marked in [
            "para|graph",
            "",
            "- a\n  mo|re",
            "```\n- a\n- b|\n```",
            "    - a\n    - b|",
            "> - a\n> - b|",
            "- - a\n- - b|",
            "- a\n\t- b|",
            "# - a|",
        ] {
            for outward in [false, true] {
                let marked = if marked.is_empty() { "|" } else { marked };
                let (outcome, text) = run(marked, outward);
                assert_eq!(outcome, None, "{marked:?} {outward}");
                assert_eq!(text, marked);
            }
        }
        // Lifting a top-level item, and any selection, are ordinary too.
        assert_eq!(run("- a\n- b|", true).0, None);
        let mut document = Document::new("- a\n- b");
        document
            .set_selection(Selection { anchor: 4, head: 7 })
            .unwrap();
        assert_eq!(nest_item(&mut document, None, false), None);
        assert_eq!(document.text(), "- a\n- b");
    }

    #[test]
    fn nesting_is_one_undo_step_and_keeps_a_mid_line_caret() {
        let mut document = Document::new("- a\n- bcd\n  - e\n");
        document.set_caret(7).unwrap();
        assert_eq!(nest_item(&mut document, None, false), Some(true));
        assert_eq!(document.text(), "- a\n  - bcd\n    - e\n");
        assert_eq!(document.selection(), Selection::caret(9));
        assert!(document.undo());
        assert_eq!(document.text(), "- a\n- bcd\n  - e\n");
        assert_eq!(document.selection(), Selection::caret(7));
        assert!(!document.can_undo());
    }

    #[test]
    fn an_empty_nested_dash_lifts_out_though_it_reads_as_an_underline() {
        // `one` and an indented `-` are a setext heading to the parser.
        let (outcome, text) = run("- one\n  - |", true);
        assert_eq!(outcome, Some(true));
        assert_eq!(text, "- one\n- |");
    }
}
