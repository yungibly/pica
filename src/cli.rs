use crate::{icons::IconSet, theme::Theme};
use std::{
    ffi::OsString,
    io,
    path::{Path, PathBuf},
};

pub const HELP: &str = "pica — a small terminal text editor, at home with Markdown

Usage: pica [OPTIONS] [+LINE[:COL]] [FILE]...
       COMMAND | pica [OPTIONS]
       pica --snapshot [--snapshot-size WxH] [OPTIONS] [FILE]...

Each FILE opens in a tab; the first is active. +LINE[:COL] before or after
a file puts the caret there, and so does FILE:LINE[:COL] when no file has
that exact name. A folder opens the file browser there. \"-\", or input
piped in, opens as an unsaved plain-text buffer. Without a FILE, pica
opens an untitled Markdown document. Files must be UTF-8 text, at most
8 MiB. Put -- before names that start with a dash or a plus.

Options:
  --theme NAME            Choose a built-in theme (name or ID)
  --icons plain|nerd      Optional Nerd Font icons (default: plain)
  --list-themes           List built-in theme IDs and names
  --no-state              Disable preferences and crash recovery
  --source                Open Markdown with its source visible
  --no-wrap               Keep each line on one row; the view scrolls sideways
  --snapshot              Print the real renderer without opening a terminal
  --snapshot-size WxH     Snapshot dimensions (default: 80x24; max: 400x160)
  -h, --help              Show this help
  -V, --version           Show the version

Keys, as in micro (Ctrl+G lists them all):
  Ctrl+S  Save              Ctrl+O  Open              Ctrl+Q  Close tab, quit
  Ctrl+E  Commands          Ctrl+G  Help              Ctrl+L  Go to line
  Ctrl+F  Find              Ctrl+N  Next match        Ctrl+P  Previous match
  Ctrl+Z  Undo              Ctrl+Y  Redo              Ctrl+A  Select all
  Ctrl+C  Copy              Ctrl+X  Cut               Ctrl+V  Paste
  Ctrl+K  Cut line          Ctrl+D  Duplicate         Ctrl+/  Comment
  Ctrl+T  New tab           Alt+,   Previous tab      Alt+.   Next tab
  Ctrl+R  Line numbers; in Markdown, live or source view; in Find, Replace
  Markdown: Ctrl+B bold, Alt+I italic, Alt+T task, Enter continues lists

Saving never overwrites a file that changed on disk without asking, and
Save as (from Ctrl+E) asks before it replaces any existing file. Saving
through a symbolic link replaces the file it points to and keeps the link.
";

/// A 1-based line and grapheme column.
pub type Position = (usize, usize);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Input {
    File(PathBuf),
    /// `-`: read standard input into an unsaved buffer.
    Stdin,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub input: Input,
    pub position: Option<Position>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Options {
    pub targets: Vec<Target>,
    /// `+LINE[:COL]` given without any file; it applies to piped input.
    pub position: Option<Position>,
    pub snapshot: bool,
    pub source: bool,
    /// `--no-wrap`: word wrap off for this launch.
    pub no_wrap: bool,
    pub theme: Option<Theme>,
    pub icons: Option<IconSet>,
    pub no_state: bool,
    pub size: (u16, u16),
}

#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    Edit(Options),
    Help,
    Version,
    ListThemes,
}

pub fn parse(arguments: impl IntoIterator<Item = OsString>) -> io::Result<Action> {
    let mut options = Options {
        targets: Vec::new(),
        position: None,
        snapshot: false,
        source: false,
        no_wrap: false,
        theme: None,
        icons: None,
        no_state: false,
        size: (80, 24),
    };
    let mut positional = false;
    let mut explicit_size = false;
    // A position binds to the file after it, or else to the last file.
    let mut pending = None;
    let mut arguments = arguments.into_iter();
    while let Some(argument) = arguments.next() {
        if !positional {
            match argument.to_str() {
                Some("--help" | "-h") => return Ok(Action::Help),
                Some("--version" | "-V") => return Ok(Action::Version),
                Some("--list-themes") => return Ok(Action::ListThemes),
                Some("--no-state") => {
                    options.no_state = true;
                    continue;
                }
                Some("--snapshot") => {
                    options.snapshot = true;
                    continue;
                }
                Some("--source") => {
                    options.source = true;
                    continue;
                }
                Some("--no-wrap") => {
                    options.no_wrap = true;
                    continue;
                }
                Some("--") => {
                    positional = true;
                    continue;
                }
                Some(value)
                    if value == "--theme"
                        || value == "--icons"
                        || value.starts_with("--icons=")
                        || value == "--snapshot-size"
                        || value.starts_with("--theme=")
                        || value.starts_with("--snapshot-size=") =>
                {
                    let (flag, value) = match value.split_once('=') {
                        Some((flag, value)) => (flag, OsString::from(value)),
                        None => (
                            value,
                            arguments
                                .next()
                                .ok_or_else(|| invalid(format!("{value} needs a value")))?,
                        ),
                    };
                    let value = value
                        .to_str()
                        .ok_or_else(|| invalid("Invalid option value"))?;
                    if flag == "--theme" {
                        options.theme = Some(Theme::from_name(value).ok_or_else(|| {
                            invalid(format!("Unknown theme {value:?}; use --list-themes"))
                        })?);
                    } else if flag == "--icons" {
                        options.icons = Some(
                            IconSet::from_name(value)
                                .ok_or_else(|| invalid("--icons must be plain or nerd"))?,
                        );
                    } else {
                        options.size = parse_size(value)?;
                        explicit_size = true;
                    }
                    continue;
                }
                Some(value) if value.starts_with('+') && parse_position(&value[1..]).is_some() => {
                    pending = parse_position(&value[1..]);
                    continue;
                }
                Some("-") => {}
                Some(value) if value.starts_with('-') => {
                    return Err(invalid(format!("Unknown option {value}; use --help")));
                }
                _ => {}
            }
        }
        if argument.is_empty() {
            return Err(invalid("The file path is empty"));
        }
        let input = if argument == "-" {
            if options.targets.iter().any(|t| t.input == Input::Stdin) {
                return Err(invalid("Standard input (-) can only be read once"));
            }
            Input::Stdin
        } else {
            Input::File(PathBuf::from(argument))
        };
        options.targets.push(Target {
            input,
            position: pending.take(),
        });
    }
    if let Some(position) = pending {
        match options.targets.last_mut() {
            Some(target) => target.position = Some(position),
            None => options.position = Some(position),
        }
    }
    if explicit_size && !options.snapshot {
        return Err(invalid("--snapshot-size requires --snapshot"));
    }
    Ok(Action::Edit(options))
}

/// `LINE` or `LINE:COL` in decimal digits; a missing column is 1.
fn parse_position(text: &str) -> Option<Position> {
    let digits = |text: &str| !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit());
    let (line, column) = match text.split_once(':') {
        Some((line, column)) if digits(line) && digits(column) => (line, Some(column)),
        None if digits(text) => (text, None),
        _ => return None,
    };
    let number = |text: &str| text.parse::<usize>().unwrap_or(usize::MAX);
    Some((number(line), column.map_or(1, number)))
}

/// Read `FILE:LINE[:COL]`, as compilers and grep print it, only when no
/// file has that literal name and the file before the colon exists.
pub fn locate(path: PathBuf, position: Option<Position>) -> (PathBuf, Option<Position>) {
    if position.is_some() || std::fs::symlink_metadata(&path).is_ok() {
        return (path, position);
    }
    let Some(text) = path.to_str() else {
        return (path, position);
    };
    // Try `FILE:LINE:COL` first, then `FILE:LINE`.
    let mut candidates = Vec::new();
    if let Some((rest, column)) = text.rsplit_once(':')
        && let Some((file, line)) = rest.rsplit_once(':')
    {
        candidates.push((file, format!("{line}:{column}")));
    }
    if let Some((file, line)) = text.rsplit_once(':') {
        candidates.push((file, line.to_owned()));
    }
    for (file, suffix) in candidates {
        if let Some(found) = parse_position(&suffix)
            && !file.is_empty()
            && std::fs::metadata(Path::new(file)).is_ok()
        {
            return (PathBuf::from(file), Some(found));
        }
    }
    (path, position)
}

fn parse_size(value: &str) -> io::Result<(u16, u16)> {
    let size = value.split_once(['x', 'X']).and_then(|(width, height)| {
        Some((width.parse::<u16>().ok()?, height.parse::<u16>().ok()?))
    });
    match size {
        Some((width @ 1..=400, height @ 1..=160)) => Ok((width, height)),
        _ => Err(invalid(
            "--snapshot-size must be WIDTHxHEIGHT, from 1x1 to 400x160",
        )),
    }
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> io::Result<Action> {
        parse(values.iter().map(OsString::from))
    }

    fn targets(values: &[&str]) -> (Vec<Target>, Option<Position>) {
        let Action::Edit(options) = args(values).unwrap() else {
            panic!("{values:?}")
        };
        (options.targets, options.position)
    }

    fn file(path: &str, position: Option<Position>) -> Target {
        Target {
            input: Input::File(PathBuf::from(path)),
            position,
        }
    }

    #[test]
    fn defaults_and_explicit_snapshot_settings() {
        assert_eq!(
            args(&[]).unwrap(),
            Action::Edit(Options {
                targets: vec![],
                position: None,
                snapshot: false,
                source: false,
                no_wrap: false,
                theme: None,
                icons: None,
                no_state: false,
                size: (80, 24),
            })
        );
        assert_eq!(
            args(&[
                "--snapshot",
                "--source",
                "--no-wrap",
                "--theme=light",
                "--icons=nerd",
                "--snapshot-size",
                "160x45",
                "notes.md"
            ])
            .unwrap(),
            Action::Edit(Options {
                targets: vec![file("notes.md", None)],
                position: None,
                snapshot: true,
                source: true,
                no_wrap: true,
                theme: Some(Theme::Light),
                icons: Some(IconSet::Nerd),
                no_state: false,
                size: (160, 45),
            })
        );
    }

    #[test]
    fn several_files_open_in_order() {
        let (targets, _) = targets(&["a.md", "b.rs", "--source", "c.txt"]);
        assert_eq!(
            targets,
            vec![file("a.md", None), file("b.rs", None), file("c.txt", None)]
        );
    }

    #[test]
    fn positions_bind_to_the_next_file_or_else_the_last() {
        assert_eq!(
            targets(&["+12", "a.md"]).0,
            vec![file("a.md", Some((12, 1)))]
        );
        assert_eq!(
            targets(&["a.md", "+3:7"]).0,
            vec![file("a.md", Some((3, 7)))]
        );
        assert_eq!(
            targets(&["a.md", "+2", "b.md"]).0,
            vec![file("a.md", None), file("b.md", Some((2, 1)))]
        );
        assert_eq!(
            targets(&["+2", "a.md", "+5"]).0,
            vec![file("a.md", Some((5, 1)))]
        );
        // Without a file, a position waits for piped input.
        assert_eq!(targets(&["+4"]), (vec![], Some((4, 1))));
        // Anything else starting with + is a file name, as is anything after --.
        assert_eq!(targets(&["+notes"]).0, vec![file("+notes", None)]);
        assert_eq!(targets(&["+3:"]).0, vec![file("+3:", None)]);
        assert_eq!(targets(&["--", "+3"]).0, vec![file("+3", None)]);
        assert_eq!(
            targets(&["+99999999999999999999999", "a"]).0,
            vec![file("a", Some((usize::MAX, 1)))]
        );
    }

    #[test]
    fn a_dash_reads_standard_input_once() {
        let (targets, _) = targets(&["a.md", "-", "+9"]);
        assert_eq!(targets[0], file("a.md", None));
        assert_eq!(
            targets[1],
            Target {
                input: Input::Stdin,
                position: Some((9, 1))
            }
        );
        assert!(args(&["-", "-"]).is_err());
    }

    #[test]
    fn invalid_flags_dimensions_and_values_fail_before_opening_a_file() {
        for values in [
            vec!["--theme"],
            vec!["--theme", "no-such-theme-987654321"],
            vec!["--theme="],
            vec!["--icons"],
            vec!["--icons", "unknown"],
            vec!["--icons="],
            vec!["--snapshot-size", "120x36"],
            vec!["--unknown"],
            vec!["-x"],
        ] {
            assert_eq!(
                args(&values).unwrap_err().kind(),
                io::ErrorKind::InvalidInput
            );
        }
        for size in [
            "0x24",
            "80x0",
            "401x24",
            "80x161",
            "999999x24",
            "80",
            "80x24x2",
            "-2x24",
        ] {
            assert!(
                args(&["--snapshot", "--snapshot-size", size]).is_err(),
                "{size}"
            );
        }
        assert!(args(&["--snapshot-size=400X160", "--snapshot"]).is_ok());
    }

    #[test]
    fn separator_preserves_option_like_filename() {
        let Action::Edit(options) = args(&["--", "--theme=light"]).unwrap() else {
            panic!()
        };
        assert_eq!(options.targets, vec![file("--theme=light", None)]);
        assert_eq!(options.theme, None);
        assert!(args(&[""]).is_err());
        assert!(args(&["--", ""]).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn filenames_do_not_need_utf8() {
        use std::os::unix::ffi::OsStringExt;
        let path = OsString::from_vec(b"notes-\xff.md".to_vec());
        let Action::Edit(options) = parse([path.clone()]).unwrap() else {
            panic!()
        };
        assert_eq!(
            options.targets,
            vec![Target {
                input: Input::File(PathBuf::from(path)),
                position: None
            }]
        );
    }

    #[test]
    fn file_colon_line_applies_only_when_the_literal_path_is_missing() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("main.rs");
        std::fs::write(&file, "fn main() {}\n").unwrap();
        let at = |suffix: &str| PathBuf::from(format!("{}{suffix}", file.display()));
        assert_eq!(locate(at(":12"), None), (file.clone(), Some((12, 1))));
        assert_eq!(locate(at(":12:5"), None), (file.clone(), Some((12, 5))));
        // An explicit +LINE wins, and other suffixes stay literal.
        assert_eq!(locate(at(":12"), Some((3, 1))), (at(":12"), Some((3, 1))));
        assert_eq!(locate(at(":x"), None), (at(":x"), None));
        assert_eq!(locate(at(":12:"), None), (at(":12:"), None));
        // A file with that exact name always wins.
        let literal = at(":7");
        std::fs::write(&literal, "").unwrap();
        assert_eq!(locate(literal.clone(), None), (literal, None));
        // A missing file before the colon makes the name literal too.
        let missing = dir.path().join("new.md:3");
        assert_eq!(locate(missing.clone(), None), (missing, None));
        assert_eq!(locate(at(""), None), (file, None));
    }
}
