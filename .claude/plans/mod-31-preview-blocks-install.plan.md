# Plan: MOD-31 — a running prompt preview makes an adapter install refuse

**Source**: `HANDOFF.md:225-235` (MOD-31, from MOD-2 finding F-121). Found at MOD-2 close-out,
2026-09-15. **Routing**: no PRD, no blueprint — routed as **plan** (one field split, one guard, one
refusal sentence; below C3's threshold). **Base**: the Rust source is byte-identical to `main` at
`68c058f` (merge of MOD-7 milestone 4); `HEAD` in this worktree is `b1f9b3e`, and the *only* diff
against `68c058f` is this plan file. Every line number below is therefore `68c058f`'s, and
`b1f9b3e`'s for the plan file itself.

**Tooling caveat (read before using Gortex on this worktree).** The Gortex index is over the **`main`
checkout at `68c058f`**; `workspace checkouts` does not list `htui-mod-31`, so graph queries will not
be scoped to this worktree. Line numbers carry over because the Rust source is byte-identical, but
verify against the working tree before acting on any graph result. Two methodological facts the
fact-check surfaced: the Gortex `callers` edge list for `claim_is_free` is **incomplete** (it reports
3 callers, the true count is 5), so call-site counts must come from literal text search; and this
plan's line numbers are reliable where anchored to a **text literal** and drift by 1–6 lines where
derived by reading a body preceded by a doc comment.

**Requirements**: `R-AGT-10` (`docs/REQUIREMENTS.md:194-201` — "An agent whose adapter is not
installed on a box can be installed **from the app** … The app re-probes after installing, so what
the box can run is always the probe's answer"), `R-TUI-8` (`:317-321` — the Settings tab carrying the
agent registry where `i` lives), `R-NF-3` (`:347-348` — "All long operations … run off the UI
thread; the TUI never blocks on network or subprocess I/O"). `R-AGT-10` is the requirement the bug
bears on most directly: an install the user can see and ask for is refused by a task that writes
nothing.

**Framing caveat on the requirements.** None of the three speaks to **admission control** of one
long operation against another. `R-NF-3` is about *where* work runs, `R-AGT-10` is about install
provenance and consent, `R-TUI-8` is about the Settings section's surface. "A preview must not block
an install" is a **correctness and race argument, not a requirement-quotation argument** — see
disagreement 5 below, which states it in that form.

**Why it matters beyond the sentence.** `AgentRuntime::preview` (`agent_worker.rs:1225-1259`) pushes
the deferred preview task into `self.background` at `:1257`. `claim_is_free` (`:1519-1552`) refuses
whenever `!self.background.is_empty()`, with *"a probe is already running on this box; install once
it has finished"*. `background` is one collection holding three different kinds of work: the agent
probe (`run_probe`, **writes** `agent_box` for one row per *enabled* agent row that yields
`ProbeOutcome::Row`), the chat staleness re-probe (`run_reprobe`, **writes** `agent_box` for one
row, pushed from `start` at `:1758`), and the preview (`run_preview`, **reaches no write method** —
MOD-2 plan D102, `.claude/plans/mod-2-prompt-assembler.plan.md:519`, "the preview is a sixth Backlog
detail sub-tab, `Prompt`, answered by a new `StoreRequest::PromptPreview` … It writes nothing",
archived at `docs/decisions/mod/mod-2.md:173`; the code says so in `preview`'s own comment at
`agent_worker.rs:1249-1253` and `run_preview` (`crates/htui/src/preview.rs:368-392`) calls no write
method — see claim 13 for the precise, and weaker, form of that claim).

**Worse than the item says.** `claim_is_free` has **five** callers, not the three the HANDOFF
credits: `on_online` (`:523`, the registration probe on an `Online` swap), `probe_box` (`:1192`),
`install_plan` (`:1287`), `install_confirm` (`:1330`) and `auth_start` (`:1402`). `:523` is
`if let Err(err) = self.claim_is_free() {`; the other four are `self.claim_is_free()?;`, each
converted by `serve` into `failed("probe_box" | "install_plan" | "install_confirm" | "auth_start",
&err)`. No test calls it. A live preview therefore also blocks `r` (the registration probe on
demand), the **login** path, and — worst — the registration probe an `Online` swap wants to start,
which `on_online` skips **silently**: the only trace is a `tracing::info!` at `:525`, and nothing was
recorded, so the box stays unprobed until the next swap. One guard fix repairs all five, because
they share the guard.

**Complexity: Low.** One new private struct in one file, four push sites, three lifecycle readers, one
guard, one refusal sentence, and the tests that pin them. **No migration, no `StoreRequest` or
`StoreReply` variant, no `.sqlx`, no snapshot, no new crate, no new store method, no SQL of any
kind** — so the `sqlx prepare` step of the gate can be skipped entirely.

**Numbering**: this is a standalone MOD item, so the plan restarts at **D1…D7** (MOD-30 and MOD-7
milestone 4 each restart in their own file; MOD-7's D104…D125 belong to the MOD-7 PRD and are cited
here as *MOD-7 D-n*). Risks **R-1…R-6**, open questions **OQ-1…OQ-3**, tasks **T0…T2**.

---

## Open questions for the maintainer

- **OQ-1 — The shape of the split.** Three candidates, weighed below.
  **Default (recommended): A — one collection of tagged entries**, `Vec<Background>` where
  `Background { task: JoinHandle<()>, writes: Writes }`, built only through two named constructors
  `Background::writing(task)` and `Background::reading(task)`, with the field private to the module.
  One sweep, one await loop, one abort loop; `background_len()` is untouched; `claim_is_free` becomes
  `self.background.iter().any(Background::writes_agent_box)` and nothing else in the file learns a
  second collection exists. **Alternative B — two `Vec`s**: `background` (writing) and a second for
  readers. The guard's `!self.background.is_empty()` reads unchanged, but `sweep_finished`,
  `finish_background` and `shutdown` each grow a second loop or a second `mem::take`, and every
  future push site has to pick the right one of two containers with nothing at the call site saying
  which. **Alternative C — an enum** `BackgroundTask::{Probe, ReProbe, Preview}`: most type-safe,
  but a fifth kind of task is a new variant *and* a new `claim_is_free` match arm, and
  `claim_is_free` would have to enumerate the writing ones by hand — the exact failure mode this
  item exists to remove. Recommend **A**.
- **OQ-2 — What `background_len()` keeps returning.** **Default (recommended): its current
  meaning — how many background tasks the runtime currently owns, both halves — plus a new narrow
  `writing_background_len()` for the new tests.** All 24 existing assertions then keep their meaning
  unchanged, including `the_preview_is_deferred_onto_a_task_the_runtime_owns`
  (`crates/htui/tests/prompt_preview.rs:634-671`, asserting `1` at `:657`), which is the test that
  pins `R-NF-3` as a fact about the runtime. **Alternative: narrow `background_len()` to the writing
  half.** That is more honest about what the guard consults, but it rewrites the meaning of ~20
  "a refusal spawned nothing" assertions in `agent_worker.rs` (they would pass either way, so the
  churn is the cost, not the correctness) and it *breaks* `prompt_preview.rs:657`, which would have
  to be amended to read `0` — a test that then says the preview is not a background task, which is
  false and is the opposite of what `R-NF-3`'s test exists to say. Recommend the default.
- **OQ-3 — Should a chat-started staleness re-probe still block an install?** **Recommend yes**
  (conservative, and it matches the existing `agent_box`-has-no-box-PK reasoning: a re-probe for row
  X and an install's re-probe for row Y can be the same row, and last-write-wins on `agent_box` is
  a stale `probed_at` the user then trusts), **and rename the refusal sentence so it names a
  re-probe as well as a probe.** The guard is box-wide, so a re-probe for row X still blocks an
  install for row Y — a real over-block, but narrowing the guard to per-`(agent_id, box_id)` is a
  different and much larger change (`agent_box` is keyed on it, not on the box alone) and out of
  scope. **Recommended new sentence:**
  *"a probe or a re-probe is already writing this box; install once it has finished"*. It names both
  kinds, it does not say the false thing the current one does (that a probe is running when a
  re-probe is), and it keeps the shape the section's status line already renders. **Alternative:**
  leave the sentence as it is and only narrow the set — cheaper, but it leaves a user staring at
  "a probe is already running" with a preview as the only thing alive. One test pins the current
  words and must be amended either way (see the Test plan).

---

## Summary

`AgentRuntime` (`agent_worker.rs:372-446`) keeps one collection of background handles,
`background: Vec<JoinHandle<()>>` (`:384`), and three code paths push into it. Two of them write
`agent_box`; the third cannot. The one guard that decides whether an install, a login, a `ProbeBox`
or a swap's registration probe may start consults that whole collection. The result is that holding
`j` in the Backlog detail — a preview that reads a dozen store methods, walks a filesystem and
reaches no write method
— locks `i` in Settings for as long as it takes, and also silently costs a `ProbeBox` and a login,
with a sentence that names a probe that is not running.

**The split (T0).** A private `Background` struct in `agent_worker.rs` carries the handle and a
`Writes` tag, and is built only through `Background::writing(task)` and `Background::reading(task)`.
The four push sites each state their property: `probe` (`:1165`) and `start` (`:1758`) write,
`preview` (`:1257`) reads, and the test placeholder at `:8212` becomes a constructor call. The three
lifecycle readers — `sweep_finished`'s `retain` (`:1022`), `finish_background`'s `mem::take` loop
(`:645`), `shutdown`'s `mem::take` + `abort` (`:1100-1102`) — each lose one deref and gain a
`Background::task` / `into_task` accessor, and nothing else about them moves. `background_len()`
(`:627-629`) is byte-identical. The two inline tests that reach into `runtime.background` directly
(`:8205-8221`, `:8226-8245`) are amended to construct a **writing** entry, so both keep expressing
the intent they were written for: a background task that *writes* holds the claim.

**The guard (T1).** `claim_is_free`'s last arm changes from `!self.background.is_empty()` to
`self.background.iter().any(Background::writes_agent_box)`, and the refusal sentence becomes OQ-3's.
All five callers — including the two the HANDOFF does not mention, `on_online` and `probe_box` — are
fixed by that one line, which is the reason to make the change here rather than at the install.

**The tests (T2).** Two new inline cases pin the new behaviour: a live preview does not refuse an
install plan, and a live staleness re-probe still does (with the new sentence in the message). One
amendment to `a_probe_and_an_install_never_write_the_same_row_at_once` (`:5762-5843`) for the
renamed words. The preview-counting assertions in `crates/htui/tests/prompt_preview.rs` do not move.

## Design decisions (settled here, not in code review)

| # | Decision | Why |
|---|---|---|
| D1 | **`Background` is a private struct in `agent_worker.rs`**, `struct Background { task: JoinHandle<()>, writes: Writes }` with `enum Writes { AgentBox, Nothing }`, both private to the module. The only ways to build one are `Background::writing(JoinHandle<()>) -> Self` and `Background::reading(JoinHandle<()>) -> Self`. Accessors: `fn task(&self) -> &JoinHandle<()>`, `fn into_task(self) -> JoinHandle<()>`, `fn writes_agent_box(&self) -> bool`. The field is never named outside the constructors, so no call site — and no test — can write `writes: true`, and no future push site can tag itself with a bare bool literal. | OQ-1's default. The tag is a **property stated at the call site**, and the two names (`writing` / `reading`) are the statement. A bare bool at a fifth push site is a maintainer's guess; `Background::writing` is a claim they can grep for. |
| D2 | **The four push sites are tagged by what their task writes, and nothing else changes about them.** `probe` (`:1165`, `run_probe`) → `Background::writing`; `start` (`:1758`, `run_reprobe`) → `Background::writing`; `preview` (`:1257`, `run_preview`) → `Background::reading`; the test placeholder at `:8212` (`tokio::spawn(async {})`) → `Background::writing`, because the test next to it is about a *held* claim (OQ-3's conservative answer), not about a preview. | The tag is a fact about the spawned future, decided once, at the only place that knows it. Nothing in the lifecycle or the guard re-derives it. |
| D3 | **`background_len()` is unchanged**, body and doc: `self.background.len()`. Its doc ("The one fact a test needs to prove D52: a refused probe spawned **nothing**, rather than spawning and then discarding") stays true — it still answers *how many tasks the runtime owns*, and "spawned nothing" is still `0`. | OQ-2's default. The function's job is *ownership*, not *the claim*; the guard is one of five readers of the collection and was never its only one. |
| D4 | **A new `pub fn writing_background_len(&self) -> usize`**, `self.background.iter().filter(Background::writes_agent_box).count()`, `#[must_use]`, documented as "how many of those tasks write `agent_box` — the set `claim_is_free` consults". It exists so the new tests can say what they mean rather than inferring it from a total. | Without it, a test that wants to prove "the preview is not in the writing set" has to assert a *total*, which is the bug the item is about. Two facts, two accessors. |
| D5 | **The guard consults a predicate, not a count.** `if self.background.iter().any(Background::writes_agent_box) { … }`. Its comment is rewritten to name the split: a probe and an install's re-probe write `agent_box`; a preview does not, so it holds nothing. The Settings-section argument that is in the comment today (about `wants_requests` re-issuing `Agents` and any `StoreReply::Agents` clearing the section's `probing` flag) stays — it is the reason the guard exists at all, and D1 does not weaken it. | The predicate is the whole point of the split, stated in the place that uses it. A `len()` comparison would invite the same confusion back. |
| D6 | **The refusal sentence becomes** *"a probe or a re-probe is already writing this box; install once it has finished"* (OQ-3's default). It is a plain literal in `claim_is_free`, not a `const` (it is the only user of it, and `BOX_PROBE_RUNNING` already has its own constant for the other arm). | The current sentence is false for the re-probe case and, after this change, never true for a preview. Naming both kinds is the smallest truthful string. |
| D7 | **The scope is `claim_is_free` and nothing else.** `AgentRuntime::probe` (`:1117-1173`) does **not** call `claim_is_free` and does not consult `background` at all — it checks `self.auth` (`:1126`), `self.install` (`:1135`) and `self.box_probe_running()` (`:1142`) inline, then pushes at `:1165`. (It does hold a `Writer`, taken from `backend.writer()` at `:1147` and handed to `run_probe` — that is the other writer in the collection being split, and it is why the probe is tagged `writing`.) So `ProbeAgents` is unaffected by the preview today and stays unaffected by this change, and two `ProbeAgents` may already run concurrently. That asymmetry is recorded, not fixed: fixing it means adding the writing half to `probe`'s own inline checks, which is a behaviour change to a path no item asks about. | Scope. Recorded as disagreement 4 below. |

## Patterns to Mirror

| Concern | Mirror | Where |
|---|---|---|
| A **private** handle struct with named constructors beside its owner | `Frames` — a private handle struct in this file | `crates/htui/src/agent_worker.rs:2940` |
| A private RAII struct that owns a claim for its scope | `ReprobeClaim` | `crates/htui/src/agent_worker.rs:2256` |
| A private tag enum that names a property rather than a bool | `LivePhase::{Planning, Installing}`; `ChatBinding::{Fresh, Promoted { .. }}` (a private tagged enum) | `LivePhase` declared at `agent_worker.rs:235` (used at `:1299`, `:1348`); `ChatBinding` at `:1790` |
| *(counter-example, do not mirror the visibility)* | `LiveInstall` (`agent_worker.rs:248`) and `LiveAuth` (`:312`) are **`pub struct`**, not private, even though they are referenced only inside this file — so `pub` buys nothing today. This matters: D1's acceptance criterion is "no other module can construct or read a `Background`", which holds only if `Background` and `Writes` are genuinely private, not `pub`. | `agent_worker.rs:248`, `:312` |
| The three lifecycle readers of `background` | `sweep_finished`, `finish_background`, `shutdown` | `agent_worker.rs:1016-1036` (`:1022` the `retain`), `:641-681` (`:645` the `mem::take` loop), `:1066-1108` (`:1100` the take + `abort`) |
| A guard that is a named method every caller shares | `claim_is_free` and its five call sites | `agent_worker.rs:1519-1552`; callers at `:523`, `:1192`, `:1287`, `:1330`, `:1402` |
| A refusal that names what holds the claim, in the other direction | `probe`'s own inline arms ("a login is running for agent …", "an install is running for agent …") and `BOX_PROBE_RUNNING` | `agent_worker.rs:1126-1145`; the constant is used at `:1522` and asserted by value at `:8131` |
| A test that reaches into a private field to stage a holder | `a_held_claim_skips_the_registration_probe_without_a_reply`, `a_finished_background_task_does_not_stop_the_registration_probe` | `agent_worker.rs:8226-8245`, `:8205-8221` (both in the inline `mod tests`) |
| A test that pins the refusal **words** | `a_probe_and_an_install_never_write_the_same_row_at_once` (`message.contains("probe is already running")`) | `agent_worker.rs:5762-5843`, the assert at `:5806` |
| A test that pins `R-NF-3` as a fact about the runtime | `the_preview_is_deferred_onto_a_task_the_runtime_owns` | `crates/htui/tests/prompt_preview.rs:634-671` (`background_len() == 1` at `:657`) |
| An inline test fixture that makes an install real | `installing_runtime`, `unresolvable_registry`, `install_row`, `Fixture::start()` | used by `a_probe_and_an_install_never_write_the_same_row_at_once` (`agent_worker.rs:5762-5786`) |

## Files to Change

| File | Action | Task | Why |
|---|---|---|---|
| `crates/htui/src/agent_worker.rs` | edit | T0, T1, T2 | `Background` (D1), the field and its doc (`:384`), four push sites (D2), three lifecycle readers, `background_len` (D3), `writing_background_len` (D4), `claim_is_free` (D5, D6), and every inline test that touches the collection |
| `crates/htui/tests/prompt_preview.rs` | **verify only** | T2 | read `the_preview_is_deferred_onto_a_task_the_runtime_owns` (`:634-671`) and `the_preview_refuses_offline_with_one_sentence` (`:570-590`) after D3; **no edit expected** under OQ-2's default |

**Not touched, on purpose:** `crates/htui/src/preview.rs` (its `run_preview` already reaches no
write method — D1 only records that, in a comment, inside `agent_worker.rs`);
`crates/htui/src/store_worker.rs`; every Settings section. On the last: **a UI-side duplicate of
the guard does exist**, but it is a *stale-flag pre-flight*, not a copy of the sentence — see
disagreement 7 and claim 21. `self.probing` is written in exactly two places
(`agents.rs:1197`, cleared at `:1217`/`:1225`) and is set only by the section's own `r` keypress;
`AgentsSection::wants_requests` (`:1110-1113`) is exactly `vec![StoreRequest::Agents]`;
`begin_install` (`:729-762`) checks `install_in_flight()`, `auth_in_flight()`, `self.probing`,
`selected()` and `declares_a_source()` and **nothing about `PromptPreview`**. So a preview passes the
UI gate and is refused by the runtime — the bug reproduces exactly as the item says, and the runtime
fix is sufficient. Also not touched: every migration, `cache_migrations/` and
`crates/htui-store/tests/migrations.rs`; `crates/htui-store/.sqlx/`; every snapshot;
`AgentRuntime::probe` (D7); `docs/**`, `HANDOFF.md`, `DECISIONS.md`.

## Tasks

**Wave structure — one wave, one lane, three serial tasks.** Independence is decided by touched-file
sets, and all three tasks touch `crates/htui/src/agent_worker.rs`: T0 changes the type the guard
consumes, T1 changes the guard, T2 adds tests in the same file's `mod tests`. So **T0 → T1 → T2 is a
chain, not a fan-out**: T0 ∩ T1 ≠ ∅, T1 ∩ T2 ≠ ∅, T0 ∩ T2 ≠ ∅, all three equal
`{crates/htui/src/agent_worker.rs}`. The main thread may run them as three sequential implementer
prompts on one branch, or hand all three to one implementer; **they may not run in parallel**, and
the wave plan does not claim they do. The only file outside that one, `crates/htui/tests/prompt_preview.rs`,
is T2's verification read only.

| Task | Files (complete list) | Parallel |
|---|---|---|
| T0 | `crates/htui/src/agent_worker.rs` | Wave 1, **serial: T0 → T1 → T2** |
| T1 | `crates/htui/src/agent_worker.rs` | after T0 |
| T2 | `crates/htui/src/agent_worker.rs` (inline tests), `crates/htui/tests/prompt_preview.rs` (verify only) | after T1 |

**Hidden couplings checked.** No new `StoreRequest`/`StoreReply` variant, so the two exhaustive
matches over them (`store_worker.rs` `name` and `try_serve`) do not move. No store method, no
`query!`, so `.sqlx` and the store `CASES` pins do not move. No snapshot renders a refusal sentence
today (the sentences reach the user through the generic status line, not a frame), so no `.snap`
moves — T2 verifies that by running the suite rather than by assertion. **Build coupling:** none
across crates; only `crates/htui` compiles. **Runtime coupling:** the `BoxProbe` and login tests
`finish_background` at the end; under T0 those loops are unchanged in behaviour, so a `Background`
holding a `JoinHandle` behaves exactly as a bare one did.

### Task 0: the split (D1, D2, D3, D4)
- **Files**: `crates/htui/src/agent_worker.rs` only.
- **Tests first.** T0's first commit is red with the two direct-push tests amended and
  `claim_is_free` untouched: `a_held_claim_skips_the_registration_probe_without_a_reply`
  (`:8226-8245`) pushes `Background::writing(tokio::spawn(std::future::pending::<()>()))` and aborts
  through `for task in std::mem::take(&mut runtime.background) { task.into_task().abort(); }`, and
  `a_finished_background_task_does_not_stop_the_registration_probe` (`:8205-8221`) pushes
  `Background::writing(tokio::spawn(async {}))` and waits with
  `while !runtime.background.iter().all(|entry| entry.task().is_finished())`. Both then **fail to
  compile** until `Background` exists — that is the honest first red for a type-level change, and
  the plan does not manufacture a runtime failure to precede it.
- **Action**: `Background` + `Writes` + the two constructors and three accessors, private to the
  module, placed immediately above `impl AgentRuntime`; the `background` field's doc (`:377-383`)
  rewritten to say the collection mixes tasks that write `agent_box` with tasks that only read, and
  that `claim_is_free` consults the writing half; the four push sites tagged (D2); the three
  lifecycle readers re-pointed; `background_len` untouched (D3); `writing_background_len` added
  (D4). `previews` (`:405`) is **not** re-keyed: it still stores the `AbortHandle` taken from the
  handle *before* the task is moved into its `Background`, which is why `preview` must call
  `task.abort_handle()` first (it already does, at `:1254`).
- **Validate**: `cargo test -p htui --all-features -- --test-threads=1`;
  `cargo clippy -p htui --all-targets --all-features -- -D warnings`.

### Task 1: the guard consults the writing half (D5, D6, D7)
- **Files**: `crates/htui/src/agent_worker.rs` only.
- **Tests first.** `a_probe_and_an_install_never_write_the_same_row_at_once` (`:5762-5843`) has its
  refusal assert at `:5806` re-pointed from `message.contains("probe is already running")` to
  `message.contains("is already writing this box")` — red until D6 lands, and the test that keeps
  the sentence honest.
- **Action**: `claim_is_free`'s last arm (D5) and its sentence (D6). Nothing else: the five callers
  are untouched, which is the point — they inherit the fix. The comment above the arm keeps the
  Settings-section argument and gains the split.
- **Validate**: `cargo test -p htui --all-features -- --test-threads=1`;
  `cargo clippy -p htui --all-targets --all-features -- -D warnings`.

### Task 2: the new cases, and the preview tests re-verified (D3, D4)
- **Files**: `crates/htui/src/agent_worker.rs` (inline `mod tests`), `crates/htui/tests/prompt_preview.rs`
  (read only).
- **Tests first, two new inline cases.**
  1. **`a_running_preview_does_not_refuse_an_install`** — the bug, pinned. Same staging as
     `a_probe_and_an_install_never_write_the_same_row_at_once` (`unresolvable_registry` +
     `install_row` + `installing_runtime` + a `Fixture::start()` whose `/registry.json` route is
     not needed here): serve a `StoreRequest::PromptPreview` for a demo item with
     `runtime.serve`, assert `matches!(served, Served::Deferred)`; assert
     `runtime.background_len() == 1` (the preview is owned, `R-NF-3`) and
     `runtime.writing_background_len() == 0` (it is not in the guard's set — D4 is what makes this
     assert possible); then serve `StoreRequest::InstallPlan { agent_id }` and assert it is
     **not** a `StoreReply::Failed` carrying a `claim_is_free` sentence. The cleanest form is to
     assert the message, if any, is not the refusal: the plan is allowed to fail for the fixture's
     own reasons, so the test matches on `!message.contains("already writing this box")` and says so
     in the assert message, rather than pretending the install succeeded.
  2. **`a_running_chat_reprobe_still_refuses_an_install`** — OQ-3's conservative answer, pinned. Serve
     a `ChatStart` for a `cli`… no: for a stale `acp` row so `start` pushes a re-probe, assert
     `writing_background_len() == 1`, then serve `InstallPlan` and assert
     `message.contains("is already writing this box")`. Reuse the staleness staging from
     `a_chat_start_on_a_stale_acp_row_re_probes_in_the_background` (`:4976-5009`).
  3. A third, cheap case worth adding: **`a_running_preview_does_not_refuse_a_box_probe`** — the
     `probe_box` caller (`:1192`), the one the HANDOFF does not mention and the one that costs a
     silent skip on an `Online` swap. Same staging as (1), serving `StoreRequest::ProbeBox` instead
     of `InstallPlan` and asserting the refusal sentence is absent. Optional; if the fixture makes
     it awkward, drop it and say so.
- **Then**: re-run the whole `crates/htui` suite and confirm **zero** amendments are needed in
  `prompt_preview.rs` (D3). If any preview-counting assert moves, OQ-2's default is wrong and the
  plan says so in the close-out rather than editing the test.
- **Validate**: `cargo test -p htui --all-features -- --test-threads=1` (includes
  `prompt_preview`); `cargo test -p htui --test prompt_preview --all-features -- --test-threads=1`;
  `cargo clippy -p htui --all-targets --all-features -- -D warnings`.

## Test plan

TDD per repo convention: T0's first commit is the two direct-push tests re-pointed at the new
constructors (red because the type does not exist), T1's is the renamed refusal assert, T2's is the
three new cases.

**`background_len()` — all 23 read sites, and what each one asserts.** Verified by literal text
search (`background_len()` across the tree, which is the authoritative method here — the Gortex
`usages` edge list and the fact-check's 24 both count the *definition* at `:627` as a read site):
**23 read sites across exactly two files, 21 in `agent_worker.rs` and 2 in
`prompt_preview.rs`**. All 21 `agent_worker.rs` sites are enumerated in the table below. Under D3
**none of them moves**.

By value: **16 sites assert `== 0`** and are invariant under the split; **7 assert `== 1`** — 5 that a
**writing** task is in the collection (`:5003`, `:5118`, `:5442`, `:5562`, `:5813`), 1 that a
**reading** task is (`prompt_preview.rs:657`, the `R-NF-3` pin), and 1 a **synthetic held** task that
is neither (`:8239`).

| Site | Asserts | Task under D3 |
|---|---|---|
| `agent_worker.rs:5003` | `background_len() == 1` after a `ChatStart` spawned a staleness re-probe | unchanged (a writing task) |
| `:5066` | `== 0` — no staleness re-probe while a box probe runs | unchanged |
| `:5118` | `== 1`, "a 25 h old row is stale" | unchanged |
| `:5160`, `:5195` | `== 0` — a `cli` row and a fresh row are not re-probed | unchanged |
| `:5270` | `== 0` — "a fresh row is not stale, so nothing was spawned on the arm" | unchanged |
| `:5408` | `== 0` — an unresolved placeholder does not re-probe | unchanged |
| `:5442` | `== 1`, "a 25 h old row is stale" | unchanged |
| `:5521`, `:5540` | `== 0` — an offline backend refuses the probe before spawning anything | unchanged |
| `:5562` | `== 1` — the probe task answers at the request's address | unchanged |
| `:5673` | `== 0` — a plan is refused before any request on a buffered writer | unchanged |
| `:5813` | `== 1`, "and the refusal spawned nothing of its own" | unchanged |
| `:6625` | `== 0` — a start is refused before any spawn on a buffered writer | unchanged |
| `:6867` | `== 0`, "and it spawned nothing" — a probe is refused while a login runs | unchanged |
| `:7907`, `:7922` | `== 0` — `ProbeBox` is refused offline before spawning anything | unchanged |
| `:8138` | `== 0` — `ProbeAgents` is refused while the registration probe runs | unchanged |
| `:8218` | `== 0`, "the finished task was swept" | **amended in T0** (constructor + `entry.task().is_finished()`), same value |
| `:8239` | `== 1`, "the held task is still there" | **amended in T0** (constructor + `into_task().abort()`), same value |
| `:8258` | `== 0` — a production runtime does not auto-probe without opt-in | unchanged |
| `prompt_preview.rs:587` | `== 0` — an offline preview is refused before a task is spawned | unchanged |
| `prompt_preview.rs:657` | `== 1`, "one task, owned by the runtime" — the `R-NF-3` pin | unchanged under D3; **would break** under OQ-2's alternative, which is the reason for the default |

**Assertions that move, exhaustively.** Three, all in T0/T1:
1. `agent_worker.rs:8233-8234` — the direct `runtime.background.push(...)` in
   `a_held_claim_skips_the_registration_probe_without_a_reply`; becomes `Background::writing(...)`.
   The intent survives **because D2 tags it writing**: the test is about a background task that
   *writes* holding the claim, and under the new guard a *reading* task would not.
2. `agent_worker.rs:8241-8243` — the `std::mem::take(&mut runtime.background)` teardown loop;
   becomes `task.into_task().abort()`.
3. `agent_worker.rs:8212-8214` — the `push` and the
   `runtime.background.iter().all(JoinHandle::is_finished)` wait in
   `a_finished_background_task_does_not_stop_the_registration_probe`; become
   `Background::writing(...)` and `.all(|entry| entry.task().is_finished())`. The import of
   `JoinHandle` at the top of the test module may become unused — check before removing it, because
   `LiveInstall`/`LiveAuth` still name it.
4. `agent_worker.rs:5806` — the refusal-word assert in
   `a_probe_and_an_install_never_write_the_same_row_at_once` (T1). This is the **only** assertion a
   rename breaks anywhere in the tree.

   **Explicitly *not* broken — do not touch these.** They assert sentences this change does not
   touch, in the *other* direction:

   | Site | Asserts | Why it survives |
   |---|---|---|
   | `agent_worker.rs:5836` | `"install is running"` | the `self.install` arm of `probe` |
   | `:5743` | `"already running"` | the `self.install` arm |
   | `:6778` | `:install` | the `self.install` arm |
   | `:6827`, `:6895` | `"a login is already running"` | the `self.auth` arm |
   | `agents.rs:1202` | `"a probe is already running"` | UI-local section message, a different string |
   | `tests/probe.rs:117` | — | a sibling probe message, not this one |
   | `tests/box_settings.rs:890`, `:895` | `BOX_PROBE_RUNNING` | the *other* `claim_is_free` arm, `agent_worker.rs:93` |
   | `tests/settings.rs:1269`, `:1989` | — | UI-local |

   **Non-test document that will go stale:** `HANDOFF.md:228`, inside the MOD-31 checklist line
   that lifecycle step 3 deletes anyway — so nothing needs amending, but note it so nobody "fixes"
   it. The tree-wide search for `"a probe is already running on this box"` returns exactly two hits:
   the guard at `:1548` and `HANDOFF.md:228`.

**Snapshots.** None move. No frame renders a refusal sentence (the sentences reach the user through
the generic status line) and no `prompt_preview` frame renders a task count. T2 verifies this by
running the suite, not by assertion.

**Test-list pins that do not move.** store `CASES` 77, `READ_CASES` 14, `htui-orch` `CASES` 72,
`StoreRequest` 69, `StoreReply` 40, `hierarchy::REQUEST_NAMES` 13, 268 `.sqlx` files, 88 snapshots,
`MIRRORED_TABLES` 21 (`HANDOFF.md:36-42`). This item adds no store method, no request, no migration
and no frame. The main thread's close-out is the full of `.claude/rules/workflow-docs.md`
lifecycle step 3, which reads verbatim:

> On completion: delete the checklist line from HANDOFF.md, write the detailed writeup + commit hash
> to `docs/decisions/<prefix>/<prefix>-N.md`, prepend its index line to the top of DECISIONS.md
> (reverse-chronological, newest first), and update HANDOFF's summary table + top status line.

So it is **four** edits, not two: delete the checklist line (`HANDOFF.md:225-235`); write
`docs/decisions/mod/mod-31.md` (the directory exists with 8 files, none named `mod-31.md`) and
prepend its one index line to `DECISIONS.md`; **drop the `MOD-N | 41 (…)` summary row at
`HANDOFF.md:748`, which names MOD-31**; and **touch the top status line**. The rule's `paths:`
frontmatter governs `docs/ANA-*.md` as well as `HANDOFF.md`, `DECISIONS.md` and
`docs/decisions/**`. (The "P1"/"P2" split used elsewhere in this plan is this plan's own labelling,
not the rule's: the rule asks for one write-up file per resolved item, and MOD-31 is
single-milestone, so it archives immediately rather than in stages.)

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| **R-1** — A future push site is tagged wrong. A task that writes `agent_box` is tagged `reading`, and the guard lets an install run beside it. | Low | D1 makes the tag a constructor name, so a wrong tag is a named, greppable claim (`Background::reading` at a `run_probe` push site) rather than a bare `false`. The reviewer check is a grep for `upsert_agent_box` inside every task named in a `Background::reading` call. The doc on `Background::writing` states the promise explicitly: *the task writes `agent_box` for at least one row, so `claim_is_free` must see it*. |
| **R-2** — A task that starts reading and later starts writing. | Low, and it is a real shape: `run_preview` is a `Backend` clone that could grow a writer. | The tag is declared once, at the push site, from what the task does **today**. D1's doc makes the promise per-constructor; the T2 case `a_running_preview_does_not_refuse_an_install` and the existing `run_preview` source (`preview.rs:368-392`, **reaches no write method** — see claim 13 for the precise form) are the two things a future change to `run_preview` would have to break. Mitigation for that future: the guard's arm is one predicate, so promoting the preview to writing is a **one-word** change at one call site, which is the property this design is for. |
| **R-3** — Something outside `claim_is_free` assumed `background` was uniformly writing. | None found | Full read: the only readers of the field are `background_len` (`:627`), `finish_background` (`:645`), `sweep_finished` (`:1022`), `shutdown` (`:1100`) and `claim_is_free` (`:1546`); the other two inline tests reach it directly (`:8212`, `:8233`). `previews` (`:405`) holds an `AbortHandle` keyed on `Origin`, not a handle, and its doc already says the map "owns the *right to cancel*, not the task". The field is private to the struct and the struct is `pub`, but the field is not `pub`, so no other module can read it — verified by the fact that only the inline `mod tests` (same file) touches it. |
| **R-4** — The narrowed guard lets an install's re-probe race a **running preview** on the same row. | None | A preview writes no `agent_box` row at all (`R-NF-3` / MOD-2 D102). There is no row to race on. This is the item's own argument and it holds. |
| **R-5** — The fix is narrower than the symptom, and a user still sees a refusal they do not understand. | Medium | This is OQ-3's whole subject, and D6 is the mitigation: after the change the only tasks in the set are a probe and a re-probe, and the sentence names both. If the maintainer prefers the OQ-3 alternative (leave the sentence), record that the sentence is knowingly inexact. |
| **R-6** — The change is mistaken for permission to let `ProbeAgents` run beside an install. | Medium | D7: `probe` never consulted `background` and still does not, so nothing about `ProbeAgents` changes. Say so in the close-out, and leave the `ProbeAgents`-does-not-share-the-claim asymmetry for a separate item if it is wanted. |

## Validation

This change touches **no SQL of any kind** — no migration, no `cache_migrations/`, no `query!`, no
store method, no `crates/htui-store/.sqlx/` entry. `README.md:492-511` makes `cargo sqlx prepare`
conditional on a changed query, so **skip `cargo sqlx prepare` entirely**.

**The gate's origin.** The authoritative statement of the gate is **`README.md:467-473`** ("##
Tests"), with `README.md:475-485` giving the `HTUI_TEST_DATABASE_URL` / `docker compose up -d` form.
The house plans in `.claude/plans/` are only a restatement of it; the fact-check confirmed they
agree, so this plan cites README as the origin and the plans as the house spelling.

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-features --all-targets -- -D warnings
USERNAME=htui-ci HTUI_TEST_DATABASE_URL=postgres://postgres:htui@localhost:5439/postgres \
  cargo test --workspace --all-features -- --test-threads=1
ls crates/htui-store/.sqlx | wc -l                      # 268, unchanged
ls crates/htui/tests/snapshots | wc -l                  # 88, unchanged
cargo doc --workspace --no-deps --keep-going            # exactly the six baseline errors (HANDOFF.md:44)
git diff --stat 68c058f -- crates/htui-store/migrations crates/htui-store/cache_migrations \
  crates/htui/tests/snapshots crates/htui/src/snapshots crates/htui-store/.sqlx   # empty
```

The `git diff --stat` line covers **five** paths, not four: `crates/htui/src/snapshots` is a fifth
snapshot directory (1 entry, `README.md:515`) and is easy to miss, because `Files to Change` names
only `crates/htui/src/agent_worker.rs` (edit) and `crates/htui/tests/prompt_preview.rs` (verify
only) — neither is SQL, which is why skipping `sqlx prepare` is right.

**Operational note — this worktree's build footprint.** The worktree has its **own** `target/`: no
`CARGO_TARGET_DIR` is set, and `.cargo/config.toml` sets only `SQLX_OFFLINE = "true"`. It measured
**5.3 GB** after fmt + clippy + lib-test, and will land higher for a full
`--workspace --all-features --all-targets` build. That is acceptable here because the worktree is on
**`/media` (`/dev/sda`, 3.0 T available)**, *not* on `/` — so unlike the usual case it does not eat
the root filesystem that the dev-Postgres disk-pressure failure mode depends on. Still worth
`df -h` before a full `--all-targets` run.

`--test-threads=1` is not optional (the keyring fake is process-wide, and this suite is
scheduling-dependent per project memory). Before believing a Postgres failure, run `df -h /` — the
dev Postgres crash-loops under disk pressure — and re-run the case alone. There are no Postgres cases
in this change's path; `crates/htui/tests/box_probe_pg.rs` exists and runs, so the `crates/htui` gate
must be run with the env var set so it is not silently skipped.

**Live check on this box (optional, after the merge).** Launch `htui` against the dev database,
open the Backlog, hold `j` on a row so a preview starts, switch to Settings > Agents, and press `i`.
Today it answers *"a probe is already running on this box"*; after the change the plan request is
answered. Repeat with an install genuinely running (a slow registry `HEAD`) and confirm the
refusal still appears, now reading *"a probe or a re-probe is already writing this box"*.

## Acceptance

- [ ] `claim_is_free` refuses only when a background task **writes** `agent_box`; a live preview in
      `background` does not refuse an install, a login, a `ProbeBox` or a swap's registration probe.
- [ ] All five callers of `claim_is_free` (`on_online`, `probe_box`, `install_plan`,
      `install_confirm`, `auth_start`) inherit the fix with no edit of their own.
- [ ] A live staleness re-probe **does** still refuse an install, with a sentence naming a re-probe.
- [ ] `background` is one collection, its four push sites are tagged through named constructors, and
      no other module can construct or read a `Background` — which holds only if `Background` and
      `Writes` are genuinely private, **not** `pub` (cf. `LiveInstall`/`LiveAuth`, which are `pub`
      and buy nothing).
- [ ] `background_len()` is unchanged and **all 23 of its read sites** pass without amendment; the
      `R-NF-3` pin at `crates/htui/tests/prompt_preview.rs:657` still asserts `1`. (The fact-check
      reported 24; literal text search finds 23 read sites — 21 in `agent_worker.rs`, 2 in
      `prompt_preview.rs` — and the 24th reference is the definition at `:627`.)
- [ ] No snapshot, no migration, no `.sqlx` entry, no store `CASES` pin and no `StoreRequest` /
      `StoreReply` variant moves; the pin counts in `HANDOFF.md:36-42` are unchanged.
- [ ] The gate above is green, and the diff over `migrations`, `cache_migrations`,
      `crates/htui/tests/snapshots`, `crates/htui/src/snapshots` and `.sqlx` is empty.
- [ ] Close-out is complete per `.claude/rules/workflow-docs.md` lifecycle step 3 — **all four**
      parts: the MOD-31 checklist line is deleted; the write-up (with its commit hash) is at
      `docs/decisions/mod/mod-31.md`; its one index line is prepended to `DECISIONS.md`; and
      `HANDOFF.md`'s summary table (the `MOD-N | 41 (…)` row at `HANDOFF.md:748`) **and** top
      status line are updated. Main thread only; no implementer touches those files.

## Where the HANDOFF, REQUIREMENTS or tree disagree

1. **`claim_is_free` has five callers, not three.** The item names the install; the tree has
   `on_online` (`:523`), `probe_box` (`:1192`), `install_plan` (`:1287`), `install_confirm` (`:1330`)
   and `auth_start` (`:1402`). Two of those two extras are the *login* (already noted in the task
   brief) and — not noted anywhere — **`probe_box`**, which is what an `Online` swap's registration
   probe goes through. `on_online` skips the probe when `claim_is_free` errors and says nothing but
   `tracing::info!` (`:525`), so a preview running at connect time costs a silent unprobed box. The
   fix is one line for all five, so this plan takes it; the discrepancy is recorded because the
   item's framing understates the blast radius.
2. **The item's line numbers have drifted** and are cited wrong: `agent_worker.rs:824` for the
   preview push (it is `preview`'s own body at `:1257`, reached from `serve`'s `PromptPreview` arm at
   `:952-960`; `:824` is inside `preview`'s doc comment) and `:1116` for the guard (it is
   `claim_is_free`, `:1519-1552`, the arm at `:1546`, the string at `:1548`).
3. **`background` has four push sites, one of them a test.** The item says "tasks that write
   `agent_box` (the probe, the re-probe)" and "tasks that only read (the preview)" — correct — but
   the fourth push (`agent_worker.rs:8212`, `tokio::spawn(async {})`) is a placeholder inside
   `a_finished_background_task_does_not_stop_the_registration_probe`, so a mechanical
   `grep background.push` rewrite has to decide its tag. D2 decides it: **writing**, because the
   test's own doc says the claim is held by a background task and the sibling test at `:8226` is
   about a *held* claim. That is a judgement, not a derivation, and the fact-check should confirm it.
4. **`AgentRuntime::probe` does not consult the claim at all** (`:1117-1173`): it checks `self.auth`,
   `self.install` and `self.box_probe_running()` inline and never looks at `background`, so two
   `ProbeAgents` may already run at once and a preview never blocks one. The item's "the guard's
   reason is real — a probe and an install's re-probe race" is therefore a **one-directional**
   exclusion today, and the tree is not symmetric. D7 leaves it alone; R-6 flags it.
5. **`R-AGT-10` is the requirement the bug bears on most directly, and the item does not say so.**
   The item lists `R-AGT-10`, `R-TUI-8`, `R-NF-3` as a set; `R-AGT-10`'s text (`:194-201`) is the one
   an install refused by a preview most directly strains ("can be installed **from the app**"),
   `R-TUI-8` names the section where the refusal surfaces, and `R-NF-3` is what makes the preview
   *exist* as a background task in the first place and what the split must not break.
   **Caveat the fact-check surfaced and this plan's framing overstates:** none of the three speaks
   to *admission control* of one long operation against another. `R-NF-3` is about *where* work
   runs, `R-AGT-10` about install provenance and consent, `R-TUI-8` about the section's surface.
   "A preview must not block an install" is a **correctness and race argument, not a
   requirement-quotation argument**, and the fix should be argued that way.
6. **D102's provenance is a MOD-2 plan decision, not a `docs/` decision.** "the preview writes
   nothing" is `.claude/plans/mod-2-prompt-assembler.plan.md:519` (D102), restated at `:52` and
   `:76` of that file (and at `:735`, `:865`), and is archived at `docs/decisions/mod/mod-2.md:173`.
   That plan file **is present on disk and is git-tracked** (`git check-ignore` exits 1;
   `git ls-files` returns it) — there is no gitignore caveat. The **code** assertion is the stronger
   and current one: `run_preview` (`crates/htui/src/preview.rs:368-392`) **reaches no write
   method**; it is `build` that reads. Note that MOD-7 milestone 4 changed `build` to read repo
   paths (`docs/decisions/mod/mod-7.md`; see the plan's D120) — still reads, still no write method.
   The `agent_worker.rs:1249-1253` comment that cites D102 (the `writes nothing` line is `:1252`) is
   the one to keep in step. A text search for the literal phrase "the preview writes nothing" misses
   it only because the phrase is split across a line wrap at `:1251`/`:1252`.
7. **A UI-side gate on `i` exists — it is just not preview-aware.** The item's framing, and this
   plan's first draft, said there was no UI-side duplicate of the guard. There is: a *stale-flag
   pre-flight* on the same `self.probing` flag, in three sibling places —
   `AgentsSection::begin_install` (`crates/htui/src/ui/tabs/settings/agents.rs:743-746`:
   `if self.probing { ctx.emit(Action::Error("a probe is running".to_owned())); return; }`), the `r`
   arm in `on_key` (`:1202`, `Action::Error("a probe is already running")`), and `begin_auth`
   (`:367`). None of them is a copy of the runtime's sentence, and the runtime's own comment at
   `agent_worker.rs:1541-1544` already documents that this section-side flag is unreliable — any
   `StoreReply::Agents` clears it, and `wants_requests` re-issues `Agents` on every activation.

   The operative conclusion is unchanged, and now positively verified: `self.probing` is written in
   exactly two places (`agents.rs:1197`, cleared at `:1217`/`:1225`) and is set **only** by the
   section's own `r` keypress; `AgentsSection::wants_requests` (`:1110-1113`) is exactly
   `vec![StoreRequest::Agents]`; `begin_install` (`:729-762`) checks `install_in_flight()`,
   `auth_in_flight()`, `self.probing`, `selected()` and `declares_a_source()` and **nothing about
   `PromptPreview`**. So **a preview passes the UI gate and is refused by the runtime — the bug
   reproduces exactly as the item says, and the runtime fix is sufficient.** No UI pre-flight keys on
   a preview, so no UI change is needed.

---

## Claims to verify

Every checkable fact this plan asserts, for the fact-check pass. Line numbers are at `68c058f`.
Found through the Gortex index (symbol source, callers, usages, tree-wide text search) and direct
reads of the non-indexed `.claude/`, `docs/` and `HANDOFF.md`.

1. **Amended.** `HEAD` of `htui-mod-31` is **`b1f9b3e`**, not `68c058f`. The **Rust source tree is
   byte-identical** to `main` at `68c058f`, and the only diff between the two commits is this plan
   file. `graphify-out/` does not exist. (The first draft of this claim named `68c058f` as HEAD.)
2. `AgentRuntime` is at `agent_worker.rs:372-446`; `background: Vec<JoinHandle<()>>` is declared at
   `:384` with the doc at `:377-383` ("Tasks this runtime spawned that answer a request of their own:
   today the probe's (MOD-2 D53)"); `previews: HashMap<Origin, AbortHandle>` at `:405` with the
   doc sentence at `:396-397` "An entry names a task that is also in `background` — this map owns
   the *right to cancel*, not the task."; `box_probe: Option<JoinHandle<()>>` at `:435`.
3. `background_len`'s body is at `:627-629`; body is `self.background.len()`. Its **doc is at
   `:625-626`** and the clause this plan quotes is only its leading half — "The one fact a test
   needs to prove D52: a refused probe spawned **nothing**" — followed by ", rather than spawning
   and then discarding" (which is why the quoted fragment reads as a dangling clause).
4. `finish_background` is at `:641-681`; it clears `self.previews` first, then
   `for handle in std::mem::take(&mut self.background)` at `:645`, aborting on a timeout, and only
   then takes `box_probe`, `install` and `auth` as four separate arms.
5. `sweep_finished` is at `:1016-1036`; the `background` line is
   `self.background.retain(|task| !task.is_finished());` at `:1022`; the same method also retains
   `live`, `previews` and `take_if`s `install`, `auth` and `box_probe`. It is called from
   `serve` (**`:889`**, the first statement of the body — the first draft said `:892`) and from
   `on_online` (`:522`).
6. `shutdown` is at `:1066-1108`; `self.previews.clear()` is at **`:1099`**, and
   `for task in std::mem::take(&mut self.background) { task.abort(); }` at `:1100-1102`.
7. **Amended.** `claim_is_free` is at `:1519-1552`. Its arms in order: `self.box_probe_running()`
   (**`:1522`**, `BOX_PROBE_RUNNING`), `self.auth` (**`:1528`**), `self.install` (**`:1534`**), and
   `if !self.background.is_empty()` (`:1546`) returning
   `StoreError::Backend("a probe is already running on this box; install once it has finished")`
   (`:1547-1549`, refusal string at **`:1548`**). The first draft put the arms at 1526/1533/1540 —
   each 4–6 lines high, the usual drift for a body located by reading past its doc comment.
8. `claim_is_free` has exactly **five** call sites: `on_online` `:523`, `probe_box` `:1192`,
   `install_plan` `:1287`, `install_confirm` `:1330`, `auth_start` `:1402`. **Call form:** `:523` is
   `if let Err(err) = self.claim_is_free() {`; the other four are `self.claim_is_free()?;`, each
   converted by `serve` into
   `failed("probe_box" | "install_plan" | "install_confirm" | "auth_start", &err)`. **No test calls
   it.** A tree-wide literal text search for `claim_is_free` returns exactly **10** hits: the
   definition, the 5 call sites, and 4 prose mentions at `:90`, `:287`, `:420`, `:1132`. (The Gortex
   `callers` edge list reports only 3 — it is incomplete, so call-site counts must come from text
   search.)
9. **Amended.** `AgentRuntime::probe` is at `:1117-1173`. It checks `self.auth` (**`:1126`**),
   `self.install` (**`:1135`**) and `self.box_probe_running()` (**`:1142`**) inline, **never calls
   `claim_is_free`, and never reads `self.background` as a guard**; it pushes at `:1165`. The first
   draft said 1127/1137/1143. Note also that `probe` takes a **`Writer` from `backend.writer()` at
   `:1147`** and hands it to `run_probe` — the other writer in the collection being split, and the
   reason the probe is tagged `writing`.
10. There are exactly **four** `background.push` sites: `probe` `:1165` (`run_probe`), `preview`
    `:1257`, `start` `:1758` (`run_reprobe`), and the test
    `a_finished_background_task_does_not_stop_the_registration_probe` `:8212`
    (`tokio::spawn(async {})`).
11. **Amended** — now confirmed from code, not from the comments. `run_probe` (`:1979-1995`) calls
    `probe_agents_on` (`:2003-2041`), which loops `for summary in &mut agents`, `continue`s on
    `!enabled`, and on `ProbeOutcome::Row(row)` calls `writer.upsert_agent_box(&row)`; a `Kept`
    outcome writes nothing. So `run_probe` writes **one row per *enabled* row that yields
    `ProbeOutcome::Row`** — "for every agent row on the box" is loose (disabled rows are skipped,
    `Kept` rows are left alone), which is harmless for the guard but should not be overstated.
    `run_reprobe` (`:2325-2370`) calls `probe_agent` once and does exactly **one**
    `writer.upsert_agent_box(&row)`. Supporting comment text: `start`'s at `:1724-1737` (the first
    draft said `:1742-1748`) and `run_reprobe`'s doc ends at `:2324` (the first draft said
    `:2317-2320`).
12. **Amended.** `preview` is at `:1225-1259`; the offline arm returns first (**`:1234-1240`**);
    the spawn is at **`:1241-1248`**; the D102 comment is at **`:1249-1253`**;
    `self.previews.insert(origin, task.abort_handle())` at **`:1254`** (a single line; the arm runs
    `:1254-1256`); `self.background.push(task)` at `:1257`; returns `Served::Deferred` at `:1258`.
    The first draft said 1239-1243, 1245-1252, 1250-1254 and 1253-1255. **Why the D102 search missed
    it:** the comment's phrase "the preview writes nothing" is split across a line wrap at
    `:1251`/`:1252`, so a literal search for the phrase does not match the code.
13. **Amended — the load-bearing half survives, the sweeping half does not.** `run_preview`
    (`crates/htui/src/preview.rs:368-392`) reaches **no write method**: that half is confirmed, and
    it is what the guard's correctness rests on. But "holds no `Writer` at all" is **false** —
    `run_preview` awaits `build`, which calls `backend.writer()` and then
    `writer.repos(row.project_id)`. That is a **SELECT**: `Writer::repos`
    (`htui-store/src/writer.rs:592`) → `PgStore::repos` (`pg/write.rs:1898`) →
    `PgStore::repo_rows` (`pg/read.rs:2225`); and `repos` is declared on the **`WriteStore` trait**
    (`htui-core/src/store/traits.rs:597`) despite being a read. `build`'s complete store surface is
    `item`, `project`, `item_kind`, `prompt_templates`, `resolve_graph`, `app_settings`,
    `documents_of_kinds`, `upstream_summaries`, `box_info`, `box_profile`, `bound_skills`, `repos`,
    `repo_paths` — all reads — plus the pure `settings::resolve_*`, `excerpt_roots`,
    `touched_prefixes`, `MinimalScrubber::new` and `assemble`; `excerpts_for`
    (`htui-agent/src/excerpt.rs:932-1017`) does a `spawn_blocking` filesystem pass. The string
    `Writer` has **0** hits in `preview.rs`, and `WriteStore as _` is imported only to bring `repos`
    into scope. **Restated as "reaches no write method", not "holds no `Writer`"** — that is the
    honest form of the guard's safety argument, and it should be argued in that form.
14. **Amended.** `start`'s re-probe is gated at **`agent_worker.rs:1754-1755`** (the first draft said
    `:1753-1754`) with `let reprobe = match (stale, reprobe) {` at **`:1756`**, and pushed at
    **`:1758`**.
15. **Amended.** `background_len` has **23 read sites** across **2** files: 21 in `agent_worker.rs`
    (`:5003`, `:5066`, `:5118`, `:5160`, `:5195`, `:5270`, `:5408`, `:5442`, `:5521`, `:5540`,
    `:5562`, `:5673`, `:5813`, `:6625`, `:6867`, `:7907`, `:7922`, `:8138`, `:8218`, `:8239`,
    `:8258`) and 2 in `crates/htui/tests/prompt_preview.rs` (`:587`, `:657`). Every one is a test;
    no production caller. **Note on the count:** the fact-check reported 24 (22 in
    `agent_worker.rs`), but a literal text search for `background_len()` returns exactly the 23 sites
    above — the 24th reference is the **definition** at `:627`, not a read site. Classification by
    value: **16 assert `== 0`** (invariant under the split) and **7 assert `== 1`** — 5 asserting a
    **writing** task (`:5003`, `:5118`, `:5442`, `:5562`, `:5813`), 1 a **reading** task
    (`prompt_preview.rs:657`, the `R-NF-3` pin), and 1 a **synthetic held** task that is neither
    (`:8239`). The plan's original table also misfiled `:5270` as `== 1`; it is `== 0`
    ("a fresh row is not stale, so nothing was spawned on the arm"), which is what makes 16 + 7 = 23
    add up. See the corrected table in the Test plan.
16. `crates/htui/tests/prompt_preview.rs::the_preview_is_deferred_onto_a_task_the_runtime_owns` is at
    `:634-671`; its doc reads "`R-NF-3` as a fact about the runtime rather than about a comment", and
    the assert at `:656-660` is `runtime.background_len() == 1` with the message "one task, owned by
    the runtime".
17. `crates/htui/tests/prompt_preview.rs::the_preview_refuses_offline_with_one_sentence` reads
    `background_len() == 0` at `:587`, with a comment at `:570-572` explaining that the box does not
    pay for a task that could only fail on its first read.
18. `agent_worker.rs::a_held_claim_skips_the_registration_probe_without_a_reply` is at `:8226-8245`;
    it pushes `tokio::spawn(std::future::pending::<()>())` at `:8233-8234`, asserts
    `!runtime.box_probe_running()`, `background_len() == 1` ("the held task is still there") at
    `:8239`, `sent(&mut rx).is_empty()` at `:8240`, and tears down with
    `for task in std::mem::take(&mut runtime.background) { task.abort(); }` at `:8241-8243`. Its doc
    is "R-12: a claim held elsewhere skips the registration probe until the next swap, and says
    nothing".
19. `agent_worker.rs::a_finished_background_task_does_not_stop_the_registration_probe` is at
    `:8205-8221`; it pushes at `:8212`, waits with
    `while !runtime.background.iter().all(JoinHandle::is_finished) { tokio::task::yield_now().await; }`
    at `:8213-8215`, then calls `on_online` and asserts `box_probe_running()` and
    `background_len() == 0` ("the finished task was swept") at `:8218`. Its doc: "Blueprint D27: a
    finished background task (a preview, a chat re-probe) does not hold the claim: `sweep_finished`
    clears it before `on_online` asks."
20. **Amended.** `a_probe_and_an_install_never_write_the_same_row_at_once` is at `:5762-5843`; it
    asserts `message.contains("probe is already running")` at **`:5806`** (the first draft said
    `:5801`) and `message.contains("install is running")` at `:5836` (the *other* direction, a
    sentence this change does not touch), plus `background_len() == 1` at `:5813`. **`:5806` is the
    only assertion in the tree that a rename breaks.** Explicitly *not* broken: `:5836`
    (`"install is running"`, the `self.install` arm), `:5743` (`"already running"`, `self.install`
    arm), `:6778` (`:install` arm), `:6827` and `:6895` (`"a login is already running"`, the
    `self.auth` arm), `agents.rs:1202` (UI-local), `tests/probe.rs:117`,
    `tests/box_settings.rs:890` and `:895` (`BOX_PROBE_RUNNING`, `agent_worker.rs:93`),
    `tests/settings.rs:1269` and `:1989` (UI-local). One **non-test** document quotes the sentence
    and will go stale: `HANDOFF.md:228`, inside the MOD-31 checklist line that lifecycle step 3
    deletes anyway.
21. **Falsified in wording, operative conclusion unchanged.** The string
    `"a probe is already running on this box"` appears in exactly **two** places in the whole tree:
    `agent_worker.rs:1548` and `HANDOFF.md:228`. But the claim that "there is **no UI-side duplicate
    of the guard**" is **false**: there **is** a UI-side gate on `i`, it just is not
    preview-aware — `AgentsSection::begin_install`
    (`crates/htui/src/ui/tabs/settings/agents.rs:743-746`:
    `if self.probing { ctx.emit(Action::Error("a probe is running".to_owned())); return; }`), the `r`
    arm in `on_key` (`:1202`, `Action::Error("a probe is already running")`), and `begin_auth`
    (`:367`) — three sibling checks on the same `self.probing` flag. The runtime's own comment at
    `agent_worker.rs:1541-1544` already documents that this section-side flag is unreliable (any
    `StoreReply::Agents` clears it; `wants_requests` re-issues `Agents` on every activation). **The
    conclusion now stated affirmatively:** `self.probing` is written in exactly two places
    (`agents.rs:1197`, cleared at `:1217`/`:1225`) and is set only by the section's own `r` keypress;
    `AgentsSection::wants_requests` (`:1110-1113`) is exactly `vec![StoreRequest::Agents]`;
    `begin_install` (`:729-762`) checks `install_in_flight()`, `auth_in_flight()`, `self.probing`,
    `selected()` and `declares_a_source()` and nothing about `PromptPreview`. So a preview passes
    the UI gate and is refused by the runtime — the bug reproduces exactly as the item says, and the
    runtime fix is sufficient.
22. **Amended.** **D102's provenance**: the decision text is
    `.claude/plans/mod-2-prompt-assembler.plan.md:519` ("It writes nothing — no `set_step_prompt`,
    no `prompt` event, no run."), restated at `:52` and `:76` of the same file (D102 also appears at
    `:735`, `:865`), and archived at `docs/decisions/mod/mod-2.md:173` under
    "The preview (D102, D103)". **The first draft's gitignore caveat was false:** that plan file
    **is present on disk and is git-tracked** (`git check-ignore` exits 1; `git ls-files` returns
    it). The runtime's code comment citing D102 is at `agent_worker.rs:1249-1253`, with the
    `writes nothing` line at `:1252` (the first draft said `:1251-1254`) — a literal search for
    "the preview writes nothing" misses it only because the phrase wraps across `:1251`/`:1252`.
23. **Amended.** `R-AGT-10` is `docs/REQUIREMENTS.md:194-201`; `R-TUI-8` is **`:317-321`** (the
    first draft said `:318-322`); `R-NF-3` is `:347-348`. A requirement-changelog line at
    `docs/REQUIREMENTS.md:6` records "R-TUI-1 and R-TUI-8 amended in place" ✓. **Framing caveat:**
    none of the three speaks to *admission control* of one long operation against another —
    `R-NF-3` is about *where* work runs, `R-AGT-10` about install provenance and consent, `R-TUI-8`
    about the Settings section's surface. "A preview must not block an install" is a **correctness
    and race argument, not a requirement-quotation argument**; the plan's framing is softened to
    match.
24. **Amended.** The MOD-31 checklist line is **`HANDOFF.md:225-235`** (the first draft said
    `:225-237`; `:236` starts MOD-32); the summary table row listing it is `HANDOFF.md:748` ✓. The
    live-coordinate pins (`store CASES 77`, `READ_CASES 14`, `htui-orch CASES 72`, `GraphSource 7`,
    `StoreRequest 69`, `StoreReply 40`, `REQUEST_NAMES 13`, `268 .sqlx`, `88 snapshots`,
    `MIRRORED_TABLES 21`, migrations `0001`..`0007`, next `0008`) are all **current and verified**,
    but the pins are at **`HANDOFF.md:36-42`** (the first draft said `:39-46`), and the six
    `cargo doc` baseline errors are at **`:44`** (the first draft said `:47-51`).
25. **Amended — the tally is dropped.** The authoritative gate is documented at
    **`README.md:467-473`** ("## Tests" — the same three commands), with the
    `HTUI_TEST_DATABASE_URL` / `docker compose up -d` form at `README.md:475-485`. The house plans
    in `.claude/plans/` are a restatement of it and agree with it, so this plan cites README as the
    origin and the plans as the house spelling. The first draft's occurrence tally was wrong on all
    three counts (it said 35 / 23 / 16; the true counts across `.claude/plans/` are 57 / 35 / 24) and
    is **removed**: a tally is a citation, not a gate. The gate itself is
    `cargo fmt --all -- --check`;
    `cargo clippy --workspace --all-features --all-targets -- -D warnings`;
    `USERNAME=htui-ci HTUI_TEST_DATABASE_URL=postgres://postgres:htui@localhost:5439/postgres cargo
    test --workspace --all-features -- --test-threads=1`. There is **no** gate script and no gate
    command in `AGENTS.md`, `.claude/rules/workflow-docs.md` or `.claude/workflow-config.json` (the
    latter is `{"reviewer": "rust-reviewer"}` only).
26. `previews` stores the `AbortHandle` taken at `preview:1254` (doc sentence at `:396-397`),
    **before** the handle is moved into `background` at `:1257`; under D1 the same order is
    required, so no change to `previews`' shape is needed.
27. **Falsified.** `LiveInstall { agent_id, phase, cancel, task }` (`agent_worker.rs:248`) and
    `LiveAuth { agent_id, cancel, commands, task, _claim }` (`:312`) are **`pub struct`, not
    private** — and they are referenced only inside this file, so `pub` buys nothing today. Only
    `LivePhase` (`:235`, a private enum with `Planning`/`Installing`) is private. **This matters:**
    D1's acceptance criterion is "no other module can construct or read a `Background`", and
    `Background`/`Writes` must therefore be genuinely private (not `pub`) for it to hold. D1 is
    kept as written; only the citation is repointed, at the real private idioms in this file:
    `LivePhase` (`:235`), `ChatBinding` (private tagged enum, `:1790`, `Fresh` / `Promoted { .. }`),
    `Frames` (private handle struct, `:2940`) and `ReprobeClaim` (private RAII struct, `:2256`).
28. T0, T1 and T2 all touch `crates/htui/src/agent_worker.rs`, so they are **serial**; the wave
    structure is one lane. The only other file named, `crates/htui/tests/prompt_preview.rs`, is a
    verification read in T2.
29. **Amended.** This change adds no `StoreRequest`/`StoreReply` variant, no store method, no
    `query!`, no migration, no snapshot and no UI frame. All four paths named in the plan's original
    `git diff --stat` line exist and the counts match (`.sqlx` 268, `crates/htui/tests/snapshots`
    88, `migrations` 7, `cache_migrations` 4), and `## Files to Change` names only
    `crates/htui/src/agent_worker.rs` (edit) and `crates/htui/tests/prompt_preview.rs` (verify
    only) — neither is SQL, so skipping `cargo sqlx prepare` is right (`README.md:492-511` makes
    regeneration conditional on a changed query). **Addition: there is a fifth snapshot directory,
    `crates/htui/src/snapshots` (1 entry, `README.md:515`),** which the original diff line missed;
    it is now included.
30. **Amended.** `.claude/rules/workflow-docs.md` exists, and its `paths:` frontmatter governs
    `HANDOFF.md`, `DECISIONS.md`, `docs/decisions/**` **and `docs/ANA-*.md`** (the original claim
    omitted the last). Lifecycle step 3 reads verbatim:

    > On completion: delete the checklist line from HANDOFF.md, write the detailed writeup + commit
    > hash to `docs/decisions/<prefix>/<prefix>-N.md`, prepend its index line to the top of
    > DECISIONS.md (reverse-chronological, newest first), and update HANDOFF's summary table + top
    > status line.

    **The original plan omitted the last clause.** So close-out is four edits, not two: delete the
    MOD-31 checklist line; write `docs/decisions/mod/mod-31.md` (the directory exists with 8 files,
    none named `mod-31.md`) plus one prepended `DECISIONS.md` index line; **drop the
    `MOD-N | 41 (…)` summary row at `HANDOFF.md:748`, which names MOD-31**; and **touch the top
    status line**. The plan's "P1"/"P2" framing was its own labelling, not the rule's: the rule
    requires one write-up file per resolved item, and MOD-31 is single-milestone, so it archives
    immediately. No implementer touches those files; they are the main thread's.

---

## Verified claims

The result of a three-way fact-check against the working tree at `b1f9b3e`. Verdicts are exactly
`verified`, `amended` or `falsified`. **Design A, D1–D7, OQ-1/2/3's recommended defaults, the task
structure and the wave structure all survive unchanged** — this pass corrected facts and citations,
not the design.

**Tally: 30 claims — 9 `verified`, 18 `amended`, 3 `falsified`.** The three falsified ones are
**claim 27** (substantive: `LiveInstall`/`LiveAuth` are `pub`, not private), **claim 21** (wording:
a UI-side duplicate of the guard does exist) and **claim 1** (wording: HEAD is `b1f9b3e`). No
falsified claim changes the design.

### The 30 claims

| # | Verdict | Evidence |
|---|---|---|
| 1 | **falsified** | HEAD is `b1f9b3e`, not `68c058f`; the Rust source is byte-identical to `main`@`68c058f` and the only diff is the plan file. Rephrased in the header. |
| 2 | verified | `AgentRuntime` `:372-446`, `background` `:384` (doc `:377-383`), `previews` `:405` (doc `:396-397`), `box_probe` `:435`. |
| 3 | amended | `background_len` body `:627-629`; its **doc is at `:625-626`**, and the plan quoted only the leading clause. |
| 4 | verified | `finish_background` `:641-681`, the `mem::take` loop at `:645`. |
| 5 | amended | `serve` calls `sweep_finished` at **`:889`**, not `:892`. |
| 6 | amended | `shutdown`: `self.previews.clear()` is **`:1099`**; the take + `abort` is `:1100-1102`. |
| 7 | amended | Arms at **`:1522` / `:1528` / `:1534`**, `!self.background.is_empty()` `:1546`, refusal string **`:1548`**. Plan said 1526/1533/1540 — 4–6 lines high. |
| 8 | amended | Five call sites confirmed. `:523` is `if let Err(err) = self.claim_is_free() {`; the other four are `self.claim_is_free()?;` → `failed("probe_box" \| "install_plan" \| "install_confirm" \| "auth_start", &err)`. No test calls it; literal search returns 10 hits. |
| 9 | amended | Inline guards at **`:1126` / `:1135` / `:1142`**, push `:1165`; plus `probe` takes a `Writer` at **`:1147`**. Plan said 1127/1137/1143. |
| 10 | verified | Four `background.push` sites: `:1165`, `:1257`, `:1758`, `:8212`. |
| 11 | amended | Confirmed **from code, not comments**: `run_probe` `:1979-1995` → `probe_agents_on` `:2003-2041` skips `!enabled` and upserts only on `ProbeOutcome::Row`; `run_reprobe` `:2325-2370` upserts exactly once. "Every agent row" is loose. |
| 12 | amended | Offline arm **`:1234-1240`**, spawn **`:1241-1248`**, D102 comment **`:1249-1253`**, `previews.insert` **`:1254`**, push `:1257`, `Served::Deferred` `:1258`. |
| 13 | amended | "Reaches no write method" **confirmed**; "holds no `Writer`" **false** (`build` calls `backend.writer()` → `writer.repos(...)`, a SELECT). Restated in the weaker, honest form. |
| 14 | amended | Re-probe gate **`:1754-1755`**, `let reprobe = match (stale, reprobe) {` `:1756`, push `:1758`. |
| 15 | amended | See the count note below — **23 read sites**, not 24; `:5270` reclassified from `== 1` to `== 0`. |
| 16 | verified | `the_preview_is_deferred_onto_a_task_the_runtime_owns` `:634-671`, assert `:656-660`. |
| 17 | verified | `the_preview_refuses_offline_with_one_sentence`, `== 0` at `:587`, comment `:570-572`. |
| 18 | verified | `:8226-8245`; push `:8233-8234`, `== 1` at `:8239`, teardown `:8241-8243`. |
| 19 | verified | `:8205-8221`; push `:8212`, wait `:8213-8215`, `== 0` at `:8218`. |
| 20 | amended | Refusal assert at **`:5806`**, not `:5801` — and it is the **only** assertion a rename breaks; a "not broken" list was added. |
| 21 | **falsified** | The UI-side gate **does** exist (`agents.rs:743-746`, `:1202`, `:367`). Conclusion restated affirmatively and is unchanged: a preview passes the UI gate and is refused by the runtime. |
| 22 | amended | The mod-2 plan **is present and git-tracked**; the gitignore caveat is removed. Code comment at **`:1249-1253`** (`writes nothing` at `:1252`). |
| 23 | amended | `R-TUI-8` is at **`:317-321`**, not `:318-322`; plus a framing caveat on admission control. |
| 24 | amended | Checklist line **`:225-235`**, pins **`:36-42`**, `cargo doc` errors **`:44`**. Values themselves all current. |
| 25 | amended | The occurrence tally was wrong on all three counts and is **dropped**; `README.md:467-473` / `:475-485` is now cited as the origin. |
| 26 | verified | `AbortHandle` taken at `preview:1254`, before the move at `:1257`. |
| 27 | **falsified** | `LiveInstall` (`:248`) and `LiveAuth` (`:312`) are **`pub struct`**. Precedent repointed at `LivePhase` `:235`, `ChatBinding` `:1790`, `Frames` `:2940`, `ReprobeClaim` `:2256`. |
| 28 | verified | T0/T1/T2 are serial; `prompt_preview.rs` is a verification read. |
| 29 | amended | All four counts verified; a **fifth** snapshot directory, `crates/htui/src/snapshots` (1 entry, `README.md:515`), was added to the diff line. |
| 30 | amended | `paths:` also covers `docs/ANA-*.md`; lifecycle step 3 quoted verbatim, including the **omitted** summary-table + top-status-line clause. |

### The F-items

| Item | Verdict | Note |
|---|---|---|
| F1 | **falsified** | `pub`, not private. Patterns table repointed; D1 kept as written, with a counter-example row added. |
| F2 | **falsified** | A UI-side pre-flight exists (three sites). Conclusion unchanged, now affirmative. |
| F3 | amended | "Reaches no write method" is the honest form of the guard's safety argument. |
| F4 | amended | Call forms and the 10-hit text search added. Gortex `callers` is incomplete (3 vs 5). |
| F5 | amended | `claim_is_free` arms corrected to `:1522/:1528/:1534`, string `:1548`. |
| F6 | amended | `probe` guards corrected to `:1126/:1135/:1142`; the `:1147` writer noted. |
| F7 | amended | `preview` ranges corrected; the line-wrap explanation recorded. |
| F8 | amended | Eight line numbers corrected across `serve`, `start`, `run_reprobe`, `shutdown`, `previews`, `background_len`. |
| F9 | amended | "For every agent row" tightened to per-enabled-row-yielding-`Row`. |
| F10 | amended | **Applied in part** — see the count note below. |
| F11 | amended | `:5806`; "not broken" list added; `HANDOFF.md:228` noted as the stale non-test document. |
| F12 | amended | Toolchain probe green; tally dropped, README cited. |
| F13 | amended | `crates/htui/src/snapshots` added as the fifth path. |
| F14 | amended | Rule quoted verbatim; "P1"/"P2" framing corrected. |
| F15 | amended | `R-TUI-8` line fixed; framing softened to a correctness/race argument. |
| F16 | amended | Four HANDOFF line references corrected. |
| F17 | amended | Gitignore caveat removed. |
| F18 | amended | Header rephrased. |
| F19 | amended | Gortex/tooling caveat and both methodological facts added to the header. |

### One count the fact-check got wrong (F10)

The fact-check reported **24** read sites (22 in `agent_worker.rs`, 2 in `prompt_preview.rs`) and
said one was missing from the plan's table. A literal text search for `background_len()` — the
method the fact-check itself mandates for call-site counts — returns exactly **23**: 21 in
`agent_worker.rs` and 2 in `prompt_preview.rs`, all of which the plan's table already enumerated.
The 24th reference is the **definition** at `:627`, not a read site.

What *was* wrong, and what F10's own classification exposed, is the plan's table row for
**`agent_worker.rs:5270`**: it recorded `== 1` ("a spawn failure re-probes off the arm") when the
line is in fact

```rust
assert_eq!(
    runtime.background_len(),
    0,
    "a fresh row is not stale, so nothing was spawned on the arm"
);
```

Correcting it to `== 0` makes the arithmetic work exactly as F10 describes — **16 `== 0` sites and 7
`== 1` sites, 16 + 7 = 23** — and the `== 1` breakdown (5 writing, 1 reading, 1 synthetic) then fits
with no residue. F10's classification was applied in full; only its headline total was not.

### Toolchain probe results

Run on this box at `b1f9b3e`, in the worktree, before applying this pass:

| Command | Result |
|---|---|
| `cargo fmt --all -- --check` | **exit 0** |
| `cargo clippy --workspace --all-features --all-targets -- -D warnings` | **exit 0** — 2m00s cold build, **zero** warnings |
| `cargo test -p htui --lib agent_worker -- --test-threads=1` | **72 passed, 0 failed**, 10.58s |
| `cargo test --workspace --all-features -- --test-threads=1` | **deliberately not run** — out of scope for a fact-check; the implementer runs the full gate |

Postgres is reachable at `localhost:5439` via the compose `htui-postgres` container (`postgres:16`,
`5439:5432`), and the gate's `HTUI_TEST_DATABASE_URL` DSN is confirmed correct. So the gate
invocation in `## Validation` is executable as written on this machine.
