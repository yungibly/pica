//! Small, versioned preferences. Loading is read-only; explicit choices save atomically.
use crate::{icons::IconSet, theme::Theme};
use pica::editing::Indent;
use std::{
    collections::BTreeMap,
    env, fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

const LIMIT: u64 = 16 * 1024;

pub struct Settings {
    path: PathBuf,
    baseline: Option<Vec<u8>>,
    values: BTreeMap<String, String>,
}

pub fn directories() -> io::Result<(PathBuf, PathBuf)> {
    fn base(variable: &str, fallback: &str) -> io::Result<PathBuf> {
        if let Some(value) = env::var_os(variable).filter(|value| !value.is_empty()) {
            let path = PathBuf::from(value);
            if path.is_absolute() {
                return Ok(path);
            }
            return Err(io::Error::other(format!(
                "{variable} must be an absolute path"
            )));
        }
        env::var_os("HOME")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .map(|path| path.join(fallback))
            .ok_or_else(|| {
                io::Error::other("No home directory; preferences and recovery are unavailable")
            })
    }
    directories_under(
        &base("XDG_CONFIG_HOME", ".config")?,
        &base("XDG_STATE_HOME", ".local/state")?,
    )
}

fn directories_under(config: &Path, state: &Path) -> io::Result<(PathBuf, PathBuf)> {
    Ok((app_directory(config)?, app_directory(state)?))
}

/// The `pica` root, existing or not; nothing is created, migrated, or
/// deleted. Directories of earlier product names are left alone. A path that
/// exists but is not a real directory is an error rather than a silent split.
fn app_directory(base: &Path) -> io::Result<PathBuf> {
    let path = base.join("pica");
    match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.file_type().is_dir() => Ok(path),
        Ok(_) => Err(io::Error::other(format!(
            "Preferences and recovery path must be a real directory: {}",
            path.display()
        ))),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(path),
        Err(error) => Err(error),
    }
}

fn read(path: &Path) -> io::Result<Option<Vec<u8>>> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(value) => value,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if !metadata.file_type().is_file() {
        return Err(io::Error::other("Settings must be a regular file"));
    }
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(path)?;
    if !file.metadata()?.is_file() {
        return Err(io::Error::other("Settings must be a regular file"));
    }
    let mut bytes = Vec::new();
    file.take(LIMIT + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > LIMIT {
        return Err(io::Error::other("Settings exceed the 16 KiB limit"));
    }
    Ok(Some(bytes))
}

impl Settings {
    pub fn load(directory: &Path) -> io::Result<Self> {
        let path = directory.join("settings.conf");
        let baseline = read(&path)?;
        let text = std::str::from_utf8(baseline.as_deref().unwrap_or_default())
            .map_err(|_| io::Error::other("Settings are not UTF-8"))?;
        let mut values = BTreeMap::new();
        for line in text
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
        {
            let (key, value) = line
                .split_once('=')
                .ok_or_else(|| io::Error::other("Invalid settings line; expected key=value"))?;
            let (key, value) = (key.trim(), value.trim());
            if key.is_empty()
                || key.chars().chain(value.chars()).any(char::is_control)
                || values.insert(key.to_owned(), value.to_owned()).is_some()
            {
                return Err(io::Error::other("Invalid or duplicate settings key"));
            }
        }
        if values.get("version").is_some_and(|value| value != "1") {
            return Err(io::Error::other(
                "Unsupported settings version; preferences were left untouched",
            ));
        }
        Ok(Self {
            path,
            baseline,
            values,
        })
    }
    pub fn theme(&self) -> Option<Theme> {
        self.values
            .get("theme")
            .and_then(|value| Theme::from_name(value))
    }
    pub fn unknown_theme(&self) -> Option<&str> {
        self.values
            .get("theme")
            .filter(|_| self.theme().is_none())
            .map(String::as_str)
    }
    pub fn sidebar(&self) -> Option<bool> {
        match self.values.get("sidebar").map(String::as_str) {
            Some("show") => Some(true),
            Some("hide") => Some(false),
            _ => None,
        }
    }
    /// The remembered Markdown view: `Some(false)` opens Markdown as source.
    pub fn markdown_live(&self) -> Option<bool> {
        match self.values.get("markdown-view").map(String::as_str) {
            Some("live") => Some(true),
            Some("source") => Some(false),
            _ => None,
        }
    }
    pub fn save_markdown_live(&mut self, live: bool) -> io::Result<()> {
        self.save("markdown-view", if live { "live" } else { "source" })
    }
    /// Indent guides in code and source views; off unless chosen.
    pub fn guides(&self) -> bool {
        self.values.get("guides").is_some_and(|value| value == "on")
    }
    pub fn save_guides(&mut self, on: bool) -> io::Result<()> {
        self.save("guides", if on { "on" } else { "off" })
    }
    /// The indentation chosen for every file; `None` follows each file.
    pub fn indent(&self) -> Option<Indent> {
        match self.values.get("indent").map(String::as_str)? {
            "tabs" => Some(Indent::Tabs),
            width => width
                .parse()
                .ok()
                .filter(|width| (1..=8).contains(width))
                .map(Indent::Spaces),
        }
    }
    pub fn save_indent(&mut self, indent: Option<Indent>) -> io::Result<()> {
        let value = match indent {
            None => "auto".to_owned(),
            Some(Indent::Tabs) => "tabs".to_owned(),
            Some(Indent::Spaces(width)) => width.to_string(),
        };
        self.save("indent", &value)
    }
    /// Whether Find matches letter case; it ignores case unless chosen.
    pub fn find_match_case(&self) -> bool {
        self.values
            .get("find-case")
            .is_some_and(|value| value == "match")
    }
    pub fn save_find_match_case(&mut self, on: bool) -> io::Result<()> {
        self.save("find-case", if on { "match" } else { "ignore" })
    }
    pub fn icons(&self) -> Option<IconSet> {
        self.values
            .get("icons")
            .and_then(|value| IconSet::from_name(value))
    }
    pub fn save_icons(&mut self, icons: IconSet) -> io::Result<()> {
        self.save("icons", icons.name())
    }
    pub fn save_theme(&mut self, theme: Theme) -> io::Result<()> {
        self.save("theme", theme.id())
    }
    pub fn save_sidebar(&mut self, preference: Option<bool>) -> io::Result<()> {
        self.save(
            "sidebar",
            match preference {
                Some(true) => "show",
                Some(false) => "hide",
                None => "auto",
            },
        )
    }
    fn save(&mut self, key: &str, value: &str) -> io::Result<()> {
        let mut values = self.values.clone();
        values.insert("version".into(), "1".into());
        values.insert(key.into(), value.into());
        let text: String = values
            .iter()
            .map(|(key, value)| format!("{key}={value}\n"))
            .collect();
        if text.len() as u64 > LIMIT {
            return Err(io::Error::other(
                "Preferences would exceed the 16 KiB settings limit",
            ));
        }
        let directory = self.path.parent().expect("settings parent");
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(directory)?;
        // The stable sidecar survives atomic replacement of the preferences inode.
        let _lock = lock(directory)?;
        let permissions = writable_permissions(&self.path)?;
        if read(&self.path)? != self.baseline {
            return Err(io::Error::other(
                "Settings changed in another process; restart before saving preferences",
            ));
        }
        let mut file = tempfile::NamedTempFile::new_in(directory)?;
        if let Some(permissions) = permissions {
            file.as_file().set_permissions(permissions)?;
        }
        file.write_all(text.as_bytes())?;
        file.as_file().sync_all()?;
        if read(&self.path)? != self.baseline {
            return Err(io::Error::other(
                "Settings changed while saving; preferences were left untouched",
            ));
        }
        writable_permissions(&self.path)?;
        if self.baseline.is_none() {
            file.persist_noclobber(&self.path)
                .map_err(|error| error.error)?;
        } else {
            file.persist(&self.path).map_err(|error| error.error)?;
        }
        self.values = values;
        self.baseline = Some(text.into_bytes());
        #[cfg(unix)]
        fs::File::open(directory)?.sync_all()?;
        Ok(())
    }
}

fn lock(directory: &Path) -> io::Result<fs::File> {
    let mut options = fs::OpenOptions::new();
    options.read(true).write(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(directory.join("settings.lock"))?;
    if !file.metadata()?.is_file() {
        return Err(io::Error::other("Settings lock must be a regular file"));
    }
    file.try_lock().map_err(|error| {
        io::Error::other(format!(
            "Preferences are being changed by another process: {error}"
        ))
    })?;
    Ok(file)
}
fn writable_permissions(path: &Path) -> io::Result<Option<fs::Permissions>> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() && !metadata.permissions().readonly() => {
            Ok(Some(metadata.permissions()))
        }
        Ok(_) => Err(io::Error::other(
            "Settings are read-only or are not a regular file",
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn earlier_product_directories_are_ignored_and_left_untouched() {
        let directory = tempfile::tempdir().unwrap();
        let config = directory.path().join("config");
        let state = directory.path().join("state");
        for name in ["eymi", "marklane"] {
            fs::create_dir_all(config.join(name)).unwrap();
            fs::create_dir_all(state.join(name).join("recovery")).unwrap();
            fs::write(
                config.join(name).join("settings.conf"),
                "version=1\ntheme=nord\n",
            )
            .unwrap();
        }
        let (chosen_config, chosen_state) = directories_under(&config, &state).unwrap();
        assert_eq!(
            (chosen_config.clone(), chosen_state),
            (config.join("pica"), state.join("pica"))
        );
        assert_eq!(Settings::load(&chosen_config).unwrap().theme(), None);
        for name in ["eymi", "marklane"] {
            assert_eq!(
                fs::read_to_string(config.join(name).join("settings.conf")).unwrap(),
                "version=1\ntheme=nord\n"
            );
        }
        assert!(!config.join("pica").exists());
    }

    #[test]
    fn a_root_that_is_not_a_real_directory_is_an_error() {
        let directory = tempfile::tempdir().unwrap();
        let base = directory.path();
        fs::write(base.join("pica"), "keep file").unwrap();
        assert!(app_directory(base).is_err());
        assert_eq!(fs::read_to_string(base.join("pica")).unwrap(), "keep file");
        assert!(app_directory(&base.join("pica/child")).is_err());
        fs::remove_file(base.join("pica")).unwrap();
        #[cfg(unix)]
        for dangling in [false, true] {
            let target = base.join("target");
            if !dangling {
                fs::create_dir(&target).unwrap();
            }
            std::os::unix::fs::symlink(&target, base.join("pica")).unwrap();
            assert!(app_directory(base).is_err());
            fs::remove_file(base.join("pica")).unwrap();
            if !dangling {
                assert_eq!(fs::read_dir(&target).unwrap().count(), 0);
                fs::remove_dir(&target).unwrap();
            }
        }
    }

    #[test]
    fn absent_roots_select_pica_without_creating_directories() {
        let directory = tempfile::tempdir().unwrap();
        let config = directory.path().join("config");
        let state = directory.path().join("state");
        assert_eq!(
            directories_under(&config, &state).unwrap(),
            (config.join("pica"), state.join("pica"))
        );
        assert!(!config.exists());
        assert!(!state.exists());
    }

    #[test]
    fn explicit_choices_persist_without_touching_unknown_preferences() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.conf");
        fs::write(&path, "version=1\nfuture=keep\n").unwrap();
        let mut settings = Settings::load(dir.path()).unwrap();
        settings.save_theme(Theme::Light).unwrap();
        settings.save_sidebar(Some(false)).unwrap();
        assert_eq!(settings.markdown_live(), None);
        settings.save_markdown_live(false).unwrap();
        assert!(!settings.guides());
        settings.save_guides(true).unwrap();
        settings.save_icons(IconSet::Nerd).unwrap();
        assert_eq!(settings.indent(), None);
        assert!(!settings.find_match_case());
        settings.save_find_match_case(true).unwrap();
        settings.save_indent(Some(Indent::Spaces(2))).unwrap();
        let loaded = Settings::load(dir.path()).unwrap();
        assert_eq!(loaded.indent(), Some(Indent::Spaces(2)));
        assert!(loaded.find_match_case());
        assert_eq!(loaded.theme(), Some(Theme::Light));
        assert_eq!(loaded.sidebar(), Some(false));
        assert_eq!(loaded.markdown_live(), Some(false));
        assert!(loaded.guides());
        assert_eq!(loaded.icons(), Some(IconSet::Nerd));
        assert!(fs::read_to_string(path).unwrap().contains("future=keep"));
    }
    #[test]
    fn indentation_is_tabs_a_width_or_each_files_own() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.conf");
        for (saved, expected) in [
            ("tabs", Some(Indent::Tabs)),
            ("1", Some(Indent::Spaces(1))),
            ("8", Some(Indent::Spaces(8))),
            ("auto", None),
            ("0", None),
            ("9", None),
            ("wide", None),
        ] {
            fs::write(&path, format!("version=1\nindent={saved}\n")).unwrap();
            assert_eq!(Settings::load(dir.path()).unwrap().indent(), expected);
        }
        let mut settings = Settings::load(dir.path()).unwrap();
        for (indent, saved) in [
            (Some(Indent::Tabs), "indent=tabs\n"),
            (Some(Indent::Spaces(4)), "indent=4\n"),
            (None, "indent=auto\n"),
        ] {
            settings.save_indent(indent).unwrap();
            assert!(fs::read_to_string(&path).unwrap().contains(saved));
            assert_eq!(Settings::load(dir.path()).unwrap().indent(), indent);
        }
    }
    #[test]
    fn read_is_lazy_and_concurrent_changes_are_preserved() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested");
        let mut settings = Settings::load(&path).unwrap();
        assert!(!path.exists());
        settings.save_theme(Theme::Dark).unwrap();
        fs::write(path.join("settings.conf"), "theme=light\n").unwrap();
        assert!(settings.save_theme(Theme::Dark).is_err());
        assert_eq!(
            fs::read_to_string(path.join("settings.conf")).unwrap(),
            "theme=light\n"
        );
    }
    #[test]
    fn oversized_updates_and_concurrent_writers_leave_preferences_intact() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.conf");
        let large = format!("future={}\n", "x".repeat(LIMIT as usize - 8));
        fs::write(&path, &large).unwrap();
        let mut settings = Settings::load(dir.path()).unwrap();
        assert!(settings.save_theme(Theme::Dark).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), large);
        fs::write(&path, "theme=dark\n").unwrap();
        let mut settings = Settings::load(dir.path()).unwrap();
        let _locked = lock(dir.path()).unwrap();
        assert!(settings.save_theme(Theme::Light).is_err());
        assert_eq!(fs::read_to_string(path).unwrap(), "theme=dark\n");
    }
    #[cfg(unix)]
    #[test]
    fn explicitly_readonly_preferences_remain_readonly() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.conf");
        fs::write(&path, "theme=dark\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
        let mut settings = Settings::load(dir.path()).unwrap();
        assert!(settings.save_theme(Theme::Light).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "theme=dark\n");
        assert!(fs::metadata(path).unwrap().permissions().readonly());
    }
    #[test]
    fn invalid_and_future_settings_are_retained() {
        let dir = tempfile::tempdir().unwrap();
        for bytes in [
            b"version=2\n".as_slice(),
            b"version=1\nversion=1\n",
            b"bad",
            b"theme=\xff",
            b"theme=dark\0",
        ] {
            fs::write(dir.path().join("settings.conf"), bytes).unwrap();
            assert!(Settings::load(dir.path()).is_err());
            assert_eq!(fs::read(dir.path().join("settings.conf")).unwrap(), bytes);
        }
    }
    #[cfg(unix)]
    #[test]
    fn symlink_preferences_are_never_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("other");
        fs::write(&target, "theme=light\n").unwrap();
        std::os::unix::fs::symlink(&target, dir.path().join("settings.conf")).unwrap();
        assert!(Settings::load(dir.path()).is_err());
        assert_eq!(fs::read_to_string(target).unwrap(), "theme=light\n");
    }
}
