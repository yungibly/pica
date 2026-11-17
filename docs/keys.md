# Keys

milli's default keys are micro-aligned: someone who uses micro with its
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
| Ctrl+E, then "Quit milli" | Quit, checking every unsaved tab |
| Ctrl+E, then "Save as" | Save to a new filename |
| Ctrl+E, then "Reload" | Reload the file from disk |

## Editing

| Key | Action |
| --- | --- |
| Ctrl+Z / Ctrl+Y | Undo / redo (Ctrl+Shift+Z also redoes) |
| Ctrl+C / Ctrl+X / Ctrl+V | Copy / cut / paste the selection |
| Ctrl+K | Cut the current line, or every line the selection touches. Pressing it again right away adds the next line to the same clipboard entry; each cut is still its own undo step. |
| Ctrl+D | Duplicate the selection, or the current line when nothing is selected |
| Ctrl+/ or Alt+/ | Comment or uncomment the current or selected lines |
| Tab | Insert a tab; with a selection, indent the selected lines |
| Shift+Tab | Outdent the current or selected lines |
| Ctrl+] / Ctrl+[ | Indent / outdent lines |
| Alt+Up / Alt+Down | Move lines up / down |
| Alt+Shift+Up / Alt+Shift+Down | Duplicate lines above / below |
| Enter | New line that keeps the current line's indentation |
| Alt+Enter or Shift+Enter | Plain new line, without indentation or list continuation |
| Backspace / Delete | Delete a character |
| Ctrl+Backspace / Ctrl+Delete | Delete a word (Alt also works) |

Enter copies the spaces and tabs at the start of the current line, up to the
caret, in every kind of file, including fenced code in Markdown. A line left
holding only that indentation is cleared, as micro does, so no trailing
whitespace stays behind. New lines use the line ending around them (LF, CRLF
or CR).

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
| PageUp / PageDown | Move a screen |
| Ctrl+L | Go to a line, or `line:column` |
| Ctrl+A | Select all |

## Finding and replacing

| Key | Action |
| --- | --- |
| Ctrl+F | Find. The selection, if any, becomes the query. |
| Ctrl+N / Ctrl+P | Next / previous match. With Find closed, they reuse the last query; with no query yet, they open Find. |
| Enter / Shift+Enter | Next / previous match, in Find |
| Ctrl+R, in Find | Expand Find to Replace |
| Tab | Switch between the Find and With fields |
| Enter, in With | Replace this match and go to the next |
| Alt+R / Alt+A | Replace one / replace all |
| Esc | Close Find |

## Markdown

| Key | Action |
| --- | --- |
| Ctrl+R | Switch between the live view and the source view |
| Ctrl+B | Bold |
| Alt+I | Italic (Ctrl+I in terminals with the enhanced keyboard protocol) |
| Alt+` | Inline code |
| Alt+T | Toggle the task under the caret |
| Enter | Continue a list item or task; on an empty item, leave the list |

Ctrl+B and Alt+T do nothing in other kinds of file.

## View and commands

| Key | Action |
| --- | --- |
| Ctrl+E | Command palette. Type to filter; a bare number such as `42` or `42:7` goes to that line. |
| Ctrl+G | This reference; Ctrl+G or Esc closes it |
| Ctrl+R | Line numbers on or off (micro's ruler) in code and plain text |

The palette also holds the commands without a key of their own: Save as,
Reload, Quit milli (all tabs), the outline sidebar, Switch document, Go to
heading, themes, icons and document recovery. In the palette, pickers and
the go-to-line prompt, Esc or Ctrl+Q closes them without doing anything.

## Function keys

Function keys from earlier versions still work but are not shown anywhere:
F1 help, F2 commands, F3 / Shift+F3 next / previous match, F4 save as, F5
reload, F6 live or source view, F7 / F8 previous / next tab, F9 outline
sidebar, F10 switch document and F11 go to heading. Nano's Ctrl+W also opens
Find.

## Terminal notes

- Most terminals send Ctrl+/ as the same byte as Ctrl+_ and Ctrl+7, so all
  three toggle comments.
- Ctrl+] sends the same byte as Ctrl+5 in most terminals; both indent.
  Ctrl+[ is the same byte as Esc, so outdenting with it needs a terminal
  with the enhanced keyboard protocol (kitty, Ghostty, WezTerm, foot, recent
  iTerm2). Shift+Tab always works.
- Alt keys on macOS need the terminal's "Option as Meta" setting.
- Ctrl+Shift+S (Save as) only differs from Ctrl+S with the enhanced
  keyboard protocol; the palette's Save as works everywhere.

## Command line

```sh
milli notes.md todo.md      # one tab each; the first is active
milli +120 src/app.rs       # open at line 120 (also: src/app.rs +120)
milli src/app.rs:120:5      # line and column, when no file has that name
milli ~/projects            # open the file browser in a folder
git log | milli             # read piped input into an unsaved buffer
milli -                     # read standard input explicitly
```

Saving a file opened through a symbolic link (as dotfile managers create)
replaces the file the link pointed to when it was opened, keeping its
permissions and the link. milli refuses to save if the link now points
somewhere else, if the file has other hard links, or if saving would hand
another user's file to you.

## Differences from micro

- Ctrl+E opens a command palette rather than micro's command line. It lists
  every command with its key; a number goes to that line.
- Ctrl+B makes text bold in Markdown instead of opening a shell prompt.
- Ctrl+W opens Find, as in nano; milli has no splits.
- Ctrl+C and Ctrl+X act on the selection only; use Ctrl+K to cut a line.
- Alt+Shift+Up / Down duplicate lines; milli has no multiple cursors.
- Ctrl+R toggles the live view in Markdown files, where the source view is
  the numbered one.
- There are no macros (Ctrl+U, Ctrl+J), and keys cannot be rebound yet.
