# htui's external editor: `$VISUAL`, `$EDITOR` and the in-pane editor

Wherever htui lets you write a longer text, you can hand it to your own editor instead: `E` on a
selected template or library entry, and `Ctrl+E` while you edit a Templates or Library draft, a
field of the Backlog item form, or the Notes and Documents compose box. htui writes the text to a
temporary file, runs the editor on it, and reads the file back into the view when the editor
exits. The view then holds the edited text, **unsaved**: `Ctrl+S` saves it as usual. This is
MOD-9 (the handoff) and MOD-57 (the in-pane editor).

By default htui gives the whole terminal to the editor and takes it back when the editor exits
([the suspended editor](#the-suspended-editor)). With `HTUI_EDITOR_PANE` set, the editor runs
inside htui instead, drawn over the view that asked for it, and the rest of htui stays usable
while it waits ([the in-pane editor](#the-in-pane-editor)).

- [Choosing the editor](#choosing-the-editor)
- [What comes back](#what-comes-back)
- [GUI editors](#gui-editors)
- [The suspended editor](#the-suspended-editor)
- [The in-pane editor](#the-in-pane-editor)
- [The keys](#the-keys)
- [While the editor is open](#while-the-editor-is-open)
- [The quit and abort questions](#the-quit-and-abort-questions)
- [The editor keys in `keys.toml`](#the-editor-keys-in-keystoml)
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
- One editor at a time. A second `E`/`Ctrl+E` while one is open (on another tab, say) is
  answered `an editor is already open: return to it or abort it first`.

If the pane cannot start (no pseudo-terminal available), the view says so and suggests unsetting
`HTUI_EDITOR_PANE`.

## The keys

| Key | Where | Does |
|---|---|---|
| `Ctrl+\`, shown as `Ctrl+4` | every tab, except under a modal pop-up ([below](#while-the-editor-is-open)) | gives htui the keys, and back to the editor; from another tab it brings the editor's tab back with the editor holding the keys |
| `Ctrl+x` | the editor's tab, htui has the keys | asks first; `Ctrl+x` again aborts the edit: the editor is killed, nothing is read back ([the abort question](#the-abort-question)) |
| `q` | every tab, htui has the keys | asks first; `y` quits htui, kills the editor and loses the edit ([the quit question](#the-quit-question)) |
| `Ctrl+C` | every tab, htui has the keys | quits htui at once, as everywhere in htui, under a question too; the editor is killed, the edit is lost |
| `?` or `F1` | htui has the keys | the help box, with the editor keys that act where you are |

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
| htui has the keys, on the editor's tab | `<value> · htui has the keys` | `Ctrl+4 to the editor · Ctrl+x abort · Tab/Shift+Tab switch tabs · q quit · ? help` |
| a modal pop-up over the editor's tab | `<value> · htui has the keys` | `the editor is open: close this first` |
| on another tab | (the pane is not drawn) | `editing in <Tab> · Ctrl+4 back · `, then that tab's own line |

`<Tab>` is the editor's tab as the tab bar names it: `Skills` for a Templates or Library edit,
`Backlog` for the item form, Notes and Documents. `Ctrl+4 back` is left out where it would not act:
under a modal pop-up, and while the next key is about to be swallowed
([below](#while-the-editor-is-open)). An error on the status line replaces the whole line, as it
does without an editor.

Both keys are generated from htui's key catalogue (the `editor` context: `editor.focus` and
`editor.abort`), so the labels follow a rebinding in `keys.toml`
([the editor keys in `keys.toml`](#the-editor-keys-in-keystoml)).

## While the editor is open

The editor waits in its pane, with the tab that asked for it; the rest of htui keeps working.

**The editor's tab stays locked.** On that tab, while htui has the keys, only the editor's keys
and the global keys act: `Ctrl+4`, `Ctrl+x`, `Tab`/`Shift+Tab` and `1` … `5` (switch tabs), `w`
(switch workspace), `Ctrl+F` (find), `Ctrl+W` (waiting), `Ctrl+Q` (queue), `q`, `?`/`F1` and
`Ctrl+C`, or the keys `keys.toml` gives them. Every other key is refused with
`the editor is open: Ctrl+4 to the editor · Ctrl+x abort` and never reaches the view: a `Ctrl+S`
there would save the text the editor has not handed back yet. A paste is dropped there; while the
editor has the keys it goes to the editor (as a bracketed paste when the editor asked for one).

**Every other tab is free.** Switch away and that tab works as it does without an editor: its
keys, pastes, pop-ups and mouse are its own, and `Ctrl+x` is that tab's key, if it has one (the
abort acts only on the editor's tab). `Ctrl+4` from any tab brings the editor's tab back with the
editor holding the keys (and closes the `?` box). Switching back by hand (`Tab`, a number key)
lands on the editor's tab with htui holding the keys; `Ctrl+4` then hands them to the editor.

**Nothing steers the editor's tab.** Two things that would reset or re-select it are refused
while an edit is open, with `the editor is open in <Tab>: finish or abort it first` on the status
line (the pop-up that asked closes; nothing else changes):

- a workspace switch: picking a workspace with `w`;
- opening a result in the editor's tab: `Enter` on an item, document or step in find, the queue
  or the waiting list, which opens it in Backlog, while the edit is in Backlog. A result that
  opens in another tab opens as usual.

**Settings › Hierarchy's own scope changes are refused too, and not retried.** Creating or deleting
a project there, or deleting the workspace htui is in, normally moves htui's scope with it. During
an edit that move is refused with the same sentence, and nothing repeats it when the edit ends:
htui stays on the old scope (the other tabs keep the old project list, or the deleted workspace)
until Settings › Hierarchy next re-reads its tree or you pick a workspace. After the edit, press
`w` and pick the workspace you are in (or, after deleting it, another one): every tab reloads.

**A modal pop-up over the editor's tab** takes htui's keys, whether a reply opened it (a migration
prompt, say), you did (find, the queue, the waiting list, the workspace switcher), or it is one of
the [questions](#the-quit-and-abort-questions). The status line reads
`the editor is open: close this first`. Keys and pastes go to the pop-up; `Ctrl+4` does not act
there (under the quit question it says `the editor is open: answer the question first`), and
`Ctrl+C` still quits. Answer or close it, and the tab is locked as before.

**The first key after a surprise is swallowed.** When a reply opens a modal prompt over the editor
or moves htui to another tab while the editor has the keys, the first key (or paste) you type for
the editor is swallowed rather than answering a prompt you had not seen or acting in a tab you did
not choose; htui has the keys from then on. The status line says
`the editor is open: close this first` under a prompt, and `the editor is open in <Tab>: Ctrl+4
back` on another tab, where the next keys are that tab's.

**The `?` box lists the editor keys that act where you are.** On the editor's tab it shows
`Editor: Ctrl+4 editor focus · Ctrl+x abort edit` and the global keys, and none of the view's (they
are refused). On another tab it shows `Editor: Ctrl+4 editor focus` above that tab's own keys.
Under a modal pop-up it shows the pop-up's keys only.

**The mouse** is the view's on every other tab, as without an editor. On the editor's tab htui
does not capture it ([Limits](#limits)).

## The quit and abort questions

### The quit question

While an editor is open, `q` (`global.quit`) asks first, from any tab:

```text
An edit is still open in the editor. Quit and lose it?
y quit · n/Esc keep editing
```

- `y` quits htui: the editor is killed and the edit is lost (its recovery files stay, see
  [Limits](#limits)).
- `n` or `Esc` close the question; the editor keeps waiting as it was.
- Any other key does nothing (a second `q` opens no second box); `Ctrl+4` says
  `the editor is open: answer the question first`.
- `Ctrl+C` quits at once, without asking, also while the question is up.
- An editor that exits while the question is up leaves it up: `y` still quits, `n` closes it.

Without an editor `q` quits at once, as always. To keep an edit, leave the editor normally (`:wq`,
nano's `Ctrl+X`) and save the view with `Ctrl+S`.

### The abort question

`Ctrl+x` (`editor.abort`) acts only on the editor's tab, and asks first:

```text
Abort the edit and lose it?
Ctrl+x again aborts · any other key keeps editing
```

- `Ctrl+x` again aborts: the editor is killed, nothing is read back, and the view says
  `the editor was aborted; nothing was changed`.
- Any other key (`y`, `Esc`, `?`, …) closes the question and keeps the edit; that key does nothing
  else. A `y` cannot confirm the abort, so nano's habit, `Ctrl+X` then `y`, typed while htui has the
  keys keeps the edit: press `Ctrl+4` and leave nano again.
- Holding `Ctrl+x` down does not confirm it: the key's auto-repeat is ignored, and only a fresh
  press aborts. A paste is dropped and leaves the question up.
- The question closes by itself when the editor exits or its tab leaves the screen.
- `Ctrl+C` quits at once, also while the question is up.

## The editor keys in `keys.toml`

The `[editor]` table of `keys.toml` rebinds the two keys (see the README's
[Changing keys](../README.md#changing-keys)); `htui --print-keys` prints it with the other tables:

```toml
[editor]
focus = ["f12"]
```

The pane's title, the status line and the `?` box then name `F12`. Because the editor takes every
other key while it has them, and `editor.focus` acts on every tab while an edit is open, the file
is held to these rules:

- **`focus` must keep a key.** `[editor] focus = []` is refused:
  `[editor] focus: must keep at least one chord: the editor takes every other key`.
- **`focus`'s key is its alone, in every table.** It never gives its key up, and no other action
  may use it anywhere: `[list] down = "ctrl-4"` is refused with
  `[list] down = "ctrl-4": "ctrl-4" is already editor.focus (default) while an edit is open`, and
  `[editor] abort = "ctrl-4"` with
  `[editor] abort = "ctrl-4": "ctrl-4" is already editor.focus (default) in [editor]`. The other
  way round is allowed: `[editor] focus = ["ctrl-x"]` takes `ctrl-x` from `abort`, which is then
  unbound, with the notice below.
- **`ctrl-c` is taken by default.** It is the default of `global.quit` and `leave.confirm`, which
  act while an edit is open, so `[editor] focus = "ctrl-c"` is refused with
  `[editor] focus = "ctrl-c": "ctrl-c" is already global.quit (default) while an edit is open`
  (and the same for `leave.confirm`).
- **A `[global]` key may take `abort`'s default.** The global keys act beside the editor's on the
  editor's tab, so a global key on an editor key is a collision, with one exception: `abort` left
  at its default gives its key up. `[global] quit = ["ctrl-x"]` loads, and `editor.abort` is then
  unbound. htui starts and says so on stderr, with the file's
  path and line: `[global] quit = "ctrl-x" takes "ctrl-x" from editor.abort, which is now unbound`;
  `htui --print-keys` prints `abort = []` marked `(unbound by global.quit)`. List
  `[editor] abort` with another key to keep an abort.
- **An `abort` you set yourself takes nothing.** On a global key it is an error:
  `[editor] abort = "ctrl-q": "ctrl-q" is already global.queue (default) while an edit is open`;
  so is a `[global]` entry on the key your `abort` lists.

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

- **Mouse.** While the editor's tab is on screen htui does not capture the mouse, so the
  terminal's own selection works (over the whole screen, not only the pane); the editor gets no
  mouse events. On another tab the mouse is that tab's, as without an editor.
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
- **Owed on a real terminal.** htui's tests drive the pane with scripted editors. A run of nvim in
  the pane, and the trial of ten real edits in the in-pane mode (none lost or different from what
  the editor saved, leaving the editor to work elsewhere and coming back), are still owed on the
  maintainer's machine.

## Windows

The in-pane editor compiles for Windows (ConPTY), and htui's checks cross-compile it, but it has
not been run on Windows yet; that verification belongs to MOD-16, and covers the other tabs, the
quit question and the abort question too. Known differences:

- Only a physical `Ctrl+4` toggles the focus and brings the editor back from another tab:
  `Ctrl+\` arrives there as itself (`ctrl-\`), not as `Ctrl+4`, so it is not an alias.
- The editor runs as `cmd /S /C "<value> <file name>"` from the temp directory. A value that
  holds a `"` is not expected to work.
- `cmd` has no `exec`: aborting or quitting ends `cmd`, and whether the editor it started ends with
  it is on MOD-16's list (the suspended mode has the same caveat).
