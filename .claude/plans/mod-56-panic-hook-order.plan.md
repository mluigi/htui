# Plan: MOD-56 — htui's panic hook runs inside ratatui's, so a contained panic still restores the terminal

> **Status: fact-checked, awaiting the CONFIRM gate** (2026-09-28). 11 claims: 11 verified, 0
> amended, 0 falsified; the verdicts are in "Verified claims" at the end. No amendment changed a
> decision.

**Source**: `HANDOFF.md` MOD-56 (from MOD-9, milestone-1 blueprint finding F-U; maintainer-decided
2026-09-26). `terminal::init` installs htui's conditional hook and then calls `ratatui::init()`,
whose `try_init` wraps whatever hook is current in ratatui's unconditional `restore()`. A panic that
`htui_agent::excerpt::run_providers` contains (H-20) therefore still drops the terminal out of raw
mode and the alternate screen mid-session, and `restores_the_terminal()` is never consulted first.

**Requirements**: `R-ID-1`, `R-TUI-1` (MOD-9 blueprint finding F-U). Fixes the wedge M1 left half
open: the predicate is right, the hook it was installed into is the wrong one.

**Complexity**: Small. One function rewritten, one seam added, one test extended. **No migration,
no store change, no snapshot, no new dependency.**

**Routing**: routed as **plan** by `/handoff-run MOD-56` (0 of 4 criteria fired: no cross-repo
reach, no new public API surface beyond a documented hook seam, no open design question, breadth of
2 files). Ultracode: not needed — one task, nothing to decompose.

## Summary

`ratatui::init()` is `try_init().expect(...)`, and `try_init` calls a private `set_panic_hook()`
that does `take_hook()` and installs `move |info| { restore(); hook(info) }` — the inner hook runs
*second*, so the terminal is restored no matter what the inner hook decides. htui's conditional hook
is the inner one and is therefore powerless against a panic that any other layer has already
decided to survive.

The fix is to stop letting ratatui install a hook at all: build the `DefaultTerminal` the way
`TerminalGuard::enter` already builds the terminal state (raw mode, alternate screen,
`CrosstermBackend`, `Terminal::new`) and keep htui's hook as the outermost one. ratatui's own
documentation states the required order ("call this *after* your app installs any other panic
hooks") — the order is the contract, and only the app can honour it when it also needs a
*conditional* restore.

## Design decisions (settled here, not in code review)

- **D217 — init builds the terminal; it does not delegate to `ratatui::init`.** `init()` keeps its
  signature and its `#[must_use]`, and still panics when the terminal cannot be taken over (there is
  no usable TUI then, and `lib.rs::run` has no error path for it). The panic is *safe* because
  htui's hook is installed before the build and restores on it — the same guarantee ratatui's own
  hook gives, and the reason the hook goes in first.
- **D218 — the restore action is a seam, not a hard-coded `ratatui::restore`.**
  `install_panic_hook()` keeps its name and behaviour and is implemented as
  `install_panic_hook_restoring(ratatui::restore)`. The seam is public because the test that must
  run the real chain is an integration test in a different binary and cannot see a private function;
  it is documented as existing for that. There is no other reason for it to be public, and it
  carries no behaviour of its own.
- **D219 — the chain test extends the existing `#[test]`, and does not add a second one.**
  `std::panic::set_hook` is process state and `tests/panic_hook.rs` says so in its own module doc:
  a second case in that binary, running in parallel, would see this one's hook. So the real-chain
  assertions are appended to `a_contained_provider_panic_leaves_the_terminal_alone`, after the
  predicate assertions it already makes, and the binary keeps exactly one test.
- **D220 — the test asserts the *effect*, not the predicate.** The existing test already pins
  `restores_the_terminal()` per panic. What is untested — and what MOD-56 is about — is that the
  decision *reaches* the restore. The new half drives the real chain with a counting restore and
  asserts: a contained provider panic leaves the count at 0, the following uncontained panic
  raises it to 1 — the second assert is the control that makes the first one non-vacuous
  (amended at blueprint: the earlier "three contained panics" wording contradicted Task 0).
  Before the fix the count would be 1 after the first contained panic.
- **D221 — `TerminalGuard::enter` keeps its comment and its shape.** It still must not call
  `ratatui::init()`, and now for a second reason: after D217 any ratatui init would stack an
  *unconditional* hook on top of htui's, reintroducing this exact bug on editor suspend/resume.
  The comment's citation is updated to the new line numbers.

## Patterns to Mirror

- `TerminalGuard::enter` (`crates/htui/src/terminal.rs`) — the raw-mode + alternate-screen
  sequence, minus `Terminal::new`. The new build path is this plus the backend.
- `restores_the_terminal` — the doc comment that explains *why* the question is asked of the agent
  crate and not guessed here. The new code's comments say the same kind of thing: which layer
  owns the decision, and what breaks if the order changes.

## Files to Change

| File | Change |
|---|---|
| `crates/htui/src/terminal.rs` | `init` builds the terminal (D217); `install_panic_hook` delegates to a new `install_panic_hook_restoring` (D218); `enter`'s comment (D221) |
| `crates/htui/tests/panic_hook.rs` | the real-chain half of the existing `#[test]` (D219, D220) |

No other file changes. `lib.rs::run` calls `terminal::init()` once and is unaffected.

## Tasks

One task. There is no independence question: the test drives the seam the fix adds, so the two
halves are one commit's worth of work and one reviewer pass covers both.

### Task 0: the hook becomes the outermost one, and the chain gets a test

1. **Test first.** Append to `a_contained_provider_panic_leaves_the_terminal_alone`: install the
   real chain through `install_panic_hook_restoring` with a counter behind an `Arc<AtomicUsize>`,
   raise one contained provider panic (the existing `Boom`, inline) and then one uncontained panic
   under `catch_unwind`, and assert `0` then `1`. Install a recording hook *under* the real chain
   first, so the chain's own `previous` is still a recorder and the test binary prints no
   backtraces. It must fail to compile (no such function) — that is the red.
2. **Green.** Rewrite `init` per D217: `install_panic_hook()` first, then
   `enable_raw_mode` → `EnterAlternateScreen` → `CrosstermBackend::new(stdout())` →
   `Terminal::new(backend)`, panicking with the `io::Error` in the message. Add
   `install_panic_hook_restoring` per D218. Reword `enter`'s comment per D221.
3. Update the module doc of `terminal.rs`: the two guarantees are now "a `Drop` impl and a
   panic hook, and the hook is the outermost one".

## Test plan

- `cargo test -p htui --test panic_hook -- --test-threads=1` — the extended test; the only new case.
- The existing three predicate assertions inside it are unchanged and must still pass — the fix
  changes the chain, not the predicate.
- Full workspace gate (below). The `htui` crate's other tests do not touch the terminal, so the
  blast radius is one binary.

## Risks

1. **A panic during `init` now runs htui's hook, not ratatui's.** Same behaviour (both restore
   unconditionally, htui's hook restores for every panic that is not contained, and nothing is
   contained at that point in startup) — but the restore runs on the *outer* hook, so the error
   print from `ratatui::restore` reaches stderr before the panic message instead of after. No
   behavioural loss; noted so a reviewer does not read it as a regression.
2. **`install_panic_hook_restoring` is public API.** Accepted (D218) and documented as a test seam.
   If the reviewer objects, the fallback is a `#[cfg(test)]` unit test inside `terminal.rs` and a
   private seam — at the cost of putting a hook-mutating test in the lib's shared test binary,
   which the repo's own convention argues against.
3. **A future `ratatui::init` call anywhere re-breaks this.** Mitigated by D221's comment, which
   now names *this* bug as the reason, not just hook stacking.
4. **The workspace lints `unused_qualifications = "warn"`** and the gate runs `-D warnings`, so
   `std::io::stdout()` and `crossterm::…` must stay fully qualified (the file's existing style). The
   only import line that changes is the `use ratatui::…` one.

## Validation

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-features --all-targets -- -D warnings
cargo test -p htui --test panic_hook -- --test-threads=1
USERNAME=htui-ci HTUI_TEST_DATABASE_URL=postgres://postgres:htui@localhost:5439/postgres \
  cargo test --workspace --all-features -- --test-threads=1
```

`--test-threads=1` is not optional (the keyring fake is process-wide). This change touches no SQL,
no migration and no snapshot, so the `sqlx prepare --check` and `.sqlx` count from the last
milestone are unchanged and are not re-run. Before believing a Postgres failure, run `df -h /`
(project memory: the dev Postgres crash-loops under disk pressure).

**Live check (optional, needs a tty).** Run `htui` in a real terminal, start a run whose excerpt
provider panics (or any contained provider), and confirm the TUI keeps drawing in the alternate
screen. Then `Ctrl-C`/kill the process ungracefully and confirm the terminal is back.

## Acceptance

- [ ] `init()` no longer calls `ratatui::init`/`try_init`/`init_with_options`; htui's hook is the
      outermost one for the whole process lifetime.
- [ ] A contained provider panic (spawned and inline) does not restore the terminal; an
      uncontained panic does. Asserted through the real hook chain, not through the predicate.
- [ ] `install_panic_hook()` keeps its signature and its behaviour for the production path.
- [ ] `tests/panic_hook.rs` still has exactly one `#[test]`.
- [ ] A panic *during* `init` still gives the terminal back.
- [ ] The workspace gate above is green.

## Claims to verify

| # | Claim |
|---|---|
| 1 | `ratatui-0.30.2`'s `try_init` installs a hook that calls `restore()` unconditionally before the previous hook |
| 2 | that hook is installed before `enable_raw_mode`, so a failure mid-init is covered |
| 3 | `ratatui::init()` is exactly `try_init().expect(...)` |
| 4 | `ratatui::restore()` is a plain `fn() -> ()` usable as an `Fn() + Send + Sync + 'static` |
| 5 | `CrosstermBackend` and `Terminal` are both nameable in 0.30.2 — `Terminal` at the crate root, `CrosstermBackend` under `ratatui::backend` (amended at blueprint: "crate root" was wrong for `CrosstermBackend`) |
| 6 | `crossterm` is a direct dependency of the `htui` crate at the same version `ratatui-crossterm` uses |
| 7 | `terminal::init()` has exactly one call site (`lib.rs::run`) |
| 8 | no other file in the workspace calls `set_hook`/`take_hook` |
| 9 | `tests/panic_hook.rs` contains exactly one `#[test]` |
| 10 | `TerminalGuard::enter` already performs the raw-mode + alternate-screen sequence the new build path needs |
| 11 | the file set of the code half and the test half intersect, so the two halves are one task |

## Verified claims

Fact-checked 2026-09-28 against `ratatui-0.30.2` in the local registry and the tree at `68c058f`.

| # | Claim | Verdict | Evidence |
|---|---|---|---|
| 1 | `try_init` installs a hook that calls `restore()` unconditionally before the previous hook | verified | `ratatui-0.30.2/src/init.rs:397-402`, `set_panic_hook()` at `:398` (line range corrected at blueprint); `:566-572` is `take_hook()` then `Box::new(move |info| { restore(); hook(info) })` — `restore` first, inner hook second |
| 2 | that hook is installed before `enable_raw_mode` | verified | same three lines: `set_panic_hook(); enable_raw_mode()?; execute!(stdout(), EnterAlternateScreen)?;` |
| 3 | `ratatui::init()` is `try_init().expect(...)` | verified | `init.rs:365-367` |
| 4 | `ratatui::restore` is a plain `fn() -> ()` | verified | `init.rs:524-529` — `pub fn restore()`, errors are `eprintln`ed, never propagated |
| 5 | `CrosstermBackend` and `Terminal` are both nameable | verified | `lib.rs:479` re-exports `Terminal`; `lib.rs:504-508` `pub mod backend` re-exports `CrosstermBackend` (crossterm feature on by default — the workspace already builds `DefaultTerminal`). `init.rs:213`: `DefaultTerminal = Terminal<CrosstermBackend<Stdout>>`, so `CrosstermBackend::new(std::io::stdout())` is the right backend |
| 6 | `crossterm` is a direct dependency at the version ratatui uses | verified | `crates/htui/Cargo.toml:35` `crossterm = { workspace = true }`; `Cargo.lock` holds one `crossterm 0.29.0` and `ratatui-crossterm 0.1.2` depends on it — no version skew with `enter` |
| 7 | one call site for `terminal::init` | verified | graph text search: `crates/htui/src/lib.rs:123` only (the other hit is the MOD-56 HANDOFF line itself) |
| 8 | no other workspace file calls `set_hook`/`take_hook` | verified | graph text search `set_hook`: 3 hits — `terminal.rs:37`, and two in `tests/panic_hook.rs` (its module doc and the test's own recorder) |
| 9 | `tests/panic_hook.rs` has exactly one `#[test]` | verified | the file's full source: one `#[test]`, `a_contained_provider_panic_leaves_the_terminal_alone` |
| 10 | `enter` already does the raw-mode + alternate-screen sequence | verified | `terminal.rs` `Suspend::enter`: `crossterm::terminal::enable_raw_mode()` then `execute!(stdout, EnterAlternateScreen)` |
| 11 | the two halves share files, so they are one task | verified | both touch `terminal.rs` and `tests/panic_hook.rs`; intersection is non-empty, and the test drives the seam the fix adds — serial by dependency, not by file |
