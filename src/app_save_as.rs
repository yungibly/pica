//! The Save As dialog: a path field that starts from where the document is,
//! completes names with Tab, and asks before it replaces a file or creates
//! a folder. Every write goes through `FileState::save`.
use super::*;
use crate::{file_io::explain, paths};
use std::path::Path;

/// The largest existing file Save As will replace: its bytes are held while
/// the question is open, so the save can prove nothing changed meanwhile.
const MAX_REPLACED: usize = 8 * 1024 * 1024;

/// What Enter found in the way, awaiting Y or N.
enum Question {
    /// The target exists; `file` holds what was there when the user was
    /// asked. `changed` marks the document's own file, changed on disk.
    Replace { file: Box<FileState>, changed: bool },
    /// The target's folder does not exist.
    CreateFolder { folder: PathBuf, target: PathBuf },
}

/// What Tab last had to say.
#[derive(Default)]
enum Completions {
    #[default]
    Quiet,
    Several(Vec<String>),
    NoMatch,
}

pub(super) struct SaveAs {
    /// The path being typed.
    path: Document,
    question: Option<Question>,
    /// Whether the last frame showed the whole question. An unseen question
    /// is never answered.
    question_visible: Cell<bool>,
    completions: Completions,
    /// Why the last attempt did not save. It belongs to the dialog and goes
    /// when the path is edited or the dialog closes.
    problem: String,
}

impl Default for SaveAs {
    fn default() -> Self {
        Self {
            path: Document::new(""),
            question: None,
            question_visible: Cell::new(false),
            completions: Completions::Quiet,
            problem: String::new(),
        }
    }
}

impl SaveAs {
    /// The path changed: what was said about the old one no longer holds.
    fn edited(&mut self) {
        self.completions = Completions::Quiet;
        self.problem.clear();
    }

    pub(super) fn hide_question(&self) {
        self.question_visible.set(false);
    }

    /// Which question is being asked, if one is.
    pub(super) fn question_kind(&self) -> Option<u8> {
        self.question.as_ref().map(|question| match question {
            Question::Replace { .. } => 3,
            Question::CreateFolder { .. } => 4,
        })
    }
}

/// Whether two existing paths are the same file.
fn same_file(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// The last name of a path, for a dialog.
fn file_name(path: &Path) -> String {
    let name = path.file_name().unwrap_or(path.as_os_str());
    ui::clipped(&safe_text(&name.to_string_lossy()), 32)
}

/// A path for a dialog sentence: its end, which tells it apart, kept short
/// enough that the sentence stays on one row.
fn short_path(path: &Path) -> String {
    ui::clipped_suffix(&safe_text(&path.display().to_string()), 36)
}

impl App {
    /// Offer a path to start Save As from when the document has no file,
    /// as a recovered copy offers the file it came from.
    pub(crate) fn suggest_save_path(&mut self, path: Option<PathBuf>) {
        self.save_hint = path;
    }

    /// Open Save As on the document's own path with its name selected, so
    /// typing renames it and the folder stays. An untitled document starts
    /// in the working folder.
    pub(super) fn start_save_as(&mut self, quit_after: bool) {
        let known = self
            .file
            .as_ref()
            .map(|file| file.path.as_path())
            .or(self.save_hint.as_deref());
        let (text, name) = match known {
            Some(path) => {
                let text = path.display().to_string();
                let name = text.rfind('/').map_or(0, |slash| slash + 1);
                (text, name)
            }
            None => {
                let text = std::env::current_dir()
                    .map(|folder| paths::folder_prompt(&folder))
                    .unwrap_or_default();
                let end = text.len();
                (text, end)
            }
        };
        let mut path = Document::new(text);
        let head = path.text().len();
        let _ = path.set_selection(Selection {
            anchor: path.floor_grapheme_boundary(name),
            head,
        });
        self.save_as = SaveAs {
            path,
            ..SaveAs::default()
        };
        self.overlay = Overlay::SaveAs { quit_after };
    }

    /// Forget the dialog's state, including any file contents it holds.
    pub(super) fn close_save_as(&mut self) {
        self.save_as = SaveAs::default();
    }

    /// The file Save As would write if confirmed now, for the workspace to
    /// compare with its other tabs.
    pub(crate) fn pending_save_path(&self) -> Option<PathBuf> {
        if !self.saving_as() {
            return None;
        }
        match &self.save_as.question {
            Some(Question::Replace { file, .. }) => Some(file.path.clone()),
            Some(Question::CreateFolder { target, .. }) => Some(target.clone()),
            None => {
                let typed = self.save_as.path.text();
                (!typed.trim().is_empty()).then(|| paths::expand_home(typed))
            }
        }
    }

    /// Say in the dialog why its path cannot be used.
    pub(crate) fn save_as_problem(&mut self, problem: impl Into<String>) {
        self.save_as.question = None;
        self.save_as.completions = Completions::Quiet;
        self.save_as.problem = problem.into();
    }

    /// Typed or `pasted` text. A path from the root or the home folder
    /// cannot follow a folder, so where a name would begin it replaces the
    /// whole field: always when pasted, and when typed over a selection,
    /// such as the name Save As opens with. A slash typed after a slash,
    /// as after Tab has completed a folder, is the one already there.
    pub(super) fn save_as_insert(&mut self, text: &str, pasted: bool) {
        if self.save_as.question.is_some() {
            return;
        }
        let text: String = text.chars().filter(|c| !c.is_control()).collect();
        if text.is_empty() {
            return;
        }
        let field = &mut self.save_as.path;
        let selection = field.selection().range();
        let before = &field.text()[..selection.start];
        let rooted = text.starts_with('/') || text.starts_with('~');
        if rooted
            && (before.is_empty() || before.ends_with('/'))
            && (pasted || !selection.is_empty())
        {
            field.select_all();
        } else if text == "/" && before.ends_with('/') && selection.is_empty() {
            return;
        }
        field.insert(&text);
        self.save_as.edited();
    }

    pub(super) fn save_as_key(&mut self, key: KeyEvent, quit_after: bool) {
        if self.save_as.question.is_some() {
            // Only a plain, deliberate press answers, and only a question
            // that was on screen.
            if key.kind == KeyEventKind::Press
                && (key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT)
            {
                match key.code {
                    KeyCode::Char('n' | 'N') => self.save_as.question = None,
                    KeyCode::Char('y' | 'Y') if self.save_as.question_visible.get() => {
                        self.save_as_confirmed(quit_after)
                    }
                    _ => {}
                }
            }
            return;
        }
        let control = key.modifiers.contains(KeyModifiers::CONTROL);
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        let word = control || alt;
        let before = self.save_as.path.revision();
        let mut clipboard_message = None;
        let field = &mut self.save_as.path;
        match key.code {
            KeyCode::Enter => return self.save_as_submit(quit_after),
            KeyCode::Tab => return self.save_as_complete(),
            KeyCode::Char('a') if control => field.select_all(),
            KeyCode::Char('u') if control => {
                field.select_all();
                field.insert("");
            }
            KeyCode::Char('z') if control => {
                field.undo();
            }
            KeyCode::Char('y') if control => {
                field.redo();
            }
            KeyCode::Char('c' | 'x') if control => {
                if !field.selection().is_empty() {
                    let message = self.clipboard.copy(field.selected_text());
                    clipboard_message = Some((message, self.clipboard.warned()));
                    if key.code == KeyCode::Char('x') {
                        field.insert("");
                    }
                }
            }
            KeyCode::Char('v') if control => {
                let (text, message) = self.clipboard.paste();
                clipboard_message = Some((message, self.clipboard.warned()));
                if let Some(text) = text {
                    self.save_as_insert(&text, true);
                }
            }
            KeyCode::Left if word => field.move_word_left(shift),
            KeyCode::Right if word => field.move_word_right(shift),
            KeyCode::Left => field.move_left(shift),
            KeyCode::Right => field.move_right(shift),
            KeyCode::Home | KeyCode::End => {
                let head = if key.code == KeyCode::Home {
                    0
                } else {
                    field.text().len()
                };
                let anchor = if shift {
                    field.selection().anchor
                } else {
                    head
                };
                let _ = field.set_selection(Selection { anchor, head });
            }
            KeyCode::Backspace if word => {
                field.delete_word_backward();
            }
            KeyCode::Delete if word => {
                field.delete_word_forward();
            }
            KeyCode::Backspace => {
                field.backspace();
            }
            KeyCode::Delete => {
                field.delete_forward();
            }
            KeyCode::Char(c) if !word => self.save_as_insert(&c.to_string(), false),
            _ => {}
        }
        if self.save_as.path.revision() != before {
            self.save_as.edited();
        }
        if let Some((message, failed)) = clipboard_message {
            self.clipboard_feedback(message, failed);
        }
    }

    /// Tab: fill in the name as far as the folder's contents decide it.
    fn save_as_complete(&mut self) {
        let completion = paths::complete(self.save_as.path.text());
        let field = &mut self.save_as.path;
        if completion.text != field.text() {
            field.select_all();
            field.insert(&completion.text);
        }
        let end = field.text().len();
        let _ = field.set_caret(end);
        self.save_as.problem.clear();
        self.save_as.completions = if !completion.found {
            Completions::NoMatch
        } else if completion.candidates.is_empty() {
            Completions::Quiet
        } else {
            Completions::Several(completion.candidates)
        };
    }

    /// Enter: save to a new name at once; ask first when a file is in the
    /// way or the folder is missing; refuse what a save would refuse.
    fn save_as_submit(&mut self, quit_after: bool) {
        self.save_as.edited();
        let typed = self.save_as.path.text().to_owned();
        if typed.trim().is_empty() {
            return;
        }
        let target = paths::expand_home(&typed);
        if typed.ends_with('/') || target.is_dir() {
            if !typed.ends_with('/') {
                let field = &mut self.save_as.path;
                let end = field.text().len();
                let _ = field.set_caret(end);
                field.insert("/");
            }
            self.save_as.problem =
                "That is a folder. Add a file name; Tab lists what is in it".into();
            return;
        }
        match std::fs::symlink_metadata(&target) {
            Ok(_) => self.save_as_existing(target, quit_after),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let folder = target
                    .parent()
                    .filter(|folder| !folder.as_os_str().is_empty());
                match folder.map(std::fs::metadata) {
                    None => self.save_as_new(target, quit_after),
                    Some(Ok(metadata)) if metadata.is_dir() => self.save_as_new(target, quit_after),
                    Some(Ok(_)) => {
                        self.save_as.problem = "Part of that path is a file, not a folder".into();
                    }
                    Some(Err(error)) if error.kind() == io::ErrorKind::NotFound => {
                        self.save_as.question = Some(Question::CreateFolder {
                            folder: folder.unwrap_or(Path::new("")).to_owned(),
                            target,
                        });
                    }
                    Some(Err(error)) => self.save_as.problem = explain(&error),
                }
            }
            Err(error) => self.save_as.problem = explain(&error),
        }
    }

    /// The target exists. The document's own file, unchanged on disk, is an
    /// ordinary save. Anything else is asked about, unless replacing it is
    /// one of the things a save always refuses.
    fn save_as_existing(&mut self, target: PathBuf, quit_after: bool) {
        let own = self
            .file
            .as_ref()
            .is_some_and(|file| same_file(&file.path, &target));
        if own
            && let Some(file) = &mut self.file
            && file.save(self.document.text()).is_ok()
        {
            self.saved(None, quit_after);
            return;
        }
        match FileState::replacing(target, MAX_REPLACED) {
            Ok(file) => {
                self.save_as.question = Some(Question::Replace {
                    file: Box::new(file),
                    changed: own,
                });
            }
            Err(error) => {
                self.save_as.problem = format!("{}. Choose another name", explain(&error));
            }
        }
    }

    fn save_as_new(&mut self, target: PathBuf, quit_after: bool) {
        match FileState::new_target(target).and_then(|mut file| {
            file.save(self.document.text())?;
            Ok(file)
        }) {
            Ok(file) => self.saved(Some(file), quit_after),
            Err(error) => self.save_as.problem = explain(&error),
        }
    }

    /// Y: do what the question asked about.
    fn save_as_confirmed(&mut self, quit_after: bool) {
        self.save_as.hide_question();
        match self.save_as.question.take() {
            Some(Question::Replace { mut file, .. }) => match file.save(self.document.text()) {
                Ok(()) => self.saved(Some(*file), quit_after),
                Err(error) => {
                    self.save_as.problem = format!("{}; nothing was replaced", explain(&error));
                }
            },
            Some(Question::CreateFolder { folder, target }) => {
                match std::fs::create_dir_all(&folder) {
                    Ok(()) => self.save_as_new(target, quit_after),
                    Err(error) => {
                        self.save_as.problem =
                            format!("The folder could not be created: {}", explain(&error));
                    }
                }
            }
            None => {}
        }
    }

    /// The document is on disk: under `file` when Save As gave it a new
    /// one, with its kind following the new name.
    pub(super) fn saved(&mut self, file: Option<FileState>, quit_after: bool) {
        if let Some(file) = file {
            self.markdown = is_markdown(&file.path);
            self.language = (!self.markdown)
                .then(|| Language::for_path(&file.path))
                .flatten();
            self.live &= self.markdown;
            self.file = Some(file);
            self.save_hint = None;
            self.refresh_mode();
            self.layout_key = None;
        }
        let name = self
            .file
            .as_ref()
            .map(|file| safe_text(&file.path.display().to_string()))
            .unwrap_or_default();
        self.message = format!("Saved {name}");
        self.message_is_error = false;
        self.message_origin = MessageOrigin::General;
        self.last_disk_change = None;
        self.document.mark_saved();
        self.overlay = Overlay::None;
        self.close_save_as();
        self.should_exit = quit_after;
    }

    pub(super) fn draw_save_as(&self, frame: &mut Frame) {
        let state = &self.save_as;
        state.hide_question();
        let colors = palette();
        let strong = |text: String| Span::styled(text, ui::surface().add_modifier(Modifier::BOLD));
        if let Some(question) = &state.question {
            let (title, body, yes) = match question {
                Question::Replace { file, changed } => {
                    let name = file_name(&file.path);
                    let first = if *changed {
                        Line::from(vec![
                            strong(name),
                            " changed on disk after you opened it.".into(),
                        ])
                    } else if let Some(target) = file.link_target() {
                        Line::from(vec![
                            strong(name),
                            format!(" is a link to {}.", short_path(target)).into(),
                        ])
                    } else {
                        Line::from(vec![strong(name), " already exists.".into()])
                    };
                    let second = if *changed {
                        "Replace the version on disk with this document?"
                    } else if file.link_target().is_some() {
                        "Replace that file with this document? What it holds now will be lost."
                    } else {
                        "Replace it with this document? What it holds now will be lost."
                    };
                    (
                        "Replace file?",
                        vec![first, Line::from(Span::styled(second, ui::muted()))],
                        "Replace",
                    )
                }
                Question::CreateFolder { folder, target } => (
                    "Create folder?",
                    vec![
                        Line::from(vec![
                            "The folder ".into(),
                            strong(short_path(folder)),
                            " does not exist.".into(),
                        ]),
                        Line::from(Span::styled(
                            format!("Create it and save {} there?", file_name(target)),
                            ui::muted(),
                        )),
                    ],
                    "Create and save",
                ),
            };
            let shown = ui::dialog(
                frame,
                title,
                &body,
                &[("Y", yes), ("N", "Choose another name"), ("Esc", "Cancel")],
                78,
            );
            state.question_visible.set(shown);
            return;
        }
        // The field, then two rows that say what Tab found or what went
        // wrong, so the dialog keeps its size while the path is typed.
        let Some(area) = ui::dialog_frame(
            frame,
            "Save as",
            |_| 3,
            &[("⏎", "Save"), ("Tab", "Complete"), ("Esc", "Cancel")],
            78,
        ) else {
            return;
        };
        draw_field(
            frame,
            Rect::new(area.x, area.y, area.width, 1),
            "❯ ",
            &state.path,
            Focus::Query,
            true,
            None,
        );
        let room = usize::from(area.width);
        let note = if !state.problem.is_empty() {
            Span::styled(safe_text(&state.problem), ui::surface().fg(colors.warning))
        } else {
            let text = match &state.completions {
                Completions::Quiet => {
                    "Tab completes names. An existing file asks before it is replaced.".to_owned()
                }
                Completions::NoMatch => "No name here starts like that.".to_owned(),
                Completions::Several(names) => listed(names, room * 2),
            };
            Span::styled(text, ui::muted())
        };
        frame.render_widget(
            Paragraph::new(Line::from(note))
                .wrap(ratatui::widgets::Wrap { trim: false })
                .style(ui::surface()),
            Rect::new(
                area.x,
                area.y + 1,
                area.width,
                area.height.saturating_sub(1),
            ),
        );
    }
}

/// As many names as fit in `room` cells, then how many more there are.
fn listed(names: &[String], room: usize) -> String {
    let mut text = String::new();
    for (index, name) in names.iter().enumerate() {
        let name = safe_text(name);
        let rest = names.len() - index;
        let more = format!("  +{rest} more");
        let separator = if text.is_empty() { "" } else { "  " };
        let after = UnicodeWidthStr::width(text.as_str())
            + separator.len()
            + UnicodeWidthStr::width(name.as_str());
        // Wrapping can waste the end of the first row; leave it a name's worth.
        let reserve = if rest > 1 { more.len() } else { 0 } + 24;
        if after + reserve > room && !text.is_empty() {
            text.push_str(&more);
            break;
        }
        text.push_str(separator);
        text.push_str(&name);
    }
    text
}

#[cfg(test)]
#[path = "app_save_as_tests.rs"]
mod tests;
