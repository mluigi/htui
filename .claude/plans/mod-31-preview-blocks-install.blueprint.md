# MOD-31 — Implementation blueprint

**Target**: worktree `/media/projects/htui-mod-31`, branch `htui-mod-31`, HEAD `6189661`.
**Base for line numbers**: `68c058f`. Verified `git diff --name-only 68c058f HEAD` returns exactly
`.claude/plans/mod-31-preview-blocks-install.plan.md` — the Rust source is byte-identical.
**Every line number below was re-read in the worktree, not taken from the plan.**

Absolute paths:
- `/media/projects/htui-mod-31/crates/htui/src/agent_worker.rs` (the only file that changes)
- `/media/projects/htui-mod-31/crates/htui/tests/prompt_preview.rs` (verify only)

---

## 0. Design decisions this blueprint does not revisit

D1–D7 stand as settled. The one place an implementer needs the *reason* rather than the decision
is R-1 (a future push site tagged wrong), and it is encoded in the constructor doc below so it is
grep-able rather than remembered.

`Writes` variant names, constructor names and accessor names are exactly as D1/§D4 specify:
`Background::writing`, `Background::reading`, `task()`, `into_task()`, `writes_agent_box()`,
`Writes::AgentBox`, `Writes::Nothing`.

---

## 1. Task order — three serial tasks, one file, one branch

T0 ∩ T1 ∩ T2 = `{agent_worker.rs}`. **They may not run in parallel.** Build order below is the
serial order and each item states the file state it expects on arrival.

| Task | What it changes | Gate before moving on |
|---|---|---|
| **T0** | `Background` + `Writes` + constructors + accessors; field doc; 4 push sites; 3 lifecycle readers; `writing_background_len`; 2 inline tests re-pointed | `cargo test -p htui --lib agent_worker -- --test-threads=1` green |
| **T1** | `claim_is_free`'s last arm + sentence + comment; 1 assert re-pointed | `cargo test -p htui --lib agent_worker -- --test-threads=1` green |
| **T2** | 3 new inline cases; `prompt_preview.rs` verified unamended | `cargo test -p htui --all-features -- --test-threads=1` green |

**T0 is green on its own** with the guard untouched: the probe is tagged `writing`, so
`!self.background.is_empty()` still refuses beside it, and
`a_probe_and_an_install_never_write_the_same_row_at_once` still passes on its old assert. Do not
amend that assert in T0.

---

## 2. Exact edit sites, in build order

Line numbers are the *current* lines in `agent_worker.rs` at `6189661`.

### T0 — commit 1 (RED): re-point the two direct-push tests

**Site T0-a — `agent_worker.rs:8212`**
```rust
        runtime.background.push(tokio::spawn(async {}));
```
→ (T0-a)
```rust
        runtime.background.push(Background::writing(tokio::spawn(async {})));
```

**Site T0-b — `agent_worker.rs:8213`**
```rust
        while !runtime.background.iter().all(JoinHandle::is_finished) {
```
→ (T0-b)
```rust
        while !runtime
            .background
            .iter()
            .all(|entry| entry.task().is_finished())
        {
```

**Site T0-c — `agent_worker.rs:8233-8235`**
```rust
        runtime
            .background
            .push(tokio::spawn(std::future::pending::<()>()));
```
→ (T0-c)
```rust
        runtime
            .background
            .push(Background::writing(tokio::spawn(std::future::pending::<()>())));
```

**Site T0-d — `agent_worker.rs:8241-8243`**
```rust
        for task in std::mem::take(&mut runtime.background) {
            task.abort();
        }
```
→ (T0-d)
```rust
        for entry in std::mem::take(&mut runtime.background) {
            entry.into_task().abort();
        }
```

**Site T0-e — `agent_worker.rs:8202-8203`** (a doc the plan does not name; see §7 item 8)
```rust
    /// Blueprint D27: a finished background task (a preview, a chat re-probe) does not hold the
    /// claim: `sweep_finished` clears it before `on_online` asks.
```
→ (T0-e)
```rust
    /// Blueprint D27: a finished background task does not hold the claim — `sweep_finished` clears
    /// it before `on_online` asks. The entry below is tagged `writing` on purpose, so the sweep is
    /// the only thing that frees the claim here (MOD-31 D2); a `reading` entry would be free
    /// already, and the sweep would be untested.
```

**Commit.** `cargo test -p htui --lib agent_worker -- --test-threads=1` and confirm the **red is a
compile failure**, not a runtime one: `error[E0599]: no function or associated item named `writing`
found for struct `Background``, on all three call sites. This is the honest first red for a
type-level change. **Do not manufacture a runtime failure to precede it.**

### T0 — commit 2 (GREEN): the type and every reader

**Site T0-1 — insert at `agent_worker.rs:457`**, i.e. between the `}` that closes
`impl core::fmt::Debug for AgentRuntime` (456) and `impl AgentRuntime {` (458). Exact text, ready
to paste:

```rust
/// One task this runtime owns in [`background`](Self::background), and what it writes (MOD-31 D1).
///
/// The collection mixes two kinds of work and the guard consults only one of them: a probe and a
/// chat's staleness re-probe both end by writing `agent_box`, and a prompt preview **reaches no
/// write method** (plan D102) — it reads a dozen tables, walks the filesystem and records nothing.
/// So a preview held in the Backlog detail used to refuse an install, a login, a `ProbeBox` and a
/// connect's registration probe, with a sentence about a probe that was not running.
///
/// The tag is a promise and the constructor is where it is made. There is no bool field and no
/// other way in, so a fifth push site has to name what its task does rather than pass a guess —
/// and a reviewer greps for `Background::reading` and checks each against the task it names.
struct Background {
    /// The task. Reached only through [`task`](Self::task) and [`into_task`](Self::into_task), so
    /// nothing outside this module can take the handle and drop the tag.
    task: JoinHandle<()>,
    /// What the task writes, decided once, at the push site that knows the spawned future.
    writes: Writes,
}

/// What a background task writes, and therefore whether it holds the install claim (MOD-31 D1).
///
/// An enum rather than a bool for [`LivePhase`](LivePhase)'s reason: the name is the claim, and
/// `reads: true` at a push site would be a maintainer's guess with nothing to grep for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Writes {
    /// The task ends by writing `agent_box` for at least one row.
    AgentBox,
    /// The task reaches no write method: it reads, and answers on the frame channel.
    Nothing,
}

impl Background {
    /// A task that **writes `agent_box`** for at least one row, so
    /// [`claim_is_free`](Self::claim_is_free) must see it.
    ///
    /// The promise: a task pushed here is one an install's re-probe would race on the same row
    /// (hazard H-10, MOD-21 D19), and the guard is entitled to refuse the install beside it. A task
    /// that writes nothing, or that writes some other table, does not belong here.
    fn writing(task: JoinHandle<()>) -> Self {
        Self {
            task,
            writes: Writes::AgentBox,
        }
    }

    /// A task that **reaches no write method** (plan D102's "the preview writes nothing").
    ///
    /// The promise: the task records nothing, so it holds no claim and leaves no row for an
    /// install to race. Promoting the preview to [`writing`](Self::writing) the day `run_preview`
    /// grows a write is one word at one call site — which is the whole reason the tag lives here
    /// and not in the guard.
    fn reading(task: JoinHandle<()>) -> Self {
        Self {
            task,
            writes: Writes::Nothing,
        }
    }

    /// The task, borrowed: for the sweep, which looks at every entry and keeps most of them.
    fn task(&self) -> &JoinHandle<()> {
        &self.task
    }

    /// The task, by move: for the wait and the abort, which consume the entry.
    fn into_task(self) -> JoinHandle<()> {
        self.task
    }

    /// Whether this task writes `agent_box`, which is the one question
    /// [`claim_is_free`](Self::claim_is_free) asks of the collection.
    fn writes_agent_box(&self) -> bool {
        matches!(self.writes, Writes::AgentBox)
    }
}
```

**Derive / attribute situation, explicitly.**

- `Background`: **no derives at all.** Nothing in the file formats one —
  `impl core::fmt::Debug for AgentRuntime` (448-456) prints `adapters`, `live.len()` and
  `box_probe.is_some()` and never touches `background`; `Served`'s hand-written `Debug` is a
  different type. `clippy::missing_debug_implementations` is a nursery lint and is not on.
  `Background` must not be `Clone` (a second handle to the same task would be a second way in).
  `#[must_use]` does not apply (it has a consuming and a borrowing accessor already).
- `Writes`: derive `Debug, Clone, Copy, PartialEq, Eq` — exactly `LivePhase`'s derive list
  (`agent_worker.rs:234`). Nothing compares it today; clippy does not demand it. Deriving matches
  the file's only other private tag enum and costs nothing.
- Both are **private to the module** — no `pub`. This is D1's acceptance criterion and it is the
  opposite of `LiveInstall`/`LiveAuth` (248, 312), which are `pub` and buy nothing. The `pub` on
  `writing_background_len` below is the only new public name.
- `#![warn(missing_docs)]` at `crates/htui/src/lib.rs:11` reaches `pub` items only. `Background`
  and `Writes` are private, so it does not apply to them; the doc comments are house style, not
  a lint requirement.

**Site T0-2 — `agent_worker.rs:377-384`**, the field and its doc.
Current:
```rust
    /// Tasks this runtime spawned that answer a request of their own: today the probe's (MOD-2
    /// D53). Swept when finished, awaited by [`finish_background`](Self::finish_background),
    /// aborted by [`shutdown`](Self::shutdown).
    ///
    /// The runtime owns the handle for the same reason it owns a chat's: a bare `tokio::spawn`
    /// inside the worker loop would leave a probe with a 60-second handshake running after the UI
    /// is gone, with nobody able to name it.
    background: Vec<JoinHandle<()>>,
```
Replace with:
```rust
    /// Tasks this runtime spawned that answer a request of their own: today the probe's (MOD-2
    /// D53), a chat's staleness re-probe's (plan D55) and the prompt preview's (plan D102). Swept
    /// when finished, awaited by [`finish_background`](Self::finish_background), aborted by
    /// [`shutdown`](Self::shutdown).
    ///
    /// The runtime owns the handle for the same reason it owns a chat's: a bare `tokio::spawn`
    /// inside the worker loop would leave a probe with a 60-second handshake running after the UI
    /// is gone, with nobody able to name it.
    ///
    /// One collection of two kinds of task: some **write `agent_box`**, some only read. The split
    /// is [`Background`]'s tag, stated once at each push site, and
    /// [`claim_is_free`](Self::claim_is_free) consults the writing half — a preview holds no claim,
    /// so holding `j` in the Backlog detail no longer refuses `i` in Settings (MOD-31 D5).
    /// [`background_len`](Self::background_len) counts them all; the half is
    /// [`writing_background_len`](Self::writing_background_len).
    background: Vec<Background>,
```
Intra-doc links from a **private field** to private items are fine here (the existing line 378
already links `[`finish_background`](Self::finish_background)`).

**Site T0-3 — insert at `agent_worker.rs:630`**, between `background_len`'s closing `}` (629) and
`finish_background`'s doc (631). `background_len` itself (622-629) is **byte-identical** — D3.
```rust
    /// How many of those background tasks write `agent_box` — the set `claim_is_free` consults.
    ///
    /// The narrow half of [`background_len`](Self::background_len), and the one a test that says
    /// "a preview holds no claim" needs: a total cannot say whether the entry inside it is one the
    /// guard would refuse on (MOD-31 D4).
    #[must_use]
    pub fn writing_background_len(&self) -> usize {
        self.background
            .iter()
            .filter(Background::writes_agent_box)
            .count()
    }
```
`#[must_use]` is D4's explicit requirement; `background_len` carries one at 626. **Do not
intra-doc-link `Background` or `Writes` from this `pub` item** — that is
`rustdoc::private_intra_doc_links`, a warning, and clippy runs with `-D warnings`. Backticks only,
as above.

**Site T0-4 — `agent_worker.rs:1165-1171`** (`probe`, `run_probe` → **writing**).
Current:
```rust
        self.background.push(tokio::spawn(run_probe(ProbeArgs {
            writer,
            box_id,
            agents,
            env,
            frames: Frames::new(replies.clone(), addr),
        })));
```
Replace with:
```rust
        self.background
            .push(Background::writing(tokio::spawn(run_probe(ProbeArgs {
                writer,
                box_id,
                agents,
                env,
                frames: Frames::new(replies.clone(), addr),
            }))));
```
Nothing else about the arm moves. `probe` takes its `Writer` at 1147 and `run_probe` upserts
`agent_box` for every enabled row that yields `ProbeOutcome::Row`, so `writing` is the truth.

**Site T0-5 — `agent_worker.rs:1257`** (`preview`, `run_preview` → **reading**).
Current:
```rust
        self.background.push(task);
```
Replace with:
```rust
        self.background.push(Background::reading(task));
```
Nothing before it moves: `task.abort_handle()` is still taken at 1254 and inserted into
`self.previews` **before** the handle is moved into its `Background` at 1257 (claim 26). That
order is load-bearing — do not hoist the push.

**Site T0-6 — `agent_worker.rs:1758`** (`start`, `run_reprobe` → **writing**).
Current:
```rust
                self.background.push(tokio::spawn(run_reprobe(args)));
```
Replace with:
```rust
                self.background.push(Background::writing(tokio::spawn(run_reprobe(args))));
```
`run_reprobe` does exactly one `writer.upsert_agent_box(&row)`, so `writing` is the truth.

**Site T0-7 — `agent_worker.rs:1022`** (`sweep_finished`'s `retain`).
Current:
```rust
        self.background.retain(|task| !task.is_finished());
```
Replace with:
```rust
        self.background.retain(|entry| !entry.task().is_finished());
```
**What changes:** the closure parameter is renamed to `entry` because it is no longer a handle, and
`task.is_finished()` loses one deref, becoming `entry.task().is_finished()`. The line above it
(1020-1021) and the four retains/take_ifs below it are unchanged.

**Site T0-8 — `agent_worker.rs:645-651`** (`finish_background`'s loop).
Current:
```rust
        for handle in std::mem::take(&mut self.background) {
            let abort = handle.abort_handle();
            if tokio::time::timeout(limit, handle).await.is_err() {
                abort.abort();
                tracing::warn!(?limit, "a background task did not finish and was aborted");
            }
        }
```
Replace with:
```rust
        for entry in std::mem::take(&mut self.background) {
            let handle = entry.into_task();
            let abort = handle.abort_handle();
            if tokio::time::timeout(limit, handle).await.is_err() {
                abort.abort();
                tracing::warn!(?limit, "a background task did not finish and was aborted");
            }
        }
```
**What changes:** the loop variable becomes an `entry`, and one line is inserted — the
`into_task()` move — because `tokio::time::timeout` needs an owned `JoinHandle`. The bound name
stays `handle`, so the two lines below it are byte-identical. The four arms after the loop
(`box_probe`, `install`, `auth`) are **not** touched, and their **order is load-bearing**: see
§6 landmine L2.

**Site T0-9 — `agent_worker.rs:1100-1102`** (`shutdown`).
Current:
```rust
        for task in std::mem::take(&mut self.background) {
            task.abort();
        }
```
Replace with:
```rust
        for entry in std::mem::take(&mut self.background) {
            entry.into_task().abort();
        }
```
**What changes:** one deref is inserted (`into_task()`) and the loop variable is renamed. The
`self.previews.clear()` at 1099 above it and the `box_probe` abort at 1105-1107 below it are
unchanged.

**After commit 2:** `cargo fmt --all` (the four multi-line push sites are hand-shaped; let rustfmt
decide), then
`cargo test -p htui --lib agent_worker -- --test-threads=1` and
`cargo clippy -p htui --all-targets --all-features -- -D warnings`.

---

### T1 — commit 1 (RED): re-point the refusal-word assert

**Site T1-a — `agent_worker.rs:5805-5808`**
```rust
                assert!(
                    message.contains("probe is already running"),
                    "the refusal names what holds the box: {message}"
                );
```
→ (T1-a)
```rust
                assert!(
                    message.contains("is already writing this box"),
                    "the refusal names what holds the box: {message}"
                );
```

**What makes it red:** the live message at that moment is `"a probe is already running on this
box; install once it has finished"` (1548), which does not contain `"is already writing this box"`.
A genuine runtime red, not a compile error. `request == "install_plan"` at 5804 and
`background_len() == 1` at 5813 are untouched.

**Note for the commit message:** this substring is a deliberate *partial* of the new sentence. It is
shared with the new T2 case `a_running_chat_reprobe_still_refuses_an_install`, so a future
reword of the leading clause does not break this test, while changing the predicate's meaning does.

### T1 — commit 2 (GREEN): the guard

**Site T1-b — `agent_worker.rs:1540-1550`** (comment + arm + sentence). Replace the whole block:
```rust
        // A probe writes `agent_box` for every row, and an install's re-probe writes one of them.
        // Run together, they race on the same row and the last write wins — so the claim covers a
        // probe in flight too. The Settings section's own `probing` flag is not enough: **any**
        // `StoreReply::Agents` clears it (`ui/tabs/settings/agents.rs`, module doc), and
        // `wants_requests` re-issues `Agents` on every activation, so `r` → switch tab → back → `i`
        // reaches here with `run_probe` still running.
        if !self.background.is_empty() {
            return Err(StoreError::Backend(
                "a probe is already running on this box; install once it has finished".to_owned(),
            ));
        }
```
with:
```rust
        // A probe writes `agent_box` for every enabled row, and an install's re-probe writes one of
        // them — and so does a chat's staleness re-probe, which is why the sentence names both.
        // Run together, they race on the same row and the last write wins, so the claim covers a
        // writing background task while it is in flight.
        //
        // A prompt preview is in the same collection and holds **no** claim: `run_preview` reaches
        // no write method (plan D102), so there is no row for an install to race. Before the split
        // this arm tested the whole collection, and holding `j` in the Backlog detail refused `i`
        // here — plus a login, a `ProbeBox` and a connect's registration probe — with a sentence
        // about a probe that was not running (MOD-31 D5).
        //
        // The Settings section's own `probing` flag is not enough either way: **any**
        // `StoreReply::Agents` clears it (`ui/tabs/settings/agents.rs`, module doc), and
        // `wants_requests` re-issues `Agents` on every activation, so `r` → switch tab → back → `i`
        // reaches here with `run_probe` still running.
        if self.background.iter().any(Background::writes_agent_box) {
            return Err(StoreError::Backend(
                "a probe or a re-probe is already writing this box; install once it has finished"
                    .to_owned(),
            ));
        }
```
**What changes, precisely:** one predicate call replaces one emptiness test; the string is D6's;
the comment **keeps** the Settings-section argument verbatim (D5 requires it — it is the reason
the guard exists) and **gains** the split. The three arms above (1522 `box_probe_running`, 1528
`self.auth`, 1534 `self.install`) and the `Ok(())` at 1551 are byte-identical. A plain literal,
not a `const` (D6): `BOX_PROBE_RUNNING` (93) already has its own constant for the other arm.

**No caller is edited.** The five call sites — `on_online` 523 (`if let Err(err) =
self.claim_is_free() {`), `probe_box` 1192, `install_plan` 1287, `install_confirm` 1330, `auth_start`
1402 (all four `self.claim_is_free()?;`) — inherit the fix. That is the acceptance criterion.

**After commit 2:** `cargo test -p htui --lib agent_worker -- --test-threads=1` and
`cargo clippy -p htui --all-targets --all-features -- -D warnings`.

---

### T2 — commit 1 (RED): the three new cases

All three go in the inline `pub(crate) mod tests` (3588), placed immediately **after**
`a_probe_and_an_install_never_write_the_same_row_at_once`, which ends at **5843**. They read as a
trio with the case they complete: probe writes → still refuses; preview reads → does not; chat
re-probe writes → still refuses.

**What makes them red:** case 1 and case 3 go **red on the pre-T1 guard** only if run against
`68c058f`'s `claim_is_free`. Under T1's arm they are **green immediately** — they are
characterisation tests for a fix that has already landed in this branch, which is the honest
ordering for a bug fix whose guard change cannot be split from its types. To keep TDD's value,
**run cases 1 and 3 once against the un-fixed guard** before T1 lands (`git stash` the T1 commit,
or check out `68c058f`'s `agent_worker.rs` alone) and record that they fail. See §5 for the exact
procedure and for why this is not guaranteed and must be checked, not assumed.

**Case 1 — `a_running_preview_does_not_refuse_an_install`**

Staging: `unresolvable_registry()` (5468) + `install_row` (3809) + `Fixture::start()` (3652) +
`installing_runtime` (5623). **No `fixture.route(...)`** — the plan's copy of the existing test's
30-second `/registry.json` delay is not wanted here, and §7 item 4 explains why it would be
actively expensive.

```rust
    /// MOD-31 D5: a prompt preview reaches no write method (plan D102), so it holds no claim and a
    /// live one does not refuse an install.
    ///
    /// The bug in one test: hold `j` in the Backlog detail — a task that reads a dozen tables and
    /// records nothing — and press `i` in Settings. The plan request used to come back *"a probe
    /// is already running on this box"* while the only thing alive was the preview.
    #[tokio::test]
    async fn a_running_preview_does_not_refuse_an_install() {
        let store = unresolvable_registry().await;
        let agent_id = AgentId::new();
        store
            .upsert_agent(&install_row(agent_id, "demo", true))
            .await
            .expect("the row lands");
        let backend = Backend::memory(store);
        let fixture = Fixture::start().await;
        let tmp = tempfile::tempdir().expect("a temporary install root");
        let mut runtime = installing_runtime(&fixture, &tmp.path().join("agents"));
        let (tx, _rx) = mpsc::unbounded_channel();

        // A preview first: deferred, owned by the runtime, and holding no claim.
        let previewed = runtime
            .serve(
                &backend,
                &tx,
                &envelope(
                    1,
                    StoreRequest::PromptPreview {
                        item: ids::HTUI_FEAT_1,
                        template_name: None,
                        scope: scope(),
                    },
                ),
            )
            .await;
        assert!(
            matches!(previewed, Served::Deferred),
            "the preview is deferred to the runtime's own task (`R-NF-3`): {previewed:?}"
        );
        assert_eq!(
            runtime.background_len(),
            1,
            "the runtime owns one task, which is the fact `R-NF-3` asks for"
        );
        assert_eq!(
            runtime.writing_background_len(),
            0,
            "and that task writes no `agent_box` row, so it is not in the guard's set (MOD-31 D4)"
        );

        let planned = runtime
            .serve(
                &backend,
                &tx,
                &envelope(2, StoreRequest::InstallPlan { agent_id }),
            )
            .await;
        // **Not** `assert!(matches!(planned, Served::Deferred))`. This case pins the *absence of
        // the refusal*, not the install's success: the plan is allowed to fail for the fixture's
        // own reasons (the registry is unrouted, so the task 404s), and an assertion about success
        // would be a claim this case cannot make. Do not "fix" it into a success assertion, and do
        // not weaken it to "is not the old sentence" — the old sentence is the bug's fingerprint
        // and is gone from the tree, so testing for it proves nothing.
        if let Served::Reply(StoreReply::Failed { request, message }) = &planned {
            assert_eq!(request, "install_plan");
            assert!(
                !message.contains("already writing this box"),
                "a preview writes no `agent_box` row, so it holds no claim (MOD-31 D5): {message}"
            );
        }
        runtime.shutdown(Duration::ZERO).await;
    }
```

Notes on the staging, all verified in this file:
- `ids::HTUI_FEAT_1` (`htui-core/src/fixtures.rs:228`) is a real demo item; `scope()` is this
  module's own helper at **4061**. The item need not exist for the request to defer — `preview`
  (1225) is not fallible and never touches the store — but a *real* item makes the task do real
  work, which is what keeps it unfinished at the moment of the second `serve`. Prefer it over a
  made-up id.
- `run_preview` (`crates/htui/src/preview.rs:368-392`) never panics and always answers exactly
  once on the frame channel, so `(tx, _rx)` is safe and nothing is dropped.
- `install_plan` reaches the guard at 1287 **before** `row_for` (1288) and `declares_a_source`
  (1289), so the outcome is entirely determined by the guard.
- `shutdown(Duration::ZERO)`, **not** `finish_background`: see §7 item 4.
- `Fixture::start()` spawns its accept loop; nothing needs the loop to run for this case.

**Case 2 — `a_running_chat_reprobe_still_refuses_an_install`** (OQ-3's conservative answer, pinned)

Staging is `a_chat_start_on_a_stale_acp_row_re_probes_in_the_background` (4978) **plus an
installer** — see §7 item 8 for why the two cannot simply be combined. Place it in the same trio.

```rust
    /// MOD-31 D5 and D6: a chat's staleness re-probe **does** write `agent_box`, so it holds the
    /// claim, and the refusal names both kinds of task that do.
    #[tokio::test]
    async fn a_running_chat_reprobe_still_refuses_an_install() {
        let store = MemStore::demo();
        let agent_id = AgentId::new();
        store
            .upsert_agent(&acp_fake_row(agent_id))
            .await
            .expect("the acp row lands");
        let backend = Backend::memory(store.clone());
        let fixture = Fixture::start().await;
        let tmp = tempfile::tempdir().expect("a temporary install root");
        // `acp_factory`, not `installing_runtime`'s empty `DriverFactory::new()`: a `ChatStart` on
        // an `acp` row needs the scripted transport or it is refused before the re-probe is ever
        // considered. The installer is `installing_runtime`'s, because `InstallPlan` checks it
        // first and answers "this runtime has no installer" instead of reaching the guard.
        let mut runtime = AgentRuntime::new(acp_factory(Script::one_turn(vec![
            ScriptEvent::Emit(DriverEvent::Done(DoneEvent {
                stop_reason: StopReason::EndTurn,
            })),
        ])))
        .with_grace(Duration::from_millis(0))
        .with_installer(InstallConfig::new(fixture.base(), Some(tmp.path().join("agents"))));
        let (tx, _rx) = mpsc::unbounded_channel();

        let started = runtime
            .serve(&backend, &tx, &envelope(1, start(agent_id, "hello")))
            .await;
        assert!(
            matches!(started, Served::Start { .. }),
            "the chat starts on what resolution already gave it: {started:?}"
        );
        assert_eq!(
            runtime.writing_background_len(),
            1,
            "an unprobed `acp` row is re-probed beside the chat, and the re-probe writes `agent_box`"
        );

        match runtime
            .serve(
                &backend,
                &tx,
                &envelope(2, StoreRequest::InstallPlan { agent_id }),
            )
            .await
        {
            Served::Reply(StoreReply::Failed { request, message }) => {
                assert_eq!(request, "install_plan");
                assert!(
                    message.contains("is already writing this box"),
                    "the refusal names the re-probe as well as the probe (MOD-31 D6): {message}"
                );
            }
            other => panic!("a re-probe and an install race on `agent_box`: {other:?}"),
        }
        runtime.shutdown(Duration::ZERO).await;
    }
```
The `Served::Start { task, .. }` is matched and dropped, exactly as the staleness test at 4998
does. `install_plan` refuses at 1287 before `declares_a_source`, so `acp_fake_row`'s lack of a
`discovery.install` is irrelevant — and no `/registry.json` route is needed because nothing is
spawned.

**Case 3 — `a_running_preview_does_not_refuse_a_box_probe`** (optional per the plan; **recommended**,
and its staging is *not* case 1's — see §7 item 7)

```rust
    /// MOD-31 D5, the caller the HANDOFF does not mention: `ProbeBox` is the path an `Online`
    /// swap's registration probe goes through, and `on_online` skips it **silently** — nothing was
    /// recorded, so the box stays unprobed until the next swap.
    #[tokio::test]
    async fn a_running_preview_does_not_refuse_a_box_probe() {
        let tmp = tempfile::tempdir().expect("temp box");
        let store = never_probed().await;
        let backend = Backend::memory(store.clone());
        let mut runtime = box_runtime(tmp.path());
        let (tx, _rx) = mpsc::unbounded_channel();

        let previewed = runtime
            .serve(
                &backend,
                &tx,
                &envelope(
                    1,
                    StoreRequest::PromptPreview {
                        item: ids::HTUI_FEAT_1,
                        template_name: None,
                        scope: scope(),
                    },
                ),
            )
            .await;
        assert!(matches!(previewed, Served::Deferred), "{previewed:?}");
        assert_eq!(runtime.writing_background_len(), 0, "a preview holds no claim");

        let probed = runtime
            .serve(&backend, &tx, &envelope(2, StoreRequest::ProbeBox))
            .await;
        // The same rule as the install case: the *refusal* is what is pinned. See it for why.
        if let Served::Reply(StoreReply::Failed { request, message }) = &probed {
            assert_eq!(request, "probe_box");
            assert!(
                !message.contains("already writing this box"),
                "a preview writes no `agent_box` row, so it holds no claim (MOD-31 D5): {message}"
            );
        }
        // A `ProbeBox` that got past the guard really does spawn a box probe, so this case has to
        // finish it. `never_probed` is the unresolvable registry, so it resolves in milliseconds
        // and spawns no adapter.
        runtime.finish_background(Duration::from_secs(10)).await;
    }
```
If case 3 turns out to be awkward, **drop it and say so in the close-out** — the plan explicitly
permits that. Do not leave a half-written case behind.

### T2 — commit 2 (GREEN)

Nothing else in `agent_worker.rs` changes. Cases 1-3 are green under T1's arm.

### T2 — verification read (no edit)

Read, do not edit:
- `crates/htui/tests/prompt_preview.rs:634-671`
  `the_preview_is_deferred_onto_a_task_the_runtime_owns` — `background_len() == 1` at 656-660,
  "one task, owned by the runtime". **Unchanged under D3.** This is the `R-NF-3` pin and it is why
  OQ-2's default was taken.
- `crates/htui/tests/prompt_preview.rs:567-604` `the_preview_refuses_offline_with_one_sentence` —
  `background_len() == 0` at 587. Unchanged.

**The check that matters:** run
`cargo test -p htui --test prompt_preview --all-features -- --test-threads=1` and confirm **zero**
amendments were needed. If any preview-counting assert moves, OQ-2's default is wrong — say so in
the close-out rather than editing the test.

---

## 3. Why each lifecycle reader "loses one deref and gains an accessor" — concretely

| Reader | Before | After | Deltas |
|---|---|---|---|
| `sweep_finished` (1022) | `self.background.retain(\|task\| !task.is_finished());` | `self.background.retain(\|entry\| !entry.task().is_finished());` | One deref inserted (`.task()`), one variable renamed. Same `retain`, same predicate, same order. |
| `finish_background` (645-651) | loop var is the handle | `for entry in …` + `let handle = entry.into_task();` | One **extra statement**: `timeout` needs an owned `JoinHandle`, so the move must land on its own line. `handle` keeps its name so `abort_handle()`, the `timeout` and the `warn!` are byte-identical. |
| `shutdown` (1100-1102) | `for task in … { task.abort(); }` | `for entry in … { entry.into_task().abort(); }` | One deref inserted inline, one variable renamed. |

None of the three changes *which* tasks it acts on, in what order, or with what timeout. Under T0
a `Background` holding a `JoinHandle` behaves exactly as a bare one did.

---

## 4. Per-task red/green, summarised

| Commit | State | What makes it red / green |
|---|---|---|
| T0-1 | RED | **Compile failure.** `Background` does not exist; `error[E0599]` for `Background::writing` and for `entry.task()` / `entry.into_task()`. Honest first red for a type-level change. |
| T0-2 | GREEN | Type + constructors + accessors + 4 push sites + 3 readers + `writing_background_len` all land together. `claim_is_free` still reads `!self.background.is_empty()` and still refuses, so `a_probe_and_an_install_never_write_the_same_row_at_once` keeps passing on its **old** assert. |
| T1-1 | RED | **Runtime.** `"a probe is already running on this box; …"` does not contain `"is already writing this box"`. |
| T1-2 | GREEN | Predicate + sentence + comment. |
| T2-1 | RED *(against `68c058f`'s guard only)* | Cases 1 and 3 return `Failed` with the old sentence. See §5 for the mandatory verification, and for the one way this red can fail to appear. |
| T2-2 | GREEN | Under T1's arm all three pass. |

---

## 5. The two new tests, specified (T2 case 1 and 2 in detail)

**Case 1 asserts the absence of the refusal sentence, not install success — and the wording is
load-bearing.** The fixture may fail the install for its own reasons: `install_plan` gets past the
guard, spawns `run_plan`, and `/registry.json` is unrouted so the responder writes
`404 Not Found` (`agent_worker.rs:3730-3735`). The install therefore *fails*, just not at the
guard. Asserting success would be a claim the fixture cannot support; asserting
`!message.contains("already writing this box")` is exactly the property under test.

The assert message and the comment above it are written to stop two specific future "fixes":

1. Rewriting it to `assert!(matches!(planned, Served::Deferred))` — that would pass on a runtime
   that refused the plan for a *different* reason, which is precisely the bug's failure mode
   (wrong reason vs. no reason). The comment says so in those words.
2. Rewriting it to `assert!(!message.contains("probe is already running"))` — the old sentence is
   gone from the tree entirely after this change, so that assertion is vacuously true and would
   pin nothing. The comment says so too.

Both are why the comment is three lines long. Do not shorten it.

**The determinism check you must run before committing T2-1.** `serve` calls `sweep_finished()` as
its **first statement** (`agent_worker.rs:889`), so the `InstallPlan` request sweeps the collection
*before* it asks the guard. The bug reproduces only if the preview entry is still in `background`
at that moment. On `#[tokio::test]`'s current-thread runtime, `tokio::spawn` does not run the
task, and nothing between the spawn at 1257 and `claim_is_free` at 1287 yields to the executor
(`recording_writer`, `registered_box` and the `Backend::memory` reads all resolve on ready
futures; the registry read is in the spawned task, not on this path). A *real* item
(`ids::HTUI_FEAT_1`) makes the preview's own work long enough that it cannot plausibly have
finished. So the entry should be present — but this is an argument about a scheduler, not a
guarantee, and the one failure direction is a **false pass**, not a flake: if the preview had been
swept, the old guard would also not have refused and the test would be vacuous.

Procedure, before committing T2-1:

```bash
# From the T0-2 commit (types landed, guard still old), with the three cases written:
cargo test -p htui --lib agent_worker a_running_preview -- --test-threads=1 --nocapture
```
It must **fail**, printing the old refusal sentence. If it passes, the preview was swept and the
case is vacuous — the fix is to re-check the staging (use a real item, keep the two `serve` calls
adjacent with no `.await` of your own between them), and to record in the commit message what went
wrong. Do not proceed to T2-2 on a case that never went red.

The same run is the red evidence for case 3 (`a_running_preview_does_not_refuse_a_box_probe`),
which has the identical shape.

---

## 6. Landmines

**L1 — The `JoinHandle` import. The plan's landmine here is false; do not touch the import.**
The plan says "the import of `JoinHandle` at the top of the test module may become unused — check
before removing it, because `LiveInstall`/`LiveAuth` still name it." Two things are wrong with it.
(a) **There is no `JoinHandle` import in the test module.** The module opens with `use super::*;`
at `agent_worker.rs:3589`; the import is **file-scope**, `use tokio::task::{AbortHandle,
JoinHandle};` at **line 60**. (b) It cannot become unused: production code names `JoinHandle` at
212 (`LiveChat::task`), 256 (`LiveInstall::task`), 321 (`LiveAuth::task`), **384 → the new
`Background::task`**, 435 (`box_probe`) and 684 (`attach`). Removing the `JoinHandle::is_finished`
use at 8213 changes nothing about the import. Leave line 60 alone.

**L2 — `finish_background`'s arm order is load-bearing, and a mistake there surfaces as a test
*timeout*, not a compile error.** The method clears `self.previews` (644) **first** — "every
preview named here is one of the tasks about to be awaited, so the right to cancel it dies with the
handle" (642-643) — then the `background` loop (645), then `box_probe` (654), then `install` (661),
then `auth` (673). T0 touches only the loop at 645. If a reordering or an early-return were
introduced, a preview would keep an `AbortHandle` the awaited handle no longer backs, and the
symptom in `crates/htui/tests/*` (which call the **public** `finish_background` directly) would be
a case that hangs until its harness deadline. This is L4 in the "load-bearing ordering" family and
it cannot be caught by `cargo check` — run the suite.

**L3 — `sweep_finished` is per-request, not per-task.** Its only two callers are `serve` (889) and
`on_online` (522) (its own doc at 1014-1015 names exactly those). So a *finished* background task
keeps holding the claim — for a **writing** task — until the next request reaches the runtime. That
behaviour is unchanged by this work. What *does* change: a finished **preview** used to keep
refusing until the next request, and now never refuses. Cases 1 and 3 assert `writing_background_len()`
*before* the guarded request; do not add a post-sweep assertion, or you will be asserting the sweep,
not the split.

**L4 — `install_plan`'s task lands in `self.install`, not `background`.** `install_plan` pushes
into `LiveInstall { phase: Planning, .. }` at **1304-1309** — it is not one of the four background
push sites. Consequences: (i) a new case that lets an `InstallPlan` through past the guard and
then calls `runtime.finish_background(Duration::from_secs(N))` will **await the plan task** at 661
and, with the existing fixture's 30-second `/registry.json` delay, cost 30 seconds. That is why
case 1 does not route `/registry.json` and tears down with `shutdown(Duration::ZERO)` — `shutdown`
trips the cancel and takes a `grace * 2 == 0` window, so it returns immediately.
(ii) `finish_background`'s `background` loop and its `install` arm are two different things; do not
"helpfully" route one into the other.

**L5 — Do not add an intra-doc link from `writing_background_len` to `Background` or `Writes`.**
Both are private and the new method is `pub`, so the link is `rustdoc::private_intra_doc_links` —
a **warning**, and the gate runs clippy with `-D warnings`. The doc above uses backticks only.

**L6 — The `run_probe` description in comments must stay accurate about *which* rows.** `run_probe`
(1979-1995) → `probe_agents_on` (2003-2041) skips `!enabled` and upserts only on
`ProbeOutcome::Row`. "Writes `agent_box` for every enabled row that yields `Row`" is the true
statement; "every row" is loose (claim 11) and the constructor doc above already says "for at least
one row", which is the weaker form the guard actually needs.

**L7 — `#[must_use]` on `writing_background_len` is not optional** (D4), and the `--test-threads=1`
flag is not optional on any of the three test commands: the keyring fake is process-wide and this
suite is scheduling-dependent. Before believing a Postgres failure, run `df -h /` and re-run the
case alone.

**L8 — `cargo fmt` last, not first.** Sites T0-4, T0-7, T0-8 and T0-c are hand-shaped multi-line
edits; rustfmt decides the final form. Run `cargo fmt --all` after the last edit of each commit and
re-read the four lines before committing.

---

## 7. What the plan got wrong or missed

**No fourth design problem.** I looked for one and did not find it. D1–D7 are internally
consistent, and every one of the five `claim_is_free` callers is repaired by T1's single line
without an edit of its own. I am not proposing a change to any of D1–D7. The items below are
corrections and gaps, none of which invalidates a decision.

1. **The `JoinHandle` import landmine is false, in both of its parts.** See L1. The import is
   file-scope at line 60, not in the test module, and it has six production uses. Plan's Test plan
   item 3, last sentence.

2. **HEAD is `6189661`, not `b1f9b3e`.** The plan's header and its verified-claim 1 both say
   `b1f9b3e`. The worktree has moved on by one commit (the OQ answers). The load-bearing part still
   holds — `git diff --name-only 68c058f HEAD` returns only the plan file — so every line number in
   the plan is still correct. The close-out must record `6189661` as the base, not `b1f9b3e`.

3. **The plan does not say that `install_plan` spawns into `self.install`, not `background`.** See
   L4. Following the plan's T2 staging literally — "Same staging as
   `a_probe_and_an_install_never_write_the_same_row_at_once`" — would route `/registry.json` with a
   30-second delay and then await it, turning a unit test into a 30-second one.

4. **The plan's third test says "Same staging as (1)", and it is not the same staging.**
   `probe_box` (1181-1204) never calls `self.install_config()` — its arms are
   `backend.writer()`, `registered_box`, `claim_is_free`, `probe_env` — so it needs **no
   `Fixture` and no `installing_runtime`**. The right staging is `box_runtime(tmp.path())` over
   `never_probed()`, which is what the existing `ProbeBox`/`on_online` cases use, plus
   `finish_background` at the end because a `ProbeBox` past the guard really does spawn a box
   probe. Following the plan here would add a 30-second route and an unnecessary `LiveInstall`
   await.

5. **The plan does not name the factory conflict in T2 case 2.** `installing_runtime` (5623) is
   `AgentRuntime::new(DriverFactory::new()).with_installer(...)` — it registers **no** `acp`
   transport, so a `ChatStart` on an `acp` row is refused by `driver_for` before the staleness
   re-probe is ever considered, and `writing_background_len()` would be `0`. The case must combine
   the staleness test's `acp_factory` with the install test's `with_installer`, which is what §2's
   case 2 does. Without this the case does not reach its assertion.

6. **The plan never says the item need not exist in the store for a preview to defer.**
   `preview` (1225-1259) is not fallible and never reads; `run_preview` (`preview.rs:368-392`)
   always answers exactly once. So the staging has no hidden precondition. Using
   `ids::HTUI_FEAT_1` (a real demo item, `htui-core/src/fixtures.rs:228`) with this module's own
   `scope()` helper (`agent_worker.rs:4061`) is the least surprising form, and it also keeps the
   task busy long enough to survive the sweep at the top of the second `serve`.

7. **`docs/decisions/mod/` holds 14 files, not 8.** The plan's verified-claim 30 says 8. Cosmetic —
   it matters only so the implementer does not go looking for a "mod-31.md" among eight and
   conclude the directory is wrong.

8. **A doc the plan does not name must change: `agent_worker.rs:8202-8203`.** The doc of
   `a_finished_background_task_does_not_stop_the_registration_probe` reads "a finished background
   task (a preview, a chat re-probe) does not hold the claim". D2 tags that test's entry
   `writing`, which is right (the sibling at 8226 is about a *held* claim) — but then the doc
   overstates: a `reading` entry would not hold the claim even unswept, and what this case actually
   demonstrates is that **`sweep_finished` frees the claim**. Site T0-e amends the doc to say so and
   to record that the `writing` tag is deliberate, so a future reader does not "correct" the tag to
   `reading` and quietly stop testing the sweep.

9. **A vacuity check the plan asks for but does not enable.** T2 cases 1 and 3 are green the moment
   T1 lands, so a naive reading of "T2 is the red commit" is not achievable for them. §5 gives the
   procedure (run them once against the un-fixed guard before T1, and record the red). Doing that
   is what makes them tests rather than assertions.

10. **`USERNAME=htui-ci` is house spelling, not README's.** README's own form
    (`README.md:475-485`) is `HTUI_TEST_DATABASE_URL=postgres://postgres:htui@localhost:5439/postgres
    cargo test --workspace --all-features` with no `USERNAME`. The plan's `USERNAME=htui-ci` prefix
    comes from the house plans and is harmless; keep it if you like, but do not cite README for it.

---

## 8. Validation

Per-task, fast loop:

```bash
cargo test -p htui --lib agent_worker -- --test-threads=1
cargo clippy -p htui --all-targets --all-features -- -D warnings
```

Full gate, from the plan's `## Validation` and `README.md:467-473` / `:475-485` (all seven
verified in this worktree):

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

**What each must print when green**

| Command | Green |
|---|---|
| `cargo fmt --all -- --check` | No output, exit 0. |
| `cargo clippy … -D warnings` | No `warning:` and no `error:` lines, exit 0. A private-item rustdoc link (L5) shows up here. |
| `cargo test --workspace --all-features -- --test-threads=1` | Every suite prints `test result: ok. N passed; 0 failed; …`. The `htui` lib suite must list `a_running_preview_does_not_refuse_an_install`, `a_running_chat_reprobe_still_refuses_an_install` and `a_running_preview_does_not_refuse_a_box_probe`. The `prompt_preview` suite must list `the_preview_is_deferred_onto_a_task_the_runtime_owns` passing **unamended**. The store/pg suites must **not** print `skipped: HTUI_TEST_DATABASE_URL not set` — `crates/htui/tests/box_probe_pg.rs` exists and runs, so the env var must be set or the gate is silently weaker. |
| `ls crates/htui-store/.sqlx \| wc -l` | `268` (verified in this worktree). |
| `ls crates/htui/tests/snapshots \| wc -l` | `88` (verified). |
| `cargo doc --workspace --no-deps --keep-going` | Exactly the **six** baseline errors listed at `HANDOFF.md:44` — `htui-core` `MIRRORED_TABLES`; `htui-store` `step_exists`, `HashEmbedder` in `embed.rs`, and three private links in `pg/write.rs`. A seventh means a new intra-doc link leaked. |
| `git diff --stat 68c058f -- <five paths>` | Empty. Five paths, not four: `crates/htui/src/snapshots` (1 entry) is the easy one to miss. |

**Not run, by design:** `cargo sqlx prepare`. This change touches no SQL of any kind — no
migration, no `cache_migrations/`, no `query!`, no store method, no `.sqlx` entry — and
`README.md:492-511` makes regeneration conditional on a changed query.

**Operational.** The worktree has its own `target/` (~5.3 GB after fmt + clippy + lib-test, more
for `--all-targets`) and sits on `/media` (`/dev/sda`), not on `/` — so it is not the dev-Postgres
disk-pressure failure mode, but `df -h` before a full `--all-targets` run anyway. Postgres is
reachable at `localhost:5439` via the compose `htui-postgres` container. `--test-threads=1` on
every test command (L7).

**Live check, optional, after the merge.** Hold `j` on a Backlog row so a preview starts, switch
to Settings > Agents, press `i`. Today: *"a probe is already running on this box"*. After:
answered. Then repeat with a genuinely running install (a slow registry `HEAD`) and confirm the
refusal now reads *"a probe or a re-probe is already writing this box"*.

**Close-out is main-thread only** and is **four** edits, per
`.claude/rules/workflow-docs.md` lifecycle step 3 (verified verbatim at `:109-113`), whose
`paths:` frontmatter governs `HANDOFF.md`, `DECISIONS.md`, `docs/decisions/**` and `docs/ANA-*.md`:
1. delete the MOD-31 checklist line at `HANDOFF.md:225-235` (it quotes the old sentence at
   `HANDOFF.md:228` — do not "fix" that string, the line is going away);
2. write the write-up with the commit hash to `docs/decisions/mod/mod-31.md`;
3. prepend one index line to `DECISIONS.md` in the house form
   `- **[MOD-31](docs/decisions/mod/mod-31.md)** - <title> (done, 2026-MM-DD)`, directly under the
   `> Format per …` line at `DECISIONS.md:4` (reverse-chronological, newest first);
4. update the summary table — drop `MOD-31 preview blocks install` from the `MOD-N | 41 (…)` row at
   `HANDOFF.md:748` and decrement the count to 40 — **and** the top status line, which the rule
   says to **cap at roughly 2-3 recent completions: drop the oldest mention rather than append**.

No implementer touches those four files.

---

## 9. Data flow, after the change

```
prompt (Backlog detail, key j)
  └─ serve()  → sweep_finished()  →  preview()  →  tokio::spawn(run_preview)
                                                   └─ Background::reading  → self.background
ChatStart on a stale acp row
  └─ serve()  → sweep_finished()  →  start()     →  tokio::spawn(run_reprobe)
                                                   └─ Background::writing  → self.background
ProbeAgents
  └─ serve()  → sweep_finished()  →  probe()     →  tokio::spawn(run_probe)
                                                   └─ Background::writing  → self.background

install_plan / install_confirm / auth_start / probe_box / on_online
  └─ claim_is_free()
       ├─ box_probe_running()  → BOX_PROBE_RUNNING
       ├─ self.auth            → "a login is already running for agent …"
       ├─ self.install         → "an install is already running for agent …"
       └─ background.iter().any(Background::writes_agent_box)
            → "a probe or a re-probe is already writing this box; install once it has finished"
         (a preview is in the collection and is invisible to this predicate)

background_len()            → self.background.len()                       (unchanged, 23 read sites)
writing_background_len()    → …filter(Background::writes_agent_box).count()   (new, D4)
```

Readers of the collection, exhaustively (literal text search over `self.background` /
`runtime.background`): `background_len` 628, `finish_background` 645, `sweep_finished` 1022,
`shutdown` 1100, `probe` 1165, `preview` 1257, `claim_is_free` 1546, `start` 1758, and the two
inline tests at 8212-8213 and 8233/8241. There is no eighth reader. `previews` (405) is untouched:
it holds an `AbortHandle` keyed on `Origin`, taken at 1254 *before* the handle moves into its
`Background` at 1257, and its own doc already says the map owns "the *right to cancel*, not the
task".

---

## 10. Acceptance, mapped to what to check

- [ ] `claim_is_free` refuses only when a background task **writes** `agent_box`; a live preview
      does not refuse an install, a login, a `ProbeBox` or a swap's registration probe.
      → T1-b; cases 1, 2, 3.
- [ ] All five callers inherit the fix with no edit of their own.
      → `git diff 68c058f -- crates/htui/src/agent_worker.rs` shows **no** hunk touching 523,
      1192, 1287, 1330 or 1402. `grep -n "claim_is_free"` returns 10 hits, unchanged.
- [ ] A live staleness re-probe **does** still refuse, with a sentence naming a re-probe.
      → case 2.
- [ ] One collection; four push sites tagged through named constructors; no other module can
      construct or read a `Background` (requires `Background` and `Writes` to be genuinely private,
      **not** `pub` — cf. `LiveInstall`/`LiveAuth` at 248/312, which are `pub` and buy nothing).
      → T0-1/T0-4/T0-5/T0-6; `background` is a private field, and `Background` is module-private.
- [ ] `background_len()` unchanged and all **23** of its read sites pass without amendment; the
      `R-NF-3` pin at `crates/htui/tests/prompt_preview.rs:656-660` still asserts `1`.
      → D3; the `prompt_preview` run in T2.
- [ ] No snapshot, no migration, no `.sqlx` entry, no store `CASES` pin and no `StoreRequest` /
      `StoreReply` variant moves; `HANDOFF.md:36-42`'s pins unchanged (store `CASES` 77,
      `READ_CASES` 14, `htui-orch` `CASES` 72, `GraphSource` 7, `StoreRequest` 69, `StoreReply` 40,
      `hierarchy::REQUEST_NAMES` 13, 268 `.sqlx`, 88 snapshots, `MIRRORED_TABLES` 21, migrations
      `0001`..`0007`).
      → the two `ls … | wc -l` lines, the `cargo doc` count and the empty `git diff --stat`.
- [ ] The gate above is green and the five-path diff is empty.
- [ ] Close-out complete, all four parts (§8). Main thread only.
