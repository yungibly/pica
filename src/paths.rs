//! Paths as people type them: `~` for the home folder, and completing a
//! name from the folder it is in. Nothing here scans below one folder.
use std::{
    fs,
    path::{Path, PathBuf},
};

/// Entries read from one folder when completing a name.
const MAX_ENTRIES: usize = 4096;

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
        .filter(|home| home.is_absolute())
}

/// `~` and `~/…` name the home folder, as in a shell. `~name` and a tilde
/// anywhere else are ordinary characters.
pub fn expand_home(text: &str) -> PathBuf {
    expand_in(text, home().as_deref())
}

fn expand_in(text: &str, home: Option<&Path>) -> PathBuf {
    match (text.strip_prefix('~'), home) {
        (Some(""), Some(home)) => home.to_owned(),
        (Some(rest), Some(home)) if rest.starts_with('/') => {
            home.join(rest.trim_start_matches('/'))
        }
        _ => PathBuf::from(text),
    }
}

/// A folder as a path to start typing from: under the home folder it
/// begins with `~/`, and it always ends with a slash.
pub fn folder_prompt(folder: &Path) -> String {
    folder_prompt_in(folder, home().as_deref())
}

fn folder_prompt_in(folder: &Path, home: Option<&Path>) -> String {
    let mut text = match home.and_then(|home| folder.strip_prefix(home).ok()) {
        Some(rest) if rest.as_os_str().is_empty() => "~".to_owned(),
        Some(rest) => format!("~/{}", rest.display()),
        None => folder.display().to_string(),
    };
    if !text.ends_with('/') {
        text.push('/');
    }
    text
}

/// What Tab makes of a typed path.
#[derive(Debug, PartialEq, Eq)]
pub struct Completion {
    /// The path with as much of the name filled in as the matches agree on.
    pub text: String,
    /// Every match, folders ending with a slash, when there are several.
    pub candidates: Vec<String>,
    /// Whether anything in the folder matched.
    pub found: bool,
}

/// Complete the last name of `text` from the folder before it, as a shell
/// does: one match is filled in whole, with a slash after a folder; several
/// are filled in as far as they agree. Names match by their start, exactly
/// or else ignoring case. Dot files match only a name that starts with a dot.
pub fn complete(text: &str) -> Completion {
    let split = text.rfind('/').map_or(0, |slash| slash + 1);
    let (folder, prefix) = text.split_at(split);
    let directory = if folder.is_empty() {
        PathBuf::from(".")
    } else {
        expand_home(folder)
    };
    let mut names: Vec<(String, bool)> = fs::read_dir(&directory)
        .into_iter()
        .flatten()
        .take(MAX_ENTRIES)
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            if name.starts_with('.') && !prefix.starts_with('.') {
                return None;
            }
            // A link to a folder completes as a folder.
            let folder = fs::metadata(entry.path()).is_ok_and(|metadata| metadata.is_dir());
            Some((name, folder))
        })
        .collect();
    let exact: Vec<_> = names
        .iter()
        .filter(|(name, _)| name.starts_with(prefix))
        .cloned()
        .collect();
    if exact.is_empty() {
        let lower = prefix.to_lowercase();
        names.retain(|(name, _)| name.to_lowercase().starts_with(&lower));
    } else {
        names = exact;
    }
    names.sort();
    let label = |(name, folder): &(String, bool)| {
        if *folder {
            format!("{name}/")
        } else {
            name.clone()
        }
    };
    match names.as_slice() {
        [] => Completion {
            text: text.to_owned(),
            candidates: vec![],
            found: false,
        },
        [only] => Completion {
            text: format!("{folder}{}", label(only)),
            candidates: vec![],
            found: true,
        },
        several => {
            let first = &several[0].0;
            let mut shared = first.len();
            for (name, _) in &several[1..] {
                shared = first
                    .char_indices()
                    .zip(name.chars())
                    .find(|((_, a), b)| a != b)
                    .map_or(shared.min(name.len()), |((at, _), _)| shared.min(at));
            }
            // Matches found by ignoring case may agree on less than was
            // typed; what was typed is never shortened.
            let name = if first[..shared].chars().count() >= prefix.chars().count() {
                &first[..shared]
            } else {
                prefix
            };
            Completion {
                text: format!("{folder}{name}"),
                candidates: several.iter().map(label).collect(),
                found: true,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_leading_tilde_is_the_home_folder_and_any_other_is_a_character() {
        let home = Path::new("/home/someone");
        assert_eq!(expand_in("~", Some(home)), home);
        assert_eq!(expand_in("~/", Some(home)), home);
        assert_eq!(
            expand_in("~/notes/a.md", Some(home)),
            home.join("notes/a.md")
        );
        for literal in ["~other/a.md", "a~", "notes/~", "~backup.md", ""] {
            assert_eq!(expand_in(literal, Some(home)), PathBuf::from(literal));
        }
        // Without a home folder a tilde stays as typed.
        assert_eq!(expand_in("~/a.md", None), PathBuf::from("~/a.md"));
        assert_eq!(folder_prompt_in(home, Some(home)), "~/");
        assert_eq!(
            folder_prompt_in(&home.join("notes"), Some(home)),
            "~/notes/"
        );
        assert_eq!(folder_prompt_in(Path::new("/tmp/x"), Some(home)), "/tmp/x/");
        assert_eq!(folder_prompt_in(Path::new("/"), Some(home)), "/");
        assert_eq!(
            folder_prompt_in(Path::new("/home/someone-else"), Some(home)),
            "/home/someone-else/"
        );
        assert_eq!(folder_prompt_in(Path::new("/tmp/x"), None), "/tmp/x/");
        // What the prompt shows expands back to the same folder.
        let prompt = folder_prompt_in(&home.join("notes"), Some(home));
        assert_eq!(expand_in(&prompt, Some(home)), home.join("notes"));
    }

    #[test]
    fn tab_fills_in_one_match_and_as_much_as_several_agree_on() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().display().to_string();
        for folder in ["notes", "Nested Folder", ".hidden-folder"] {
            fs::create_dir(dir.path().join(folder)).unwrap();
        }
        for file in [
            "notes.md",
            "notes-old.md",
            "plan.md",
            ".env",
            "résumé-a.md",
            "résumé-b.md",
        ] {
            fs::write(dir.path().join(file), "").unwrap();
        }
        let complete_in = |typed: &str| {
            let completion = complete(&format!("{root}/{typed}"));
            let text = completion
                .text
                .strip_prefix(&format!("{root}/"))
                .map(str::to_owned);
            (text.unwrap(), completion.candidates, completion.found)
        };
        // One match completes whole; a folder gains its slash.
        assert_eq!(complete_in("pl"), ("plan.md".into(), vec![], true));
        assert_eq!(complete_in("Nes"), ("Nested Folder/".into(), vec![], true));
        // Several complete as far as they agree and are listed by name,
        // a folder with its slash.
        let (text, candidates, found) = complete_in("no");
        assert_eq!(text, "notes");
        assert_eq!(candidates, ["notes/", "notes-old.md", "notes.md"]);
        assert!(found);
        assert_eq!(complete_in("notes").0, "notes");
        assert_eq!(complete_in("notes.").0, "notes.md");
        // Shared text stops on a character boundary.
        let (text, candidates, _) = complete_in("r");
        assert_eq!(text, "résumé-");
        assert_eq!(candidates.len(), 2);
        // Case is ignored only when nothing matches exactly, and the real
        // name's case is taken.
        assert_eq!(complete_in("PLAN").0, "plan.md");
        assert_eq!(complete_in("nested").0, "Nested Folder/");
        let (text, candidates, _) = complete_in("NO");
        assert_eq!(text, "notes");
        assert_eq!(candidates.len(), 3);
        // Dot files need a dot.
        let (text, candidates, _) = complete_in("");
        assert_eq!(text, "");
        assert!(!candidates.iter().any(|name| name.starts_with('.')));
        assert_eq!(candidates.len(), 7);
        assert_eq!(complete_in(".e").0, ".env");
        assert_eq!(complete_in(".h").0, ".hidden-folder/");
        // Nothing matches: the text stays as typed.
        assert_eq!(complete_in("zzz"), ("zzz".into(), vec![], false));
        assert_eq!(
            complete(&format!("{root}/missing-folder/x")),
            Completion {
                text: format!("{root}/missing-folder/x"),
                candidates: vec![],
                found: false,
            }
        );
        // Inside a completed folder, and through a link to one.
        fs::write(dir.path().join("notes/inner.md"), "").unwrap();
        assert_eq!(complete_in("notes/i").0, "notes/inner.md");
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(dir.path().join("notes"), dir.path().join("link")).unwrap();
            assert_eq!(complete_in("li").0, "link/");
            assert_eq!(complete_in("link/in").0, "link/inner.md");
        }
    }
}
