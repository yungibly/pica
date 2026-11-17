use std::{
    fs,
    io::Write,
    process::{Command, Stdio},
};

fn binary() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pica"))
}

#[test]
fn snapshots_use_requested_dimensions_and_preserve_source() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("snapshot.md");
    let source = "\u{feff}# A heading\r\n\r\nSome **strong** source.\r\n";
    fs::write(&path, source).unwrap();
    for (size, height) in [
        ("1x1", 1),
        ("20x6", 6),
        ("80x24", 24),
        ("120x36", 36),
        ("160x45", 45),
    ] {
        for theme in ["dark", "light"] {
            let output = binary()
                .args(["--snapshot", "--snapshot-size", size, "--theme", theme])
                .arg(&path)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{size} {theme}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let text = String::from_utf8(output.stdout).unwrap();
            assert_eq!(text.lines().count(), height + 2);
            assert!(text.contains("Caret source byte:"));
            assert_eq!(fs::read(&path).unwrap(), source.as_bytes());
        }
    }
}

#[test]
fn command_line_open_rejects_binary_and_oversized_files() {
    let directory = tempfile::tempdir().unwrap();
    let binary_path = directory.path().join("binary.md");
    fs::write(&binary_path, b"before\0after").unwrap();
    let output = binary()
        .arg("--snapshot")
        .arg(&binary_path)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("NUL"));
    assert_eq!(fs::read(binary_path).unwrap(), b"before\0after");

    let large = directory.path().join("large.md");
    fs::File::create(&large)
        .unwrap()
        .set_len(8 * 1024 * 1024 + 1)
        .unwrap();
    let output = binary().arg("--snapshot").arg(&large).output().unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("open limit"));
    assert_eq!(fs::metadata(large).unwrap().len(), 8 * 1024 * 1024 + 1);
}

#[test]
fn invalid_snapshot_options_fail_without_creating_the_target() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("missing.md");
    let output = binary()
        .args(["--snapshot", "--snapshot-size", "999x999"])
        .arg(&path)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("WIDTHxHEIGHT"));
    assert!(!path.exists());
}

#[test]
fn builtin_theme_catalog_and_named_snapshots_are_state_free() {
    let directory = tempfile::tempdir().unwrap();
    let config = directory.path().join("config");
    let state = directory.path().join("state");
    let output = binary()
        .arg("--list-themes")
        .env("XDG_CONFIG_HOME", &config)
        .env("XDG_STATE_HOME", &state)
        .output()
        .unwrap();
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.lines().count() > 600);
    for name in [
        "Catppuccin Mocha",
        "Dracula",
        "Nord",
        "Gruvbox Dark",
        "TokyoNight",
    ] {
        assert!(text.contains(name), "{name}");
    }
    for name in ["Catppuccin Mocha", "nord", "one-dark", "solarized-light"] {
        let output = binary()
            .args(["--snapshot", "--theme", name])
            .env("XDG_CONFIG_HOME", &config)
            .env("XDG_STATE_HOME", &state)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{name}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    assert!(!config.exists());
    assert!(!state.exists());
}

#[test]
fn icon_snapshots_use_explicit_flags_without_reading_or_changing_preferences() {
    let directory = tempfile::tempdir().unwrap();
    let config = directory.path().join("config");
    let state = directory.path().join("state");
    fs::create_dir_all(config.join("pica")).unwrap();
    let preferences = config.join("pica/settings.conf");
    fs::write(&preferences, "version=1\nicons=nerd\n").unwrap();
    for (arguments, nerd) in [
        (vec!["--snapshot"], false),
        (vec!["--snapshot", "--icons=nerd"], true),
        (vec!["--snapshot", "--icons", "plain"], false),
    ] {
        let output = binary()
            .args(arguments)
            .env("XDG_CONFIG_HOME", &config)
            .env("XDG_STATE_HOME", &state)
            .output()
            .unwrap();
        assert!(output.status.success());
        let text = String::from_utf8(output.stdout).unwrap();
        assert_eq!(text.contains('\u{e73e}'), nerd);
    }
    assert_eq!(
        fs::read_to_string(preferences).unwrap(),
        "version=1\nicons=nerd\n"
    );
    assert!(!state.exists());
}

#[test]
fn pica_command_branding_does_not_initialize_user_state() {
    let directory = tempfile::tempdir().unwrap();
    let config = directory.path().join("config");
    let state = directory.path().join("state");
    for (argument, expected) in [
        ("--help", "pica — a small terminal text editor".to_owned()),
        ("--version", format!("pica {}", env!("CARGO_PKG_VERSION"))),
        ("--snapshot", " untitled ".to_owned()),
    ] {
        let output = binary()
            .arg(argument)
            .env("XDG_CONFIG_HOME", &config)
            .env("XDG_STATE_HOME", &state)
            .output()
            .unwrap();
        assert!(output.status.success());
        let text = String::from_utf8(output.stdout).unwrap();
        assert!(text.contains(&expected), "{argument}: {text}");
        assert!(!text.to_lowercase().contains("marklane"));
    }
    let output = binary()
        .arg("--unknown-option")
        .env("XDG_CONFIG_HOME", &config)
        .env("XDG_STATE_HOME", &state)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).starts_with("pica:"));
    assert!(!config.exists());
    assert!(!state.exists());
}

fn snapshot(arguments: &[&std::ffi::OsStr]) -> String {
    let output = binary()
        .args(["--snapshot", "--no-state", "--snapshot-size", "100x12"])
        .args(arguments)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

fn piped(arguments: &[&str], input: &[u8]) -> std::process::Output {
    let mut child = binary()
        .args(["--snapshot", "--no-state", "--snapshot-size", "80x10"])
        .args(arguments)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    // A rejected oversized input may close the pipe before it is all written.
    let _ = stdin.write_all(input);
    drop(stdin);
    child.wait_with_output().unwrap()
}

#[test]
fn several_files_open_as_tabs_with_the_first_active() {
    let directory = tempfile::tempdir().unwrap();
    let first = directory.path().join("first.md");
    let second = directory.path().join("second.rs");
    fs::write(&first, "# First\n").unwrap();
    fs::write(&second, "fn main() {}\n").unwrap();
    let text = snapshot(&[first.as_os_str(), second.as_os_str()]);
    let tabs = text.lines().next().unwrap();
    assert!(
        tabs.contains("first.md") && tabs.contains("second.rs"),
        "{text}"
    );
    assert!(tabs.find("first.md") < tabs.find("second.rs"));
    assert!(text.contains(" markdown"), "{text}");
}

#[test]
fn plus_line_and_file_colon_line_place_the_caret() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("lines.txt");
    let source: String = (1..=40).map(|n| format!("line {n}\n")).collect();
    fs::write(&path, &source).unwrap();
    let plus = std::ffi::OsStr::new("+30:4");
    for arguments in [vec![plus, path.as_os_str()], vec![path.as_os_str(), plus]] {
        let text = snapshot(&arguments);
        assert!(text.contains("Ln 30, Col 4"), "{text}");
        assert!(text.contains("line 30"), "{text}");
    }
    let colon = format!("{}:12:2", path.display());
    let text = snapshot(&[std::ffi::OsStr::new(&colon)]);
    assert!(text.contains("Ln 12, Col 2"), "{text}");
    assert!(!directory.path().join("lines.txt:12:2").exists());
    assert_eq!(fs::read_to_string(path).unwrap(), source);
}

#[test]
fn a_folder_opens_the_file_browser_there() {
    let directory = tempfile::tempdir().unwrap();
    fs::write(directory.path().join("inside.md"), "x").unwrap();
    let text = snapshot(&[directory.path().as_os_str()]);
    assert!(text.contains("Open file"), "{text}");
    assert!(text.contains("inside.md"), "{text}");
}

#[test]
fn piped_input_opens_as_unsaved_plain_text() {
    let output = piped(&[], b"# not a heading\n  two\r\n");
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains(" untitled \u{25cf}"), "{text}");
    assert!(text.contains("# not a heading"), "{text}");
    assert!(text.contains(" text"), "{text}");
    assert!(!text.contains("markdown"), "{text}");
    // `-` reads it explicitly, beside files, and takes a position.
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("file.md");
    fs::write(&file, "file").unwrap();
    let output = piped(&["-", "+2", file.to_str().unwrap()], b"one\ntwo\n");
    let text = String::from_utf8(output.stdout).unwrap();
    // The position binds to the file after it; the piped buffer is first.
    assert!(text.contains("Ln 1, Col 1"), "{text}");
    assert!(text.lines().next().unwrap().contains("untitled"), "{text}");
    let output = piped(&["-", "+2"], b"one\ntwo\n");
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .contains("Ln 2, Col 1")
    );
}

#[test]
fn piped_input_rejects_binary_invalid_and_oversized_text() {
    for (input, message) in [
        (b"bad \xff byte".to_vec(), "UTF-8"),
        (b"nul\0byte".to_vec(), "NUL"),
        (vec![b'x'; 8 * 1024 * 1024 + 1], "8 MiB"),
    ] {
        let output = piped(&[], &input);
        assert!(!output.status.success());
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(
            error.starts_with("pica:") && error.contains(message),
            "{error}"
        );
    }
    let output = piped(&["-", "-"], b"");
    assert!(!output.status.success());
}

#[test]
fn interactive_mode_still_needs_a_terminal_for_output() {
    let directory = tempfile::tempdir().unwrap();
    let output = binary()
        .arg("--no-state")
        .arg(directory.path().join("a.md"))
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("needs a terminal"));
    assert!(!directory.path().join("a.md").exists());
}
