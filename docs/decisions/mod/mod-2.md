# MOD-2 - Agent driver + chat tab (done, 2026-09-15)

**Requirements:** `R-AGT-1..8`, `R-PRM-1..4`, `R-TUI-3`, `R-TUI-6`, `R-TUI-8`, `R-HIS-1..2`,
`R-SEC-3`, `R-SKL-2`, `R-ID-4..6`, `R-NF-3..4`.
**Design authority:** `docs/ANA-4.md` (the driver, `docs/decisions/ana/ana-4.md`) and
`docs/ANA-5.md` (the prompt contract, `docs/decisions/ana/ana-5.md`). Neither was edited by this
item; every amendment either analysis needed is recorded below, maintainer-only, per the
milestone-5 precedent.
**Artifacts:** PRD `.claude/prds/mod-2-agent-driver-chat.prd.md` (nine milestones) and nine plans
under `.claude/plans/mod-2-*.plan.md`, each with its `code-architect` blueprint beside it.
**Commits:** nine milestones, `5c717d0`..`32e516d` plus this close-out. Per-milestone ranges are
named in each phase below.

## What shipped

`htui` stopped being a backlog viewer with a database behind it. It starts agent sessions, streams
typed events into a chat tab, answers permissions inline, persists and replays every event, tracks
quota and cancels on a cap — over **three transports** that pass **one** conformance list — and it
assembles the single self-contained prompt a step will be given, with a read-only preview that
renders it from the running binary.

The load-bearing proof obligation was `R-AGT-5`, and it is a passing test rather than a claim: an
agent the codebase has never heard of reaches a working session from a registry row alone
(`crates/htui-agent/tests/extensibility.rs`).

---

## The nine phases

### 1 — Driver seam + conformance (`5c717d0`..`1e7de4d`, 2026-09-07)

New crate `htui-agent` carrying ANA-4 §4.1's `AgentDriver`/`AgentSession` seam, an 11-variant
`DriverEvent`, the recorder, `FakeDriver`, and a transport-neutral conformance suite. `htui-core`
gained `scrub` (`Scrubber` plus the fail-closed `MinimalScrubber` that MOD-10 replaces behind an
unchanged trait), six `WriteStore` methods including `start_chat_run`/`finish_chat_run`, inherent
`agents()` over three `Backend` arms, and `append_pending`. `conformance::CASES` 15 → 20.
`rust-reviewer` adjudicated 23 findings through an adversarial pass (12 real, 5 partly, 6 refuted);
10 were fixed with bite-proven tests and 7 deferred into the plan's amendments.

### 2 — Registry, launch and the extensibility proof (`5c717d0`..`1e7de4d`, 2026-09-07)

Workspace MSRV 1.85 → **1.98** (a maintainer override of ANA-4 §4.2's 1.88: `sqlx-core 0.9.0`
already floors at 1.94, so 1.88 would have been a second unbuildable declaration replacing the
first). `agent-client-protocol` pinned `=2.1.0`. `htui-agent::launch` resolves `${tool}`
placeholders and spawns supervised — a job object on Windows, a process group on unix.
`DriverFactory` is keyed by transport, and `R-AGT-5` becomes `extensibility.rs`. The Settings tab
gained `SettingsRegistry` and its agent section. Nine findings, all fixed; the review gate blocked
on one HIGH (`spawn` resolved its command with a blocking `which` inside async code), fixed in
`34d5f04` with two more, the fourth accepted with its reason.

### 3 — Live `claude` over ACP (`a142fbf`..`682a423`, 2026-09-07)

The first real transport (`acp/{mod,map,client,fs}.rs`): one task per session owning the whole
`connect_with` future, the §6.1 mapper written over raw JSON because the adapter ships five update
kinds the schema does not know, `fs/write_text_file` intercepted into an `edit_proposal` with a
`similar` unified diff, model selection by config-option **id**, the session banner, and the
three-stage permission pipeline. The chat tab streams it.

### 4 — Durable history and replay (`81d247b`, 2026-09-08)

Events persist scrubbed; an offline chat buffers to `<cache_dir>/pending/<project>.<run>.jsonl`
and uploads idempotently; any recorded step reopens read-only and replays. Replay **decodes**
rather than re-renders — `htui_agent::replay` inverts the recorder's encode — and the Chat tab's
replay mode is read-only *structurally*: `on_key_replay` takes no `Ctx`, so it cannot request. The
mirror gained the `agent` table (`cache_migrations/0002_agent_mirror.sql`), **which is why MOD-4's
cache migration is `0003_orchestration.sql` and not the `0002` ANA-2 §9 reserved**. The summing
rule moved to `htui_core::model::UsageTotals` so the recorder and the uploader cannot drift.

### 5 — Autodiscovery and the box probe (`fb626a8`, 2026-09-08)

`migrations/0002_agent_probe.sql` landed — ANA-4 §9's `agent_box.probe JSONB` plus **all** of
ANA-5 §9 (five `COMMENT ON COLUMN`, ten `app_setting` defaults) — **so MOD-4's `0003` is no longer
held**. Tier 1 is `probe.rs` (one shared resolver, a hand-rolled `*`-per-segment glob walker, no
new crate); tier 2 is `acp/handshake.rs` (`initialize` and nothing else, the child owned by a
`ChildGuard` so timeout, actor failure, garbage, success and a dropped future all kill it).
`Settings > r` with its `probing…` column, and a 24 h `PROBE_TTL` re-probe at `ChatStart` that
cannot block or fail the chat. The review gate **blocked** on two HIGH findings in the
orphan-process class the milestone was gated on; both closed by one bounded-spawn path
(`run_bounded`) and the lifted `ChildGuard`. Also `crates/htui-store/build.rs`, because
`sqlx::migrate!` registers rerun-if-changed **per file** and adding `0002` did not invalidate a
warm `target/` — the same trap awaits MOD-4's `0003`.

### 6 — `agy` over ACP (`acf16f7`, 2026-09-09; `T34` closed inside milestone 7)

A second agent through the same code paths, differing only by registry row and capability banner.
**D58** makes the driver spawn `agent_box.probe.resolved` rather than resolving a second time,
which is the only path carrying a glob tool's per-platform `args` — and `--uid=` is **mandatory**,
without it the server aborts in `ChangeRootAndUser` before reading stdin. **D59** gives
`unauthenticated` a declarative `discovery.credential { env, files }` block resolved by the probe;
nothing is keyed on an agent name (`R-AGT-5`). **D60**: a chat that fails to *spawn* re-probes its
row inline under a runtime-wide claim, and there is **no transport fallback** — by maintainer
decision a CLI agent is its own registry row. **D61** stops `open_session`'s handshake timeout
orphaning the adapter. Beyond the plan, the gate found a message the SDK swallows: `start_session`
sends from an actor whose failure **drops the foreground future**, so the vendor's
`Authentication required` — the first thing an `agy` user on a fresh box meets — was being
replaced by "the session task ended before the handshake".

### 7 — Quota and caps (`142beb1`..`acec1b8`, 2026-09-10)

`htui_core::model::quota` holds ANA-4 §7's document (`normalize`, `Quota`, `ProjectCaps`, and
`available` — `R-AGT-8`'s skip predicate, which **MOD-4 consumes and MOD-2 only tests**). The ACP
mapper lifts `_meta["_claude/rateLimit"]` verbatim; the recorder **latches** the document after
every `usage` row; the **per-run cap** is detected in the recorder and cancelled by the loop that
holds the session, leaving `error{cap_exceeded}` then `done{stop_reason:cancelled}` as a step's
last two rows. Caps are **USD micros** in `project.settings` (`per_token_cap_run` enforced,
`per_token_cap_batch` read and logged — **MOD-12 enforces it**); absent means unbounded, `0`
cancels on the first costed row, a malformed value refuses the chat. Milestone 6's `T34` closed
here with all three remaining ANA-4 §11.14 `agy` items answered live.

`T46` also landed here and was not in scope: the review gate's live evidence showed ANA-4 §11
criterion 6's `edit_proposal` dedup holding only *inside* one flush window, so one `agy` file write
left **three** rows. **D77** fixes it with no new store seam — an `edit_proposal` reserves its
`seq` at announcement and only its *write* waits — measured 3 → 1 on the committed fixture. The
lasting fix is the amended conformance case: its script now flushes between two *differing* writes,
because the fake never flushed mid-tool-call and that is precisely why a transport-neutral suite
could not see a live defect.

### 8 — The degraded CLI transport (`a3cbfff`..`42294d8`, 2026-09-10..11)

An agent that speaks only its own headless JSON stream reaches the same chat tab, recorder, store
rows and replay as an ACP one — losing exactly three event kinds and saying so on screen.
`R-AGT-3` is met. `CASES` is still **15** and **three** transports report all fifteen, because
D80/D91 gave six cases a *capability-gated second arm* rather than a skip: a transport without a
capability must prove the **negative**. That change immediately caught a real defect —
`extensibility.rs` was replaying a script the fake's own declared capabilities said it could not
have produced.

**Nine live probes ran before a line of `src/cli/` was written** (fourteen committed transcripts,
findings F-1..F-15). Five changed code about to ship and three would have shipped silently: **F-4**
— `modelUsage[*]` tokens are **cumulative** while `result.usage` is per-turn, the inversion of what
the names suggest, so a two-turn chat would have double-counted into `run_step.usage` with **no
test in the tree failing**; **F-7** — a turn's thinking block and its reply share one `message.id`,
so the coalescing key is `(message.id, block index)`; **F-13** — `quota::normalize` **discarded**
the blob for `CliRateLimitEvent`, making D86 a no-op that every ACP-sourced quota test would have
missed.

Two holes in the fixtures' own redaction were found and closed, each by a *later* probe reading an
*earlier* one's committed transcript: a needle **split across streaming deltas** (and the
self-check re-read the same unreassembled text, so a green redaction check proved nothing) and a
needle **slugged** by the CLI's cwd-derived project directory. What leaked was a tempdir name and a
username already in every commit, so the exposure is nil; the hole is the same size for the
`SECRET_NAMES` half, which exists so a token echoed by a hook cannot ride into git.

### 9 — Prompt assembler + preview (`b8c62aa`..`32e516d`, 2026-09-11..15)

The whole of `htui_core::prompt` — which did not exist in any form at `a601931` — plus
`htui-agent::excerpt`, five store-seam methods, a sixth Backlog detail sub-tab and two `Runs`
indicators. Twelve tasks (T59–T70) over the plan
`.claude/plans/mod-2-prompt-assembler.plan.md` and its blueprint.

- **The template contract** (`template.rs`): `{{name}}` with **no** whitespace tolerance inside the
  braces, so one body has one spelling and the digest cannot drift; four checks in ANA-5's order
  (syntax, closed set, role, required), each refusing with a **byte offset** so MOD-9's editor can
  put the cursor on the mistake. `{{{{` is the only escape.
- **The estimator** (`estimate.rs`): `chars-v2`, characters not bytes, two whole-line toggles (a
  fence, and a `<file ` block) picking the rate — see D108 below, and F-16, which is the number
  this milestone exists to have measured.
- **The ten §5.4 default bodies** as `prompt::defaults::DEFAULT_TEMPLATES` (D104), parsed by the
  validator and pinned by golden prompts. None of the ten needed a character changed (F-27).
- **Render, digest, trim and `assemble()`**: §4.7's pipeline — parse, render in canonical order,
  scrub, estimate, refuse, trim, substitute, canonicalise, digest, record. `assemble()` is pure by
  test, not by assertion: the suite greps its nine source files for a clock, an environment read
  and an unordered map, and it re-sorts the upstream walk, the skill bindings and the judge's
  candidates rather than trusting its caller.
- **Excerpts**: the `ExcerptProvider`/`RepoReader` seams and the five-tier ranker in `htui-core`;
  `FsRepoReader` and the deadline runner in `htui-agent`. Every failure is a note and never an
  error (§4.5 step 1's fail-open): an unresolved root, an unlistable repo, a declared-but-denied
  file, an unreadable candidate and an exhausted budget each leave a valid prompt and a record that
  says why.
- **The store seam**: `ReadStore` gains `document`, `documents_of_kinds`, `upstream_summaries` and
  `project`; `WriteStore` gains `set_step_prompt`. `CASES` 22 → **23**, and D96's `READ_CASES`
  (6 cases) lands beside it so `CacheStore` — a `ReadStore` only — is bound to the same spec.
- **The preview** (D102, D103): a sixth Backlog detail sub-tab, `Prompt`, answered by
  `StoreRequest::PromptPreview` served as deferred work on an owned `Backend` clone — never in the
  worker's `select!` arm, never on the UI task (`R-NF-3`). It calls the **same** `assemble()` MOD-4
  will call with the same `PromptSpec` type; what differs is who fills three fields, and every one
  of those stand-ins is recorded verbatim in `trim_record.notes`. **It writes nothing.**
- **The Runs indicators** (D106): `RunStepSummary` gains `prompt_tokens` and `trimmed`, derived
  from `trim_record` by each of three backends' projections, rendered as `~36k` and `!` on a second
  line inside the existing phase cell.

The review gate over the whole change set returned **1 CRITICAL, 7 HIGH and 11 MEDIUM, all
applied**. They are listed under "Review gate" below, because three of them are the sharpest facts
this milestone produced.

---

## ANA-5 §12 criteria 1–20

Every name below resolves in the tree. `--` names a `conformance::CASES`/`READ_CASES` case, which
is executed by the binding tests named in the same row; everything else is a test function, cited
as `<file or module>::<name>`.

| §12 | Proved by |
|---|---|
| 1 | `htui-core` `prompt::defaults::tests::every_default_body_parses_in_its_role`; `prompt::defaults::tests::every_phase_body_is_wrong_role_for_judge_and_handoff_and_vice_versa` |
| 2 | `htui-core` `prompt::template::tests::{spaces_inside_braces_are_unknown, a_typo_is_unknown_with_its_offset, uppercase_is_unknown, a_bare_open_is_unterminated, quadruple_brace_is_a_literal_double_brace}` |
| 3 | `prompt_digest.rs::an_unknown_placeholder_refuses_with_the_stage_3_text` — the `blocked` **transition** is MOD-4's, by name |
| 4 | `prompt_golden.rs::the_all_empty_prompt_omits_every_optional_section` (snapshot `prompt_all_empty`); from the running binary, `htui/tests/prompt_preview.rs::an_item_with_nothing_attached_still_previews` |
| 5 | `prompt_digest.rs::fan_out_siblings_get_identical_bytes`; the grep half is `prompt_render.rs::no_rendered_byte_carries_a_path_a_clock_or_a_carriage_return` and `htui-core` `prompt::render::tests::an_edit_proposal_path_is_repo_qualified_and_never_absolute` |
| 6 | `prompt_digest.rs::crlf_inputs_digest_identically`; `prompt_render.rs::crlf_inputs_render_identical_bytes` |
| 7 | case `upstream_diamond_dedup`, run over three backends by `mem_store.rs::mem_store_read_conformance`, `pg_conformance.rs::pg_store_read_conformance` and `htui-store/tests/cache.rs::the_mirror_passes_the_read_cases`. The honest reading is F-32's: the two walks agree **transitively through the shared spec**, not in one assertion holding a `PgStore` and a `CacheStore` side by side |
| 8 | `prompt_digest.rs::payload_sections_are_the_records_projection`, and `prompt_digest.rs::the_only_section_entry_constructor_is_the_records_projection`, which greps the tree so `TrimRecord::section_entries` stays the only builder (ANA-5 risk 12 as a test rather than a convention) |
| 9 | `prompt_digest.rs::the_oversize_fixture_lands_at_or_under_target`; `prompt_digest.rs::every_marker_matches_its_record` |
| 10 | `prompt_digest.rs::a_protected_set_over_target_refuses_before_anything`; `prompt_digest.rs::skills_over_the_cap_refuse` |
| 11 | case `set_step_prompt_writes_digest_and_trim` over `MemStore` and `PgStore`; the "and only them" half is `pg_criteria.rs::set_step_prompt_writes_only_the_digest_and_the_record` (a whole-row `jsonb` diff either side of the write, so a column added later is covered without anyone remembering) and `htui-core` `store::mem::tests::set_step_prompt_writes_both_columns`. The two-writer identity is `htui-core` `prompt::digest::tests::sha256_matches_the_recorder_call` |
| 12 | `htui-core` `prompt::excerpt::tests::no_roots_means_no_section_and_a_no_path_root_per_repo`; from the running binary, `htui/tests/prompt_preview.rs::the_prompt_sub_tab_previews_feat_1` |
| 13 | `htui-agent/tests/excerpt.rs::every_provider_failure_leaves_a_valid_prompt` |
| 14 | `htui-agent/tests/excerpt.rs::a_denied_file_under_a_touched_prefix_is_never_selected_and_is_noted` |
| 15 | `prompt_digest.rs::skill_order_is_unchanged_by_store_order`; `htui-core` `model::skill::tests::{phase_overrides_project_once, equal_positions_break_on_name_bytes, pinned_version_wins_over_latest}` |
| 16 | `htui-core` `prompt::defaults::tests::the_review_body_states_the_front_matter_verbatim` — the **parser** half is MOD-4's, by name |
| 17 | `prompt_golden.rs::the_judge_prompt_renders_its_golden_bytes` (snapshot `prompt_judge_three_candidates`); `prompt_digest.rs::the_second_judge_call_differs_only_in_candidate_order`; `prompt_digest.rs::a_candidate_over_its_share_renders_stat_only` |
| 18 | `prompt_digest.rs::a_handoff_summary_carries_only_the_windowed_tail`. The `follow_up` **persistence** half is **re-deferred to MOD-4 by name** (D107): it requires a promotion, which is `R-ORCH-5` |
| 19 | `mem_store.rs::mem_store_conformance` and `pg_conformance.rs::pg_store_conformance` over 23 `CASES`, with `pg_conformance.rs::case_list_matches_mem_store` holding the two literals in step; the second clause is `mem_store.rs::mem_store_read_conformance`, `pg_conformance.rs::pg_store_read_conformance` and `cache.rs::the_mirror_passes_the_read_cases` over 6 `READ_CASES` |
| 20 | **No test, by nature — this criterion is the toolchain gate**, and it is the one row in this table that is a command rather than a name. Run whole-repo at close-out: `cargo fmt --all -- --check` clean; `cargo clippy --workspace --all-targets --all-features -- -D warnings` clean; `cargo doc --workspace --no-deps` clean (**green for the first time since F-33**, which was CLEAN-1's fifteen pre-existing errors); `cargo sqlx prepare --check -- --all-targets --all-features` from `crates/htui-store` clean. No workspace dependency was added: `htui-core` gained `sha2` and the dev-dependency `insta`, both already workspace entries and already in `Cargo.lock`, so no package entered the tree |

## ANA-5 §12 criterion 21 — all five closed

| Item | Closed by | Answer |
|---|---|---|
| the no-workspace `Scope` bound | **D95** | A new `PromptScope { workspace: Option<WorkspaceId>, project: ProjectId }` in `model/scope.rs`. The shipped `Scope` is **not** touched: it documents itself as "always one workspace, never a bare project list", `R-ENT-2` says there are no implicit workspace rows, and 20+ sites across five crates mean "the active workspace" by it. The SQL already takes `$workspace` and `$project` separately, so the second type costs nothing at the query |
| generalising `conformance::run_case` over `ReadStore` | **D96** | Neither. The question has a sharper answer than it expected: `run_case` is *already* generic, and the bound cannot be relaxed because eleven of the twenty-two existing cases call write methods. `READ_CASES` + `run_read_case<S: ReadStore>` lands **beside** it, additively, and `CacheStore` binds to that |
| the real per-family `chars-v1` constants | **D99 → D108** | Measured, not deferred again. See "The estimator" below |
| dropping `set_step_usage`'s third parameter | **D97** | It keeps it, and this is not a deferral. Its only production caller is the **free-standing chat** path, whose prompt has no template, no sections and no assembler — its digest is computed in the recorder and has nowhere else to be written. Dropping the parameter would delete the chat path's only digest writer to tidy a signature |
| per-family excerpt renderings | **D98** | One rendering for both families: line numbers on, `N \| ` form. `prompt_digest` stays agent-independent, which is what keeps invariant 3's fan-out identity true the moment MOD-4 mixes families across a fan-out |

---

## The estimator, and the ANA-5 amendment it forced (D108)

**`docs/ANA-5.md` §4.4's Claude-family constants and §5.1's example `estimator` value are amended
here rather than in the ANA** (maintainer-only, the milestone-5 precedent). The estimator ships as
**`chars-v2`, Claude family prose 25 / code 24** (the ×10 integers, i.e. 2.5 and 2.4 characters per
token). The GPT/Gemini row keeps ANA-5's 40 / 33 under a **separate** id, `chars-v1-gpt`, and keeps
its "Unverified" status. The Claude row stays the default for an unknown agent.

**F-16, the measurement** (2026-09-11, `claude` 2.1.267, `claude-opus-5[1m]`, differencing
`input_tokens + cache_creation_input_tokens + cache_read_input_tokens` on the terminal `result`
against a minimal-prompt baseline — **all three fields**, because `input_tokens` alone reports 2,
which is why milestone 8's committed fixtures could calibrate nothing):

| Run | chars | total input tokens | delta vs baseline | chars/token |
|---|---|---|---|---|
| baseline (`Reply with exactly the word: ok`) | — | 17 150 | — | — |
| prose | 40 000 | 33 108 | 15 958 | **2.507** |
| prose, half size | 20 000 | 25 168 | 8 018 | **2.494** |
| code | 40 000 | 33 543 | 16 393 | **2.440** |

ANA-5 §4.4 ships 3.5 / 3.0. Prose is **28.4%** away, outside D99's ±25% threshold — but the
threshold is not the argument, the **direction** is. Assuming 3.5 where reality is 2.51
under-counts every prose section by **40%**: a prompt the assembler believes is 108 000 tokens is
151 000, against a reserve sized for a rounding error. That is ANA-5 risk 1 arriving at twice its
assumed size, **and it would have shipped silently**, because every trim assertion in the plan
checks *internal* arithmetic — that `tokens_after` sums to `estimated_after`, that
`estimated_after <= target` — and all of them pass just as well against a wrong constant.

The two prose sizes agree to 0.5%, which is what makes this a measurement rather than a sample.
ANA-5 predicted the sign itself and did not apply it: §4.4 cites the note that later Claude models
"produce approximately 30 percent more tokens than on earlier models" and then adopts constants
read off pre-2026 published ranges anyway. 2.51 against 3.5 is exactly that 30%.

A **new id** rather than new constants under the old one, because `trim_record.estimator` is what
makes a stored record interpretable — "every token figure in the record is by this estimator and no
other" — which is false the moment one id carries two arithmetics. Code moved with prose even
though 18.7% is inside the threshold: one estimator with one row measured and one row guessed is
worse than either.

**F-17 — the GPT/Gemini row is not measurable on this box**, and that is now recorded rather than
assumed: it would be measured against `agy`, and milestone 7 established that `agy_acp_server`
emits **no `usage_update` whatsoever**, so there is no per-turn token figure to difference.

**F-18 — the conservative-default argument survives and is strengthened.** At 3.5 "the Claude row
is the more conservative" was false for any agent whose real ratio is lower; at 2.4 it is true
against both published rows, because a lower chars-per-token yields a *higher* estimate for the
same text.

The measurement is a committed, `#[ignore]`d live test rather than a table in a plan:
`crates/htui-agent/tests/estimator_live.rs`, four turns, asserting the ratios against
`TokenEstimator::DEFAULT`'s own constants within ±10% (so a constants edit that did not re-measure
fails there) and asserting the two prose sizes agree within 2% (so a broken baseline subtraction
fails loudly instead of silently re-deriving a plausible number). A confirming run on 2026-09-14
against `claude` 2.1.272 is recorded in the file's own module doc: baseline 17 327, prose 2.447 and
2.449 at the two sizes (0.07% apart), code 2.475 — `chars-v2` holds, both far inside ±10%.

---

## Review gate — milestone 9

`rust-reviewer` over the full change set: **1 CRITICAL, 7 HIGH, 11 MEDIUM, all applied.** Three are
worth more than their severity label.

**CRITICAL — the trim rungs re-rendered from unscrubbed inputs, undoing D100** (`f48b82b`). D100 put
the scrub between the render and the estimate. Every trim rung then re-rendered from `PromptSpec` —
the stat-only diff, the stubbed upstream ladder, the tail-cut verification block, the surviving
excerpts, the stat-only candidate — so a masked byte came **back** at step 6, while
`surviving_audit` went on hashing a masked block the prompt no longer carried. Proven, not
suspected. The fix is an input-layer scrub: `scrubbed_inputs` masks every digested string **once**,
before the first render, and hands the trimmer masked data, so a rung added later inherits the
property. The per-section pass stays as the fail-closed residue *scan*.

**HIGH — `FsRepoReader::read` followed symlinks with no size cap, while `select` never intersected
provider candidates with the listing** (`4b24420`, `ab67ee1`, `4ea2f8b`). Two halves of one hole.
`read` tested for `..` and a leading `/` and then called `std::fs::read(root.join(path))`, which
follows a link at **any** component; and `select` merged provider candidates after the path
predicates and called `read` on them without ever checking that the walk had listed them. A
provider proposing an ordinary-looking repo-relative path that is a symlink to `~/.ssh/id_rsa` put
bytes from outside
the repository into a prompt under a repo-relative path — hazard H-1 arriving through the
filesystem, and invisible to the pure crate, which names no `std::fs`. `read` now descends one
component at a time with `symlink_metadata`; `vetted()` intersects every candidate with the
listing, which is the walk's own output and therefore already carries all six skip rules. The cost
is stated rather than hidden: a candidate under a root that would not resolve, a repo whose listing
failed, or a path past `scan_cap`, is refused and noted.

The other five HIGH: raw template literals reaching digested bytes unscrubbed; `edit_proposal.path`
absolute in digested bytes (`da65e6a` — ACP carries absolute paths by protocol and nothing between
the wire and `from_events` rewrote one, so a handoff prompt shipped
`<config_dir>/trees/<run_id>/<step_id>/…`: an absolute path, a run id **and** a step id, against
§4.2 rule 5 and §4.7 rule 8 at once); the Postgres `prompt_tokens` projection **raising** on an
integral float (`98090a7` — `35988.0` satisfies every numeric jsonpath predicate, so `::int` got
the text `35988.0` and raised `22P02`, failing the *whole* `runs()` read where `prompt_summary`
answers `None`); the Pg `trimmed` projection diverging under lax-jsonpath auto-wrap (`65c65d7`);
and the upstream walk returning the **root itself** on a `blocked_by` cycle (`459f080` — `0001_init`
forbids a self-loop only, so `R blocked_by A, A blocked_by R` is storable, and both recursive CTEs
start at the root's neighbours with nothing downstream excluding it: the assembler would have fed
an item its own summary as upstream context).

Of the eleven MEDIUM, two are worth naming. `head+tail` searched with one yardstick and the record
re-measured with another (`e54616b`), two ceil-divisions apart, so a cut the search believed cleared
the deficit exactly could land a token short — and a 1-token residual after the last document's
head+tail is what sends `trim_documents` into its **drop** loop, removing a section whole that never
reached its floor. And `catch_unwind` stops the unwind, not the **hook** (`6f98318`): a provider
panic that `run_providers` catches and records per H-20 was still running `ratatui::restore()`,
leaving the TUI with no alternate screen and no raw mode under a running event loop.

---

## Findings, and the open ANA questions

The milestone-9 findings ledger is `.claude/plans/mod-2-prompt-assembler.plan.md`, "Implementation
findings" — F-16..F-122 across ten task agents, three reviewers and five fix agents. Two things
about it belong here rather than only there.

**A numbering collision was resolved at close-out.** Two different findings were both issued as
**F-50**. The one pinned in the tree — "the plan's `Validate` command names `--features
demo,test-support`, and `htui` declares exactly one feature, `testkit`", cited at
`crates/htui/tests/prompt_preview.rs:5` — **keeps F-50**, because renumbering it means editing
source to fix bookkeeping. The other — "an unreadable subdirectory must not cost a repo its
excerpts" (`f3b96d6`) — is **renumbered F-122**.

**Two ANA-amendment questions stay open for the maintainer.** Neither is resolved here and neither
should be read as closed:

- **F-34 — ANA-5 §4.4 step 7 and §5.1's worked example disagree, and the example is arithmetically
  impossible.** Step 7 says a section reaching its floor without clearing the deficit moves to the
  drop and the pass advances only after it; §5.1's example keeps `upstream` at 1300 and
  `previous_diff` at 1180 while `documents:plan` pays the residual, which its own arithmetic cannot
  produce under step 7. **Step 7 is what the code implements**, per blueprint D.2, because it is
  what the keep-priority means; the rungs a section passed through are still recorded, and three
  tests clear the deficit inside one ladder to prove the intermediate states. Which of the two the
  ANA meant is a **maintainer decision and is still open**.
- **F-37 — the separator between the N blocks of `{{documents}}` / `{{candidates}}` is undecided in
  the ANA.** A blank line was chosen so the milestone could render something; it is a digest input,
  so changing it later invalidates every golden snapshot. **Closed 2026-09-16 by the maintainer as
  bigger than a separator:** whether the section framing should be calibrated *per model* is an
  analysis in its own right, now **ANA-17**. The blank line stays until that concludes.
- **L-5 — `render.rs` renders `hostname:` into digested bytes.** It is sanctioned: §4.2's closed
  field list includes it. But it is a machine identifier, so identical inputs digest differently on
  another box, which is a weaker form of the reproducibility invariant than §4.7 reads as
  promising. **Closed 2026-09-16 by the maintainer, and it amends ANA-5:** the hostname leaves the
  **digest** but stays in the prompt, because an agent building the same project across several
  machines has to know which box it is on — the maintainer's own graphics-engine case. It
  additionally gains a settings switch so a project that does not want it can omit the field
  entirely. §4.2's closed field list gains a conditional field and §4.7 rule 8's "no machine
  identifier" becomes a statement about the digest rather than about the prompt. The work is
  **MOD-33**; per the milestone-5 precedent the amendment is recorded here and in `HANDOFF.md`
  rather than in `docs/ANA-5.md`, which only the maintainer edits.

---

## Known, accepted, and handed on

- **ANA-4 §11 criterion 12 is withdrawn with the mode it proved, which is a different sentence from
  "unproven".** It was proven end to end on 2026-09-08 (`crates/htui/tests/chat_offline.rs`: an
  offline chat driven by the production runtime writes the buffer, `upload_pending` lands exactly
  those rows, second pass a no-op). **MOD-25** then made `htui` online-only by maintainer decision,
  so the writable-offline direction is being retired. The buffered-write path is **disabled, not
  deleted**, and MOD-25 owns both the disabling and the CLEAN item that removes it once the
  decision has sat.
- **Windows is unverified for every phase and that is MOD-16's, by name.** The Windows lint target
  cannot be built on this box at all (**TOOL-3**), so `htui-agent`'s Windows-conditional code was
  reviewed by eye rather than linted. Criterion 11's CLI half holds on Linux only.
- **The skills section has no writer.** `Skill`, `SkillVersion`, `SkillBinding` and `BoundSkill`
  land read-only (D105) with the `R-SKL-2` collapse and the cap, because criteria 10 and 15 cannot
  be tested without them. `upsert_skill` / `add_skill_version` / `set_skill_binding` are **MOD-9's**
  and are deliberately absent — the tables have existed since `0001_init.sql` and nothing writes
  them yet.
- **`render::template_text` is now unused by the assembler** and is kept for **MOD-9's** editor,
  which has a `ParsedTemplate` and no scrubber. Its doc says so (`fd9e752`).
- **The preview's own evidence is weaker than the PRD asked for in exactly one corner** (D110). No
  `Repo` fixture rows and no `repos()` read, so `trim_record.excerpts.roots` is `[]` in the preview
  and criterion 12's per-repo `no_path` is proved by unit test rather than from the binary. Cheap to
  reverse: ~40 lines, additive to `fixtures.rs` and `pg/demo.rs`, no signature moves.
- **The excerpt walk's root-level symlink gap is MOD-7's.** `FsRepoReader` refuses a symlink at any
  component *below* the root; a `RepoRoot` whose own path is a link is resolved by whoever writes
  `repo_box_path`, and nothing writes it yet.
- **`scan_cap` can veto a provider candidate, and that is MOD-4's to loosen.** `vetted()` refuses a
  candidate the listing never offered, and a listing truncated by `scan_cap` is a prefix — so on a
  very large repo a legitimate provider candidate can be refused and noted. The conservative
  direction, and the price of not leaning on `htui-agent` for a security property.
- **`trim_record`'s own strings are not scrubbed** (F-80). The scrub covers every *digested* byte;
  the record's notes and audit paths are generated text written to `run_step.trim_record` beside
  them. Minted as its own item rather than left as a sentence, because `R-SEC-3` gates persist
  paths, not prompt paths.
- **Two UI defects found at close-out** and minted: the sixth sub-tab overflows the detail strip by
  two columns at the pinned 100×30 (F-71), and a running prompt preview makes an adapter install
  refuse with a message about a probe (F-121). Both are maintainer's-choice fixes, both cross-cutting
  enough not to fold into this close-out.
- **MOD-4 consumes**: `DriverCaps`, `SessionSpec.cwd`/`extra_dirs`, `session_ref`, the
  `session_started` row, `quota::available`, `assemble()` for the judge and handoff prompts, and
  `set_step_prompt` as the pre-flight digest writer. **`0003_orchestration.sql` is still unheld** —
  this item added no migration after `0002`.

---

## Tests

At close-out, workspace-wide on **Linux** with Postgres live (`USERNAME=htui-ci`, TOOL-2):

```
1017 passed, 0 failed, 25 ignored
```

up from 687 at MOD-21's close-out. The 25 ignored are the `#[ignore]`d `*_live` suites, which spawn
real agents and spend real tokens. `--no-fail-fast` is not optional on this tree and milestone 8
recorded why: `cargo test` stops at the first failing binary, and a run that reported one flake and
exited is how two width-broken suites were briefly called green.

`cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`,
`cargo doc --workspace --no-deps` and `cargo sqlx prepare --check` are all clean.
