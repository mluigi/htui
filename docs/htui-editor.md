# htui's external editor: `$VISUAL`, `$EDITOR` and the in-pane editor

Wherever htui lets you write a longer text, you can hand it to your own editor instead: `E` on a
selected template or library entry, and `Ctrl+E` while you edit a Templates or Library draft, a
field of the Backlog item form, or the Notes and Documents compose box. htui writes the text to a
temporary file, runs the editor on it, and reads the file back into the view when the editor
exits. The view then holds the edited text, **unsaved**: `Ctrl+S` saves it as usual. This is
MOD-9 (the handoff) and MOD-57 (the in-pane editor).

By default htui gives the whole terminal to the editor and takes it back when the editor exits
([the suspended editor](#the-suspended-editor)). With `HTUI_EDITOR_PANE` set, the editor runs
inside htui instead, drawn over the view that asked for it
([the in-pane editor](#the-in-pane-editor)).

- [Choosing the editor](#choosing-the-editor)
- [What comes back](#what-comes-back)
- [GUI editors](#gui-editors)
- [The suspended editor](#the-suspended-editor)
- [The in-pane editor](#the-in-pane-editor)
- [The keys](#the-keys)
- [While the editor is open (the M1 lock)](#while-the-editor-is-open-the-m1-lock)
- [Where the pane draws](#where-the-pane-draws)
- [Limits](#limits)
- [Windows](#windows)

## Choosing the editor

htui runs the first of these that is set and not blank:

1. `$VISUAL`
2. `$EDITOR`
3. `vi` (Linux and macOS) or `notepad` (Windows)

The value is a command and its arguments, as you would type them: `nvim`, `vim -u NONE`,
`code --wait`. On Linux and macOS htui runs it as `sh -c 'exec <value> "$1"' htui-editor <file>`,
so `sh` reads the value the way your shell would (git runs `$EDITOR` the same way) and the file is
passed as an argument, never pasted into the command. The `exec` makes the editor htui's own
child, which is what lets `Ctrl+C` reach the editor and lets htui end it. It also means the value
must be **one command**:

- `VAR=x nvim` (a leading assignment) does not start.
- `a; b` and `a && b` run only `a`, and without the file: `exec` replaces the shell, so `b`
  (which the file was appended to) never runs.
- `a | b` runs both sides, and only `b` gets the file.

Wrap anything more in a small script and point `$VISUAL` at the script.

The temporary file is `htui-<name>-<random>.md` in the system temp directory (`<name>` is the
template's or field's name, shortened and made file-safe). It is removed when the edit ends, on
every path: saved, unchanged, failed or aborted.

## What comes back

| The editor | The view shows |
|---|---|
| exited 0 and the file changed | the edited text, unsaved (Templates, Library and the item form say `edited in $EDITOR — Ctrl+S saves`) |
| exited 0 and the file is the same | `no changes` |
| the same, within one second | ``no changes — a GUI editor needs its wait flag, e.g. `code --wait` `` ([GUI editors](#gui-editors)) |
| exited with a non-zero code, or was killed by a signal | `` `<value>` exited with <code>; nothing was changed `` |
| could not be found or started | ``could not start `<value>` (…) — set $VISUAL or $EDITOR``, or ``no $VISUAL or $EDITOR is set and `vi` could not start (…)`` |
| was aborted ([the keys](#the-keys)) | `the editor was aborted; nothing was changed` |
| in the pane: started, but htui could not wait on it (an operating-system error) | ``lost track of `<value>` (…); nothing was changed`` |

Line endings are normalised to `\n`. In the Backlog item form and the compose box, a field that had
no final newline does not get the one most editors add on save, and control characters (an escape
sequence a script left in the file) are dropped, as a paste's are.

## GUI editors

A GUI editor returns at once unless it is told to wait for the window, so htui reads back an
untouched file: `no changes — a GUI editor needs its wait flag`. Give it its wait flag:

| Editor | Value |
|---|---|
| VS Code | `code --wait` |
| Sublime Text | `subl --wait` |
| Zed | `zed --wait` |
| gedit | `gedit --wait` |

In the in-pane mode a GUI editor works the same way, but it draws nothing in the pane: the pane
stays empty while you edit in the editor's window, and the edit ends when you close the file there.

## The suspended editor

The default. htui leaves its screen, the editor gets the whole terminal, and htui comes back when
the editor exits. While the editor runs, `Ctrl+C` and `Ctrl+\` are the editor's: they cannot end
htui. Nothing else in htui runs on screen meanwhile (replies wait and are applied on return).

## The in-pane editor

Set `HTUI_EDITOR_PANE` to `1`, `true` or `yes` (any case) and the editor runs on a
pseudo-terminal inside htui, drawn over the view that asked for it, while htui keeps drawing
around it. Anything else (unset, blank, `0`) is the suspended editor.

```sh
export HTUI_EDITOR_PANE=1
export VISUAL=nvim
htui
```

- The setting is per machine (an environment variable, not a database row), and htui reads it at
  each edit, with `$VISUAL`/`$EDITOR`. It works with every backend, `--demo` and `--offline`
  included.
- The editor is the same command on the same file as in the suspended mode, in htui's working
  directory, with `TERM=xterm-256color` and the pane's size (`LINES` and `COLUMNS` are removed, so a
  stale value cannot override it).
- What comes back is read the same way ([What comes back](#what-comes-back)): the view cannot tell
  the two modes apart.
- A paste, like all input queued on its way to the editor, is zeroized in htui's memory once
  written (a paste may be a credential). That covers the input path only: the text you edit also
  sits in the temporary file and in the pane's screen grid, neither of which is zeroized.
- One editor at a time. A second `E`/`Ctrl+E` while one is open (from a view a reply opened, say)
  is answered `an editor is already open: return to it or abort it first`.

If the pane cannot start (no pseudo-terminal available), the view says so and suggests unsetting
`HTUI_EDITOR_PANE`.

## The keys

| Key | When | Does |
|---|---|---|
| `Ctrl+\`, shown as `Ctrl+4` | always, except under a modal prompt ([below](#while-the-editor-is-open-the-m1-lock)) | gives htui the keys, and back to the editor |
| `Ctrl+x` | htui has the keys | aborts the edit: the editor is killed, nothing is read back |
| `q` | htui has the keys | quits htui at once; the editor is killed, the edit is lost |
| `Ctrl+C` | htui has the keys | quits htui, as everywhere in htui |
| `?` or `F1` | htui has the keys | the help box |

**While the editor has the keys, every key is the editor's**, `Ctrl+C` included (it is the
editor's own interrupt, as in a terminal: vim cancels, nano shows the cursor position), `Ctrl+x`
included (nano's exit). The one exception is `editor.focus`, whose default is `Ctrl+4`.

On Linux and macOS terminals, pressing `Ctrl+\` sends the same byte as `Ctrl+4`, so htui sees
`Ctrl+4` and labels the key that way; press whichever is easier. **This alias is unix-only:** on
Windows `Ctrl+\` arrives as itself, so there only a physical `Ctrl+4` toggles the focus (see
[Windows](#windows)). The pane's title and the status line always name the key in effect:

| | Pane title | Status line |
|---|---|---|
| the editor has the keys | `<value> · Ctrl+4 to htui` | `the editor has the keys · Ctrl+4 to htui` |
| htui has the keys | `<value> · htui has the keys` | `Ctrl+4 to the editor · Ctrl+x abort · q quit · ? help` |

Both keys are generated from htui's key catalogue (the `editor` context: `editor.focus` and
`editor.abort`), so the labels follow a rebinding. Rebinding them through `keys.toml` arrives with
MOD-67's user keymap; `editor.focus` will have to stay bound.

## While the editor is open (the M1 lock)

In this first version, while an editor is open and htui has the keys, htui answers **only** the
keys in the table above: `Ctrl+4`, `Ctrl+x`, `q`, `?`, `F1` and `Ctrl+C`. Every other key is refused
with `the editor is open: Ctrl+4 to the editor · Ctrl+x abort`, and nothing in the view, the tab
bar or an overlay moves. A paste goes to the editor while it has the keys (as a bracketed paste
when the editor asked for one); while htui has the keys it is dropped, unless a modal prompt is
open, which takes it as it takes the keys.

- `q` and `Ctrl+C` quit at once, without a confirmation, and kill the editor: an unsaved edit is
  lost. Leave the editor normally (`:wq`, `Ctrl+X`) to keep it.
- A modal prompt that a store reply opens during the edit (a migration prompt, say) takes htui's
  keys: the status line reads `the editor is open: answer or close the prompt first`. Keys and
  pastes go to the prompt, `Ctrl+4` included (`Ctrl+C` still quits). Answer or close it; then the
  lock is back. The first key you type for the editor when such a prompt appears is swallowed
  rather than answering a prompt you had not seen yet.
- The editor stays with the tab that asked. Should a reply move htui to another tab, the first
  key typed for the editor is swallowed the same way, and `Ctrl+4` brings the editor's tab back
  with the keys.

A later milestone (MOD-57 M2) opens the rest of htui while an edit is open: tabs, views and
overlays, with the editor waiting in its pane.

## Where the pane draws

The pane has a one-line title rule and the editor below it. It goes over:

- **the view's editing area**: the Templates or Library draft's text (the view keeps showing the
  draft, with the hint `the draft is in $EDITOR`), the Notes or Documents compose box, or the
  item form's field that was handed out; otherwise
- **the whole tab body**: when that area is smaller than 40 columns by 8 rows (the item form's
  three-row Paths field, a small terminal), or when the edit came from a list (`E` on a template
  or library entry, where no draft is drawn).

The pane is drawn only while its tab is on screen. The editor follows the pane's size: resizing
the terminal resizes the editor (it gets `SIGWINCH`) right after the next frame.

## Limits

- **Mouse.** While an editor is open htui does not capture the mouse, so the terminal's own
  selection works (over the whole screen, not only the pane); the editor gets no mouse events.
- **No scrollback.** The pane shows the editor's current screen only.
- **The cursor's shape and the window title** are the terminal's: the editor's cursor-shape and
  title sequences are not passed on. The cursor shows only while the editor has the keys.
- **Terminal queries.** htui answers the cursor-position (DSR) and device-attributes (DA1)
  queries. Others, such as the secondary device attributes and colour queries nvim sends at
  start, go unanswered; editors carry on without them.
- **Abort and quit leave the editor's recovery files.** An abort or quit hangs the editor up
  (`SIGHUP`, then `SIGKILL` to its process group after a short grace). nano saves a modified
  buffer to `htui-*.md.save` and vim keeps its swap file `.htui-*.md.swp`, both in the temp
  directory; nvim keeps its swap under `$XDG_STATE_HOME/nvim/swap/` (usually
  `~/.local/state/nvim/swap/`), named after the file's full path. htui does not delete them: they
  hold your text.
- **A fast quit** may leave an editor that ignores `SIGHUP` running, if htui exits within the
  grace before `SIGKILL`.
- **An editor that exits while you abort** loses the race: the abort wins, and what it saved is
  not read back.
- **An editor that detaches a helper** into a session of its own (gvim without `-f`, a wrapper
  that daemonises) and leaves it on the terminal puts that helper out of reach of the pane's
  `SIGHUP` and `SIGKILL`. The pane still closes when the editor exits, and the file is read back,
  but htui keeps one idle reader thread and the pseudo-terminal open until the helper exits or
  htui quits.

## Windows

The in-pane editor compiles for Windows (ConPTY), and htui's checks cross-compile it, but it has
not been run on Windows yet; that verification belongs to MOD-16. Known differences:

- Only a physical `Ctrl+4` toggles the focus; `Ctrl+\` is not an alias there.
- The editor runs as `cmd /S /C "<value> <file name>"` from the temp directory. A value that
  holds a `"` is not expected to work.
- `cmd` has no `exec`: aborting or quitting ends `cmd`, and whether the editor it started ends with
  it is on MOD-16's list (the suspended mode has the same caveat).
