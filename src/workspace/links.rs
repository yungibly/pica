//! Following links. Only an explicit request gets here: the command, its
//! key, or Ctrl+click. `links::classify` decides what a destination is; web
//! and mail addresses go to the platform's opener, local text files open in
//! a tab, folders in the file browser, and `#fragments` go to a heading.
use super::{Browser, Workspace, safe_text};
use crate::links::{self, Target};
use std::{fs, io, path::PathBuf};

impl Workspace {
    /// Alt+L, or "Open link" from the palette.
    pub(super) fn open_link_at_caret(&mut self) {
        match self.editor_mut().link_at_caret() {
            Some(destination) => self.open_link(&destination),
            None => self.editor_mut().inform("No link under the caret"),
        }
    }

    /// Follow `destination`, written in the active document.
    pub(super) fn open_link(&mut self, destination: &str) {
        let folder = self.editor().folder();
        match links::classify(destination, &folder) {
            Target::Web(address) => match (self.opener)(&address) {
                Ok(()) => {
                    let shown = safe_text(&address);
                    self.editor_mut().inform(format!("Opening {shown}"));
                }
                Err(error) => self
                    .editor_mut()
                    .set_message(format!("Could not open the link: {error}")),
            },
            Target::Heading(fragment) => self.go_to_fragment(&fragment),
            Target::File { path, fragment } => self.open_linked_file(path, fragment),
            Target::Refused(reason) => self.editor_mut().inform(reason),
        }
    }

    /// A link to a local path: a text file opens in a tab, or comes to the
    /// front if it is open already; a folder opens the file browser there.
    /// Nothing is created, and nothing but a regular file is read. Each
    /// answer leads with what happened, since a long path may be cut short.
    fn open_linked_file(&mut self, path: PathBuf, fragment: Option<String>) {
        let shown = safe_text(&path.display().to_string());
        let kind = match fs::metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                self.editor_mut()
                    .inform(format!("The linked file does not exist: {shown}"));
                return;
            }
            Err(error) => {
                let problem = crate::file_io::explain(&error);
                self.editor_mut()
                    .inform(format!("Cannot open the linked file: {problem} · {shown}"));
                return;
            }
        };
        if kind.is_dir() {
            self.editor_mut().deactivate();
            match Browser::new(path) {
                Ok(browser) => self.browser = Some(browser),
                Err(error) => {
                    let problem = crate::file_io::explain(&error);
                    self.editor_mut().inform(format!(
                        "Cannot open the linked folder: {problem} · {shown}"
                    ));
                }
            }
            self.tab_hits.clear();
            self.sidebar.invalidate();
            return;
        }
        if !kind.is_file() {
            self.editor_mut()
                .inform(format!("The link does not lead to a file: {shown}"));
            return;
        }
        match self.open_path(path) {
            Ok(()) => {
                if let Some(fragment) = fragment {
                    self.go_to_fragment(&fragment);
                }
            }
            Err(error) => {
                let problem = crate::file_io::explain(&error);
                self.editor_mut()
                    .inform(format!("Cannot open the linked file: {problem} · {shown}"));
            }
        }
    }

    /// Put the caret on the heading a `#fragment` names in the active
    /// document.
    fn go_to_fragment(&mut self, fragment: &str) {
        let found = self.editor_mut().outline().and_then(|outline| {
            let titles: Vec<&str> = outline.iter().map(|entry| entry.title.as_str()).collect();
            links::heading(&titles, fragment).map(|index| outline[index].offset)
        });
        match found {
            Some(offset) => self.editor_mut().jump_to_source(offset),
            None => {
                let shown = safe_text(fragment);
                self.editor_mut()
                    .inform(format!("No heading here matches #{shown}"));
            }
        }
    }
}
