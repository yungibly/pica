use std::{collections::VecDeque, fmt, ops::Range};

use unicode_segmentation::{GraphemeCursor, UnicodeSegmentation};

use crate::markdown::{self, MarkdownSnapshot, Task};

/// Revisions whose first changed offset stays known to `first_change_since`.
const EDIT_LOG: usize = 64;

/// Byte offsets where source lines begin. A line ends after LF, after CRLF,
/// or after a lone CR, so CRLF never splits and every start is a grapheme
/// boundary. Kept current through each replacement.
#[derive(Clone, Debug)]
struct LineIndex {
    starts: Vec<usize>,
}

impl LineIndex {
    fn new(text: &str) -> Self {
        let mut starts = vec![0];
        Self::scan(text.as_bytes(), 1, text.len(), &mut starts);
        Self { starts }
    }

    fn starts_line(bytes: &[u8], at: usize) -> bool {
        at > 0
            && match bytes[at - 1] {
                b'\n' => true,
                b'\r' => bytes.get(at) != Some(&b'\n'),
                _ => false,
            }
    }

    /// Append the line starts in `from..=to`.
    fn scan(bytes: &[u8], from: usize, to: usize, out: &mut Vec<usize>) {
        let from = from.max(1);
        if from > to {
            return;
        }
        for (index, byte) in bytes[from - 1..to].iter().enumerate() {
            if matches!(byte, b'\n' | b'\r') && Self::starts_line(bytes, from + index) {
                out.push(from + index);
            }
        }
    }

    /// Account for `start..old_end` having become `start..new_end` in `text`.
    fn edit(&mut self, text: &str, start: usize, old_end: usize, new_end: usize) {
        // Only starts whose preceding or following byte changed can differ.
        let first = self.starts.partition_point(|&at| at < start.max(1));
        let last = self.starts.partition_point(|&at| at <= old_end);
        let mut fresh = Vec::new();
        Self::scan(text.as_bytes(), start, new_end, &mut fresh);
        for at in &mut self.starts[last..] {
            *at = *at - old_end + new_end;
        }
        self.starts.splice(first..last, fresh);
    }

    /// The zero-based line containing `offset`.
    fn line(&self, offset: usize) -> usize {
        self.starts.partition_point(|&at| at <= offset) - 1
    }
}

/// Source byte offsets, always at extended grapheme boundaries in a Document.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    pub anchor: usize,
    pub head: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsaved_buffers_have_no_baseline_even_when_empty() {
        for source in ["", "e\u{301}👩🏽‍💻\r\n"] {
            let mut doc = Document::unsaved(source);
            assert!(doc.is_dirty());
            assert!(!doc.can_undo());
            doc.insert("x");
            doc.undo();
            assert_eq!(doc.text(), source);
            assert!(doc.is_dirty());
            doc.mark_saved();
            assert!(!doc.is_dirty());
        }
    }

    #[test]
    fn reload_is_undoable_against_the_latest_disk_baseline() {
        let mut doc = Document::new("original long text");
        doc.insert("local ");
        let before = doc.text().to_owned();
        let selection = Selection {
            anchor: before.len(),
            head: 2,
        };
        doc.set_selection(selection).unwrap();
        assert!(doc.reload_saved("e\u{301}界\r\n".into()).unwrap());
        assert_eq!(
            doc.selection(),
            Selection {
                anchor: doc.text().len(),
                head: 0
            }
        );
        assert!(!doc.is_dirty());
        assert!(doc.undo());
        assert_eq!(doc.text(), before);
        assert_eq!(doc.selection(), selection);
        assert!(doc.is_dirty());
        assert!(doc.redo());
        assert!(!doc.is_dirty());
        doc.undo();
        doc.type_text("new branch");
        assert!(!doc.can_redo());
        assert!(doc.is_dirty());
    }

    #[test]
    fn reload_rejects_unretainable_snapshot_without_advancing_baseline() {
        for limits in [
            HistoryLimits {
                max_entries: 0,
                max_bytes: usize::MAX,
            },
            HistoryLimits {
                max_entries: 10,
                max_bytes: std::mem::size_of::<State>() + 2,
            },
        ] {
            let mut doc = Document::new("base");
            doc.insert("local");
            doc.set_history_limits(limits);
            let before = (
                doc.text().to_owned(),
                doc.selection(),
                doc.revision(),
                doc.saved_text.clone(),
                doc.can_redo(),
            );
            assert_eq!(
                doc.reload_saved("disk".into()),
                Err(EditError::HistoryUnavailable)
            );
            assert_eq!(
                (
                    doc.text(),
                    doc.selection(),
                    doc.revision(),
                    doc.saved_text.as_str(),
                    doc.can_redo()
                ),
                (
                    before.0.as_str(),
                    before.1,
                    before.2,
                    before.3.as_str(),
                    before.4
                )
            );
            assert!(doc.is_dirty());
        }
    }

    #[test]
    fn reload_retains_its_immediate_inverse_under_a_one_entry_limit() {
        let mut doc = Document::new("base");
        doc.set_history_limits(HistoryLimits {
            max_entries: 1,
            max_bytes: 1024,
        });
        doc.type_text("local");
        let old = doc.text().to_owned();
        doc.reload_saved("fresh".into()).unwrap();
        assert_eq!(doc.undo.len(), 1);
        doc.undo();
        assert_eq!(doc.text(), old);
        assert!(doc.is_dirty());
        doc.redo();
        assert_eq!(doc.text(), "fresh");
        assert!(!doc.is_dirty());
    }

    #[test]
    fn identical_reload_adopts_baseline_without_fabricating_an_edit() {
        let mut doc = Document::unsaved("");
        doc.set_history_limits(HistoryLimits {
            max_entries: 0,
            max_bytes: 0,
        });
        assert!(!doc.reload_saved("".into()).unwrap());
        assert!(!doc.is_dirty());
        assert!(!doc.can_undo());
        assert_eq!(doc.revision(), 0);
    }

    #[test]
    fn source_fidelity_and_navigation_do_not_dirty() {
        for source in [
            "",
            "no final newline",
            "\u{feff}# Title\r\nline  \n\tend\r",
            "e\u{301} 世界 👩🏽‍💻\r\n",
        ] {
            let mut doc = Document::new(source);
            doc.select_all();
            doc.move_left(false);
            doc.move_right(true);
            let _ = doc.markdown();
            doc.mark_saved();
            assert_eq!(doc.text().as_bytes(), source.as_bytes());
            assert_eq!(doc.revision(), 0);
            assert!(!doc.is_dirty());
            assert!(!doc.can_undo());
        }
    }

    #[test]
    fn unicode_navigation_and_delete_use_whole_graphemes() {
        let graphemes = ["e\u{301}", "界", "👩🏽‍💻", "🇺🇸", "\r\n"];
        let source = graphemes.concat();
        let mut doc = Document::new(&source);
        let mut position = 0;
        for grapheme in graphemes {
            doc.move_right(false);
            position += grapheme.len();
            assert_eq!(doc.selection(), Selection::caret(position));
        }
        for grapheme in graphemes.into_iter().rev() {
            assert!(doc.backspace());
            position -= grapheme.len();
            assert_eq!(doc.text(), &source[..position]);
            assert_eq!(doc.selection(), Selection::caret(position));
        }
        assert!(!doc.backspace());
        for _ in graphemes {
            assert!(doc.undo());
        }
        assert_eq!(doc.text(), source);
        doc.set_caret(0).unwrap();
        for grapheme in graphemes {
            assert!(doc.delete_forward());
            position += grapheme.len();
            assert_eq!(doc.text(), &source[position..]);
        }
        assert!(!doc.delete_forward());
    }

    #[test]
    fn invalid_byte_and_grapheme_positions_are_rejected_without_mutation() {
        let mut doc = Document::new("e\u{301}界\r\n");
        for offset in [1, 2, 4, 5, 7, 9, usize::MAX] {
            assert_eq!(
                doc.set_caret(offset),
                Err(EditError::InvalidBoundary(offset))
            );
            assert_eq!(doc.selection(), Selection::caret(0));
        }
        assert_eq!(
            doc.replace_range(0..1, "x"),
            Err(EditError::InvalidBoundary(1))
        );
        assert_eq!(
            doc.replace_range(Range { start: 3, end: 0 }, "x"),
            Err(EditError::InvalidRange)
        );
        assert_eq!(doc.revision(), 0);
        assert!(!doc.is_dirty());
    }

    #[test]
    fn edits_can_merge_graphemes_and_still_leave_a_valid_caret() {
        for (source, caret, inserted, expected, after) in [
            ("a!", 1, "\u{301}", "a\u{301}!", 3),
            ("\u{301}!", 0, "a", "a\u{301}!", 3),
            ("🇸!", 0, "🇺", "🇺🇸!", 8),
        ] {
            let mut doc = Document::new(source);
            doc.set_caret(caret).unwrap();
            doc.insert(inserted);
            assert_eq!(doc.text(), expected);
            assert_eq!(doc.selection(), Selection::caret(after));
            assert!(doc.is_grapheme_boundary(after));
            doc.undo();
            assert_eq!(doc.text(), source);
            assert_eq!(doc.selection(), Selection::caret(caret));
        }
        let mut doc = Document::new("🇺x🇸");
        doc.set_selection(Selection { anchor: 4, head: 5 }).unwrap();
        doc.backspace();
        assert_eq!(doc.text(), "🇺🇸");
        assert_eq!(doc.selection(), Selection::caret(8));
    }

    #[test]
    fn literal_paste_is_one_transaction_and_restores_directional_selection() {
        let mut doc = Document::new("before replace after");
        let selection = Selection {
            anchor: 14,
            head: 7,
        };
        doc.set_selection(selection).unwrap();
        let paste = "- [x] task\r\n9. nine\n\t literal\u{1b}[31m";
        doc.insert(paste);
        assert_eq!(doc.text(), format!("before {paste} after"));
        assert_eq!(doc.revision(), 1);
        assert!(doc.undo());
        assert_eq!(doc.text(), "before replace after");
        assert_eq!(doc.selection(), selection);
        assert!(!doc.can_undo());
        assert!(doc.redo());
        assert_eq!(doc.text(), format!("before {paste} after"));
    }

    #[test]
    fn dirty_state_follows_saved_content_across_history_and_branches() {
        let mut doc = Document::new("base");
        doc.insert("a");
        assert!(doc.is_dirty());
        doc.mark_saved();
        assert!(!doc.is_dirty());
        doc.insert("b");
        doc.undo();
        assert!(!doc.is_dirty());
        doc.undo();
        assert!(doc.is_dirty());
        doc.redo();
        assert!(!doc.is_dirty());
        doc.insert("c");
        assert!(!doc.can_redo());
        assert!(doc.is_dirty());
        assert_eq!(doc.revision(), 6);
    }

    #[test]
    fn task_toggle_is_marker_only_and_undo_restores_the_caret() {
        let source = "> * [X] first  \r\n> * [ ] second\n";
        let mut doc = Document::new(source);
        let selection = Selection {
            anchor: source.len(),
            head: 10,
        };
        doc.set_selection(selection).unwrap();
        let task = doc.markdown().tasks[0].clone();
        assert!(doc.toggle_task(&task).unwrap());
        assert_eq!(doc.text(), source.replacen("[X]", "[ ]", 1));
        assert_eq!(doc.selection(), selection);
        assert!(matches!(
            doc.toggle_task(&task),
            Err(EditError::StaleRevision { .. })
        ));
        doc.undo();
        assert_eq!(doc.text(), source);
        assert_eq!(doc.selection(), selection);
        assert!(!doc.is_dirty());
        assert!(matches!(
            doc.toggle_task(&task),
            Err(EditError::StaleRevision { .. })
        ));
    }

    #[test]
    fn forged_task_target_is_rejected() {
        let mut doc = Document::new("plain [ ] text");
        let task = Task {
            marker_range: 7..8,
            item_range: 0..14,
            checked: false,
            revision: 0,
        };
        assert_eq!(doc.toggle_task(&task), Err(EditError::InvalidTask));
        assert!(!doc.toggle_task_at_caret());
        assert_eq!(doc.text(), "plain [ ] text");
    }

    #[test]
    fn no_edit_or_identical_replacement_creates_no_history() {
        let mut doc = Document::new("same");
        doc.select_all();
        assert!(!doc.insert("same"));
        assert!(!doc.insert(""));
        assert!(!doc.delete_forward());
        assert!(!doc.is_dirty());
        assert_eq!(doc.revision(), 0);
        assert!(!doc.undo());
    }

    #[test]
    fn selection_collapse_and_extension_respect_direction() {
        let mut doc = Document::new("a界b");
        doc.set_selection(Selection { anchor: 4, head: 1 }).unwrap();
        doc.move_right(false);
        assert_eq!(doc.selection(), Selection::caret(4));
        doc.move_left(true);
        assert_eq!(doc.selection(), Selection { anchor: 4, head: 1 });
        doc.move_left(false);
        assert_eq!(doc.selection(), Selection::caret(1));
        assert_eq!(doc.revision(), 0);
    }

    #[test]
    fn find_is_literal_case_sensitive_nonoverlapping_source_search() {
        let doc = Document::new("aaaaa Aa a.* aZZ [label](secret/path) and `secret/path`");
        assert_eq!(doc.find_matches("aa"), vec![0..2, 2..4]);
        assert_eq!(doc.find_matches("Aa"), vec![6..8]);
        assert_eq!(doc.find_matches("a.*"), vec![9..12]);
        let paths = doc.find_matches("secret/path");
        assert_eq!(paths.len(), 2);
        for range in paths {
            assert_eq!(&doc.text()[range], "secret/path");
        }
        assert!(doc.find_matches("").is_empty());
        assert!(doc.find_matches("missing").is_empty());
        assert!(
            doc.find_matches("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA")
                .is_empty()
        );
        assert_eq!(doc.revision(), 0);
        assert!(!doc.is_dirty());
    }

    #[test]
    fn find_rejects_partial_graphemes_without_skipping_valid_overlaps() {
        let doc = Document::new("e\u{301} 👩🏽‍💻 🇺🇸\r\n界\n");
        for query in ["e", "\u{301}", "👩", "🏽", "💻", "🇺", "\r"] {
            assert!(doc.find_matches(query).is_empty(), "{query:?}");
        }
        for query in ["e\u{301}", "👩🏽‍💻", "🇺🇸", "\r\n", "界"] {
            let ranges = doc.find_matches(query);
            assert_eq!(ranges.len(), 1);
            assert_eq!(&doc.text()[ranges[0].clone()], query);
        }
        assert_eq!(
            doc.find_matches("\n"),
            vec![doc.text().len() - 1..doc.text().len()]
        );
        // The first three regional indicators end inside a flag; the later,
        // overlapping occurrence starts at a whole flag and ends at EOF.
        let flags = Document::new("🇦🇧🇦🇧🇦");
        assert_eq!(flags.find_matches("🇦🇧🇦"), vec![8..20]);
        let flags = Document::new("🇽🇺🇺🇺🇺🇺");
        assert_eq!(flags.find_matches("🇺🇺"), vec![8..16, 16..24]);
    }

    #[test]
    fn replace_all_is_literal_non_cascading_and_keeps_unmatched_bytes() {
        let mut doc = Document::new("aaaaa");
        assert_eq!(doc.replace_all("aa", "aaa"), 2);
        assert_eq!(doc.text(), "aaaaaaa");
        assert_eq!(doc.revision(), 1);
        doc.undo();
        assert_eq!(doc.text(), "aaaaa");
        assert!(!doc.can_undo());

        let mut doc = Document::new("\u{feff}[label](secret/path)\r\n`secret/path`  \nlast\t");
        assert_eq!(doc.replace_all("secret/path", "$1\\n"), 2);
        assert_eq!(doc.text(), "\u{feff}[label]($1\\n)\r\n`$1\\n`  \nlast\t");
        let mut doc = Document::new("\u{feff}alpha\r\nalpha\nalpha  ");
        assert_eq!(doc.replace_all("alpha\r\nalpha", "beta\r\ngamma"), 1);
        assert_eq!(doc.text(), "\u{feff}beta\r\ngamma\nalpha  ");
        assert_eq!(doc.replace_all("\n", "!"), 1);
        assert_eq!(doc.text(), "\u{feff}beta\r\ngamma!alpha  ");
    }

    #[test]
    fn replace_all_noops_preserve_selection_revision_baseline_and_redo() {
        let mut doc = Document::new("cat cat");
        doc.insert("!");
        doc.undo();
        let selection = Selection { anchor: 6, head: 1 };
        doc.set_selection(selection).unwrap();
        let revision = doc.revision();
        assert_eq!(doc.replace_all("cat", "cat"), 2);
        assert_eq!(doc.replace_all("", "extra"), 0);
        assert_eq!(doc.replace_all("missing", ""), 0);
        assert_eq!(doc.text(), "cat cat");
        assert_eq!(doc.selection(), selection);
        assert_eq!(doc.revision(), revision);
        assert!(!doc.is_dirty());
        assert!(!doc.can_undo());
        assert!(doc.can_redo());
        doc.redo();
        assert_eq!(doc.text(), "!cat cat");
    }

    #[test]
    fn replace_all_has_one_undo_step_and_restores_exact_selection() {
        let source = "cat cat cat";
        let mut doc = Document::new(source);
        let before = Selection {
            anchor: 10,
            head: 1,
        };
        doc.set_selection(before).unwrap();
        assert_eq!(doc.replace_all("cat", "elephant"), 3);
        assert_eq!(doc.text(), "elephant elephant elephant");
        let after = Selection {
            anchor: 26,
            head: 0,
        };
        assert_eq!(doc.selection(), after);
        assert_eq!(doc.revision(), 1);
        assert!(doc.is_dirty());
        doc.mark_saved();
        assert!(!doc.is_dirty());
        assert!(doc.undo());
        assert_eq!(doc.text(), source);
        assert_eq!(doc.selection(), before);
        assert!(doc.is_dirty());
        assert!(!doc.can_undo());
        assert!(doc.redo());
        assert_eq!(doc.text(), "elephant elephant elephant");
        assert_eq!(doc.selection(), after);
        assert!(!doc.is_dirty());
    }

    #[test]
    fn replace_all_maps_selection_boundaries_and_direction_through_all_edits() {
        for (before, after) in [
            ((3, 5), (3, 4)),
            ((4, 10), (3, 9)),
            ((10, 4), (9, 3)),
            ((5, 9), (4, 8)),
            ((0, 14), (0, 12)),
            ((3, 3), (3, 3)),
            ((4, 4), (4, 4)),
            ((5, 5), (4, 4)),
            ((9, 9), (8, 8)),
            ((10, 10), (9, 9)),
            ((14, 14), (12, 12)),
        ] {
            let mut doc = Document::new("ab XX cd XX ef");
            let before = Selection {
                anchor: before.0,
                head: before.1,
            };
            doc.set_selection(before).unwrap();
            assert_eq!(doc.replace_all("XX", "Q"), 2);
            assert_eq!(doc.text(), "ab Q cd Q ef");
            assert_eq!(
                doc.selection(),
                Selection {
                    anchor: after.0,
                    head: after.1
                }
            );
            doc.undo();
            assert_eq!(doc.selection(), before);
        }
    }

    #[test]
    fn replace_all_repairs_graphemes_joined_across_replacement_edges() {
        for (source, query, replacement, before, expected, after) in [
            ("aXb", "X", "\u{301}", (1, 2), "a\u{301}b", (0, 3)),
            ("aXb", "X", "\u{301}", (2, 1), "a\u{301}b", (3, 0)),
            ("aXb", "X", "\u{301}", (1, 1), "a\u{301}b", (3, 3)),
            ("🇺x🇸", "x", "", (4, 5), "🇺🇸", (8, 8)),
            ("x🇸", "x", "🇺", (0, 1), "🇺🇸", (0, 8)),
            ("x\n", "x", "\r", (0, 1), "\r\n", (0, 2)),
        ] {
            let mut doc = Document::new(source);
            let before = Selection {
                anchor: before.0,
                head: before.1,
            };
            doc.set_selection(before).unwrap();
            assert_eq!(doc.replace_all(query, replacement), 1);
            assert_eq!(doc.text(), expected);
            assert_eq!(
                doc.selection(),
                Selection {
                    anchor: after.0,
                    head: after.1
                }
            );
            assert!(doc.is_grapheme_boundary(doc.selection().anchor));
            assert!(doc.is_grapheme_boundary(doc.selection().head));
            doc.undo();
            assert_eq!(doc.text(), source);
            assert_eq!(doc.selection(), before);
            doc.redo();
            assert_eq!(
                doc.selection(),
                Selection {
                    anchor: after.0,
                    head: after.1
                }
            );
        }
    }

    /// Deterministic edits mixing line endings, combining marks, and flags.
    fn scrambled_edits(mut check: impl FnMut(&Document)) {
        let pieces = [
            "", "a", "\n", "\r", "\r\n", "e\u{301}", "\u{301}", "🇺", "🇸", "界", "x\ny", "\r\r\n\n",
        ];
        let mut seed = 0x9e37_79b9_7f4a_7c15_u64;
        let mut next = |bound: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % bound as u64) as usize
        };
        let mut doc = Document::new("one\r\ntwo\rthree\n\nfour");
        for step in 0..600 {
            let len = doc.text().len();
            let a = doc.floor_grapheme_boundary(next(len + 1));
            let b = doc.floor_grapheme_boundary(next(len + 1));
            let (start, end) = (a.min(b), a.max(b));
            match step % 7 {
                5 => {
                    doc.undo();
                }
                6 => {
                    doc.redo();
                }
                _ => {
                    let _ = doc.replace_range(start..end, pieces[next(pieces.len())]);
                }
            }
            check(&doc);
        }
    }

    #[test]
    fn line_index_follows_every_edit_undo_and_redo() {
        scrambled_edits(|doc| {
            let fresh = LineIndex::new(doc.text());
            assert_eq!(doc.lines.starts, fresh.starts, "{:?}", doc.text());
            let text = doc.text();
            for line in 0..doc.line_count() {
                let range = doc.line_content(line);
                assert!(!text[range.clone()].contains(['\n', '\r']));
                assert_eq!(doc.line_index(range.start), line);
                assert_eq!(doc.line_range(range.end), range);
            }
            let lines = text
                .split("\r\n")
                .flat_map(|l| l.split(['\n', '\r']))
                .count();
            assert_eq!(doc.line_count(), lines);
        });
    }

    #[test]
    fn local_grapheme_boundaries_match_whole_text_segmentation() {
        scrambled_edits(|doc| {
            let text = doc.text();
            let boundaries: Vec<_> = text
                .grapheme_indices(true)
                .map(|(i, _)| i)
                .chain([text.len()])
                .collect();
            for offset in 0..=text.len() + 1 {
                let floor = *boundaries.iter().rfind(|&&b| b <= offset).unwrap();
                assert_eq!(
                    doc.is_grapheme_boundary(offset),
                    boundaries.contains(&offset)
                );
                assert_eq!(doc.floor_grapheme_boundary(offset), floor);
                let ceil = boundaries
                    .iter()
                    .copied()
                    .find(|&b| b >= offset)
                    .unwrap_or(text.len());
                assert_eq!(doc.ceil_grapheme_boundary(offset), ceil);
            }
        });
    }

    #[test]
    fn edit_log_reports_the_first_changed_offset() {
        let mut doc = Document::new("alpha\nbeta\ngamma\n");
        let start = doc.revision();
        assert_eq!(doc.first_change_since(start), None);
        doc.set_caret(11).unwrap();
        doc.type_text("G");
        doc.set_caret(6).unwrap();
        doc.type_text("B");
        assert_eq!(doc.first_change_since(start), Some(6));
        assert_eq!(doc.first_change_since(start + 1), Some(6));
        let after = doc.revision();
        doc.undo();
        assert_eq!(doc.first_change_since(after), Some(6));
        for _ in 0..EDIT_LOG {
            doc.insert("z");
        }
        assert_eq!(doc.first_change_since(start), Some(0));
    }

    #[test]
    fn edit_commands_round_trip_at_every_grapheme_boundary_in_corpus() {
        let sources = [
            include_str!("../tests/core/fixtures/interactions.md"),
            "\u{feff}- [X] café\r\n  - e\u{301} 👩🏽‍💻\n\tend  ",
            "> - quoted\n>   - child\n>     - ",
            "# Title\n\nTitle\n---\n\n- parent\n  - ",
            "```md\n- [ ] code\n```\n\n    1. literal\n",
            "- **bold** and `inline`\n  continuation\r\n",
            "| a | b |\n| - | - |\n| e\u{301} | 🇺🇸 |\n",
            "-\t[x]\ttext\r> - [ ]\r\n- - \n",
            "🇺x🇸\r\ne\u{301}界\u{1b}[0m",
        ];
        for source in sources {
            let boundaries = source
                .grapheme_indices(true)
                .map(|(i, _)| i)
                .chain(std::iter::once(source.len()));
            for offset in boundaries {
                for command in 0..6 {
                    let mut doc = Document::new(source);
                    doc.set_caret(offset).unwrap();
                    let changed = match command {
                        0 => doc.insert("\u{301}🇨🇦\r\n- [x] pasted\n"),
                        1 => doc.backspace(),
                        2 => doc.delete_forward(),
                        3 => doc.enter(),
                        4 => doc.literal_newline(),
                        _ => doc.toggle_task_at_caret(),
                    };
                    assert!(
                        doc.is_grapheme_boundary(doc.selection().head),
                        "{source:?} at {offset}, command {command}"
                    );
                    assert!(doc.is_grapheme_boundary(doc.selection().anchor));
                    if changed {
                        let result = doc.text().to_owned();
                        assert!(doc.undo());
                        assert_eq!(doc.text(), source);
                        assert_eq!(doc.selection(), Selection::caret(offset));
                        assert!(!doc.is_dirty());
                        assert!(!doc.can_undo());
                        assert!(doc.redo());
                        assert_eq!(doc.text(), result);
                    } else {
                        assert_eq!(doc.text(), source);
                        assert!(!doc.can_undo());
                    }
                }
            }
        }
    }
}

impl Selection {
    pub const fn caret(offset: usize) -> Self {
        Self {
            anchor: offset,
            head: offset,
        }
    }

    pub fn range(self) -> Range<usize> {
        self.anchor.min(self.head)..self.anchor.max(self.head)
    }

    pub fn is_empty(self) -> bool {
        self.anchor == self.head
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditError {
    InvalidBoundary(usize),
    InvalidRange,
    StaleRevision { expected: u64, actual: u64 },
    InvalidTask,
    HistoryUnavailable,
}

impl fmt::Display for EditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidBoundary(offset) => {
                write!(f, "byte offset {offset} is not a grapheme boundary")
            }
            Self::InvalidRange => write!(f, "source range is reversed"),
            Self::StaleRevision { expected, actual } => write!(
                f,
                "stale task revision {expected}; current revision is {actual}"
            ),
            Self::InvalidTask => write!(f, "source range is not a current task marker"),
            Self::HistoryUnavailable => write!(
                f,
                "Reload cannot retain your current text in Undo within the history limits; save it separately first"
            ),
        }
    }
}

impl std::error::Error for EditError {}

#[derive(Clone, Debug)]
struct State {
    text: String,
    selection: Selection,
}

/// Limits for the combined undo and redo history. Zero disables retention.
/// Bytes count retained String capacities and State sizes, excluding the live
/// document, saved baseline, and allocator/collection bookkeeping.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HistoryLimits {
    pub max_entries: usize,
    pub max_bytes: usize,
}

impl Default for HistoryLimits {
    fn default() -> Self {
        Self {
            max_entries: 256,
            max_bytes: 32 * 1024 * 1024,
        }
    }
}

/// An authoritative UTF-8 buffer with bounded snapshot history. Commands each
/// create one undo step; only explicit `type_text` calls may coalesce.
/// Current text and the saved baseline are never discarded by history limits.
#[derive(Clone, Debug)]
pub struct Document {
    state: State,
    saved_text: String,
    has_saved_baseline: bool,
    revision: u64,
    undo: VecDeque<State>,
    redo: VecDeque<State>,
    history_limits: HistoryLimits,
    typing_end: Option<usize>,
    lines: LineIndex,
    /// The first changed byte of each recent revision, oldest first.
    edits: VecDeque<(u64, usize)>,
    id: u64,
}

impl Document {
    pub fn new(text: impl Into<String>) -> Self {
        static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let text = text.into();
        Self {
            id: NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            saved_text: text.clone(),
            has_saved_baseline: true,
            state: State {
                text,
                selection: Selection::default(),
            },
            revision: 0,
            undo: VecDeque::new(),
            redo: VecDeque::new(),
            history_limits: HistoryLimits::default(),
            typing_end: None,
            lines: LineIndex::new(""),
            edits: VecDeque::new(),
        }
        .with_lines()
    }

    fn with_lines(mut self) -> Self {
        self.lines = LineIndex::new(&self.state.text);
        self
    }

    /// Create file-detached content with no accepted saved baseline, including
    /// an empty recovered buffer. No fake edit or undo entry is introduced.
    pub fn unsaved(text: impl Into<String>) -> Self {
        let mut document = Self::new(text);
        document.saved_text = String::new();
        document.has_saved_baseline = false;
        document
    }

    /// Accept fresh disk content as the saved baseline. A changed source is one
    /// independent undo step, preserving the old directional selection. Undo
    /// restores old source as a local edit against the newly accepted baseline.
    /// Selection offsets clamp downward to valid grapheme boundaries in the new
    /// source. Reject replacements whose prior snapshot cannot be retained.
    pub fn reload_saved(&mut self, text: String) -> Result<bool, EditError> {
        self.break_undo_group();
        let changed = text != self.state.text;
        if changed {
            let previous = self.state.clone();
            if self.history_limits.max_entries == 0
                || previous.text.capacity() + std::mem::size_of::<State>()
                    > self.history_limits.max_bytes
            {
                return Err(EditError::HistoryUnavailable);
            }
            let clamp = |offset: usize| floor_boundary(&text, offset);
            let selection = Selection {
                anchor: clamp(self.selection().anchor),
                head: clamp(self.selection().head),
            };
            self.undo.push_back(previous);
            self.state = State { text, selection };
            self.redo.clear();
            self.revision += 1;
            let (start, old_end, new_end) = delta(&self.undo.back().unwrap().text, self.text());
            self.lines.edit(&self.state.text, start, old_end, new_end);
            self.log_edit(start);
            self.trim_history();
        }
        self.mark_saved();
        Ok(changed)
    }

    pub fn text(&self) -> &str {
        &self.state.text
    }
    pub fn selection(&self) -> Selection {
        self.state.selection
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    /// Distinguishes documents, whose revisions each start at zero, so a
    /// cache keyed by `(id, revision)` never mistakes one for another.
    pub fn id(&self) -> u64 {
        self.id
    }
    pub fn is_dirty(&self) -> bool {
        !self.has_saved_baseline || self.state.text != self.saved_text
    }
    pub fn mark_saved(&mut self) {
        self.break_undo_group();
        self.saved_text.clone_from(&self.state.text);
        self.has_saved_baseline = true;
    }
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
    pub fn selected_text(&self) -> &str {
        &self.text()[self.selection().range()]
    }

    /// The source line containing `offset`, without its line ending. CRLF,
    /// LF, and a lone CR each end a line.
    pub fn line_range(&self, offset: usize) -> Range<usize> {
        self.line_content(self.line_index(offset))
    }

    /// Source lines, counting an empty line after a final line ending.
    pub fn line_count(&self) -> usize {
        self.lines.starts.len()
    }

    /// The zero-based line containing `offset`, clamped to the text.
    pub fn line_index(&self, offset: usize) -> usize {
        self.lines.line(offset.min(self.text().len()))
    }

    /// Where zero-based `line` begins, clamped to the last line.
    pub fn line_start(&self, line: usize) -> usize {
        self.lines.starts[line.min(self.lines.starts.len() - 1)]
    }

    /// Zero-based `line` without its ending, clamped to the last line.
    pub fn line_content(&self, line: usize) -> Range<usize> {
        let line = line.min(self.lines.starts.len() - 1);
        let start = self.lines.starts[line];
        let next = self
            .lines
            .starts
            .get(line + 1)
            .copied()
            .unwrap_or(self.text().len());
        let ending = &self.text().as_bytes()[start..next];
        let end = next
            - if ending.ends_with(b"\r\n") {
                2
            } else {
                usize::from(ending.ends_with(b"\n") || ending.ends_with(b"\r"))
            };
        start..end
    }

    /// The lowest byte offset that edits since `revision` may have changed:
    /// `None` when nothing changed, and zero when the edits are not recorded.
    pub fn first_change_since(&self, revision: u64) -> Option<usize> {
        if revision >= self.revision {
            return None;
        }
        match self.edits.front() {
            Some((oldest, _)) if *oldest <= revision + 1 => self
                .edits
                .iter()
                .filter(|(at, _)| *at > revision)
                .map(|(_, start)| *start)
                .min(),
            _ => Some(0),
        }
    }

    fn log_edit(&mut self, start: usize) {
        if self.edits.len() == EDIT_LOG {
            self.edits.pop_front();
        }
        self.edits.push_back((self.revision, start));
    }

    /// Find case-sensitive, nonoverlapping literal source matches. A match
    /// must start and end at extended-grapheme boundaries; empty queries match
    /// nothing. Markdown syntax and link destinations are searched verbatim.
    pub fn find_matches(&self, query: &str) -> Vec<Range<usize>> {
        if query.is_empty() {
            return Vec::new();
        }
        let mut matches = Vec::new();
        let mut cursor = 0;
        while let Some(relative) = self.text()[cursor..].find(query) {
            let start = cursor + relative;
            let end = start + query.len();
            if self.is_grapheme_boundary(start) && self.is_grapheme_boundary(end) {
                matches.push(start..end);
                cursor = end;
            } else {
                // A rejected partial-grapheme match must not hide a later
                // overlapping match whose endpoints are both valid.
                cursor = start + self.text()[start..].chars().next().unwrap().len_utf8();
            }
        }
        matches
    }

    /// Replace all accepted source matches in one undo transaction. Returns
    /// the match count, even if replacement equals query (a complete no-op).
    /// Replacement text is literal and is never searched again in this command.
    /// Selection starts inside a replaced span map to its new start, selection
    /// ends/carets inside it map to its new end; exact boundaries stay anchored.
    /// Nonempty selections expand to whole graphemes if adjacent text combines.
    pub fn replace_all(&mut self, query: &str, replacement: &str) -> usize {
        self.break_undo_group();
        let matches = self.find_matches(query);
        let count = matches.len();
        if count == 0 || query == replacement {
            return count;
        }
        let selection = self.selection();
        let map_offset = |offset: usize, toward_end: bool| {
            let mut old_end = 0;
            let mut new_end = 0;
            for range in &matches {
                let new_start = new_end + range.start - old_end;
                if offset <= range.start {
                    return new_end + offset - old_end;
                }
                if offset < range.end {
                    return new_start + if toward_end { replacement.len() } else { 0 };
                }
                old_end = range.end;
                new_end = new_start + replacement.len();
            }
            new_end + offset - old_end
        };
        let after = Selection {
            anchor: map_offset(selection.anchor, selection.anchor >= selection.head),
            head: map_offset(selection.head, selection.head >= selection.anchor),
        };
        let mut text = String::new();
        let mut cursor = 0;
        for range in matches {
            text.push_str(&self.text()[cursor..range.start]);
            text.push_str(replacement);
            cursor = range.end;
        }
        text.push_str(&self.text()[cursor..]);
        self.replace_with_selection(0..self.text().len(), &text, after)
            .expect("whole document has grapheme boundaries");
        if !after.is_empty() {
            // A new combining mark or regional indicator can merge across a
            // replacement boundary. Preserve the selected span's direction.
            self.state.selection = if after.anchor < after.head {
                Selection {
                    anchor: self.floor_grapheme_boundary(after.anchor),
                    head: self.ceil_grapheme_boundary(after.head),
                }
            } else {
                Selection {
                    anchor: self.ceil_grapheme_boundary(after.anchor),
                    head: self.floor_grapheme_boundary(after.head),
                }
            };
        }
        count
    }

    pub fn set_selection(&mut self, selection: Selection) -> Result<(), EditError> {
        self.validate_boundary(selection.anchor)?;
        self.validate_boundary(selection.head)?;
        self.break_undo_group();
        self.state.selection = selection;
        Ok(())
    }

    pub fn set_caret(&mut self, offset: usize) -> Result<(), EditError> {
        self.set_selection(Selection::caret(offset))
    }

    pub fn select_all(&mut self) {
        self.break_undo_group();
        self.state.selection = Selection {
            anchor: 0,
            head: self.text().len(),
        };
    }

    pub fn is_grapheme_boundary(&self, offset: usize) -> bool {
        is_boundary(self.text(), offset)
    }

    /// The nearest grapheme boundary at or before `offset`.
    pub fn floor_grapheme_boundary(&self, offset: usize) -> usize {
        floor_boundary(self.text(), offset)
    }

    fn ceil_grapheme_boundary(&self, offset: usize) -> usize {
        let text = self.text();
        if offset >= text.len() {
            return text.len();
        }
        if is_boundary(text, offset) {
            return offset;
        }
        next_boundary(text, floor_boundary(text, offset))
    }

    fn validate_boundary(&self, offset: usize) -> Result<(), EditError> {
        if self.is_grapheme_boundary(offset) {
            Ok(())
        } else {
            Err(EditError::InvalidBoundary(offset))
        }
    }

    pub fn move_left(&mut self, extend: bool) {
        self.break_undo_group();
        let selection = self.selection();
        let head = if !extend && !selection.is_empty() {
            selection.range().start
        } else {
            previous_boundary(self.text(), selection.head)
        };
        self.state.selection = Selection {
            anchor: if extend { selection.anchor } else { head },
            head,
        };
    }

    pub fn move_right(&mut self, extend: bool) {
        self.break_undo_group();
        let selection = self.selection();
        let head = if !extend && !selection.is_empty() {
            selection.range().end
        } else {
            next_boundary(self.text(), selection.head)
        };
        self.state.selection = Selection {
            anchor: if extend { selection.anchor } else { head },
            head,
        };
    }

    /// Insert literal bytes, replacing the selection. Paste never invokes Enter helpers.
    pub fn insert(&mut self, text: &str) -> bool {
        self.replace_range(self.selection().range(), text)
            .expect("valid document selection")
    }

    /// One undoable source replacement; the caret ends after the inserted text.
    pub fn replace_range(&mut self, range: Range<usize>, text: &str) -> Result<bool, EditError> {
        let caret = range.start.saturating_add(text.len());
        self.replace_with_selection(range, text, Selection::caret(caret))
    }

    pub(crate) fn replace_with_selection(
        &mut self,
        range: Range<usize>,
        text: &str,
        after: Selection,
    ) -> Result<bool, EditError> {
        self.break_undo_group();
        self.apply_replacement(range, text, after, false)
    }

    fn apply_replacement(
        &mut self,
        range: Range<usize>,
        text: &str,
        after: Selection,
        coalesce: bool,
    ) -> Result<bool, EditError> {
        if range.start > range.end {
            return Err(EditError::InvalidRange);
        }
        self.validate_boundary(range.start)?;
        self.validate_boundary(range.end)?;
        let changed = &self.text()[range.clone()] != text;
        if changed {
            if !coalesce {
                self.undo.push_back(self.state.clone());
            }
            self.state.text.replace_range(range.clone(), text);
            self.redo.clear();
            self.trim_history();
            self.revision += 1;
            let new_end = range.start + text.len();
            self.lines
                .edit(&self.state.text, range.start, range.end, new_end);
            self.log_edit(range.start);
        }
        // Inserting/deleting a combining sequence can merge adjacent graphemes.
        // The resulting caret must not remain inside that newly formed grapheme.
        self.state.selection = if after.anchor < after.head {
            Selection {
                anchor: self.floor_grapheme_boundary(after.anchor),
                head: self.ceil_grapheme_boundary(after.head),
            }
        } else if after.anchor > after.head {
            Selection {
                anchor: self.ceil_grapheme_boundary(after.anchor),
                head: self.floor_grapheme_boundary(after.head),
            }
        } else {
            Selection::caret(self.ceil_grapheme_boundary(after.head))
        };
        Ok(changed)
    }

    pub fn backspace(&mut self) -> bool {
        let selection = self.selection();
        if !selection.is_empty() {
            return self.insert("");
        }
        let start = previous_boundary(self.text(), selection.head);
        self.replace_range(start..selection.head, "")
            .expect("grapheme boundary")
    }

    pub fn delete_forward(&mut self) -> bool {
        let selection = self.selection();
        if !selection.is_empty() {
            return self.insert("");
        }
        let end = next_boundary(self.text(), selection.head);
        self.replace_range(selection.head..end, "")
            .expect("grapheme boundary")
    }

    pub fn enter(&mut self) -> bool {
        self.break_undo_group();
        crate::editing::enter(self)
    }

    /// `enter`, reusing `snapshot` when it is from the current revision
    /// rather than parsing the document again.
    pub fn enter_with(&mut self, snapshot: &MarkdownSnapshot) -> bool {
        self.break_undo_group();
        crate::editing::enter_with(self, Some(snapshot))
    }
    pub fn literal_newline(&mut self) -> bool {
        let newline =
            crate::editing::preferred_newline(self.text(), self.selection().range().start);
        self.insert(newline)
    }

    pub fn undo(&mut self) -> bool {
        self.break_undo_group();
        let Some(previous) = self.undo.pop_back() else {
            return false;
        };
        self.redo
            .push_back(std::mem::replace(&mut self.state, previous));
        let (start, old_end, new_end) = delta(&self.redo.back().unwrap().text, self.text());
        self.trim_history();
        self.revision += 1;
        self.lines.edit(&self.state.text, start, old_end, new_end);
        self.log_edit(start);
        true
    }

    pub fn redo(&mut self) -> bool {
        self.break_undo_group();
        let Some(next) = self.redo.pop_back() else {
            return false;
        };
        self.undo
            .push_back(std::mem::replace(&mut self.state, next));
        let (start, old_end, new_end) = delta(&self.undo.back().unwrap().text, self.text());
        self.trim_history();
        self.revision += 1;
        self.lines.edit(&self.state.text, start, old_end, new_end);
        self.log_edit(start);
        true
    }

    /// End a typing transaction before UI commands such as view/tab changes.
    /// Navigation, selection changes, saving and semantic edit APIs do this too.
    pub fn break_undo_group(&mut self) {
        self.typing_end = None;
    }

    /// Insert keyboard text, grouping contiguous non-whitespace calls. Whitespace
    /// calls (including mixed/multiline strings) are standalone transactions.
    /// Replacing a selection is standalone. No time heuristic is used: UI callers
    /// explicitly break groups for commands; literal paste must use `insert`.
    pub fn type_text(&mut self, text: &str) -> bool {
        let selection = self.selection();
        let groupable =
            !text.is_empty() && !text.chars().any(char::is_whitespace) && selection.is_empty();
        let coalesce =
            groupable && self.typing_end == Some(selection.head) && !self.undo.is_empty();
        self.break_undo_group();
        let after = Selection::caret(selection.range().start.saturating_add(text.len()));
        let changed = self
            .apply_replacement(selection.range(), text, after, coalesce)
            .expect("valid document selection");
        if changed && groupable {
            self.typing_end = Some(self.selection().head);
        }
        changed
    }

    /// Apply limits immediately. Evict the oldest undo snapshots first, then the
    /// farthest redo snapshots. An oversized snapshot cannot be retained, so an
    /// edit/undo of an oversized document may not have an inverse in history.
    pub fn set_history_limits(&mut self, limits: HistoryLimits) {
        self.break_undo_group();
        self.history_limits = limits;
        self.trim_history();
    }

    pub fn history_limits(&self) -> HistoryLimits {
        self.history_limits
    }

    fn history_bytes(&self) -> usize {
        self.undo
            .iter()
            .chain(&self.redo)
            .map(|state| state.text.capacity() + std::mem::size_of::<State>())
            .sum()
    }

    fn trim_history(&mut self) {
        let mut bytes = self.history_bytes();
        while self.undo.len() + self.redo.len() > self.history_limits.max_entries
            || bytes > self.history_limits.max_bytes
        {
            let Some(oldest) = self.undo.pop_front().or_else(|| self.redo.pop_front()) else {
                break;
            };
            bytes -= oldest.text.capacity() + std::mem::size_of::<State>();
        }
    }

    /// Skip adjacent whitespace, then one Unicode word-boundary segment.
    /// Punctuation and emoji are stops too; endpoints are whole graphemes.
    pub fn move_word_left(&mut self, extend: bool) {
        let selection = self.selection();
        let head = if !extend && !selection.is_empty() {
            selection.range().start
        } else {
            self.word_left(selection.head)
        };
        self.set_selection(Selection {
            anchor: if extend { selection.anchor } else { head },
            head,
        })
        .expect("word boundary is a grapheme boundary");
    }

    /// Rightward counterpart of `move_word_left`.
    pub fn move_word_right(&mut self, extend: bool) {
        let selection = self.selection();
        let head = if !extend && !selection.is_empty() {
            selection.range().end
        } else {
            self.word_right(selection.head)
        };
        self.set_selection(Selection {
            anchor: if extend { selection.anchor } else { head },
            head,
        })
        .expect("word boundary is a grapheme boundary");
    }

    fn word_left(&self, offset: usize) -> usize {
        let target = self.text()[..offset]
            .split_word_bound_indices()
            .rev()
            .find(|(_, segment)| !segment.chars().all(char::is_whitespace))
            .map_or(0, |(start, _)| start);
        self.floor_grapheme_boundary(target)
    }

    fn word_right(&self, offset: usize) -> usize {
        let target = self.text()[offset..]
            .split_word_bound_indices()
            .find(|(_, segment)| !segment.chars().all(char::is_whitespace))
            .map_or(self.text().len(), |(start, segment)| {
                offset + start + segment.len()
            });
        self.ceil_grapheme_boundary(target)
    }

    /// The word a double click at `offset` selects: the Unicode word segment
    /// that starts at or holds `offset` on its line, so `can't` and `3.14`
    /// stay whole, split further at `.` and `:` unless digits surround them,
    /// so `self.name` selects one name. Whitespace and punctuation select
    /// their own run. Past the last character, the line's last segment.
    pub fn word_at(&self, offset: usize) -> Range<usize> {
        let text = self.text();
        let offset = self.floor_grapheme_boundary(offset.min(text.len()));
        let line = self.line_range(offset);
        let mut last = offset..offset;
        for (start, segment) in text[line.clone()].split_word_bound_indices() {
            let range = line.start + start..line.start + start + segment.len();
            if offset < range.end {
                last = range;
                break;
            }
            last = range;
        }
        let bytes = text.as_bytes();
        let digit = |at: usize| bytes.get(at).is_some_and(u8::is_ascii_digit);
        let mut start = last.start;
        for at in last.clone() {
            let separator = matches!(bytes[at], b'.' | b':')
                && !(at > last.start && digit(at - 1) && digit(at + 1));
            if !separator {
                continue;
            }
            if offset < at {
                return start..at;
            }
            if offset == at {
                return at..at + 1;
            }
            start = at + 1;
        }
        start..last.end
    }

    /// The source line holding `offset` with its line ending, as a triple
    /// click selects it.
    pub fn line_with_ending(&self, offset: usize) -> Range<usize> {
        let line = self.line_range(offset.min(self.text().len()));
        let ending = match &self.text().as_bytes()[line.end..] {
            [b'\r', b'\n', ..] => 2,
            [b'\r' | b'\n', ..] => 1,
            _ => 0,
        };
        line.start..line.end + ending
    }

    pub fn delete_word_backward(&mut self) -> bool {
        let selection = self.selection();
        let range = if selection.is_empty() {
            self.word_left(selection.head)..selection.head
        } else {
            selection.range()
        };
        self.replace_range(range, "")
            .expect("valid word boundaries")
    }

    pub fn delete_word_forward(&mut self) -> bool {
        let selection = self.selection();
        let range = if selection.is_empty() {
            selection.head..self.word_right(selection.head)
        } else {
            selection.range()
        };
        self.replace_range(range, "")
            .expect("valid word boundaries")
    }

    /// Indent affected physical source lines by four spaces in one transaction.
    pub fn indent_lines(&mut self) -> bool {
        self.indent_lines_by(crate::editing::Indent::Spaces(4))
    }

    /// Remove up to four leading spaces or one leading tab from affected lines.
    pub fn outdent_lines(&mut self) -> bool {
        self.outdent_lines_by(crate::editing::Indent::Spaces(4))
    }

    /// Indent affected physical source lines by one level of `indent`.
    pub fn indent_lines_by(&mut self, indent: crate::editing::Indent) -> bool {
        crate::editing::indent_lines(self, false, indent)
    }

    /// Remove one leading tab, or up to one level of leading spaces, from
    /// affected lines.
    pub fn outdent_lines_by(&mut self, indent: crate::editing::Indent) -> bool {
        crate::editing::indent_lines(self, true, indent)
    }

    /// Move affected source lines up once, preserving separators and selection.
    pub fn move_lines_up(&mut self) -> bool {
        crate::editing::rearrange_lines(self, true, false)
    }

    /// Move affected source lines down once, preserving separators and selection.
    pub fn move_lines_down(&mut self) -> bool {
        crate::editing::rearrange_lines(self, false, false)
    }

    /// Duplicate affected source lines above and select the new copy.
    pub fn duplicate_lines_up(&mut self) -> bool {
        crate::editing::rearrange_lines(self, true, true)
    }

    /// Duplicate affected source lines below and select the new copy.
    pub fn duplicate_lines_down(&mut self) -> bool {
        crate::editing::rearrange_lines(self, false, true)
    }

    pub fn toggle_inline(&mut self, style: crate::editing::InlineStyle) -> bool {
        crate::editing::toggle_inline(self, style)
    }

    pub fn markdown(&self) -> MarkdownSnapshot {
        markdown::analyze(self.text(), self.revision)
    }

    /// Toggle only the state character, leaving caret/selection anchored.
    pub fn toggle_task(&mut self, task: &Task) -> Result<bool, EditError> {
        self.break_undo_group();
        if task.revision != self.revision {
            return Err(EditError::StaleRevision {
                expected: task.revision,
                actual: self.revision,
            });
        }
        if !self.markdown().tasks.contains(task) {
            return Err(EditError::InvalidTask);
        }
        self.replace_with_selection(
            task.marker_range.clone(),
            if task.checked { " " } else { "x" },
            self.selection(),
        )
    }

    pub fn toggle_task_at_caret(&mut self) -> bool {
        self.break_undo_group();
        let caret = self.selection().head;
        let task = self
            .markdown()
            .tasks
            .into_iter()
            .filter(|task| {
                task.item_range.contains(&caret)
                    || (caret == self.text().len() && task.item_range.end == caret)
            })
            .min_by_key(|task| task.item_range.len());
        task.is_some_and(|task| self.toggle_task(&task).unwrap_or(false))
    }
}

/// The changed span between two texts, as `(start, old_end, new_end)`.
fn delta(old: &str, new: &str) -> (usize, usize, usize) {
    const BLOCK: usize = 4096;
    let (old, new) = (old.as_bytes(), new.as_bytes());
    let shortest = old.len().min(new.len());
    // Equal blocks compare at memcmp speed; the last block byte by byte.
    let mut prefix = 0;
    while prefix + BLOCK <= shortest && old[prefix..prefix + BLOCK] == new[prefix..prefix + BLOCK] {
        prefix += BLOCK;
    }
    prefix += old[prefix..shortest]
        .iter()
        .zip(&new[prefix..shortest])
        .take_while(|(a, b)| a == b)
        .count();
    let room = shortest - prefix;
    let mut suffix = 0;
    while suffix + BLOCK <= room
        && old[old.len() - suffix - BLOCK..old.len() - suffix]
            == new[new.len() - suffix - BLOCK..new.len() - suffix]
    {
        suffix += BLOCK;
    }
    suffix += old[..old.len() - suffix]
        .iter()
        .rev()
        .zip(new[..new.len() - suffix].iter().rev())
        .take(room - suffix)
        .take_while(|(a, b)| a == b)
        .count();
    (prefix, old.len() - suffix, new.len() - suffix)
}

/// Whether `offset` is an extended grapheme boundary of `text`. Boundaries
/// come from local context, so finding one costs the same anywhere in a
/// document; between two ASCII characters other than CRLF there always is
/// one.
pub fn is_boundary(text: &str, offset: usize) -> bool {
    let bytes = text.as_bytes();
    if offset == 0 || offset == bytes.len() {
        return true;
    }
    if offset > bytes.len() {
        return false;
    }
    let (before, after) = (bytes[offset - 1], bytes[offset]);
    if before.is_ascii() && after.is_ascii() {
        return !(before == b'\r' && after == b'\n');
    }
    text.is_char_boundary(offset)
        && GraphemeCursor::new(offset, text.len(), true)
            .is_boundary(text, 0)
            .unwrap_or(false)
}

fn floor_boundary(text: &str, offset: usize) -> usize {
    if offset >= text.len() {
        return text.len();
    }
    let mut at = offset;
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    if is_boundary(text, at) {
        at
    } else {
        previous_boundary(text, at)
    }
}

/// The last grapheme boundary before `offset`, or zero.
fn previous_boundary(text: &str, offset: usize) -> usize {
    if offset == 0 {
        return 0;
    }
    GraphemeCursor::new(offset.min(text.len()), text.len(), true)
        .prev_boundary(text, 0)
        .ok()
        .flatten()
        .unwrap_or(0)
}

/// The first grapheme boundary after `offset`, or the end of the text.
fn next_boundary(text: &str, offset: usize) -> usize {
    if offset >= text.len() {
        return text.len();
    }
    GraphemeCursor::new(offset, text.len(), true)
        .next_boundary(text, 0)
        .ok()
        .flatten()
        .unwrap_or(text.len())
}

#[cfg(test)]
#[path = "document_commands_tests.rs"]
mod command_tests;
