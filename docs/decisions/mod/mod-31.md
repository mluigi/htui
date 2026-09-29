# MOD-31 - A running prompt preview makes an adapter install refuse (done, 2026-09-28)

**Requirements:** `R-AGT-10` (`docs/REQUIREMENTS.md:194`), `R-TUI-8` (`:317`), `R-NF-3` (`:347`).
None of the three speaks to *admission control* of one long operation against another — `R-NF-3` is
about where work runs, `R-AGT-10` about install provenance and consent, `R-TUI-8` about the
Settings surface. The fix is a correctness and race argument, not a requirement-quotation one.
**Origin:** MOD-2 close-out, finding F-121, 2026-09-15.
**Artifacts:** [plan](../../../.claude/plans/mod-31-preview-blocks-install.plan.md) and
[blueprint](../../../.claude/plans/mod-31-preview-blocks-install.blueprint.md). The plan was
fact-checked against the tree before confirmation: 30 claims, **9 verified / 18 amended / 3 falsified**
(§ *Verified claims* in the plan records each verdict with its evidence). Decisions **D1–D7**,
risks **R-1–R-6**, open questions **OQ-1–OQ-3** (all three answered by the maintainer 2026-09-28,
all taking the recommended default).

## What was wrong

`AgentRuntime.background` was one `Vec<JoinHandle<()>>` holding three different kinds of work: the
agent probe and the chat staleness re-probe, which **write** `agent_box`, and the prompt preview,
which **writes nothing** (MOD-2 plan D102, archived at `docs/decisions/mod/mod-2.md:173`).
`claim_is_free` refused whenever that collection was non-empty. So holding `j` in the Backlog
detail — a task that reads a dozen tables, walks the filesystem and records nothing — refused `i`
in Settings for as long as it took, with a message about a probe that was not running.

## What was built

`background` is now `Vec<Background>`, where `Background` is a module-private struct carrying the
handle and a private `Writes { AgentBox, Nothing }` tag, constructible **only** through
`Background::writing(..)` and `Background::reading(..)`. The tag is a property stated at the push
site and greppable as a named claim, never a bare `bool` a maintainer has to guess. The guard
consults a predicate rather than a count:

```rust
if self.background.iter().any(Background::writes_agent_box) {
    return Err(StoreError::Backend(
        "a probe or a re-probe is already writing this box; install once it has finished".to_owned(),
    ));
}
```

The four push sites are tagged by what their task actually does: `probe` and the re-probe in
`start` are `writing`; `preview` is `reading`; the test placeholder is `writing`. `background_len()`
is byte-identical, so all 23 of its read sites — including the `R-NF-3` pin in
`crates/htui/tests/prompt_preview.rs:657` — keep their meaning unamended. A narrow
`writing_background_len()` sits beside it so a test can say what it means rather than infer from a
total. The three lifecycle readers (`sweep_finished`, `finish_background`, `shutdown`) keep one
sweep, one await loop and one abort loop; `finish_background`'s load-bearing ordering
(`previews` → `background` → `box_probe` → `install` → `auth`) is unchanged.

The refusal sentence now names a re-probe as well as a probe, because after the change those are
exactly the two kinds of task that hold the claim.

## Blast radius, wider than the item stated

`claim_is_free` has **five** callers, not the three the HANDOFF credited: `on_online`, `probe_box`,
`install_plan`, `install_confirm` and `auth_start`. A live preview therefore also blocked a login
and a `ProbeBox` — and, worst, the registration probe an `Online` swap wants to start, which
`on_online` skips **silently**, recording nothing but a `tracing::info!` and leaving the box
unprobed until the next swap. One line repairs all five, which is the reason the change was made at
the guard rather than at the install.

Recorded and **not** fixed (D7): `AgentRuntime::probe` (`ProbeAgents`) never consulted
`claim_is_free` and still does not. It checks `auth`, `install` and `box_probe_running` inline, so
two `ProbeAgents` may already run concurrently, and a preview never blocks one. The exclusion is
one-directional today. Widening it is a behaviour change to a path no item asked for.

## Verification

- `cargo fmt --all -- --check` — clean.
- `cargo clippy -p htui --all-targets --all-features -- -D warnings` — clean, no warnings.
- `cargo test -p htui --lib agent_worker -- --test-threads=1` — 75 passed, 0 failed (72 before,
  plus the three new cases).
- `cargo test -p htui --all-features -- --test-threads=1` — 28 test binaries, 0 failures. The
  Postgres suites ran rather than skipping (`box_probe_pg` 9, `templates_pg` 2).
- No SQL: `.sqlx` still 268, `crates/htui/tests/snapshots` still 88, and the diff over
  `migrations`, `cache_migrations`, both snapshot directories and `.sqlx` is empty. `sqlx prepare`
  is not part of this change's gate.
- **The three new tests were confirmed able to fail.** Reverting the guard to
  `!self.background.is_empty()` locally turns the two preview cases red with the guard's own
  message. The third, `a_running_chat_reprobe_still_refuses_an_install`, is the counterweight and is
  *supposed* to stay green under a revert: it asserts a refusal still happens when a **writing**
  task holds the claim, which both guards do. Its job is to fail if someone over-narrows the guard
  to exclude re-probes — a different mutation.

## Review

`rust-reviewer` (the repo's configured reviewer) returned **correct-with-fixes**: no CRITICAL, no
HIGH. It verified against the implementation — not the comments — that no caller is left
under-protected, that all four push-site tags match the spawned futures' terminal effects (including
tracing `run_preview` to confirm no reachable write path), and that `finish_background`'s ordering
and both `mem::take` consumers are intact. All six findings were applied:

- **MEDIUM** — the two preview cases asserted the *absence of a refusal* inside an `if let`, so a
  runtime that refused for any unrelated reason (an `installing_runtime` that stopped attaching its
  installer fails at `install_config`, checked before `claim_is_free`) went green with the claim
  entirely untested. They now assert the positive `Served::Deferred`, which is strictly stronger: a
  refusal is never `Deferred`, so every precondition must have returned `Ok` and the spawn must have
  happened. The comment justifying the old shape was itself inverted and has been rewritten.
- **MEDIUM** — three intra-doc links inside `Background` used `Self::`, which resolves to
  `Background` rather than `AgentRuntime`; all three now name `AgentRuntime` explicitly. The crate
  sets `broken_intra_doc_links = "deny"`, so these were binding even though nothing currently
  reddens.
- **NITs** — a miscounted `serve` call, a missing `background_len() == 1` precondition in the third
  case, "frame channel" for a reply channel, and a missing note on why `any` takes the function-item
  form where `filter` cannot.

## Also worth recording

Two defects in the blueprint were found during implementation and fixed rather than worked around,
both of which would have shipped a test that could not fail:

1. `Iterator::filter(Background::writes_agent_box)` does not compile — `filter` hands its predicate
   `&&Background` while a `&self` method coerces to `fn(&Background)`. `any` takes `FnMut(Self::Item)`
   and is fine, so the guard keeps the function-item form. The distinction is now documented at
   `writing_background_len`.
2. The blueprint specified `!message.contains("already writing this box")`, which is **vacuous
   against the pre-fix guard** — which refuses with different words, so the new substring is
   trivially absent on broken code. The test would have passed on exactly the code it existed to
   fail. Superseded entirely by the positive assertion above.

## Commits

| | |
|---|---|
| `b1f9b3e` | plan |
| `044a0fd` | fact-check amendments (9/18/3) |
| `6189661` | maintainer's answers to OQ-1..OQ-3 |
| `2fa6f38` | blueprint |
| `9731d9e`, `fd44a43` | T0 — the tagged split (red, green) |
| `12a1225`, `a0be406` | T1 — the guard consults the writing half (red, green) |
| `e3968c3`, `9add749`, `e81faa7` | T2 — the new cases, and their de-vacuuming |
| `a54ac63`, `4e9ff9b`, `8a56e85` | review findings applied |
