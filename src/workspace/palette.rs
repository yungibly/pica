//! A small command picker. Query edits always stay in its own document.
use super::clipped;
use crate::ui;
use crossterm::event::{Event, KeyCode, KeyModifiers, MouseButton, MouseEventKind};
use pica::Document;
use ratatui::{
    Frame,
    layout::Rect,
    widgets::{Clear, Paragraph},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Command {
    New,
    Open,
    Save,
    SaveAs,
    Close,
    Find,
    FindNext,
    FindPrevious,
    Replace,
    View,
    Ruler,
    Wrap,
    Sidebar,
    NextTab,
    PreviousTab,
    GoToLine,
    Documents,
    Headings,
    Help,
    Quit,
    Undo,
    Redo,
    CutLine,
    Duplicate,
    Comment,
    Bold,
    Italic,
    InlineCode,
    Task,
    Indent,
    Outdent,
    MoveUp,
    MoveDown,
    DuplicateUp,
    DuplicateDown,
    Theme,
    Icons,
    Guides,
    Indentation,
    TabWidth,
    Reload,
    Recover,
}
impl Command {
    pub const ALL: [Self; 42] = [
        Self::New,
        Self::Open,
        Self::Save,
        Self::SaveAs,
        Self::Close,
        Self::Find,
        Self::FindNext,
        Self::FindPrevious,
        Self::Replace,
        Self::View,
        Self::Ruler,
        Self::Wrap,
        Self::Sidebar,
        Self::NextTab,
        Self::PreviousTab,
        Self::GoToLine,
        Self::Documents,
        Self::Headings,
        Self::Help,
        Self::Quit,
        Self::Undo,
        Self::Redo,
        Self::CutLine,
        Self::Duplicate,
        Self::Comment,
        Self::Bold,
        Self::Italic,
        Self::InlineCode,
        Self::Task,
        Self::Indent,
        Self::Outdent,
        Self::MoveUp,
        Self::MoveDown,
        Self::DuplicateUp,
        Self::DuplicateDown,
        Self::Theme,
        Self::Icons,
        Self::Guides,
        Self::Indentation,
        Self::TabWidth,
        Self::Reload,
        Self::Recover,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::New => "New tab",
            Self::Open => "Open file…",
            Self::Save => "Save document",
            Self::SaveAs => "Save as…",
            Self::Close => "Close tab",
            Self::Find => "Find in document",
            Self::FindNext => "Find next match",
            Self::FindPrevious => "Find previous match",
            Self::Replace => "Find and replace",
            Self::View => "Toggle live / source view",
            Self::Ruler => "Toggle line numbers",
            Self::Wrap => "Toggle word wrap",
            Self::Sidebar => "Toggle outline sidebar",
            Self::NextTab => "Next tab",
            Self::PreviousTab => "Previous tab",
            Self::GoToLine => "Go to line…",
            Self::Documents => "Switch document…",
            Self::Headings => "Go to heading…",
            Self::Help => "Help / keyboard shortcuts",
            Self::Quit => "Quit pica (all tabs)",
            Self::Undo => "Undo edit",
            Self::Redo => "Redo edit",
            Self::CutLine => "Cut line",
            Self::Duplicate => "Duplicate line or selection",
            Self::Comment => "Toggle comment",
            Self::Bold => "Format bold",
            Self::Italic => "Format italic",
            Self::InlineCode => "Format inline code",
            Self::Task => "Toggle task",
            Self::Indent => "Indent selected lines",
            Self::Outdent => "Outdent selected lines",
            Self::MoveUp => "Move lines up",
            Self::MoveDown => "Move lines down",
            Self::DuplicateUp => "Duplicate lines above",
            Self::DuplicateDown => "Duplicate lines below",
            Self::Theme => "Choose theme…",
            Self::Icons => "Toggle Nerd Font icons",
            Self::Guides => "Toggle indent guides",
            Self::Indentation => "Indentation: tabs or spaces…",
            Self::TabWidth => "Tab width…",
            Self::Reload => "Reload file from disk",
            Self::Recover => "Recover documents…",
        }
    }
    /// The name `keys.conf` binds a key to. Names are part of that file's
    /// format: add new ones freely, but never rename one.
    pub fn name(self) -> &'static str {
        match self {
            Self::New => "new-tab",
            Self::Open => "open",
            Self::Save => "save",
            Self::SaveAs => "save-as",
            Self::Close => "close-tab",
            Self::Find => "find",
            Self::FindNext => "find-next",
            Self::FindPrevious => "find-previous",
            Self::Replace => "replace",
            Self::View => "toggle-view",
            Self::Ruler => "toggle-line-numbers",
            Self::Wrap => "toggle-word-wrap",
            Self::Sidebar => "toggle-outline",
            Self::NextTab => "next-tab",
            Self::PreviousTab => "previous-tab",
            Self::GoToLine => "go-to-line",
            Self::Documents => "switch-document",
            Self::Headings => "go-to-heading",
            Self::Help => "help",
            Self::Quit => "quit",
            Self::Undo => "undo",
            Self::Redo => "redo",
            Self::CutLine => "cut-line",
            Self::Duplicate => "duplicate",
            Self::Comment => "toggle-comment",
            Self::Bold => "bold",
            Self::Italic => "italic",
            Self::InlineCode => "inline-code",
            Self::Task => "toggle-task",
            Self::Indent => "indent",
            Self::Outdent => "outdent",
            Self::MoveUp => "move-lines-up",
            Self::MoveDown => "move-lines-down",
            Self::DuplicateUp => "duplicate-lines-up",
            Self::DuplicateDown => "duplicate-lines-down",
            Self::Theme => "choose-theme",
            Self::Icons => "toggle-icons",
            Self::Guides => "toggle-indent-guides",
            Self::Indentation => "indentation",
            Self::TabWidth => "tab-width",
            Self::Reload => "reload",
            Self::Recover => "recover-documents",
        }
    }
    /// The primary key, if any. Function keys are silent aliases and never
    /// shown; Ctrl+R names whichever toggle it performs in this document.
    pub fn shortcut(self, markdown: bool) -> &'static str {
        match self {
            Self::New => "Ctrl+T",
            Self::Open => "Ctrl+O",
            Self::Save => "Ctrl+S",
            Self::Close => "Ctrl+Q",
            Self::Find => "Ctrl+F",
            Self::FindNext => "Ctrl+N",
            Self::FindPrevious => "Ctrl+P",
            Self::View if markdown => "Ctrl+R",
            Self::Ruler if !markdown => "Ctrl+R",
            Self::Wrap => "Alt+Z",
            Self::NextTab => "Alt+.",
            Self::PreviousTab => "Alt+,",
            Self::GoToLine => "Ctrl+L",
            Self::Help => "Ctrl+G",
            Self::Undo => "Ctrl+Z",
            Self::Redo => "Ctrl+Y",
            Self::CutLine => "Ctrl+K",
            Self::Duplicate => "Ctrl+D",
            Self::Comment => "Ctrl+/",
            Self::Bold => "Ctrl+B",
            Self::Italic => "Alt+I",
            Self::InlineCode => "Alt+`",
            Self::Task => "Alt+T",
            Self::Indent => "Ctrl+]",
            Self::Outdent => "Ctrl+[",
            Self::MoveUp => "Alt+Up",
            Self::MoveDown => "Alt+Down",
            Self::DuplicateUp => "Alt+Shift+Up",
            Self::DuplicateDown => "Alt+Shift+Down",
            _ => "",
        }
    }
}

/// One row of the palette: a command, or a line typed as a bare number.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Entry {
    Line(usize, usize),
    Command(Command),
}

pub(super) enum Action {
    None,
    Close,
    Command(Command),
    Line(usize, usize),
}

/// `N` or `N:C`, as micro's `goto` accepts: a 1-based line and column.
pub(super) fn parse_line(text: &str) -> Option<(usize, usize)> {
    let (line, column) = match text.trim().split_once(':') {
        Some((line, column)) => (line, Some(column)),
        None => (text.trim(), None),
    };
    let digits = |text: &str| !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit());
    if !digits(line) || column.is_some_and(|column| !digits(column)) {
        return None;
    }
    let line = line.parse::<usize>().ok().filter(|line| *line > 0)?;
    let column = match column {
        Some(column) => column.parse::<usize>().ok().filter(|column| *column > 0)?,
        None => 1,
    };
    Some((line, column))
}

impl Entry {
    fn action(self) -> Action {
        match self {
            Self::Line(line, column) => Action::Line(line, column),
            Self::Command(command) => Action::Command(command),
        }
    }
}

pub(super) struct Palette {
    pub query: Document,
    pub selected: usize,
    pub line_mode: bool,
    pub hits: Vec<(Rect, Entry)>,
    ready: bool,
    error: Option<&'static str>,
    /// The key shown for each command, in `Command::ALL` order: its default
    /// unless the user's bindings say otherwise.
    shortcuts: Vec<String>,
}
impl Palette {
    /// `markdown` picks the keys that follow the active document's kind.
    pub fn new(line_mode: bool, markdown: bool, keys: &super::bindings::Bindings) -> Self {
        Self {
            query: Document::new(""),
            selected: 0,
            line_mode,
            hits: vec![],
            ready: false,
            error: None,
            shortcuts: Command::ALL
                .into_iter()
                .map(|command| keys.shortcut(command, command.shortcut(markdown)))
                .collect(),
        }
    }
    /// The key shown beside `command`.
    pub fn shortcut(&self, command: Command) -> &str {
        Command::ALL
            .iter()
            .position(|other| *other == command)
            .map_or("", |index| self.shortcuts[index].as_str())
    }
    pub fn matches(&self) -> Vec<Command> {
        let query = self.query.text().to_lowercase();
        Command::ALL
            .into_iter()
            .filter(|command| {
                let label =
                    format!("{} {}", command.label(), self.shortcut(*command)).to_lowercase();
                query.split_whitespace().all(|part| label.contains(part))
            })
            .collect()
    }
    /// Commands, led by a jump when the query is a bare line number.
    pub fn entries(&self) -> Vec<Entry> {
        parse_line(self.query.text())
            .map(|(line, column)| Entry::Line(line, column))
            .into_iter()
            .chain(self.matches().into_iter().map(Entry::Command))
            .collect()
    }
    pub fn invalidate(&mut self) {
        self.hits.clear();
        self.ready = false;
    }
    pub fn handle(&mut self, event: Event) -> Action {
        let revision = self.query.revision();
        match event {
            Event::Resize(..) => self.invalidate(),
            Event::Paste(text) => {
                let clean: String = text.chars().filter(|c| !c.is_control()).collect();
                if !clean.is_empty() {
                    self.query.insert(&clean);
                }
            }
            Event::Key(key) => {
                let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
                match key.code {
                    // Ctrl+Q aborts the prompt, as at micro's command line.
                    KeyCode::Esc => return Action::Close,
                    KeyCode::Char('q') if ctrl => return Action::Close,
                    KeyCode::Char('a') if ctrl => self.query.select_all(),
                    KeyCode::Char('u') if ctrl => {
                        self.query.select_all();
                        self.query.insert("");
                    }
                    KeyCode::Up if !self.line_mode => {
                        self.selected = self.selected.saturating_sub(1);
                        self.hits.clear();
                    }
                    KeyCode::Down if !self.line_mode => {
                        self.selected =
                            (self.selected + 1).min(self.entries().len().saturating_sub(1));
                        self.hits.clear();
                    }
                    KeyCode::Enter if self.ready => {
                        if self.line_mode {
                            match parse_line(self.query.text()) {
                                Some((line, column)) => return Action::Line(line, column),
                                None => self.error = Some("Enter a line, or line:column"),
                            }
                        } else if let Some(entry) = self.entries().get(self.selected) {
                            return entry.action();
                        }
                    }
                    KeyCode::Left => self
                        .query
                        .move_left(key.modifiers.contains(KeyModifiers::SHIFT)),
                    KeyCode::Right => self
                        .query
                        .move_right(key.modifiers.contains(KeyModifiers::SHIFT)),
                    KeyCode::Home => {
                        let _ = self.query.set_caret(0);
                    }
                    KeyCode::End => {
                        let _ = self.query.set_caret(self.query.text().len());
                    }
                    KeyCode::Backspace => {
                        self.query.backspace();
                    }
                    KeyCode::Delete => {
                        self.query.delete_forward();
                    }
                    KeyCode::Char(c)
                        if !ctrl
                            && !key.modifiers.contains(KeyModifiers::ALT)
                            && !c.is_control() =>
                    {
                        self.query.insert(&c.to_string());
                    }
                    _ => {}
                }
            }
            Event::Mouse(mouse) if mouse.kind == MouseEventKind::Down(MouseButton::Left) => {
                if let Some((_, entry)) = self
                    .hits
                    .iter()
                    .find(|(rect, _)| rect.contains((mouse.column, mouse.row).into()))
                {
                    return entry.action();
                }
            }
            _ => {}
        }
        if revision != self.query.revision() {
            self.selected = 0;
            self.error = None;
            self.invalidate();
        }
        Action::None
    }
    pub fn draw(&mut self, frame: &mut Frame) {
        let area = frame.area();
        self.invalidate();
        if area.width == 0 || area.height == 0 {
            return;
        }
        if area.width < 12 || area.height < 5 {
            frame.render_widget(Clear, area);
            frame.render_widget(
                Paragraph::new("Resize for commands\nEsc: close").style(ui::surface()),
                area,
            );
            return;
        }
        let width = area.width.saturating_sub(4).clamp(12, 76).min(area.width);
        let max_height = area.height.saturating_sub(2).clamp(5, 19).min(area.height);
        let entries = self.entries();
        let height = if self.line_mode {
            6.min(max_height)
        } else {
            (entries.len() as u16 + 4).clamp(5, max_height)
        };
        let rect = Rect::new(
            area.x + (area.width - width) / 2,
            // Keep the query anchored while the list grows or shrinks below it.
            area.y + (area.height - max_height) / 3,
            width,
            height,
        );
        let count = format!("{}/{}", self.matches().len(), Command::ALL.len());
        let inner = ui::panel(
            frame,
            rect,
            if self.line_mode {
                "Go to line"
            } else {
                "Commands"
            },
            (!self.line_mode).then_some(count.as_str()),
        );
        let field = Rect::new(inner.x + 1, inner.y, inner.width.saturating_sub(2), 1);
        let caret = ui::prompt(
            frame,
            field,
            &self.query,
            if self.line_mode {
                "Line or line:column…"
            } else {
                "Type to filter commands…"
            },
        );
        if field.width > 0 {
            frame.set_cursor_position(caret);
        }
        ui::divider(frame, rect, rect.y + 2);
        let available = usize::from(height.saturating_sub(4));
        self.ready = available > 0;
        if self.line_mode {
            frame.render_widget(
                Paragraph::new(clipped(
                    self.error.unwrap_or("Enter jumps to the line"),
                    field.width as usize,
                ))
                .style(if self.error.is_some() {
                    ui::surface().fg(crate::theme::palette().warning)
                } else {
                    ui::muted()
                }),
                Rect::new(field.x, rect.y + 3, field.width, 1),
            );
        } else {
            self.selected = self.selected.min(entries.len().saturating_sub(1));
            let start = self.selected.saturating_sub(available.saturating_sub(1));
            let query = self.query.text().to_owned();
            for (row, entry) in entries.iter().skip(start).take(available).enumerate() {
                let row_rect = Rect::new(inner.x, rect.y + 3 + row as u16, inner.width, 1);
                let (label, shortcut) = match entry {
                    Entry::Line(line, 1) => (format!("Go to line {line}"), "Ctrl+L"),
                    Entry::Line(line, column) => {
                        (format!("Go to line {line}, column {column}"), "Ctrl+L")
                    }
                    Entry::Command(command) => {
                        (command.label().to_owned(), self.shortcut(*command))
                    }
                };
                ui::item(
                    frame,
                    row_rect,
                    &label,
                    shortcut,
                    start + row == self.selected,
                    &query,
                );
                self.hits.push((row_rect, *entry));
            }
            if entries.is_empty() {
                frame.render_widget(
                    Paragraph::new("  No matching commands").style(ui::muted()),
                    Rect::new(inner.x, rect.y + 3, inner.width, 1),
                );
            }
        }
        ui::hints(
            frame,
            rect,
            if self.line_mode {
                "⏎ jump · esc close"
            } else {
                "↑↓ select · ⏎ run · esc close"
            },
        );
    }
}
