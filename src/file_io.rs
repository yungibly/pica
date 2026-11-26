//! Source-preserving local file I/O. Atomic replacement is not a cross-process lock.
use std::{
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    time::SystemTime,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExternalChange {
    Unchanged,
    Changed,
    Missing,
    Unreadable(String),
}

/// What tells one version of a file on disk from the next without reading
/// it: size, times, permissions and, on Unix, which file it is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fingerprint {
    len: u64,
    modified: Option<SystemTime>,
    readonly: bool,
    #[cfg(unix)]
    identity: (u64, u64, i64, i64, u32, u64),
}

impl Fingerprint {
    fn of(metadata: &fs::Metadata) -> Self {
        Self {
            len: metadata.len(),
            modified: metadata.modified().ok(),
            readonly: metadata.permissions().readonly(),
            #[cfg(unix)]
            identity: {
                use std::os::unix::fs::MetadataExt;
                (
                    metadata.dev(),
                    metadata.ino(),
                    metadata.ctime(),
                    metadata.ctime_nsec(),
                    metadata.mode(),
                    metadata.nlink(),
                )
            },
        }
    }
}

pub struct FileState {
    /// The path as opened, which names the document. A symbolic link stays a
    /// link: saves replace the file it pointed to when it was opened.
    pub path: PathBuf,
    /// The link's fully resolved target, recorded at open. Every read, check
    /// and replacement uses it; `path` is only verified to still lead here.
    link_target: Option<PathBuf>,
    baseline: Option<Vec<u8>>,
    observed: Option<Fingerprint>,
    external: ExternalChange,
}

impl FileState {
    #[cfg(test)]
    pub fn open(path: PathBuf) -> io::Result<(String, Self)> {
        Self::open_with_limit(path, None)
    }

    /// Open like `open`, accepting at most `max_bytes` of UTF-8 source. The
    /// bounded read probes at most one extra byte, even if the file grows after
    /// metadata inspection. Missing paths retain the new-empty-file behavior.
    pub fn open_bounded(path: PathBuf, max_bytes: usize) -> io::Result<(String, Self)> {
        Self::open_with_limit(path, Some(max_bytes))
    }

    /// A symbolic link is resolved once, here, and its target read. A link
    /// to nothing is refused rather than opened as a new file.
    fn open_with_limit(path: PathBuf, max_bytes: Option<usize>) -> io::Result<(String, Self)> {
        let link_target = resolve_link(&path)?;
        let baseline = match &link_target {
            Some(target) => {
                let baseline =
                    read_optional_with_limit(target, max_bytes)?.ok_or_else(dangling_link_error)?;
                verify_link(&path, target)?;
                Some(baseline)
            }
            None => read_optional_with_limit(&path, max_bytes)?,
        };
        Self::from_baseline(path, link_target, baseline)
    }

    /// Where reads and writes happen: the link's recorded target, or the path.
    fn file_path(&self) -> &Path {
        self.link_target.as_deref().unwrap_or(&self.path)
    }

    fn from_baseline(
        path: PathBuf,
        link_target: Option<PathBuf>,
        baseline: Option<Vec<u8>>,
    ) -> io::Result<(String, Self)> {
        let text = String::from_utf8(baseline.clone().unwrap_or_default()).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "Only valid UTF-8 files are supported; the file was not changed",
            )
        })?;
        if text.contains('\0') {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "This file contains binary NUL bytes; choose a UTF-8 text file",
            ));
        }
        Ok((
            text,
            Self {
                path,
                link_target,
                baseline,
                observed: None,
                external: ExternalChange::Unchanged,
            },
        ))
    }

    /// A file that does not exist yet. Saving creates it and never replaces
    /// anything that appears there in the meantime.
    pub fn new_target(path: PathBuf) -> io::Result<Self> {
        if fs::symlink_metadata(&path).is_ok() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "A file with that name already exists",
            ));
        }
        Ok(Self {
            path,
            link_target: None,
            baseline: None,
            observed: None,
            external: ExternalChange::Unchanged,
        })
    }

    /// An existing file that Save As may replace once the user agrees. What
    /// it holds now, text or not, becomes the baseline: the save replaces
    /// exactly the file that was asked about and refuses one that changed
    /// since. Everything `save` would refuse is refused here, before asking:
    /// folders and other non-regular files, read-only files, files with other
    /// hard links, and files of another user. A symbolic link is followed as
    /// when opening, so its target is what gets replaced.
    pub fn replacing(path: PathBuf, max_bytes: usize) -> io::Result<Self> {
        let link_target = resolve_link(&path)?;
        let target = link_target.as_deref().unwrap_or(&path);
        if !fs::metadata(target)?.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Only regular files can be replaced",
            ));
        }
        let metadata = writable_target_metadata(target)?
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "That file no longer exists"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            // SAFETY: geteuid has no preconditions and cannot fail.
            let user = unsafe { libc::geteuid() };
            if user != 0 && metadata.uid() != user {
                return Err(another_users_file());
            }
        }
        #[cfg(not(unix))]
        let _ = metadata;
        let baseline = open_regular_file(target)
            .and_then(|file| read_limited(file, max_bytes))
            .map_err(|error| {
                if error.kind() == io::ErrorKind::InvalidData {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        "That file is too large for pica to replace safely",
                    )
                } else {
                    error
                }
            })?;
        if let Some(target) = &link_target {
            verify_link(&path, target)?;
        }
        Ok(Self {
            path,
            link_target,
            baseline: Some(baseline),
            observed: None,
            external: ExternalChange::Unchanged,
        })
    }

    /// Where a symbolic link leads, when the document was opened through one.
    pub fn link_target(&self) -> Option<&Path> {
        self.link_target.as_deref()
    }

    /// Advisory periodic check. Unchanged metadata avoids a content read; save
    /// always compares exact bytes regardless of this cache. Read errors are
    /// retried so access can recover without a detectable metadata change.
    pub fn external_change(&mut self) -> ExternalChange {
        if let Some(target) = &self.link_target {
            let status = match verify_link(&self.path, target) {
                Ok(()) => None,
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    Some(ExternalChange::Missing)
                }
                // Pointing elsewhere now: Reload follows the link afresh.
                Err(error) if error.kind() == io::ErrorKind::Other => Some(ExternalChange::Changed),
                Err(error) => Some(ExternalChange::Unreadable(explain(&error))),
            };
            if let Some(status) = status {
                self.observed = None;
                self.external = status;
                return self.external.clone();
            }
        }
        let metadata = match fs::metadata(self.file_path()) {
            Ok(metadata) if metadata.is_file() => metadata,
            Ok(_) => {
                self.observed = None;
                self.external = ExternalChange::Unreadable(non_regular_error().to_string());
                return self.external.clone();
            }
            Err(error) => {
                self.observed = None;
                self.external = if error.kind() == io::ErrorKind::NotFound {
                    if self.baseline.is_none() {
                        ExternalChange::Unchanged
                    } else {
                        ExternalChange::Missing
                    }
                } else {
                    ExternalChange::Unreadable(explain(&error))
                };
                return self.external.clone();
            }
        };
        let fingerprint = Fingerprint::of(&metadata);
        if self.observed.as_ref() == Some(&fingerprint)
            && !matches!(self.external, ExternalChange::Unreadable(_))
        {
            return self.external.clone();
        }
        self.external = match open_regular_file(self.file_path()) {
            Ok(file) => match &self.baseline {
                Some(baseline) => match matches_baseline(file, baseline) {
                    Ok(true) => ExternalChange::Unchanged,
                    Ok(false) => ExternalChange::Changed,
                    Err(error) => ExternalChange::Unreadable(explain(&error)),
                },
                None => ExternalChange::Changed,
            },
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                if self.baseline.is_none() {
                    ExternalChange::Unchanged
                } else {
                    ExternalChange::Missing
                }
            }
            Err(error) => ExternalChange::Unreadable(explain(&error)),
        };
        self.observed = Some(fingerprint);
        self.external.clone()
    }

    /// The version now on disk, to recognize it later without reading it.
    pub fn disk_stamp(&self) -> Option<Fingerprint> {
        fs::metadata(self.file_path())
            .ok()
            .map(|metadata| Fingerprint::of(&metadata))
    }

    /// Whether the file was last written at least `quiet` ago. A file
    /// written this instant may be half written: many programs empty a file
    /// and then fill it.
    pub fn settled(&self, quiet: std::time::Duration) -> bool {
        fs::metadata(self.file_path())
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(|modified| modified.elapsed().ok())
            .is_none_or(|age| age >= quiet)
    }

    /// Read an existing regular file and stage a replacement FileState. A failed
    /// reload never changes the accepted baseline. Detect replacement or writes
    /// during the read and ask the caller to retry rather than accepting a mix.
    /// Reload is an explicit request, so a symbolic link is resolved afresh.
    pub fn reload_bounded(&self, max_bytes: usize) -> io::Result<(String, Self)> {
        let link_target = resolve_link(&self.path)?;
        let source = link_target.as_deref().unwrap_or(&self.path);
        let mut file = open_regular_file(source)?;
        let before = Fingerprint::of(&file.metadata()?);
        let bytes = read_limited(&mut file, max_bytes)?;
        let after = Fingerprint::of(&file.metadata()?);
        let path = fs::metadata(source)?;
        let relinked = match &link_target {
            Some(target) => verify_link(&self.path, target).is_err(),
            None => false,
        };
        if relinked || !path.is_file() || before != after || after != Fingerprint::of(&path) {
            // `Interrupted`: nothing is wrong, and trying again will do.
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "The file changed while it was being read; try Reload again",
            ));
        }
        Self::from_baseline(self.path.clone(), link_target, Some(bytes))
    }

    /// Replace the file atomically with `text`, after checking that it still
    /// holds the accepted baseline. Through a symbolic link, the link must
    /// still lead to the target recorded at open; that target is replaced,
    /// keeping its metadata, and the link is left as it is.
    pub fn save(&mut self, text: &str) -> io::Result<()> {
        self.check_link()?;
        self.check_baseline()?;
        // A no-edit save must preserve both bytes and file metadata.
        if self.baseline.as_deref() == Some(text.as_bytes()) {
            return Ok(());
        }
        let target = self.file_path().to_owned();
        let metadata = writable_target_metadata(&target)?;
        let parent = target
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let mut builder = tempfile::Builder::new();
        // A replacement starts private, then takes the target's permissions.
        // A new file gets the usual mode after the umask, like other editors.
        #[cfg(unix)]
        if metadata.is_none() {
            use std::os::unix::fs::PermissionsExt;
            builder.permissions(fs::Permissions::from_mode(0o666));
        }
        // The error names the temporary file, which means nothing to anyone.
        let mut temporary = builder.tempfile_in(parent).map_err(|error| {
            let kind = error.kind();
            let reason = match kind {
                io::ErrorKind::PermissionDenied | io::ErrorKind::ReadOnlyFilesystem => {
                    return io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "Saving writes a new file into the folder, but the folder is read-only",
                    );
                }
                io::ErrorKind::NotFound => "The folder does not exist",
                kind => sentence(kind).unwrap_or("A file could not be created in the folder"),
            };
            io::Error::new(kind, reason)
        })?;
        temporary.write_all(text.as_bytes())?;
        if let Some(metadata) = metadata {
            #[cfg(unix)]
            keep_owner(temporary.as_file(), &metadata)?;
            #[cfg(target_os = "macos")]
            keep_attributes(&target, temporary.as_file());
            temporary
                .as_file()
                .set_permissions(metadata.permissions())?;
        }
        temporary.as_file().sync_all()?;
        self.check_link()?;
        self.check_baseline()?;
        // Permissions and link type can change while the temporary file is
        // written. Recheck them as well as bytes before replacing the target.
        writable_target_metadata(&target)?;
        if self.baseline.is_none() {
            temporary.persist_noclobber(&target).map_err(|e| e.error)?;
        } else {
            temporary.persist(&target).map_err(|e| e.error)?;
        }
        self.baseline = Some(text.as_bytes().to_vec());
        self.observed = None;
        self.external = ExternalChange::Unchanged;
        Ok(())
    }

    /// A link opened to one file must not now lead to another, even one with
    /// the same bytes: that would overwrite a file nobody opened.
    fn check_link(&self) -> io::Result<()> {
        let Some(target) = &self.link_target else {
            return Ok(());
        };
        verify_link(&self.path, target)
    }

    fn check_baseline(&self) -> io::Result<()> {
        // A file may have grown arbitrarily since opening it. Compare a bounded
        // stream with the accepted baseline instead of allocating the new file.
        let reason = match open_regular_file(self.file_path()) {
            Ok(file) => match &self.baseline {
                Some(baseline) if matches_baseline(file, baseline)? => return Ok(()),
                Some(_) => "The file changed on disk",
                None => "A file with that name has appeared on disk",
            },
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                if self.baseline.is_none() {
                    return Ok(());
                }
                "The file was removed from disk"
            }
            Err(error) => return Err(error),
        };
        Err(io::Error::other(reason))
    }
}

/// The fully resolved target when `path` is a symbolic link, through any
/// chain of links. A link to nothing is an error, not a new file.
fn resolve_link(path: &Path) -> io::Result<Option<PathBuf>> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => match fs::canonicalize(path) {
            Ok(target) => Ok(Some(target)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Err(dangling_link_error()),
            Err(error) => Err(error),
        },
        Ok(_) => Ok(None),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

/// `path` must still be a symbolic link that resolves to `target`. A missing
/// link or target is `NotFound`; a link that now leads elsewhere is `Other`.
fn verify_link(path: &Path, target: &Path) -> io::Result<()> {
    let link = fs::symlink_metadata(path)?;
    let resolved = fs::canonicalize(path).map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            dangling_link_error()
        } else {
            error
        }
    })?;
    if !link.file_type().is_symlink() || resolved != target {
        return Err(io::Error::other(
            "The symbolic link now points to a different file",
        ));
    }
    Ok(())
}

fn dangling_link_error() -> io::Error {
    io::Error::new(
        io::ErrorKind::NotFound,
        "This symbolic link points to a file that does not exist",
    )
}

/// Read piped input, such as standard input, as UTF-8 text of at most
/// `max_bytes` without NUL bytes. It never becomes a file's baseline.
pub fn read_input(reader: impl Read, max_bytes: usize) -> io::Result<String> {
    let bytes = read_limited(reader, max_bytes).map_err(|error| {
        if error.kind() == io::ErrorKind::InvalidData {
            let limit = if max_bytes.is_multiple_of(1024 * 1024) {
                format!("{} MiB", max_bytes / (1024 * 1024))
            } else {
                format!("{max_bytes}-byte")
            };
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Standard input exceeds the {limit} limit"),
            )
        } else {
            error
        }
    })?;
    let text = String::from_utf8(bytes).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "Standard input is not valid UTF-8 text",
        )
    })?;
    if text.contains('\0') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Standard input contains binary NUL bytes; pipe UTF-8 text",
        ));
    }
    Ok(text)
}

fn matches_baseline(mut reader: impl Read, baseline: &[u8]) -> io::Result<bool> {
    let mut buffer = [0; 8192];
    for expected in baseline.chunks(buffer.len()) {
        let actual = &mut buffer[..expected.len()];
        match reader.read_exact(actual) {
            Ok(()) if actual == expected => {}
            Ok(()) => return Ok(false),
            Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(false),
            Err(error) => return Err(error),
        }
    }
    match reader.read_exact(&mut [0]) {
        Ok(()) => Ok(false),
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => Ok(true),
        Err(error) => Err(error),
    }
}

fn read_optional_with_limit(path: &Path, max_bytes: Option<usize>) -> io::Result<Option<Vec<u8>>> {
    match fs::metadata(path) {
        Ok(metadata) if !metadata.is_file() => return Err(non_regular_error()),
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    }
    let result = match max_bytes {
        Some(limit) => open_regular_file(path).and_then(|file| read_limited(file, limit)),
        None => read_regular_file(path),
    };
    match result {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn read_regular_file(path: &Path) -> io::Result<Vec<u8>> {
    let mut file = open_regular_file(path)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn open_regular_file(path: &Path) -> io::Result<fs::File> {
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // A regular path can become a FIFO after the path metadata check.
        // O_NONBLOCK avoids waiting for a writer before we can check its type.
        options.custom_flags(libc::O_NONBLOCK);
    }
    let file = options.open(path)?;
    // Validate the opened handle, not another pathname lookup, before reading.
    if !file.metadata()?.is_file() {
        return Err(non_regular_error());
    }
    Ok(file)
}

fn read_limited(reader: impl Read, max_bytes: usize) -> io::Result<Vec<u8>> {
    // Do not reserve based on metadata or the caller's potentially huge limit.
    // Saturation also makes usize::MAX a valid bound without addition overflow.
    let probe_limit = u64::try_from(max_bytes)
        .unwrap_or(u64::MAX)
        .saturating_add(1);
    let mut bytes = Vec::new();
    reader.take(probe_limit).read_to_end(&mut bytes)?;
    if bytes.len() > max_bytes {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("File exceeds the {max_bytes}-byte open limit; the file was not changed"),
        ));
    }
    Ok(bytes)
}

fn non_regular_error() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        "Only regular files are supported; the file was not changed",
    )
}

fn writable_target_metadata(path: &Path) -> io::Result<Option<fs::Metadata>> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(io::Error::other(
            "The file became a link or a non-regular file since it was opened",
        ));
    }
    let read_only = || io::Error::new(io::ErrorKind::PermissionDenied, "The file is read-only");
    if metadata.permissions().readonly() {
        return Err(read_only());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() > 1 {
            return Err(io::Error::other(
                "This file has other hard links, which saving would break",
            ));
        }
        // Replacing through a writable folder would bypass the file's own
        // owner, ACLs, and flags, which write bits alone do not show.
        may_write(path).map_err(|error| match error.kind() {
            io::ErrorKind::PermissionDenied | io::ErrorKind::ReadOnlyFilesystem => read_only(),
            _ => error,
        })?;
    }
    Ok(Some(metadata))
}

#[cfg(unix)]
fn may_write(path: &Path) -> io::Result<()> {
    use std::os::unix::ffi::OsStrExt;
    let path = std::ffi::CString::new(path.as_os_str().as_bytes())?;
    // SAFETY: `path` is NUL-terminated and outlives the call.
    let result =
        unsafe { libc::faccessat(libc::AT_FDCWD, path.as_ptr(), libc::W_OK, libc::AT_EACCESS) };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// A replacement must not hand someone else's file to this user. Root, as
/// under sudo, keeps the owner; anyone keeps a group they belong to.
#[cfg(unix)]
fn keep_owner(replacement: &fs::File, target: &fs::Metadata) -> io::Result<()> {
    use std::os::unix::fs::{MetadataExt, fchown};
    let created = replacement.metadata()?;
    if (created.uid(), created.gid()) == (target.uid(), target.gid()) {
        return Ok(());
    }
    match fchown(replacement, Some(target.uid()), Some(target.gid())) {
        Ok(()) => Ok(()),
        Err(_) if created.uid() != target.uid() => Err(another_users_file()),
        // Outside the file's group, the copy keeps this user's default group.
        Err(_) => Ok(()),
    }
}

#[cfg(unix)]
fn another_users_file() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "This file belongs to another user, and saving would make it yours",
    )
}

/// A plain sentence for the common ways the system refuses file work.
fn sentence(kind: io::ErrorKind) -> Option<&'static str> {
    Some(match kind {
        io::ErrorKind::NotFound => "No such file or folder",
        io::ErrorKind::PermissionDenied => "You do not have permission for that",
        io::ErrorKind::ReadOnlyFilesystem => "The disk is read-only",
        io::ErrorKind::StorageFull => "The disk is full",
        io::ErrorKind::QuotaExceeded => "Your disk quota is used up",
        io::ErrorKind::FileTooLarge => "The file is too large for this disk",
        io::ErrorKind::IsADirectory => "That is a folder, not a file",
        io::ErrorKind::NotADirectory => "Part of that path is a file, not a folder",
        io::ErrorKind::InvalidFilename => "That name is too long or not allowed here",
        io::ErrorKind::ResourceBusy | io::ErrorKind::ExecutableFileBusy => "The file is in use",
        io::ErrorKind::TimedOut => "The disk did not answer in time",
        _ => return None,
    })
}

/// What went wrong, for the status line: pica's own sentences as they are,
/// and the system's without its error number.
pub fn explain(error: &io::Error) -> String {
    if error.raw_os_error().is_none() {
        return error.to_string();
    }
    if let Some(sentence) = sentence(error.kind()) {
        return sentence.to_owned();
    }
    let text = error.to_string();
    match text.rfind(" (os error ") {
        Some(at) => text[..at].to_owned(),
        None => text,
    }
}

/// Finder tags, ACLs, and other extended attributes belong to the file, so
/// a replacement copies them when it can. Failing to copy never blocks a save.
#[cfg(target_os = "macos")]
fn keep_attributes(target: &Path, replacement: &fs::File) {
    use std::os::fd::AsRawFd;
    let Ok(original) = open_regular_file(target) else {
        return;
    };
    // SAFETY: both descriptors stay open for the call, which accepts a null state.
    unsafe {
        libc::fcopyfile(
            original.as_raw_fd(),
            replacement.as_raw_fd(),
            std::ptr::null_mut(),
            libc::COPYFILE_ACL | libc::COPYFILE_XATTR,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn external_status_tracks_writes_removal_reappearance_and_nonregular_paths() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("disk.md");
        fs::write(&path, "base").unwrap();
        let (_, mut state) = FileState::open(path.clone()).unwrap();
        assert_eq!(state.external_change(), ExternalChange::Unchanged);
        assert!(state.observed.is_some());
        fs::write(&path, "external text").unwrap();
        assert_eq!(state.external_change(), ExternalChange::Changed);
        assert_eq!(state.external_change(), ExternalChange::Changed);
        assert!(state.save("local").is_err());
        fs::remove_file(&path).unwrap();
        assert_eq!(state.external_change(), ExternalChange::Missing);
        assert!(state.reload_bounded(1024).is_err());
        fs::create_dir(&path).unwrap();
        assert!(matches!(
            state.external_change(),
            ExternalChange::Unreadable(_)
        ));
        fs::remove_dir(&path).unwrap();
        fs::write(&path, "base").unwrap();
        assert_eq!(state.external_change(), ExternalChange::Unchanged);
        state.save("local").unwrap();
        assert_eq!(state.external_change(), ExternalChange::Unchanged);
        assert_eq!(state.baseline.as_deref(), Some(b"local".as_slice()));
    }

    #[test]
    fn metadata_only_changes_and_noop_save_preserve_file_identity() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("disk.md");
        fs::write(&path, "base").unwrap();
        let (_, mut state) = FileState::open(path.clone()).unwrap();
        state.external_change();
        let writable = fs::metadata(&path).unwrap().permissions();
        let mut readonly = writable.clone();
        readonly.set_readonly(true);
        fs::set_permissions(&path, readonly).unwrap();
        let before = Fingerprint::of(&fs::metadata(&path).unwrap());
        let status = state.external_change();
        let saved = state.save("base");
        let after = Fingerprint::of(&fs::metadata(&path).unwrap());
        fs::set_permissions(&path, writable).unwrap();
        assert_eq!(status, ExternalChange::Unchanged);
        saved.unwrap();
        assert_eq!(before, after);
        // Even if the advisory fingerprint were fooled, save still checks bytes.
        fs::write(&path, "evil").unwrap();
        state.observed = Some(Fingerprint::of(&fs::metadata(&path).unwrap()));
        state.external = ExternalChange::Unchanged;
        assert_eq!(state.external_change(), ExternalChange::Unchanged);
        assert!(state.save("base").is_err());
        assert_eq!(fs::read_to_string(path).unwrap(), "evil");
    }

    #[test]
    fn reload_requires_existing_bounded_text_and_leaves_old_state_on_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("disk.md");
        fs::write(&path, "base").unwrap();
        let (_, mut state) = FileState::open(path.clone()).unwrap();
        for bytes in [vec![0xff], b"null\0byte".to_vec(), b"oversized".to_vec()] {
            fs::write(&path, bytes).unwrap();
            assert!(state.reload_bounded(8).is_err());
            assert_eq!(state.baseline.as_deref(), Some(b"base".as_slice()));
        }
        fs::remove_file(&path).unwrap();
        assert!(state.reload_bounded(8).is_err());
        assert!(!path.exists());
        fs::write(&path, "é\r\n").unwrap();
        let (text, mut fresh) = state.reload_bounded(8).unwrap();
        assert_eq!(text, "é\r\n");
        fresh.save(&text).unwrap();
        assert!(state.save("base").is_err());
        fresh.save("new").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "new");
    }

    #[test]
    fn missing_new_target_is_unchanged_until_created_externally() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("new.md");
        let mut state = FileState::new_target(path.clone()).unwrap();
        assert_eq!(state.external_change(), ExternalChange::Unchanged);
        assert!(state.reload_bounded(10).is_err());
        fs::write(path, "").unwrap();
        assert_eq!(state.external_change(), ExternalChange::Changed);
    }

    #[cfg(unix)]
    #[test]
    fn external_unreadability_recovers_and_reload_preserves_link_rules() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("disk.md");
        fs::write(&path, "base").unwrap();
        let (_, mut state) = FileState::open(path.clone()).unwrap();
        state.external_change();
        let writable = fs::metadata(&path).unwrap().permissions();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();
        let status = state.external_change();
        let reload = state.reload_bounded(20);
        fs::set_permissions(&path, writable).unwrap();
        if unsafe { libc::geteuid() } != 0 {
            assert!(matches!(status, ExternalChange::Unreadable(_)));
            assert!(reload.is_err());
        }
        assert_eq!(state.external_change(), ExternalChange::Unchanged);
        let link = dir.path().join("link.md");
        symlink(&path, &link).unwrap();
        let (_, linked) = FileState::open(link).unwrap();
        fs::write(&path, "fresh").unwrap();
        let (text, mut linked) = linked.reload_bounded(20).unwrap();
        assert_eq!(text, "fresh");
        linked.save(&text).unwrap();
        // Reloading through a link keeps saving through it.
        linked.save("changes").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "changes");
        let hard = dir.path().join("hard.md");
        fs::hard_link(&path, &hard).unwrap();
        let (_, hard) = FileState::open(hard).unwrap();
        let (text, mut hard) = hard.reload_bounded(20).unwrap();
        hard.save(&text).unwrap();
        assert!(hard.save("more").is_err());
        // The target now has two names, so the link refuses to break them.
        assert!(linked.save("more").is_err());
        assert_eq!(fs::read_to_string(path).unwrap(), "changes");
    }

    #[test]
    fn baseline_comparison_is_exact_and_reads_at_most_one_extra_byte() {
        use std::{cell::Cell, io::Cursor};
        struct Counted<'a> {
            read: &'a Cell<usize>,
            reader: Cursor<Vec<u8>>,
        }
        impl Read for Counted<'_> {
            fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
                let count = self.reader.read(buffer)?;
                self.read.set(self.read.get() + count);
                Ok(count)
            }
        }
        let baseline = vec![b'x'; 9000];
        let read = Cell::new(0);
        assert!(
            !matches_baseline(
                Counted {
                    read: &read,
                    reader: Cursor::new(vec![b'x'; 100_000])
                },
                &baseline,
            )
            .unwrap()
        );
        assert_eq!(read.get(), baseline.len() + 1);
        assert!(matches_baseline(Cursor::new(&baseline), &baseline).unwrap());
        assert!(!matches_baseline(Cursor::new(&baseline[..8999]), &baseline).unwrap());
        assert!(!matches_baseline(Cursor::new(b"different"), b"original!").unwrap());
        assert!(matches_baseline(io::empty(), b"").unwrap());
        assert!(!matches_baseline(Cursor::new(b"x"), b"").unwrap());
    }

    #[test]
    fn piped_input_is_bounded_utf8_text_without_nul() {
        use std::io::Cursor;
        assert_eq!(
            read_input(Cursor::new("é\r\nlog\n"), 8).unwrap(),
            "é\r\nlog\n"
        );
        assert_eq!(read_input(io::empty(), 0).unwrap(), "");
        let error = read_input(Cursor::new(vec![b'x'; 9]), 8).unwrap_err();
        assert!(error.to_string().contains("8-byte limit"), "{error}");
        let error = read_input(
            Cursor::new(vec![b'x'; 2 * 1024 * 1024 + 1]),
            2 * 1024 * 1024,
        )
        .unwrap_err();
        assert!(error.to_string().contains("2 MiB limit"), "{error}");
        let error = read_input(Cursor::new(vec![0xff, b'a']), 8).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("UTF-8"), "{error}");
        let error = read_input(Cursor::new("a\0b"), 8).unwrap_err();
        assert!(error.to_string().contains("NUL"), "{error}");
    }

    #[test]
    fn bytes_and_noop_saves_are_exact() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("note.md");
        let bytes = b"\xef\xbb\xbf# Heading\r\n\r\nbody  \nlast\t";
        fs::write(&path, bytes).unwrap();
        let (text, mut file) = FileState::open(path.clone()).unwrap();
        file.save(&text).unwrap();
        assert_eq!(fs::read(path).unwrap(), bytes);
    }

    #[test]
    fn bounded_open_accepts_exact_byte_limit_and_preserves_utf8_baseline() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("note.md");
        let source = "\u{feff}# e\u{301}界\r\nbody  \nlast\t";
        fs::write(&path, source).unwrap();
        let (text, mut file) = FileState::open_bounded(path.clone(), source.len()).unwrap();
        assert_eq!(text.as_bytes(), source.as_bytes());
        assert_eq!(file.baseline.as_deref(), Some(source.as_bytes()));
        file.save(&text).unwrap();
        assert_eq!(fs::read(&path).unwrap(), source.as_bytes());
        let (text, _) = FileState::open_bounded(path, usize::MAX).unwrap();
        assert_eq!(text, source);
    }

    #[test]
    fn bounded_open_rejects_oversized_bytes_without_changing_unbounded_open() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("note.md");
        fs::write(&path, "12345").unwrap();
        let error = FileState::open_bounded(path.clone(), 4).err().unwrap();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("4-byte open limit"));
        assert_eq!(fs::read(&path).unwrap(), b"12345");
        assert_eq!(FileState::open(path).unwrap().0, "12345");
    }

    #[test]
    fn bounded_open_zero_limit_accepts_only_empty_or_missing_files() {
        let dir = tempfile::tempdir().unwrap();
        let empty = dir.path().join("empty.md");
        fs::write(&empty, []).unwrap();
        let (text, file) = FileState::open_bounded(empty, 0).unwrap();
        assert!(text.is_empty());
        assert_eq!(file.baseline, Some(Vec::new()));
        let missing = dir.path().join("missing.md");
        let (text, file) = FileState::open_bounded(missing.clone(), 0).unwrap();
        assert!(text.is_empty());
        assert_eq!(file.baseline, None);
        assert!(!missing.exists());
        let nonempty = dir.path().join("nonempty.md");
        fs::write(&nonempty, "x").unwrap();
        assert!(FileState::open_bounded(nonempty, 0).is_err());
    }

    #[test]
    fn bounded_open_validates_utf8_after_enforcing_the_byte_limit() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("text.md");
        fs::write(&path, "é").unwrap();
        let error = FileState::open_bounded(path.clone(), 1).err().unwrap();
        assert!(error.to_string().contains("open limit"));
        assert_eq!(FileState::open_bounded(path.clone(), 2).unwrap().0, "é");
        fs::write(&path, [0xff]).unwrap();
        let error = FileState::open_bounded(path.clone(), 1).err().unwrap();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("UTF-8"));
        assert_eq!(fs::read(&path).unwrap(), [0xff]);
    }

    #[test]
    fn bounded_read_stops_after_limit_plus_one_when_file_grows_after_open() {
        use std::io::Seek;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("growing.md");
        fs::write(&path, "base").unwrap();
        let mut file = open_regular_file(&path).unwrap();
        assert_eq!(file.metadata().unwrap().len(), 4);
        fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(&[b'x'; 32_768])
            .unwrap();
        let error = read_limited(&mut file, 4).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(file.stream_position().unwrap(), 5);
        assert_eq!(fs::metadata(path).unwrap().len(), 32_772);
    }

    #[test]
    fn bounded_open_rejects_directories() {
        let dir = tempfile::tempdir().unwrap();
        let error = FileState::open_bounded(dir.path().to_owned(), usize::MAX)
            .err()
            .unwrap();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    }

    #[cfg(unix)]
    #[test]
    fn bounded_open_preserves_link_and_nonblocking_fifo_behavior() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target.md");
        let link = dir.path().join("link.md");
        let hardlink = dir.path().join("hardlink.md");
        fs::write(&target, "source").unwrap();
        symlink(&target, &link).unwrap();
        fs::hard_link(&target, &hardlink).unwrap();
        for path in [link, hardlink] {
            let (text, mut file) = FileState::open_bounded(path, 6).unwrap();
            assert_eq!(text, "source");
            file.save(&text).unwrap();
            assert!(file.save("changed").is_err());
        }
        assert_eq!(fs::read(&target).unwrap(), b"source");
        let fifo = dir.path().join("pipe");
        make_fifo(&fifo);
        let fifo_link = dir.path().join("pipe-link");
        symlink(&fifo, &fifo_link).unwrap();
        for path in [fifo.clone(), fifo_link] {
            assert_rejected_promptly(move || {
                FileState::open_bounded(path, 8 * 1024 * 1024).map(|_| ())
            });
        }
        // Exercise replacement after pathname preflight: validating the opened
        // handle must still reject the FIFO without waiting for a writer.
        assert_rejected_promptly(move || {
            open_regular_file(&fifo)
                .and_then(|file| read_limited(file, 8))
                .map(|_| ())
        });
    }

    #[test]
    fn external_change_or_deletion_is_never_silently_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("note.md");
        fs::write(&path, "base").unwrap();
        let (_, mut file) = FileState::open(path.clone()).unwrap();
        fs::write(&path, "external").unwrap();
        assert!(file.save("local").is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "external");
        fs::remove_file(&path).unwrap();
        assert!(file.save("local").is_err());
        assert!(!path.exists());
    }

    #[test]
    fn new_targets_refuse_overwrite_and_invalid_utf8_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("note.md");
        let mut file = FileState::new_target(path.clone()).unwrap();
        fs::write(&path, [0xff]).unwrap();
        assert!(FileState::open(path.clone()).is_err());
        assert!(file.save("new").is_err());
        assert_eq!(fs::read(&path).unwrap(), [0xff]);
    }

    #[test]
    fn replacing_holds_what_is_there_and_refuses_a_file_that_changed_since() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("other.bin");
        // Whatever is there, text or not, is what the user is asked about.
        let bytes = [0xff, 0x00, b'x'];
        fs::write(&path, bytes).unwrap();
        let mut staged = FileState::replacing(path.clone(), 1024).unwrap();
        assert_eq!(staged.baseline.as_deref(), Some(bytes.as_slice()));
        assert!(staged.link_target().is_none());
        let error = FileState::replacing(path.clone(), 2).err().unwrap();
        assert!(error.to_string().contains("too large"), "{error}");
        fs::write(&path, "newer").unwrap();
        let error = staged.save("mine").unwrap_err();
        assert_eq!(error.to_string(), "The file changed on disk");
        assert_eq!(fs::read_to_string(&path).unwrap(), "newer");
        let mut staged = FileState::replacing(path.clone(), 1024).unwrap();
        staged.save("mine").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "mine");
        assert_eq!(staged.external_change(), ExternalChange::Unchanged);
        // Removed after the question: nothing is created in its place.
        let mut staged = FileState::replacing(path.clone(), 1024).unwrap();
        fs::remove_file(&path).unwrap();
        let error = staged.save("again").unwrap_err();
        assert_eq!(error.to_string(), "The file was removed from disk");
        assert!(!path.exists());
        // Only an existing regular file can be staged.
        let error = FileState::replacing(path, 1024).err().unwrap();
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
        assert!(FileState::replacing(dir.path().to_owned(), 1024).is_err());
        // A new target that appears before the save is never overwritten.
        let fresh = dir.path().join("fresh.md");
        let mut file = FileState::new_target(fresh.clone()).unwrap();
        fs::write(&fresh, "someone else").unwrap();
        let error = file.save("mine").unwrap_err();
        assert_eq!(
            error.to_string(),
            "A file with that name has appeared on disk"
        );
        assert_eq!(fs::read_to_string(fresh).unwrap(), "someone else");
    }

    #[cfg(unix)]
    #[test]
    fn replacing_refuses_up_front_what_a_save_refuses() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("note.md");
        fs::write(&path, "text").unwrap();
        let second = dir.path().join("second-name.md");
        fs::hard_link(&path, &second).unwrap();
        let error = FileState::replacing(path.clone(), 1024).err().unwrap();
        assert!(error.to_string().contains("hard links"), "{error}");
        fs::remove_file(&second).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
        let refused = FileState::replacing(path.clone(), 1024);
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        if unsafe { libc::geteuid() } != 0 {
            assert_eq!(
                refused.err().unwrap().kind(),
                io::ErrorKind::PermissionDenied
            );
        }
        // Through a link, the file it leads to is staged and later replaced.
        let link = dir.path().join("link.md");
        symlink(&path, &link).unwrap();
        let mut staged = FileState::replacing(link.clone(), 1024).unwrap();
        assert_eq!(
            staged.link_target(),
            Some(fs::canonicalize(&path).unwrap().as_path())
        );
        staged.save("through the link").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "through the link");
        assert!(is_link(&link));
        let dangling = dir.path().join("dangling.md");
        symlink(dir.path().join("nowhere.md"), &dangling).unwrap();
        let error = FileState::replacing(dangling, 1024).err().unwrap();
        assert!(error.to_string().contains("does not exist"), "{error}");
        assert!(!dir.path().join("nowhere.md").exists());
    }

    #[cfg(unix)]
    #[test]
    fn system_errors_become_sentences_without_numbers_or_temporary_names() {
        for (code, sentence) in [
            (libc::ENOENT, "No such file or folder"),
            (libc::EACCES, "You do not have permission for that"),
            (libc::ENOSPC, "The disk is full"),
            (libc::EROFS, "The disk is read-only"),
            (libc::EISDIR, "That is a folder, not a file"),
            (libc::ENOTDIR, "Part of that path is a file, not a folder"),
            (
                libc::ENAMETOOLONG,
                "That name is too long or not allowed here",
            ),
        ] {
            assert_eq!(explain(&io::Error::from_raw_os_error(code)), sentence);
        }
        // An unlisted error keeps the system's words and loses its number.
        let other = explain(&io::Error::from_raw_os_error(libc::EMLINK));
        assert!(!other.is_empty() && !other.contains("os error"), "{other}");
        // pica's own sentences pass through untouched.
        let own = io::Error::new(io::ErrorKind::PermissionDenied, "The file is read-only");
        assert_eq!(explain(&own), "The file is read-only");
        // Saving into a folder that is gone never names the temporary file.
        let dir = tempfile::tempdir().unwrap();
        let mut file = FileState::new_target(dir.path().join("gone/notes.md")).unwrap();
        let error = file.save("text").unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
        assert_eq!(explain(&error), "The folder does not exist");
        assert_eq!(error.to_string(), "The folder does not exist");
    }

    #[test]
    fn atomic_save_preserves_permissions() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("note.md");
        fs::write(&path, "old").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
        }
        let (_, mut file) = FileState::open(path.clone()).unwrap();
        file.save("new\r\n").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"new\r\n");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o640
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn saving_needs_write_access_rather_than_any_write_bit() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("note.md");
        fs::write(&path, "base").unwrap();
        let (_, mut file) = FileState::open(path.clone()).unwrap();
        // Its group may write, but not its owner, this user. The folder
        // alone would still allow replacing it.
        fs::set_permissions(&path, fs::Permissions::from_mode(0o464)).unwrap();
        let saved = file.save("changes");
        let bytes = fs::read(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        if unsafe { libc::geteuid() } != 0 {
            assert_eq!(saved.unwrap_err().kind(), io::ErrorKind::PermissionDenied);
            assert_eq!(bytes, b"base");
            file.save("changes").unwrap();
        }
        assert_eq!(fs::read(&path).unwrap(), b"changes");
    }

    #[cfg(unix)]
    #[test]
    fn replacements_keep_their_group_and_new_files_follow_the_umask() {
        use std::os::unix::fs::{MetadataExt, chown};
        let dir = tempfile::tempdir().unwrap();
        let created = dir.path().join("created.md");
        fs::File::create(&created).unwrap();
        let path = dir.path().join("note.md");
        FileState::new_target(path.clone())
            .unwrap()
            .save("new")
            .unwrap();
        let mode = |path: &Path| fs::metadata(path).unwrap().mode() & 0o7777;
        assert_eq!(mode(&path), mode(&created));
        // Another group this user belongs to, if any.
        let default = fs::metadata(&path).unwrap().gid();
        let mut groups = vec![0; 256];
        // SAFETY: the buffer holds as many groups as the call is told.
        let count = unsafe { libc::getgroups(groups.len() as i32, groups.as_mut_ptr()) };
        groups.truncate(count.max(0) as usize);
        let Some(group) = groups.into_iter().find(|group| *group != default) else {
            return;
        };
        chown(&path, None, Some(group)).unwrap();
        let (_, mut file) = FileState::open(path.clone()).unwrap();
        file.save("changed").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"changed");
        assert_eq!(fs::metadata(&path).unwrap().gid(), group);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn replacements_keep_extended_attributes() {
        use std::process::Command;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("note.md");
        fs::write(&path, "base").unwrap();
        let xattr = |args: &[&str]| {
            let output = Command::new("xattr")
                .args(args)
                .arg(&path)
                .output()
                .unwrap();
            assert!(output.status.success(), "{output:?}");
            String::from_utf8(output.stdout).unwrap()
        };
        xattr(&["-w", "org.pica.test", "kept"]);
        let (_, mut file) = FileState::open(path.clone()).unwrap();
        file.save("changed").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"changed");
        assert_eq!(xattr(&["-p", "org.pica.test"]).trim(), "kept");
    }

    #[test]
    fn read_only_targets_allow_noop_but_reject_edits_without_advancing_baseline() {
        // Cover files already read-only at open and chmod after opening.
        for readonly_at_open in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("note.md");
            let original = b"\xef\xbb\xbfbase\r\nlast  ";
            fs::write(&path, original).unwrap();
            let writable = fs::metadata(&path).unwrap().permissions();
            let mut readonly = writable.clone();
            readonly.set_readonly(true);
            if readonly_at_open {
                fs::set_permissions(&path, readonly.clone()).unwrap();
            }
            let (text, mut file) = FileState::open(path.clone()).unwrap();
            fs::set_permissions(&path, readonly).unwrap();
            let noop = file.save(&text);
            let changed = file.save("local changes");
            let disk_bytes = fs::read(&path).unwrap();
            let remains_readonly = fs::metadata(&path).unwrap().permissions().readonly();
            // Restore before assertions so Windows can clean up the temp file.
            fs::set_permissions(&path, writable).unwrap();

            assert!(noop.is_ok());
            assert_eq!(changed.unwrap_err().kind(), io::ErrorKind::PermissionDenied);
            assert_eq!(disk_bytes, original);
            assert!(remains_readonly);
            assert_eq!(file.baseline.as_deref(), Some(original.as_slice()));
            // A rejected save leaves the accepted disk baseline usable.
            file.save("local changes").unwrap();
            assert_eq!(fs::read(path).unwrap(), b"local changes");
        }
    }

    #[test]
    fn directory_targets_are_rejected_before_reading() {
        let dir = tempfile::tempdir().unwrap();
        let error = FileState::open(dir.path().to_owned()).err().unwrap();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    }

    #[cfg(unix)]
    fn make_fifo(path: &Path) {
        assert!(
            std::process::Command::new("mkfifo")
                .arg(path)
                .status()
                .unwrap()
                .success()
        );
    }

    #[cfg(unix)]
    fn assert_rejected_promptly(action: impl FnOnce() -> io::Result<()> + Send + 'static) {
        let (sender, receiver) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            sender.send(action()).unwrap();
        });
        let error = receiver
            .recv_timeout(std::time::Duration::from_secs(2))
            .expect("file operation blocked on a non-regular target")
            .unwrap_err();
        worker.join().unwrap();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    }

    #[cfg(unix)]
    #[test]
    fn fifo_open_and_stale_disk_save_are_rejected_without_waiting_for_writer() {
        use std::os::unix::fs::FileTypeExt;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("note.md");
        fs::write(&path, "baseline").unwrap();
        let (_, mut file) = FileState::open(path.clone()).unwrap();
        fs::remove_file(&path).unwrap();
        make_fifo(&path);

        let open_path = path.clone();
        assert_rejected_promptly(move || FileState::open(open_path).map(|_| ()));
        assert_rejected_promptly(move || {
            let result = file.save("local changes");
            assert_eq!(file.baseline.as_deref(), Some(b"baseline".as_slice()));
            result
        });
        assert!(fs::symlink_metadata(&path).unwrap().file_type().is_fifo());
    }

    #[cfg(unix)]
    #[test]
    fn opened_handle_check_rejects_fifo_even_without_path_preflight() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pipe");
        make_fifo(&path);
        // Exercise the path used if a regular file becomes a FIFO between the
        // metadata preflight and open. No writer exists to release a blocking open.
        assert_rejected_promptly(move || read_regular_file(&path).map(|_| ()));
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_save_their_target_but_hard_links_are_still_refused() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target.md");
        let link = dir.path().join("link.md");
        let hardlink = dir.path().join("hardlink.md");
        fs::write(&target, "original").unwrap();
        symlink(&target, &link).unwrap();
        let (text, mut linked) = FileState::open(link.clone()).unwrap();
        linked.save(&text).unwrap();
        linked.save("edits").unwrap();
        assert!(is_link(&link));
        assert_eq!(fs::read(&target).unwrap(), b"edits");
        fs::hard_link(&target, &hardlink).unwrap();
        let (text, mut hardlinked) = FileState::open(hardlink.clone()).unwrap();
        hardlinked.save(&text).unwrap();
        assert!(hardlinked.save("more").is_err());
        assert!(linked.save("more").is_err());
        assert_eq!(fs::read(&target).unwrap(), b"edits");
        assert_eq!(fs::read(&hardlink).unwrap(), b"edits");

        let fifo = dir.path().join("fifo");
        let fifo_link = dir.path().join("fifo-link");
        make_fifo(&fifo);
        symlink(&fifo, &fifo_link).unwrap();
        assert_rejected_promptly(move || FileState::open(fifo_link).map(|_| ()));
    }

    #[cfg(unix)]
    fn is_link(path: &Path) -> bool {
        fs::symlink_metadata(path).unwrap().file_type().is_symlink()
    }

    #[cfg(unix)]
    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<_> = fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    /// A dotfile manager's layout: `home/.zshrc -> ../dotfiles/zshrc`.
    #[cfg(unix)]
    #[test]
    fn saving_through_a_relative_link_replaces_its_target_and_keeps_metadata() {
        use std::os::unix::fs::{PermissionsExt, symlink};

        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let dotfiles = dir.path().join("dotfiles");
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(&dotfiles).unwrap();
        let target = dotfiles.join("zshrc");
        fs::write(&target, "export A=1\n").unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o640)).unwrap();
        let link = home.join(".zshrc");
        symlink("../dotfiles/zshrc", &link).unwrap();
        let (text, mut file) = FileState::open_bounded(link.clone(), 1024).unwrap();
        assert_eq!(text, "export A=1\n");
        assert_eq!(file.path, link);
        file.save("export A=2\n").unwrap();
        assert_eq!(fs::read_to_string(&target).unwrap(), "export A=2\n");
        assert!(is_link(&link));
        assert_eq!(
            fs::read_link(&link).unwrap(),
            Path::new("../dotfiles/zshrc")
        );
        assert_eq!(
            fs::metadata(&target).unwrap().permissions().mode() & 0o777,
            0o640
        );
        // The replacement was written beside the target; nothing is left over.
        assert_eq!(names(&home), [".zshrc"]);
        assert_eq!(names(&dotfiles), ["zshrc"]);
        assert_eq!(file.external_change(), ExternalChange::Unchanged);
        fs::write(&target, "edited elsewhere\n").unwrap();
        assert_eq!(file.external_change(), ExternalChange::Changed);
        assert!(file.save("mine\n").is_err());
        assert_eq!(fs::read_to_string(&target).unwrap(), "edited elsewhere\n");
        // A chain of links resolves to the final file.
        let second = home.join("chain");
        symlink(&link, &second).unwrap();
        let (_, mut chained) = FileState::open(second.clone()).unwrap();
        chained.save("through two links\n").unwrap();
        assert_eq!(fs::read_to_string(&target).unwrap(), "through two links\n");
        assert!(is_link(&second) && is_link(&link));
    }

    #[cfg(unix)]
    #[test]
    fn a_retargeted_link_is_never_saved_even_to_identical_bytes() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("first.conf");
        let second = dir.path().join("second.conf");
        // Identical contents: a save-time canonicalization would accept both.
        fs::write(&first, "same\n").unwrap();
        fs::write(&second, "same\n").unwrap();
        let link = dir.path().join("current.conf");
        symlink(&first, &link).unwrap();
        let (_, mut file) = FileState::open(link.clone()).unwrap();
        fs::remove_file(&link).unwrap();
        symlink(&second, &link).unwrap();
        assert_eq!(file.external_change(), ExternalChange::Changed);
        let error = file.save("edited\n").unwrap_err();
        assert!(error.to_string().contains("different file"), "{error}");
        assert!(file.save("same\n").is_err());
        assert_eq!(fs::read_to_string(&first).unwrap(), "same\n");
        assert_eq!(fs::read_to_string(&second).unwrap(), "same\n");
        // Replacing the link with a regular file is a change of target too.
        fs::remove_file(&link).unwrap();
        fs::write(&link, "same\n").unwrap();
        assert!(file.save("edited\n").is_err());
        assert_eq!(fs::read_to_string(&link).unwrap(), "same\n");
        // Pointing it back makes saving safe again.
        fs::remove_file(&link).unwrap();
        symlink(&first, &link).unwrap();
        file.save("edited\n").unwrap();
        assert_eq!(fs::read_to_string(&first).unwrap(), "edited\n");
        // Reload follows the link to its new target and saves there.
        fs::remove_file(&link).unwrap();
        symlink(&second, &link).unwrap();
        let (text, mut reloaded) = file.reload_bounded(64).unwrap();
        assert_eq!(text, "same\n");
        reloaded.save("second edit\n").unwrap();
        assert_eq!(fs::read_to_string(&second).unwrap(), "second edit\n");
        assert_eq!(fs::read_to_string(&first).unwrap(), "edited\n");
    }

    #[cfg(unix)]
    #[test]
    fn dangling_links_are_refused_at_open_and_at_save() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("missing.md");
        let link = dir.path().join("dangling.md");
        symlink(&missing, &link).unwrap();
        let error = FileState::open_bounded(link.clone(), 1024).err().unwrap();
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
        assert!(error.to_string().contains("does not exist"), "{error}");
        assert!(!missing.exists());
        // A link whose target disappears after opening becomes dangling.
        fs::write(&missing, "here").unwrap();
        let (_, mut file) = FileState::open(link.clone()).unwrap();
        fs::remove_file(&missing).unwrap();
        assert_eq!(file.external_change(), ExternalChange::Missing);
        assert!(file.save("recreated").is_err());
        assert!(!missing.exists());
        assert!(is_link(&link));
        assert!(file.reload_bounded(64).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn links_to_directories_are_refused() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("folder");
        fs::create_dir(&folder).unwrap();
        let link = dir.path().join("folder-link");
        symlink(&folder, &link).unwrap();
        let error = FileState::open(link.clone()).err().unwrap();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        // A file link retargeted to a folder after opening is refused too.
        let note = dir.path().join("note.md");
        fs::write(&note, "note").unwrap();
        let file_link = dir.path().join("note-link.md");
        symlink(&note, &file_link).unwrap();
        let (_, mut file) = FileState::open(file_link.clone()).unwrap();
        fs::remove_file(&file_link).unwrap();
        symlink(&folder, &file_link).unwrap();
        assert!(file.save("changed").is_err());
        assert_eq!(fs::read_to_string(&note).unwrap(), "note");
        assert!(fs::read_dir(&folder).unwrap().next().is_none());
    }

    #[cfg(unix)]
    #[test]
    fn links_to_read_only_targets_are_refused() {
        use std::os::unix::fs::{PermissionsExt, symlink};

        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("locked.md");
        fs::write(&target, "locked").unwrap();
        let link = dir.path().join("link.md");
        symlink(&target, &link).unwrap();
        let (_, mut file) = FileState::open(link).unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o444)).unwrap();
        let saved = file.save("changed");
        fs::set_permissions(&target, fs::Permissions::from_mode(0o644)).unwrap();
        if unsafe { libc::geteuid() } != 0 {
            assert_eq!(saved.unwrap_err().kind(), io::ErrorKind::PermissionDenied);
            assert_eq!(fs::read_to_string(&target).unwrap(), "locked");
        }
    }
}
