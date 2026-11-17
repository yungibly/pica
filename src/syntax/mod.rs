//! Small, dependency-free syntax highlighting for code files and fenced code.
//! Every lexer is a line-oriented state machine: spans never leave their line,
//! while block comments and multi-line strings carry over to the next line.

/// `rebind!(Lexer { fields })` copies a lexer's state onto another source.
/// State refers to source text only by offset, so a state saved at a line
/// start stays valid while the text before that line is unchanged.
macro_rules! rebind {
    ($name:ident { $($field:ident),* $(,)? } $(, nested $inner:ident)?) => {
        impl $name<'_> {
            pub(super) fn rebind<'b>(&self, src: &'b str) -> $name<'b> {
                $name {
                    src,
                    $($inner: self.$inner.rebind(src),)?
                    $($field: self.$field.clone(),)*
                }
            }
        }
    };
}

mod languages;
mod lexer;
mod markup;
mod shell;
mod special;
#[cfg(test)]
mod tests;

use std::{fmt, ops::Range, path::Path};

/// Highlight classes shared by every language.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Token {
    Comment,
    Keyword,
    Type,
    Function,
    String,
    /// Escape sequences inside strings and HTML entities.
    Escape,
    Number,
    /// Literals such as `true` and `None`, ALL_CAPS names, Ruby/Elixir symbols.
    Constant,
    Operator,
    Punctuation,
    /// Rust `name!`, C preprocessor directives, Zig and Julia `@builtin`.
    Macro,
    /// Rust `#[...]`, decorators and annotations, Elixir `@attr`.
    Attribute,
    /// Keys: JSON/YAML/TOML/INI keys, CSS properties, HTML attributes.
    Property,
    /// HTML/XML tag names and CSS element selectors.
    Tag,
    /// Sigil variables: `$x`, `${x}`, `@x`, CSS `--x`, SCSS `$x`.
    Variable,
    /// Rust lifetimes and loop labels, YAML anchors, heredoc delimiters.
    Label,
    /// Diff file headers, INI/TOML sections, Markdown headings.
    Heading,
    Inserted,
    Deleted,
}

/// A registered language; every instance is static.
pub struct Language {
    name: &'static str,
    icon: &'static str,
    /// Lowercase, without the dot; matched case-insensitively.
    extensions: &'static [&'static str],
    /// Exact file names.
    file_names: &'static [&'static str],
    /// Lowercase fence info words.
    aliases: &'static [&'static str],
    lexer: Lexer,
}

#[derive(Clone, Copy)]
enum Lexer {
    Code(&'static lexer::Grammar),
    Json,
    Yaml,
    Toml,
    Ini,
    Html,
    Xml,
    Css,
    Scss,
    Diff,
    Make,
    Docker,
    Shell,
    Markdown,
}

impl Language {
    /// Human display name, e.g. "Rust", "C++", "JavaScript", "Shell", "JSON".
    pub fn name(&self) -> &'static str {
        self.name
    }

    /// One Nerd Font glyph of terminal width 1 (nvim-web-devicons conventions).
    pub fn icon(&self) -> &'static str {
        self.icon
    }

    /// Detect by exact file name, then by case-insensitive extension. Plain
    /// text and unknown files have no language.
    pub fn for_path(path: &Path) -> Option<&'static Language> {
        let name = path.file_name()?.to_str()?;
        if let Some(language) = Self::all().iter().find(|l| l.file_names.contains(&name)) {
            return Some(language);
        }
        if name.starts_with("Dockerfile.") || name.starts_with("Containerfile.") {
            return Self::all()
                .iter()
                .find(|l| matches!(l.lexer, Lexer::Docker));
        }
        Self::by_extension(path.extension()?.to_str()?)
    }

    /// Detect from a Markdown fence info string such as `rust,ignore`,
    /// `{.python}` or `sh title="x"`. Aliases win over extensions.
    pub fn for_fence(info: &str) -> Option<&'static Language> {
        let info = info.trim_start();
        let info = info.strip_prefix('{').unwrap_or(info);
        let info = info.strip_prefix('.').unwrap_or(info);
        let end = info
            .find(|c: char| c.is_whitespace() || matches!(c, ',' | '{' | '}'))
            .unwrap_or(info.len());
        let word = &info[..end];
        if word.is_empty() {
            return None;
        }
        Self::all()
            .iter()
            .find(|l| l.aliases.iter().any(|a| a.eq_ignore_ascii_case(word)))
            .or_else(|| Self::by_extension(word))
    }

    /// All registered languages, for tests and diagnostics.
    pub fn all() -> &'static [Language] {
        &languages::LANGUAGES
    }

    fn by_extension(extension: &str) -> Option<&'static Language> {
        Self::all().iter().find(|l| {
            l.extensions
                .iter()
                .any(|e| e.eq_ignore_ascii_case(extension))
        })
    }
}

impl fmt::Debug for Language {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Language").field(&self.name).finish()
    }
}

/// Names are unique, so they identify a language.
impl PartialEq for Language {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
    }
}

impl Eq for Language {}

/// Highlight `segments` of `source` as one continuous stream: lexer state (block comments,
/// multi-line strings, etc.) carries across segment boundaries, and segment boundaries act as
/// line breaks. Returned spans are in `source` byte coordinates, sorted by start, non-overlapping,
/// non-empty, on UTF-8 char boundaries, fully inside one segment each (split a multi-line token at
/// segment boundaries). Unclassified text (plain identifiers, whitespace) gets no span.
/// Segments are sorted and non-overlapping; they may or may not include their trailing newline.
pub fn highlight(
    language: &Language,
    source: &str,
    segments: &[Range<usize>],
) -> Vec<(Range<usize>, Token)> {
    run(language, source, segments).spans
}

fn run<'a>(language: &Language, source: &'a str, segments: &[Range<usize>]) -> Out<'a> {
    let mut out = Out {
        src: source,
        spans: Vec::new(),
        segment: 0,
        rejected: 0,
    };
    feed(
        &mut Machine::new(language, source, segments),
        segments,
        &mut out,
    );
    out
}

/// Any language's lexer, so its state can be saved and resumed.
enum Machine<'a> {
    Code(lexer::Code<'a>),
    Json(special::Json<'a>),
    Yaml(special::Yaml<'a>),
    Toml(special::Toml<'a>),
    Ini(special::Ini<'a>),
    Diff(special::Diff<'a>),
    Markdown(special::Markdown<'a>),
    Markup(markup::Markup<'a>),
    Css(markup::Css<'a>),
    Shell(shell::Shell<'a>),
    Make(shell::Make<'a>),
    Docker(shell::Docker<'a>),
}

impl<'a> Machine<'a> {
    fn new(language: &Language, source: &'a str, segments: &[Range<usize>]) -> Self {
        match language.lexer {
            Lexer::Code(grammar) => Self::Code(lexer::Code::new(grammar, source, segments)),
            Lexer::Json => Self::Json(special::Json::new(source)),
            Lexer::Yaml => Self::Yaml(special::Yaml::new(source)),
            Lexer::Toml => Self::Toml(special::Toml::new(source)),
            Lexer::Ini => Self::Ini(special::Ini::new(source)),
            Lexer::Diff => Self::Diff(special::Diff::new(source)),
            Lexer::Markdown => Self::Markdown(special::Markdown::new(source)),
            Lexer::Html => Self::Markup(markup::Markup::new(source, true)),
            Lexer::Xml => Self::Markup(markup::Markup::new(source, false)),
            Lexer::Css => Self::Css(markup::Css::new(source, false)),
            Lexer::Scss => Self::Css(markup::Css::new(source, true)),
            Lexer::Shell => Self::Shell(shell::Shell::new(source)),
            Lexer::Make => Self::Make(shell::Make::new(source)),
            Lexer::Docker => Self::Docker(shell::Docker::new(source)),
        }
    }

    fn rebind<'b>(&self, src: &'b str) -> Machine<'b> {
        match self {
            Self::Code(lexer) => Machine::Code(lexer.rebind(src)),
            Self::Json(lexer) => Machine::Json(lexer.rebind(src)),
            Self::Yaml(lexer) => Machine::Yaml(lexer.rebind(src)),
            Self::Toml(lexer) => Machine::Toml(lexer.rebind(src)),
            Self::Ini(lexer) => Machine::Ini(lexer.rebind(src)),
            Self::Diff(lexer) => Machine::Diff(lexer.rebind(src)),
            Self::Markdown(lexer) => Machine::Markdown(lexer.rebind(src)),
            Self::Markup(lexer) => Machine::Markup(lexer.rebind(src)),
            Self::Css(lexer) => Machine::Css(lexer.rebind(src)),
            Self::Shell(lexer) => Machine::Shell(lexer.rebind(src)),
            Self::Make(lexer) => Machine::Make(lexer.rebind(src)),
            Self::Docker(lexer) => Machine::Docker(lexer.rebind(src)),
        }
    }
}

impl Lex for Machine<'_> {
    fn line(&mut self, start: usize, end: usize, out: &mut Out) {
        match self {
            Self::Code(lexer) => lexer.line(start, end, out),
            Self::Json(lexer) => lexer.line(start, end, out),
            Self::Yaml(lexer) => lexer.line(start, end, out),
            Self::Toml(lexer) => lexer.line(start, end, out),
            Self::Ini(lexer) => lexer.line(start, end, out),
            Self::Diff(lexer) => lexer.line(start, end, out),
            Self::Markdown(lexer) => lexer.line(start, end, out),
            Self::Markup(lexer) => lexer.line(start, end, out),
            Self::Css(lexer) => lexer.line(start, end, out),
            Self::Shell(lexer) => lexer.line(start, end, out),
            Self::Make(lexer) => lexer.line(start, end, out),
            Self::Docker(lexer) => lexer.line(start, end, out),
        }
    }
}

/// Source bytes between saved lexer states.
const CHECKPOINT_SPACING: usize = 4096;

/// Highlights a whole file a window at a time. Lexer state is saved at line
/// starts as lexing passes them, so a window is lexed from the nearest saved
/// state before it rather than from the top of the file. Saved states before
/// an edit stay valid; later ones are dropped.
pub struct Highlighter {
    language: &'static Language,
    spacing: usize,
    revision: Option<u64>,
    /// Lexer state at the start of a line, by offset, ascending.
    checkpoints: Vec<(usize, Machine<'static>)>,
}

impl Highlighter {
    pub fn new(language: &'static Language) -> Self {
        Self {
            language,
            spacing: CHECKPOINT_SPACING,
            revision: None,
            checkpoints: Vec::new(),
        }
    }

    pub fn language(&self) -> &'static Language {
        self.language
    }

    /// The revision of the last window, which saved states belong to.
    pub fn revision(&self) -> Option<u64> {
        self.revision
    }

    /// Spans on lines that begin in `start..end`, identical to highlighting
    /// all of `source` after `skip` bytes (its BOM). `unchanged` is how much
    /// of the source is the same as at the previous call's revision.
    pub fn window(
        &mut self,
        source: &str,
        skip: usize,
        revision: u64,
        unchanged: usize,
        start: usize,
        end: usize,
    ) -> Vec<(Range<usize>, Token)> {
        if self.revision != Some(revision) {
            // A state at `at` depends on the text before it, and on the byte
            // at `at` for whether a CR ended its line.
            let keep = self.checkpoints.partition_point(|(at, _)| *at < unchanged);
            self.checkpoints.truncate(keep);
            self.revision = Some(revision);
        }
        let start = start.max(skip);
        let saved = self.checkpoints.partition_point(|(at, _)| *at <= start);
        let (mut line, mut machine) = match saved.checked_sub(1) {
            Some(index) => {
                let (at, machine) = &self.checkpoints[index];
                (*at, machine.rebind(source))
            }
            None => (
                skip,
                Machine::new(
                    self.language,
                    source,
                    std::slice::from_ref(&(skip..source.len())),
                ),
            ),
        };
        let mut out = Out {
            src: source,
            spans: Vec::new(),
            segment: line,
            rejected: 0,
        };
        let mut previous = line;
        let mut next = saved;
        let bytes = source.as_bytes();
        while line < source.len() && line < end.max(start + 1) {
            let stop = bytes[line..]
                .iter()
                .position(|&b| b == b'\n' || b == b'\r')
                .map_or(source.len(), |at| line + at);
            machine.line(line, stop, &mut out);
            line = stop + usize::from(stop < source.len());
            if bytes.get(stop) == Some(&b'\r') && bytes.get(line) == Some(&b'\n') {
                line += 1;
            }
            if line >= source.len() {
                break;
            }
            while self
                .checkpoints
                .get(next)
                .is_some_and(|(at, _)| *at <= line)
            {
                previous = self.checkpoints[next].0;
                next += 1;
            }
            if line - previous >= self.spacing {
                self.checkpoints.insert(next, (line, machine.rebind("")));
                previous = line;
                next += 1;
            }
        }
        let first = out.spans.partition_point(|(range, _)| range.end <= start);
        out.spans.drain(..first);
        out.spans
    }
}

/// A lexer consumes one line at a time, carrying state to the next line.
trait Lex {
    /// Highlight `start..end`: one line without its terminator.
    fn line(&mut self, start: usize, end: usize, out: &mut Out);
}

/// Split segments into lines at `\n`, `\r\n` and lone `\r`. A segment's end is a
/// line break too, but a terminator at its very end does not add an empty line.
fn feed(lexer: &mut impl Lex, segments: &[Range<usize>], out: &mut Out) {
    let src = out.src;
    let bytes = src.as_bytes();
    let mut floor = 0;
    for segment in segments {
        let mut start = segment.start.max(floor);
        let mut end = segment.end.min(src.len());
        if start > end {
            continue;
        }
        while !src.is_char_boundary(start) {
            start += 1;
        }
        while !src.is_char_boundary(end) {
            end -= 1;
        }
        if start > end {
            continue;
        }
        floor = end;
        out.segment = start;
        let mut line = start;
        loop {
            let Some(offset) = bytes[line..end]
                .iter()
                .position(|&b| b == b'\n' || b == b'\r')
            else {
                lexer.line(line, end, out);
                break;
            };
            let stop = line + offset;
            lexer.line(line, stop, out);
            line = stop + 1;
            if bytes[stop] == b'\r' && line < end && bytes[line] == b'\n' {
                line += 1;
            }
            if line >= end {
                break;
            }
        }
    }
}

/// Collects spans and enforces the public invariants even if a lexer errs.
struct Out<'a> {
    src: &'a str,
    spans: Vec<(Range<usize>, Token)>,
    /// Start of the current segment; spans never merge across it.
    segment: usize,
    /// Spans dropped for violating an invariant; tests require none.
    rejected: usize,
}

impl Out<'_> {
    fn push(&mut self, start: usize, end: usize, token: Token) {
        if start >= end {
            return;
        }
        let misplaced = self.spans.last().is_some_and(|(last, _)| last.end > start);
        if misplaced || !self.src.is_char_boundary(start) || !self.src.is_char_boundary(end) {
            self.rejected += 1;
            return;
        }
        if let Some((last, kind)) = self.spans.last_mut()
            && *kind == token
            && last.end == start
            && last.start >= self.segment
        {
            last.end = end;
            return;
        }
        self.spans.push((start..end, token));
    }
}

// Shared scanning helpers. Positions are byte offsets; `end` bounds the line.
// Delimiters are ASCII, so byte-wise searches never stop inside a character.

fn is_ws(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\x0b' | b'\x0c')
}

fn skip_ws(b: &[u8], mut i: usize, end: usize) -> usize {
    while i < end && is_ws(b[i]) {
        i += 1;
    }
    i
}

/// Byte length of the character starting at `i`.
fn char_len(b: &[u8], i: usize) -> usize {
    let len = match b[i] {
        0xf0.. => 4,
        0xe0.. => 3,
        0xc0.. => 2,
        _ => 1,
    };
    len.min(b.len() - i)
}

fn char_at(src: &str, i: usize) -> Option<char> {
    src.get(i..)?.chars().next()
}

fn ident_start(c: char) -> bool {
    c == '_' || c.is_alphabetic()
}

/// End of an identifier run from `i`; `extra` lists further ASCII characters.
fn ident_end(src: &str, mut i: usize, end: usize, extra: &[u8]) -> usize {
    let b = src.as_bytes();
    while i < end {
        let c = b[i];
        if c < 0x80 {
            if !(c.is_ascii_alphanumeric() || c == b'_' || extra.contains(&c)) {
                break;
            }
            i += 1;
        } else {
            match src.get(i..end).and_then(|rest| rest.chars().next()) {
                Some(c) if c.is_alphanumeric() => i += c.len_utf8(),
                _ => break,
            }
        }
    }
    i
}

fn starts(b: &[u8], i: usize, end: usize, pattern: &[u8]) -> bool {
    i <= end && end <= b.len() && b[i..end].starts_with(pattern)
}

/// First occurrence of a short `pattern` in `from..end`.
fn find(b: &[u8], from: usize, end: usize, pattern: &[u8]) -> Option<usize> {
    let first = *pattern.first()?;
    let mut i = from;
    while i < end {
        let k = i + b[i..end].iter().position(|&c| c == first)?;
        if starts(b, k, end, pattern) {
            return Some(k);
        }
        i = k + 1;
    }
    None
}

/// Binary search in a sorted table; `fold` compares ASCII case-insensitively
/// against a lowercase table without allocating.
fn lookup(table: &[&str], word: &str, fold: bool) -> bool {
    if !fold {
        return table.binary_search(&word).is_ok();
    }
    let mut buffer = [0; 32];
    let bytes = word.as_bytes();
    if bytes.len() > buffer.len() {
        return false;
    }
    for (slot, byte) in buffer.iter_mut().zip(bytes) {
        *slot = byte.to_ascii_lowercase();
    }
    let lower = &buffer[..bytes.len()];
    table
        .binary_search_by(|probe| probe.as_bytes().cmp(lower))
        .is_ok()
}

/// At least two characters, a letter, and no lowercase letters.
fn is_all_caps(word: &str) -> bool {
    word.len() >= 2
        && word.bytes().any(|b| b.is_ascii_uppercase())
        && word
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
}

/// End of a number starting at `i` (a digit, or `.` before a digit): radix
/// prefixes, `_` separators, a fraction, an exponent, and suffixes such as
/// `u32`, `f64`, `n` or `px`. `quotes` accepts C++ `1'000` separators.
fn number_end(b: &[u8], i: usize, end: usize, quotes: bool) -> usize {
    let digits = |mut j: usize, hex: bool| {
        while j < end {
            let c = b[j];
            let separator = quotes
                && c == b'\''
                && j > i
                && b[j - 1].is_ascii_hexdigit()
                && j + 1 < end
                && b[j + 1].is_ascii_hexdigit();
            if !(c.is_ascii_digit() || c == b'_' || (hex && c.is_ascii_hexdigit()) || separator) {
                break;
            }
            j += 1;
        }
        j
    };
    let radix = b[i] == b'0' && i + 1 < end && matches!(b[i + 1] | 0x20, b'x' | b'o' | b'b');
    let mut j = if radix {
        digits(i + 2, true)
    } else if b[i] == b'.' {
        digits(i + 1, false)
    } else {
        let whole = digits(i, false);
        if whole + 1 < end && b[whole] == b'.' && b[whole + 1].is_ascii_digit() {
            digits(whole + 1, false)
        } else {
            whole
        }
    };
    if !radix && j < end && b[j] | 0x20 == b'e' {
        let mut k = j + 1;
        if k < end && matches!(b[k], b'+' | b'-') {
            k += 1;
        }
        if k < end && b[k].is_ascii_digit() {
            j = digits(k, false);
        }
    }
    while j < end && (b[j].is_ascii_alphanumeric() || b[j] == b'_') {
        j += 1;
    }
    j
}

/// End of the escape sequence whose escape character is at `i`.
fn escape_end(b: &[u8], i: usize, end: usize) -> usize {
    let j = i + 1;
    if j >= end {
        return end;
    }
    let hex = |from: usize, max: usize| {
        let mut k = from;
        while k < end && k - from < max && b[k].is_ascii_hexdigit() {
            k += 1;
        }
        k
    };
    let braced = j + 1 < end && b[j + 1] == b'{';
    match b[j] {
        b'x' => hex(j + 1, 2),
        b'u' if braced => {
            let k = hex(j + 2, 8);
            if k < end && b[k] == b'}' { k + 1 } else { k }
        }
        b'u' => hex(j + 1, 4),
        b'U' => hex(j + 1, 8),
        b'N' if braced => find(b, j + 2, end.min(j + 64), b"}").map_or(j + 1, |k| k + 1),
        b'0'..=b'7' => {
            let mut k = j + 1;
            while k < end && k < j + 3 && (b'0'..=b'7').contains(&b[k]) {
                k += 1;
            }
            k
        }
        _ => (j + char_len(b, j)).min(end),
    }
}

/// Emit `start..stop` as a string whose `escape` sequences are Escape spans.
fn string_spans(out: &mut Out, b: &[u8], start: usize, stop: usize, escape: u8) {
    let mut run = start;
    let mut j = start;
    while j < stop {
        if escape != 0 && b[j] == escape {
            let e = escape_end(b, j, stop);
            out.push(run, j, Token::String);
            out.push(j, e, Token::Escape);
            j = e;
            run = e;
        } else {
            j += 1;
        }
    }
    out.push(run, stop, Token::String);
}
