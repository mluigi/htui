# Plan: MOD-31 — a running prompt preview makes an adapter install refuse

**Source**: `HANDOFF.md:225-237` (MOD-31, from MOD-2 finding F-121). Found at MOD-2 close-out,
2026-09-15. **Routing**: no PRD, no blueprint — routed as **plan** (one field split, one guard, one
refusal sentence; below C3's threshold). **Base**: `68c058f` (merge of MOD-7 milestone 4), tree
identical to `main` at drafting. Every line number below is that commit's.

**Requirements**: `R-AGT-10` (`docs/REQUIREMENTS.md:194-201` — "An agent whose adapter is not
installed on a box can be installed **from the app** … The app re-probes after installing, so what
the box can run is always the probe's answer"), `R-TUI-8` (`:318-322` — the Settings tab carrying the
agent registry where `i` lives), `R-NF-3` (`:347-348` — "All long operations … run off the UI
thread; the TUI never blocks on network or subprocess I/O"). `R-AGT-10` is the requirement the bug
breaks: an install the user can see and ask for is refused by a task that writes nothing.

**Why it matters beyond the sentence.** `AgentRuntime::preview` (`agent_worker.rs:1225-1259`) pushes
the deferred preview task into `self.background` at `:1257`. `claim_is_free` (`:1519-1552`) refuses
whenever `!self.background.is_empty()`, with *"a probe is already running on this box; install once
it has finished"*. `background` is one collection holding three different kinds of work: the agent
probe (`run_probe`, **writes** `agent_box` for every agent row on the box), the chat staleness
re-probe (`run_reprobe`, **writes** `agent_box` for one row, pushed from `start` at `:1758`), and the
preview (`run_preview`, **writes nothing** — MOD-2 plan D102, `.claude/plans/mod-2-prompt-assembler.plan.md:519`,
"the preview is a sixth Backlog detail sub-tab, `Prompt`, answered by a new `StoreRequest::PromptPreview`
… It writes nothing", archived at `docs/decisions/mod/mod-2.md:173`; the code says so in `preview`'s
own comment at `agent_worker.rs:1251-1254` and `run_preview` (`crates/htui/src/preview.rs:368-392`)
holds no `Writer` at all).

**Worse than the item says.** `claim_is_free` has **five** callers, not the three the HANDOFF
credits: `on_online` (`:523`, the registration probe on an `Online` swap), `probe_box` (`:1192`),
`install_plan` (`:1287`), `install_confirm` (`:1330`) and `auth_start` (`:1402`). A live preview
therefore also blocks `r` (the registration probe on demand), the **login** path, and — worst — the
registration probe an `Online` swap wants to start, which `on_online` skips **silently**: the only
trace is a `tracing::info!` at `:525`, and nothing was recorded, so the box stays unprobed until the
next swap. One guard fix repairs all five, because they share the guard.

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
  `writing_background_len()` for the new tests.** All 23 existing assertions then keep their meaning
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
`j` in the Backlog detail — a preview that reads five tables, walks a filesystem and writes nothing
— locks `i` in Settings for as long as it takes, and also silently costs a `ProbeBox` and a login,
with a sentence that names a probe that is not running.

**The split (T0).** A private `Background` struct in `agent_worker.rs` carries the handle and a
`Writes` tag, and is built only through `Background::writing(task)` and `Background::reading(task)`.
The four push sites each state their property: `probe` (`:1165`) and `start` (`:1758`) write,
`preview` (`:1257`) reads, and the test placeholder at `:8212` becomes a constructor call. The three
lifecycle readers — `sweep_finished`'s `retain` (`:1022`), `finish_background`'s `mem::take` loop
(`:645`), `shutdown`'s `mem::take` + `abort` (`:1100`) — each lose one deref and gain a
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
| D7 | **The scope is `claim_is_free` and nothing else.** `AgentRuntime::probe` (`:1117-1173`) does **not** call `claim_is_free` and does not consult `background` at all — it checks `self.auth`, `self.install` and `self.box_probe_running()` inline, then pushes. So `ProbeAgents` is unaffected by the preview today and stays unaffected by this change, and two `ProbeAgents` may already run concurrently. That asymmetry is recorded, not fixed: fixing it means adding the writing half to `probe`'s own inline checks, which is a behaviour change to a path no item asks about. | Scope. Recorded as disagreement 4 below. |

## Patterns to Mirror

| Concern | Mirror | Where |
|---|---|---|
| A private handle wrapper with named constructors beside its owner | `LiveInstall` / `LiveAuth` (private structs in the same file, one field of the runtime each) | `crates/htui/src/agent_worker.rs:412-433` (`installer`/`install`/`auth` field docs), the structs themselves below the `impl` block |
| A private tag enum that names a property rather than a bool | `LivePhase::Installing` / `LivePhase::Planning` | `crates/htui/src/agent_worker.rs:1299`, `:1348` (the two `self.install = Some(LiveInstall { … phase: … })` sites) |
| The three lifecycle readers of `background` | `sweep_finished`, `finish_background`, `shutdown` | `agent_worker.rs:1016-1036` (`:1022` the `retain`), `:641-681` (`:645` the `mem::take` loop), `:1066-1108` (`:1100` the take + `abort`) |
| A guard that is a named method every caller shares | `claim_is_free` and its five call sites | `agent_worker.rs:1519-1552`; callers at `:523`, `:1192`, `:1287`, `:1330`, `:1402` |
| A refusal that names what holds the claim, in the other direction | `probe`'s own inline arms ("a login is running for agent …", "an install is running for agent …") and `BOX_PROBE_RUNNING` | `agent_worker.rs:1127-1146`; the constant is used at `:1527` and asserted by value at `:8131` |
| A test that reaches into a private field to stage a holder | `a_held_claim_skips_the_registration_probe_without_a_reply`, `a_finished_background_task_does_not_stop_the_registration_probe` | `agent_worker.rs:8226-8245`, `:8205-8221` (both in the inline `mod tests`) |
| A test that pins the refusal **words** | `a_probe_and_an_install_never_write_the_same_row_at_once` (`message.contains("probe is already running")`) | `agent_worker.rs:5762-5843`, the assert at `:5801` |
| A test that pins `R-NF-3` as a fact about the runtime | `the_preview_is_deferred_onto_a_task_the_runtime_owns` | `crates/htui/tests/prompt_preview.rs:634-671` (`background_len() == 1` at `:657`) |
| An inline test fixture that makes an install real | `installing_runtime`, `unresolvable_registry`, `install_row`, `Fixture::start()` | used by `a_probe_and_an_install_never_write_the_same_row_at_once` (`agent_worker.rs:5762-5786`) |

## Files to Change

| File | Action | Task | Why |
|---|---|---|---|
| `crates/htui/src/agent_worker.rs` | edit | T0, T1, T2 | `Background` (D1), the field and its doc (`:384`), four push sites (D2), three lifecycle readers, `background_len` (D3), `writing_background_len` (D4), `claim_is_free` (D5, D6), and every inline test that touches the collection |
| `crates/htui/tests/prompt_preview.rs` | **verify only** | T2 | read `the_preview_is_deferred_onto_a_task_the_runtime_owns` (`:634-671`) and `the_preview_refuses_offline_with_one_sentence` (`:570-590`) after D3; **no edit expected** under OQ-2's default |

**Not touched, on purpose:** `crates/htui/src/preview.rs` (its `run_preview` already writes nothing —
D1 only records that, in a comment, inside `agent_worker.rs`); `crates/htui/src/store_worker.rs`;
every Settings section (no UI-side pre-flight on `i` exists — the string is in exactly two places in
the tree, `claim_is_free` and `HANDOFF.md`, verified); every migration, `cache_migrations/` and
`crates/htui-store/tests/migrations.rs`; `crates/htui-store/.sqlx/`; every snapshot; `AgentRuntime::probe`
(D7); `docs/**`, `HANDOFF.md`, `DECISIONS.md`.

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
  `task.abort_handle()` first (it already does, at `:1253`).
- **Validate**: `cargo test -p htui --all-features -- --test-threads=1`;
  `cargo clippy -p htui --all-targets --all-features -- -D warnings`.

### Task 1: the guard consults the writing half (D5, D6, D7)
- **Files**: `crates/htui/src/agent_worker.rs` only.
- **Tests first.** `a_probe_and_an_install_never_write_the_same_row_at_once` (`:5762-5843`) has its
  refusal assert at `:5801` re-pointed from `message.contains("probe is already running")` to
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

**`background_len()` — all 23 read sites, and what each one asserts.** Verified by
`relations(usages)`: 23 reference edges across exactly two files (21 in `agent_worker.rs`,
2 in `prompt_preview.rs`). Under D3 **none of them moves**.

| Site | Asserts | Task under D3 |
|---|---|---|
| `agent_worker.rs:5003` | `background_len() == 1` after a `ChatStart` spawned a staleness re-probe | unchanged (a writing task) |
| `:5066` | `== 0` — no staleness re-probe while a box probe runs | unchanged |
| `:5118` | `== 1`, "a 25 h old row is stale" | unchanged |
| `:5160`, `:5195` | `== 0` — a `cli` row and a fresh row are not re-probed | unchanged |
| `:5270` | `== 1` — a spawn failure re-probes off the arm | unchanged |
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
4. `agent_worker.rs:5801` — the refusal-word assert in
   `a_probe_and_an_install_never_write_the_same_row_at_once` (T1). This is the **only** test in the
   tree that pins the sentence's words (the tree-wide search for `"a probe is already running on
   this box"` returns exactly two hits: the guard at `:1548` and `HANDOFF.md:228`; the
   `"install is running"` / `BOX_PROBE_RUNNING` asserts in the *other* direction are on sentences
   this change does not touch, at `:5836` and `:8131`).

**Snapshots.** None move. No frame renders a refusal sentence (the sentences reach the user through
the generic status line) and no `prompt_preview` frame renders a task count. T2 verifies this by
running the suite, not by assertion.

**Test-list pins that do not move.** store `CASES` 77, `READ_CASES` 14, `htui-orch` `CASES` 72,
`StoreRequest` 69, `StoreReply` 40, `hierarchy::REQUEST_NAMES` 13, 268 `.sqlx` files, 88 snapshots,
`MIRRORED_TABLES` 21 (`HANDOFF.md:39-46`). This item adds no store method, no request, no migration
and no frame, so **the main thread's `HANDOFF.md` bookkeeping is limited to deleting the MOD-31 line
and adding the decision-file line** (`.claude/rules/workflow-docs.md` lifecycle step 3).

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| **R-1** — A future push site is tagged wrong. A task that writes `agent_box` is tagged `reading`, and the guard lets an install run beside it. | Low | D1 makes the tag a constructor name, so a wrong tag is a named, greppable claim (`Background::reading` at a `run_probe` push site) rather than a bare `false`. The reviewer check is a grep for `upsert_agent_box` inside every task named in a `Background::reading` call. The doc on `Background::writing` states the promise explicitly: *the task writes `agent_box` for at least one row, so `claim_is_free` must see it*. |
| **R-2** — A task that starts reading and later starts writing. | Low, and it is a real shape: `run_preview` is a `Backend` clone that could grow a writer. | The tag is declared once, at the push site, from what the task does **today**. D1's doc makes the promise per-constructor; the T2 case `a_running_preview_does_not_refuse_an_install` and the existing `run_preview` source (`preview.rs:368-392`, no `Writer`) are the two things a future change to `run_preview` would have to break. Mitigation for that future: the guard's arm is one predicate, so promoting the preview to writing is a **one-word** change at one call site, which is the property this design is for. |
| **R-3** — Something outside `claim_is_free` assumed `background` was uniformly writing. | None found | Full read: the only readers of the field are `background_len` (`:627`), `finish_background` (`:645`), `sweep_finished` (`:1022`), `shutdown` (`:1100`) and `claim_is_free` (`:1546`); the other two inline tests reach it directly (`:8212`, `:8233`). `previews` (`:405`) holds an `AbortHandle` keyed on `Origin`, not a handle, and its doc already says the map "owns the *right to cancel*, not the task". The field is private to the struct and the struct is `pub`, but the field is not `pub`, so no other module can read it — verified by the fact that only the inline `mod tests` (same file) touches it. |
| **R-4** — The narrowed guard lets an install's re-probe race a **running preview** on the same row. | None | A preview writes no `agent_box` row at all (`R-NF-3` / MOD-2 D102). There is no row to race on. This is the item's own argument and it holds. |
| **R-5** — The fix is narrower than the symptom, and a user still sees a refusal they do not understand. | Medium | This is OQ-3's whole subject, and D6 is the mitigation: after the change the only tasks in the set are a probe and a re-probe, and the sentence names both. If the maintainer prefers the OQ-3 alternative (leave the sentence), record that the sentence is knowingly inexact. |
| **R-6** — The change is mistaken for permission to let `ProbeAgents` run beside an install. | Medium | D7: `probe` never consulted `background` and still does not, so nothing about `ProbeAgents` changes. Say so in the close-out, and leave the `ProbeAgents`-does-not-share-the-claim asymmetry for a separate item if it is wanted. |

## Validation

This change touches **no SQL of any kind** — no migration, no `cache_migrations/`, no `query!`, no
store method, no `crates/htui-store/.sqlx/` entry. **Skip `cargo sqlx prepare` entirely.**

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-features --all-targets -- -D warnings
USERNAME=htui-ci HTUI_TEST_DATABASE_URL=postgres://postgres:htui@localhost:5439/postgres \
  cargo test --workspace --all-features -- --test-threads=1
ls crates/htui-store/.sqlx | wc -l                      # 268, unchanged
ls crates/htui/tests/snapshots | wc -l                  # 88, unchanged
cargo doc --workspace --no-deps --keep-going            # exactly the six baseline errors (HANDOFF.md:47-51)
git diff --stat 68c058f -- crates/htui-store/migrations crates/htui-store/cache_migrations \
  crates/htui/tests/snapshots crates/htui-store/.sqlx   # empty
```

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
      no other module can construct or read a `Background`.
- [ ] `background_len()` is unchanged and all 23 of its read sites pass without amendment; the
      `R-NF-3` pin at `crates/htui/tests/prompt_preview.rs:657` still asserts `1`.
- [ ] No snapshot, no migration, no `.sqlx` entry, no store `CASES` pin and no `StoreRequest` /
      `StoreReply` variant moves; the pin counts in `HANDOFF.md:39-46` are unchanged.
- [ ] The gate above is green, and the diff over `migrations`, `cache_migrations`, `snapshots` and
      `.sqlx` is empty.
- [ ] `HANDOFF.md`'s MOD-31 line is deleted and the close-out write-up is under
      `docs/decisions/mod/mod-31.md` with its `DECISIONS.md` index line (main thread, per
      `.claude/rules/workflow-docs.md`).

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
5. **`R-AGT-10` is the requirement this breaks, and the item does not cite it as the broken one.**
   The item lists `R-AGT-10`, `R-TUI-8`, `R-NF-3` as a set; `R-AGT-10`'s text (`:194-201`) is the one
   an install refused by a preview violates ("can be installed **from the app**"), `R-TUI-8` names the
   section where the refusal surfaces, and `R-NF-3` is what makes the preview *exist* as a background
   task in the first place and what the split must not break.
6. **D102's provenance is a MOD-2 plan decision, not a `docs/` decision.** "the preview writes
   nothing" is `.claude/plans/mod-2-prompt-assembler.plan.md:519` (D102) and is archived at
   `docs/decisions/mod/mod-2.md:173`. The **code** assertion is stronger and current: `run_preview`
   (`crates/htui/src/preview.rs:368-392`) holds a `Backend` clone and sends one reply; it is
   `build` that reads. Note that MOD-7 milestone 4 changed `build` to read repo paths
   (`docs/decisions/mod/mod-7.md`; see the plan's D120) — still reads, still no writer. The
   `agent_worker.rs:1251-1254` comment that cites D102 is the one to keep in step.

---

## Claims to verify

Every checkable fact this plan asserts, for the fact-check pass. Line numbers are at `68c058f`.
Found through the Gortex index (symbol source, callers, usages, tree-wide text search) and direct
reads of the non-indexed `.claude/`, `docs/` and `HANDOFF.md`.

1. `HEAD` of `htui-mod-31` is `68c058f`, the same tree as `main`; `graphify-out/` does not exist.
2. `AgentRuntime` is at `agent_worker.rs:372-446`; `background: Vec<JoinHandle<()>>` is declared at
   `:384` with the doc at `:377-383` ("Tasks this runtime spawned that answer a request of their own:
   today the probe's (MOD-2 D53)"); `previews: HashMap<Origin, AbortHandle>` at `:405` with the
   doc sentence "An entry names a task that is also in `background` — this map owns the *right to
   cancel*, not the task."; `box_probe: Option<JoinHandle<()>>` at `:435`.
3. `background_len` is at `:627-629`; body is `self.background.len()`; doc "The one fact a test
   needs to prove D52: a refused probe spawned **nothing**."
4. `finish_background` is at `:641-681`; it clears `self.previews` first, then
   `for handle in std::mem::take(&mut self.background)` at `:645`, aborting on a timeout, and only
   then takes `box_probe`, `install` and `auth` as four separate arms.
5. `sweep_finished` is at `:1016-1036`; the `background` line is
   `self.background.retain(|task| !task.is_finished());` at `:1022`; the same method also retains
   `live`, `previews` and `take_if`s `install`, `auth` and `box_probe`. It is called from
   `serve` (`:892`, the first statement of the body) and from `on_online` (`:522`).
6. `shutdown` is at `:1066-1108`; `for task in std::mem::take(&mut self.background) { task.abort(); }`
   at `:1100-1102`, after `self.previews.clear()`.
7. `claim_is_free` is at `:1519-1552`. Its arms in order: `self.box_probe_running()` (`:1526`,
   `BOX_PROBE_RUNNING`), `self.auth` (`:1533`), `self.install` (`:1540`), and
   `if !self.background.is_empty()` (`:1546`) returning
   `StoreError::Backend("a probe is already running on this box; install once it has finished")`
   (`:1547-1549`).
8. `claim_is_free` has exactly **five** call sites: `on_online` `:523`, `probe_box` `:1192`,
   `install_plan` `:1287`, `install_confirm` `:1330`, `auth_start` `:1402`. Each is inside an
   `if let Err(err) = … { return }` or a `?` that maps to a `StoreReply::Failed` naming the request.
9. `AgentRuntime::probe` is at `:1117-1173`. It checks `self.auth` (`:1127`), `self.install` (`:1137`)
   and `self.box_probe_running()` (`:1143`) inline, **never calls `claim_is_free`, and never reads
   `self.background` as a guard**; it pushes at `:1165`.
10. There are exactly **four** `background.push` sites: `probe` `:1165` (`run_probe`), `preview`
    `:1257`, `start` `:1758` (`run_reprobe`), and the test
    `a_finished_background_task_does_not_stop_the_registration_probe` `:8212`
    (`tokio::spawn(async {})`).
11. `run_probe` writes `agent_box` for every agent row on the box (the field doc on `background` and
    `claim_is_free`'s own comment both say so; `probe`'s doc: "a probe writes `agent_box` for every
    row, and an install's re-probe writes one of them"). `run_reprobe` writes one row —
    `start`'s comment at `:1742-1748` and `run_reprobe`'s doc at `:2317-2320`.
12. `preview` is at `:1225-1259`; the offline arm returns first (`:1239-1243`); the spawn is at
    `:1245-1252`; `self.previews.insert(origin, task.abort_handle())` at `:1253-1255`;
    `self.background.push(task)` at `:1257`; returns `Served::Deferred` at `:1258`. Its comment at
    `:1250-1254` reads "Aborting is safe at any await point the task is parked on: `run_preview`
    only reads — plan D102's "the preview writes nothing" — so there is no half-finished row to
    leave behind".
13. `run_preview` (`crates/htui/src/preview.rs:368-392`) holds a `Backend` clone, calls `build`, and
    sends one `ReplyEnvelope`. It holds no `Writer` and calls no write method.
14. `start`'s re-probe is gated on `needs_reprobe(..) && !self.box_probe_running()`
    (`agent_worker.rs:1753-1754`) and pushed at `:1757-1758` inside
    `let reprobe = match (stale, reprobe) { (true, Some(args)) => { … } … };`.
15. `background_len` has **23** reference edges across **2** files: 21 in `agent_worker.rs`
    (`:5003`, `:5066`, `:5118`, `:5160`, `:5195`, `:5270`, `:5408`, `:5442`, `:5521`, `:5540`,
    `:5562`, `:5673`, `:5813`, `:6625`, `:6867`, `:7907`, `:7922`, `:8138`, `:8218`, `:8239`,
    `:8258`) and 2 in `crates/htui/tests/prompt_preview.rs` (`:587`, `:657`). Every one is a test;
    no production caller.
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
20. `a_probe_and_an_install_never_write_the_same_row_at_once` is at `:5762-5843`; it asserts
    `message.contains("probe is already running")` at `:5801` and
    `message.contains("install is running")` at `:5836` (the *other* direction, a sentence this
    change does not touch), plus `background_len() == 1` at `:5813`.
21. The string `"a probe is already running on this box"` appears in exactly **two** places in the
    whole tree: `agent_worker.rs:1548` and `HANDOFF.md:228`. There is **no UI-side duplicate of the
    guard**: a tree-wide text search of `crates/htui/src/ui/tabs/settings/agents.rs` for
    `"install once it has finished"` returns 0 matches, and no other module holds the sentence. The
    refusal reaches the user through the generic status line.
22. **D102's provenance**: the decision text is `.claude/plans/mod-2-prompt-assembler.plan.md:519`
    ("It writes nothing — no `set_step_prompt`, no `prompt` event, no run."), it is restated at
    `:52` and `:76` of the same file, and it is archived at `docs/decisions/mod/mod-2.md:173` under
    "The preview (D102, D103)". The runtime's code comment citing it is `agent_worker.rs:1251-1254`.
    A tree-wide text search for the phrase "the preview writes nothing" returns exactly one match —
    `HANDOFF.md:233` — so the code comment's wording is a variant, not a literal quote.
23. `R-AGT-10` is `docs/REQUIREMENTS.md:194-201`; `R-TUI-8` is `:318-322`; `R-NF-3` is `:347-348`.
    A requirement-changelog line at `docs/REQUIREMENTS.md:6` records `R-TUI-8` as amended in place.
24. The MOD-31 checklist line is `HANDOFF.md:225-237`; the summary table row listing it is
    `HANDOFF.md:748`. The live-coordinate pins (`store CASES 77`, `READ_CASES 14`, `htui-orch CASES
    72`, `GraphSource 7`, `StoreRequest 69`, `StoreReply 40`, `REQUEST_NAMES 13`, `268 .sqlx`, `88
    snapshots`, `MIRRORED_TABLES 21`, migrations `0001`..`0007`, next `0008`) are at
    `HANDOFF.md:39-46`; the six `cargo doc` baseline errors at `:47-51`.
25. The gate invocation, copied from the house plans (35 occurrences of `cargo fmt --all -- --check`,
    23 of `cargo clippy --workspace --all-targets --all-features -- -D warnings`, 16 of
    `cargo test --workspace --all-features -- --test-threads=1` across `.claude/plans/`; the MOD-7
    milestone 4 plan's Validation block is the fullest form): `cargo fmt --all -- --check`;
    `cargo clippy --workspace --all-features --all-targets -- -D warnings`;
    `USERNAME=htui-ci HTUI_TEST_DATABASE_URL=postgres://postgres:htui@localhost:5439/postgres cargo
    test --workspace --all-features -- --test-threads=1`. There is **no** gate script and no gate
    command in `AGENTS.md`, `.claude/rules/workflow-docs.md` or `.claude/workflow-config.json` (the
    latter is `{"reviewer": "rust-reviewer"}` only) — the fact-check should confirm the plans really
    are the source of truth here.
26. `previews` stores the `AbortHandle` taken at `preview:1253`, **before** the handle is moved into
    `background` at `:1257`; under D1 the same order is required, so no change to `previews`' shape
    is needed.
27. `LiveInstall { agent_id, phase, cancel, task }` and `LiveAuth { agent_id, cancel, commands, task,
    _claim }` are the private per-runtime handle structs this file already models, and
    `LivePhase::{Planning, Installing}` is the existing private tag enum — the pattern D1 mirrors.
28. T0, T1 and T2 all touch `crates/htui/src/agent_worker.rs`, so they are **serial**; the wave
    structure is one lane. The only other file named, `crates/htui/tests/prompt_preview.rs`, is a
    verification read in T2.
29. This change adds no `StoreRequest`/`StoreReply` variant, no store method, no `query!`, no
    migration, no snapshot and no UI frame; the diff over `crates/htui-store/migrations`,
    `crates/htui-store/cache_migrations`, `crates/htui-store/.sqlx` and
    `crates/htui/tests/snapshots` must be empty, and `cargo sqlx prepare` is not part of the gate.
30. `.claude/rules/workflow-docs.md` governs `HANDOFF.md`, `DECISIONS.md` and
    `docs/decisions/**`; its lifecycle step 3 is what deletes the MOD-31 checklist line and adds the
    `docs/decisions/mod/mod-31.md` write-up plus its `DECISIONS.md` index line. No implementer
    touches those files; they are the main thread's.
