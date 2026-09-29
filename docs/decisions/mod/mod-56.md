# MOD-56 - htui's panic hook is the outermost one (done, 2026-09-28)

**Requirements:** `R-ID-1`, `R-TUI-1`. Closes MOD-9 milestone-1 blueprint finding F-U.
**Design authority:** no ANA precedes this item. The predicate it fixes is review finding M1's
(`htui_agent::excerpt::run_providers` contains a provider panic on purpose, hazard H-20), and the
fix shape is the maintainer's, from the item text.
**Artifacts:** plan [`.claude/plans/mod-56-panic-hook-order.plan.md`](../../../.claude/plans/mod-56-panic-hook-order.plan.md)
(routed plan 2026-09-28, 0 of 4 routing criteria fired, decisions D217–D221), blueprint
[`.claude/plans/mod-56-panic-hook-order.blueprint.md`](../../../.claude/plans/mod-56-panic-hook-order.blueprint.md).
**Commits:** `5ec6958` (plan), `ffc2ffd` (the fix and the chain test), `47d1e97` (blueprint),
`8867c86` (the review's test findings), `bff43f8` (the review's doc findings), plus the close-out
commit.

## What shipped

`terminal::init` no longer calls `ratatui::init`. It installs htui's panic hook first and then
builds the `DefaultTerminal` itself — `enable_raw_mode`, `EnterAlternateScreen`,
`ratatui::backend::CrosstermBackend::new(std::io::stdout())`, `ratatui::Terminal::new` — the same
sequence `try_init` runs, minus its `set_panic_hook()` call. htui's conditional hook is therefore
the outermost hook for the process lifetime, so `restores_the_terminal()` is consulted **before**
anything restores, and a panic that `run_providers` contains no longer drops the terminal out of
raw mode and the alternate screen under a running event loop.

The signature, the `#[must_use]`, the `TerminalGuard` shape and the panic-on-failure contract are
unchanged. A panic *during* `init` still gives the terminal back: the hook is installed before the
first `expect`, and the hook is the outermost one, so it is the one that runs.

**Decisions.** D217 `init` builds the terminal instead of delegating. D218 the restore action is a
seam — `install_panic_hook_restoring(restore: impl Fn() + Send + Sync + 'static)`, with
`install_panic_hook` now one line delegating to it — because the test that must drive the real
chain lives in another binary and cannot see a private function. D219 the chain assertions extend
the existing `#[test]` in `tests/panic_hook.rs` rather than adding a second one; `set_hook` is
process state and that binary is deliberately one-test. D220 the test asserts the *effect*, not the
predicate: a counting restore reads 0 after a contained provider panic and 1 after an uncontained
one, the second being the control that makes the first non-vacuous. D221 `Suspend::enter` keeps its
body unchanged and its comment now names the real reason a ratatui init must not come back.

## The review's contribution

`rust-reviewer` returned changes requested: nothing in the shipped code, two findings that mattered
about the test that is supposed to guard it.

**The chain test did not pin the order.** It drives the seam directly and never touches `init`, so
reverting `init` to `ratatui::init()` — the whole fix — leaves it green. The only guard against
re-introducing MOD-56 was a comment. Plan risk 3 had named that residual and accepted it; the review
ranked it HIGH because the plan had picked the weakest mitigation for a risk it had already
identified, and a fix whose only guard is a comment has the same shape as the defect it replaces.
The plan was amended accordingly.

The fix is `tests/panic_hook_order.rs`, a source-text guard in its own binary, following
`prompt_settings.rs::no_key_name_is_spelled_in_the_section`. It reads `terminal.rs` **with its
comments stripped** and bans `ratatui::init`, `ratatui::try_init`, `init_with_options` and
`set_panic_hook` from the code. The comment-stripping is a deliberate deviation from the finding's
literal wording: the fix is *about* naming the functions it forbids, and D221 requires `enter` to
keep naming `ratatui::init()`, so a word-ban over the whole file would be red on correct code and
would push the next author into a vaguer comment. The guard's limits are stated in its own module
doc — a call from another file, a name written with odd spacing, or a hand-rolled copy of
`set_panic_hook`'s body all pass it.

Demonstrated, not asserted: with `init` temporarily reverted, `panic_hook_order` goes red and
`panic_hook` stays green. That asymmetry is the finding.

**The chain's base hook was a no-op.** `set_hook(Box::new(|_info| {}))` makes the chain's `previous`
a no-op, and Rust renders a panic's message and location *in the hook* — so a failing `assert_eq!`
would have reported with no message, no file, no line, no left/right. The base is now the captured
default hook, forwarded, which restores legible failures at the cost of two `thread panicked at`
lines for the two *expected* panics (invisible unless the test fails or runs with `--nocapture`).
A recording hook would not have fixed it: it swallows an assertion failure just the same.

The remaining findings were documentation: `init`'s doc named one of its three `expect` failure
points, the module doc claimed a ratatui init was "already running" when after this fix none is,
and a line citation read `init.rs:397-402` for a body that starts at 398.

## Verification

`cargo fmt --all -- --check`, `cargo clippy --workspace --all-features --all-targets -- -D warnings`
and `cargo test --workspace --all-features -- --test-threads=1` all green — 40 test binaries, 0
failed. No SQL, migration, snapshot or `.sqlx` file was touched, so `sqlx prepare --check` and the
268-file `.sqlx` count are unchanged. TDD held: the chain test was red with `E0425` (no such
function) before the seam existed, and the source guard was red against the pre-fix `init` before it
passed.

## Residual

The hand-rolled `init` will drift if upstream `try_init` grows a step. The source guard catches the
worst form of that drift; the rest is what the module doc and the two in-code comments are for.
