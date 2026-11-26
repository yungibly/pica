mod app;
mod browser;
mod cli;
mod clipboard;
mod file_io;
mod icons;
mod paths;
mod projection;
mod recovery;
mod search;
mod search_highlight;
mod settings;
mod simulation;
mod syntax;
mod terminal;
mod theme;
mod ui;
mod workspace;

use std::io::{self, IsTerminal};

/// Piped input and files share the open limit; undo history is 32 MiB.
const MAX_BYTES: usize = 8 * 1024 * 1024;

fn main() {
    if let Err(error) = run() {
        eprintln!("pica: {error}");
        std::process::exit(1);
    }
}

fn run() -> io::Result<()> {
    let options = match cli::parse(std::env::args_os().skip(1))? {
        cli::Action::Help => {
            print!("{}", cli::HELP);
            return Ok(());
        }
        cli::Action::Version => {
            println!("pica {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        cli::Action::ListThemes => {
            for theme in theme::Theme::choices() {
                println!("{}\t{}", theme.id(), theme.name());
            }
            return Ok(());
        }
        cli::Action::Edit(options) => options,
    };
    theme::set_theme(options.theme.unwrap_or_default());
    // Headless snapshots stay reproducible regardless of the outer terminal.
    icons::set(options.icons.unwrap_or_else(|| {
        if options.snapshot {
            icons::IconSet::Plain
        } else {
            icons::IconSet::detect(|name| std::env::var(name).ok())
        }
    }));
    let mut read_stdin = false;
    let mut items = Vec::new();
    for target in options.targets {
        items.push(match target.input {
            cli::Input::File(path) => {
                let (path, position) = cli::locate(path, target.position);
                workspace::Launch::Path(path, position)
            }
            cli::Input::Stdin => {
                read_stdin = true;
                workspace::Launch::Piped(stdin_text()?, target.position)
            }
        });
    }
    // Like micro, piped input with no file opens as a buffer: `cmd | pica`.
    if items.is_empty() && stdin_is_piped() {
        read_stdin = true;
        items.push(workspace::Launch::Piped(stdin_text()?, options.position));
    }
    let mut app = workspace::Workspace::launch(items)?;
    if options.source {
        app.show_source();
    }
    if options.no_wrap {
        app.set_wrap(false);
    }
    if options.snapshot {
        print!(
            "{}",
            simulation::snapshot(&mut app, options.size.0, options.size.1)?
        );
        return Ok(());
    }
    // Keys come from the terminal. After reading piped input, Crossterm
    // falls back to /dev/tty for them; otherwise stdin must be a terminal.
    if !io::stdout().is_terminal() || !(io::stdin().is_terminal() || read_stdin && has_tty()) {
        return Err(io::Error::other(
            "Interactive mode needs a terminal; use --snapshot for a headless render",
        ));
    }
    app.enable_system_clipboard();
    if !options.no_state {
        match settings::directories() {
            Ok((config, state)) => {
                app.enable_state(&config, &state, options.theme.is_some());
                if let Some(icons) = options.icons {
                    icons::set(icons);
                }
                // The flag outlasts the remembered choice, for this launch.
                if options.no_wrap {
                    app.set_wrap(false);
                }
            }
            Err(error) => app.editor_mut().set_message(error.to_string()),
        }
    }
    terminal::run(&mut app)
}

fn stdin_text() -> io::Result<String> {
    file_io::read_input(io::stdin().lock(), MAX_BYTES)
}

/// A pipe or redirected file, not a terminal or a device such as /dev/null.
#[cfg(unix)]
fn stdin_is_piped() -> bool {
    use std::os::{fd::AsFd, unix::fs::FileTypeExt};
    let Ok(descriptor) = io::stdin().as_fd().try_clone_to_owned() else {
        return false;
    };
    std::fs::File::from(descriptor)
        .metadata()
        .is_ok_and(|metadata| metadata.file_type().is_fifo() || metadata.is_file())
}

#[cfg(not(unix))]
fn stdin_is_piped() -> bool {
    !io::stdin().is_terminal()
}

#[cfg(unix)]
fn has_tty() -> bool {
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")
        .is_ok()
}

/// Crossterm's Windows backend reads the console, not standard input.
#[cfg(not(unix))]
fn has_tty() -> bool {
    true
}
