//! Hands a web or mail address to the platform's opener: `xdg-open` on
//! Linux and the BSDs, `/usr/bin/open` on macOS. Callers pass only addresses that
//! `links::classify` accepted, and only when the user asked.
//!
//! The opener is run directly, never through a shell, with the address as
//! its one argument, so nothing in an address is ever interpreted. It gets
//! no terminal: its input and output are closed, and on Unix it starts in a
//! session of its own, without the editor's controlling terminal. A text
//! browser chosen by `xdg-open` on a machine with no desktop therefore
//! fails quietly instead of drawing over the editor.
use std::io;
#[cfg(unix)]
use std::{
    ffi::OsStr,
    process::{Child, Command, Stdio},
};

/// macOS ships its opener at a fixed path, so nothing earlier on PATH can
/// stand in for it. `xdg-open` has no fixed home and is found on PATH.
#[cfg(target_os = "macos")]
#[cfg_attr(test, allow(dead_code))]
const OPENER: &str = "/usr/bin/open";
#[cfg(all(unix, not(target_os = "macos")))]
#[cfg_attr(test, allow(dead_code))]
const OPENER: &str = "xdg-open";

/// Open `address` in the browser or mail program. An error means the
/// opener could not be started; what it does next is its own business.
#[cfg(unix)]
#[cfg_attr(test, allow(dead_code))]
pub fn open(address: &str) -> io::Result<()> {
    let child = launch(OsStr::new(OPENER), address).map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            io::Error::new(error.kind(), format!("{OPENER} is not installed"))
        } else {
            error
        }
    })?;
    reap(child);
    Ok(())
}

#[cfg(not(unix))]
#[cfg_attr(test, allow(dead_code))]
pub fn open(_address: &str) -> io::Result<()> {
    Err(io::Error::other(
        "Opening links is not supported on this system",
    ))
}

/// Start `program` with `address` as its only argument, detached from the
/// terminal.
#[cfg(unix)]
fn launch(program: &OsStr, address: &str) -> io::Result<Child> {
    let mut command = Command::new(program);
    command
        .arg(address)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    use std::os::unix::process::CommandExt;
    // A session of its own has no controlling terminal: the opener and
    // whatever it starts cannot open /dev/tty or receive the editor's
    // signals. `setsid` is async-signal-safe, as `pre_exec` requires.
    unsafe {
        command.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    command.spawn()
}

/// Collect the opener's exit status when it comes, so it is not left as a
/// zombie while the editor runs.
#[cfg(unix)]
#[cfg_attr(test, allow(dead_code))]
fn reap(mut child: Child) {
    std::thread::spawn(move || {
        let _ = child.wait();
    });
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::{fs, os::unix::fs::PermissionsExt, time::Duration};

    /// A stand-in opener that records what it was given and how it ran.
    fn recorder(directory: &std::path::Path) -> std::path::PathBuf {
        let script = directory.join("opener");
        let out = directory.join("out");
        fs::write(
            &script,
            format!(
                "#!/bin/sh\n\
                 {{\n\
                 printf 'count=%s\\n' \"$#\"\n\
                 printf 'first=%s\\n' \"$1\"\n\
                 if [ -t 0 ] || [ -t 1 ] || [ -t 2 ]; then echo stdio=terminal; else echo stdio=closed; fi\n\
                 if (: < /dev/tty) 2>/dev/null; then echo tty=reachable; else echo tty=none; fi\n\
                 echo done\n\
                 }} > '{}.tmp'\n\
                 mv '{}.tmp' '{}'\n",
                out.display(),
                out.display(),
                out.display()
            ),
        )
        .unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        script
    }

    #[test]
    fn the_opener_gets_the_address_as_one_argument_and_no_terminal() {
        let directory = tempfile::tempdir().unwrap();
        let script = recorder(directory.path());
        // Everything a shell would act on, in an address the classifier
        // accepts: it must arrive untouched, as a single argument.
        let address = "https://example.com/a?b=$(touch pwned)&c=`id`;rm -rf ~|x>y#'\"*";
        let marker = directory.path().join("pwned");
        let mut child = launch(script.as_os_str(), address).unwrap();
        // A session leader has its own id as its session's.
        let pid = child.id() as libc::pid_t;
        let session = unsafe { libc::getsid(pid) };
        assert!(
            session == pid || session == -1,
            "the opener leads its own session"
        );
        assert_ne!(session, unsafe { libc::getsid(0) });
        assert!(child.wait().unwrap().success());
        let out = directory.path().join("out");
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !out.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        let recorded = fs::read_to_string(&out).unwrap();
        assert_eq!(
            recorded,
            format!("count=1\nfirst={address}\nstdio=closed\ntty=none\ndone\n")
        );
        assert!(!marker.exists(), "nothing in the address was run");
        assert!(!std::path::Path::new("pwned").exists());
    }

    #[test]
    fn a_missing_opener_is_an_error_and_not_a_panic() {
        let error = launch(
            OsStr::new("/nonexistent/pica-opener"),
            "https://example.com",
        )
        .expect_err("no such program");
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
    }
}
