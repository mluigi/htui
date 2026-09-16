# Plan: MOD-25 (`htui` is online-only — disable the offline buffered-write path)

Routed as **plan** (maintainer-accepted, 2026-09-16). Two maintainer rulings scope it: the masked
DSN field is **deferred to MOD-15**, and the `R-STO-6` residue in `docs/REQUIREMENTS.md` **is**
cleaned here.

## Objective

Make a chat on a box whose Postgres is unreachable **refuse with the unreachable-database sentence**
instead of recording into `<cache_dir>/pending/`, while `Writer::Buffered`, `BufferedWriter`,
`append_pending`, `upload_pending` and every test that proves them **stay in the tree and keep
compiling** for one release, so the reversal costs nothing. Clean the one piece of `R-STO-6` residue
the maintainer authorised. Everything else the item names (the `CLEAN-N` mint, the MOD-17/18/19 line
deletion, `docs/decisions/mod/mod-25.md`, the DSN field) is close-out bookkeeping or deferred, and is
listed as out of scope below.

## Context

**Already landed, not redone.** `docs/REQUIREMENTS.md:11-12` carries the 2026-09-11 amendment line;
`R-STO-4` at `:130-132` reads "When Postgres is unreachable, the TUI opens in offline read-only mode
from the cache ... No item creation, no runs."; `R-STO-7` at `:141` is "(withdrawn) ... by maintainer
decision MOD-25". `docs/decisions/mod/mod-2.md` already restates ANA-4 §11 criterion 12 as
withdrawn-with-the-mode. So scope point (1) of the HANDOFF item is done except for the `R-STO-6`
residue in Task 4.

**The machinery to disable.** The single production decision point is `Backend::writer()`,
`crates/htui-store/src/backend.rs:143-151`: the `Offline` arm at `:147-149` builds
`Writer::Buffered(BufferedWriter::new(cache.clone()))`. Its own doc at `:139-141` reserved this
reversal in advance — *"The return type stays `Option` even though no variant answers `None` today:
`writable()` is the paired API and stays `Option`, and a fourth variant that cannot write should be a
one-arm change here rather than a signature change at every call site."*

Exactly three production callers exist, all in `crates/htui/src/agent_worker.rs`: the probe at `:736`,
the chat start at `:1163`, and `recording_writer` (install/login) at `:1742`. Each already has an
`ok_or_else` for `None`. The probe and install/login paths *already* refuse a `Buffered` writer
before spawning (`:742-746`, `:1744-1748`) with `REGISTRY_ON_SERVER_ONLY`; the re-probe skips it
(`:1290-1291`); `project_caps_for` and `quota_latch_for` special-case it (`:1410`, `:1447`). Only the
chat start (`:1159-1166`) accepts it. After this item no production consumer accepts
`Writer::Buffered` at all.

**Where the warning already lives — no second surface is built.** `R-STO-4` read-only mode ships
today as the top bar's store field `offline · <age>` (`backend.rs:84-96` → `crates/htui/src/ui/top_bar.rs:39`,
pinned by `crates/htui/tests/snapshots/shell__offline_label.snap`), plus the status line, which
renders every `StoreReply::Failed` as `{request}: {message}` (`crates/htui/src/app/update.rs:132-133`;
the request name is `"chat_start"`, `crates/htui/src/store_worker.rs:244`), plus the Chat tab body,
which shows a refusal in `theme.error` whenever no session is open
(`crates/htui/src/ui/tabs/chat/mod.rs:571-581`). The standing warning stays the top bar; the refusal
sentence is what the chat says over it. The top bar's word stays `offline`, which is `R-STO-4`'s own
word.

**Design decision — how "disabled" is spelled.** Four candidates, weighed against "reversal costs
nothing" and "stays in the tree, compiling":

| Candidate | Verdict | Why |
|---|---|---|
| **A. `Backend::writer()` answers `None` on `Offline`** (one arm, `backend.rs:147-149`) | **Chosen** | It is the seam the code reserved for exactly this (`backend.rs:139-141`). One production line; every caller already handles `None`. `BufferedWriter` / `Writer::Buffered` / `append_pending` / `upload_pending` stay `pub` and re-exported (`crates/htui-store/src/lib.rs:30`), still directly constructible by the suites that prove them. Reversal is restoring one arm. |
| B. Guard only in `AgentRuntime::start` (`agent_worker.rs:1163`) | Rejected | Leaves `Backend::writer()` advertising an offline write path no consumer accepts — the seam would lie, and the disable would sit in a consumer rather than at the point the docs designate. |
| C. Cargo feature (`offline-buffer`) gating the arm | Rejected | The documented test command is `cargo test --workspace --all-features` (`README.md:468`); feature unification would re-enable the buffer under the standard run, so the refusal proof and the buffer proofs could not both be green in one invocation. |
| D. `#[cfg]` out `BufferedWriter` / `cache::pending` | Rejected | Violates "stays in the tree, compiling"; `upload_pending` is called on every refresh pass (`cache/refresh.rs:300`) and would need gating too. |

Consequences of A:

1. **The upload side stays live.** `refresh.rs:300` still calls `upload_pending` on every pass and
   `CacheStore::open` still seals orphans, so a box that buffered a chat under the previous build
   still lands it on its next connection. Only the *write* side is disabled — which is what the item
   asks.
2. The two registry `ok_or_else` closures (`agent_worker.rs:736`, `:1742`) currently say *"this
   backend hands out no writer"* and are shadowed by the `Buffered` guard that follows; under A they
   fire first. Making them say `REGISTRY_ON_SERVER_ONLY` — true, and the sentence those paths already
   answered offline — keeps the three offline probe/install/login tests green and untouched. Only the
   chat's closure (`:1163`) gets the new sentence.
3. `Writer::label() == "buffered"` (`writer.rs:62`), `BUFFERED_LABEL`/`BUFFERED_NOTE` and the D42
   header branch (`chat/mod.rs:57-68`, `:304-305`) stay as compiled-but-unreachable machinery.

**The refusal sentence.** One new `pub const` in `crates/htui-store/src/writer.rs` beside
`REGISTRY_ON_SERVER_ONLY` (`:400`) and `PROMPT_ON_SERVER_ONLY` (`:418`), re-exported at `lib.rs:30`.
Name `DATABASE_UNREACHABLE`; text, in the repo's lowercase-no-period style:

```
"the database is unreachable: this box browses its read-only cache and starts no run"
```

It echoes `R-STO-4`'s "No item creation, no runs". On screen: status line
`chat_start: the database is unreachable: ...`, and the same sentence in the Chat body, under a top
bar reading `offline · <age>`.

**Test invocation.** Dev Postgres from `compose.yaml` on port 5439. Full run:

```
USERNAME=htui-ci HTUI_TEST_DATABASE_URL=postgres://postgres:htui@localhost:5439/postgres \
  cargo test --workspace --all-features
```

Lint gate is `cargo clippy --workspace --all-features --all-targets -- -D warnings`
(`README.md:456`) — **warnings fail**, so an import left unused by Task 1 breaks the build. Toolchain
is pinned at 1.98.1 (`rust-toolchain.toml`, `cargo 1.98.1` probed). Convention is TDD: failing test
first. `#[ignore = "reason"]` is the established way to park a test.

## Tasks

### 1. The seam: `Backend::writer()` answers `None` offline (htui-store, TDD)

**Files touched:** `crates/htui-store/src/backend.rs`, `crates/htui-store/src/writer.rs`,
`crates/htui-store/src/lib.rs`, `crates/htui-store/src/cache/pending.rs`,
`crates/htui-store/tests/writer_buffered.rs`.
**Independence:** root task. Tasks 2 and 3 consume the constant it adds, so they run after it. Its
file set is disjoint from every other task's.

1. **Failing tests first.**
   - `writer.rs:690` `an_offline_backend_hands_out_a_buffered_writer_and_no_user` → rename
     `an_offline_backend_hands_out_no_writer_and_no_user`; replace the
     `.expect(...).label() == "buffered"` assertion with `assert!(backend.writer().is_none(), ...)`;
     keep the `!is_writable()` and `this_user` `NotFound` assertions (both still true and
     load-bearing — `backend.rs:98-111`: the re-dial ticker keys on `is_writable`).
   - `tests/writer_buffered.rs:495-524` `an_offline_backend_hands_out_a_buffered_writer` → rename
     `an_offline_backend_hands_out_no_writer`; assert `writer().is_none()`, keep `!is_writable()` and
     `writable().is_none()`. Update its doc to say the seam T21 walked through is closed by MOD-25.
   - `cargo test -p htui-store --all-features writer` → both fail.
2. **The change.** `backend.rs:147-149`: `Self::Offline { .. } => None`. `BufferedWriter` then has no
   other use in that file (`:35` import, `:136` doc link, `:148` the arm) — **drop it from the `use`
   at `:35`** or `-D warnings` fails; the type stays `pub` via `writer.rs` and `lib.rs:30`, and the
   `:136` doc link is `[`crate::BufferedWriter`]`, which does not need the import.
3. **The constant.** In `writer.rs`, after `PROMPT_ON_SERVER_ONLY` (`:418`), add
   `pub const DATABASE_UNREACHABLE: &str` with a doc comment naming MOD-25 and `R-STO-4` and saying
   it is the one sentence a chat is refused with off the server. Add it to the
   `pub use writer::{...}` at `lib.rs:30`.
4. **Docs that would otherwise lie** (comments here are load-bearing): `backend.rs:11-19` (module
   doc: "there **is** an offline write path"), `:54-55` (`Offline` variant doc), `:125-141`
   (`writer()` doc — `Offline` answers `None` since MOD-25; the `Buffered` arm and `BufferedWriter`
   are kept one release for the reversal; the CLEAN item removes them); `writer.rs:12-20` (module
   doc) and `:50-51` (`Buffered` variant doc: "kept, not constructed by any backend since MOD-25");
   `cache/pending.rs:1-8` (one sentence: since MOD-25 no writer appends here; `upload_pending` still
   runs so buffers from earlier builds land). Add the MOD-25 sentence; leave the D34/D35 references
   as history.
5. **Untouched, by design:** the nine `BufferedWriter::new` cases in `tests/writer_buffered.rs`;
   every case in `tests/cache.rs` and `tests/pg_criteria.rs` (they import `append_pending` /
   `upload_pending` directly and never call `.writer()`); `refresh.rs:300`; `identity.rs:161`.
6. **Validate:** `cargo test -p htui-store --all-features` (Postgres env for `cache.rs` /
   `pg_criteria.rs`); `cargo clippy -p htui-store --all-features --all-targets -- -D warnings`;
   `cargo doc -p htui-store --no-deps` (intra-doc links are `deny`).

### 2. The runtime: the chat refuses with the sentence (htui `agent_worker`, TDD)

**Files touched:** `crates/htui/src/agent_worker.rs`.
**Independence:** depends on Task 1 for the constant. File set disjoint from all other tasks.

1. **Failing test first.** `agent_worker.rs:3896`
   `an_offline_backend_with_an_empty_mirror_refuses_and_names_what_is_missing` changes meaning →
   rename `an_offline_backend_refuses_a_chat_with_the_unreachable_warning`. Replace
   `assert!(backend.writer().is_some(), ...)` (`:3906`) with `is_none()`; keep the
   `Served::Reply(StoreReply::Failed { request, message })` match; assert `request == "chat_start"`
   and `message == htui_store::DATABASE_UNREACHABLE` (equality, not `contains` — the sentence is the
   contract); add `assert!(runtime.steps().is_empty())` (`steps()` at `:438`). Rewrite its doc.
2. **The change.** `:1159-1166`: replace the milestone-4 comment and make the `ok_or_else` produce
   `StoreError::Unreachable(htui_store::DATABASE_UNREACHABLE.to_owned())`. `:736-737` and
   `:1742-1743`: make those two closures produce `REGISTRY_ON_SERVER_ONLY` (the sentence the
   following `matches!` guard already used), with a one-line comment that since MOD-25 an offline
   backend answers `None` here and the `Buffered` guard below is kept for the reversal. Leave the
   guards at `:742-746`, `:1290-1291`, `:1410`, `:1447`, `:1744-1748` untouched.
3. **Park the one test that can no longer be driven.** `:4305` `a_buffered_writer_never_re_probes`
   serves a chat over `Backend::Offline` and expects `Served::Start`; under Task 1 it is refused. Add
   `#[ignore = "MOD-25: no backend hands out Writer::Buffered; kept for the reversal, removed by the CLEAN item"]`.
   Body stays byte-for-byte.
4. **Untouched:** `:3715` `an_unmirrored_project_is_unbounded_offline_and_refused_online` and `:3771`
   `a_buffered_writer_gets_no_latch` (they build `Writer::Buffered(BufferedWriter::new(..))` directly
   and test pure functions — they still prove the kept machinery); the three offline
   probe/install/login tests (green via step 2); the preview refusal at `:801-806` (matches
   `Backend::Offline` directly, plan D109, unrelated).
5. **Validate:** `cargo test -p htui --features testkit agent_worker`;
   `cargo clippy -p htui --all-features --all-targets -- -D warnings`.

### 3. The shell proof: `chat_offline.rs` proves the refusal (TDD)

**Files touched:** `crates/htui/tests/chat_offline.rs`, `crates/htui/src/ui/tabs/chat/mod.rs`.
**Independence:** depends on Tasks 1 and 2 to go green. File set disjoint from all other tasks.

1. **Failing test first — the new proof.** Add `an_offline_chat_is_refused_with_the_unreachable_warning`,
   shaped like `:378-410`: `offline_mirror(agent_id)` (`:97`) + `offline_harness(&cache, one_turn())`
   (`:122`) + `compose` + `drive`. Assert `rendered.contains(htui_store::DATABASE_UNREACHABLE)` (the
   Chat body, `chat/mod.rs:571-581`); `harness.chat_steps().is_empty()` (`testkit.rs:145`); and that
   `cache.dir().join("pending")` holds no entry — neither an `.open` file nor a sealed one (the
   negative of `sealed_buffer`, `:181`). `read_dir` is safe there: `CacheStore::open` creates
   `pending/` eagerly (`cache/mod.rs:101-102`, `:130`). Plain asserts, no snapshot — they add nothing
   for the CLEAN item to delete.
2. **Park the four cases that prove the withdrawn mode.** Add
   `#[ignore = "MOD-25: the offline buffer is disabled (Backend::writer answers None offline); kept for the reversal, removed by the CLEAN item"]`
   to `:223` `an_offline_chat_is_accepted_and_its_header_says_it_is_buffered`; `:250`
   `an_offline_chat_writes_its_rows_to_the_pending_buffer_in_seq_order`; `:299`
   `a_buffered_chat_lands_in_postgres_on_the_next_connection` (the criterion-12 end-to-end proof, the
   one `HTUI_TEST_DATABASE_URL`-gated case in this file); `:379`
   `an_offline_box_that_never_synced_this_user_refuses_and_says_so` (D33's `app_user` refusal is now
   unreachable from the shell — `start()` refuses at the writer, `agent_worker.rs:1163`, before
   `this_user()`). Bodies stay byte-for-byte; the helpers `sealed_buffer`, `stable`, `scripted_row`
   stay referenced by the ignored cases, so no `dead_code` warning appears.
3. **Keep as-is:** `:415` `an_online_chat_header_says_nothing_about_a_buffer` (memory backend; still
   the correct negative).
4. **Module doc** `:1-12`: restate what the file proves now (the refusal) and what it keeps ignored
   (the withdrawn criterion-12 proof), naming MOD-25 and `docs/decisions/mod/mod-2.md` "Known,
   accepted, and handed on".
5. **Snapshot** `crates/htui/tests/snapshots/chat_offline__chat_buffered.snap` stays: referenced by an
   ignored test, and insta is never run with `--unreferenced` here, so it is inert. It is on the
   CLEAN item's list.
6. **`chat/mod.rs:16-19`** module doc: one sentence that since MOD-25 no backend hands out the
   buffered writer, so the D42 header branch (`:57-68`, `:304-305`) is kept for the reversal and never
   taken. No code change in this file.
7. **Validate:** `cargo test -p htui --features testkit --test chat_offline`, then
   `cargo test -p htui --features testkit --test chat_offline -- --ignored` once, to confirm the
   parked cases still compile and run (they are expected to fail — that is what "disabled" means).

### 4. Prose: README and the `R-STO-6` residue

**Files touched:** `README.md`, `docs/REQUIREMENTS.md`.
**Independence:** fully independent of Tasks 1-3 (prose only); may run in parallel with Task 1.

1. **README.md** — three places describe the buffer as live behaviour: `:111`
   (`pending\*.jsonl  # offline chat buffers, uploaded on connect`), `:244-246` ("A chat can be
   started while the shell is offline. Its events go to a JSON-lines buffer ... the header says
   `buffered · uploads when the store returns`"), `:298-299` ("An offline chat latches nothing ... its
   usage rows are buffered"). Replace with the online-only statement: with Postgres unreachable the
   shell opens read-only from the cache, the top bar reads `offline · <age>`, and starting a chat is
   refused with the unreachable-database sentence — noting that `pending/*.jsonl` left by an earlier
   build is still uploaded on the next connection.
2. **`docs/REQUIREMENTS.md:136-140`** — the only edit in this maintainer-owned file, keeping the ID
   and `(must)`:

```diff
 - **R-STO-6 (must).** Startup with a warm cache and reachable Postgres is under one second on the
-  reference workstation. Cache refresh runs in the background and never blocks input. Unchanged by
-  ANA-10, and a decision is owed: as written the budget is conditioned on a reachable server and
-  binds no local-only start (R-STO-7). `docs/ANA-10.md` §10.4 asks whether to extend it; §12
-  criterion 13 takes the measurement either way.
+  reference workstation. Cache refresh runs in the background and never blocks input. The budget is
+  conditioned on a reachable server and binds no offline start (R-STO-4); that scope was decided,
+  not overlooked (maintainer, 2026-09-08).
```

   Rationale: the maintainer's 2026-09-08 answer to ANA-10 §10 row 4 was **"(a) with the measurement
   kept"**, leaving `R-STO-6` unamended *with a sentence recording that a decision was taken rather
   than overlooked* (`docs/ANA-10.md:2556`). The measurement it kept (§12 criterion 13) was MOD-17's
   to take, and MOD-17 is withdrawn with ANA-10, so that pointer now dangles. The replacement keeps
   the decided-not-overlooked intent while citing a live requirement instead of a withdrawn one and a
   not-taken analysis.

## Verified Claims

Checked on the working tree on 2026-09-16 by the router, independently of the drafting agent. `≈`
marks a claim true with a small line-number drift from the draft.

| Claim | Verdict | Evidence |
|---|---|---|
| Scope point (1) landed: amendment recorded, `R-STO-7` withdrawn, `R-STO-4` online-only | Confirmed | `docs/REQUIREMENTS.md:11-12`, `:130-132`, `:141` |
| `R-STO-6` still cites `R-STO-7`, ANA-10 §10.4 and §12 criterion 13 | Confirmed | `docs/REQUIREMENTS.md:136-140` |
| Maintainer's ANA-10 §10 row 4 answer was (a), measurement kept, `R-STO-6` left unamended with a decided-not-overlooked sentence | Confirmed | `docs/ANA-10.md:2556` ("**ANSWERED (a) with the measurement kept, by the maintainer, 2026-09-08**") |
| The masked DSN field has no scaffolding in `crates/` | Confirmed | `grep -rn 'SectionId("connection")\|MaskedField\|FocusSection' crates/` → 0 hits |
| `Backend::writer()` builds `Writer::Buffered` on `Offline`, and its doc reserves a `None` arm | Confirmed | `crates/htui-store/src/backend.rs:143-151`, `:139-141` |
| Exactly three production callers of `Backend::writer()` | Confirmed | `grep -rn "\.writer()" crates/` → `agent_worker.rs:736,1163,1742`; tests at `writer.rs:650,701`, `writer_buffered.rs:504`, `agent_worker.rs:3906` |
| Probe and install/login already refuse `Buffered` with `REGISTRY_ON_SERVER_ONLY`; both `ok_or_else` closures currently say "this backend hands out no writer" | Confirmed | `agent_worker.rs:736-746`, `:1742-1748` |
| `StoreError::Unreachable(String)` exists and is the right variant | Confirmed | `crates/htui-store/src/error.rs:9`, `:29`, `:132` |
| `REGISTRY_ON_SERVER_ONLY` / `PROMPT_ON_SERVER_ONLY` sit at `writer.rs:400` / `:418` and are re-exported | Confirmed | `writer.rs:400`, `:418`; `lib.rs:30` |
| A `Failed` reply lands on the status line as `{request}: {message}`; request name is `chat_start`; the Chat body renders a refusal in `theme.error` when no session is open | Confirmed | `app/update.rs:132-133`; `store_worker.rs:244`; `ui/tabs/chat/mod.rs:571-581` |
| `chat_offline.rs` has five tests: three prove the buffer, one proves D33, one is the online negative | Confirmed | `:223`, `:250`, `:299`, `:379`, `:415`; `:299` is the only `demo_db()`-gated case |
| Harness helpers exist: `chat_steps()`, `cache.dir()`, `sealed_buffer` | Confirmed | `testkit.rs:145`; `chat_offline.rs:182`, `:181` |
| `CacheStore::open` creates `pending/` eagerly, so the new test's `read_dir` cannot error | Confirmed | `cache/mod.rs:101-102`, `:130` |
| `append_pending` also creates it on demand | Confirmed | `cache/pending.rs:119` |
| `agent_worker` test anchors exist | Confirmed ≈ | `:3896` (draft said 3891), `:4305` (draft 4301), `:3715` (draft 3714), `:3771`; `steps()` at `:438` |
| Test suites that build `BufferedWriter` directly never call `.writer()`, so they stay green | Confirmed | `.writer()` grep above has no hit in `tests/cache.rs` or `tests/pg_criteria.rs` |
| `upload_pending` runs on every refresh pass, so the upload side stays live | Confirmed | `cache/refresh.rs:299-300` |
| `htui-agent/src/record.rs` and `htui-core/src/model/usage.rs` "Buffered" hits are unrelated | Confirmed | `record.rs:341-343` is a private `enum RawTarget { Buffered(usize) }`; the rest are doc comments; neither file references `htui_store` |
| The documented test command uses `--all-features`, so a feature gate (candidate C) would flip under it | Confirmed | `README.md:468` |
| Lint gate denies warnings, so Task 1 must drop the now-unused `BufferedWriter` import | Confirmed | `README.md:456` (`-D warnings`); `backend.rs:35` is the only non-doc use besides the arm |
| Workspace lints do not deny dead code; rustdoc intra-doc links are `deny` | Confirmed | `Cargo.toml [workspace.lints.*]`: rust `unsafe_code=forbid`, `missing_debug_implementations=warn`, `unused_qualifications=warn`; rustdoc `broken_intra_doc_links=deny`; clippy `all=warn`. No `deny(warnings)` attribute anywhere in `crates/` |
| `#[ignore = "..."]` is an established repo convention | Confirmed | `htui-store/src/secret.rs`, `htui-agent/tests/*_live.rs` (30+ occurrences) |
| Toolchain pin | Probed | `rust-toolchain.toml` `channel = "1.98.1"`; `cargo 1.98.1 (797e8a9bc 2026-08-05)` |
| README describes the buffer as live behaviour in three places | Confirmed | `README.md:111`, `:244-246`, `:298-299` |
| Dev Postgres reachable for the Postgres-gated suites | Confirmed | `htui-postgres` container healthy, `0.0.0.0:5439->5432` |
| Workflow-docs validator green at baseline | Confirmed | 0 errors, 1 warning (`docs/decisions/mod/mod-25.md` not found — this item creates it at close-out) |
| Task file sets are pairwise disjoint | Confirmed | Task 1 `{backend.rs, writer.rs, lib.rs, cache/pending.rs, tests/writer_buffered.rs}`, Task 2 `{agent_worker.rs}`, Task 3 `{chat_offline.rs, chat/mod.rs}`, Task 4 `{README.md, REQUIREMENTS.md}` — empty pairwise intersections |
| Tasks are nevertheless **not** all parallel | Correction | Disjoint files but a real dependency chain: 2 and 3 consume Task 1's constant, 3 needs 2 to go green. Only **Task 4 ‖ Task 1** is safe to fan out; 1 → 2 → 3 runs serial |

## File modifications

- `crates/htui-store/src/backend.rs` — Task 1: `Offline` arm of `writer()` → `None`; docs `:11-19`,
  `:54-55`, `:125-141`; drop `BufferedWriter` from the `use` at `:35`.
- `crates/htui-store/src/writer.rs` — Task 1: new `pub const DATABASE_UNREACHABLE`; docs `:12-20`,
  `:50-51`; unit test `:690` flipped.
- `crates/htui-store/src/lib.rs` — Task 1: re-export the constant at `:30`.
- `crates/htui-store/src/cache/pending.rs` — Task 1: one-sentence module-doc note (`:1-8`).
- `crates/htui-store/tests/writer_buffered.rs` — Task 1: `:495-524` flipped to assert `None`.
- `crates/htui/src/agent_worker.rs` — Task 2: `:736-737`, `:1159-1166`, `:1742-1743` sentences; test
  `:3896` flipped; `#[ignore]` on `:4305`.
- `crates/htui/tests/chat_offline.rs` — Task 3: new refusal test; `#[ignore]` on `:223`, `:250`,
  `:299`, `:379`; module doc `:1-12`.
- `crates/htui/src/ui/tabs/chat/mod.rs` — Task 3: module-doc sentence at `:16-19` only.
- `README.md` — Task 4: `:111`, `:244-246`, `:298-299`.
- `docs/REQUIREMENTS.md` — Task 4: `:136-140` only (the diff above).

**Explicitly out of scope** (close-out or deferred): minting `CLEAN-N` (skill ID mint at close-out —
its deletion list is the `Buffered` arm and `BufferedWriter` in `writer.rs`, `cache::pending`'s
append/seal side, the `matches!(Writer::Buffered)` guards and the `project_caps_for` /
`quota_latch_for` arms in `agent_worker.rs`, `BUFFERED_LABEL` / `BUFFERED_NOTE` /
`ChatSessionState::buffered` in `chat/mod.rs`, the four ignored `chat_offline.rs` cases and
`chat_offline__chat_buffered.snap`, the ignored `a_buffered_writer_never_re_probes`,
`tests/writer_buffered.rs`, and the pending cases in `tests/cache.rs` / `tests/pg_criteria.rs`);
deleting the MOD-17/18/19 checklist lines and any other `HANDOFF.md` edit; writing
`docs/decisions/mod/mod-25.md`; the masked DSN field (deferred — the close-out records it as owed to
MOD-15's `SectionId("connection")` section); `docs/ANA-10.md` (stays as the analysis not taken); the
top bar's `offline` wording and `shell__offline_label.snap`; the `--offline` flag; the upload side
(`upload_pending`, `seal_orphaned`), which stays live on purpose.
