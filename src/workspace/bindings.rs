//! Keys the user has bound in `keys.conf`. Each line gives a key the action
//! of a command or of another key, or takes its action away:
//!
//! ```text
//! ctrl+j = duplicate
//! alt+q  = quit
//! f5     = reload
//! alt+w  = ctrl+q
//! ctrl+k = none
//! ```
//!
//! pica only reads this file. Without one, every key keeps its default.
use super::palette::Command;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// A key as bindings compare it: letters in lowercase, and only the
/// modifiers a binding can name.
type Key = (KeyCode, KeyModifiers);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Bound {
    Command(Command),
    /// Act as this key does by default.
    Key(KeyCode, KeyModifiers),
    /// Do nothing.
    Nothing,
}

#[derive(Default)]
pub(super) struct Bindings {
    /// One entry a key; a later line for the same key replaces an earlier one.
    keys: Vec<(Key, Bound)>,
}

/// The modifiers a binding can name.
const NAMED: KeyModifiers = KeyModifiers::CONTROL
    .union(KeyModifiers::ALT)
    .union(KeyModifiers::SHIFT);

/// Shift is part of a letter or a named key (`ctrl+shift+s`, `shift+f3`),
/// but a symbol already says how it is typed: `alt+?` is one key however
/// the terminal reports Shift.
fn normalize(code: KeyCode, modifiers: KeyModifiers) -> Key {
    let modifiers = modifiers & NAMED;
    match code {
        KeyCode::Char(c) if c.is_alphabetic() => (
            KeyCode::Char(c.to_lowercase().next().unwrap_or(c)),
            modifiers,
        ),
        KeyCode::Char(c) => (KeyCode::Char(c), modifiers - KeyModifiers::SHIFT),
        KeyCode::BackTab => (KeyCode::Tab, modifiers | KeyModifiers::SHIFT),
        code => (code, modifiers),
    }
}

/// `ctrl+shift+s`, `alt+up`, `f5`, `ctrl+/`: modifiers, then one key.
pub(super) fn parse_key(text: &str) -> Result<Key, String> {
    let lower = text.trim().to_lowercase();
    // A final `+` is the plus key itself, as in `ctrl++`.
    let (rest, name) = match lower.strip_suffix('+') {
        Some(rest) if rest.is_empty() || rest.ends_with('+') => (rest, "+"),
        _ => match lower.rsplit_once('+') {
            Some((rest, name)) => (rest, name),
            None => ("", lower.as_str()),
        },
    };
    let mut modifiers = KeyModifiers::NONE;
    for word in rest
        .split('+')
        .map(str::trim)
        .filter(|word| !word.is_empty())
    {
        modifiers |= match word {
            "ctrl" | "control" => KeyModifiers::CONTROL,
            "alt" | "option" | "meta" => KeyModifiers::ALT,
            "shift" => KeyModifiers::SHIFT,
            _ => return Err(format!("unknown modifier {word:?}")),
        };
    }
    let name = name.trim();
    let mut characters = name.chars();
    let code = match name {
        "enter" | "return" => KeyCode::Enter,
        "tab" => KeyCode::Tab,
        "backspace" => KeyCode::Backspace,
        "delete" | "del" => KeyCode::Delete,
        "space" => KeyCode::Char(' '),
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        "pageup" | "pgup" => KeyCode::PageUp,
        "pagedown" | "pgdn" => KeyCode::PageDown,
        "esc" | "escape" => KeyCode::Esc,
        _ => match (characters.next(), characters.next()) {
            (Some(c), None) if !c.is_control() => KeyCode::Char(c),
            (Some('f'), Some(_)) => match name[1..].parse::<u8>() {
                Ok(number @ 1..=12) => KeyCode::F(number),
                _ => return Err(format!("unknown key {name:?}")),
            },
            _ => return Err(format!("unknown key {name:?}")),
        },
    };
    Ok(normalize(code, modifiers))
}

/// How the palette and messages show a key: `Ctrl+Shift+S`, `Alt+Up`, `F5`.
pub(super) fn key_name((code, modifiers): Key) -> String {
    let mut name = String::new();
    for (modifier, label) in [
        (KeyModifiers::CONTROL, "Ctrl+"),
        (KeyModifiers::ALT, "Alt+"),
        (KeyModifiers::SHIFT, "Shift+"),
    ] {
        if modifiers.contains(modifier) {
            name.push_str(label);
        }
    }
    match code {
        KeyCode::Char(' ') => name.push_str("Space"),
        KeyCode::Char(c) => name.extend(c.to_uppercase()),
        KeyCode::F(number) => name.push_str(&format!("F{number}")),
        KeyCode::Enter => name.push_str("Enter"),
        KeyCode::Tab => name.push_str("Tab"),
        KeyCode::Backspace => name.push_str("Backspace"),
        KeyCode::Delete => name.push_str("Delete"),
        KeyCode::Up => name.push_str("Up"),
        KeyCode::Down => name.push_str("Down"),
        KeyCode::Left => name.push_str("Left"),
        KeyCode::Right => name.push_str("Right"),
        KeyCode::Home => name.push_str("Home"),
        KeyCode::End => name.push_str("End"),
        KeyCode::PageUp => name.push_str("PageUp"),
        KeyCode::PageDown => name.push_str("PageDown"),
        KeyCode::Esc => name.push_str("Esc"),
        _ => name.push('?'),
    }
    name
}

impl Bindings {
    /// Read `keys.conf` text. Lines that cannot be used are skipped and
    /// described, each with its line number, so the rest still apply.
    pub fn parse(text: &str) -> (Self, Vec<String>) {
        let mut bindings = Self::default();
        let mut problems = Vec::new();
        for (number, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut problem =
                |problem: String| problems.push(format!("line {}: {problem}", number + 1));
            // Either side may be the `=` key, as in `alt+= = undo`.
            let Some((key, action)) = line.split_once(" = ").or_else(|| line.rsplit_once('='))
            else {
                problem("expected KEY = COMMAND".into());
                continue;
            };
            let key = match parse_key(key) {
                Ok(key) => key,
                Err(error) => {
                    problem(error);
                    continue;
                }
            };
            // Plain characters and editing keys must keep typing text.
            let chord = key.1.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);
            if key.0 == KeyCode::Esc || !(chord || matches!(key.0, KeyCode::F(_))) {
                problem(format!(
                    "{} cannot be bound; use Ctrl, Alt or a function key",
                    key_name(key)
                ));
                continue;
            }
            let action = action.trim().to_lowercase();
            let bound = if action == "none" {
                Bound::Nothing
            } else if let Some(command) = Command::ALL
                .into_iter()
                .find(|command| command.name() == action)
            {
                Bound::Command(command)
            } else if let Ok((code, modifiers)) = parse_key(&action) {
                Bound::Key(code, modifiers)
            } else {
                problem(format!("unknown command {action:?}"));
                continue;
            };
            bindings.keys.retain(|(bound_key, _)| *bound_key != key);
            bindings.keys.push((key, bound));
        }
        (bindings, problems)
    }

    /// What the user bound this key press to, if anything. Terminals without
    /// the enhanced keyboard protocol send Ctrl+/ as Ctrl+7 or Ctrl+_ and
    /// Ctrl+] as Ctrl+5, so those reach the bindings of the keys meant.
    pub fn lookup(&self, key: &KeyEvent) -> Option<Bound> {
        let (code, modifiers) = normalize(key.code, key.modifiers);
        let meant = match code {
            KeyCode::Char('7' | '_') if modifiers == KeyModifiers::CONTROL => KeyCode::Char('/'),
            KeyCode::Char('5') if modifiers == KeyModifiers::CONTROL => KeyCode::Char(']'),
            code => code,
        };
        self.bound((code, modifiers))
            .or_else(|| self.bound((meant, modifiers)))
    }

    fn bound(&self, key: Key) -> Option<Bound> {
        self.keys
            .iter()
            .find(|(bound_key, _)| *bound_key == key)
            .map(|(_, bound)| *bound)
    }

    /// The key to show beside `command`: the first the user bound to it,
    /// else whatever now does what its default key did.
    pub fn shortcut(&self, command: Command, default: &str) -> String {
        if let Some((key, _)) = self
            .keys
            .iter()
            .find(|(_, bound)| *bound == Bound::Command(command))
        {
            return key_name(*key);
        }
        self.key_for(default)
    }

    /// The key that now does what `default` does by default: that key
    /// itself unless the user gave it another use, then the first key
    /// they pointed at it, and nothing when it has no key left.
    pub fn key_for(&self, default: &str) -> String {
        match parse_key(default) {
            Ok(key) if self.bound(key).is_some() => self
                .keys
                .iter()
                .find(|(_, bound)| *bound == Bound::Key(key.0, key.1))
                .map(|(user, _)| key_name(*user))
                .unwrap_or_default(),
            _ => default.to_owned(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    #[test]
    fn keys_parse_with_modifiers_names_and_symbols() {
        let (control, alt, shift) = (
            KeyModifiers::CONTROL,
            KeyModifiers::ALT,
            KeyModifiers::SHIFT,
        );
        for (text, expected) in [
            ("ctrl+j", (KeyCode::Char('j'), control)),
            ("Ctrl+J", (KeyCode::Char('j'), control)),
            (
                " control + shift + s ",
                (KeyCode::Char('s'), control | shift),
            ),
            ("alt+up", (KeyCode::Up, alt)),
            ("option+shift+down", (KeyCode::Down, alt | shift)),
            ("f5", (KeyCode::F(5), KeyModifiers::NONE)),
            ("shift+F12", (KeyCode::F(12), shift)),
            ("ctrl+/", (KeyCode::Char('/'), control)),
            ("alt+`", (KeyCode::Char('`'), alt)),
            ("ctrl++", (KeyCode::Char('+'), control)),
            ("alt+=", (KeyCode::Char('='), alt)),
            ("ctrl+pagedown", (KeyCode::PageDown, control)),
            ("alt+enter", (KeyCode::Enter, alt)),
            ("ctrl+space", (KeyCode::Char(' '), control)),
            // A symbol carries its own Shift.
            ("alt+shift+?", (KeyCode::Char('?'), alt)),
            ("alt+é", (KeyCode::Char('é'), alt)),
        ] {
            assert_eq!(parse_key(text), Ok(expected), "{text:?}");
        }
        for text in ["", "ctrl+", "hyper+j", "ctrl+f13", "ctrl+f0", "ctrl+jk"] {
            assert!(parse_key(text).is_err(), "{text:?}");
        }
        assert_eq!(parse_key("f"), Ok((KeyCode::Char('f'), KeyModifiers::NONE)));
        assert_eq!(parse_key("+"), Ok((KeyCode::Char('+'), KeyModifiers::NONE)));
    }

    #[test]
    fn key_names_read_as_the_palette_writes_them() {
        for (text, name) in [
            ("ctrl+j", "Ctrl+J"),
            ("shift+ctrl+s", "Ctrl+Shift+S"),
            ("alt+up", "Alt+Up"),
            ("f5", "F5"),
            ("ctrl+/", "Ctrl+/"),
            ("alt+shift+pagedown", "Alt+Shift+PageDown"),
            ("ctrl+space", "Ctrl+Space"),
        ] {
            assert_eq!(key_name(parse_key(text).unwrap()), name);
        }
        // Every default shown in the palette parses back to the same name.
        for command in Command::ALL {
            for markdown in [false, true] {
                let default = command.shortcut(markdown);
                if !default.is_empty() {
                    assert_eq!(
                        key_name(parse_key(default).unwrap()),
                        default,
                        "{command:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn lines_bind_commands_keys_or_nothing_and_later_lines_win() {
        let (bindings, problems) = Bindings::parse(
            "# my keys\n\nctrl+j = duplicate\nalt+q=quit\n  F5 = Reload  \nalt+w = ctrl+q\nctrl+k = none\nctrl+j = save\nalt+= = undo\nalt+u = alt+=\n",
        );
        assert_eq!(problems, Vec::<String>::new());
        let (control, alt) = (KeyModifiers::CONTROL, KeyModifiers::ALT);
        for (key, expected) in [
            (
                press(KeyCode::Char('j'), control),
                Some(Bound::Command(Command::Save)),
            ),
            (
                press(KeyCode::Char('q'), alt),
                Some(Bound::Command(Command::Quit)),
            ),
            (
                press(KeyCode::F(5), KeyModifiers::NONE),
                Some(Bound::Command(Command::Reload)),
            ),
            (
                press(KeyCode::Char('w'), alt),
                Some(Bound::Key(KeyCode::Char('q'), control)),
            ),
            (press(KeyCode::Char('k'), control), Some(Bound::Nothing)),
            (
                press(KeyCode::Char('='), alt),
                Some(Bound::Command(Command::Undo)),
            ),
            (
                press(KeyCode::Char('u'), alt),
                Some(Bound::Key(KeyCode::Char('='), alt)),
            ),
            (press(KeyCode::Char('d'), control), None),
            (press(KeyCode::Char('j'), alt), None),
            (press(KeyCode::Char('j'), KeyModifiers::NONE), None),
        ] {
            assert_eq!(bindings.lookup(&key), expected, "{key:?}");
        }
        let (empty, problems) = Bindings::parse("# nothing\n\n");
        assert!(empty.keys.is_empty() && problems.is_empty());
    }

    #[test]
    fn terminals_spellings_of_one_key_reach_the_same_binding() {
        let (bindings, _) = Bindings::parse(
            "ctrl+shift+s = save\nctrl+/ = help\nctrl+] = undo\nalt+? = find\nshift+f3 = redo\nalt+shift+tab = new-tab\n",
        );
        let (control, alt, shift) = (
            KeyModifiers::CONTROL,
            KeyModifiers::ALT,
            KeyModifiers::SHIFT,
        );
        for (key, command) in [
            // The enhanced protocol reports a capital or a small letter.
            (press(KeyCode::Char('S'), control | shift), Command::Save),
            (press(KeyCode::Char('s'), control | shift), Command::Save),
            // Legacy bytes for Ctrl+/ and Ctrl+].
            (press(KeyCode::Char('/'), control), Command::Help),
            (press(KeyCode::Char('7'), control), Command::Help),
            (press(KeyCode::Char('_'), control), Command::Help),
            (press(KeyCode::Char('5'), control), Command::Undo),
            // A symbol with or without a reported Shift.
            (press(KeyCode::Char('?'), alt), Command::Find),
            (press(KeyCode::Char('?'), alt | shift), Command::Find),
            (press(KeyCode::F(3), shift), Command::Redo),
            (press(KeyCode::BackTab, alt | shift), Command::New),
            (press(KeyCode::BackTab, alt), Command::New),
        ] {
            assert_eq!(
                bindings.lookup(&key),
                Some(Bound::Command(command)),
                "{key:?}"
            );
        }
        // Without Shift, Ctrl+S is still the default key.
        assert_eq!(bindings.lookup(&press(KeyCode::Char('s'), control)), None);
        // Lock keys and other modifiers do not make a different key.
        let noisy = KeyEvent::new(KeyCode::F(3), shift | KeyModifiers::SUPER);
        assert_eq!(bindings.lookup(&noisy), Some(Bound::Command(Command::Redo)));
    }

    #[test]
    fn unusable_lines_are_reported_and_the_rest_apply() {
        let (bindings, problems) = Bindings::parse(
            "ctrl+j duplicate\nhyper+j = save\nctrl+j = dance\nj = save\nenter = save\nshift+up = save\nesc = quit\nctrl+esc = quit\nalt+j = save\n",
        );
        assert_eq!(
            problems,
            [
                "line 1: expected KEY = COMMAND",
                "line 2: unknown modifier \"hyper\"",
                "line 3: unknown command \"dance\"",
                "line 4: J cannot be bound; use Ctrl, Alt or a function key",
                "line 5: Enter cannot be bound; use Ctrl, Alt or a function key",
                "line 6: Shift+Up cannot be bound; use Ctrl, Alt or a function key",
                "line 7: Esc cannot be bound; use Ctrl, Alt or a function key",
                "line 8: Ctrl+Esc cannot be bound; use Ctrl, Alt or a function key",
            ]
        );
        assert_eq!(
            bindings.lookup(&press(KeyCode::Char('j'), KeyModifiers::ALT)),
            Some(Bound::Command(Command::Save))
        );
    }

    #[test]
    fn the_palette_shows_the_users_key_and_drops_a_default_put_to_other_use() {
        let (bindings, _) =
            Bindings::parse("ctrl+j = duplicate\nctrl+s = find\nctrl+k = none\nalt+w = ctrl+q\n");
        for (command, default, shown) in [
            (Command::Duplicate, "Ctrl+D", "Ctrl+J"),
            (Command::Find, "Ctrl+F", "Ctrl+S"),
            // Ctrl+S now finds, and Ctrl+K does nothing.
            (Command::Save, "Ctrl+S", ""),
            (Command::CutLine, "Ctrl+K", ""),
            // Untouched defaults stay, including a key another key imitates.
            (Command::Undo, "Ctrl+Z", "Ctrl+Z"),
            (Command::Close, "Ctrl+Q", "Ctrl+Q"),
            (Command::SaveAs, "", ""),
        ] {
            assert_eq!(bindings.shortcut(command, default), shown, "{command:?}");
        }
        let none = Bindings::default();
        assert_eq!(none.shortcut(Command::Save, "Ctrl+S"), "Ctrl+S");
        assert!(none.is_empty() && !bindings.is_empty());
        // A default key put to other use still names the key pointed at it.
        let (swapped, _) =
            Bindings::parse("alt+s = ctrl+s\nctrl+s = find\nalt+p = ctrl+e\nctrl+e = none\n");
        assert_eq!(swapped.shortcut(Command::Save, "Ctrl+S"), "Alt+S");
        assert_eq!(swapped.shortcut(Command::Find, "Ctrl+F"), "Ctrl+S");
        assert_eq!(swapped.key_for("Ctrl+E"), "Alt+P");
        assert_eq!(swapped.key_for("Ctrl+G"), "Ctrl+G");
        let (gone, _) = Bindings::parse("ctrl+e = none\n");
        assert_eq!(gone.key_for("Ctrl+E"), "");
    }

    #[test]
    fn every_command_has_one_name_of_its_own() {
        let mut names: Vec<_> = Command::ALL.into_iter().map(Command::name).collect();
        for name in &names {
            assert!(
                !name.is_empty() && name.chars().all(|c| c.is_ascii_lowercase() || c == '-'),
                "{name:?}"
            );
            assert!(
                *name != "none" && parse_key(name).is_err(),
                "{name:?} reads as a key"
            );
        }
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), Command::ALL.len());
    }

    #[test]
    fn the_key_reference_lists_every_command_name() {
        let reference = include_str!("../../docs/keys.md");
        let section = reference
            .split("## Your own keys")
            .nth(1)
            .and_then(|rest| rest.split("\n## ").next())
            .expect("docs/keys.md has a Your own keys section");
        for command in Command::ALL {
            let row = format!(
                "| `{}` | {} |",
                command.name(),
                command.label().trim_end_matches('…')
            );
            assert!(
                section.contains(&row),
                "docs/keys.md must list this row under Your own keys: {row}"
            );
        }
    }
}
