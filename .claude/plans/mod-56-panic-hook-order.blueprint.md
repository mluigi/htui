# Blueprint: MOD-56, "htui's panic hook is the outermost one"

**Status**: **proposed** (2026-09-28).

**Plan**: `.claude/plans/mod-56-panic-hook-order.plan.md` at `5ec6958`, fact-checked and
maintainer-confirmed. **D217–D221 are binding and are not reopened here.** Everything below is
their mechanical consequence; where the plan's wording drifted from the tree, §0 says so and the
build follows the tree.

**Verified at**: `5ec6958`, branch `mod-56`, worktree `/media/projects/htui-mod-56`. The whole
change was applied, built and tested in that worktree before this file was written — the results
are in §5 and the two source files were then reverted, so the worktree carries this blueprint
alone.

**Scope**: two files, one task, one commit's worth of work.

| File | Change |
|---|---|
| `crates/htui/src/terminal.rs` | `init` builds the terminal (D217); `install_panic_hook` delegates to the new `install_panic_hook_restoring` (D218); `enter`'s comment (D221); the module doc |
| `crates/htui/tests/panic_hook.rs` | the real-chain half of the one existing `#[test]` (D219, D220) |

No migration, no store change, no snapshot, no new dependency, no new file, no new `#[test]`.
`crates/htui/Cargo.toml:35,37` already carry `crossterm` and `ratatui` directly, so the build needs
no manifest edit.

**House style (carried)**: comments say *which layer owns the decision* and *what breaks if the
order changes*; they do not restate the code. `unused_qualifications = "warn"` is a workspace lint,
so `std::io::stdout()` and `crossterm::…` stay fully qualified (that is what the file already
does); only the `ratatui` import line is extended.

---

## 0. Plan wording vs the tree (three, all cosmetic)

| # | Plan says | Tree | Build follows |
|---|---|---|---|
| **A-1** | Claim 5: "`CrosstermBackend` and `Terminal` are reachable from the ratatui **crate root**". | `Terminal` is (`ratatui-0.30.2/src/lib.rs:479`); `CrosstermBackend` is not — it is `ratatui::backend::CrosstermBackend` (`lib.rs:504-508`, a module). The plan's own evidence row says so. | §1 imports `backend::CrosstermBackend`. Writing `ratatui::CrosstermBackend` does not compile. |
| **A-2** | Claim 1 cites `init.rs:397-403` for the `set_panic_hook()` call. | `try_init` is `:397`, the call is `:398`, the body ends `:402`; `set_panic_hook` itself is `:566-572`. | Citations are written `init.rs:398` and `init.rs:397-402`. D221's "updated to the new line numbers" is exactly this. |
| **A-3** | D220: "**three** contained panics leave the count at 0, one uncontained panic raises it to 1". | The tree's three panics are two contained and one uncontained, and the same plan's Task 0 step 1 says "raise **one** contained provider panic … and then one uncontained panic … assert `0` then `1`". | §3 follows the Task text: one contained inline `Boom`, one uncontained. The spawned case is already pinned by the predicate half, and both go through the identical chain. |

---

## 1. `crates/htui/src/terminal.rs`

Pre-edit anchors: module doc `1-5`; `use ratatui::DefaultTerminal;` `:7`; `init` `:18-29`;
`install_panic_hook` `:31-43`; `Suspend::enter`'s doc `:86-88`. Line numbers are pre-edit.

### 1.1 The module doc — append a third paragraph after `:5`

```rust
//!
//! The hook is a *decision* — [`restores_the_terminal`] may answer `false` — and a decision only
//! counts if it is the outermost hook in the process. So [`init`] builds the terminal itself
//! instead of calling `ratatui::init`, which installs a hook of its own that restores the
//! terminal unconditionally *before* calling the one it wrapped (`ratatui-0.30.2/src/init.rs:398`,
//! `:566-572`); behind that one, this predicate is an opinion with no way to act on it (MOD-56
//! D217). Nothing here may take a ratatui init again; `Suspend::enter` says so a second time,
//! for the case where the one that is tempting is the one that is already running.
```

`Suspend::enter` is plain backticks, not an intra-doc link: `Suspend` is `crate::editor::Suspend`,
not in scope here, and `private_intra_doc_links` is denied.

### 1.2 The import — replace `:7`

```rust
use ratatui::{DefaultTerminal, Terminal, backend::CrosstermBackend};
```

(rustfmt 2024 keeps that order; `cargo fmt --all -- --check` is clean with it.)

### 1.3 `init` — replace `:18-29`

```rust
/// Installs the panic hook and takes the terminal over.
///
/// Panics if the terminal cannot be put into raw mode, which is [`ratatui::init()`]'s contract;
/// there is no usable TUI in that case, and the panic is safe because the hook is installed first
/// and restores on it (MOD-56 D217).
#[must_use]
pub fn init() -> TerminalGuard {
    install_panic_hook();
    // `ratatui::try_init`'s body, minus its `set_panic_hook` (`init.rs:397-402`). Written out
    // because the order is the fix: the hook above is the outermost one from here to the end of
    // the process, and a second one — restoring unconditionally ahead of it — would put this
    // crate back where MOD-56 found it, with the predicate right and the terminal gone anyway.
    crossterm::terminal::enable_raw_mode().expect("htui cannot put the terminal into raw mode");
    crossterm::execute!(std::io::stdout(), crossterm::terminal::EnterAlternateScreen)
        .expect("htui cannot enter the alternate screen");
    let backend = CrosstermBackend::new(std::io::stdout());
    TerminalGuard {
        terminal: Terminal::new(backend).expect("htui cannot measure the terminal"),
        restored: false,
    }
}
```

- `#[must_use]` and the signature are unchanged (D217, acceptance box 3).
- The build is `try_init`'s body minus the hook, in ratatui's own order: hook, raw mode, alternate
  screen, backend, `Terminal::new`. The hook goes first precisely so a panic in any of the four
  still restores (plan claim 2, acceptance box 5).
- The three `expect`s carry the `io::Error` in their `Debug` output, so a failure names the syscall
  that failed; `ratatui::init` used the same `try_init().expect(..)` shape, and there is no error
  path in `lib.rs::run` to return into.
- `restores_the_terminal`'s doc and body are **untouched** (`:45-61`). It was already right; the
  hook it was installed into was the wrong one.

### 1.4 The hook pair — replace `:31-43`

```rust
/// Chains a hook that restores the terminal before the previous hook prints the panic.
///
/// Without it a panic leaves the alternate screen up and raw mode on, and the backtrace is drawn
/// over the TUI's last frame.
pub fn install_panic_hook() {
    install_panic_hook_restoring(ratatui::restore);
}

/// The hook, with the restore it performs as a parameter.
///
/// [`install_panic_hook`] is this with `ratatui::restore`, which is the only restore this crate
/// ever wants. The seam is public for one reason: the test that has to prove the *decision*
/// reaches the restore runs the real chain, and it is an integration test in another binary that
/// cannot see a private function (MOD-56 D218). Before the seam it could only assert
/// [`restores_the_terminal`], which is what made the defect invisible: the predicate was right
/// and the terminal was still torn down underneath the event loop, by a hook outside it.
pub fn install_panic_hook_restoring(restore: impl Fn() + Send + Sync + 'static) {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if restores_the_terminal() {
            restore();
        }
        previous(info);
    }));
}
```

`ratatui::restore` is a plain `pub fn()` (`init.rs:524`), so it satisfies
`Fn() + Send + Sync + 'static` as a fn item. The hook body is byte-for-byte the one that was there;
only the restore moved into the parameter. `install_panic_hook`'s signature and behaviour for the
production path are unchanged (acceptance box 3) — the doc comment is kept verbatim.

### 1.5 `Suspend::enter`'s comment (D221) — replace the third doc line, `:88`

```rust
    /// Not `ratatui::init()`, and now for two reasons: it would stack another panic hook
    /// (`init.rs:398`), and since MOD-56 the hook it would wrap is the one that decides whether a
    /// panic gives the terminal back at all — so on every editor suspend it would re-break the
    /// defect this file just closed (D221).
```

`enter`'s **body** is unchanged (`raw mode → EnterAlternateScreen → clear`), as D221 requires; only
the citation and the reason move. The first two doc lines and the
`ratatui-core-0.1.2/src/terminal/buffers.rs:147-173` citation stay as they are.

---

## 2. Ordering note for the implementer

Apply in this order and commit once (or as red/green if the plan's TDD shape is wanted):

1. `tests/panic_hook.rs` §3 — **red**, and it is red by *not compiling*: no such function
   (`install_panic_hook_restoring`).
2. `terminal.rs` §1.1–§1.5 — green.
3. `cargo fmt --all` (no diff expected; the text above is already rustfmt-clean).

One task, one commit. `lib.rs::run` calls `terminal::init()` once (`:123`) and is untouched; no other
file in the workspace calls `set_hook`, `take_hook`, or a ratatui init (plan claims 7, 8).

---

## 3. `crates/htui/tests/panic_hook.rs`

### 3.1 Imports — add one line above `:13`

```rust
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
```

### 3.2 Append to the end of `a_contained_provider_panic_leaves_the_terminal_alone`

The three predicate assertions that are already there stay exactly as they are; this goes after
the last one, still inside the same `#[test]` (D219 — the file keeps exactly one test, because
`set_hook` is process state and a second case would see this one's hook).

```rust

    // The second half of MOD-56, and the half that was missing: everything above pins the
    // *decision*, and the decision was right while the terminal was still torn down underneath the
    // event loop, because the hook that asked it was ratatui's inner one and ratatui's outer one
    // restored regardless of the answer. So this drives the real chain, with a counting restore
    // standing where `ratatui::restore` stands in production. It goes *over* a recorder rather
    // than replacing one — these are the same expected panics as the three above, and a default
    // hook underneath would print three unwind backtraces that read as a failure.
    let restored = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&restored);
    std::panic::set_hook(Box::new(|_info| {}));
    htui::terminal::install_panic_hook_restoring(move || {
        counter.fetch_add(1, Ordering::Relaxed);
    });

    // A contained panic, inline, on this thread — the case that `catch_unwind` swallows without
    // stopping the hook, and the one H-20 exists for.
    let (_, set) = run_providers(&inline, &owned.as_request());
    assert_eq!(set, vec!["boom@0.1:panic".to_owned()]);
    assert_eq!(
        restored.load(Ordering::Relaxed),
        0,
        "a `false` from the predicate has to reach the restore, or the terminal goes down under a \
         running event loop (MOD-56)"
    );

    // And then the panic the process does not survive, on this same thread.
    let _ = std::panic::catch_unwind(|| panic!("the shell itself"));
    assert_eq!(
        restored.load(Ordering::Relaxed),
        1,
        "an uncontained panic still gives the terminal back — that is the whole reason the hook \
         exists"
    );
}
```

What the second half pins, and why it is not vacuous:

- **The recorder is installed first, the real chain over it.** So the chain's own `previous` is a
  no-op, and the two expected panics print nothing. `install_panic_hook_restoring` *chains*
  (`take_hook` → `set_hook`), so the order is the only one that is quiet.
- **The counter is `Arc<AtomicUsize>` behind an `impl Fn() + Send + Sync + 'static`**, because
  `set_hook` needs `Send + Sync + 'static` and the panics land on other threads than the one that
  installed it. `Ordering::Relaxed` is enough: the hook and the assertion are separated by a
  `catch_unwind` or a `run_providers` return, which already orders them.
- **`inline` and `owned` are reused, not re-minted** — they are the same `Vec<Arc<dyn
  ExcerptProvider>>` and `OwnedExcerptRequest` the predicate half ran, and both are still owned.
  No new fixtures.
- **The `0` is the assertion that MOD-56 is about.** Before the fix the hook that asked the
  predicate was the inner one, so an outer `restore()` ran first and the count was already 1.
  The `1` afterwards is the control: the counter really is reachable, so the `0` is a decision and
  not a dead seam.
- The test module's doc is **not** changed (the plan does not ask; its claim — "one `#[test]` in a
  process-state binary" — is still the reason for that shape).

---

## 4. What the reviewer should check

- `init` no longer names `ratatui::init` / `try_init` / `init_with_options`, and
  `install_panic_hook()` is reached before the first `expect` (acceptance 1, 5).
- `install_panic_hook` still exists, with the same signature, and the production path is still
  `ratatui::restore` (acceptance 3).
- `enter`'s body is byte-identical to the old one; only its doc's third line moved (D221).
- `tests/panic_hook.rs` still has exactly **one** `#[test]` (acceptance 4).
- The three existing predicate assertions are unchanged and still pass — the fix changes the chain,
  not the predicate.

## 5. Build order and verification

One task, one commit, in the worktree `/media/projects/htui-mod-56` on branch `mod-56`.

```bash
# red first: the new test does not compile until the seam exists
cargo test -p htui --test panic_hook -- --test-threads=1        # E0425: no install_panic_hook_restoring

# after the terminal.rs half
cargo fmt --all -- --check
cargo test -p htui --test panic_hook -- --test-threads=1
cargo test -p htui --all-features --lib -- --test-threads=1     # 346 pass
cargo clippy -p htui --all-features --all-targets -- -D warnings
```

Then the workspace gate, `--test-threads=1` throughout (the keyring fake is process-wide; project
memory):

```bash
df -h /                                        # target/ fills the disk; 129 G free at 5ec6958
cargo fmt --all -- --check
cargo clippy --workspace --all-features --all-targets -- -D warnings
USERNAME=htui-ci HTUI_TEST_DATABASE_URL=postgres://postgres:htui@localhost:5439/postgres \
  cargo test --workspace --all-features -- --test-threads=1
```

**Not re-run**: `sqlx prepare --check` and the `.sqlx` count (no SQL, no migration), and any
snapshot count. Before believing a Postgres failure, `df -h /` — the dev Postgres crash-loops under
disk pressure (project memory).

**Results of the run that produced this blueprint** (change applied, then reverted):

| Command | Result |
|---|---|
| `cargo fmt --all -- --check` | clean, no diff |
| `cargo test -p htui --test panic_hook -- --test-threads=1` | 1 passed, 0 failed |
| `cargo test -p htui --all-features --lib -- --test-threads=1` | 346 passed, 0 failed |
| `cargo clippy -p htui --all-features --all-targets -- -D warnings` | clean |

The workspace gate and the Postgres suites were not run for this blueprint: the change touches no
store path, and `htui`'s own targets are the whole blast radius (plan §Test plan).

**Live check (optional, needs a tty)** — plan §Validation: run `htui`, start a run whose excerpt
provider panics, confirm the TUI keeps drawing in the alternate screen; then kill the process
ungracefully and confirm the terminal comes back.

**Risk carried from the plan (not a regression)**: a panic *during* `init` now runs htui's hook
rather than ratatui's, so `ratatui::restore`'s error `eprintln` reaches stderr before the panic
message instead of after. Same restore, different print order.
