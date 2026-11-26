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
/// its parent. A number with more or fewer digits moves the item's content
/// column, so what the item holds moves that much further.
///
/// Lifting also turns the item's later siblings into its children, so
/// numbered ones are renumbered to follow on. They stay where they are when
/// that already makes them its children, as under a parent whose marker is
/// as wide as the item's. Otherwise they move with everything after the
/// item to line up with the children it has, or with its content: a
/// numbered item lifted from under a bullet has its content a column
/// further in than its siblings were. No other numbers change.
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
    let (at, width) = marker(source, &item.range)?;
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
    // How many columns a marker gains or loses with its new number. What
    // the item holds is indented to its content, so it moves as far.
    let grown = |digits: &Range<usize>, number: u64| {
        number.to_string().len() as isize - digits.len() as isize
    };
    let own = |number: Option<u64>| match (&numbered, number) {
        (Some((digits, ..)), Some(number)) => grown(digits, number),
        _ => 0,
    };
    // `base`: columns the item moves, right or left. `adopted`: later
    // siblings to renumber. `tail`: the rest of the parent after a lifted
    // item, and the columns it moves to stay inside that item.
    let (base, number, adopted, tail) = if outward {
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
        // The columns the later siblings' markers start at, nearest the
        // margin and furthest from it.
        let mut columns: Option<(isize, isize)> = None;
        for sibling in following {
            // Skip the children of a sibling already seen.
            if sibling.range.start < next {
                continue;
            }
            next = sibling.range.end;
            let sibling_marker = marker(source, &sibling.range);
            if let Some((sibling_at, _)) = sibling_marker {
                let sibling_line = line_range(source, sibling_at);
                let column = (sibling_at - content_start(source, sibling_line.start)) as isize;
                let (low, high) = columns.unwrap_or((column, column));
                columns = Some((low.min(column), high.max(column)));
            }
            let ordered = sibling_marker.and_then(|(sibling_at, _)| ordinal(source, sibling_at));
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
                adopted.push((digits, new, sibling.range.clone()));
            }
        }
        // The siblings are this item's children only if they start within
        // three columns of its content: nearer the margin they would be its
        // siblings, numbered as children; further in, code or stray text.
        // Past the content of a child it already has, they would be that
        // child's. Out of place, everything after the item moves to line up
        // with its last child, or with its content.
        let held = own(number) - depth as isize;
        let content = (at - start) as isize + width as isize + held;
        let child = last_in(item.range.start..item_end)
            .filter(|child| child.range.start > item.range.start)
            .and_then(|child| marker(source, &child.range))
            .map(|(child_at, child_width)| {
                let child_line = line_range(source, child_at);
                let column = (child_at - content_start(source, child_line.start)) as isize;
                (column + held, child_width as isize)
            });
        let (target, furthest) = match child {
            Some((column, child_width)) => (column, (content + 3).min(column + child_width - 1)),
            None => (content, content + 3),
        };
        let reach = columns.map_or(0, |(low, high)| {
            if low >= content && high <= furthest {
                0
            } else {
                target - low
            }
        });
        let tail = (item_end..parent.range.end.max(item_end), reach);
        (-(depth as isize), number, adopted, Some(tail))
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
        let (sibling_at, sibling_width) = marker(source, &sibling.range)?;
        let sibling_line = line_range(source, sibling.range.start);
        // Line up under the sibling's content, from wherever this item sits.
        let target = sibling_at - content_start(source, sibling_line.start) + sibling_width;
        let Some(added) = target.checked_sub(at - start).filter(|added| *added > 0) else {
            return Some(false);
        };
        let number = numbered
            .as_ref()
            .map(|(_, _, delimiter)| after(last_in(sibling.range.clone()), *delimiter));
        (added as isize, number, Vec::new(), None)
    };
    let held = base + own(number);
    let (region_end, reach) = tail
        .as_ref()
        .map_or((item_end, 0), |(range, reach)| (range.end, *reach));
    // The columns the line starting at `line_start` moves: the item's first
    // line by `base`; what the item holds by that and its marker's growth;
    // the lines after a lifted item by `reach`, and inside a renumbered
    // sibling by its marker's growth too.
    let shift_of = |line_start: usize| {
        if line_start == line.start {
            base
        } else if line_start < item_end {
            held
        } else {
            adopted
                .iter()
                .filter(|(digits, _, range)| {
                    line_start > line_range(source, digits.start).start && line_start < range.end
                })
                .map(|(digits, number, _)| grown(digits, *number))
                .sum::<isize>()
                + reach
        }
    };
    let mut edits: Vec<(Range<usize>, String)> = Vec::new();
    // Each line that has text, from the item's first to the last one moved.
    let mut next = line.start;
    while next < region_end {
        let current = line_range(source, next);
        let from = content_start(source, current.start);
        let first = current.start == line.start;
        if first || !source[from..current.end].trim().is_empty() {
            let shift = shift_of(current.start);
            if shift > 0 {
                edits.push((from..from, " ".repeat(shift as usize)));
            } else if shift < 0 {
                let leading = source[from..current.end]
                    .bytes()
                    .take_while(|byte| *byte == b' ')
                    .count()
                    .min(shift.unsigned_abs());
                if leading > 0 {
                    edits.push((from..from + leading, String::new()));
                }
            }
        }
        if first
            && let (Some((digits, ..)), Some(number)) = (&numbered, number)
            && source[digits.clone()] != number.to_string()
        {
            edits.push((digits.clone(), number.to_string()));
        }
        // Step over this line's ending; CRLF is two bytes.
        next = current.end
            + match source.as_bytes().get(current.end..) {
                Some([b'\r', b'\n', ..]) => 2,
                Some([_, ..]) => 1,
                _ => break,
            };
    }
    for (digits, number, _) in &adopted {
        edits.push((digits.clone(), number.to_string()));
    }
    // An insertion at a line's start comes before a number that starts there.
    edits.sort_by_key(|(range, _)| range.start);
    let edits: Vec<(Range<usize>, &str)> = edits
        .iter()
        .map(|(range, text)| (range.clone(), text.as_str()))
        .collect();
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

    /// The nesting depth of each list item, in document order.
    fn depths(document: &Document) -> Vec<usize> {
        let mut items: Vec<Range<usize>> = document
            .markdown()
            .blocks
            .iter()
            .filter(|block| block.kind == BlockKind::ListItem)
            .map(|block| block.range.clone())
            .collect();
        items.sort_by_key(|item| (item.start, std::cmp::Reverse(item.end)));
        let holds = |outer: &Range<usize>, inner: &Range<usize>| {
            outer.start <= inner.start && outer.end >= inner.end && outer != inner
        };
        items
            .iter()
            .map(|item| items.iter().filter(|other| holds(other, item)).count())
            .collect()
    }

    #[test]
    fn a_lifted_item_keeps_its_later_siblings_as_its_children() {
        for (before, after, levels) in [
            // A numbered item's content is a column further in than the
            // siblings it leaves under a bullet or a task.
            (
                "- [ ] ship\n  1. tag|\n  2. build\n  3. publish\n- [ ] tell\n",
                "- [ ] ship\n1. tag|\n   1. build\n   2. publish\n- [ ] tell\n",
                vec![0, 0, 1, 1, 0],
            ),
            (
                "- a\n  1. x\n  2. y|\n     more\n  3. z\n     - deep\n",
                "- a\n  1. x\n1. y|\n   more\n   1. z\n      - deep\n",
                vec![0, 1, 0, 1, 2],
            ),
            // Under a wider marker, the siblings would land past the content
            // of a child the item has, and so inside that child.
            (
                "1.  wide\n    - one|\n      - kid\n    - two\n    - three\n",
                "1.  wide\n- one|\n  - kid\n  - two\n  - three\n",
                vec![0, 0, 1, 1, 1],
            ),
            // Further in still, they would be code or stray text.
            (
                "10.   wide\n      - one|\n\n      - two\n\n      - three\n",
                "10.   wide\n- one|\n\n  - two\n\n  - three\n",
                vec![0, 0, 1, 1],
            ),
            // Already in place: nothing but the item moves.
            (
                "- a\n    - one|\n    - two\n",
                "- a\n- one|\n    - two\n",
                vec![0, 0, 1],
            ),
            (
                "1. a\n   - one|\n   - two\n",
                "1. a\n- one|\n   - two\n",
                vec![0, 0, 1],
            ),
        ] {
            let (outcome, text) = run(before, true);
            assert_eq!(outcome, Some(true), "{before:?}");
            assert_eq!(text, after, "{before:?}");
            let moved = Document::new(after.replacen('|', "", 1));
            assert_eq!(depths(&moved), levels, "{after:?}");
        }
    }

    #[test]
    fn a_number_that_gains_or_loses_a_digit_takes_its_contents_along() {
        let nine: String = (1..=9).map(|n| format!("   {n}. child\n")).collect();
        let ones = |count: usize| vec![1; count];
        for (before, outward, after, levels) in [
            // Lifted after a ninth item, 1 becomes 10: content moves a column.
            (
                "9. nine\n   1. sub|\n      - detail\n      more\n   2. next\n".to_owned(),
                true,
                "9. nine\n10. sub|\n    - detail\n    more\n    1. next\n".to_owned(),
                vec![0, 0, 1, 1],
            ),
            // Nested after nine children, 2 becomes 10.
            (
                format!("1. a\n{nine}2. b|\n   - kid\n   text\n"),
                false,
                format!("1. a\n{nine}   10. b|\n       - kid\n       text\n"),
                [vec![0], ones(9), vec![1, 2]].concat(),
            ),
            // Nested under a tenth item, 11 becomes 1.
            (
                "10) ten\n11) next|\n    - kid\n".to_owned(),
                false,
                "10) ten\n    1) next|\n       - kid\n".to_owned(),
                vec![0, 1, 2],
            ),
            // A sibling renumbered as a child gains a digit in the same way.
            (
                format!(
                    "- p\n  - x|\n{}  1. s\n     - sk\n",
                    nine.replace("   ", "    ")
                ),
                true,
                format!(
                    "- p\n- x|\n{}  10. s\n      - sk\n",
                    nine.replace("   ", "  ")
                ),
                [vec![0, 0], ones(9), vec![1, 2]].concat(),
            ),
        ] {
            let (outcome, text) = run(&before, outward);
            assert_eq!(outcome, Some(true), "{before:?}");
            assert_eq!(text, after, "{before:?}");
            // Whatever the item held, it still holds.
            let moved = Document::new(after.replacen('|', "", 1));
            assert_eq!(depths(&moved), levels, "{after:?}");
        }
    }

    /// A random list of bullets, tasks and numbers, tight or loose, with
    /// continuation lines, wide markers and children.
    fn random_list(seed: &mut u64, indent: usize, depth: usize, out: &mut String) {
        fn below(seed: &mut u64, n: u64) -> u64 {
            *seed ^= *seed << 13;
            *seed ^= *seed >> 7;
            *seed ^= *seed << 17;
            *seed % n
        }
        let ordered = below(seed, 10) < 4;
        let bullet = ["-", "*", "+"][below(seed, 3) as usize];
        let delimiter = if below(seed, 10) < 7 { "." } else { ")" };
        // Only a list that starts at 1 can begin right after its parent's text.
        let first = if depth > 0 || below(seed, 10) < 7 {
            1
        } else {
            below(seed, 12) + 1
        };
        let loose = below(seed, 10) < 2;
        let count = 1 + below(seed, 4);
        for index in 0..count {
            let marker = if ordered {
                format!("{}{delimiter}", first + index)
            } else {
                bullet.to_owned()
            };
            let gap = if below(seed, 10) < 1 {
                1 + below(seed, 3) as usize
            } else {
                1
            };
            let task = match (ordered, below(seed, 8)) {
                (false, 0) => "[ ] ",
                (false, 1) => "[x] ",
                _ => "",
            };
            let content = indent + marker.len() + gap;
            let pad = " ".repeat(indent);
            let spaces = " ".repeat(gap);
            out.push_str(&format!("{pad}{marker}{spaces}{task}item{index}d{depth}\n"));
            if below(seed, 10) < 2 {
                out.push_str(&format!("{}continued text\n", " ".repeat(content)));
            }
            if below(seed, 10) < 1 {
                out.push_str(&format!("\n{}second paragraph\n", " ".repeat(content)));
            }
            if depth < 3 && below(seed, 100) < 45 {
                random_list(seed, content, depth + 1, out);
            }
            if loose && index + 1 < count {
                out.push('\n');
            }
        }
    }

    /// On any list, a move takes the item and what it holds one level in or
    /// out and leaves every other item at its level; nothing stops being a
    /// list item; no text changes but indentation and numbers; and undo
    /// restores the source.
    #[test]
    fn a_move_changes_one_items_level_and_nothing_elses() {
        let text_of = |source: &str| -> Vec<String> {
            source
                .lines()
                .map(|line| {
                    let line = line.trim_start_matches(' ');
                    let digits = line.bytes().take_while(u8::is_ascii_digit).count();
                    line[digits..].to_owned()
                })
                .collect()
        };
        let mut moves = 0;
        for seed in 1..=250u64 {
            let mut state = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
            let mut source = String::new();
            random_list(&mut state, 0, 0, &mut source);
            let base = Document::new(source.clone());
            let before = depths(&base);
            let mut items: Vec<Range<usize>> = base
                .markdown()
                .blocks
                .iter()
                .filter(|block| block.kind == BlockKind::ListItem)
                .map(|block| block.range.clone())
                .collect();
            items.sort_by_key(|item| (item.start, std::cmp::Reverse(item.end)));
            for (index, item) in items.iter().enumerate() {
                for outward in [false, true] {
                    let mut document = Document::new(source.clone());
                    let line = line_range(&source, item.start);
                    document.set_caret(line.end).unwrap();
                    if nest_item(&mut document, None, outward) != Some(true) {
                        assert_eq!(document.text(), source);
                        continue;
                    }
                    moves += 1;
                    let context = format!(
                        "seed {seed}, item {index}, outward {outward}:\n{source}---\n{}",
                        document.text()
                    );
                    assert_eq!(text_of(document.text()), text_of(&source), "{context}");
                    let expected: Vec<usize> = items
                        .iter()
                        .zip(&before)
                        .map(|(other, depth)| {
                            let held = other.start >= item.start && other.end <= item.end;
                            match (held, outward) {
                                (true, false) => depth + 1,
                                (true, true) => depth - 1,
                                _ => *depth,
                            }
                        })
                        .collect();
                    assert_eq!(depths(&document), expected, "{context}");
                    assert!(document.undo());
                    assert_eq!(document.text(), source);
                }
            }
        }
        assert!(moves > 2000, "{moves}");
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
