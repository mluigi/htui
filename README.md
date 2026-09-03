# htui

`htui` is a keyboard-driven terminal UI for a single developer's cross-box workflow store: it opens
on a **workspace**, lists that workspace's items grouped by project, and shows the selected item's
body, runs, link graph, documents and notes side by side. The product contract it implements is
`docs/REQUIREMENTS.md` (`R-TUI-1..3` for the surfaces, `R-NF-1` for the platforms, `R-NF-3` for
"the UI never blocks"), and the data it shows is shaped by `docs/ANA-9.md`, the concluded data
model: every view reads through the ANA-9 §6.1 store seam (`ReadStore` / `WriteStore` / `Backend`)
rather than through a database handle, so the in-memory store this milestone ships behind that seam
is replaceable by the Postgres store and the per-box cache (MOD-6) without a single view changing.

## Build

Requires **rustc 1.98+** (edition 2024; `rust-toolchain.toml` pins 1.98.1 exactly, so `rustup` will
fetch it on first build).

```
cargo build --release
```

The binary lands in `target/release/htui` (`target/release/htui.exe` on Windows).

## Run

```
cargo run -p htui -- --demo
```

| Flag | Meaning |
|---|---|
| `--demo` | Load the deterministic demo fixture (two workspaces, three projects, items, runs, notes and documents) instead of an empty store. |
| `--log <PATH>` | Append `tracing` output to a file. Never stdout — stdout is the TUI. Also read from `HTUI_LOG`; `HTUI_LOG_FILTER` overrides the default `info` level. |
| `--help`, `--version` | Print usage / version and exit. |

With `--demo` the shell enters the first workspace by name (`Graphics`) and opens on the Backlog
tab. With an empty store nothing can be entered, so the workspace switcher stays up over the shell
reading `no workspaces` — creating one is MOD-15.

## Keys

Keys are a table, not a `match`: a key reaches the topmost overlay first, then the overlay's
bindings, then the active tab, then the tab's bindings, then the global table.

### Global

| Key | Action |
|---|---|
| `q` | Quit |
| `Tab` / `Shift+Tab` | Next / previous tab |
| `1` … `9` | Select a tab by position (`1` Backlog, `2` Skills, `3` Settings) |
| `w` | Open the workspace switcher |
| `?` | Toggle the key help box |

### Workspace switcher (overlay)

| Key | Action |
|---|---|
| `j` / `Down` | Next workspace |
| `k` / `Up` | Previous workspace |
| `Enter` | Enter the selected workspace (changes the scope) |
| `Esc` | Close the overlay |

`Esc` closes any overlay; the switcher is modal, so a key it does not handle never reaches the tab
underneath.

### Backlog tab

| Key | Action |
|---|---|
| `j` / `Down` | Next row |
| `k` / `Up` | Previous row |
| `g` / `Home` | First row |
| `G` / `End` | Last row |
| `Enter` | Fold or unfold the project group, when the cursor is on a project header |
| `l` / `]` / `Right` | Next detail sub-tab |
| `h` / `[` / `Left` | Previous detail sub-tab |
| `J` / `K` | Scroll the detail pane one row down / up |
| `PageDown` / `PageUp` | Scroll the detail pane ten rows |

The five detail sub-tabs are **Body**, **Runs**, **Graph**, **Documents** and **Notes**
(`R-TUI-3`). The Skills and Settings tabs are placeholders in this milestone.

## Platforms

`R-NF-1` is Windows 10+, Linux and macOS, and nothing in the crate is platform-specific: the
terminal layer is `crossterm` and the drawing layer is `ratatui`.

- **Windows**: **Windows Terminal** is the target and the only configuration the rendering is
  tuned for — it has the Unicode box-drawing characters, the middle dot in the top bar and the
  colour depth the theme assumes.
- **Legacy `conhost.exe`** (the old console host, and `cmd.exe` windows opened outside Windows
  Terminal) is **best-effort**: it runs, but box-drawing and the `·` separator depend on the
  console code page and the font, so expect replacement characters with a raster font.
- **Linux / macOS**: any `xterm`-compatible terminal with UTF-8.

The terminal is restored on every exit path, panics included: a panic hook runs `ratatui::restore()`
before the default hook prints, and the terminal guard restores again on drop.

## Tests

```
cargo test --workspace --all-features
cargo fmt --all -- --check
cargo clippy --workspace --all-features --all-targets -- -D warnings
```

The UI tests are `insta` snapshots rendered against a `TestBackend` at 100x30 through
`htui::testkit::Harness`, which serves store requests inline: no sleeps, no spawned worker, so a
snapshot is byte-stable. Snapshots live next to their tests in `crates/htui/tests/snapshots/` and
`crates/htui/src/snapshots/`. When a change moves a rendering on purpose, re-record with
`INSTA_UPDATE=always cargo test --workspace --all-features`, then run the suite again without the
variable to confirm the recorded snapshots match, and commit the `.snap` files.

## Scope of this milestone (MOD-1)

MOD-1 is the **scaffold**: the shell, the key table, the tab and overlay registries, the store
worker seam and **read-only** views over an in-memory store loaded with demo fixtures. Nothing here
writes. The rest is tracked as its own item and lands as an additive module — a file plus one
registration line, with no change to the event loop:

| Deferred to | What |
|---|---|
| **MOD-13** | Filters and item editing (new, edit, close) |
| **MOD-14** | Navigable Graph traversal, re-rooting and multi-hop |
| **MOD-15** | Hierarchy management: creating workspaces, projects, repos and item kinds |
| **MOD-4** | Run actions: run, approve, reject, retry, cancel |
| **MOD-6** | The Postgres store and the per-box offline cache behind the same seam |
| **MOD-2** | The Chat tab |

The TUI scope is always a workspace (a single project still lives in one), and the store seam is
`docs/ANA-9.md` §6.1 — read it before changing a signature there.
