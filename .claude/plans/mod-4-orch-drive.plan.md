# Plan: MOD-4 milestone 6 — the maintainer drives it

**Source**: `.claude/prds/mod-4-orchestrator-manual-mode.prd.md`, milestone 6 (`:312`): "`run_worker.rs`,
the seven `R-TUI-4` actions plus `Unblock`/`AcceptArtifact`/`CloseOut`, the step list with agent,
model, gate, usage and duration, promotion to chat, and close-out." Scope bullets `:245-251`
(`run_worker.rs` + `StoreRequest::Orch`/`RunStream`, the Runs tab) and `:252-255` (the three MOD-2
hand-overs). Success-metric rows `:194` (promotion keeps one identity), `:195` (close-out is a
transaction), `:196` (the Runs tab is complete) and `:197` (nothing regresses offline). Design
authority: the PRD's D1–D8 (cited **PRD Dn**); this plan's own decisions are plain **Dn**; earlier
milestones are **M1 Dn** … **M5 Dn** (M5 includes the follow-up round, D146–D152). `docs/ANA-2.md`
§4.3 (`:520-687`, item table `:563-589`, run table `:598-616`, step table `:628-654`), §4.8
(`:1160-1240`: promotion `:1182-1197`, context paths `:1199-1206`, accept artifact `:1223-1233`),
§4.10 close-out (`:1380-1392`), §6.2 (`:1551-1580`), §8 (`:1681-1697`), §12 criteria 3, 14, 17,
19, 20, 21 (`:2090-2091`, `:2122-2124`, `:2131-2134`, `:2138-2140`, `:2141-2143`, `:2144-2146`);
`docs/ANA-5.md` §12 criteria 3, 16, 18 (`:2271-2273`, `:2309-2311`, `:2316-2318` — amended at
fact-check: `:2313-2315` and `:2319-2321` are criteria 17 and 19);
`docs/REQUIREMENTS.md` `R-ORCH-5` (`:196-198`), `R-TUI-2` (`:283-284`), `R-TUI-4` (`:287-289`),
`R-TUI-9` (`:302-303`), `R-NF-3` (`:326-327`).

**Requirements**: `R-TUI-4` (the Runs tab and its seven actions), `R-TUI-9` (close-out),
`R-TUI-2`'s `run` and `close`, `R-ORCH-5` (promotion to chat and resume from the next step),
`R-ORCH-4` (manual mode runs one item end to end, now from the TUI), `R-NF-3` (every orchestrator
command off the UI task), `R-HIS-1` (the sweep and resume are finally called by a process).

**Complexity**: Large. A new `crates/htui/src/run_worker.rs`; `htui` gains its first dependency on
`htui-orch`; four new `Command` variants and their engine bodies; two pure `htui-orch` modules
(`closeout.rs`, `promote.rs`); one `htui-agent` recorder constructor; one item-law edge in
`htui-core`; the Runs pane rewritten around ten actions plus `run` and `close`; the Chat tab bound
to a promoted graph step; Backlog snapshot re-recording. **No migration, no new `WriteStore` or
`ReadStore` method, no `.sqlx` change expected** (every write this milestone needs already exists:
`promote_step`, `close_out`, `add_note`, `transition`, `write_document`; `traits.rs:972-1057`).

**Routing**: routed as **plan** by `/handoff-run MOD-4` (the PRD and its milestone table exist).
**Staffing: Opus 5.5 for every step — plan, fact-check, architect, implementers, verifiers and
reviewer (`rust-reviewer`, `.claude/workflow-config.json:2`); Fable is not used (maintainer
standing instruction).** Ultracode for the implementers only, one workflow per task, verify fan-out
per round; the architect and the reviewer stay plain agents.

**Numbering**: the highest decision number across every `mod-4-*` plan and blueprint is **D152**
(`.claude/plans/mod-4-orch-lease.blueprint.md:1170`, §23.1), so this plan starts at **D153**. The
highest risk is **R-37** (`mod-4-orch-lease.blueprint.md:1207`); new risks start at **R-38**.

**Status**: **confirmed** (2026-09-24) — fact-checked at `ba68682`, then confirmed by the maintainer
with every open question's adopted default (OQ-1..OQ-13 answered as written).
Branch `mod-4-m6` (cut after MOD-30 landed, `ba68682`). Amendments made by the fact-check are
marked "(amended at fact-check)"; the ledger is the "Verified claims" table at the end.

**Graphify note**: `graphify-out/` is deleted in the working tree (git status at branch start), so
nothing here was read from it. Every tree fact below was read through the Gortex index or the
file itself at `ba68682`; each carries a `file:line`, and anything not re-opened at its line is
marked **UNVERIFIED**.

---

## Open questions for the maintainer (read these first)

Each has a default this plan adopts so implementation is not blocked.

- [x] **OQ-1 — Where does `run_worker` live, and what owns a walk?** ANA-2 §8 describes "the
      supervising task: one per box, owns the lease refresh, spawns one engine task per active
      run" (`docs/ANA-2.md:1681-1682`) and forbids a fourth `select!` arm in `event_loop.rs`
      (`:1687-1692`). The store worker is the only owner of a `Backend`
      (`crates/htui/src/store_worker.rs:1-6`) and already hosts one runtime, `AgentRuntime`, served
      ahead of `try_serve` through `Served { Reply, Deferred, Start }`
      (`crates/htui/src/agent_worker.rs:258-271`, consumed at `store_worker.rs:1335-1359`).
      **Default adopted (D153):** `run_worker.rs` defines a `RunRuntime` that lives **inside the
      store worker loop beside `AgentRuntime`**, is handed `&Backend` and the reply sender on each
      request, and spawns one Tokio task per command (and per adopted run) that owns cloned
      handles. Its periodic sweep is a second ticker arm **in the store worker's `select!`**
      (`store_worker.rs:1110-1115`), not in `event_loop.rs`. **Alternative:** a free-standing task
      fed a `watch::Receiver<Option<Backend>>` that the loop publishes on every swap — cleaner
      ownership, but every swap site (`go_online` `:1511`, `go_offline` `:1547`, the `SetDsn`
      block `:1166-1236`) gains a publish.
- [x] **OQ-2 — The per-run mutex (R-27).** `take_lease` always succeeds for its own owner, so two
      commands of one process can walk one run at once (`mod-4-orch-lease.blueprint.md:1075`);
      `DeadWalks`'s doc names the same race (`crates/htui-orch/src/engine.rs:262-275`).
      **Default adopted (D157):** the mutex lives in `RunRuntime`, **not** in the engine: a
      `RunLocks` map `RunId -> Arc<tokio::sync::Mutex<()>>` taken for the whole of every command,
      every resume and every sweep-driven walk of that run. Every orchestrator call in the process
      goes through `RunRuntime`, so this fences all of them, and `EngineParts` (built at six sites
      today — `engine.rs:5031` (`fake_parts`), `:5360`, `:5447`, `:7858`, `conformance.rs:5032`,
      `tests/gix_isolator.rs:88` (a macro) — plus the new worker; amended at fact-check, the draft
      cited stale lines and missed the `gix_isolator` macro) does not grow a field.
      **Alternative:** a field on `EngineParts` holding the map — the engine then fences a direct
      `dispatch` too, at the cost of editing all seven `EngineParts` literals.
- [x] **OQ-3 — Does `Unblock` double as R-7's resume verb?** R-7: a run parked by a refused
      reconcile has no resume verb (`HANDOFF.md:281-285`; `command.rs:505-521` says "milestone 6's
      `Unblock`-shaped verb owns that retry"). The tree already has the walk that answers it:
      `Engine::resume` unparks an `awaiting_approval` run whose cursor is `Create`, `Run` or
      `Finished` and re-reconciles the frontier (`walk_resumed`, `engine.rs:2036-2066`, M5 D132).
      **Default adopted (D161):** `Command::Unblock { item }` clears whatever holds the item, in
      this order: (1) item `blocked` and no non-terminal run → `blocked → open`; (2) item
      `blocked` with a live `awaiting_approval` run (an escalation, `gate.rs:997`) →
      `blocked → awaiting_approval` (a **new item-law edge**, OQ-4) so the Runs tab's promote,
      approve and cancel reach the run; (3) item `awaiting_approval` with a run whose cursor is not
      parked at a gate step (R-7's reconcile refusal, D132's crash leftovers, R-31) →
      `Engine::resume(run)` under the lease. Anything else is refused with a sentence naming the
      item's status. **Alternative:** two verbs, `Unblock { item }` (ANA-2's, item only) and a
      separate `ResumeRun { run }` for R-7.
- [x] **OQ-4 — A new item-law edge `blocked → awaiting_approval`.** `Status::can_move_to` gives
      `Blocked → Open | Closed` only (`crates/htui-core/src/model/item.rs:55`), and ANA-2 says an
      escalation is cleared by "`retry` or `approve` on the Runs tab, or `unblock`"
      (`docs/ANA-2.md:551`) — no row lets the item follow its parked run back, which is M2's
      carried R-4 (`mod-4-orch-engine.blueprint.md`, F-J). **Default adopted (D161):** add the one
      edge, used only by `Unblock` case (2); the table test `SANCTIONED`
      (`item.rs:262-285`) gains the row; the store conformance transition case iterates
      `Status::ALL` against `can_move_to` (`crates/htui-core/src/store/conformance.rs:6392`) and
      needs no edit; no SQL changes, because both stores enforce the law in Rust through
      `legal_move` (`traits.rs:1124`). ANA-2 §4.3 is amended by the main thread. **Alternative:**
      the literal table — `Unblock` on an escalated item moves it `blocked → open` and cancels the
      parked run (its trees are cleaned, its work is lost).
- [x] **OQ-5 — Promoting a `running` step.** ANA-2 admits `running` and asks for "the driver's
      grace window first" (`docs/ANA-2.md:1182-1197`, `:641`); the shipped writer accepts
      `failed | awaiting_approval` only (`traits.rs:974-983`), and in this milestone a running
      step's session lives inside a walk task. **Default adopted (D163):** a `running` step is
      promoted by **preempting its walk** — the run's cancel token drops the walk (M5 D86's
      abandon path, which kills the agent through `ChildGuard::drop`), then the engine moves the
      step `running → awaiting_approval` and calls `promote_step`. The grace window and the ACP
      "answer every parked permission request" step are **not** honoured (R-38). **Alternative:**
      refuse `running` with "cancel or wait for the gate", which keeps ANA-4 §4.3 intact and makes
      `R-ORCH-5`'s "by user request" mid-session impossible.
- [x] **OQ-6 — Promotion reuses the existing Chat tab.** **Default adopted (D165):** yes. The Runs
      pane emits `Action::Promote { run, step }`; the shell focuses the Chat tab and dispatches the
      promote request **from the Chat tab's origin**, exactly as `Action::Replay` does
      (`crates/htui/src/app/update.rs:129-136`, `app/mod.rs:76`), so every chat frame lands in
      `ChatTab::on_reply` and `ChatSend`/`ChatAnswer`/`ChatCancel` keyed by `step_id`
      (`store_worker.rs:151-171`) drive the promoted step unchanged. **Alternative:** a chat pane
      inside the Runs sub-tab (43 columns wide at 100×30).
- [x] **OQ-7 — Key bindings for the Runs pane.** Taken today: `j k g G l ] h [ Enter` by the Backlog
      tab (`crates/htui/src/ui/tabs/backlog/mod.rs:203-229`), `J K` by the detail pane and the Runs
      cursor (`detail/mod.rs:293-294`, `detail/runs.rs:202-203`), and `q ? Tab BackTab 1-9 Esc`
      globally (`crates/htui/src/keymap.rs:200-236`). **Default adopted (D168)**, active only while
      the Runs sub-tab is shown: `a` approve, `x` reject with note, `r` retry, `p` promote, `c`
      cancel run, `o` open artifact, `s` select fan-out winner, `u` unblock, `A` accept artifact,
      `R` run (`R-TUI-2`), `C` close out (`R-TUI-2`/`R-TUI-9`), `T` retry the cleanup of a terminal
      run (R-25). Destructive `c` asks `y`/`n`; `C` is the two-stage confirmation (D167).
      **Alternative:** put `R` and `C` in the Backlog list pane, which needs a `match` arm in the
      Backlog tab that ANA-2 `:1694-1697` forbids.
      **Key capture (amended at fact-check).** `BacklogTab::on_key` consumes `j k g G l h [ ]`,
      the arrows, `Home`/`End` and `Enter` **before** it offers a key to the detail pane
      (`crates/htui/src/ui/tabs/backlog/mod.rs:210-226`), so the Runs pane can neither fill the
      `x` note field (a note containing `g`, `h`, `j`, `k` or `l` loses those letters and jumps the
      list) nor take a typed-back item key containing `G`, nor "swallow every unlisted key"
      (D167). The default therefore adds one generic routing seam, not an action arm:
      `DetailTab::captures_input(&self) -> bool` (default `false`,
      `ui/tabs/backlog/detail/mod.rs:59-75`) and `DetailRegistry::captures_input`, and one guard at
      the top of `BacklogTab::on_key` that hands every key to the detail pane while the active
      sub-tab captures. ANA-2 `:1694-1697` forbids a per-action `match` arm in the Backlog tab;
      this guard names no action. Both files join T8.
- [x] **OQ-8 — How the step list fits 43 columns.** The pane is 43 columns at 100×30
      (`detail/runs.rs:21-25`, `PANE_WIDTH` test const) and already folds each run into two lines;
      steps are one line (two with a trim figure) of `cursor, position, status, phase, started`.
      **Default adopted (D169):** two lines per step. Line 1: cursor (2), slot `p.a` or `p.a/i`
      (5), status (9), phase (12), usage (7), duration (5), single spaces between. Line 2 (indented
      under the status column): gate (9, `approved`/`rejected`/`retried`/`skipped`/`—`, with `*`
      for `promoted_at` and `✓` for `selected = true`), the existing trim figure (12, under the
      phase, `detail/runs.rs:34-42`), and `agent/model` truncated with `…` (13). A width test pins
      every row to 43 columns, as MOD-30 pinned the strip (`backlog/mod.rs:281`). **Alternative:**
      one line per step plus a three-line footer for the selected step carrying agent, model,
      gate, usage and duration in full — fewer truncations, but criterion 21 asks for them "in the
      step list" (`docs/ANA-2.md:2145-2146`).
- [x] **OQ-9 — Where "open artifact" opens.** ANA-2 says "in the Documents sub-tab, read-only"
      (`:1565`); the Documents sub-tab renders heads only (`detail/documents.rs:1`) and a sub-tab
      cannot switch its siblings (`DetailRegistry`, `detail/mod.rs:79-195`). **Default adopted
      (D173):** the Runs pane opens a read-only body view over itself (`Esc` closes), fed by a new
      `StoreRequest::Document(DocumentId)` read over the existing `ReadStore::document`
      (`crates/htui-core/src/store/traits.rs:88`, `crates/htui-store/src/backend.rs:671`).
      **Alternative:** a `DetailRegistry::focus(DetailId)` plus a body view in the Documents
      sub-tab.
- [x] **OQ-10 — ANA-5 criterion 3 (`blocked` on a prompt refusal).** Today an `AssembleError` at
      stage 3 becomes `EngineError::Prompt` by `?` inside `assemble_prompt` (`engine.rs:4171`),
      escapes `walk_live_step` by `.await?` (`engine.rs:2387-2389`) and `walk_step`'s error arm
      fails the step and the run (`engine.rs:2339-2346`, `fail_hard` → `finish_run(Failed)`), so
      the item ends `failed`, not `blocked`. **The fan-out path has the same escape (amended at
      fact-check):** `drive_group` assembles once for the group and `?`s the same error with every
      candidate still `pending` and the run `running` (`engine.rs:2704-2706`); D162 covers both
      call sites.
      **Default adopted (D162):** every `EngineError::Prompt` raised at stage 3 before a token is
      spent is a configuration refusal a human must clear: step `failed`, item
      `in_progress → blocked` **before** `finish_run(Failed)` (the order `refuse_no_candidate`
      already uses, `engine.rs:2227`, doc `:2061-2069`), and an `item_note` with the assembler's
      sentence. `Unblock` then returns the item to `open`. **Alternative:** only
      `TemplateError::UnknownPlaceholder` (criterion 3's literal case) blocks; budget refusals
      (criterion 10) keep failing the run.
- [x] **OQ-11 — Criterion 19 (the offline window) after MOD-25.** M5 D103 handed criterion 19 to
      this milestone (`mod-4-orch-lease.plan.md:263`); its text needs events in
      `pending/<project>.<run>.jsonl` and `upload_pending` (`docs/ANA-2.md:2138-2140`), but MOD-25
      made `htui` online-only and `Backend::writer()` answers `None` offline
      (`crates/htui-store/src/backend.rs:152-158`). **Default adopted (D175):** criterion 19 is
      re-scoped to what online-only allows and proved in `run_worker`'s tests: a store outage mid-
      step fences the walk before its lease lapses (M5 D122), and after the backend comes back
      this process's sweep adopts and adjudicates the run (M5 D139/D140). Nothing is buffered.
      **Alternative:** strike criterion 19 outright in ANA-2.
- [x] **OQ-12 — Which carried risks this milestone closes.** **Default adopted** (the table under
      "What this milestone touches"): closes R-4 (D161), R-7 (D161), R-12 (D158), R-25 (D177),
      R-26 (D160), R-27 (D157), R-28 (D179), R-36 (D159) and the offline half of criterion 19
      (D175); partially closes R-31 (D180); leaves R-3, R-5, R-6, R-9, R-10, R-29, R-30, R-32 and
      R-37 carried, each with a named owner. **Alternative:** also fold R-37 (open the repository
      once in `git::reconcile_parent`, speed only) and R-32 (diagnostic text) into T5.
- [x] **OQ-13 — Secrets for a graph session.** `drive_once` builds every `SessionSpec` with an
      empty `env` and says "ANA-7 resolves secrets and milestone 6 wires them"
      (`engine.rs:4358`); no secret provider exists in the tree (the only hits for one are the
      `project.secret_provider`/`secret_scope` columns, `crates/htui-core/src/model/hierarchy.rs:66-68`).
      **Default adopted (D176):** no secrets are wired; the comment is rewritten to name the item
      that owns the provider, **MOD-10 — Secret provider (from ANA-7)** (`HANDOFF.md:398`; amended
      at fact-check, was UNVERIFIED). **Alternative:**
      wire the OS keyring for `project.secret_scope` keys now.

---

## Summary

Milestones 1–5 built an engine nothing calls. `htui` does not depend on `htui-orch`
(`crates/htui/Cargo.toml` `[dependencies]` names `htui-agent`, `htui-core`, `htui-store` and no
orchestrator), the Runs pane answers `J`, `K` and `Enter` only (`detail/runs.rs:200-213`), and
`Engine::sweep`/`resume`/`claim` document milestone 6 as their caller (`engine.rs:436-448`,
`:570-587`, `:1940-1950`).

**The worker.** `run_worker.rs` holds a `RunRuntime` inside the store worker loop (D153). It serves
`StoreRequest::Orch(OrchRequest)` through the `Served` pattern — nothing on the UI task awaits a
walk (`R-NF-3`) — and builds an `Engine` per command from owned handles: a `Writer`
(`Backend::writer`, `backend.rs:152`), a `BackendGraphs` newtype that implements `GraphSource`
(D155; the orphan rule rules out implementing it on `Backend` from `htui`), one `GixIsolator` and
one `ShellVerifier` per process (D156), `SystemClock`, `FirstCandidate`, a progress sink, and a
driver closure over `DriverFactory::production` (`crates/htui-agent/src/registry.rs:76-110`). A
per-run lock fences commands of one process (R-27, D157); a cancel token lets `CancelRun` and
`PromoteStep` preempt a live walk (D157). The sweep runs at the first `Online` and every
`lease_ttl_seconds`; every adopted `Walk` is resumed on its own task, and a walk task that dies is
handed to `DeadWalks` so this process's next sweep adopts it (D158, R-12). Progress reaches the Runs
pane as `StoreReply::RunStream` frames on a subscription the pane opens per item (D172). With no
server, every `Orch` request is refused with MOD-25's sentence and no run starts (D174).

**The commands.** `htui-orch` gains `PromoteStep`, `AcceptArtifact`, `Unblock` and `CloseOut`
(`command.rs:28-33` reserves six — these four plus `CancelStep` and `OpenArtifact`, which D178 and
D173 leave undeclared; amended at fact-check, the draft said "exactly these"). Promotion keeps the step's identity: no
`run(kind='chat')`, the step moves to `awaiting_approval` with `promoted_at`, and the engine hands
the worker an *opening* — resume the agent-side session when `DriverCaps.resume` allows, else a
handoff prompt assembled from the step's own transcript (D163, `docs/ANA-2.md:1199-1206`). The Chat
tab attaches to the step and records the opening as a `follow_up` at the next `turn` through a
recorder that continues the step's log, leaving `prompt_digest` untouched — criterion 17 and
ANA-5 criterion 18's persistence half (D164, D165). `AcceptArtifact` runs verification and capture
and then the ordinary `approve` path, resuming at `position + 1` (D166). `Unblock` clears a blocked
item, follows an escalated run back to `awaiting_approval` through one new law edge, and resumes a
reconcile-refused park — R-4 and R-7 (D161). `CloseOut` writes one `summary` document with one row
per `(repo, step)` through the existing `close_out` transaction, behind MOD-15's two-stage
confirmation (D167).

**The pane.** Ten actions plus `run` and `close` in `runs.rs::on_key` (D168), each greyed by the
same guard the engine refuses with (`command.rs:1-8`), each re-reading the runs on a
compare-and-set miss so the table shows the actual state (D171, criterion 21). The step list shows
agent, model, gate, usage and duration in two lines per step at 43 columns (D169, D170). The Backlog
runs snapshots are re-recorded once (PRD D5; MOD-30 already fixed the strip).

**The hand-overs.** ANA-5 criterion 16's parser shipped in milestone 2 (`gate.rs:1-2`, `:86-101`);
criterion 18's persistence half lands here (D164); criterion 3's `blocked` transition lands here
(D162).

## Design decisions (settled here, not in code review)

| # | Decision | Why |
|---|---|---|
| D153 | **`RunRuntime` lives inside the store worker loop, beside `AgentRuntime` (OQ-1).** `store_worker::spawn_with` keeps its four-argument signature (called at `store_worker.rs:1057`, `:2212`, `:2289`, `:2417`) and builds `RunRuntime::production()`; a new `spawn_with_runtimes(started, rx, tx, agent, runs)` is what tests call. `Orch`, `RunStream` and `Document` requests are matched ahead of `try_serve` like the thirteen chat/probe/install/auth requests (`:1335-1359`); `try_serve`'s exhaustive `match` (`:911-1030`) gains one or-ed arm answering `Failed { "no run runtime in this build" }` for a caller with no runtime, which is exactly what the chat requests do (`:973-990`). | The loop is the only owner of `Backend` (`store_worker.rs:1-6`) and already hosts the one runtime pattern that satisfies `R-NF-3` by construction (`Served::Deferred => continue`, `:1354`). ANA-2's "no fourth `select!` arm" is about `event_loop.rs` (`docs/ANA-2.md:1687`), which this does not touch. *Amended at fact-check:* `try_serve` is `:911-1012` and its chat/probe/install/auth refusal arm is `:938-954`; `spawn`'s own call is the `:1057` site. |
| D154 | **Request and reply shapes.** `StoreRequest::Orch(OrchRequest)` where `OrchRequest` is `Command(htui_orch::Command)` plus `CloseOutPreview { item }` and `Cleanup { run }`; `StoreRequest::RunStream { item }` (a subscription); `StoreRequest::Document(DocumentId)` (a read). Replies: `StoreReply::Orch(OrchReply)` (`Done(CommandOutcome-as-text-and-rest)`, `CloseOutPreview { counts, title }`, `Promoted { step }`), `StoreReply::RunStream(RunFrame { item, run, kind })`, `StoreReply::Document(Box<Option<Document>>)`; three `name()` arms (`store_worker.rs:501`). None carries a secret (`:65-78`). | ANA-2 `:1553-1556` and `:1687-1692`: one `Orch` variant plus a `RunStream` discriminant nothing else uses, because `App::latest` keys freshness by `(Origin, Discriminant<StoreRequest>)` (`crates/htui/src/app/state.rs:158`, `:283-305`) and a later `Orch` request would otherwise orphan the stream. |
| D155 | **`BackendGraphs(Backend)` implements `GraphSource` in `run_worker.rs`,** each method delegating to the `Backend`-inherent read of the same name (`backend.rs:454-507`; `agent` filters `Backend::agents()` exactly as the `MemStore` impl does, `crates/htui-orch/src/fake.rs:852-859`). | `GraphSource` is foreign to `htui` (`crates/htui-orch/src/graph.rs:56`) and so is `Backend` (`backend.rs:49`), so `impl GraphSource for Backend` in `htui` is an E0117; `graph.rs:45` and `fake.rs:831` say "htui implements it for `Backend`", which cannot compile as written. A newtype is the standard answer and keeps invariant 10 (the orchestrator never names `htui-store`). |
| D156 | **Production parts, built once per process where state requires it.** `GixIsolator::new(IsolatorConfig)` (`crates/htui-orch/src/isolate/real.rs:314`, config `:231-245`) is built **lazily at the first `Orch` command** from `Backend::repo_paths(box)` (`backend.rs:529`) and the repos of the scope's projects, because its `shared_serialized` guards and admin locks are in-process (`real.rs:249-258`) and a second isolator would not serialise against the first; it is **rebuilt only while no walk of this process is live**, and a `StartRun` naming a repo the isolator does not know while a walk is live is refused with "a repo was added since the first run; wait for the live runs to rest" (R-39). `ShellVerifier::new(limits, scrubber, clock)` once per process (`crates/htui-orch/src/verify.rs:194-198`, "built once per process", `engine.rs:337-340`). `SystemClock` (`crates/htui-orch/src/isolate.rs:267`), `FirstCandidate` (`engine.rs:145-157`). The driver closure (`DriverFor`, `engine.rs:252-254`, infallible) looks the candidate's `agent` and `agent_box` rows up in a map read before the walk and calls `DriverFactory::driver_for` (`registry.rs:98-110`); a refusal is returned as a `RefusedDriver` whose `start` answers that `DriverError`, which the walk already fails as a spawn failure under every gate (`engine.rs:4260-4266`). `app` = `Backend::app_settings()` (`backend.rs:396`), `box_profile` = `Backend::box_profile` (`:381`), `box_id` from `Backend::box_info()` read in `run_worker.rs` itself (*amended at fact-check:* `agent_worker.rs:1733`'s `registered_box` is a private free function in T7's file, so T6 re-reads `box_info()` rather than editing `agent_worker.rs`), `user` = `Backend::this_user()` (`backend.rs:174`), `owner` = one `Uuid::now_v7()` per process. | Everything the engine borrows (`EngineParts`, `engine.rs:321-365`) has exactly one production source, and the two with in-process locks must be singletons or the locks mean nothing. `GixIsolator::new`'s own doc says "milestone 6 calls it at worker start, once per process" (`real.rs:303-306`); lazily at the first command is that, on a process that may start offline. |
| D157 | **Per-run lock and preemption (OQ-2, R-27).** `RunRuntime` holds `RunLocks` and, per live walk, a `tokio_util::sync::CancellationToken` (already a dependency, `crates/htui/Cargo.toml`). Every command, every resume and every sweep-driven walk of run *r* holds `RunLocks[r]` for its whole duration; `StartRun` holds an item lock until `create_run` returns the run id. A walk task is `select!(walk, token.cancelled())`; `CancelRun` and `PromoteStep` on a `running` step cancel the token, then wait for the lock. A dropped walk is M5 D86's abandon: its session's agent is killed by `ChildGuard::drop`, its guards are released by `walk_leased`'s own path, and its lease is released (D127/D139). | R-27 (`mod-4-orch-lease.blueprint.md:1075`): two commands of one process otherwise walk one run at once under one owner. Holding the lock for a whole walk is what makes "a second command waits" true; preemption is what keeps `cancel` usable while a session runs for hours. |
| D158 | **The sweep and supervision.** `RunRuntime` owns one `DeadWalks` (`engine.rs:276-316`) and one `owner`. `Engine::sweep` (`engine.rs:1325`) runs at the first `Online` and every `lease_ttl_seconds` (a ticker arm in the store loop, D153); each `Adopted { next: Walk }` is resumed on its own task under its run lock, a locked run is skipped for this pass, and `Parked`/`Finished`/`Error` emit a `RunStream` frame. After any walk of this process rests, `Engine::claim` (`engine.rs:570`, M5 D84) is retried for this process's `queued` runs in `queued_at` order. A walk task that ends in a `JoinError` (panic) is recorded in `DeadWalks` through a new `pub fn DeadWalks::mark(&self, run)` so the next sweep releases and adopts it. | M5 D98 made the sweep walk nothing and named `run_worker` as the caller (`engine.rs:442-448`); R-12 (`mod-4-orch-lease.plan.md:677`): a run whose walking task died inside a live process is never re-adopted until restart, because D88 keeps the sweep off its own leases. `DeadWalks::insert` is private (`engine.rs:299-301`), so the worker needs a `pub` door. |
| D159 | **`resume` parks a `running` run on a topology mismatch (R-36, criterion 3).** Where `resume_window` returns `TopologyChanged` (`engine.rs:2000-2025`, amended at fact-check) for a `running` run, the engine moves the run `running → awaiting_approval` and the item `in_progress → awaiting_approval` before releasing the lease; a run already parked is left as it is. | Criterion 3 says the run "refuses to resume after a restart and **parks** at `awaiting_approval`" (`docs/ANA-2.md:2090-2091`); M5 D150's release-only answer re-adopts the run on every sweep and appends a note each time once a worker resumes `Walk` runs (R-36, `mod-4-orch-lease.blueprint.md:1187`). A parked mismatch is a human's decision (§4.9); `CancelRun` is the way out. |
| D160 | **Primary-changing `git` verbs run on a detached task (R-26).** `merge --no-ff`, `merge --abort` and `reset --hard` (`Cli::merge_no_ff` `git.rs:656`, `Cli::abort_merge` `:742`, `Cli::reset_hard` `:610`; amended at fact-check, the draft named the second `merge_abort`; `Cli` is `Clone`, `:179`, so the task owns a clone and owned paths) are spawned inside `tokio::spawn` and awaited through the `JoinHandle`, so dropping the awaiting future (a preempted or abandoned walk) no longer drops the child's `kill_on_drop` handle mid-write. | R-26 (`mod-4-orch-lease.blueprint.md:1074`): a SIGKILL during a primary write can leave `index.lock` or `MERGE_HEAD`. D157 makes drops routine (every `cancel` of a running run), so the risk moves from "a lease loss during a primary write" to "any cancel during a merge". The `worktree add/remove` verbs touch only scratch trees and the admin area and stay as they are. |
| D161 | **`Unblock { item }` (OQ-3, OQ-4).** Three cases in order: (1) `blocked`, no non-terminal run → `transition(blocked → open)`; (2) `blocked` with an `awaiting_approval` run → `transition(blocked → awaiting_approval)` (new edge in `Status::can_move_to`, `item.rs:55`); (3) `awaiting_approval` with a run whose `cursor` is not `Rest { AwaitingApproval }`/`Fan`/`Select` → `Engine::resume(run)` (D132's path). Otherwise `EngineError::NotBlocked { item, status }`. Guard `command::unblock_enabled(item, runs, cursor)` is pure, so the pane greys `u` by the same rule. | ANA-2 §4.3 verdict 1: "cleared by exactly one human action" (`:545-552`) and the escalation row's "`retry` or `approve` … or `unblock`" (`:551`), which R-4 made unreachable (`engine.rs:800-804`, `command.rs:229-236`). R-7's resume already exists as `walk_resumed` (`engine.rs:2036-2066`); `Unblock` is the verb that calls it. |
| D162 | **A stage-3 prompt refusal blocks the item (OQ-10, ANA-5 criterion 3).** In `walk_live_step`, an `Err(EngineError::Prompt(_))` from `assemble_prompt` (`engine.rs:2387-2398`) takes a new `refuse_prompt` path: step `running → failed` through `fail_hard`'s own step write (`transition_step`, which writes **no** `gate_note`; *amended at fact-check:* the draft put the sentence in `gate_note`, but the only `running → failed` writer that sets one is `interrupt_step`, the sweep's, whose `interrupted` prefix `retry_enabled` keys on — so the sentence lives in `run.failure` and the `item_note`), item `in_progress → blocked`, then `finish_run(Failed, RunFailure::PromptRefused)` and `cleanup_run`, then an `item_note` — the order `refuse_no_candidate` uses (`engine.rs:2200-2250`). **The same refusal in `drive_group` (`engine.rs:2704-2706`, amended at fact-check)** moves the group's `pending` candidates to `failed` the way `fail_group_before_a_token` does and then takes the same item/run order. No session starts (the recorder is opened only in stage 4, `engine.rs:4258`). | ANA-5 criterion 3 (`docs/ANA-5.md:2271-2273`): "fails the step at stage 3 …, sets the item to `blocked`, and starts no session". `finish_run(Failed)` mirrors `in_progress → failed` only when the item is still `in_progress` (`traits.rs:1001-1015`), so blocking first is what keeps the item `blocked`. |
| D163 | **`PromoteStep { run, step }` (OQ-5).** Guard `command::promote_enabled(run, step)`: step `running | awaiting_approval | failed`, run non-terminal (`docs/ANA-2.md:1563`). Engine: a `running` step (reached only after D157's preemption) is moved `running → awaiting_approval` with run and item parked (`gate::park`'s order); a `failed` step whose item is `blocked` needs `Unblock` first (D161 case 2); then `promote_step(step, now)` (`traits.rs:983`). The answer is `CommandOutcome::Promoted { step, opening }`, `Opening::Resume { session_ref, cwd, extra_dirs, agent, model }` when the step's agent has `DriverCaps.resume` (`crates/htui-agent/src/driver.rs:325-352`, CLI `resume: true`, ACP from settings, `registry.rs:138-168`) and a `session_started` banner exists (`crates/htui-agent/src/event.rs:163`), else `Opening::Handoff { prompt: AssembledPrompt, … }` built by the new pure `promote.rs` through `assemble()` with the seeded `handoff` template (`crates/htui-core/src/prompt/defaults.rs:199`, `:229`). `cwd` and `extra_dirs` are the step's own `run_step_tree` paths (`docs/ANA-2.md:1208-1213`). No `run(kind='chat')` row is written. | Criterion 17 (`:2131-2134`) and `R-ORCH-5`. The `follow_up_in_session` path of §4.8 (`:1203`) is unreachable in this milestone: a walk's session ends at its `done` before the step parks, and a preempted one was killed. |
| D164 | **A recorder that continues a step's log.** `htui_agent::record::Recorder::continuing(store, scrubber, step, retain_raw, ui, tail: &[SessionEvent])` where `tail` is the step's persisted rows: `next_seq = max(seq) + 1`, `turn` = the last row's turn, `turns = turn + 1`, `prompt_digest`/`digest_pending` = `None`, and **`usage = UsageTotals::from_rows(tail)`** (*amended at fact-check:* the recorder writes the whole `run_step.usage` document from its own running total, `set_step_usage(step, self.usage.to_value(), digest)` at `record.rs:1071-1073`, and `set_step_usage` replaces the column, `mem.rs:1429-1449`; a continuation seeded with `UsageTotals::default()` would overwrite the step's pre-promotion spend with the chat's alone, so the draft's `(next_seq, turn)` alternative is dropped). The opening is recorded with `record_follow_up` (`record.rs:623`), which opens `turn + 1` (`record.rs:14-17`); nothing calls `record_prompt`, so `run_step.prompt_digest` is never rewritten (a `None` digest leaves the column alone, `mem.rs:1444-1446`). | ANA-5 criterion 18 (`docs/ANA-5.md:2316-2318`) and criterion 17's "appends `follow_up` events with an incrementing `turn` to the same `session_event` stream". `Recorder::new` always starts at `seq = 0`, `turn = 0`, `usage` default (`record.rs:455-490`, body read at fact-check), and `seq` is gapless with one writer (`record.rs:14-20`), so continuing a step needs a constructor that knows where the log ends and what it has spent. |
| D165 | **The Chat tab drives the promoted step (OQ-6).** `Action::Promote { run, step }` → `App::promote` focuses `App::replay_tab`'s tab (the Chat tab, `app/mod.rs:76`) and dispatches `StoreRequest::Orch(Command(PromoteStep))` from `Origin::Tab(chat)`. `RunRuntime` answers `RunServed::Attach { step, opening }` after the engine's writes (*amended at fact-check:* a run_worker-owned `RunServed { Reply, Deferred, Attach }` defined in T6, not a new variant of `agent_worker::Served` — T6 does not own `agent_worker.rs`, and `Served` is matched exhaustively by both the store loop, `store_worker.rs:1351-1359`, and the harness, `testkit.rs:216-219`; T6 answers `Attach` in both places with a `Failed { "promotion needs the chat runtime" }` stub and T7 replaces both stubs); the loop hands it to a new `AgentRuntime::attach_promoted(backend, tx, envelope, opening)` which starts a session with `SessionSpec.resume = Some(ref)` or a fresh session with the handoff text, records through D164's recorder, attaches the task under `step_id` (`agent_worker.rs:495`), and answers `ChatAccepted` like `start` does (`agent_worker.rs:1145-1335`). When the session ends the step is **not** finished: its status is the engine's (`awaiting_approval`, `promoted`); `finish_chat_run` is never called for a graph step. | `R-TUI-6` allows a chat "bound to a run step" and ANA-2 adopts "the same `run_step`, continued with `follow_up` events" (`:1170-1172`). Reusing the tab reuses the permission strip, the composer, the transcript and the scrubbed render channel. |
| D166 | **`AcceptArtifact { run, step }`.** Guard `command::accept_enabled(step, phase, has_output, chat_live)`: step `awaiting_approval`, `promoted_at` set, a document of `output_kind` produced by the step exists, and no chat session of this process is live on the step (the worker supplies `chat_live` from `AgentRuntime::steps`, `agent_worker.rs:438`; "end the chat first (Esc Esc)"). Engine: stage 5 for the step — `verify` (`engine.rs:2515`), `capture` + `record_commits`, `finish_step` — then the `AnswerGate(Approved)` tail (`answer_guarded`, `engine.rs:633`): step `done` with `gate_outcome = approved`, unpark, walk from `position + 1` under the lease. `GateAnswer::Skipped` stays unexposed. | ANA-2 `:1223-1233` lists exactly verify, capture, gate, resume at `position + 1`. `command.rs:88-92` calls `Skipped` "§4.8's accept-artifact shape" admitted *without* the document, which contradicts `:1229-1231`'s guard; this plan follows ANA-2 (listed under disagreements). |
| D167 | **`CloseOut { item }` and its two stages.** Stage 1: `OrchRequest::CloseOutPreview { item }` answers the item's status, how many runs and how many `(repo, step)` commit rows the summary will carry, and the summary's next version — computed by the same pure `closeout::summary(item, runs, commits, trees, repos) -> NewDocument` that stage 2 writes — or the refusal (a non-terminal run, an item not `done | failed | blocked`). Stage 2: the pane asks the item key typed back (`FEAT-1`), mirroring MOD-15's slug confirmation with every unlisted key swallowed (`crates/htui/src/ui/tabs/settings/hierarchy.rs:177-207`, `:635-735`) — which the pane can only do through OQ-7's `captures_input` seam, because the Backlog tab consumes `G`, `j`, `k`, `l`, `h`, `[`, `]` and `Enter` first (amended at fact-check); on a match `Command::CloseOut { item }` calls `WriteStore::close_out(item, summary, &[])` (`traits.rs:1045-1050`), `produced_by_step_id = None`, `created_by = user`. The body is a generated markdown table, one row per `(repo, step)` with a `run_step_commit.after_hash`, `repo · phase attempt · before..after`, plus one line per run (`kind`, `status`, `finished_at`). | Criterion 20 (`docs/ANA-2.md:2141-2143`) and §4.10 (`:1380-1392`): "records commit hashes" is the body plus the rows that already exist, so `commits` is empty. `close_out` already refuses a live run and an illegal item status before its first write (`traits.rs:1034-1043`); the preview surfaces the refusal before the typed stage. |
| D168 | **Runs pane bindings (OQ-7).** As listed in OQ-7. Each key is enabled by the `command.rs` guard for the selected row (`answer_gate_enabled` `:413`, `retry_enabled` `:459`, `select_enabled` `:522`, `retry_group_enabled` `:569`, `cancel_enabled` `:662`, plus D161/D163/D166's new ones); a disabled key writes the guard's sentence to the status line through `Action::Error` and sends nothing. `x` opens a one-line `TextField` (`crates/htui/src/ui/text_field.rs`) for the note, with `captures_input` true while it is open (OQ-7, amended at fact-check); `s` requires the cursor on a candidate of a parked slot. | `command.rs:1-8`: "the Runs tab greys an action out by exactly the rule the engine refuses it by … so the rules live here as free functions over rows". |
| D169 | **Step rows at 43 columns (OQ-8).** Two lines per step as in OQ-8; run header rows unchanged (`detail/runs.rs:21-25`); a run with `failure` gains a third header line with the failure text truncated. A unit test renders every fixture row at `PANE_WIDTH = 43` and asserts no line exceeds it. | Criterion 21 (`:2144-2146`); every field it names is already on `RunStepSummary` (`crates/htui-core/src/model/run.rs:718-764`: `agent_name`, `model`, `gate_outcome`, `usage`, `started_at`, `finished_at`, `selected`, `promoted_at`), built by the three builders milestone 1 kept in step, so **no projection change and no mirror change**. |
| D170 | **Deterministic usage and duration.** Duration is `finished_at − started_at` rendered `45s`/`12m`/`1h04`; a step without `finished_at` renders `…` (no wall clock in `render`, so snapshots stay byte-stable, `crates/htui/src/testkit.rs:3-7`). Usage renders `$0.42` from the usage document's `cost_micros` when non-zero, else `12k` tokens (`input_tokens + output_tokens`), else `—`. The document is exactly `UsageTotals`' five nullable keys — `input_tokens`, `output_tokens`, `cache_read_tokens`, `cache_write_tokens`, `cost_micros` (`crates/htui-core/src/model/usage.rs:26-38`, `to_value` `:77-85`); parse it with `serde_json::from_value::<UsageTotals>` (amended at fact-check, was UNVERIFIED). | The harness renders without sleeps and without a clock; a live "elapsed" figure would make every snapshot of a running step flaky. |
| D171 | **A compare-and-set miss renders the actual state.** Every `StoreReply::Failed` for an `Orch` request already reaches the status line (`app/update.rs:139-148`); in addition the Runs pane re-requests `StoreRequest::Runs(item)` on any `Orch` reply, failed or not, so the table redraws from the rows. | Criterion 21's "each renders the actual state on a compare-and-set miss" (`:2144-2145`). The engine's refusals (`StaleWrite`, `NotGated`, `RunStatus`, `LeaseHeld`) are already sentences (`command.rs:172-395`). |
| D172 | **`RunStream` frames are invalidations.** The pane sends `StoreRequest::RunStream { item }` when a `Runs` reply for a new item arrives (it has a `Ctx` there, `detail/runs.rs:215`); `RunRuntime` keeps the latest `(origin, seq, item)` per origin and sends `RunFrame { item, run, kind: Started | SessionDone | Rested(Rest) | Adopted | Error(String) }` for that item's runs; the pane re-requests `Runs(item)` on each. `SessionDone` comes from a production `SessionSink` whose `after_done` (`engine.rs:172-190`) posts to the worker. | ANA-2 `:1687-1692`. The engine has no step-start hook, and adding one would touch every walk path; a frame per session end and per rest is enough for a human-paced pane (R-40). |
| D173 | **Open artifact is a read-only view in the Runs pane (OQ-9)**, fed by `StoreRequest::Document(id)`; the document is the newest one of `output_kind` with `produced_by_step_id = step` from the `Documents` reply the Backlog tab already requests (`DocumentHead`, `crates/htui-core/src/model/document.rs:36-50`). No `Command` variant: ANA-2's `OpenArtifact` is a read, not an orchestrator command. | Keeps the registry rule (`detail/mod.rs:4-7`) and needs one read over an existing `ReadStore` method (`traits.rs:88`). |
| D174 | **Nothing regresses offline.** `RunRuntime::serve` checks `backend.writer()` (`backend.rs:152-158`) before building anything and answers `Failed { DATABASE_UNREACHABLE }` (the sentence `orchestration_offline` uses, `backend.rs:607-609`) for every `Orch` request; no sweep runs while the backend is `Offline`; `RunStream` and `Document` answer from the mirror like any read. | PRD metric `:197`; MOD-25. No new seam method means no new `BufferedWriter` arm is owed. |
| D175 | **Criterion 19 re-scoped (OQ-11).** Proven in `run_worker.rs` tests over `MemStore`'s `MemFault` hook (`htui_core::store::mem::MemFault`, `test-support`, M5 D152): a refresh outage fences the walk (`Heartbeat::Expired`, M5 D122) and the run enters `DeadWalks`; after the fault clears, this process's next sweep adopts it. | ANA-2 `:2138-2140` presumes an offline writer MOD-25 removed. |
| D176 | **No secret wiring (OQ-13).** `engine.rs:4358`'s comment is rewritten to name the owning item. | No provider exists to wire. |
| D177 | **R-25: a manual cleanup retry.** `OrchRequest::Cleanup { run }` on a terminal run calls `Engine::cleanup_run` (`engine.rs:4008`, `pub` for this reason per `mod-4-orch-lease.blueprint.md:1021`); bound to `T`. | R-25: a failed terminal cleanup is never retried (`:1021`). A manual, idempotent retry is the smallest honest close. |
| D178 | **`CancelStep` is not built.** `CancelRun` only. | ANA-2 `:1564` offers either; nothing in criterion 21 needs the step form, and a cancelled step under a live run has no row in §4.3's run table. |
| D179 | **`cancel_run` takes the lease (R-28).** `Engine::cancel_run` (`engine.rs:1058-1089`) takes the lease through `take_lease` for a `running`/`awaiting_approval` run before its first write (a live stranger's lease → `LeaseHeld`, nothing written) and releases it after the cleanup; a `queued` run has no lease to take. The `refresh_lease` status clause R-28 also suggests is **not** done (it is SQL and would move `.sqlx`). | R-28 (`mod-4-orch-lease.blueprint.md:1076`): from a TUI, `cancel` can target a run another process is walking. |
| D180 | **R-31, the part this milestone needs.** `walk_resumed` honours `unpark`'s `Ok(false)` as `StaleWrite` (`engine.rs:2046`, `unpark` `:4403-4418`), and the three untested D132 crash paths get a case each. A crash right after `AnswerGate(Rejected)` is left parked on a failed step (R-31's remainder, carried). | `Unblock` case (3) calls `resume` on parked runs routinely, so the re-merge-and-note on every call R-31 describes (`:1118`) would now be user-visible. |

## What this milestone touches from the carried list

| Carried | Disposition |
|---|---|
| **R-3** (`run.failure` NULL on a parked run), M2 blueprint `:27` | **Carried.** The pane shows `awaiting_approval` and the step's gate; the reason is on the Notes sub-tab. Adding `gate_note` to `RunStepSummary` would touch three builders and the mirror. Owner: a later Runs-tab refinement. |
| **R-4** (a `blocked` item cannot resume), M2 F-J | **Closed** by D161 case (2) and the new law edge (OQ-4). |
| **R-5** (no `gate_outcome = skipped` writer) | **Carried.** The step list renders `—` for a NULL gate. |
| **R-6** (`phase_agent` copy, `is_override` writers) | **Untouched.** Owner: MOD-15's phase editor. |
| **R-7** (reconcile-refusal park has no resume verb), `HANDOFF.md:281-285` | **Closed** by D161 case (3), which calls the existing `walk_resumed` (`engine.rs:2036-2066`). `command.rs:505-521`'s sentence is rewritten to name `Unblock`. |
| **R-9** (`NoProgressReview` unreachable) | **Carried**; proposed as its own `CLEAN` item (changes which stop reason shipped loop cases reach). |
| **R-10** (a SIGKILLed orchestrator's agent survives) | **Carried** to MOD-16 (signalling a stale pid needs `unsafe` or a dependency). |
| **R-12** (a dead walk task inside a live process is not re-adopted) | **Closed** by D158 (`DeadWalks::mark` on `JoinError`). |
| **R-25** (failed terminal cleanup never retried) | **Closed** by D177 (manual retry). |
| **R-26** (drop SIGKILLs a primary-changing `git` child) | **Closed** by D160. |
| **R-27** (no per-run mutex in one process) | **Closed** by D157. |
| **R-28** (`cancel_run` without the lease) | **Closed** by D179 (the `refresh_lease` status clause is not done). |
| **R-29**, **R-30** | **Recorded only**, unchanged. |
| **R-31** (`walk_resumed` ignores `unpark`'s answer; untested crash paths) | **Partly closed** by D180; the rejected-crash resume stays carried. |
| **R-32** (diagnostic detail lost) | **Carried**. |
| **R-36** (a mismatched `running` run re-adopted each sweep) | **Closed** by D159. |
| **R-37** (`reconcile_parent` opens the checkout six times) | **Carried** (speed only; OQ-12 offers folding it into T5). |
| **M4 behaviour 5** (`drive_group` not cancel-safe, `HANDOFF.md:311-313`) | Already closed by M5 D86/D95/D99; D157 relies on it: a preempting drop is the intended abandon. |
| **OQ-4 of M4** (every production judge fails until MOD-11, `HANDOFF.md:325-327`) | **Relied on.** Production fan-outs park for a human; `s` (`SelectFanout`) is how the maintainer finishes them. A dedicated pane test pins the path. |
| **PRD D5 / MOD-30** (strip fits, done 2026-09-24) | **Relied on.** The Backlog runs snapshots are re-recorded once, in T8. |
| **ANA-5 criterion 16** (front-matter parser) | **Already delivered** in milestone 2 (`crates/htui-orch/src/gate.rs:1-2`, `:36-101`), with M2 D10's deviation (only an exact `request-changes` rejects). Nothing to do. |
| **ANA-5 criterion 18** (persistence half) | **Delivered here** (D164, D165). |
| **ANA-5 criterion 3** (`blocked`) | **Delivered here** (D162). |
| **Criterion 19** (offline window, M5 D103) | **Re-scoped and delivered** (D175). |

## Patterns to Mirror

| Concern | Mirror | Where |
|---|---|---|
| A runtime served ahead of `try_serve`, deferred work on its own task | `AgentRuntime::serve` + the loop's `Served` arm | `crates/htui/src/agent_worker.rs:515-649`; `crates/htui/src/store_worker.rs:1335-1359` |
| A read served as deferred work on an owned `Backend` clone | `StoreRequest::PromptPreview` | `store_worker.rs:120-134`; `agent_worker.rs:789` (`preview`) |
| Focus a tab and address a reply to it | `App::replay` | `crates/htui/src/app/update.rs:129-136`; `app/mod.rs:76` |
| Two-stage destructive confirmation, keys swallowed | `Mode::Deleting` / `DeleteStage` | `crates/htui/src/ui/tabs/settings/hierarchy.rs:177-207`, `:635-735` |
| A step-bound chat session with frames per event | `AgentRuntime::start` | `agent_worker.rs:1145-1335` |
| Item blocked before the run fails, with a note | `refuse_no_candidate` | `crates/htui-orch/src/engine.rs:2200-2250` |
| A command's lease window released on error | `Engine::leased_window` | `engine.rs:1246` (M5 D149) |
| Pure enabling guard over rows | `retry_enabled`, `select_enabled` | `crates/htui-orch/src/command.rs:459`, `:522` |
| `htui-orch` conformance case + two pins | `CASES` + `cases_are_unique_and_fifty_two` + `cases_len_is_fifty_two` | `crates/htui-orch/src/conformance.rs:292`, `:4307`; `crates/htui-orch/tests/fake_conformance.rs:15-16` |
| Store outage in a test | `MemFault` on `MemStore` | M5 D152, `htui_core::store::mem::MemFault` (`test-support`) |
| Snapshot tests over the harness | `the_six_sub_tabs_render_the_selected_item` | `crates/htui/tests/backlog.rs:144-190` |
| A width pinned by a test | `the_detail_strip_fits_the_detail_pane` | `crates/htui/src/ui/tabs/backlog/mod.rs:281` |

## Files to Change

| File | Action | Task | Why |
|---|---|---|---|
| `crates/htui-core/src/model/item.rs` | edit | T1 | `Blocked → AwaitingApproval` in `can_move_to` (`:55`) and `SANCTIONED` (`:262-285`) (D161) |
| `crates/htui-agent/src/record.rs` | edit | T2 | `Recorder::continuing` + unit tests (D164) |
| `crates/htui-agent/tests/recorder.rs` | edit | T2 | a continued log is gapless and keeps `prompt_digest` (criterion 18) |
| `crates/htui-orch/src/closeout.rs` | create | T3 | pure `summary(..) -> NewDocument` and `preview` counts (D167) |
| `crates/htui-orch/src/promote.rs` | create | T3 | pure `opening(..)` choice and the handoff `PromptSpec` builder (D163) |
| `crates/htui-orch/src/lib.rs` | edit | T3, T4 | `pub mod closeout; pub mod promote;` (T3); re-exports of new command types (T4) |
| `crates/htui-orch/src/isolate/git.rs` | edit | T5 | detached primary-changing verbs (D160) |
| `crates/htui-orch/tests/gix_isolator.rs` | edit | T5 | a dropped reconcile leaves no `index.lock`/`MERGE_HEAD` |
| `crates/htui-orch/src/command.rs` | edit | T4 | four `Command` variants, `CommandOutcome::{Promoted, Accepted, Unblocked, ClosedOut}`, `Opening`, guards `promote_enabled`/`accept_enabled`/`unblock_enabled`/`close_out_enabled`, `EngineError::NotBlocked`/`NotPromoted`/`ChatLive`, the R-7 sentence (`:505-521`), the module doc (`:1-3`, `:28-33`) |
| `crates/htui-orch/src/engine.rs` | edit | T4 | `promote`, `accept_artifact`, `unblock`, `close_out`, `cancel_run` lease (D179), `resume` park on mismatch (D159), `refuse_prompt` (D162), `walk_resumed` stale unpark (D180), `DeadWalks::mark` (D158), `dispatch` arms (`:491-508`), secrets comment (`:4358`) |
| `crates/htui-orch/src/status.rs` | edit | T4 | `RunFailure::PromptRefused` + `Display` test |
| `crates/htui-orch/src/fake.rs` | edit | T4 | harness: promotion, a live-chat flag, prompt-refusal scripting |
| `crates/htui-orch/src/conformance.rs` | edit | T4 | new cases; `CASES` 52 → ~63 and its in-file pin (`:292`, `:4307`) |
| `crates/htui-orch/tests/fake_conformance.rs` | edit | T4 | the out-of-crate pin (`:15-16`) |
| `Cargo.lock` | regenerate | T6 | `htui` gains `htui-orch` |
| `crates/htui/Cargo.toml` | edit | T6 | `htui-orch = { workspace = true }`; dev-dep with `test-support`; workspace dep entry if absent |
| `Cargo.toml` | edit | T6 | `htui-orch` in `[workspace.dependencies]` (`Cargo.toml:23-26` lists the other three path crates; `htui-orch` appears only in `members`, `:3`) |
| `crates/htui/src/run_worker.rs` | create | T6 | `RunRuntime`, `BackendGraphs`, `RunLocks`, sweep, `RunStream` subscribers, `ProgressSink`, `RefusedDriver` (D153–D158, D172, D174, D175, D177) |
| `crates/htui/src/lib.rs` | edit | T6 | `pub mod run_worker;` |
| `crates/htui/src/store_worker.rs` | edit | T6, T7 | T6: `Orch`/`RunStream`/`Document` variants, replies, `name()` arms, `try_serve` arm, loop arm, sweep ticker, `spawn_with_runtimes`; T7: the `Served::Attach` hand-off to `AgentRuntime` |
| `crates/htui/src/app/action.rs` | edit | T6 | `Action::Promote { run, step }` |
| `crates/htui/src/app/update.rs` | edit | T6 | `App::promote` (mirror of `replay`, `:129-136`) |
| `crates/htui/src/testkit.rs` | edit | T6 | `Harness::with_run_runtime(..)` serving `Orch` inline over `MemStore` + fakes |
| `crates/htui/src/agent_worker.rs` | edit | T7 | `attach_promoted` (D165) |
| `crates/htui/src/ui/tabs/chat/mod.rs` | edit | T7 | header names the promoted step (`step · phase · promoted`) and the opening path (`resumed` / `handoff`) |
| `crates/htui/tests/chat.rs` | edit | T7 | promotion attaches, follow-ups land at `turn + 1`, no chat run row |
| `crates/htui/tests/snapshots/chat__*.snap` | add | T7 | one promoted-chat frame |
| `crates/htui/src/testkit.rs` | edit | T7 | replace T6's `RunServed::Attach` stub with `attach_promoted` in the harness's inline serve (amended at fact-check) |
| `crates/htui/src/ui/tabs/backlog/detail/runs.rs` | edit | T8 | actions, step rows, close-out stages, artifact view, `RunStream` subscription (D166–D173) |
| `crates/htui/src/ui/tabs/backlog/detail/mod.rs` | edit | T8 | `DetailTab::captures_input` (default `false`) and `DetailRegistry::captures_input` (OQ-7, amended at fact-check) |
| `crates/htui/src/ui/tabs/backlog/mod.rs` | edit | T8 | one guard at the top of `on_key`: a capturing sub-tab gets every key first (OQ-7, amended at fact-check) |
| `crates/htui/tests/snapshots/replay__runs_step_selected.snap` | re-record | T8 | renders the Runs pane's step rows (`tests/replay.rs:251`), so D169 changes it (amended at fact-check) |
| `crates/htui/tests/backlog.rs` | edit | T8 | pane cases over the harness |
| `crates/htui/tests/snapshots/backlog__detail_runs.snap`, `backlog__empty_runs.snap` | re-record | T8 | the new step rows |
| `crates/htui/tests/snapshots/backlog__runs_*.snap` | add | T8 | close-out warn, close-out typed, artifact view, reject note |
| `crates/htui/tests/runs_pg.rs` | create | T9 | criteria 17 and 20 over Postgres through the worker |

**Not touched, on purpose:** every migration and `cache_migrations/` file; `crates/htui-store/src/**`
(no new seam method, so no `PgStore`, `Writer` or `BufferedWriter` arm) and `.sqlx/`;
`crates/htui-core/src/store/**` (the transition case iterates the law); `RunStepSummary` and its
three builders (`pg/rows.rs`, `mem.rs`, `cache/read.rs`); `event_loop.rs` (ANA-2 `:1687`);
no action `match` arm in `crates/htui/src/ui/tabs/backlog/mod.rs` (ANA-2 `:1694-1697`; the pane
subscribes itself) — *amended at fact-check:* the file does gain OQ-7's one generic
`captures_input` guard, in T8; `settings/kinds.rs`'s `MOD-4 owns these` label (`:94`, not in this milestone's row);
`queue.rs` (MOD-12's); `docs/**`, `HANDOFF.md`, the PRD (the deviations are the main thread's to
record).

## Tasks

**T1 alone first. Then Wave A: T2 ∥ T3 ∥ T5, each in its own git worktree, merged T2, then T5, then
T3, with the `htui-agent` and `htui-orch` gates re-run on the real tree after each merge. Then T4.
Then T6. Then Wave B: T7 ∥ T8, each in its own worktree, merged T7 then T8, with the `htui` gate
re-run after each merge. Then T9.** Independence is decided by intersecting the file sets below, and
by build coupling (M5 D105): a red or mid-edit commit in a dependency crate stops every dependent
crate compiling, which is why every parallel task runs in its own worktree.

| Task | Files (complete list) | Parallel |
|---|---|---|
| T1 | `crates/htui-core/src/model/item.rs` | first, alone, until green |
| T2 | `crates/htui-agent/src/record.rs`, `crates/htui-agent/tests/recorder.rs` | Wave A, own worktree, merged first |
| T5 | `crates/htui-orch/src/isolate/git.rs`, `crates/htui-orch/tests/gix_isolator.rs` | Wave A, own worktree, merged second |
| T3 | `crates/htui-orch/src/closeout.rs`, `crates/htui-orch/src/promote.rs`, `crates/htui-orch/src/lib.rs` | Wave A, own worktree, merged third |
| T4 | `crates/htui-orch/src/command.rs`, `crates/htui-orch/src/engine.rs`, `crates/htui-orch/src/status.rs`, `crates/htui-orch/src/fake.rs`, `crates/htui-orch/src/conformance.rs`, `crates/htui-orch/src/lib.rs`, `crates/htui-orch/tests/fake_conformance.rs` | serial, after T1 and Wave A |
| T6 | `Cargo.toml`, `Cargo.lock`, `crates/htui/Cargo.toml`, `crates/htui/src/run_worker.rs`, `crates/htui/src/lib.rs`, `crates/htui/src/store_worker.rs`, `crates/htui/src/app/action.rs`, `crates/htui/src/app/update.rs`, `crates/htui/src/testkit.rs` | serial, after T4 |
| T7 | `crates/htui/src/agent_worker.rs`, `crates/htui/src/store_worker.rs`, `crates/htui/src/testkit.rs`, `crates/htui/src/ui/tabs/chat/mod.rs`, `crates/htui/tests/chat.rs`, `crates/htui/tests/snapshots/chat__*.snap` (amended at fact-check: `testkit.rs` added; `chat__*` widened because a header change may re-record existing chat frames) | Wave B, own worktree, merged first |
| T8 | `crates/htui/src/ui/tabs/backlog/detail/runs.rs`, `crates/htui/src/ui/tabs/backlog/detail/mod.rs`, `crates/htui/src/ui/tabs/backlog/mod.rs`, `crates/htui/tests/backlog.rs`, `crates/htui/tests/snapshots/backlog__detail_runs.snap`, `crates/htui/tests/snapshots/backlog__empty_runs.snap`, `crates/htui/tests/snapshots/backlog__runs_*.snap`, `crates/htui/tests/snapshots/replay__runs_step_selected.snap` (amended at fact-check: the two Backlog files for OQ-7's capture seam, and the replay snapshot, which renders the step rows) | Wave B, own worktree, merged second |
| T9 | `crates/htui/tests/runs_pg.rs` | serial, last |

**Intersections, checked.** Wave A: T2 ∩ T3 ∩ T5 = ∅ (three different directories); `lib.rs` is T3's
and T4's only, never concurrently. Wave B: T7 ∩ T8 = ∅ — T7 owns `store_worker.rs`,
`agent_worker.rs` and the chat files; T8 owns the Runs pane and the Backlog test files; neither
edits `app/*` (T6 put `Action::Promote` and its routing there). `store_worker.rs` and `testkit.rs`
are in T6 and T7, which are serial. *Re-checked at fact-check with the amended lists:* T7 ∩ T8 = ∅
still — T7 adds `testkit.rs` and `chat__*.snap`, T8 adds `backlog/mod.rs`, `detail/mod.rs` and
`replay__runs_step_selected.snap`; `tests/replay.rs` itself is untouched by both. The draft lists
were incomplete (the replay snapshot renders the Runs pane; `Served` is matched exhaustively in the
harness), not intersecting. **Build coupling, checked:** T1 changes a `const fn` body and a test table, with
no signature change; T2 adds an associated function and changes no existing signature; T3 adds two
modules that nothing calls yet; T5 changes private bodies behind unchanged `Cli` signatures
(UNVERIFIED: the implementer confirms no signature in `git.rs` changes; fact-check: feasible without one, since `Cli` is `Clone`, `git.rs:179`, so each body can move a clone and owned paths into the task). T7's and T8's red commits
are failing assertions over existing public APIs plus T6's `Action::Promote`, so each compiles on
its own tree. **Hidden coupling, checked:** only T6 changes `Cargo.lock`; only T8 changes
`backlog__*.snap` and `replay__*.snap`; only T7 changes `chat__*.snap`; no task changes `.sqlx`, a
seed, a migration or `RunStepSummary`; `CASES` moves only in T4. **Wave A re-checked at
fact-check:** T2 = {`htui-agent/src/record.rs`, `htui-agent/tests/recorder.rs`}, T5 =
{`htui-orch/src/isolate/git.rs`, `htui-orch/tests/gix_isolator.rs`}, T3 =
{`htui-orch/src/closeout.rs`, `htui-orch/src/promote.rs`, `htui-orch/src/lib.rs`}: pairwise ∅; none
adds a crate dependency (so no `Cargo.lock` move), a snapshot, a `.sqlx` file, a seed or a shared
case list; `skip_without_git!` is `#[macro_export]`ed from `git.rs:1877` and T5 only uses it.

Every implementer prompt carries: PRD D1–D8 win over this plan where they disagree; read the tree,
not `graphify-out/` (deleted); no migration and no new seam method — if one seems needed, stop and
report; nothing sets `updated_at` by hand; the only `git` subprocesses stay in `isolate/git.rs` and
`verify.rs`; **commit incrementally** (uncommitted subagent work does not survive the session, and
there is no stash on a shared tree); verify your gate with `--test-threads=1` on the real tree after
your merge.

### Task 1: `htui-core` — the escalated item may follow its run back (D161, OQ-4)
- **Files**: `crates/htui-core/src/model/item.rs`.
- **Test first**: `SANCTIONED` (`:262-285`) gains `(Status::Blocked, Status::AwaitingApproval)`;
  `the_item_status_table_sanctions_exactly_the_ana_2_pairs` fails until `can_move_to` changes. Its
  doc cites this plan's D161 as the one deviation from `docs/ANA-2.md:583-584`.
- **Action**: `Self::Blocked => matches!(to, Self::Open | Self::AwaitingApproval | Self::Closed)`
  (`:55`); the `can_move_to` doc (`:38-45`) names the deviation.
- **Commit boundary**: one red commit (the table), one green commit (the law).
- **Validate**: `cargo test -p htui-core --all-features -- --test-threads=1`, then
  `USERNAME=htui-ci HTUI_TEST_DATABASE_URL=… cargo test -p htui-store --all-features
  -- --test-threads=1` (the store transition case, `conformance.rs:6392`, iterates the law and must
  stay green on both stores). *Amended at fact-check:* that case (`illegal_transitions_are_constraint`,
  `conformance.rs:6378`) drives the item through `open`, `queued`, `in_progress` and `done` only
  (`:6381-6388`), so `blocked` is never a `from` and the new edge is **not** re-proved on either
  store by it; it stays green, and the edge's store-level proof is T4's
  `unblock_lets_an_escalated_run_be_promoted_and_approved` (MemStore) and T9 (Postgres, below).
  PgStore enforces the law in Rust before its `UPDATE` (`crates/htui-store/src/pg/write.rs:595-603`,
  `legal_move`), and no migration carries a status trigger, so no SQL changes.

### Task 2: `htui-agent` — a recorder that continues a step's log (D164)
- **Files**: `crates/htui-agent/src/record.rs`, `crates/htui-agent/tests/recorder.rs`.
- **Test first** (`record.rs` unit tests over `MemStore`):
  `a_continued_recorder_starts_after_the_last_seq_and_turn`;
  `a_continued_follow_up_opens_the_next_turn`;
  `a_continued_recorder_never_writes_prompt_digest` (the step's digest is unchanged after
  `finish`); `continuing_an_empty_log_is_seq_zero_turn_zero`;
  `a_continued_recorder_keeps_the_step_s_earlier_usage` (amended at fact-check: the continuation's
  `set_step_usage` must carry the pre-promotion totals plus the new rows). `tests/recorder.rs`:
  `a_handoff_is_a_follow_up_at_the_next_turn_not_a_second_prompt` (ANA-5 criterion 18's
  persistence half: one `prompt` row at seq 0 before and after, a `follow_up` row at `turn + 1`).
- **Action**: `pub fn continuing(store, scrubber, step, retain_raw, ui, tail: &[SessionEvent])
  -> Self` (amended at fact-check: the tail, not `(next_seq, turn)`, because `usage` must be
  seeded with `UsageTotals::from_rows(tail)` — D164; the caller reads the tail with
  `ReadStore::step_events`, `traits.rs:80`, which answers `Option<Vec<_>>`, `None` = not cached),
  sharing `new`'s field initialisation; module doc items 2 and 3 (`record.rs:14-25`) gain one
  sentence each.
- **Mirror**: `Recorder::new` (`record.rs:455`).
- **Validate**: `cargo test -p htui-agent --all-features -- --test-threads=1`; clippy.

### Task 3: `htui-orch` — the two pure modules (D163, D167)
- **Files**: `crates/htui-orch/src/closeout.rs` (new), `crates/htui-orch/src/promote.rs` (new),
  `crates/htui-orch/src/lib.rs`.
- **Test first** (`closeout.rs`): `one_row_per_repo_and_step_with_an_after_hash`;
  `a_step_that_committed_nothing_has_no_row`; `rows_are_in_position_attempt_repo_order`;
  `the_summary_is_kind_summary_produced_by_nobody`; `the_preview_counts_what_the_summary_holds`.
  (`promote.rs`): `a_resumable_agent_with_a_banner_resumes`; `no_banner_means_handoff`;
  `a_non_resumable_agent_means_handoff`; `the_handoff_spec_carries_the_windowed_tail_and_the_failure`
  (over `htui_core::prompt::fixtures`' handoff events, `crates/htui-core/src/prompt/fixtures.rs:395-486`, `handoff_events` at `:432-486`; amended at fact-check);
  `the_opening_uses_the_step_s_own_trees_as_cwd`.
- **Action**: `closeout::summary(item: &Item, runs: &[RunSummary], commits: &[(RunStep,
  Vec<RunStepCommit>)], repos: &[Repo], id: DocumentId, user: UserId, at) -> NewDocument` and
  `closeout::Preview { runs, rows, version }`; `promote::opening(caps: DriverCaps, banner:
  Option<AgentSessionRef>) -> OpeningKind` and `promote::handoff_spec(..) -> PromptSpec` (the
  `handoff` field `Some`, role `Handoff`, `crates/htui-core/src/prompt/mod.rs:104-110`). `lib.rs`:
  `pub mod closeout; pub mod promote;`.
- **Validate**: `cargo test -p htui-orch --all-features --lib -- closeout:: promote::`; clippy.

### Task 4: `htui-orch` — the four commands and the carried engine fixes (D158–D163, D166, D167, D179, D180)
- **Files**: the T4 row above.
- **Test first**, new `CASES` (52 → ~63, the exact figure pinned in the commit that adds the last):
  `promote_keeps_the_step_and_writes_no_chat_run` (**criterion 17, first half**: same `run_step`
  id, `promoted_at` set, run and item `awaiting_approval`, zero `run(kind='chat')` rows);
  `promote_a_failed_step_of_a_parked_run`; `promote_refuses_a_terminal_run`;
  `accept_artifact_verifies_captures_and_resumes_at_the_next_position` (**criterion 17, second
  half**); `accept_artifact_needs_the_document_and_the_promotion`;
  `unblock_opens_a_blocked_item_with_no_run` (**criterion 14's `Unblock` half**, over rung four's
  `no_candidate_agent`, `engine.rs:594-611`, since `R-ORCH-10`'s capability refusal is MOD-7's);
  `unblock_lets_an_escalated_run_be_promoted_and_approved` (R-4);
  `unblock_resumes_a_reconcile_refused_park` (R-7, over `FakeIsolator`'s reconcile refusal);
  `close_out_writes_one_summary_and_closes_the_item` and
  `close_out_is_refused_while_a_run_is_live` (**criterion 20**);
  `a_prompt_refusal_blocks_the_item_and_starts_no_session` (**ANA-5 criterion 3**);
  `a_topology_mismatch_parks_a_running_run` (**criterion 3**, D159, replacing M5's
  `a_topology_mismatch_on_a_running_run_leaves_it_adoptable_by_the_same_process`,
  `engine.rs:9540`); `cancel_meets_a_live_lease_and_writes_nothing` (D179). Engine unit tests:
  `a_marked_dead_walk_is_adopted_by_the_next_sweep` (D158); `walk_resumed_stops_on_a_stale_unpark`
  and one case per untested D132 crash path (D180). `command.rs` guard tests for the four new
  guards, each asserting the named refusal.
- **Action**: as the Files table. `Command` doc (`command.rs:27-38`) and the module doc (`:1-3`)
  list nine verbs; `dispatch` (`engine.rs:491-508`) gains four arms. `promote` computes the opening
  through `promote::` and reads the banner with `ReadStore::step_events`. `accept_artifact` reuses
  `walk_live_step`'s stage-5 helpers rather than duplicating them (factor a `stage_five(run, step,
  phase, trees)` out of `walk_live_step`, `engine.rs:2354-2515`, if the implementer finds no
  existing seam; that factoring is its own commit).
- **Commit boundaries**: (a) command types + guards + guard tests; (b) D162 + D159 + D179 + D180 +
  D158 with their tests; (c) promote + accept; (d) unblock; (e) close-out; (f) pins.
- **Validate**: `cargo test -p htui-orch --all-features -- --test-threads=1`; clippy;
  `cargo doc -p htui-orch --no-deps --all-features` exits 0.

### Task 5: `htui-orch` — primary writes survive a dropped walk (D160, R-26)
- **Files**: `crates/htui-orch/src/isolate/git.rs`, `crates/htui-orch/tests/gix_isolator.rs`.
- **Test first** (`gix_isolator.rs`, `skip_without_git!()`):
  `a_reconcile_dropped_mid_merge_leaves_no_index_lock` — poll a reconcile once with
  `now_or_never` after the merge child is spawned (the implementer finds the seam: a
  `FakeIsolator`-free real repository with a `pre-merge-commit` hook that sleeps is one way),
  drop it, wait, and assert no `.git/index.lock` and no `MERGE_HEAD`.
- **Action**: `Cli::merge_no_ff`, `Cli::abort_merge` and `Cli::reset_hard` (amended at
  fact-check: the draft said `merge_abort`) spawn their child inside `tokio::spawn` over a cloned
  `Cli` and owned paths, and await the `JoinHandle`; the `with_retry` wrapper is unchanged.
- **Validate**: `cargo test -p htui-orch --all-features --test gix_isolator -- --test-threads=1`;
  `grep -rn 'Command::new' crates/htui-orch/src/` still hits `isolate/git.rs` and `verify.rs` only.

### Task 6: `htui` — `run_worker.rs` and the request plumbing (D153–D158, D172, D174, D175, D177)
- **Files**: the T6 row above.
- **Test first** (`run_worker.rs` unit tests over `Backend::Memory` + `FakeIsolator` +
  `FakeDriver`, through `spawn_with_runtimes`):
  `an_orch_request_is_served_off_the_loop` (the loop answers a `Items` request while a walk is
  mid-session — the `the_loop_answers_other_requests_while_a_probe_is_in_flight` shape,
  `store_worker.rs:2212`); `two_commands_on_one_run_are_serialised` (R-27);
  `cancel_preempts_a_live_walk` (D157); `the_sweep_resumes_each_adopted_run_on_its_own_task`;
  `a_panicked_walk_is_adopted_by_the_next_sweep` (R-12); `a_refused_claim_is_retried_when_a_walk_rests`
  (M5 D84); `an_offline_backend_refuses_every_orch_request_with_one_sentence` (D174, PRD `:197`);
  `a_store_outage_fences_the_walk_and_the_sweep_adopts_it_after` (D175);
  `run_stream_frames_reach_the_subscribed_origin_only`; `backend_graphs_delegates_each_read`
  (D155); `the_isolator_is_built_once_per_process`; `cleanup_retries_a_terminal_run` (D177).
  `app/update.rs`: `promote_focuses_the_chat_tab_and_addresses_it`.
- **Action**: `htui-orch` as a dependency (and dev-dependency with `test-support`); `RunRuntime`
  with `production()` and `with_parts(..)` taking `Arc<dyn Isolator>`, `Arc<dyn Verifier>` and a
  `DriverFactory` (both traits are `?Sized` in `EngineParts`, `engine.rs:321-330`);
  `StoreRequest::{Orch, RunStream, Document}`, the replies, `name()` arms, the loop arm, the
  `try_serve` arm, a sweep ticker arm; `Action::Promote` and `App::promote`; the harness hook.
- **Commit boundaries**: (a) dependency + `BackendGraphs` + tests; (b) request/reply variants and
  the `try_serve` arm (compiles, refuses); (c) `RunRuntime::serve` for `Command`; (d) locks and
  preemption; (e) sweep and supervision; (f) `RunStream`; (g) offline and outage cases; (h)
  `Action::Promote`; (i) harness.
- **Validate**: `cargo test -p htui --all-features -- --test-threads=1`; clippy on the workspace;
  `cargo doc --workspace --no-deps --keep-going` shows no new error.

### Task 7: `htui` — the Chat tab drives a promoted step (D164, D165)
- **Files**: the T7 row above.
- **Test first** (`tests/chat.rs` over the harness with both runtimes):
  `promotion_opens_the_chat_on_the_same_step`; `a_follow_up_lands_at_the_next_turn_of_the_step`
  (criterion 17: `session_event.turn` increments on the same `run_step_id`);
  `promotion_writes_no_chat_run`; `a_promoted_session_end_leaves_the_step_awaiting`;
  `the_handoff_opening_is_one_follow_up_row` (ANA-5 criterion 18). One snapshot,
  `chat__promoted`.
- **Action**: `AgentRuntime::attach_promoted`; the loop hands `Served::Attach` to it; the chat
  header shows `promoted · <phase> · resumed|handoff`.
- **Validate**: `cargo test -p htui --all-features -- --test-threads=1`; clippy.

### Task 8: `htui` — the Runs pane (D166–D173)
- **Files**: the T8 row above.
- **Test first** (`runs.rs` unit tests and `tests/backlog.rs`): one test per binding asserting the
  emitted request for an enabled row and the status sentence (no request) for a disabled one —
  `a_approves_a_parked_step`, `x_asks_for_a_note_then_rejects`, `r_retries`, `p_emits_promote`,
  `c_asks_then_cancels`, `o_opens_the_artifact_read_only`, `s_selects_a_candidate`, `u_unblocks`,
  `shift_a_accepts_the_artifact`, `shift_r_starts_a_run`, `shift_c_counts_then_asks_for_the_key`,
  `t_retries_the_cleanup` (**criterion 21's reachability**, `docs/ANA-2.md:2144`);
  `a_refusal_re_reads_the_runs` (D171); `a_run_stream_frame_re_reads_the_runs` (D172);
  `every_step_row_fits_forty_three_columns` (D169); `a_running_step_shows_no_duration` (D170);
  `a_parked_fanout_shows_its_candidates_and_s_picks_one` (M4 OQ-4's human path);
  `the_close_out_key_must_match_and_other_keys_are_swallowed` (D167). Snapshots:
  `backlog__detail_runs` and `backlog__empty_runs` re-recorded; new `backlog__runs_closeout_warn`,
  `backlog__runs_closeout_typed`, `backlog__runs_artifact`, `backlog__runs_reject_note`.
- **Action**: as D166–D173; the module doc (`runs.rs:1-6`) describes the actions.
- **Validate**: `cargo test -p htui --all-features -- --test-threads=1`; review every changed
  `.snap` with `cargo insta review` before accepting; clippy.

### Task 9: Postgres end to end (criteria 17, 20)
- **Files**: `crates/htui/tests/runs_pg.rs` (new).
- **Test first**: `a_promoted_step_continues_its_own_log_on_postgres` and
  `close_out_is_one_transaction_on_postgres` over `htui_store::testkit::TestDb` (the
  throwaway-database harness `chat_usage_pg.rs` uses: `let Some(db) = testkit::demo_db().await
  else { … print testkit::SKIP … }`, then `db.store`, `db.pool` and `db.drop_db().await` —
  `crates/htui-store/src/testkit.rs:39-66`, `:170`, `:196`; amended at fact-check, was
  UNVERIFIED), each skipping without `HTUI_TEST_DATABASE_URL`. Add
  `unblock_follows_an_escalated_run_on_postgres` (amended at fact-check: the only proof of D161's new
  item-law edge on `PgStore`, since the store transition case never starts from `blocked`).
- **Validate**: the workspace gate below.

## Test plan

TDD per repo convention: every task's first commit is its failing tests, named in the task.

**`htui-orch` conformance (`FakeDriver` + `FakeIsolator` + `MemStore`)**: ~11 new cases in T4, by
criterion: 3 (one, replacing an M5 case), 14's `Unblock` half (one), 17 (four), 20 (two), ANA-5 3
(one), R-4/R-7 (two), R-28 (one).

**Store conformance**: no new case. *Amended at fact-check:* the existing transition case stays
green but does not exercise `blocked` as a `from`, so it does not re-prove the new edge; that proof
is T4 (MemStore) and T9 (Postgres).

**`htui` unit and harness tests**: T6's worker cases (criterion 19 as re-scoped, `R-NF-3`, R-12,
R-27, offline); T7's promotion cases (criterion 17 and ANA-5 18 through the Chat tab); T8's twelve
binding cases (criterion 21) and the layout pins.

**Postgres**: T9's three cases (amended at fact-check: the `unblock` edge case added).

**Real git**: T5's dropped-merge case.

**Count pins that move:**

| Pin | Now | After | Where |
|---|---|---|---|
| `htui-orch` `CASES` | 52 | ~63 | `crates/htui-orch/src/conformance.rs:292`; `cases_are_unique_and_fifty_two` `:4307` (name and message); `crates/htui-orch/tests/fake_conformance.rs:15-16` (`cases_len_is_fifty_two`) |
| `htui-core` store `CASES` | 53 | 53 | `crates/htui-core/src/store/conformance.rs:37`; `crates/htui-core/tests/mem_store.rs:36`; `crates/htui-store/tests/pg_conformance.rs:19` |
| `READ_CASES` | 9 | 9 | `conformance.rs:219` |
| `.sqlx` files | 227 | 227 | `crates/htui-store/.sqlx/` |
| `StoreRequest` variants | 58 (second count at fact-check: 58, enum `:79-499`) | 61 | `crates/htui/src/store_worker.rs:79-499` |
| Migration pins | unchanged | unchanged | `crates/htui-store/tests/migrations.rs`, `connect.rs:95` |

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| **R-38** — Preempting a running step (D157, D163) kills the agent without ANA-4 §4.3's grace window and without answering parked permission requests | Certain on every `cancel`/`promote` of a running step | Recorded deviation (OQ-5); the step is re-derived from rows and the transcript is intact up to the kill. A graceful path needs a cancel seam inside `pump`, which is a later refinement |
| **R-39** — One isolator per process (D156): a repo added in Settings while a walk is live is unknown until every walk rests | Low | Named refusal; the isolator is rebuilt when no walk is live |
| **R-40** — `RunStream` frames are invalidations at session end and rest only; a step's `pending → running` is not signalled | Certain | Human-paced pane; the next frame, or re-selecting the item (which re-issues `Runs`), shows it — *amended at fact-check:* the draft said "`R` refresh", but D168 binds `R` to `run`, and no refresh key exists. A step-start hook is a later refinement |
| **R-41** — An `Orch` reply can arrive hours after its request; a newer `Orch` request from the same origin makes it stale and it is dropped (`app/state.rs:283-305`) | Medium | The walk's result also reaches the pane as a `RunStream` frame and the rows; a dropped *error* is still in the Notes/`run.failure` |
| **R-42** — D161's new item-law edge changes a table ANA-2 states as complete (`docs/ANA-2.md:563-589`) | Certain | The main thread amends ANA-2 §4.3; T1's table test pins the one deviation |
| **R-43** — The close-out summary is generated; there is no human prose in it | Certain | `R-TUI-9` asks for commit hashes and status; an editable summary is MOD-13's editor's |
| **R-44** — Step rows at 43 columns truncate `agent/model` | Certain for long model ids | `…` marks the cut; D169's width test keeps it from clipping silently |
| **R-45** — `htui` now links `gix`, `process-wrap` and `walkdir` through `htui-orch` (build time, binary size, MOD-16's Windows facts) | Certain | Recorded for MOD-16 |
| **R-46** — A walk task keeps the `Backend` clone it was started with; after an `Online → Offline` swap its `PgStore` handle keeps failing until the heartbeat fences (M5 D122) | Medium | That is the fence's job; the sweep after reconnect adopts the run (D175) |
| **R-47** — Deviations the main thread must record: OQ-3/OQ-4 (item law), OQ-5 (grace), OQ-9 (artifact view), OQ-11 (criterion 19), D162 (ANA-5 criterion 3 generalised), D166 (`GateAnswer::Skipped` unexposed), D178 (`CancelStep`) | Medium | Listed here and under disagreements; each has a failing case if the literal reading returns |

## Validation

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-features --all-targets -- -D warnings
USERNAME=htui-ci HTUI_TEST_DATABASE_URL=postgres://postgres:htui@localhost:5439/postgres \
  cargo test --workspace --all-features -- --test-threads=1
# from crates/htui-store, against a migrated scratch database (the compose `htui` one is empty):
DATABASE_URL=postgres://postgres:htui@localhost:5439/htui_prepare_check \
  cargo sqlx prepare --check -- --all-targets --all-features   # must stay at 227 files
cargo doc --workspace --no-deps --keep-going   # no error beyond the M5 baseline's two
cargo doc -p htui-orch --no-deps --all-features  # exits 0
# snapshots (T7, T8): record, then review every change before accepting
cargo insta test -p htui --all-features -- --test-threads=1
cargo insta review
```

`--test-threads=1` is not optional (the keyring fake is process-wide). `cargo sqlx prepare --check`
is run although no query changes, because T6 adds a crate dependency edge and a stray `query!` would
show there first. The `cargo doc` baseline at M5's close was exactly two errors
(`crate::store::MIRRORED_TABLES` in `htui-core`, `step_exists` in `htui-store`;
`mod-4-orch-lease.plan.md:705-709`). **Confirmed unchanged at `ba68682` by the fact-check**
(amended, was UNVERIFIED): `cargo doc --workspace --no-deps --keep-going` exits 101 with exactly
those two errors, now at `htui-core/src/store/traits.rs:954` and `htui-store/src/pg/write.rs:3476`
(plus the two "could not document" summaries); `htui-agent`, `htui-orch` and `htui` document
cleanly; `cargo doc -p htui-orch --no-deps --all-features` exits 0. Git-backed cases
skip without `git` ≥ 2.33.0; this box has 2.43.0.

## Acceptance

- [ ] Criterion 17 passes over the fakes (T4), through the Chat tab (T7) and on Postgres (T9): one
      `run_step` id, `follow_up` rows at an incrementing `turn`, `promoted_at` set, no chat run,
      `prompt_digest` unchanged; `accept artifact` verifies, captures, marks `done` and resumes at
      `position + 1`.
- [ ] Criterion 20 passes over the fakes and on Postgres; the close-out is refused while a run is
      live and behind the typed confirmation in the pane.
- [ ] Criterion 21: all seven `R-TUI-4` actions plus `Unblock`, `AcceptArtifact`, `CloseOut`, `run`
      and the cleanup retry are reachable from `runs.rs::on_key`; each disabled key names the guard's
      reason; a refusal re-reads the rows; every step row shows agent, model, gate, usage and
      duration within 43 columns.
- [ ] Criterion 3 parks; criterion 14's `Unblock` half passes; ANA-5 criterion 3 blocks the item;
      ANA-5 criterion 18's persistence half passes.
- [ ] `R-NF-3`: no `Orch` request is awaited on the UI task; the loop answers other requests while a
      walk is mid-session.
- [ ] Offline: every `Orch` request is refused with MOD-25's sentence and no run starts.
- [ ] R-4, R-7, R-12, R-25, R-26, R-27, R-28, R-36 closed as tabled; criterion 19 re-scoped and
      proved.
- [ ] `htui-orch` `CASES` pinned at its new figure in both places; store `CASES` 53, `READ_CASES` 9,
      `.sqlx` 227; no migration; `grep -rn 'Command::new' crates/htui-orch/src/` hits
      `isolate/git.rs` and `verify.rs` only.
- [ ] The workspace gate above is green.

## Where the PRD, ANA-2 or HANDOFF disagree with the tree

1. **ANA-2 §6.2 cites `docs/REQUIREMENTS.md:254-256` for `R-TUI-4`** (`docs/ANA-2.md:1553`); the
   requirement is now at `:287-289`.
2. **`graph.rs:45` and `fake.rs:831` say `htui` implements `GraphSource` for `Backend`**; the orphan
   rule forbids it (both types are foreign to `htui`). D155's newtype.
3. **ANA-2 promotes a `running` step** (`:1182-1197`, `:641`, `:1563`); `WriteStore::promote_step`
   accepts `failed | awaiting_approval` only (`crates/htui-core/src/store/traits.rs:974-983`). D163
   moves the step first.
4. **`command.rs:88-92` calls `GateAnswer::Skipped` the accept-artifact shape, admitted without a
   document**; ANA-2 guards accept artifact on the document and lands `approved`
   (`docs/ANA-2.md:1223-1231`, `:644`). D166 follows ANA-2.
5. **ANA-2 §4.3's item table has no road back from `blocked` to a parked run** (`:583-584`), while
   its escalation row is cleared by "`retry` or `approve`" (`:551`). D161 adds one edge.
6. **Criterion 19 needs an offline writer** (`:2138-2140`); MOD-25 removed it
   (`backend.rs:152-158`). D175.
7. **The PRD scope reads ANA-5 criterion 16 as "unparseable read as `request-changes` with a note"**
   (`.claude/prds/mod-4-orchestrator-manual-mode.prd.md:252-255`, ANA-5 `:2309-2311`, amended at
   fact-check); the tree
   reads only an exact `request-changes` as a rejection (`gate.rs:36-59`, M2 D10). This milestone
   keeps M2 D10.
8. **ANA-5 criterion 3 wants the item `blocked`**; the tree fails the run and the item
   (`engine.rs:2339-2346`). D162.
9. **The PRD's Evidence counts 51 `StoreRequest` variants** (`:124`); the tree has 58 (counted
   at `store_worker.rs:79-499`, confirmed by a second count at fact-check).
10. **`engine.rs:4358` says milestone 6 wires secrets**; no provider exists. D176.
11. **PRD D5 says MOD-30 lands "before milestone 7"** (`:357-360`); after the D0 renumbering the
    Runs-tab milestone is 6 (`:312`, `:314-320`). MOD-30 is done, so nothing is blocked.
12. **`command.rs:505-521` says `select_enabled`'s R-7 case is "milestone 6's `Unblock`-shaped
    verb"**, and `HANDOFF.md:281-285` says the same; the resume walk already exists
    (`engine.rs:2036-2066`) and needs only a verb (D161).
13. **ANA-2 §6.2 lists `CancelStep`** (`:1564`); not built (D178).

---

## Verified claims

| Claim | Verdict | Evidence |
|---|---|---|
| HEAD is `ba68682` on `mod-4-m6` | confirmed | `git log --oneline -1` |
| `htui` does not depend on `htui-orch` | confirmed | `crates/htui/Cargo.toml` `[dependencies]`/`[dev-dependencies]` name `htui-agent`, `htui-core`, `htui-store` only |
| `htui-orch` is only in `members`, absent from `[workspace.dependencies]` | confirmed | `Cargo.toml:2-3` (members), `:24-26` (three path crates) |
| `tokio-util` (`CancellationToken`) is already an `htui` dependency | confirmed | `crates/htui/Cargo.toml:39` |
| `impl GraphSource for Backend` in `htui` is E0117; a newtype compiles | confirmed | scratch crates under `/tmp/orphan_probe` (trait crate `a`, enum crate `b`, impl in `c`): `error[E0117]: only traits defined in the current crate can be implemented for types defined outside of the crate`; `struct BackendGraphs(b::Backend)` + impl: `cargo check` OK |
| `graph.rs:45` and `fake.rs:831` say `htui` implements it for `Backend` | confirmed | `graph.rs:45`, `fake.rs:831`; trait at `graph.rs:56` |
| `MemStore`'s `agent` filters `agents()` | confirmed | `fake.rs:854-861` |
| `Backend`-inherent reads `phase_agents`/`prompt_template`/`resolve_graph`/`agent_boxes` | confirmed | `backend.rs:454`, `:468`, `:487`, `:501` |
| `Backend::writer()` answers `None` offline | confirmed | `backend.rs:152-158` |
| `Backend` line cites: enum `:49`, `this_user` `:174`, `box_profile` `:381`, `app_settings` `:396`, `repo_paths` `:529`, `orchestration_offline` `:607-609`, `document` `:671` | confirmed | read at each line |
| `store_worker.rs:1-6`: the worker is the only `Backend` owner | confirmed | `store_worker.rs:1` |
| `StoreRequest` has 58 variants (PRD says 51) | confirmed | second count (script over the enum body): 58, enum `:79-499`; PRD `:124` says 51 |
| `name()` at `:501`; no-secret doc `:65-78` | confirmed | `impl StoreRequest` `:501`, `name` `:504`; doc `:71-78` |
| 13 chat/probe/install/auth requests served ahead of `try_serve` via `Served` | confirmed | `store_worker.rs:1338-1359`; `Served` `agent_worker.rs:258-271` |
| `try_serve` `:911-1030`, its no-runtime arm `:973-990` | partial | `try_serve` is `:911-1012`, the arm `:938-954` (D153 amended) |
| `spawn_with` four args, called at `:1057`, `:2212`, `:2289`, `:2417` | confirmed | `store_worker.rs:1064-1069`; `:1057` is `spawn`'s own call |
| store `select!` `:1110-1115`; `go_online` `:1511`; `go_offline` `:1547`; `SetDsn` block `:1166-1236` | confirmed | `select!` `:1111`, ticker arm `:1436`; `:1511`, `:1547`; `SetDsn` arm `:1168` |
| `agent_worker.rs`: `steps` `:438`, `attach` `:495`, `serve` `:515-649`, `preview` `:789`, `start` `:1145-1335` | confirmed | read at each line |
| `box_id` "via the registered box (`agent_worker.rs:1733`)" usable from `run_worker` | partial | `registered_box` is a private free fn in T7's file (`agent_worker.rs:1733`); D156 amended to read `box_info()` in `run_worker` |
| `App::latest` keyed by `(Origin, Discriminant<StoreRequest>)` | confirmed | `app/state.rs:158`, `dispatch` `:283-296`, `is_fresh` `:304` |
| `App::replay` focuses the replay tab and dispatches from its origin; `replay_tab = ChatTab` | confirmed | `app/update.rs:129-136`; `app/mod.rs:76` |
| `StoreReply::Failed` reaches the status line | confirmed | `app/update.rs:147-149` |
| Runs pane answers `J`, `K`, `Enter` only; `on_reply` has a `Ctx` | confirmed | `detail/runs.rs:200-213`, `:215` |
| Runs pane is 43 columns, two lines per run, trim figure in the phase cell | confirmed | `detail/runs.rs:21-25`, `:34-42`, `PANE_WIDTH` `:468` |
| Taken keys: Backlog `j k g G l h [ ] Enter`, pane `J K`, global `q ? Tab BackTab 1-9`, overlay `Esc` | confirmed | `backlog/mod.rs:203-229`, `detail/mod.rs:293-294`, `detail/runs.rs:202-203`, `keymap.rs:200-236` |
| The Runs pane can take a typed note / typed key with every unlisted key swallowed (D167, D168) | falsified | `BacklogTab::on_key` consumes `j k g G l h [ ] Enter` and arrows before `self.detail.on_key` (`backlog/mod.rs:210-226`); OQ-7, D167, D168, T8 amended with a `captures_input` seam |
| `DetailRegistry` cannot switch siblings; Documents renders heads only | confirmed | `detail/mod.rs:79-189` (no focus API), `detail/documents.rs:1` |
| `detail/mod.rs:4-7` registry rule | confirmed | `detail/mod.rs:4-7` |
| MOD-30 strip width test | confirmed | `backlog/mod.rs:281` |
| MOD-15 two-stage delete (`Mode::Deleting`/`DeleteStage`) | confirmed | `settings/hierarchy.rs:178`, `:193`, `:666-732` |
| `ui/text_field.rs` exists; `settings/kinds.rs:94` "MOD-4 owns these" | confirmed | both read |
| `command.rs:28-33` reserves exactly `PromoteStep`, `AcceptArtifact`, `Unblock`, `CloseOut` | falsified | it lists six: also `CancelStep` and `OpenArtifact` (`command.rs:28-33`); Summary amended |
| Guard lines `answer_gate_enabled` `:413`, `retry_enabled` `:459`, `select_enabled` `:522`, `retry_group_enabled` `:569`, `cancel_enabled` `:662` | confirmed | `command.rs` at each line |
| `command.rs:1-8` "greys by exactly the rule"; `:88-92` `Skipped` is the accept-artifact shape | confirmed | read |
| `command.rs:505-521` R-7 sentence; `HANDOFF.md:281-285` R-7 | confirmed | `command.rs:514-516`; `HANDOFF.md:281-283` |
| R-4: `ItemBlocked` refusal | confirmed | `command.rs:229-236`, `engine.rs:800-804` |
| `EngineError::Prompt(#[from] AssembleError)` exists | confirmed | `command.rs:330` |
| `EngineParts` built at six sites `engine.rs:5045`, `:5383`, `:5470`, `:7872`, `conformance.rs:5046` | partial | actual `engine.rs:5031`, `:5360`, `:5447`, `:7858`, `conformance.rs:5032`, plus `tests/gix_isolator.rs:88` (macro) (OQ-2 amended) |
| `EngineParts` `:321-365`; `I`/`V` are `?Sized` | confirmed | `engine.rs:321-365`, bounds `:324-330` |
| `FirstCandidate` `:145-157`; `SessionSink::after_done` `:172-190`; `DriverFor` `:252-254` (infallible) | confirmed | read |
| `DeadWalks` `:276-316`, `insert` private `:299-301`, R-27 race named in its doc | confirmed | `engine.rs:279-316`, `fn insert` `:300`; doc `:268-273` |
| `dispatch` `:491-508` has five arms | confirmed | `engine.rs:491-508` |
| `claim` `:570` (M5 D84); `sweep` `:1325` walks nothing; `leased_window` `:1246`; `answer_guarded` `:633`; `verify` `:2515` | confirmed | read |
| `sweep`/`resume`/`claim` document milestone 6 as their caller | confirmed | `engine.rs:442-446`, `:560-561`, `:1941-1944` |
| `Engine::resume` releases the lease on a topology mismatch (D150) | confirmed | `engine.rs:1950-1962` |
| `resume_window` mismatch block `:1997-2023` | partial | it is `:2000-2025` (D159 amended) |
| `walk_resumed` unparks an `awaiting_approval` run whose cursor is `Create`/`Run`/`Finished` and re-reconciles the frontier | confirmed | `engine.rs:2036-2059` |
| `walk_resumed` ignores `unpark`'s `bool` (R-31) | confirmed | `engine.rs:2045`; `unpark` `:4403-4416` returns `Ok(unparked)` |
| `cancel_run` `:1058-1089` takes no lease | confirmed | `engine.rs:1058-1089` |
| `cleanup_run` `:4008` is `pub` for milestone 6 | confirmed | `engine.rs:4004-4008` |
| `refuse_no_candidate` blocks the item before `finish_run(Failed)` | confirmed | `engine.rs:2224-2235`; stage-1 doc `:2067-2070` |
| Criterion 3's prompt error fails the item via `walk_step`'s error arm `engine.rs:2339-2346` | confirmed | `:2339-2346` → `fail_hard` (`:2608-2632`) → `finish_run(Failed)`; mirror `in_progress -> failed` (`traits.rs:1284`) |
| The `AssembleError` escapes `walk_live_step` at `:2385-2388`, the only call site | partial | `.await?` at `:2387-2389` (conversion at `:4171`); `drive_group` has the same escape at `:2704-2706` (OQ-10, D162 amended) |
| D162 writes the sentence as the step's `gate_note` through `fail_hard`'s step write | falsified | `fail_hard` uses `transition_step` (`engine.rs:2615-2619`), which writes no `gate_note`; only `interrupt_step` does (`traits.rs:846-853`) (D162 amended) |
| `finish_run(Failed)` leaves a `blocked` item alone | confirmed | `finish_run_item_mirror` `traits.rs:1281-1290` |
| Recorder opened only in stage 4 | confirmed | `engine.rs:4258`, `open_recorder` `:4294-4319` |
| Spawn failure fails under every gate | confirmed | `engine.rs:4259-4265` |
| `drive_once` builds `SessionSpec.env` empty with the "milestone 6 wires them" comment | confirmed | `engine.rs:4358-4359` |
| No secret provider exists; owner item | confirmed | no `SecretProvider` type in `crates/`; columns `hierarchy.rs:66-68`; owner MOD-10 (`HANDOFF.md:398`) (OQ-13 UNVERIFIED resolved) |
| `promote_step` accepts `failed \| awaiting_approval` only, refuses a terminal run | confirmed | `traits.rs:974-983`; `mem.rs:3934-3948` |
| `close_out` refuses a live run and an item not `done \| failed \| blocked` before any write | confirmed | `traits.rs:1034-1050` |
| Every write this milestone needs exists (`promote_step`, `close_out`, `add_note`, `transition`, `write_document`) | confirmed | `traits.rs:211`, `:972`, `:983`, `:1045`, `:1057` |
| `ReadStore::step_events` `:80`, `document` `:88`, `legal_move` `:1124` | confirmed | read |
| `Status::can_move_to` `Blocked -> Open \| Closed` (`item.rs:55`), a `const fn`; `SANCTIONED` `:262-285`; doc `:38-45` | confirmed | `item.rs:46-59`, `:262-290` |
| Both stores enforce the item law in Rust; no SQL change | confirmed | `mem.rs` via `legal_move`; `pg/write.rs:595-603`; no status trigger in any migration |
| The store transition case iterates `Status::ALL` and re-proves the new edge on both stores | falsified | it iterates targets from `open`, `queued`, `in_progress`, `done` only (`conformance.rs:6381-6392`); `blocked` is never a `from` (T1, test plan amended; a T9 Postgres case added) |
| Escalation parks run `awaiting_approval`, item `blocked` | confirmed | `gate.rs:996-997` |
| ANA-5 criterion 16 parser delivered in M2 with D10's deviation | confirmed | `gate.rs:1-2`, `:36-58`, `parse_verdict` `:86-102` |
| `Recorder::new` starts at `seq = 0`, `turn = 0` | confirmed | `record.rs:455-490` (UNVERIFIED body read) |
| `record_follow_up` opens `turn + 1`; `seq` gapless, one writer | confirmed | `record.rs:623-640`; doc `:14-20` |
| A continuing recorder needs only `(next_seq, turn)` | falsified | `usage` is a running total written whole (`record.rs:1071-1073`) and `set_step_usage` replaces the column (`mem.rs:1443`); D164 and T2 amended to seed `UsageTotals::from_rows(tail)` |
| `usage` JSON keys | confirmed | `input_tokens`, `output_tokens`, `cache_read_tokens`, `cache_write_tokens`, `cost_micros` (`usage.rs:26-38`, `:77-85`) (D170 UNVERIFIED resolved) |
| `RunStepSummary` carries `agent_name`, `model`, `gate_outcome`, `usage`, `started_at`, `finished_at`, `selected`, `promoted_at`; three builders | confirmed | `run.rs:718-764`; builders `pg/rows.rs:145`, `mem.rs:1126`, `cache/read.rs:557` |
| `DocumentHead` has `produced_by_step_id` | confirmed | `document.rs:34-51` |
| `DriverCaps.resume`: CLI `true`, ACP from settings; `SESSION_STARTED`; `SessionSpec.resume` | confirmed | `driver.rs:325-352`, `:273`; `registry.rs:165`, `:184`; `event.rs:163` |
| `DriverFactory::production` `:76-110`, `driver_for` `:98-110` | partial | `production` `:76`, `driver_for` `:108-123` (no plan text depends on the span) |
| Seeded `handoff` template; `PromptSpec.handoff` | confirmed | `defaults.rs:199`, `:229`; `prompt/mod.rs:109-110` |
| Handoff fixture events `fixtures.rs:395-441` | partial | ids `:395-412`, `handoff_events` `:432-486` (T3 amended) |
| `GixIsolator::new` `:314`, config `:231-245`, in-process guards/admin locks `:247-256`, "milestone 6 calls it … once per process" `:303-306` | confirmed | read |
| `ShellVerifier::new(limits, scrubber, clock)` `:194-198`, "built once per process" | confirmed | `verify.rs:156`, `:194-198` |
| `SystemClock` `isolate.rs:267` | confirmed | struct `:265`, impl `:267` |
| Primary-changing verbs are `merge_no_ff`, `merge_abort`, `reset_hard` | partial | the abort is `Cli::abort_merge` (`git.rs:742`); `merge_no_ff` `:656`, `reset_hard` `:610`; `Cli: Clone` `:179` (D160, T5 amended) |
| `Command::new` only in `isolate/git.rs` and `verify.rs` | confirmed | 3 hits in `git.rs`, 1 in `verify.rs` |
| `MemFault` in `htui_core::store::mem`, `test-support` | confirmed | `mem.rs:63-77` (`RefreshLease`, `ReleaseLease`, `ItemTransition`); reachable from `htui` tests through `htui-orch/test-support` → `htui-core/test-support` (`htui-orch/Cargo.toml:15`) |
| `FakeIsolator` can refuse a reconcile | confirmed | `fake.rs:160` `refuse_reconcile` |
| `htui-orch` `CASES` = 52, pinned at `conformance.rs:292`/`:4307` and `fake_conformance.rs:15-16` | confirmed | count 52; `cases_are_unique_and_fifty_two` `:4307`; `cases_len_is_fifty_two` `:15-17` |
| Store `CASES` = 53 (`conformance.rs:37`, `mem_store.rs:36`, `pg_conformance.rs:19`); `READ_CASES` = 9 (`:219`) | confirmed | counts 53 and 9; `EXPECTED_CASES = 53` `pg_conformance.rs:19` |
| `.sqlx` has 227 files | confirmed | `ls crates/htui-store/.sqlx \| wc -l` = 227 |
| `testkit::TestDb` helper names | confirmed | `htui-store/src/testkit.rs:39` `TestDb { store, pool, … }`, `demo_db` `:170`, `drop_db` `:196`, `SKIP` `:32` (T9 UNVERIFIED resolved) |
| Harness renders with no clock | confirmed | `crates/htui/src/testkit.rs:3-7` |
| `the_six_sub_tabs_render_the_selected_item` `tests/backlog.rs:144-190` | confirmed | `tests/backlog.rs:144` |
| `the_loop_answers_other_requests_while_a_probe_is_in_flight` at `store_worker.rs:2212` | confirmed | test at `:2208`, its `spawn_with` at `:2212` |
| Only T8 re-records Runs-pane snapshots (`backlog__detail_runs`, `backlog__empty_runs`) | partial | `replay__runs_step_selected.snap` (`tests/replay.rs:251`) renders the step rows too; added to T8 |
| T7's file list is complete (`Served::Attach` in `agent_worker`) | partial | `Served` is matched exhaustively in `testkit.rs:216-219` and the loop; `RunServed` moved to T6, `testkit.rs` added to T7 (D165 amended) |
| Numbering: highest decision D152, highest risk R-37 | confirmed | max over every `mod-4-*` plan/blueprint: D152 and R-37, both in `mod-4-orch-lease.blueprint.md` (`:1170`, `:1207`) |
| Lease blueprint rows R-25 `:1021`, R-26 `:1074`, R-27 `:1075`, R-28 `:1076`, R-31 `:1118`, R-36 `:1187` | confirmed | read |
| Lease plan D103 `:263`, R-12 `:677`, doc baseline `:705-709` | confirmed | read |
| `cargo doc` baseline: two errors, unchanged at `ba68682` | confirmed | `cargo doc --workspace --no-deps --keep-going` exit 101: `MIRRORED_TABLES` (`traits.rs:954`), `step_exists` (`pg/write.rs:3476`); `cargo doc -p htui-orch --no-deps --all-features` exit 0 (Validation UNVERIFIED resolved) |
| ANA-2 cites: `:551`, `:563-589`, `:641`, `:1182-1197`, `:1553`, `:1563-1566`, `:1687-1697`, criteria `:2090`, `:2122`, `:2131`, `:2138`, `:2141`, `:2144` | confirmed | read at each line |
| ANA-5 criterion 3 `:2271-2273`; 16 `:2313-2315`; 18 `:2319-2321` | partial | 3 confirmed; 16 is `:2309-2311`, 18 is `:2316-2318` (header, D164, disagreement 7 amended) |
| REQUIREMENTS `R-ORCH-5` `:196`, `R-TUI-2` `:283`, `R-TUI-4` `:287`, `R-TUI-9` `:302`, `R-NF-3` `:326` | confirmed | `grep -n` on `docs/REQUIREMENTS.md` |
| PRD milestone 6 `:312`; metrics `:194-197`; D5 `:357-360` | confirmed | read |
| Reviewer is `rust-reviewer` (`workflow-config.json:2`) | confirmed | `.claude/workflow-config.json:2` |
| R-40's "`R` refresh" | falsified | D168 binds `R` to `run`; no refresh key exists (R-40 amended) |
