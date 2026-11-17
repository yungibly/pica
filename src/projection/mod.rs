//! A conservative, source-anchored projection. Unrecognized syntax stays
//! visible. Decoration without source — table grids, hanging indents, row
//! surfaces — maps to neighbouring source and is never an insertion target.
mod layout;
mod parse;
#[cfg(test)]
mod tests;

#[cfg(test)]
use milli::Selection;
use milli::markdown::Task;
pub use parse::{OutlineEntry, Parsed, code_styles};
use ratatui::style::{Color, Style};
use std::{fmt, ops::Range};

/// Tables with more rows than this stay literal source, so one table never
/// costs more than this to lay out.
pub const TABLE_ROW_LIMIT: usize = 200;

/// A glyph's display text. Graphemes are short, so they are stored inline
/// without a heap allocation; longer text, such as a table rule, is boxed.
#[derive(Clone, PartialEq, Eq)]
pub struct GlyphText(Repr);

#[derive(Clone, PartialEq, Eq)]
enum Repr {
    Inline { len: u8, bytes: [u8; 22] },
    Heap(Box<str>),
}

impl GlyphText {
    pub fn new(text: &str) -> Self {
        if text.len() <= 22 {
            let mut bytes = [0; 22];
            bytes[..text.len()].copy_from_slice(text.as_bytes());
            Self(Repr::Inline {
                len: text.len() as u8,
                bytes,
            })
        } else {
            Self(Repr::Heap(text.into()))
        }
    }

    pub fn as_str(&self) -> &str {
        match &self.0 {
            Repr::Inline { len, bytes } => {
                std::str::from_utf8(&bytes[..usize::from(*len)]).expect("copied from a str")
            }
            Repr::Heap(text) => text,
        }
    }
}

impl std::ops::Deref for GlyphText {
    type Target = str;

    fn deref(&self) -> &str {
        self.as_str()
    }
}

impl AsRef<str> for GlyphText {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl From<&str> for GlyphText {
    fn from(text: &str) -> Self {
        Self::new(text)
    }
}

impl PartialEq<str> for GlyphText {
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}

impl PartialEq<&str> for GlyphText {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

impl fmt::Debug for GlyphText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.as_str(), f)
    }
}

/// Where a window of rows begins and how far it reaches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Window {
    /// A restart point: the start of a line that begins a visual row, where
    /// layout can begin afresh. See `Parsed::restarts_at`.
    pub start: usize,
    /// One-based source line number at `start`.
    pub line: usize,
    /// The window holds the row containing this offset…
    pub through: usize,
    /// …and at least this many rows after it, ending where a row begins.
    pub after: usize,
    /// Begin at a later row of the line at `start` instead of its first,
    /// in literal (source) layout only. See `Resume`.
    pub resume: Option<Resume>,
}

/// A row inside one long line where a window may begin: the row's number
/// within the line and its source offset, from an earlier layout of the same
/// line. Literal rows break by width alone, so the rows before an edit keep
/// their starts and layout can skip them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Resume {
    pub row: usize,
    pub offset: usize,
}

impl Window {
    /// Every row of the source, for tests and comparisons.
    #[cfg(test)]
    pub const WHOLE: Self = Self {
        start: 0,
        line: 1,
        through: usize::MAX,
        after: usize::MAX,
        resume: None,
    };
}

#[derive(Clone, Debug)]
pub struct Glyph {
    pub text: GlyphText,
    /// Empty for inserted decoration, which never takes selection or matches.
    pub source: Range<usize>,
    pub column: usize,
    pub width: usize,
    pub style: Style,
    pub task: Option<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FillKind {
    /// A solid surface behind the row's text.
    Band,
    /// A thin horizontal line, for thematic breaks and the edges of code
    /// blocks where the theme has no surface color.
    Rule,
}

/// A row surface drawn beneath glyphs, from `from` to the right text edge.
#[derive(Clone, Debug, PartialEq)]
pub struct Fill {
    /// Relative to the text column; negative values hang into the margin.
    pub from: isize,
    pub kind: FillKind,
    pub color: Color,
    /// Drawn at `from`, such as a code language tab or a heading edge.
    pub label: Option<(String, Style)>,
}

#[derive(Clone, Debug)]
pub struct VisualRow {
    pub glyphs: Vec<Glyph>,
    pub start: usize,
    pub end: usize,
    /// One-based source line, on the first visual row of each line only.
    pub line: Option<usize>,
    pub fill: Option<Fill>,
    /// False for rows of pure decoration, which vertical movement skips.
    pub navigable: bool,
    /// Layout starting afresh at `start` would begin with this row.
    pub restart: bool,
}

impl VisualRow {
    fn new(start: usize, line: Option<usize>) -> Self {
        Self {
            glyphs: Vec::new(),
            start,
            end: start,
            line,
            fill: None,
            navigable: true,
            restart: false,
        }
    }
}

/// Visual rows for a window of the source, from a restart point to a line
/// end. Row numbers are local to the window.
#[derive(Clone, Debug)]
pub struct Projection {
    rows: Vec<VisualRow>,
    pub revision: u64,
    positions: Positions,
    /// Source covered: `start..end`, and whether `end` is the source's end.
    start: usize,
    end: usize,
    complete: bool,
    /// The row number within its line of the first row, when a window
    /// resumed inside a long line.
    first_row: usize,
}

impl Default for Projection {
    /// One empty row, before anything has been laid out.
    fn default() -> Self {
        let mut row = VisualRow::new(0, Some(1));
        row.restart = true;
        Self {
            rows: vec![row],
            revision: 0,
            positions: Positions(vec![(0, (0, 0))]),
            start: 0,
            end: 0,
            complete: false,
            first_row: 0,
        }
    }
}

/// Caret cells by source offset. Layout records them in source order, so
/// recording almost always appends or refines the latest entry.
#[derive(Clone, Debug, Default)]
struct Positions(Vec<(usize, (usize, usize))>);

impl Positions {
    fn insert(&mut self, offset: usize, cell: (usize, usize)) {
        match self.0.last_mut() {
            Some((last, value)) if *last == offset => *value = cell,
            Some((last, _)) if *last > offset => {
                match self.0.binary_search_by_key(&offset, |(at, _)| *at) {
                    Ok(index) => self.0[index].1 = cell,
                    Err(index) => self.0.insert(index, (offset, cell)),
                }
            }
            _ => self.0.push((offset, cell)),
        }
    }

    /// The cell recorded at `offset`, or else at the nearest earlier offset.
    fn at_or_before(&self, offset: usize) -> Option<(usize, usize)> {
        let index = self.0.partition_point(|(at, _)| *at <= offset);
        index.checked_sub(1).map(|index| self.0[index].1)
    }

    fn contains(&self, offset: usize) -> bool {
        self.0.binary_search_by_key(&offset, |(at, _)| *at).is_ok()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Affinity {
    Upstream,
    Downstream,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hit {
    pub offset: usize,
    pub task: Option<usize>,
    pub affinity: Affinity,
}

impl Projection {
    /// Lay out the whole source, disclosing around `selection`.
    #[cfg(test)]
    pub fn build(
        source: &str,
        parsed: &Parsed,
        selection: Selection,
        width: usize,
        live: bool,
    ) -> Self {
        let disclosed = if live {
            parsed.disclosed(source, selection)
        } else {
            Vec::new()
        };
        Self::layout(source, parsed, &disclosed, width, live)
    }

    /// Lay out the whole source with precomputed disclosure, as returned by
    /// `Parsed::disclosed`.
    #[cfg(test)]
    pub fn layout(
        source: &str,
        parsed: &Parsed,
        disclosed: &[Range<usize>],
        width: usize,
        live: bool,
    ) -> Self {
        let code = parsed.code_styles(source, 0..source.len());
        layout::build(source, parsed, &code, disclosed, width, live, Window::WHOLE)
    }

    /// Lay out only `window`, which must start at a restart point. `code`
    /// holds highlighted code for the window, sorted and nonoverlapping,
    /// as from `Parsed::code_styles` or a `Highlighter`.
    pub fn window(
        source: &str,
        parsed: &Parsed,
        code: &[(Range<usize>, Style)],
        disclosed: &[Range<usize>],
        width: usize,
        live: bool,
        window: Window,
    ) -> Self {
        layout::build(source, parsed, code, disclosed, width, live, window)
    }

    /// The source offset of the first row.
    pub fn start(&self) -> usize {
        self.start
    }

    /// Where layout stopped: the start of the first line not laid out.
    pub fn end(&self) -> usize {
        self.end
    }

    /// Whether the rows reach the end of the source.
    pub fn complete(&self) -> bool {
        self.complete
    }

    /// The first row's number within its source line: zero unless the
    /// window resumed inside a long line.
    pub fn first_row(&self) -> usize {
        self.first_row
    }

    /// The caret cell for `offset`, when this window lays it out.
    pub fn locate(&self, offset: usize) -> Option<(usize, usize)> {
        (offset >= self.start && (offset < self.end || (self.complete && offset == self.end)))
            .then(|| self.cursor(offset))
    }

    /// The row where the line starting at `offset` begins, or else the first
    /// row starting after it. A table's closing border shares its start
    /// with the line that follows it, so restart rows win.
    pub fn first_row_at(&self, offset: usize) -> usize {
        let first = self.rows.partition_point(|row| row.start < offset);
        self.rows[first..]
            .iter()
            .take_while(|row| row.start == offset)
            .position(|row| row.restart)
            .map_or(first, |at| first + at)
            .min(self.rows.len() - 1)
    }

    /// The nearest row at or above `row` where layout could restart.
    pub fn restart_row(&self, row: usize) -> usize {
        (0..=row.min(self.rows.len() - 1))
            .rev()
            .find(|row| self.rows[*row].restart)
            .unwrap_or(0)
    }

    /// A visual row of this projection. Row numbers are local to it.
    pub fn row(&self, index: usize) -> &VisualRow {
        &self.rows[index]
    }

    pub fn row_count(&self) -> usize {
        self.rows.len()
    }

    /// Consecutive rows, clamped to those laid out.
    pub fn rows(&self, range: Range<usize>) -> &[VisualRow] {
        let end = range.end.min(self.rows.len());
        &self.rows[range.start.min(end)..end]
    }

    pub fn cursor(&self, offset: usize) -> (usize, usize) {
        self.positions.at_or_before(offset).unwrap_or((0, 0))
    }

    pub fn hit(&self, row: usize, column: usize) -> Hit {
        let row = &self.rows[row.min(self.rows.len() - 1)];
        for glyph in &row.glyphs {
            if column < glyph.column + glyph.width {
                let after = column.saturating_sub(glyph.column) * 2 >= glyph.width;
                return Hit {
                    offset: if after {
                        glyph.source.end
                    } else {
                        glyph.source.start
                    },
                    task: glyph.task,
                    affinity: if after {
                        Affinity::Upstream
                    } else {
                        Affinity::Downstream
                    },
                };
            }
        }
        Hit {
            offset: row.end,
            task: None,
            affinity: Affinity::Upstream,
        }
    }

    /// A wrap boundary belongs to the next row unless the caret arrived from
    /// its left; then it stays at the end of the previous row.
    pub fn cursor_with_affinity(&self, offset: usize, affinity: Affinity) -> (usize, usize) {
        let (row, col) = self.cursor(offset);
        if affinity == Affinity::Upstream
            && row > 0
            && self.rows[row].start == offset
            && self.rows[row - 1].end == offset
            && self.rows[row].line.is_none()
        {
            let previous = &self.rows[row - 1];
            return (
                row - 1,
                previous.glyphs.last().map_or(0, |g| g.column + g.width),
            );
        }
        (row, col)
    }

    /// The nearest row in `direction` that vertical movement may enter.
    pub fn navigable_row(&self, from: usize, delta: isize) -> usize {
        let last = self.rows.len() - 1;
        let target = from.saturating_add_signed(delta).min(last);
        let step = if delta < 0 { -1 } else { 1 };
        let walk = |step: isize| {
            std::iter::successors(Some(target), move |row: &usize| {
                row.checked_add_signed(step).filter(|row| *row <= last)
            })
        };
        // Continue past decoration in the direction of travel, then fall back.
        walk(step)
            .chain(walk(-step))
            .find(|row| self.rows[*row].navigable)
            .unwrap_or(from)
    }

    /// Pointer-only padding; insertion and keyboard hit testing stay exact.
    pub fn pointer_task<'a>(
        &self,
        parsed: &'a Parsed,
        row: usize,
        column: usize,
        viewport_width: usize,
    ) -> Option<&'a Task> {
        if self.revision != parsed.snapshot.revision || column >= viewport_width {
            return None;
        }
        let row = self.rows.get(row)?;
        let occupied = row
            .glyphs
            .iter()
            .find(|glyph| glyph.column <= column && column < glyph.column + glyph.width);
        if let Some(glyph) = occupied {
            if let Some(index) = glyph.task {
                return parsed.snapshot.tasks.get(index);
            }
            // Padding can claim whitespace, never label text or another glyph.
            if !glyph.text.chars().all(char::is_whitespace) {
                return None;
            }
        }
        let mut neighbors = row.glyphs.iter().filter(|glyph| {
            glyph.task.is_some()
                && glyph.column < viewport_width
                && ((glyph.column > 0 && column == glyph.column - 1)
                    || column == glyph.column + glyph.width)
        });
        let target = neighbors.next()?;
        // Keep an ambiguous shared padding cell available for text placement.
        if neighbors.next().is_some() {
            return None;
        }
        parsed.snapshot.tasks.get(target.task?)
    }
}

/// Characters a terminal would act on rather than draw: C0 and C1 controls,
/// DEL, and bidirectional overrides and isolates.
pub(crate) fn unsafe_char(c: char) -> bool {
    matches!(
        c,
        '\u{00}'..='\u{1f}' | '\u{7f}'..='\u{9f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'
    )
}

pub fn safe_text(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            '\u{00}'..='\u{1f}' => char::from_u32(0x2400 + c as u32).unwrap(),
            '\u{7f}' => '␡',
            '\u{80}'..='\u{9f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' => '�',
            _ => c,
        })
        .collect()
}

fn overlaps(a: &Range<usize>, b: &Range<usize>) -> bool {
    a.start < b.end && b.start < a.end
}
