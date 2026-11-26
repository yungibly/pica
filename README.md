# pica

[![CI](https://github.com/yungibly/pica/actions/workflows/ci.yml/badge.svg)](https://github.com/yungibly/pica/actions/workflows/ci.yml)

A small terminal text editor in the nano and micro family: micro's keys, nano's quiet, and Markdown rendered as you write.

```sh
brew install yungibly/tap/pica
pica notes.md
```

It opens however you launch it, uses the keys you already know from micro, takes your terminal's own colors, and stays out of the way. Configs, scripts, and commit messages get a plain, fast editor. Markdown also gets rendered headings, tables, code, and checklists that turn back into their exact source as soon as you edit them.

## Install

`brew install yungibly/tap/pica` works on macOS (Apple Silicon and Intel) and Linux x86_64. [Release archives](https://github.com/yungibly/pica/releases) contain the same binaries: extract one and put `pica` on your PATH. Linux needs glibc 2.35 or newer. There are no Linux ARM or Windows builds yet. All themes are built in; a Nerd Font is optional.

## Opening files

```sh
pica notes.md todo.md      # one tab each; the first is active
pica +120 src/main.rs      # open at line 120 (also: src/main.rs +120, +120:5)
pica src/main.rs:120:5     # line and column, when no file has that name
pica ~/projects            # browse a folder
git log | pica             # read piped input into an unsaved buffer
pica -                     # read standard input explicitly
```

With no arguments, `pica` opens an untitled Markdown buffer. A missing file opens empty and is created when you save. `pica` works as `$EDITOR` and `$VISUAL`.

## Keys

The defaults follow micro's. Press **Ctrl+G** for help and **Ctrl+E** for every command with its key; the status line always shows both.

| Key | Action | Key | Action |
| --- | --- | --- | --- |
| Ctrl+S | Save | Ctrl+F | Find (Ctrl+N / Ctrl+P: next / previous) |
| Ctrl+O | Open a file | Ctrl+R in Find | Replace |
| Ctrl+T | New tab | Ctrl+L | Go to line |
| Alt+, / Alt+. | Previous / next tab | Ctrl+K | Cut line; repeat to collect more |
| Ctrl+Q | Close tab; the last one quits | Ctrl+D | Duplicate line or selection |
| Ctrl+Z / Ctrl+Y | Undo / redo | Ctrl+/ or Alt+/ | Comment or uncomment |
| Ctrl+C / X / V | Copy / cut / paste | Alt+↑ / Alt+↓ | Move lines |
| Ctrl+A | Select all | Home | Line text, then line start |
| Ctrl+R | Line numbers (code) or live/source view (Markdown) | Ctrl+B / Alt+I / Alt+T | Bold / italic / toggle task (Markdown) |

Enter keeps the current line's indentation and continues Markdown lists. Tab indents the way the file already does: a tab in a tab-indented file, otherwise spaces to the file's own width, so a Python or YAML file never gains a stray tab. Shift+Tab and Backspace take a level back out, and with a selection Tab indents the selected lines. In a Markdown list, Tab and Shift+Tab nest an item under the one above or lift it out. [The full key reference](docs/keys.md) lists every key, the differences from micro, and terminal quirks. The mouse places the caret and drags select; a double click selects a word and a triple click a line. Function keys from earlier versions still work but are not shown.

## Markdown

Markdown opens in a live view. Headings carry a mark in the margin, tables become aligned grids, fenced code and front matter become labelled, highlighted surfaces, `> [!NOTE]` alerts become callouts, and task boxes can be clicked. Bold, italic, inline code, links, and images show their text without their markers, so a badge reads as its label. The block holding the caret always shows its exact source, so you edit what is in the file and nothing else changes. Ctrl+R switches to the plain numbered source view. pica remembers that choice for later Markdown files.

Lists continue on Enter; Enter on an empty item leaves the list. Tab nests an item under the one above it and Shift+Tab lifts it back out, with its own lines and children. Clicking a task box, or pressing Alt+T, changes only its `[ ]` / `[x]` marker. From Ctrl+E you can open an outline sidebar of the document's headings (it stays open once chosen), jump to any heading, or switch documents by fuzzy name. Footnotes, math, raw HTML, and other unsupported constructs stay visible as source.

Code files and plain text open as numbered source with highlighting for 41 languages, including Rust, Python, JavaScript, TypeScript, Go, C and C++, shell, SQL, HTML, CSS, JSON, YAML, TOML, and diffs. The highlighter is small and built in: it knows comments, strings, keywords, and literals, not full grammars.

## Looks

The default **Terminal** theme uses your terminal's own background, foreground, and colors. Selections and bars use reverse video, as nano draws them, so pica looks at home in light and dark terminals alike. **624 other themes**, including Catppuccin, Dracula, Nord, Gruvbox, Solarized, and Tokyo Night, are built in. Press Ctrl+E, choose **Choose theme**, and arrow through them to preview; Enter keeps one. `pica --list-themes` lists them, and `--theme NAME` picks one for a single launch. [Theme provenance and licenses](third_party/iterm2-themes/README.md) accompany the catalog.

A lone document has no tab row: just the text and one status line naming the file, its position, kind, and size. Tabs appear when a second document opens. Indent guides for code are off until you choose them from Ctrl+E. **Nerd Font icons** are on by default in Ghostty, Kitty, and WezTerm outside tmux, and off elsewhere. Toggle them from Ctrl+E, or launch with `--icons nerd` or `--icons plain`.

Theme, icon, outline, guide, indentation, and view choices are saved to `$XDG_CONFIG_HOME/pica/settings.conf` (by default `~/.config/pica/settings.conf`). `--no-state` disables preferences and recovery.

## Files and safety

- **Exact bytes.** Opening, viewing, and saving never reformat a file. Line endings, a BOM, and trailing whitespace are kept outside your edits.
- **Changed on disk.** The open file is checked about once a second. Reload from Ctrl+E; reload is undoable, and a changed disk version is never silently overwritten.
- **Careful saves.** Saves are atomic and keep the file's permissions, group, and (on macOS) extended attributes. A file opened through a symbolic link, as dotfile managers create, is saved to the file the link pointed to when opened. pica refuses saves if that link has moved, if the file has other hard links, if it belongs to another user, or if you cannot write it; Save as stays available.
- **Speed.** Typing stays around a millisecond or two whatever the file's size: only what is on screen is laid out and highlighted. Files over 256 KiB open in a plain source mode without Markdown rendering.
- **Limits.** Files must be UTF-8 text without NUL bytes, up to 8 MiB. Hostile Markdown, such as deep nesting or runaway emphasis, cannot stall the editor. Control and bidirectional characters are shown as symbols, never sent to the terminal.
- **Crash recovery.** Unsaved work is checkpointed about once a second to `$XDG_STATE_HOME/pica/recovery`. After an interrupted session, the next launch says how many documents can be recovered, and Ctrl+E → **Recover documents** reopens each copy as a new unsaved tab. Recovery never writes over your files.
- **Clipboard.** Ctrl+C/X/V use the system clipboard locally and an internal one over SSH or when the system clipboard fails, with a notice. Your terminal's own paste always works and is inserted literally. OSC 52 and native Wayland clipboard support are not implemented.

## Development

Building needs **Rust 1.89 or newer**.

```sh
cargo build --locked
cargo test --locked
cargo run --locked -- --snapshot --snapshot-size 100x30 README.md   # headless render
python3 tools/bench/typing.py --help                               # keystroke latency
```

- [Real-terminal test runner](tools/tui-test/README.md): drives the binary through a PTY and captures cells and PNGs; it runs on macOS and Linux.
- [Release guide](docs/releases.md): CI, archives, and the Homebrew tap.
- [Key reference](docs/keys.md) and [naming](docs/naming-shortlist.md).
- Earlier plans: [product](docs/product-plan.md), [technical](docs/technical-plan.md), [visual testing](docs/visual-testing-plan.md), and [checks](docs/prototype-checks.md).

pica was called Eymi through version 0.2.1. Its code is [MIT licensed](LICENSE); bundled themes and Rust dependencies keep their own notices in release archives.
