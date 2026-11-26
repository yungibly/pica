# Keys

pica's default keys are micro-aligned: someone who uses micro with its
default settings should find the same keys doing the same things. It is not a
clone of micro, and the differences are listed at the end. Ctrl+G shows this
reference inside the editor, and Ctrl+E opens a searchable list of every
command with its key.

## Files and tabs

| Key | Action |
| --- | --- |
| Ctrl+S | Save |
| Ctrl+O | Open a file in a new tab (file browser) |
| Ctrl+T | New untitled Markdown tab |
| Alt+, / Alt+. | Previous / next tab |
| Ctrl+PageUp / Ctrl+PageDown | Previous / next tab |
| Ctrl+Q | Close the current tab, asking first if it has unsaved changes. Closing the last tab quits. |
| Ctrl+E, then "Quit pica" | Quit, checking every unsaved tab |
| Ctrl+E, then "Save as" | Save under another name or in another place; see below |
| Ctrl+E, then "Reload" | Reload the file from disk. A file that changes while you have no unsaved edits reloads by itself. |

## Save as

Ctrl+S in a document that has no file yet, or "Save as" from Ctrl+E, asks
for a path. It starts from the document's own path with the name selected,
so typing renames it in the same folder; an untitled document starts in the
working folder.

| Key | Action |
| --- | --- |
| Enter | Save. A new name saves at once. |
| Tab | Complete the folder or name from what is on disk. When several match, they are listed and the path is completed as far as they agree. |
| Y / N | Answer when pica asks before replacing an existing file or creating a missing folder. N goes back to the path. |
| Left / Right, Home / End, Shift with them | Move and select in the path; Ctrl+Left / Ctrl+Right move by word |
| Ctrl+A, Ctrl+U | Select the whole path; clear it |
| Ctrl+C / Ctrl+X / Ctrl+V, Ctrl+Z / Ctrl+Y | Copy, cut and paste; undo and redo, in the path |
| Esc | Cancel |

`~` is your home folder. A path that starts with `/` or `~`, pasted, or
typed over the selected name, replaces the whole path. Saving onto a file
asks first and then replaces it atomically, keeping its permissions. pica
never asks about, and never replaces, a folder, a file with other hard
links, a read-only file or another user's file: choose another name for
those. Saving a document onto its own file is an ordinary save, unless the
file changed on disk since you opened it; then Save as asks whether to
replace the version on disk.

## File browser

Ctrl+O lists the folder of the current file: folders first, then text files.

| Key | Action |
| --- | --- |
| Letters and digits | Narrow the listing to matching names. Case does not matter, and letters need only appear in order: `ntm` finds `notes.md`. |
| Up / Down, PageUp / PageDown, Home / End | Choose among the names shown |
| Enter | Open the chosen file, or go into the chosen folder |
| / | After a filter, go into the folder it chose, so `sr/ap` reads like a path. With nothing typed, start a path from the root. |
| ~ | With nothing typed, start a path from your home folder |
| Backspace | Delete from the filter; when it is empty, go up to the parent folder |
| Ctrl+U | Clear the filter |
| Tab | Switch between the listing and the path field, where a whole path can be typed, pasted or edited |
| Alt+H | Show or hide hidden (dot) files. A filter that starts with a dot finds them either way. |
| Alt+A | Show every file, or only those that look like text |
| Esc or Ctrl+Q | Close the browser |

The folder and what you have typed read as one path at the top. The same
controls are along the second row for the mouse.

## Editing

| Key | Action |
| --- | --- |
| Ctrl+Z / Ctrl+Y | Undo / redo (Ctrl+Shift+Z also redoes) |
| Ctrl+C / Ctrl+X / Ctrl+V | Copy / cut / paste the selection |
| Ctrl+K | Cut the current line, or every line the selection touches. Pressing it again right away adds the next line to the same clipboard entry; each cut is still its own undo step. |
| Ctrl+D | Duplicate the selection, or the current line when nothing is selected |
| Ctrl+/ or Alt+/ | Comment or uncomment the current or selected lines |
| Tab | Indent to the next stop, the way the file is indented; with a selection, indent the selected lines |
| Shift+Tab | Outdent the current or selected lines |
| Ctrl+] / Ctrl+[ | Indent / outdent lines |
| Alt+Up / Alt+Down | Move lines up / down |
| Alt+Shift+Up / Alt+Shift+Down | Duplicate lines above / below |
| Enter | New line that keeps the current line's indentation |
| Alt+Enter or Shift+Enter | Plain new line, without indentation or list continuation |
| Backspace / Delete | Delete a character; Backspace in a line's leading spaces returns to the previous indent stop |
| Ctrl+Backspace / Ctrl+Delete | Delete a word (Alt also works) |

Enter copies the spaces and tabs at the start of the current line, up to the
caret, in every kind of file, including fenced code in Markdown. A line left
holding only that indentation is cleared, as micro does, so no trailing
whitespace stays behind. New lines use the line ending around them (LF, CRLF
or CR).

Tab follows the file. A file whose lines start with tabs gets a tab; a file
indented with spaces gets spaces to its own width, read from the most common
step between its lines, so Tab never mixes the two. A file with no
indentation yet uses its type's convention: tabs for Makefiles, Go and
tab-separated values, two spaces for YAML, JSON, JavaScript, TypeScript, HTML,
CSS, Ruby and a few others, and four spaces otherwise. Tab characters are
drawn four columns wide; **Tab width** in Ctrl+E changes that to two or
eight, in every file. To use one style in every file, choose
**Indentation: tabs or spaces** from Ctrl+E: tabs, or two, four or eight
spaces; "Follow each file" returns to the default. Indenting a selection
leaves its empty lines empty.

Comments use the file's language: `//` for Rust, C, Go, JavaScript and
similar, `#` for Python, shell, YAML, TOML and plain text, `--` for SQL, Lua
and Haskell, and `/* */` for CSS. Markdown and HTML use `<!-- -->`; inside a
Markdown code fence the fence's language decides, and front matter uses `#`.
If every selected non-blank line is already commented, the markers are
removed; otherwise every non-blank line is commented at the block's smallest
indentation. The whole change is one undo step.

## Moving and selecting

| Key | Action |
| --- | --- |
| Arrows | Move |
| Shift+arrows | Extend the selection |
| Ctrl+Left / Ctrl+Right | Move by word (Alt also works) |
| Home | First non-blank character of the line; press again for the line start |
| Shift+Home | Extend the selection the same way |
| End | End of the row |
| Ctrl+Home / Ctrl+End | Start / end of the file |
| PageUp / PageDown | Scroll a screen. The caret moves with the view and keeps its row on screen; at either end of the file it goes on to the first or last line. |
| Ctrl+L | Go to a line, or `line:column` |
| Ctrl+A | Select all |
| Click, Shift+click, drag | Place the caret, extend the selection to the pointer, select |
| Double click | Select the word; dragging on extends word by word |
| Triple click | Select the whole line; dragging on extends line by line |

A double click keeps `can't` and `3.14` whole and stops at the dots and
colons between names, so it selects `name` in `self.name`. On a space or a
punctuation mark it selects that run. In the live Markdown view the first
click may reveal a block's source and move its text; the second click still
selects the word first clicked.

## Finding and replacing

| Key | Action |
| --- | --- |
| Ctrl+F | Find, ignoring letter case. The selection, if any, becomes the query. |
| Alt+C, in Find | Match case, or ignore it again; the **Aa** toggle beside the count does the same |
| Ctrl+N / Ctrl+P | Next / previous match. With Find closed, they reuse the last query; with no query yet, they open Find. |
| Enter / Shift+Enter | Next / previous match, in Find |
| Ctrl+R, in Find | Expand Find to Replace |
| Tab | Switch between the Find and With fields |
| Enter, in With | Replace this match and go to the next |
| Alt+R / Alt+A | Replace one / replace all |
| Esc | Close Find |

Find matches text literally and ignores letter case: `pica` finds `Pica` and
`PICA`, and `école` finds `École`. Alt+C switches to an exact match and back.
The choice applies to Replace as well, holds for every document, and is
remembered. Find has no regular expressions or whole-word matching.

## Markdown

| Key | Action |
| --- | --- |
| Ctrl+R | Switch between the live view and the source view |
| Ctrl+B | Bold |
| Alt+I | Italic (Ctrl+I in terminals with the enhanced keyboard protocol) |
| Alt+` | Inline code |
| Alt+T | Toggle the task under the caret |
| Enter | Continue a list item or task; on an empty item, leave the list |
| Tab / Shift+Tab | On a list item's first line, nest it under the item above / lift it out a level |

Ctrl+B and Alt+T do nothing in other kinds of file.

Nesting moves the item's continuation lines and children with it and lines
it up under the text of the item above. A numbered item takes the number its
new place needs (Markdown only lets a numbered list begin inside another item
at 1), and numbered items it gains as children are renumbered to follow.
Tab on the first item of a list does nothing, since there is nothing to nest
it under. With a selection, on other lines, and in quoted lists, Tab indents
as it does everywhere else.

## View and commands

| Key | Action |
| --- | --- |
| Ctrl+E | Command palette. Type to filter; a bare number such as `42` or `42:7` goes to that line. |
| Ctrl+G | This reference; Ctrl+G or Esc closes it |
| Ctrl+R | Line numbers on or off (micro's ruler) in code and plain text |
| Alt+Z | Word wrap on or off |

Long lines wrap between words, and only a word longer than a row is broken
inside. With word wrap off, code, plain text and Markdown's source view keep
every line on one row: the view scrolls sideways to follow the caret, a `›`
at the right edge or a `‹` beside the line number marks a line that
continues out of view, and the line numbers stay where they are. Home and
End then go to the ends of the whole line. A sideways wheel, or Shift with
the ordinary wheel, scrolls along the lines without moving the caret. The
Markdown live view always wraps. The choice applies to every tab and is
remembered; `--no-wrap` turns wrap off for one launch. A line longer than
4,000 columns still continues on the next row, which keeps minified and
generated files quick to edit.

The palette also holds the commands without a key of their own: Save as,
Reload, Quit pica (all tabs), the outline sidebar, Switch document, Go to
heading, themes, icons, indentation, tab width and document recovery. In the palette, pickers and
the go-to-line prompt, Esc or Ctrl+Q closes them without doing anything.

The outline sidebar is a preference: once turned on, it shows beside every
Markdown document that has at least one heading, in a window at least 60
columns wide, and nowhere else. Code, plain text and Markdown without
headings keep the whole window. While you type, it waits for a pause before
it appears or goes, so a heading half written does not move the text.

The status line shows two kinds of message. A notice, such as "Saved" or
the count of documents to recover at launch, goes away with the next
keypress. A problem, such as a failed save, is highlighted and stays while
you work until Esc dismisses it; the status line says so.

## Your own keys

pica reads `$XDG_CONFIG_HOME/pica/keys.conf` (by default
`~/.config/pica/keys.conf`) when it starts, if the file exists, and never
writes to it. Each line gives one key a new job:

```
# comments start with a hash
ctrl+j = duplicate      # a command, by the names below
alt+w  = ctrl+q         # whatever another key does by default
ctrl+k = none           # nothing at all
```

A key is any of `ctrl`, `alt` and `shift`, then one key: a letter, digit or
symbol, `f1` to `f12`, or `enter`, `tab`, `space`, `backspace`, `delete`,
`up`, `down`, `left`, `right`, `home`, `end`, `pageup` or `pagedown`. A key
you bind needs Ctrl or Alt, or must be a function key, so typing and plain
movement always work. If a key appears twice, the later line wins.

A default key keeps its job until you give that key another one. Your keys
apply in the text and in Find, not while the command palette, a picker, the
file browser or a prompt has the keyboard. Ctrl+E shows every command with
the key it has now. Lines pica cannot use are skipped and named in the
status line at startup; `--no-state` ignores the file. The terminal notes
below apply to your keys too: some combinations never reach a program.

| Name | Command |
| --- | --- |
| `new-tab` | New tab |
| `open` | Open file |
| `save` | Save document |
| `save-as` | Save as |
| `close-tab` | Close tab |
| `find` | Find in document |
| `find-next` | Find next match |
| `find-previous` | Find previous match |
| `replace` | Find and replace |
| `toggle-view` | Toggle live / source view |
| `toggle-line-numbers` | Toggle line numbers |
| `toggle-word-wrap` | Toggle word wrap |
| `toggle-outline` | Toggle outline sidebar |
| `next-tab` | Next tab |
| `previous-tab` | Previous tab |
| `go-to-line` | Go to line |
| `switch-document` | Switch document |
| `go-to-heading` | Go to heading |
| `help` | Help / keyboard shortcuts |
| `quit` | Quit pica (all tabs) |
| `undo` | Undo edit |
| `redo` | Redo edit |
| `cut-line` | Cut line |
| `duplicate` | Duplicate line or selection |
| `toggle-comment` | Toggle comment |
| `bold` | Format bold |
| `italic` | Format italic |
| `inline-code` | Format inline code |
| `toggle-task` | Toggle task |
| `indent` | Indent selected lines |
| `outdent` | Outdent selected lines |
| `move-lines-up` | Move lines up |
| `move-lines-down` | Move lines down |
| `duplicate-lines-up` | Duplicate lines above |
| `duplicate-lines-down` | Duplicate lines below |
| `choose-theme` | Choose theme |
| `toggle-icons` | Toggle Nerd Font icons |
| `toggle-indent-guides` | Toggle indent guides |
| `indentation` | Indentation: tabs or spaces |
| `tab-width` | Tab width |
| `reload` | Reload file from disk |
| `recover-documents` | Recover documents |

To bind something that has no command name, such as select all or paste,
point your key at its default key: `alt+a = ctrl+a`.

## Function keys

Function keys from earlier versions still work but are not shown anywhere:
F1 help, F2 commands, F3 / Shift+F3 next / previous match, F4 save as, F5
reload, F6 live or source view, F7 / F8 previous / next tab, F9 outline
sidebar, F10 switch document and F11 go to heading. Inside the file browser,
F2 and F5 are Alt+H and Alt+A. Nano's Ctrl+W also opens Find.

## Terminal notes

- Most terminals send Ctrl+/ as the same byte as Ctrl+_ and Ctrl+7, so all
  three toggle comments.
- Ctrl+] sends the same byte as Ctrl+5 in most terminals; both indent.
  Ctrl+[ is the same byte as Esc, so outdenting with it needs a terminal
  with the enhanced keyboard protocol (kitty, Ghostty, WezTerm, foot, recent
  iTerm2). Shift+Tab always works.
- Alt keys on macOS need the terminal's "Option as Meta" setting.
- Over SSH, Ctrl+C and Ctrl+X send the copy to your terminal with OSC 52,
  so it reaches the clipboard of the machine you are sitting at. Most
  current terminals take it (kitty, Ghostty, WezTerm, iTerm2, Alacritty,
  foot, Windows Terminal, xterm when allowed); Apple's Terminal does not.
  Inside tmux it needs `set -g set-clipboard on`. Copies over 75,000
  bytes stay inside pica, since terminals drop longer ones. Ctrl+V pastes
  what pica last copied; to paste the system clipboard over SSH, use the
  terminal's own paste. pica never asks the terminal to read the clipboard.
- On a Wayland desktop the system clipboard needs `wl-copy` and `wl-paste`
  from wl-clipboard. Without them pica uses X11 through XWayland where
  that exists, and otherwise the terminal as over SSH.
- Ctrl+Shift+S (Save as) only differs from Ctrl+S with the enhanced
  keyboard protocol; the palette's Save as works everywhere.

## Command line

```sh
pica notes.md todo.md      # one tab each; the first is active
pica +120 src/app.rs       # open at line 120 (also: src/app.rs +120)
pica src/app.rs:120:5      # line and column, when no file has that name
pica ~/projects            # open the file browser in a folder
git log | pica             # read piped input into an unsaved buffer
pica -                     # read standard input explicitly
pica --no-wrap server.log  # keep each line on one row for this launch
```

Saving a file opened through a symbolic link (as dotfile managers create)
replaces the file the link pointed to when it was opened, keeping its
permissions and the link. pica refuses to save if the link now points
somewhere else, if the file has other hard links, or if saving would hand
another user's file to you.

## Differences from micro

- Ctrl+E opens a command palette rather than micro's command line. It lists
  every command with its key; a number goes to that line.
- Ctrl+B makes text bold in Markdown instead of opening a shell prompt.
- Ctrl+W opens Find, as in nano; pica has no splits.
- Ctrl+C and Ctrl+X act on the selection only; use Ctrl+K to cut a line.
- Alt+Shift+Up / Down duplicate lines; pica has no multiple cursors.
- Ctrl+R toggles the live view in Markdown files, where the source view is
  the numbered one.
- Tab follows each file's own indentation, where micro uses its `tabstospaces`
  and `tabsize` options.
- Find matches literal text and ignores case until Alt+C asks for an exact
  match; micro's find takes a regular expression.
- Long lines wrap until Alt+Z turns wrapping off; micro leaves them
  unwrapped until its `softwrap` option is set.
- There are no macros (Ctrl+U, Ctrl+J). Keys are rebound in `keys.conf`,
  described above, not in micro's `bindings.json`.
