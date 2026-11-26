//! The session's clipboard. A copy goes as far as it can: to the system
//! clipboard where one can be reached, otherwise to the terminal's (OSC 52),
//! and always to pica's own. Native access is lazy, serialized, and bounded.
use std::{
    io::{self, Write},
    sync::mpsc,
    thread,
    time::Duration,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    Empty,
    /// There is no system clipboard to reach. The detail is for debugging
    /// and is never shown.
    Unavailable(String),
    Busy,
    TimedOut,
}

pub trait Backend: Send {
    fn get_text(&mut self) -> Result<String, Error>;
    fn set_text(&mut self, text: &str) -> Result<(), Error>;
}

/// Why the system clipboard is out of use for this session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Why {
    /// The system clipboard here is not the one at the user's hands.
    Remote,
    Missing,
    Busy,
    Silent,
}

impl Why {
    fn of(error: &Error) -> Self {
        match error {
            Error::Busy => Self::Busy,
            Error::TimedOut => Self::Silent,
            Error::Empty | Error::Unavailable(_) => Self::Missing,
        }
    }

    /// A short, plain reason, to open or close a status message.
    fn phrase(self) -> &'static str {
        match self {
            Self::Remote => "over SSH",
            Self::Missing => "no system clipboard here",
            Self::Busy => "the system clipboard is busy",
            Self::Silent => "the system clipboard did not answer",
        }
    }
}

/// How far a copy got, best last.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Reach {
    /// Only pica's own clipboard: Ctrl+V in pica pastes it.
    Inside,
    /// Sent to the terminal, which sets the clipboard of the machine the
    /// user sits at if it allows OSC 52.
    Terminal,
    System,
}

/// Text sent through OSC 52 above this size is dropped by common terminals;
/// encoded, it is 100,000 bytes.
const OSC52_LIMIT: usize = 75_000;

type Sink = Box<dyn FnMut(&[u8]) -> io::Result<()> + Send>;

pub struct Clipboard {
    backend: Box<dyn Backend>,
    internal: String,
    fallback: Option<Why>,
    /// Where OSC 52 goes: the terminal, when it can be trusted to take it.
    terminal: Option<Sink>,
    /// How far the last copy got, so a change for the worse is said as a
    /// problem once and as plain news after that.
    reach: Reach,
    /// Whether the last message reported a problem the user must see.
    warned: bool,
}

impl Clipboard {
    /// No system clipboard and no terminal: tests, snapshots, and documents
    /// before the session's clipboard is lent to them.
    pub fn internal() -> Self {
        let mut clipboard = Self::with_backend(Box::new(InternalOnly));
        clipboard.fallback = Some(Why::Missing);
        clipboard.reach = Reach::Inside;
        clipboard
    }

    /// Over SSH the system clipboard is the wrong machine's. Copies go to
    /// the terminal; pastes come from pica's own copy, or from the
    /// terminal's paste.
    pub fn remote() -> Self {
        Self::internal().over_ssh(stdout_sink())
    }

    pub fn system() -> Self {
        let mut clipboard = Self::with_backend(native());
        clipboard.terminal = stdout_sink();
        clipboard
    }

    pub fn with_backend(backend: Box<dyn Backend>) -> Self {
        Self {
            backend,
            internal: String::new(),
            fallback: None,
            terminal: None,
            reach: Reach::System,
            warned: false,
        }
    }

    fn over_ssh(mut self, terminal: Option<Sink>) -> Self {
        self.fallback = Some(Why::Remote);
        self.reach = if terminal.is_some() {
            Reach::Terminal
        } else {
            Reach::Inside
        };
        self.terminal = terminal;
        self
    }

    /// Whether the last copy or paste reported a problem: something the
    /// user asked for did not happen, or happened less fully than before.
    /// Such a message stays until dismissed; others go with the next key.
    pub fn warned(&self) -> bool {
        self.warned
    }

    /// Copy `text` as far as it will go, and say how far. pica's own copy
    /// is kept whatever happens.
    pub fn copy(&mut self, text: &str) -> String {
        self.internal = text.to_owned();
        let mut failed = None;
        let reach = match self.fallback {
            None => match self.backend.set_text(text) {
                Ok(()) => Reach::System,
                Err(error) => {
                    let why = Why::of(&error);
                    self.fallback = Some(why);
                    failed = Some(why);
                    self.send(text)
                }
            },
            Some(_) => self.send(text),
        };
        // Worse than last time is a problem to see; the same is only news.
        self.warned = reach < self.reach;
        self.reach = reach;
        let why = self.fallback.unwrap_or(Why::Missing);
        match (reach, failed) {
            (Reach::System, _) => "Copied to the system clipboard".into(),
            (Reach::Terminal, Some(why)) => {
                format!(
                    "{} · copied to the terminal instead",
                    capitalized(why.phrase())
                )
            }
            (Reach::Terminal, None) => "Copied to the terminal's clipboard".into(),
            (Reach::Inside, _) if self.terminal.is_some() && text.len() > OSC52_LIMIT => {
                "Copied inside pica only · too large for the terminal's clipboard".into()
            }
            (Reach::Inside, _) if self.terminal.is_some() => {
                "Copied inside pica only · the terminal did not take it".into()
            }
            (Reach::Inside, _) => format!("Copied inside pica only · {}", why.phrase()),
        }
    }

    /// Hand `text` to the terminal as OSC 52, if there is one to hand it to
    /// and the text is of a size terminals accept.
    fn send(&mut self, text: &str) -> Reach {
        let Some(terminal) = &mut self.terminal else {
            return Reach::Inside;
        };
        if text.len() > OSC52_LIMIT {
            return Reach::Inside;
        }
        match terminal(&osc52(text)) {
            Ok(()) => Reach::Terminal,
            Err(_) => Reach::Inside,
        }
    }

    /// The system clipboard's text where it can be read, else pica's own
    /// copy. `None` leaves the document as it is.
    pub fn paste(&mut self) -> (Option<String>, String) {
        let mut failed = None;
        if self.fallback.is_none() {
            match self.backend.get_text() {
                Ok(text) if !text.is_empty() => {
                    self.warned = false;
                    return (Some(text), "Pasted from the system clipboard".into());
                }
                Ok(_) | Err(Error::Empty) => {
                    self.warned = true;
                    return (None, "The system clipboard has no text".into());
                }
                Err(error) => {
                    let why = Why::of(&error);
                    self.fallback = Some(why);
                    self.reach = self.reach.min(Reach::Terminal);
                    failed = Some(why);
                }
            }
        }
        // The system clipboard, if the user meant it, is the terminal's to
        // paste: say so, since Ctrl+V here cannot reach it.
        const ELSEWHERE: &str = "the terminal's paste has the system clipboard";
        let own = (!self.internal.is_empty()).then(|| self.internal.clone());
        self.warned = failed.is_some() || own.is_none();
        let message = match (&own, failed) {
            (Some(_), Some(why)) => {
                format!("{} · pasted pica's own copy", capitalized(why.phrase()))
            }
            (Some(_), None) => format!("Pasted pica's own copy · {ELSEWHERE}"),
            (None, Some(why)) => format!("{} · nothing to paste", capitalized(why.phrase())),
            (None, None) => format!("Nothing copied in pica yet · {ELSEWHERE}"),
        };
        (own, message)
    }
}

fn capitalized(phrase: &str) -> String {
    let mut characters = phrase.chars();
    match characters.next() {
        Some(first) => first.to_uppercase().chain(characters).collect(),
        None => String::new(),
    }
}

/// OSC 52: set the clipboard (`c`) to the Base64 of `text`.
fn osc52(text: &str) -> Vec<u8> {
    let mut sequence = b"\x1b]52;c;".to_vec();
    sequence.extend(base64(text.as_bytes()));
    sequence.extend(b"\x1b\\");
    sequence
}

/// Standard Base64 with padding (RFC 4648).
fn base64(bytes: &[u8]) -> Vec<u8> {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut encoded = Vec::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let group = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        for (index, shift) in [18, 12, 6, 0].into_iter().enumerate() {
            encoded.push(if index <= chunk.len() {
                ALPHABET[(group >> shift) as usize & 63]
            } else {
                b'='
            });
        }
    }
    encoded
}

/// Standard output as the way to the terminal, unless the terminal is one
/// known to print an OSC 52 it does not understand.
fn stdout_sink() -> Option<Sink> {
    let term = std::env::var("TERM").unwrap_or_default();
    if !terminal_takes_osc52(&term) {
        return None;
    }
    Some(Box::new(|bytes| {
        let mut output = io::stdout().lock();
        output.write_all(bytes)?;
        output.flush()
    }))
}

/// The Linux console prints the payload of sequences it does not know, and
/// a dumb or unnamed terminal promises nothing. Every other terminal takes
/// OSC 52 or ignores it.
fn terminal_takes_osc52(term: &str) -> bool {
    !matches!(term, "" | "dumb" | "linux")
}

pub fn remote_session() -> bool {
    ["SSH_CONNECTION", "SSH_CLIENT", "SSH_TTY"]
        .into_iter()
        .any(|name| std::env::var_os(name).is_some_and(|value| !value.is_empty()))
}

struct InternalOnly;
impl Backend for InternalOnly {
    fn get_text(&mut self) -> Result<String, Error> {
        Err(Error::Unavailable("no system clipboard in use".into()))
    }
    fn set_text(&mut self, _: &str) -> Result<(), Error> {
        self.get_text().map(|_| ())
    }
}

/// The system clipboard of this desktop: Wayland's through wl-clipboard
/// where a Wayland session has it installed, otherwise the platform's own.
fn native() -> Box<dyn Backend> {
    #[cfg(all(unix, not(target_os = "macos")))]
    if let Some(wayland) = Wayland::detect(|name| std::env::var_os(name)) {
        // Starting a program takes longer than a call; allow for it.
        return Box::new(Worker::new(
            Box::new(move || Box::new(wayland)),
            Duration::from_millis(1500),
        ));
    }
    Box::new(Worker::new(
        Box::new(|| Box::new(Native { clipboard: None })),
        Duration::from_millis(350),
    ))
}

struct Native {
    clipboard: Option<arboard::Clipboard>,
}
impl Native {
    fn get(&mut self) -> Result<&mut arboard::Clipboard, Error> {
        if self.clipboard.is_none() {
            self.clipboard = Some(arboard::Clipboard::new().map_err(native_error)?);
        }
        Ok(self.clipboard.as_mut().unwrap())
    }
}
impl Backend for Native {
    fn get_text(&mut self) -> Result<String, Error> {
        self.get()?.get_text().map_err(native_error)
    }
    fn set_text(&mut self, text: &str) -> Result<(), Error> {
        self.get()?.set_text(text).map_err(native_error)
    }
}
fn native_error(error: arboard::Error) -> Error {
    match error {
        arboard::Error::ContentNotAvailable => Error::Empty,
        arboard::Error::ClipboardOccupied => Error::Busy,
        error => Error::Unavailable(error.to_string()),
    }
}

/// Wayland's clipboard through the `wl-copy` and `wl-paste` programs of
/// wl-clipboard, which most Wayland desktops have. arboard reaches only
/// X11 here, and a Wayland-only session has no X11.
#[cfg(all(unix, not(target_os = "macos")))]
struct Wayland {
    copy: std::path::PathBuf,
    paste: std::path::PathBuf,
}

#[cfg(all(unix, not(target_os = "macos")))]
impl Wayland {
    /// Both programs, when this is a Wayland session that has them.
    fn detect(variable: impl Fn(&str) -> Option<std::ffi::OsString>) -> Option<Self> {
        variable("WAYLAND_DISPLAY").filter(|display| !display.is_empty())?;
        let path = variable("PATH")?;
        let find = |name: &str| {
            std::env::split_paths(&path)
                .map(|folder| folder.join(name))
                .find(|candidate| {
                    use std::os::unix::fs::PermissionsExt;
                    std::fs::metadata(candidate).is_ok_and(|metadata| {
                        metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
                    })
                })
        };
        Some(Self {
            copy: find("wl-copy")?,
            paste: find("wl-paste")?,
        })
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
impl Backend for Wayland {
    fn get_text(&mut self) -> Result<String, Error> {
        use std::process::{Command, Stdio};
        let output = Command::new(&self.paste)
            .arg("--no-newline")
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .map_err(|error| Error::Unavailable(error.to_string()))?;
        // wl-paste fails when nothing is copied or it is not text.
        if !output.status.success() {
            return Err(Error::Empty);
        }
        String::from_utf8(output.stdout).map_err(|_| Error::Empty)
    }

    fn set_text(&mut self, text: &str) -> Result<(), Error> {
        use std::process::{Command, Stdio};
        let unavailable = |error: io::Error| Error::Unavailable(error.to_string());
        // wl-copy stays behind to serve the clipboard and must not hold
        // any pipe of ours, or the wait below would never end.
        let mut child = Command::new(&self.copy)
            .args(["--type", "text/plain;charset=utf-8"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(unavailable)?;
        let written = child
            .stdin
            .take()
            .map_or(Ok(()), |mut input| input.write_all(text.as_bytes()));
        let status = child.wait().map_err(unavailable)?;
        written.map_err(unavailable)?;
        if status.success() {
            Ok(())
        } else {
            Err(Error::Unavailable(format!("wl-copy: {status}")))
        }
    }
}

type Factory = Box<dyn FnOnce() -> Box<dyn Backend> + Send>;
enum Action {
    Read,
    Write(String),
}
struct Request {
    action: Action,
    reply: mpsc::Sender<Result<Option<String>, Error>>,
}

struct Worker {
    factory: Option<Factory>,
    requests: Option<mpsc::Sender<Request>>,
    disabled: Option<Error>,
    timeout: Duration,
    completed: Option<mpsc::Receiver<()>>,
}

impl Worker {
    fn new(factory: Factory, timeout: Duration) -> Self {
        Self {
            factory: Some(factory),
            requests: None,
            disabled: None,
            timeout,
            completed: None,
        }
    }

    fn request(&mut self, action: Action) -> Result<Option<String>, Error> {
        if let Some(reason) = &self.disabled {
            return Err(reason.clone());
        }
        if self.requests.is_none() {
            let (sender, receiver) = mpsc::channel::<Request>();
            let (complete, completed) = mpsc::channel();
            let factory = self
                .factory
                .take()
                .ok_or_else(|| Error::Unavailable("System clipboard worker unavailable".into()))?;
            thread::Builder::new()
                .name("pica-clipboard".into())
                .spawn(move || {
                    // The worker owns the native handle for its lifetime, including X11 selection ownership.
                    let mut backend = factory();
                    while let Ok(request) = receiver.recv() {
                        let result = match request.action {
                            Action::Read => backend.get_text().map(Some),
                            Action::Write(text) => backend.set_text(&text).map(|()| None),
                        };
                        let _ = request.reply.send(result);
                    }
                    drop(backend);
                    let _ = complete.send(());
                })
                .map_err(|error| {
                    Error::Unavailable(format!("Cannot start system clipboard worker: {error}"))
                })?;
            self.requests = Some(sender);
            self.completed = Some(completed);
        }
        let (reply, response) = mpsc::channel();
        let sent = self
            .requests
            .as_ref()
            .unwrap()
            .send(Request { action, reply });
        // Completion is unknown after a timeout, so native access ends for
        // the session rather than risk a second, queued operation.
        let failure = if sent.is_ok() {
            match response.recv_timeout(self.timeout) {
                Ok(result) => return result,
                Err(mpsc::RecvTimeoutError::Timeout) => Error::TimedOut,
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    Error::Unavailable("System clipboard worker stopped".into())
                }
            }
        } else {
            Error::Unavailable("System clipboard worker stopped".into())
        };
        // Never spawn another worker after a timeout or queue more operations on the stalled one.
        self.requests = None;
        self.disabled = Some(failure.clone());
        Err(failure)
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.requests = None;
        if let Some(completed) = self.completed.take() {
            // Allow normal native teardown to finish, but never hang quitting.
            let _ = completed.recv_timeout(self.timeout.min(Duration::from_millis(150)));
        }
    }
}
impl Backend for Worker {
    fn get_text(&mut self) -> Result<String, Error> {
        self.request(Action::Read)?.ok_or(Error::Empty)
    }
    fn set_text(&mut self, text: &str) -> Result<(), Error> {
        self.request(Action::Write(text.to_owned())).map(|_| ())
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    };

    #[derive(Default)]
    pub struct State {
        pub text: String,
        pub writes: Vec<String>,
        pub reads: usize,
        pub error: Option<Error>,
    }
    pub struct Fake(pub Arc<Mutex<State>>);
    impl Backend for Fake {
        fn get_text(&mut self) -> Result<String, Error> {
            let mut state = self.0.lock().unwrap();
            state.reads += 1;
            if let Some(error) = &state.error {
                Err(error.clone())
            } else {
                Ok(state.text.clone())
            }
        }
        fn set_text(&mut self, text: &str) -> Result<(), Error> {
            let mut state = self.0.lock().unwrap();
            state.writes.push(text.to_owned());
            if let Some(error) = &state.error {
                Err(error.clone())
            } else {
                state.text = text.into();
                Ok(())
            }
        }
    }

    #[test]
    fn empty_native_paste_does_not_resurrect_internal_copy() {
        let state = Arc::new(Mutex::new(State::default()));
        let mut clipboard = Clipboard::with_backend(Box::new(Fake(state.clone())));
        clipboard.copy("older internal text");
        state.lock().unwrap().text.clear();
        assert_eq!(clipboard.paste().0, None);
        state.lock().unwrap().error = Some(Error::Empty);
        assert_eq!(clipboard.paste().0, None);
        state.lock().unwrap().error = Some(Error::Unavailable("offline".into()));
        let (text, message) = clipboard.paste();
        assert_eq!(text.as_deref(), Some("older internal text"));
        assert_eq!(message, "No system clipboard here · pasted pica's own copy");
        assert!(clipboard.warned());
    }

    #[test]
    fn worker_starts_only_on_request_and_serializes_native_access() {
        let starts = Arc::new(AtomicUsize::new(0));
        let count = starts.clone();
        let state = Arc::new(Mutex::new(State::default()));
        let fake = state.clone();
        let mut worker = Worker::new(
            Box::new(move || {
                count.fetch_add(1, Ordering::SeqCst);
                Box::new(Fake(fake))
            }),
            Duration::from_secs(1),
        );
        assert_eq!(starts.load(Ordering::SeqCst), 0);
        worker.set_text("**source**\r\n").unwrap();
        assert_eq!(worker.get_text().unwrap(), "**source**\r\n");
        assert_eq!(starts.load(Ordering::SeqCst), 1);
        assert_eq!(state.lock().unwrap().reads, 1);
    }

    #[test]
    fn failed_native_copy_switches_to_internal_instead_of_pasting_old_native_text() {
        let state = Arc::new(Mutex::new(State {
            text: "stale system value".into(),
            error: Some(Error::Unavailable("busy".into())),
            ..State::default()
        }));
        let mut clipboard = Clipboard::with_backend(Box::new(Fake(state.clone())));
        assert!(
            clipboard
                .copy("new retained copy")
                .starts_with("Copied inside pica only")
        );
        state.lock().unwrap().error = None;
        let (text, message) = clipboard.paste();
        assert_eq!(text.as_deref(), Some("new retained copy"));
        assert!(message.starts_with("Pasted pica's own copy"));
        assert_eq!(state.lock().unwrap().reads, 0);
    }

    #[test]
    fn a_clipboard_with_no_system_or_terminal_keeps_copies_inside_quietly() {
        let mut clipboard = Clipboard::internal();
        assert_eq!(
            clipboard.copy("source"),
            "Copied inside pica only · no system clipboard here"
        );
        // Nothing got worse: this is how the session began.
        assert!(!clipboard.warned());
        assert_eq!(clipboard.paste().0.as_deref(), Some("source"));
        assert!(!clipboard.warned());
    }

    /// What a test's terminal was sent.
    type Sent = Arc<Mutex<Vec<u8>>>;

    /// A clipboard whose terminal is a buffer the test can read.
    fn with_terminal(clipboard: Clipboard) -> (Clipboard, Sent) {
        let sent = Arc::new(Mutex::new(Vec::new()));
        let sink = sent.clone();
        let mut clipboard = clipboard;
        clipboard.terminal = Some(Box::new(move |bytes| {
            sink.lock().unwrap().extend_from_slice(bytes);
            Ok(())
        }));
        (clipboard, sent)
    }
    fn over_ssh() -> (Clipboard, Sent) {
        let (clipboard, sent) = with_terminal(Clipboard::internal());
        let terminal = clipboard.terminal;
        let clipboard = Clipboard::internal().over_ssh(terminal);
        (clipboard, sent)
    }
    fn failing(error: Error) -> (Clipboard, Arc<Mutex<State>>, Sent) {
        let state = Arc::new(Mutex::new(State {
            text: "what the system clipboard held".into(),
            error: Some(error),
            ..State::default()
        }));
        let (clipboard, sent) =
            with_terminal(Clipboard::with_backend(Box::new(Fake(state.clone()))));
        (clipboard, state, sent)
    }

    #[test]
    fn osc52_carries_the_exact_text_as_padded_base64() {
        // RFC 4648 test vectors.
        for (text, encoded) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64(text.as_bytes()), encoded.as_bytes(), "{text:?}");
        }
        assert_eq!(base64(&[0xfb, 0xff, 0xfe]), b"+//+");
        assert_eq!(osc52("héllo\n"), b"\x1b]52;c;aMOpbGxvCg==\x1b\\");
        assert_eq!(
            osc52("# Titre\n\n**gras** 界\n"),
            b"\x1b]52;c;IyBUaXRyZQoKKipncmFzKiog55WMCg==\x1b\\"
        );
    }

    #[test]
    fn over_ssh_a_copy_goes_to_the_terminal_and_a_paste_comes_from_pica() {
        let (mut clipboard, sent) = over_ssh();
        // Nothing copied yet: Ctrl+V has nothing, and says where to look.
        let (text, message) = clipboard.paste();
        assert_eq!(text, None);
        assert_eq!(
            message,
            "Nothing copied in pica yet · the terminal's paste has the system clipboard"
        );
        assert!(clipboard.warned());
        assert_eq!(
            clipboard.copy("**source**\r\n"),
            "Copied to the terminal's clipboard"
        );
        assert!(!clipboard.warned());
        assert_eq!(*sent.lock().unwrap(), osc52("**source**\r\n"));
        let (text, message) = clipboard.paste();
        assert_eq!(text.as_deref(), Some("**source**\r\n"));
        assert_eq!(
            message,
            "Pasted pica's own copy · the terminal's paste has the system clipboard"
        );
        assert!(!clipboard.warned());
        // One sequence per copy, each whole.
        clipboard.copy("second");
        let mut both = osc52("**source**\r\n");
        both.extend(osc52("second"));
        assert_eq!(*sent.lock().unwrap(), both);
    }

    #[test]
    fn a_copy_too_large_for_the_terminal_stays_inside_and_is_a_problem_once() {
        let (mut clipboard, sent) = over_ssh();
        let most = "x".repeat(OSC52_LIMIT);
        assert_eq!(clipboard.copy(&most), "Copied to the terminal's clipboard");
        assert_eq!(sent.lock().unwrap().len(), 7 + 100_000 + 2);
        sent.lock().unwrap().clear();
        let too_much = "x".repeat(OSC52_LIMIT + 1);
        assert_eq!(
            clipboard.copy(&too_much),
            "Copied inside pica only · too large for the terminal's clipboard"
        );
        assert!(clipboard.warned());
        assert!(sent.lock().unwrap().is_empty());
        // The same again is news, not a new problem; pica still has it.
        clipboard.copy(&too_much);
        assert!(!clipboard.warned());
        assert_eq!(clipboard.paste().0.as_deref(), Some(too_much.as_str()));
        // Back in reach, and out of it again: the problem is said again.
        clipboard.copy("small");
        assert!(!clipboard.warned());
        clipboard.copy(&too_much);
        assert!(clipboard.warned());
    }

    #[test]
    fn a_missing_system_clipboard_falls_back_to_the_terminal_and_warns_once() {
        for (error, opening) in [
            (
                Error::Unavailable("Unknown error while interacting with the clipboard".into()),
                "No system clipboard here",
            ),
            (Error::Busy, "The system clipboard is busy"),
            (Error::TimedOut, "The system clipboard did not answer"),
        ] {
            let (mut clipboard, state, sent) = failing(error);
            assert_eq!(
                clipboard.copy("one"),
                format!("{opening} · copied to the terminal instead")
            );
            assert!(clipboard.warned());
            assert_eq!(*sent.lock().unwrap(), osc52("one"));
            // From then on the terminal is simply where copies go.
            state.lock().unwrap().error = None;
            assert_eq!(clipboard.copy("two"), "Copied to the terminal's clipboard");
            assert!(!clipboard.warned());
            assert_eq!(state.lock().unwrap().writes, ["one"]);
            // Ctrl+V pastes what pica copied, not what the system held.
            let (text, _) = clipboard.paste();
            assert_eq!(text.as_deref(), Some("two"));
            assert!(!clipboard.warned());
            assert_eq!(state.lock().unwrap().reads, 0);
        }
        // A paste that finds the system clipboard gone says so once too.
        let (mut clipboard, _, sent) = failing(Error::Unavailable("offline".into()));
        let (text, message) = clipboard.paste();
        assert_eq!(text, None);
        assert_eq!(message, "No system clipboard here · nothing to paste");
        assert!(clipboard.warned());
        assert_eq!(
            clipboard.copy("later"),
            "Copied to the terminal's clipboard"
        );
        assert!(!clipboard.warned());
        assert_eq!(*sent.lock().unwrap(), osc52("later"));
    }

    #[test]
    fn without_a_terminal_to_take_it_a_copy_stays_inside_and_says_so() {
        // A terminal that cannot be written to.
        let mut clipboard =
            Clipboard::internal().over_ssh(Some(Box::new(|_| Err(io::Error::other("closed")))));
        assert_eq!(
            clipboard.copy("text"),
            "Copied inside pica only · the terminal did not take it"
        );
        assert!(clipboard.warned());
        assert_eq!(clipboard.paste().0.as_deref(), Some("text"));
        // No terminal at all, as on the Linux console.
        let mut clipboard = Clipboard::internal().over_ssh(None);
        assert_eq!(clipboard.copy("text"), "Copied inside pica only · over SSH");
        assert!(!clipboard.warned());
        let state = Arc::new(Mutex::new(State {
            error: Some(Error::Unavailable("no display".into())),
            ..State::default()
        }));
        let mut clipboard = Clipboard::with_backend(Box::new(Fake(state)));
        assert_eq!(
            clipboard.copy("text"),
            "Copied inside pica only · no system clipboard here"
        );
        assert!(clipboard.warned());
        clipboard.copy("more");
        assert!(!clipboard.warned());
        for (term, takes) in [
            ("xterm-256color", true),
            ("xterm-ghostty", true),
            ("tmux-256color", true),
            ("screen", true),
            ("linux", false),
            ("dumb", false),
            ("", false),
        ] {
            assert_eq!(terminal_takes_osc52(term), takes, "{term:?}");
        }
    }

    #[test]
    fn every_message_is_short_and_in_plain_words() {
        let mut messages = Vec::new();
        let (mut clipboard, _) = over_ssh();
        messages.push(clipboard.paste().1);
        messages.push(clipboard.copy("text"));
        messages.push(clipboard.paste().1);
        messages.push(clipboard.copy(&"x".repeat(OSC52_LIMIT + 1)));
        for error in [
            Error::Unavailable(
                "Unknown error while interacting with the clipboard: X11 server connection timed out because it was unreachable".into(),
            ),
            Error::Busy,
            Error::TimedOut,
        ] {
            let (mut clipboard, _, _) = failing(error.clone());
            messages.push(clipboard.copy("text"));
            messages.push(clipboard.paste().1);
            let (mut clipboard, _, _) = failing(error.clone());
            messages.push(clipboard.paste().1);
            let state = Arc::new(Mutex::new(State {
                error: Some(error),
                ..State::default()
            }));
            let mut clipboard = Clipboard::with_backend(Box::new(Fake(state)));
            messages.push(clipboard.copy("text"));
            messages.push(clipboard.paste().1);
        }
        let state = Arc::new(Mutex::new(State::default()));
        let mut clipboard = Clipboard::with_backend(Box::new(Fake(state)));
        messages.push(clipboard.paste().1);
        messages.push(clipboard.copy("text"));
        messages.push(clipboard.paste().1);
        for message in messages {
            // Room for " Esc" beside it in an 80-column status line.
            assert!(message.chars().count() <= 74, "{message}");
            for raw in ["rror", "Unknown", "X11", "internal", "unavailable", "OSC"] {
                assert!(!message.contains(raw), "{message}");
            }
        }
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    mod wayland {
        use super::*;
        use std::{ffi::OsString, fs, os::unix::fs::PermissionsExt, path::Path};

        /// Stand-ins for wl-clipboard that keep the clipboard in a file and
        /// note how they were called.
        fn tools(folder: &Path, names: &[&str]) {
            for name in names {
                let script = match *name {
                    "wl-copy" => {
                        "#!/bin/sh\necho \"$@\" > \"$(dirname \"$0\")/copy-arguments\"\ncat > \"$(dirname \"$0\")/clipboard\"\n"
                    }
                    _ => {
                        "#!/bin/sh\necho \"$@\" > \"$(dirname \"$0\")/paste-arguments\"\n[ -s \"$(dirname \"$0\")/clipboard\" ] || exit 1\ncat \"$(dirname \"$0\")/clipboard\"\n"
                    }
                };
                let path = folder.join(name);
                fs::write(&path, script).unwrap();
                fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
            }
        }
        fn session(
            display: Option<&str>,
            path: &Path,
        ) -> impl Fn(&str) -> Option<OsString> + use<> {
            let display = display.map(OsString::from);
            let path = path.as_os_str().to_owned();
            move |name| match name {
                "WAYLAND_DISPLAY" => display.clone(),
                "PATH" => Some(path.clone()),
                _ => None,
            }
        }
        /// A program written a moment ago can be "busy" to run while another
        /// test's child process still holds it open; try again.
        fn settled<T>(mut attempt: impl FnMut() -> Result<T, Error>) -> Result<T, Error> {
            for _ in 0..50 {
                match attempt() {
                    Err(Error::Unavailable(detail)) if detail.contains("busy") => {
                        std::thread::sleep(Duration::from_millis(20));
                    }
                    result => return result,
                }
            }
            attempt()
        }

        #[test]
        fn a_wayland_session_with_wl_clipboard_is_found_and_nothing_else_is() {
            let dir = tempfile::tempdir().unwrap();
            let none = Wayland::detect(session(Some("wayland-0"), dir.path()));
            assert!(none.is_none());
            tools(dir.path(), &["wl-copy"]);
            assert!(Wayland::detect(session(Some("wayland-0"), dir.path())).is_none());
            tools(dir.path(), &["wl-paste"]);
            let found = Wayland::detect(session(Some("wayland-0"), dir.path())).unwrap();
            assert_eq!(found.copy, dir.path().join("wl-copy"));
            assert_eq!(found.paste, dir.path().join("wl-paste"));
            // Not a Wayland session, or the programs cannot be run.
            assert!(Wayland::detect(session(None, dir.path())).is_none());
            assert!(Wayland::detect(session(Some(""), dir.path())).is_none());
            fs::set_permissions(
                dir.path().join("wl-paste"),
                fs::Permissions::from_mode(0o644),
            )
            .unwrap();
            assert!(Wayland::detect(session(Some("wayland-0"), dir.path())).is_none());
        }

        #[test]
        fn wl_clipboard_copies_and_pastes_the_exact_text() {
            let dir = tempfile::tempdir().unwrap();
            tools(dir.path(), &["wl-copy", "wl-paste"]);
            let mut wayland = Wayland::detect(session(Some("wayland-0"), dir.path())).unwrap();
            // Nothing copied: wl-paste fails, which is an empty clipboard.
            assert_eq!(settled(|| wayland.get_text()), Err(Error::Empty));
            let text = "**source** 界\r\nlast line\n";
            settled(|| wayland.set_text(text)).unwrap();
            assert_eq!(
                fs::read_to_string(dir.path().join("clipboard")).unwrap(),
                text
            );
            assert_eq!(wayland.get_text().unwrap(), text);
            assert_eq!(
                fs::read_to_string(dir.path().join("copy-arguments")).unwrap(),
                "--type text/plain;charset=utf-8\n"
            );
            assert_eq!(
                fs::read_to_string(dir.path().join("paste-arguments")).unwrap(),
                "--no-newline\n"
            );
            // Bytes that are not text are not pasted.
            fs::write(dir.path().join("clipboard"), [0xff, 0xfe]).unwrap();
            assert_eq!(wayland.get_text(), Err(Error::Empty));
            // As the session's clipboard it is the system clipboard.
            let mut clipboard = Clipboard::with_backend(Box::new(wayland));
            assert_eq!(
                clipboard.copy("through pica"),
                "Copied to the system clipboard"
            );
            assert_eq!(
                clipboard.paste(),
                (
                    Some("through pica".to_owned()),
                    "Pasted from the system clipboard".to_owned()
                )
            );
            assert!(!clipboard.warned());
            // Programs that have gone away are a missing clipboard.
            fs::remove_file(dir.path().join("wl-copy")).unwrap();
            assert!(
                clipboard
                    .copy("again")
                    .starts_with("Copied inside pica only")
            );
            assert!(clipboard.warned());
            assert_eq!(clipboard.paste().0.as_deref(), Some("again"));
        }
    }

    #[test]
    fn normal_shutdown_waits_for_backend_drop() {
        struct DropSpy(Arc<AtomicUsize>);
        impl Backend for DropSpy {
            fn get_text(&mut self) -> Result<String, Error> {
                Ok("text".into())
            }
            fn set_text(&mut self, _: &str) -> Result<(), Error> {
                Ok(())
            }
        }
        impl Drop for DropSpy {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }
        let dropped = Arc::new(AtomicUsize::new(0));
        let spy = dropped.clone();
        let mut worker = Worker::new(
            Box::new(move || Box::new(DropSpy(spy))),
            Duration::from_secs(1),
        );
        worker.get_text().unwrap();
        drop(worker);
        assert_eq!(dropped.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn timeout_disables_worker_without_retry_or_unbounded_ui_wait() {
        struct Blocked(mpsc::Receiver<()>);
        impl Backend for Blocked {
            fn get_text(&mut self) -> Result<String, Error> {
                let _ = self.0.recv();
                Ok("late".into())
            }
            fn set_text(&mut self, _: &str) -> Result<(), Error> {
                self.get_text().map(|_| ())
            }
        }
        let (release, wait) = mpsc::channel();
        let mut worker = Worker::new(
            Box::new(move || Box::new(Blocked(wait))),
            Duration::from_millis(10),
        );
        assert_eq!(worker.get_text(), Err(Error::TimedOut));
        assert!(worker.requests.is_none());
        assert_eq!(worker.get_text(), Err(Error::TimedOut));
        release.send(()).unwrap();
    }
}
