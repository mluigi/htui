# Plan: MOD-7 milestone 4 — paths and excerpts are real

> **Status: fact-checked, open questions answered** (2026-09-26). Every tree fact this plan relies
> on is listed under "Claims to verify"; "Verified claims" holds the fact-check verdicts, and
> amendments are marked inline with "(amended at fact-check)".

> **Fact-check (2026-09-26).** 56 claims: 46 verified, 10 amended, 0 falsified; task independence
> holds (the wave plan survives the file-set intersection). The maintainer adopted every open
> question's default (OQ-26…OQ-32). What changed:
> - **D125 (new).** The excerpt pass moves out of `phase_spec` into `assemble_prompt`, because
>   `phase_spec` is also the promote/handoff path's spec builder (`engine.rs:1270`, `strict =
>   false`) and `promote::handoff_spec` keeps `excerpts` through `..phase`. T2 gains a test that a
>   handoff spec carries no excerpts and runs no pass.
> - **D107.** `run.repo_scope` is `Vec<RepoId>` (`model/run.rs:200`), so the `(RepoId, String)`
>   scope is built by joining `repos(run.project_id)` names.
> - **D119.** A bare `touched_paths` glob uses `overlap::resolve`'s primary rule (the `is_primary`
>   repo, else `""`), not "the first scope repo". A scrub-dropped file still counts in
>   `audit.selected`, and the plan accepts that (see D119).
> - **T3** also rewrites the `no_path` frame asserts (`prompt_preview.rs:119-122`, `:481`), the
>   `preview.rs:479-488` unit test and the `feat_1` snapshot's notes line.
> - **T4.** The Postgres case goes in a new `crates/htui/tests/hierarchy_pg.rs`, and T4 rewords the
>   "twelve" in `serve`'s doc and the `try_serve` comment.
> - **T0.** The `mem_store.rs` assert message gains a MOD-7 milestone 4 clause.
> - **T2's file list** gains `crates/htui-agent/src/lib.rs` (an optional re-export). The Tasks
>   section records the independence check's three conditions.
> - **R-49** is downgraded, and D114, D116, D120 and the patterns table have corrected citations.

**Source**: `.claude/prds/mod-7-box-registry.prd.md`, milestone 4 (Delivery Milestones table, row 4):
"Repo paths inferred per project under the workspace root with a manual text box on failure, and
phase, judge and preview prompts carry excerpts read from those roots." Scope bullets "Repo path
inference (D5)" ("For every repo of every project in the workspace, look under this box's
`workspace_box_path` root for its checkout, matching normalised `remote_url` first and the directory
name `repo.name` second. One match writes a canonical row (F-102: resolve or refuse a linked root);
zero or several matches write nothing and the section offers a manual path text box. A manual row is
never replaced by inference."), "Excerpts from path rows (D6)" ("Phase and judge prompts and the
preview resolve the excerpt root through ANA-5 §4.5's fallback (`run_step_tree.path`, then
`repo_box_path.local_path`, then `no_path`) instead of `no_excerpts`.") and "Record corrections"
(the stale "no writer" comments). Success-metric rows "Path inference", "Manual wins", "Excerpts
non-empty" and "UI never blocks". Risk rows "Inference writes a wrong path" and "Wiring excerpts
changes every production prompt digest". PRD open question "How far below the workspace root
inference searches, and how `remote_url` is normalised — plan's call, tested". Design authority:
`docs/ANA-5.md` §4.4 (budget and trim order) and §4.5 (excerpt selection, step 1's fallback root,
the fail-open rules); the PRD's gate decisions **PRD D0–D7** are binding (D5 and D6 are this
milestone's).

**Requirements**: `R-BOX-4` (per-box repo and workspace paths), `R-PRM-1` (htui-selected file
excerpts), `R-PRM-3` (budget), `R-ID-4` (no writes into a managed repo), `R-SEC-3` (fail-closed
scrubbing), `R-NF-3` (no store handle and no blocking work on the render side), `R-TUI-8` (Settings).

**Complexity**: Medium. **No migration** under the recommended answer to OQ-26 (next stays `0008`).
One new `WriteStore` method (insert-if-absent) across five implementations and one store conformance
case; one new `htui-orch` module (checkout discovery and remote-URL matching); one shared excerpt
pass in `htui-agent` and one residual-budget function in `htui-core`; the engine's phase prompt and
the preview wired to them; one new `StoreRequest`/`StoreReply` pair served by `crate::hierarchy`;
one new key in Settings > Hierarchy. `GraphSource` does **not** change (D106).

**Routing**: routed as **plan** by `/handoff-run MOD-7` (the PRD, accepted 2026-09-25, and its
milestone table exist; milestones 1–3 are complete). **Staffing: Opus 5.5 for every step — plan,
fact-check, architect, implementers, verifiers and reviewer (`rust-reviewer`); Fable is not used
(maintainer standing instruction).** Ultracode for the implementers only, one workflow per task,
verify fan-out per round; the architect and the reviewer stay plain agents.

**Numbering**: milestone 1 used D1–D38, R-1–R-19, OQ-1–OQ-12; milestone 2's plan D39–D53,
R-20–R-28, OQ-13–OQ-19, its blueprint D54–D74 and R-29–R-32; milestone 3's plan D75–D89,
R-33–R-40, OQ-20–OQ-25, its blueprint D90–D103 and R-41–R-43. This plan's decisions are
**D104…D125** (D125 added at fact-check), risks start at **R-44**, open questions at **OQ-26**. Tasks restart at **T0** and are
always cited as "milestone 4 T*n*" outside this file. The PRD's gate decisions are cited as **PRD
D0…PRD D7**; ANA-5's open-for-the-maintainer items as **ANA-5 open 8** and so on.

**Base**: `main` at `9151eaa` (merge of milestone 3). Every line number below is the file's at
`9151eaa`, pre-edit. `df -h /` showed 85 GB free (81 % used) when this plan was drafted.

**Graphify note**: `graphify-out/` does not exist in this checkout, so nothing here was read from
it. Tree facts were located through the Gortex index (symbol search, symbol source, callers,
implementations) and cited at the line Gortex or a direct read reported; the fact-check pass
re-reads each at its line.

---

## Open questions for the maintainer (all answered 2026-09-26)

Each had a default this plan adopts so implementation is not blocked; the maintainer adopted every
default.

- [x] **OQ-26 — How a manual row is marked.** Answered 2026-09-26: default adopted (D110). `repo_box_path` has no column saying who wrote a row
      (`0001_init.sql:202-208`: `repo_id`, `box_id`, `local_path`, `updated_at`). **Default (D110):**
      no marker and no migration; inference only ever **inserts where no row exists**
      (`INSERT … ON CONFLICT (repo_id, box_id) DO NOTHING`), so no existing row — manual or
      inferred — is ever replaced by inference, and a manual `SetRepoPath` (an upsert) always wins
      over an inferred one. **Alternative:** migration `0008` adds `repo_box_path.source TEXT NOT
      NULL DEFAULT 'manual' CHECK (source IN ('manual', 'inferred'))`, so a later inference may
      refresh an inferred row that went stale and the section can label it; costs a migration, the
      `tests/migrations.rs` pins and a `COMMENT ON COLUMN`.
- [x] **OQ-27 — When inference runs.** Answered 2026-09-26: default adopted (D114). **Default (D114):** on demand with a new `i` key in
      Settings > Hierarchy, and automatically after a `set_workspace_root`, `create_repo` or
      `update_repo` reply applies (the section sends the follow-up request). Never on the render
      side and never per keystroke. **Alternative:** also at every `Online` swap for every workspace
      that has a root on this box, beside milestone 1's registration probe — no key press needed on
      an existing database, at the cost of a filesystem walk on every connect.
- [x] **OQ-28 — Where the manual path text box lives.** Answered 2026-09-26: default adopted
      (D115). The PRD says "the section offers a manual
      path text box"; MOD-49's HANDOFF text places repo paths "in the box section". Settings >
      Hierarchy already has one: `b` on a repo row opens `EditorKind::RepoPath` and sends
      `StoreRequest::SetRepoPath`, canonicalised (F-102) by `crate::hierarchy::canonical`
      (`crates/htui/src/hierarchy.rs:415`). **Default (D115):** reuse it; inference reports its
      failures in the Hierarchy section's notice and points at `b`. No new writer, no new editor.
      **Alternative:** a repo-path list with its own text box in Settings > Boxes (a second editor
      over the same `SetRepoPath`, new snapshots, and a Boxes section that starts reading the
      hierarchy).
- [x] **OQ-29 — The excerpt pass's budget.** Answered 2026-09-26: default adopted (D118).
      `ExcerptRequest::budget_tokens` is "the residual
      budget from §4.4 step 6" and nothing computes it today. **Default (D118):** measure it —
      assemble the spec once with no excerpts, take `trim.target − trim.estimated_after` (floored at
      0), select under that, then assemble for real. **Alternative:** pass the whole `target` and
      let §4.4's trim drop excerpt files first (excerpts are first in the trim order); simpler, but
      the pass then reads up to `max_files` files it may immediately drop, and ANA-5 calls excerpts
      "budget-derived, not budget-trimmed".
- [x] **OQ-30 — An excerpt that trips the scrubber** (ANA-5 open 8). Answered 2026-09-26: default
      adopted (D119). `assemble` masks every excerpt
      and fails the whole prompt on residue (`crates/htui-core/src/prompt/mod.rs:706-851`,
      `AssembleError::Unmasked`), and `MinimalScrubber` treats any `PRIVATE KEY` substring as
      unmaskable (`crates/htui-core/src/scrub.rs:43`, `:242`) — `scrub.rs` itself contains it. Wired
      as is, one selected file blocks the item (`RunFailure::PromptRefused`). **Default (D119):**
      the pass pre-scrubs each selected file with the caller's scrubber and **drops** one that fails,
      with a note naming its `repo:path` and the rule (never the content); excerpts are optional
      context and §4.5 is fail-open. **Alternative:** keep today's fail-closed behaviour and let the
      step be refused.
- [x] **OQ-31 — How strict the name fallback is.** Answered 2026-09-26: default adopted (D112).
      **Default (D112):** a directory-name match is
      accepted only if the repo has no `remote_url` or the candidate has no remote configured; a
      same-named checkout whose remotes all normalise to something else (a fork, an unrelated
      project) is rejected, and zero accepted candidates is `no match`. **Alternative:** accept any
      exact name match, as the PRD's sentence reads literally.
- [x] **OQ-32 — A fan-out group has no `run_step_tree` row at stage 3.** Answered 2026-09-26:
      default adopted (D108). `drive_group` assembles the
      group's one prompt (`crates/htui-orch/src/engine.rs:3444-3447`) before any candidate is
      prepared (`run_candidate` -> `candidate_live`, `upsert_step_tree` at `:3711`; amended at
      fact-check), whereas ANA-5 §4.5 assumes stage 2
      created every candidate tree first. **Default (D108):** the fallback applies as written — no
      tree row, so `repo_box_path` — and a note says the group read the managed checkout. The
      single-step path (`walk_live_step`, trees written at `:3110` before stage 3 at `:3125`) reads
      the step's own tree. **Alternative:** prepare the first candidate's tree before assembling
      (a reordering of `drive_group`, out of this milestone's size).

---

## Summary

`repo_box_path` has one writer, MOD-15's `SetRepoPath` (`crates/htui/src/hierarchy.rs:303-321`),
which upserts a canonical path (`canonical_root`, `crates/htui-core/src/root_path.rs:40-61`,
resolves a link and refuses a dangling or relative one). Nothing infers a row, so a checkout under
the workspace root still fails isolation with "no checkout for this repo on this box"
(`crates/htui-orch/src/isolate/real.rs:32-38`) until the maintainer types its path. And every
production prompt carries an empty excerpt set: phase prompts call `no_excerpts(caps)`
(`engine.rs:5031`), the judge too (`:4402`), and the preview `empty_excerpts(caps)`
(`crates/htui/src/preview.rs:271`, `:301`). `htui_core::prompt::excerpt::select` and
`htui_agent::excerpt::{FsRepoReader, run_providers}` exist, tested, with no production caller.

**The store writer (T0).** `WriteStore::infer_repo_box_path(&RepoBoxPath) -> Result<bool>` inserts
a row only where none exists for `(repo_id, box_id)` and answers whether it did. It is the
compare-and-set the PRD constraint asks for, keyed on absence, which no reconnect changes.

**Discovery and matching (T1).** A new `htui_orch::infer` module: a remote-URL normaliser, a bounded
deterministic walk that lists git checkouts under a canonical root without following links, and a
pure `choose` that applies remote-first, name-second, ambiguity-is-failure.

**The excerpt pass (T2).** A shared `htui_agent::excerpt::excerpt_pass` runs the built-in provider
and `select` over an `FsRepoReader`; `htui_core::prompt::excerpt_residual` measures the budget.
`Engine::assemble_prompt`, the phase-prompt caller of `phase_spec` (D125, amended at fact-check),
resolves one root per repo of `run.repo_scope`: the step's `run_step_tree` row, else this box's
`repo_box_path` row, else `no_path`. It does this through store reads the engine already has. When
the phase template places `{{excerpts}}`, it runs the pass on a blocking thread and replaces the
spec's `no_excerpts`. `phase_spec` itself still builds `no_excerpts`, so the promote/handoff path
reads no files. The judge keeps `no_excerpts` because its placeholder set cannot place excerpts
(D109).

**The preview (T3).** `preview::build` resolves roots for the project's repos from this box's
`repo_box_path` rows and runs the same pass, so the Prompt sub-tab shows what a run would read.

**Inference served and shown (T4).** `StoreRequest::InferRepoPaths(WorkspaceId)` is served by
`crate::hierarchy` on the store worker: this box's root for the workspace, canonicalised, walked
once off the async task, every repo without a row matched, each single match canonicalised and
inserted-if-absent, answered with the fresh tree and a per-repo report. Settings > Hierarchy sends it
on `i` and after a root or repo write, and renders the report as its notice.

## Design decisions (settled here, not in code review)

| # | Decision | Why |
|---|---|---|
| D104 | **Insert-if-absent writer.** `async fn infer_repo_box_path(&self, path: &RepoBoxPath) -> Result<bool>` on `WriteStore` (`crates/htui-core/src/store/traits.rs`, beside `upsert_repo_box_path` at `:604` and `repo_box_paths` at `:610`). `true` = inserted; `false` = a row for `(repo_id, box_id)` already existed and is untouched. `Constraint` when either id names no row, like `upsert_repo_box_path`. `MemStore` (`State` beside `upsert_repo_box_path`, `mem.rs:2292-2316`): the same two existence checks, then push only when absent, stamping `updated_at = now`. `PgStore` (`pg/write.rs` beside `:1908-1921`): one `query!` `INSERT INTO repo_box_path (repo_id, box_id, local_path) VALUES ($1, $2, $3) ON CONFLICT (repo_id, box_id) DO NOTHING`, answering `rows_affected() == 1`, `23503` mapped by `map_sqlx`. Forwards in `Writer` (`crates/htui-store/src/writer.rs`, beside `:599`), `UsageSpy` (`crates/htui-agent/src/conformance.rs`, beside `:882`) and `SpyStore` (`crates/htui-agent/tests/recorder.rs`, beside `:577`). | PRD constraint "Every new box writer is a compare-and-set keyed on a token a reconnect does not bump": the token is the row's absence, decided atomically by the conflict clause, so a manual write racing inference either lands first (inference then answers `false`) or overwrites the inferred row (manual wins). MOD-15's `WriteStore` has five implementors today (no `BufferedWriter`), all needing the method because the trait has no default bodies. |
| D105 | **One store conformance case**, `infer_repo_box_path_inserts_only_where_absent`, appended to `CASES` with its `run_case` arm: insert on an empty pair answers `true` and reads back through `repo_box_paths`; a second call with another path answers `false` and leaves the first path; after `upsert_repo_box_path` (the manual writer) a call answers `false` and the manual path stands; an unknown repo and an unknown box are each `Constraint`. Store `CASES` 76 → 77. | Both stores, one spec (`R-NF-4` parity). |
| D106 | **No `GraphSource` change.** The engine is `Engine<'a, S: WriteStore, …>` (`engine.rs:457-470`) and every read the root needs is already on `S`: `ReadStore::step_trees(step)` (`traits.rs:161`), `WriteStore::repos(project)` (`:597`) and `WriteStore::repo_box_paths(repo)` (`:610`), filtered to `self.parts.box_id`. | `GraphSource` exists for inherent reads the engine cannot reach through `S` (`graph.rs:33-55`); these it can. `run_worker::repo_map` (`crates/htui/src/run_worker.rs:1130-1159`) already joins the same rows through `Backend::repo_paths` for the isolator. |
| D107 | **Root resolution** (ANA-5 §4.5 step 1), a private free function in `engine.rs`, `excerpt_roots(scope: &[(RepoId, String)], trees: &[RunStepTree], paths: &[RepoBoxPath]) -> Vec<RepoRoot>`: one `RepoRoot` per scope repo, `repo` = `repo.name` (the slug `PathPrefix` and the rendered `path="repo:…"` use), rung by **row presence**: the step's `run_step_tree` row for that repo (`RootSource::RunStepTree`), else this box's `repo_box_path` row (`RootSource::RepoBoxPath`), else `RootSource::NoPath` with an empty `root`. No `stat` here: an unreadable root is `select`'s "could not be listed" note (`crates/htui-core/src/prompt/excerpt.rs:1101-1113`), which keeps resolution pure and deterministic. A scope id with no `repo` row in the item's project is left out with a note. The scope is `run.repo_scope`, which is a `Vec<RepoId>` (`crates/htui-core/src/model/run.rs:200`), so the `(RepoId, String)` pairs are built in the excerpt step (D125) by joining each id to its name from `repos(run.project_id)` (amended at fact-check). | ANA-5 §4.5 step 1 verbatim, and criterion 12's per-repo `no_path` becomes true in production. Unit-testable without a filesystem. |
| D108 | **Fan-out groups** (OQ-32): `drive_group` assembles before any candidate tree exists, so the group's roots come from `repo_box_path`; the excerpt step (D125) adds the note `excerpt: no run_step_tree row yet for this step; roots read from repo_box_path` when a scope repo resolved through that rung while the step had no tree rows at all. | Honest about what the agent's tree and the excerpt tree are; no reordering of the walk. |
| D109 | **The pass runs only where it can render.** The excerpt step (D125) parses the spec's pinned template (`htui_core::prompt::parse(spec.role, &spec.body)`, re-exported at `prompt/mod.rs:46`) and runs the pass only when `parsed.used` contains `Placeholder::Excerpts` and at least one root is not `NoPath`; otherwise `no_excerpts(caps)` as today, with the resolved `NoPath` roots still recorded in `audit.roots`. **The judge keeps `no_excerpts`**: `Placeholder::allowed_in(TemplateRole::Judge)` admits only `item_key`, `phase`, `skills`, `task`, `candidates` (`crates/htui-core/src/prompt/template.rs:188-215`), so no judge body can place `{{excerpts}}`, and the default `verdict` phase body does not either (`crates/htui-core/src/prompt/defaults.rs:126-137`). The comment at `engine.rs:4402` is reworded to say so. | Reading files that cannot reach the prompt is I/O for an audit row only. The PRD's "judge prompts" clause is a disagreement recorded below. |
| D110 | **No manual marker, no migration** (OQ-26 default). Inference never replaces a row; a stale inferred row is fixed by hand with `b`, exactly like a stale manual one. | Satisfies PRD "A manual row is never replaced by inference" and the success metric "Manual wins" without schema work. |
| D111 | **Remote-URL normalisation**, `htui_orch::infer::normalise_remote(url: &str) -> Option<String>`, pure: trim; drop a trailing `/` then a trailing `.git`; scp-like `[user@]host:path` (no `://`, a `:` before the first `/`) becomes `host/path`; a URL with scheme `https`, `http`, `ssh`, `git`, `git+ssh` or `ssh+git` drops the scheme, any `user[:password]@` and any `:port`; `file://…` and an absolute local path become `file:<path>`; the host is lowercased, the path kept byte-exact, a leading `/` and repeated `/` collapsed. `None` for an empty or unparseable input. The key never carries credentials, and no raw remote URL is logged, rendered or put into the report. | PRD open question, "plan's call, tested": `https://github.com/o/r.git`, `git@github.com:o/r`, `ssh://git@github.com:22/o/r/` and `https://user:tok@GitHub.com/o/r` are one repo; path case is kept because a mismatch only falls through to the name rung (fail-safe). |
| D112 | **Choice** (`htui_orch::infer::choose`, pure) per repo that has no row on this box: (1) if `remote_url` normalises to a key, the candidates with **any** remote normalising to it — one is `Inferred { by: Remote }`, several `Ambiguous`, none falls to (2); (2) candidates whose directory name equals `repo.name` byte-exact, filtered by OQ-31's rule — one is `Inferred { by: Name }`, several `Ambiguous`, none `NoMatch`. A candidate path already held by another repo's row on this box is excluded before either rung. | PRD D5: "Remote URL first, then directory name … ambiguity is a failure"; PRD risk "a fork or a second clone" both land in `Ambiguous` or the rejected name rung. |
| D113 | **Discovery**, `htui_orch::infer::find_checkouts(root: &Path) -> Scan { checkouts: Vec<Checkout>, truncated: bool }`, synchronous, called under `spawn_blocking`: depth-first from the canonical root, entries sorted by file-name bytes at every level (the `FsRepoReader` walk's determinism rule), **depth ≤ 3** below the root (the root itself is depth 0 and may be a checkout), entries whose name starts with `.` skipped, links never followed (`symlink_metadata`), unreadable directories skipped, a directory holding a `.git` entry (directory or `gitdir:` file) is a candidate and is not descended into, and at most **5 000** directories examined (constants `MAX_DEPTH`, `MAX_DIRS`). `Checkout { path, name, remotes }` where `remotes` are every configured remote's fetch URL, read with `gix` (already `htui-orch`'s, `Cargo.toml:38`) through the repository's own config; an unopenable checkout has no remotes. **A truncated scan infers nothing** (every repo reports `ScanTruncated`). | Bounded cost on the store worker (R-45). Links not followed means every candidate path is already canonical under a canonical root, and F-102 holds by construction; T4 still passes each chosen path through `canonical_root` so there is one refusal path. Truncation fails closed because an unseen second clone would turn a wrong single match into a write. |
| D114 | **Trigger** (OQ-27 default): `StoreRequest::InferRepoPaths(WorkspaceId)`, served in `crate::hierarchy::serve` and routed in `store_worker::try_serve` with the other hierarchy requests (`crates/htui/src/store_worker.rs:1073-1089`); `REQUEST_NAMES` 12 → 13 (`hierarchy.rs:456`), name `infer_repo_paths`. Settings > Hierarchy sends it on `i` (Browse only, refused while busy like the other write keys) and once after a `set_workspace_root`, `create_repo` or `update_repo` reply applies. Only this box's filesystem is searched and only this box's rows written. | PRD `R-NF-3`: served on the store worker, the walk under `spawn_blocking` (the precedent is `canonical`, `hierarchy.rs:415-430`). `i` is free in the section (`on_key`'s own comment lists the global keys, `settings/hierarchy.rs:1049-1051`). |
| D115 | **The manual text box is Hierarchy's `b`** (OQ-28 default). No new writer and no new editor. | It exists, canonicalises (F-102), and is tested (`crates/htui/tests/hierarchy.rs:333`). |
| D116 | **Serving `InferRepoPaths`** (`crate::hierarchy`): `box_id(this_box)?`; this box's `workspace_box_paths(ws)` row, else report `NoRoot`; `canonical(root)` (a legacy link row walks its target); every repo of every project of the workspace (`snapshot`'s own traversal, `hierarchy.rs:113-153`) minus those with a row on this box (`AlreadySet`); `spawn_blocking(find_checkouts)`; `choose` per repo; for `Inferred`, `canonical(path)` then `writer.infer_repo_box_path(..)` (`false` → `AlreadySet`, a lost race with a manual write); answer `StoreReply::RepoPathsInferred { tree: Box<HierarchySnapshot>, report: InferReport }` with the re-read tree. `InferReport { root: Option<String>, truncated: bool, repos: Vec<RepoInference { repo: RepoId, name: String, outcome: InferOutcome }> }`, `InferOutcome = AlreadySet \| Inferred { path, by } \| NoMatch \| Ambiguous { candidates: usize } \| Refused(String) \| ScanTruncated`; `NoRoot` is `root: None` with `repos` empty. No `BoxId` or `UserId` in the reply. | One reply carries both what changed and why the rest did not, so the section never patches rows locally (MOD-15 D5's one source of truth). |
| D117 | **Section rendering.** `on_reply` treats `RepoPathsInferred` as a tree (`on_tree`) plus a notice built from the report: `inferred N of M · K already set · no checkout: a, b · ambiguous: c (2) — b on a repo sets it by hand`, or `no root on this box for this workspace — b on the workspace row sets it`, or `the scan stopped at 5000 directories; nothing inferred`. `HINT_BROWSE` (`settings/hierarchy.rs:46`) gains `i infer`. Repo rows already print the path or `unset` (`:311-326`). | The notice is the section's existing channel (`:357-370`); PRD "path shown in the section". |
| D118 | **The residual budget** (OQ-29 default): `pub fn excerpt_residual(spec: &PromptSpec, scrubber: &dyn Scrubber) -> Result<i64, AssembleError>` in `crates/htui-core/src/prompt/mod.rs`: `assemble` a clone of `spec` with `excerpts` emptied, answer `(trim.target - trim.estimated_after).max(0)` (`TrimRecord`, `prompt/trim.rs:183-211`). An `Err` is returned unchanged and the caller skips the pass, because the real `assemble` will refuse identically. The excerpt section's framing paragraph is not in the residual; an overshoot is absorbed by §4.4's trim, which drops excerpt files first. | §4.4: excerpts are "budget-derived rather than budget-trimmed". Pure, deterministic, in the crate both callers depend on. |
| D119 | **The shared pass**, `pub fn excerpt_pass(req: &OwnedExcerptRequest, est: TokenEstimator) -> ExcerptSet` in `crates/htui-agent/src/excerpt.rs`: `run_providers(&[Arc::new(BuiltinRanker)], &req.as_request())`, then `select(&FsRepoReader::new(req.caps), &req.as_request(), merged, provider_set, est)`. Callers run it under `tokio::task::spawn_blocking`; a `JoinError` is recorded as the note `excerpt: the pass panicked; no excerpts` and the prompt proceeds with `no_excerpts` (§4.5 fail-open). **Scrub filter** (OQ-30 default): after the join the caller keeps only files whose `content` scrubs clean with its own scrubber, and notes each dropped one as ``excerpt: `repo:path` dropped; the scrubber refused it (rule `…`)``. `assemble` then rebuilds `audit.files` from the survivors in `surviving_audit` (`prompt/mod.rs:924-976`, called from `assemble` at `:510`), and `notes()` (`:978-984`) puts the spec's own notes first, then `excerpts.notes`. **Audit counters (amended at fact-check):** `surviving_audit` copies `spec.excerpts.audit` and rebuilds only `files`, so a dropped file still counts in `audit.selected` (and `considered`). The filter **does not adjust the counters**, and the plan accepts the disagreement. `ExcerptAudit` (`crates/htui-core/src/prompt/excerpt.rs:316-330`, doc sentence at `:313`) defines `selected` as "how many the ranker chose" and `files` as "what survived into the prompt", and calls that asymmetry the point. A scrub-dropped file was chosen and read, and the drop note names it. The request: `item_key`, `item_body`, `phase`, the resolved input documents' bodies, `touched_prefixes` from `item.touched_paths` via `PathPrefix::parse(glob, primary)`, `changed_paths` empty (D122), the roots, `budget_tokens` from D118, and `caps`, `scan_cap`, `deadline` from `settings::resolve_excerpt_caps(app)`. **Primary (amended at fact-check):** `primary` is the project's `is_primary` repo's name, else `""`. This is the rule of `crates/htui-orch/src/overlap.rs:43-54` (`resolve`), so a bare glob in a project with no primary maps to no repo and selects nothing, exactly as the overlap check treats it. T2 copies the rule and does not edit `overlap.rs`. | One function both callers share, so the preview's bytes and a run's cannot drift (MOD-2 D103's rule). `FsRepoReader::new(caps)` from the same resolved caps is F-101's requirement. |
| D120 | **The preview** (`preview::build`, `crates/htui/src/preview.rs:151-291`): the repo set is the project's repos (`backend.writer()` → `repos(project)`; ANA-5 §4.5 step 1 "otherwise the project's repos"), roots from `backend.repo_paths(this box)` else `NoPath`, the same pass under `spawn_blocking`, the scrub filter with `MinimalScrubber::new([])`. `EXCERPTS_NOTE` (`:104-107`) is reworded to "preview: no run exists, so no run_step_tree row; roots come from this box's repo_box_path rows, else no_path" and stays in `STAND_INS`. `empty_excerpts` is deleted or kept only for the no-repo case. Either way, its second caller changes too: the `preview.rs` unit test `the_empty_audit_registers_the_builtin_and_records_the_caps` (`:479-488`, the call at `:486`, the "plan D110" assert at `:488`) is rewritten or deleted (amended at fact-check). The Prompt sub-tab's empty-roots copy (`crates/htui/src/ui/tabs/backlog/detail/prompt.rs:243-246`, inside `excerpt_lines` at `:235-264`) becomes `none · this project has no repo`. That removes `no_path` from the demo frame, so the frame asserts at `crates/htui/tests/prompt_preview.rs:119-122` and `:481` move with it. | Reverses MOD-2 D110 (no repo read in the preview) now that rows exist. The render side still holds no store handle: the preview runs on the agent runtime's deferred task (`preview.rs:15-20`). |
| D121 | **Record corrections in code.** The "no writer" comments at `engine.rs:5028-5030` and `:5750-5754`, `preview.rs:1-10` (module doc), `:292-300` (`empty_excerpts` doc) and `:104-107`, and `prompt.rs:243-244` are rewritten to the new facts. `crates/htui/tests/settings.rs:794` ("before MOD-7 and MOD-15 land") is corrected in T3. The strip comment the PRD names was already fixed in milestone 2 (`tests/settings.rs:945`: "61 of the 100 columns"). | PRD "Record corrections"; docs and HANDOFF are the main thread's. |
| D122 | **Not changed, on purpose:** tier 2 (`changed_paths`, the previous attempt's diff) stays empty — `DiffBlock` carries no repo-qualified path list (`prompt/mod.rs:159-166`) and the PRD does not ask for it; `ExcerptProvider`s beyond the built-in (ANA-3); `GraphSource`; the SQLite mirror (`repo_box_path` is not mirrored); the isolator (`run_worker::singletons` re-reads the repo map on `StartRun`, `run_worker.rs:856-905`, so an inferred row is picked up on the next start); `R-ORCH-10`; `docs/**`, `HANDOFF.md`, the PRD. | Scope. |
| D123 | **Where the discovery code lives: `htui-orch`.** `gix` and the `spawn_blocking` git-read pattern are already there (`crates/htui-orch/src/isolate/real.rs:1-11`), and the isolator is the consumer of the rows. `crates/htui` depends on `htui-orch` (`crates/htui/Cargo.toml`). | Alternative, `htui-agent` beside `box_probe`, would add `gix` to a crate that has none. |
| D124 | **Inference writes this box's rows only**, for the requested workspace only, and searches only this box's filesystem; nothing in this milestone infers for another box. | A box can only stat its own disk (PRD D4: probing runs on this box only). |
| D125 | **The excerpt pass runs in `assemble_prompt`, not in `phase_spec`** (added at fact-check, claim 13). `phase_spec` (`engine.rs:4926-5049`) has two production callers. One is `assemble_prompt` (`:4898-4914`, the call at `:4907` with `strict = true`), the phase prompt for `walk_live_step` and `drive_group`. The other is `opening` (`:1228-1341`, the call at `:1270` with `strict = false`), the promote/handoff path. `opening` passes the spec to `promote::handoff_spec` (`crates/htui-orch/src/promote.rs:81-106`), which keeps `excerpts` through `..phase`. So `phase_spec` keeps building `excerpts: no_excerpts(caps)`, with its comment at `:5028-5030` reworded (D121). After `phase_spec` answers `Ok(spec)`, `assemble_prompt` calls one new private method, `Engine::with_excerpts(&self, run: &Run, step: &RunStep, item: ItemId, spec: &mut PromptSpec) -> Result<(), EngineError>`, and then assembles. `with_excerpts` does the rest of the pass: D109's placeholder test on `spec.role`/`spec.body`; D107's roots from `step_trees(step.id)`, `repos(run.project_id)` joined over `run.repo_scope`, and `repo_box_paths` filtered to `self.parts.box_id`; D108's note; D118's residual over the complete spec; D119's pass under `spawn_blocking` and the scrub filter. It re-reads the item row for `touched_paths` with the same `self.item(item)` read `phase_spec` uses. `phase_spec`, its direct unit tests (`:12275`, `:12360`) and `handoff_spec` do not change, so a handoff spec carries `no_excerpts` and triggers no file I/O. | This is the cleaner of the two fixes. Gating on `strict` would overload a flag that means "a missing required input is an error". Resetting `excerpts` in `handoff_spec` would still run the pass's file I/O for every handoff, and would parse the phase template, not the handoff body. In `assemble_prompt` the pass sees the finished spec, which D118's residual needs anyway. |

## Patterns to Mirror

| Concern | Mirror | Where |
|---|---|---|
| A `repo_box_path` writer on both stores | `State::upsert_repo_box_path`, `PgStore::upsert_repo_box_path` | `crates/htui-core/src/store/mem.rs:2292-2316`; `crates/htui-store/src/pg/write.rs:1908-1921` |
| A writer forwarded by every `WriteStore` implementor | `upsert_repo_box_path` in `Writer`, `UsageSpy`, `SpyStore` | `crates/htui-store/src/writer.rs:599`; `crates/htui-agent/src/conformance.rs:882`; `crates/htui-agent/tests/recorder.rs:577` |
| A store conformance case over a hierarchy writer | `repo_round_trip_and_primary_flag` | `crates/htui-core/src/store/conformance.rs:2778` (its `CASES` entry `:72`, `run_case` arm `:171`; amended at fact-check) |
| Canonicalise off the async task, refusal as `Constraint` | `crate::hierarchy::canonical` | `crates/htui/src/hierarchy.rs:405-430` |
| A hierarchy request served and re-read | `SetRepoPath`, `SetWorkspaceRoot`, `reread` | `hierarchy.rs:285-321`, `:361-369` |
| A deterministic sorted `std::fs` walk that never follows links | `FsRepoReader::walk` | `crates/htui-agent/src/excerpt.rs` (`walk`) |
| `gix` reads under `spawn_blocking` | `isolate::git::blocking`, `gix::open` | `crates/htui-orch/src/isolate/git.rs:1254-1259` |
| Building an excerpt request and running providers and `select` | `base_request`, `every_provider_failure_leaves_a_valid_prompt` | `crates/htui-agent/tests/excerpt.rs:125-145`, `:185` |
| The spec builder, and the caller the pass plugs into (D125) | `Engine::phase_spec`, `Engine::assemble_prompt` | `crates/htui-orch/src/engine.rs:4926-5049`, `:4898-4914` |
| The primary-repo rule for a bare glob (D119) | `overlap::resolve` | `crates/htui-orch/src/overlap.rs:43-54` |
| The preview spec builder | `preview::build` | `crates/htui/src/preview.rs:151-291` |
| A Postgres test in a `crates/htui` suite (T4) | `testkit::{fresh_db, demo_db}`, `testkit::SKIP` | `crates/htui/tests/box_probe_pg.rs:29-31` |
| A section key that sends a request, and its notice | `b`, `r`, `notice` | `crates/htui/src/ui/tabs/settings/hierarchy.rs:1101-1122`, `:357-370` |

## Files to Change

| File | Action | Task | Why |
|---|---|---|---|
| `crates/htui-core/src/store/traits.rs` | edit | T0 | `WriteStore::infer_repo_box_path` (D104) |
| `crates/htui-core/src/store/mem.rs` | edit | T0 | `MemStore`/`State` implementation (D104) |
| `crates/htui-core/src/store/conformance.rs` | edit | T0 | D105's case and arm; `CASES` 76 → 77 |
| `crates/htui-core/tests/mem_store.rs` | edit | T0 | pin 76 → 77 (`:37`, inside the assert at `:35-37`), and a MOD-7 milestone 4 clause in the assert's message, which lists every milestone's cases |
| `crates/htui-store/src/pg/write.rs` | edit | T0 | `PgStore` implementation (D104) |
| `crates/htui-store/src/writer.rs` | edit | T0 | forward |
| `crates/htui-store/tests/pg_conformance.rs` | edit | T0 | `EXPECTED_CASES` 76 → 77 (`:19`) |
| `crates/htui-store/.sqlx/` | regenerate | T0 | +1 statement (267 → 268) |
| `crates/htui-agent/src/conformance.rs` | edit | T0 | `UsageSpy` forward |
| `crates/htui-agent/tests/recorder.rs` | edit | T0 | `SpyStore` forward |
| `crates/htui-orch/src/infer.rs` | **new** | T1 | D111–D113, unit tests |
| `crates/htui-orch/src/lib.rs` | edit | T1 | `pub mod infer;` |
| `crates/htui-orch/tests/infer.rs` | **new** | T1 | filesystem cases over real `git init` trees |
| `crates/htui-core/src/prompt/mod.rs` | edit | T2 | `excerpt_residual` (D118) and its unit tests |
| `crates/htui-agent/src/excerpt.rs` | edit | T2 | `excerpt_pass` (D119) |
| `crates/htui-agent/src/lib.rs` | edit (optional) | T2 | add `excerpt_pass` to the `excerpt::{…}` re-export at `:148`; skip it if callers use `htui_agent::excerpt::excerpt_pass` |
| `crates/htui-agent/tests/excerpt.rs` | edit | T2 | `excerpt_pass` over a tempdir tree |
| `crates/htui-orch/src/engine.rs` | edit | T2 | D107–D109, D125's `with_excerpts` called from `assemble_prompt`, D119's call, D121's comments; unit tests |
| `crates/htui-orch/src/fake.rs` | edit | T2 | `FakeIsolator` tree-root override for tests (`FAKE_TREE_ROOT`, `:47`, `:431`) |
| `crates/htui/src/preview.rs` | edit | T3 | D120, D121; the unit test at `:479-488` that calls `empty_excerpts` and asserts `roots` empty |
| `crates/htui/src/ui/tabs/backlog/detail/prompt.rs` | edit | T3 | the empty-roots copy (D120) |
| `crates/htui/tests/prompt_preview.rs` | edit | T3 | the roots assertion at `:143`; the `frame.contains("no_path")` asserts at `:119-122` and `:481`; a repo-path case |
| `crates/htui/tests/settings.rs` | edit | T3 | the stale comment at `:794` (D121) |
| `crates/htui/tests/snapshots/{backlog__detail_prompt,prompt_preview__preview_ana_2,prompt_preview__preview_feat_1}.snap` | update | T3 | the `roots` line (`:15`) and the reworded note (`feat_1`'s notes line, `:36`, clipped) |
| `crates/htui/src/hierarchy.rs` | edit | T4 | D116, `REQUEST_NAMES` 12 → 13; `serve`'s doc (`:165-167`) no longer says "twelve" |
| `crates/htui/src/store_worker.rs` | edit | T4 | the request and reply variants, `name` (`:586`), the `try_serve` arm (`:1030`), and the "twelve" comment at `:1073` reworded |
| `crates/htui/src/ui/tabs/settings/hierarchy.rs` | edit | T4 | D114's key and follow-up, D117 |
| `crates/htui/tests/hierarchy.rs` | edit | T4 | MemStore and Offline serve cases, section cases, the names tests (`:92`, `:667`) |
| `crates/htui/tests/hierarchy_pg.rs` | **new** | T4 | the Postgres serve case, following `box_probe_pg.rs:29-31` (amended at fact-check: `hierarchy.rs` has no Postgres case) |
| `crates/htui/tests/snapshots/hierarchy__demo.snap` + new `hierarchy__inferred.snap` | update / add | T4 | hint line; the report notice |

**Not touched, on purpose:** every migration, `cache_migrations/` and
`crates/htui-store/tests/migrations.rs` (OQ-26 default); `crates/htui-orch/src/graph.rs` and every
`GraphSource` implementation (D106); `crates/htui-orch/src/conformance.rs` and its `CASES` pins
(engine unit tests carry this milestone); `crates/htui/src/run_worker.rs` and the isolator (D122);
`crates/htui-core/src/fixtures.rs` and the demo loader (cases create repos and paths at run time);
the Boxes section (OQ-28 default); `docs/**`, `HANDOFF.md`, the PRD.

## Tasks

**Order.** **Wave 1:** T0, T1 and T2 in parallel, each in its own worktree. Merge T0, T1, T2 in that
order, re-running the gates of every crate the merged task touched on the real tree after each merge
(`--test-threads=1`). **Wave 2:** T3 and T4 in parallel, each in its own worktree, both after Wave 1
is merged (T3 needs T2; T4 needs T0 and T1).

| Task | Files (complete list) | Parallel |
|---|---|---|
| T0 | `crates/htui-core/src/store/traits.rs`, `crates/htui-core/src/store/mem.rs`, `crates/htui-core/src/store/conformance.rs`, `crates/htui-core/tests/mem_store.rs`, `crates/htui-store/src/pg/write.rs`, `crates/htui-store/src/writer.rs`, `crates/htui-store/tests/pg_conformance.rs`, `crates/htui-store/.sqlx/`, `crates/htui-agent/src/conformance.rs`, `crates/htui-agent/tests/recorder.rs` | Wave 1, independent |
| T1 | `crates/htui-orch/src/infer.rs` (new), `crates/htui-orch/src/lib.rs`, `crates/htui-orch/tests/infer.rs` (new) | Wave 1, independent |
| T2 | `crates/htui-core/src/prompt/mod.rs`, `crates/htui-agent/src/excerpt.rs`, `crates/htui-agent/src/lib.rs` (optional re-export), `crates/htui-agent/tests/excerpt.rs`, `crates/htui-orch/src/engine.rs`, `crates/htui-orch/src/fake.rs` | Wave 1, independent |
| T3 | `crates/htui/src/preview.rs`, `crates/htui/src/ui/tabs/backlog/detail/prompt.rs`, `crates/htui/tests/prompt_preview.rs`, `crates/htui/tests/settings.rs`, the three preview snapshots | Wave 2, depends on T2; parallel with T4 |
| T4 | `crates/htui/src/hierarchy.rs`, `crates/htui/src/store_worker.rs`, `crates/htui/src/ui/tabs/settings/hierarchy.rs`, `crates/htui/tests/hierarchy.rs`, `crates/htui/tests/hierarchy_pg.rs` (new), `hierarchy__demo.snap`, `hierarchy__inferred.snap` (new) | Wave 2, depends on T0 and T1; parallel with T3 |

**Intersections, checked (verified at fact-check, claims 55-56 and the independence check).**
T0 ∩ T1 = ∅. T0 ∩ T2 = ∅: T0 touches `htui-agent/src/conformance.rs` and `tests/recorder.rs`, T2
touches `htui-agent/src/excerpt.rs`, `src/lib.rs` and `tests/excerpt.rs`; T0 touches
`htui-core/src/store/`, T2 touches `htui-core/src/prompt/mod.rs`. T1 ∩ T2 = ∅: T1 owns `htui-orch`'s
`lib.rs` and the new `infer.rs`, and T2 adds no module. T3 ∩ T4 = ∅: disjoint `crates/htui` files and
disjoint snapshot names. With `htui-agent/src/lib.rs` added to T2 and `hierarchy_pg.rs` to T4, **the
wave plan survives the file-set intersection**.

**Hidden couplings checked.** `.sqlx/` moves in T0 only (T1–T4 add no `query!`). Store `CASES` pins
move in T0 only; `htui-orch` `CASES` do not move. Snapshots: T3 owns the three preview snapshots, T4
the hierarchy ones. **Build coupling:** T0 adds a trait method. `htui-orch` and `crates/htui`
implement no `WriteStore` (verified, claim 10: five implementors, none in those crates), so T1 and T2
compile against the base `htui-core`. T2's engine calls only existing store reads. T4 adds
`StoreRequest`/`StoreReply` variants. A compile probe found only two exhaustive matches over them,
both in `store_worker.rs` (`StoreRequest::name` at `:586`, `try_serve` at `:1030`) and both T4's,
and none over `StoreReply` (verified, claim 45). T3 and T4 both compile `crates/htui`, which is why
each runs in its own worktree. **Runtime coupling:** after T2, walks in suites outside T2's file
list run the excerpt pass over `FakeIsolator` trees that do not exist. Those suites are
`run_worker.rs`, `chat.rs:959`, `runs_pg.rs:214` and the orch conformance suite. Each of those steps
gains a "could not be listed" note, and no snapshot renders it. T2's `crates/htui` gate covers this.

**Conditions from the independence check (binding on the main thread and the implementers):**
1. **T2 adds no `WriteStore` implementor.** A test-only wrapper in `engine.rs` would stop compiling
   once T0's method, which has no default body, merges. The engine tests use `MemStore` and the
   existing `upsert_repo_box_path`.
2. **Merge order T0, then T1, then T2.** After each merge, re-run the gates of every crate the
   merged task touched on the merged tree, with `--test-threads=1`. After T2 that means the
   `htui-core`, `htui-agent`, `htui-orch` and `htui` gates, plus `cargo clippy -p htui-orch
   --all-features --all-targets -- -D warnings`. In Wave 2, merge T3 and T4 one at a time and re-run
   `cargo test -p htui --all-features -- --test-threads=1` and `ls crates/htui/tests/snapshots | wc
   -l` (88) on the merged tree, not in either lane.
3. **The main thread updates the `HANDOFF.md:39-42` pins after Wave 2.** They are: store `CASES`
   77, `.sqlx` 268, `StoreRequest`/`StoreReply` 69/40 and snapshots 88. No script or test checks
   them, and no implementer touches `HANDOFF.md`.

Every implementer prompt carries: PRD D0–D7 win over this plan where they disagree; read the tree,
not `graphify-out/` (it does not exist); a refusal or a skipped repo is a note a human can read,
never a silent skip; no raw remote URL, credential or excerpt content in a log line, a note or a
report; **commit incrementally** (uncommitted subagent work does not survive the session, and there
is no stash on a shared tree), staging your own paths only; verify your gate with
`--test-threads=1` on the real tree after your merge.

### Task 0: the insert-if-absent writer (D104, D105)
- **Files**: as tabled.
- **Tests first.** D105's case `infer_repo_box_path_inserts_only_where_absent`, appended to `CASES`
  with its `run_case` arm; pins 76 → 77 (`mem_store.rs:37`, `pg_conformance.rs:19`). The long
  message of the `mem_store.rs` assert (from `:38`) lists every milestone's cases, so it gains a
  MOD-7 milestone 4 clause naming the new case. `HANDOFF.md:42` also records "store CASES 76"; that
  doc pin belongs to the main thread (Tasks, condition 3), not to T0. The first commit is red with
  `todo!()` bodies only in the new methods.
- **Action**: D104 in the trait, `MemStore`, `PgStore`, `Writer`, `UsageSpy`, `SpyStore`. `cargo
  sqlx prepare` against a scratch database migrated through `0007` (the compose `htui` database is
  empty; project memory).
- **Validate**: `cargo test -p htui-core --all-features -- --test-threads=1`; `USERNAME=htui-ci
  HTUI_TEST_DATABASE_URL=… cargo test -p htui-store --test pg_conformance --all-features --
  --test-threads=1`; `cargo check --workspace --all-features --all-targets`; `cargo sqlx prepare
  --check`; `ls crates/htui-store/.sqlx | wc -l` (268).

### Task 1: discovery and matching (D111, D112, D113, D123)
- **Files**: as tabled.
- **Tests first.** Unit tests in `infer.rs`: `normalise_remote` over the equivalence set of D111 and
  its negatives (another host, another path, an empty string, a credential never surviving in the
  key); `choose` over one remote match, two remote matches (ambiguous, no name fallback), zero remote
  matches with one name match, a name match with a contradicting remote (rejected, OQ-31), a name
  match on a repo with no `remote_url`, two name matches, an excluded already-held path. Filesystem
  cases in `tests/infer.rs` over `tempfile` trees with `git init` and a written remote: one, two and
  zero checkouts; a checkout at depth 3 found and at depth 4 not; a nested checkout inside a
  checkout not listed; a symlinked checkout not followed; a dot-directory skipped; a `gitdir:` file
  checkout listed; the order deterministic; the directory cap setting `truncated`.
- **Action**: the module and `pub mod infer;`. The fact-check's `gix` 0.87.1 probe (claim 19)
  gives three notes. (a) `repo.find_remote(name.as_ref())` does not compile (E0283), so annotate
  it: `let n: &gix::bstr::BStr = name.as_ref(); repo.find_remote(n)`. (b) `gix::open` works on a
  repository with an unborn `HEAD`. (c) `remote.url(Direction::Fetch)` already lowercases the host
  but keeps `user:tok@`, so D111's normaliser must strip the userinfo itself, and a test pins that.
  No new `gix` feature is needed.
- **Validate**: `cargo test -p htui-orch --all-features -- --test-threads=1`; `cargo clippy -p
  htui-orch --all-features --all-targets -- -D warnings`.

### Task 2: the excerpt pass in the engine (D106–D109, D118, D119, D121, D122, D125)
- **Files**: as tabled.
- **Tests first.** `prompt/mod.rs` unit tests: `excerpt_residual` answers `target − estimated_after`
  for a spec that fits and `0` for one already trimmed, and passes an `AssembleError` through.
  `tests/excerpt.rs`: `excerpt_pass` over a tempdir with a `touched_paths` file selects it,
  `provider_set` starts `builtin@1`, a `NoPath` root is recorded and scanned for nothing. `engine.rs`
  unit tests: `excerpt_roots` resolves the three rungs and a missing name (pure);
  `a_phase_prompt_reads_excerpts_from_the_step_tree` (the `FakeIsolator` rooted at a tempdir with
  `htui/src/lib.rs`, an item with `touched_paths = ["src/lib.rs"]`: the assembled text holds
  `<file path="htui:src/lib.rs"`, `trim.excerpts.roots` is `htui run_step_tree`);
  `a_template_without_excerpts_runs_no_pass` (a `verdict` body: `roots` recorded, `considered 0`);
  `a_scrubber_refused_excerpt_is_dropped_with_a_note` (a file containing `PRIVATE KEY` is dropped,
  the step is not refused, the drop note names `repo:path`, and `audit.selected` still counts the
  file while `audit.files` does not; OQ-30, D119); `a_bare_glob_without_a_primary_selects_nothing`
  (D119's primary rule, in a project with no `is_primary` repo);
  `a_fan_out_group_reads_repo_box_path` (D108, if the harness can set a `repo_box_path` row for its
  box, otherwise covered by `excerpt_roots`); **`a_handoff_spec_carries_no_excerpts_and_runs_no_pass`**
  (D125, claim 13). This test uses the same fixture as the step-tree test: a tree that holds
  `htui/src/lib.rs`, and `touched_paths = ["src/lib.rs"]`. It calls `phase_spec(.., false)`, the
  `opening` path, and asserts that `spec.excerpts == no_excerpts(caps)`: no files, no roots,
  `considered 0`. It then asserts that `promote::handoff_spec(spec, ..).excerpts` is still that
  value. Finally it drives `opening` to its `Handoff` path and asserts the assembled text holds no
  `<file path=`. The same fixture through `assemble_prompt` does select the file, which shows that
  only the phase-prompt caller runs the pass.
- **Action**: D118 and D119's functions. D125: `phase_spec` keeps `no_excerpts(caps)`, and the new
  `Engine::with_excerpts`, called from `assemble_prompt` only, resolves roots, measures the
  residual, runs the pass under `spawn_blocking`, applies the scrub filter and sets `excerpts`. Also
  the judge's comment (D109), D121's two engine comments and the fake's root override. **Add no
  `WriteStore` implementor** (Tasks, condition 1): the engine tests use `MemStore` plus
  `upsert_repo_box_path`.
- **Validate**: `cargo test -p htui-core --all-features -- --test-threads=1`; `cargo test -p
  htui-agent --all-features -- --test-threads=1`; `cargo test -p htui-orch --all-features --
  --test-threads=1` (the whole conformance suite still green: fake roots do not exist, so each adds
  one "could not be listed" note and no file); `cargo test -p htui --all-features --
  --test-threads=1` (covers the untouched `run_worker.rs`, `chat.rs` and `runs_pg.rs` walks, which
  have repos in scope).

### Task 3: the preview (D120, D121)
- **Files**: as tabled.
- **Tests first.** `prompt_preview.rs`: the roots assertion at `:143` becomes "no repo in the demo
  project, so `roots` is empty" (the demo seeds no repo); a new case creates a repo and this box's
  `repo_box_path` at a tempdir holding a `touched_paths` file, and the preview's text carries its
  `<file` block with `roots` = `repo repo_box_path`; a repo with no path records `no_path`. The
  frame asserts `frame.contains("no_path")` at `prompt_preview.rs:119-122`
  (`the_prompt_sub_tab_previews_feat_1`) and `:481` (the ANA-2 100x30 case) are rewritten to the
  new empty-roots copy (`this project has no repo`); the `:138-141` assert (no `<section
  name="excerpts">`) still holds for the demo. The `preview.rs` unit test
  `the_empty_audit_registers_the_builtin_and_records_the_caps` (`:479-488`, `empty_excerpts` at
  `:486`, "plan D110" at `:488`) is rewritten for what `empty_excerpts` becomes, or deleted with it.
  Snapshot updates reviewed with `cargo insta review`: only the `roots` line (`:15`) and the
  reworded note move (in `prompt_preview__preview_feat_1.snap` that is the clipped notes line
  `:36`, so the note's tail does not reach the frame), and the digest line (`:11`) does not (the demo
  prompts select no file). `backlog__detail_prompt.snap` is produced by `tests/backlog.rs`, which
  T3 does not edit; only the snapshot moves.
- **Action**: D120; D121's preview comments and `tests/settings.rs:794`.
- **Validate**: `cargo test -p htui --all-features -- --test-threads=1`.

### Task 4: inference served and shown (D114–D117, D124)
- **Files**: as tabled.
- **Tests first.** `tests/hierarchy.rs` serve cases over the demo `Backend::Memory` with a tempdir
  root: one checkout by remote writes one canonical row and reports `Inferred { by: Remote }`; a
  manual row is left and reported `AlreadySet`; two clones report `Ambiguous { candidates: 2 }` and
  write nothing; no checkout reports `NoMatch`; a symlinked workspace root yields a path under its
  target, never through the link; no root reports `root: None`; offline answers `Unreachable`.
  **One Postgres case in a new `crates/htui/tests/hierarchy_pg.rs`** (amended at fact-check).
  `tests/hierarchy.rs` has no Postgres case today; every case there runs on `Backend::memory` or
  `Backend::Offline`. A separate `*_pg.rs` file keeps it that way and matches the other `crates/htui`
  Postgres suites. The case is `inference_writes_one_row_and_keeps_a_manual_one_on_postgres`, through
  `serve` over `PgStore`. It follows `box_probe_pg.rs:29-31`: `htui_store::testkit::{fresh_db |
  demo_db}`, print `testkit::SKIP` and return when `HTUI_TEST_DATABASE_URL` is unset, and panic when
  `CI` is set. The names tests `hierarchy_names_are_stable` (`:92`) and
  `offline_refuses_every_hierarchy_request_by_name` (`:667`, via `hierarchy_requests()`) gain the
  thirteenth name. Section cases: `i` sends `InferRepoPaths(scope)`; a `set_workspace_root` reply is
  followed by one `InferRepoPaths`; a `RepoPathsInferred` reply renders the tree and the notice;
  snapshots `hierarchy__demo` (hint) and new `hierarchy__inferred`.
- **Action**: D114, D116, D117. Reword the "twelve" in `serve`'s doc (`hierarchy.rs:165-167`) and
  in the `try_serve` comment (`store_worker.rs:1073`) so the count is no longer stated, or state
  it as thirteen.
- **Validate**: `cargo test -p htui --all-features -- --test-threads=1`; `USERNAME=htui-ci
  HTUI_TEST_DATABASE_URL=… cargo test -p htui --test hierarchy_pg --all-features --
  --test-threads=1` (must run the case, not print `SKIP`).

## Test plan

TDD per repo convention: every task's first commit is its failing tests, named in the task. **The
first red tests of the milestone** are T0's `infer_repo_box_path_inserts_only_where_absent`, T1's
`normalise_remote` equivalence test and T2's `a_phase_prompt_reads_excerpts_from_the_step_tree`, one
per Wave 1 lane.

**PRD metrics, mapped.** Path inference (two, one, zero candidates, symlinked root): T1's filesystem
cases and T4's serve cases. Manual wins: T0's conformance case over both stores, T4's `AlreadySet`
case on MemStore and Postgres. Excerpts non-empty: T2's engine test and T3's preview test. UI never
blocks: the walk and the pass run under `spawn_blocking` on the store worker and the agent runtime,
and no section gains a store handle (reviewer check).

**Digest change (PRD risk).** Every production phase prompt whose template places `{{excerpts}}` and
whose scope has a readable root now carries excerpt blocks, so its `prompt_digest` differs from
before. No test pins a production digest value: the `htui-orch` and `crates/htui` run tests assert
`prompt_digest` `Some`/`None` or equal to itself (`crates/htui-orch/src/conformance.rs:1146`,
`:3074`, `:4971`; `crates/htui/tests/runs_pg.rs:603`, `:745`); the `htui-core` golden and digest
tests build their specs directly. The preview snapshots show a digest prefix
(`backlog__detail_prompt.snap:11`, `prompt_preview__preview_ana_2.snap:11`,
`prompt_preview__preview_feat_1.snap:11`), which does not move because the demo project has no repo
row. Verified at fact-check (claims 40-42). `render::excerpts` (`render.rs:474`) returns `None`
when there are no files, so `roots` and notes never reach the digest. A handoff opening's digest
does not move either (D125).

**Count pins that move:**

| Pin | Now | After | Where |
|---|---|---|---|
| Store `CASES` | 76 | 77 (T0) | `crates/htui-core/src/store/conformance.rs`; `crates/htui-core/tests/mem_store.rs:37`; `crates/htui-store/tests/pg_conformance.rs:19`; doc pin `HANDOFF.md:42` (main thread) |
| `htui-orch` `CASES` | 72 | 72 | — |
| `.sqlx` files | 267 | 268 (T0) | `crates/htui-store/.sqlx/` |
| `StoreRequest` / `StoreReply` | 68 / 39 | 69 / 40 (T4) | `crates/htui/src/store_worker.rs` |
| Hierarchy `REQUEST_NAMES` | 12 | 13 (T4) | `crates/htui/src/hierarchy.rs:456` |
| `WriteStore` methods | n | n + 1 (T0) | `crates/htui-core/src/store/traits.rs` |
| `GraphSource` methods | 7 | 7 | `crates/htui-orch/src/graph.rs` |
| Migrations | `0001`..`0007` | unchanged; next still `0008` (OQ-26 default) | `crates/htui-store/migrations/` |
| `crates/htui/tests/snapshots` | 87 | 88 (T4 adds one; T3 and T4 update four) | — |

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| **R-44** — An excerpt trips the fail-closed scrubber and blocks items (`PRIVATE KEY` in `scrub.rs` itself) | High without D119's filter | OQ-30's default drops the file with a note; T2's test pins it |
| **R-45** — The inference walk stalls the store worker on a large or cold tree | Medium | `MAX_DEPTH` 3, `MAX_DIRS` 5 000, `spawn_blocking`; a truncated scan infers nothing and says so |
| **R-46** — Inference writes a wrong path (fork, second clone, same name) | Medium | Remote first; ambiguity writes nothing; OQ-31's contradiction rule; the path is shown in the section and `b` corrects it |
| **R-47** — Every production phase prompt digest changes (handoff digests do not, D125) | High | Expected (PRD risk row); digests are recorded per step and no test pins a value (verified, claim 42) |
| **R-48** — A fan-out group's excerpts come from the managed checkout, which may differ from the candidates' fresh trees (uncommitted edits) | Medium | D108's note; OQ-32 |
| **R-49** — `spawn_blocking` inside engine tests that run with paused time lets auto-advance fire while the pass runs | Very low (downgraded at fact-check) | The fact-check found the original citation wrong: `recover.rs:561`… are heartbeat tests that never assemble a prompt. The paused tests that walk steps are in `engine.rs` (13, `:8000-11084`) and `run_worker.rs` (2, `:3778`, `:4388`). A probe on tokio 1.53.1 (the `Cargo.lock` version) showed the paused clock does not auto-advance while a `spawn_blocking` task runs. Kept only as a watch item: the pass is skipped when every root is `NoPath` or the template places no `{{excerpts}}`, and T2's gate runs the whole suite |
| **R-50** — New rows move the isolator's repo map, so a `StartRun` while a walk is live is refused with `REPOS_MOVED` (`run_worker.rs:894-896`) | Low | Existing behaviour for `SetRepoPath`; the refusal names itself; runs after the walk ends pick the rows up |
| **R-51** — A remote URL with a token leaks through a note, a log or the report | Low | D111: only normalised keys (no userinfo) are compared; the report carries names and outcomes, never URLs; reviewer grep |
| **R-52** — `gix` 0.87.1 with the workspace's feature set cannot enumerate remotes | Retired at fact-check | A compile probe with the workspace features enumerated both remotes and their fetch URLs (claim 19); T1's action carries its three API notes |
| **R-53** — Deviations the main thread must record: the judge clause (D109), the fan-out root (D108), no manual marker (D110), the manual box in Hierarchy (D115), the scrub filter (D119) | Medium | Listed under "Where the PRD, HANDOFF or tree disagree" |

## Validation

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-features --all-targets -- -D warnings
USERNAME=htui-ci HTUI_TEST_DATABASE_URL=postgres://postgres:htui@localhost:5439/postgres \
  cargo test --workspace --all-features -- --test-threads=1
# the prepare check needs a scratch database migrated through 0007 (the compose `htui` database
# is empty; project memory):
docker exec htui-postgres psql -U postgres -c "CREATE DATABASE htui_prepare_check;"   # once
cd crates/htui-store && \
  DATABASE_URL=postgres://postgres:htui@localhost:5439/htui_prepare_check sqlx migrate run --source migrations && \
  DATABASE_URL=postgres://postgres:htui@localhost:5439/htui_prepare_check \
  cargo sqlx prepare --check -- --all-targets --all-features
ls crates/htui-store/.sqlx | wc -l                      # 268
ls crates/htui/tests/snapshots | wc -l                  # 88
cargo doc --workspace --no-deps --keep-going            # exactly the six baseline errors (HANDOFF.md:43-47)
git diff --stat 9151eaa -- crates/htui-store/migrations crates/htui-store/cache_migrations \
  crates/htui-core/src/fixtures.rs crates/htui-orch/src/graph.rs   # empty
```

`--test-threads=1` is not optional (the keyring fake is process-wide). Before believing a Postgres
failure, run `df -h /` (the dev Postgres crash-loops under disk pressure) and re-run the case alone.

**Live check on this box (optional; after Wave 2 merges).** Launch `htui` against the dev database.
Settings > Hierarchy: set the workspace root (`b` on the workspace row) to the directory holding this
checkout; the section re-reads and the `htui` repo row shows the canonical path; `i` reports it
`already set`. Start a run of an item with `touched_paths` in this repo; the Runs pane's step detail
shows `roots htui run_step_tree` and a non-zero `selected`. The Backlog Prompt sub-tab shows
`roots htui repo_box_path` and `<file` blocks.

## Acceptance

- [ ] For every repo of the workspace with no row on this box, a single checkout under this box's
      workspace root matched by normalised remote URL, or else by name, gets a canonical row;
      ambiguous, absent, truncated or refused repos get none and are named in the section's notice.
- [ ] No existing `repo_box_path` row is ever replaced by inference, on either store.
- [ ] A phase prompt whose template places `{{excerpts}}` carries excerpts read from the step's
      tree, else this box's `repo_box_path`, and `trim.excerpts.roots` names the rung per repo,
      `no_path` included.
- [ ] A handoff opening's spec carries no excerpts and runs no pass (D125).
- [ ] The preview reads the same way from `repo_box_path` and shows the same `roots`.
- [ ] A file the scrubber refuses is dropped with a note and never blocks the item (OQ-30 default).
- [ ] The walk and the pass never run on the UI task; no section holds a store handle.
- [ ] Store `CASES` 77 in all three places; `.sqlx` regenerated (268) and `prepare --check` clean;
      no migration; the workspace gate above is green.

## Where the PRD, HANDOFF or tree disagree

1. **The PRD's "phase and judge prompts" carry excerpts**, but ANA-5 §4.1's judge placeholder set
   excludes `{{excerpts}}` (`template.rs:198-202`), so a judge prompt cannot carry one. D109 keeps
   `no_excerpts` for the judge.
2. **ANA-5 §4.5 assumes every candidate tree exists at stage 3**; `drive_group` assembles before
   preparing candidates (`engine.rs:3444-3477`; `run_candidate` -> `candidate_live`,
   `upsert_step_tree` at `:3711`; amended at fact-check). OQ-32, D108.
3. **The PRD's "the section offers a manual path text box"** and MOD-49's "repo paths in the box
   section" vs the tree: the manual box already exists in Settings > Hierarchy (`b`). OQ-28, D115.
4. **The PRD's constraint "Next migration is `0005`"** is obsolete; `0005`–`0007` have landed and the
   next is `0008`. This milestone needs none under OQ-26's default.
5. **The PRD's Evidence line numbers have drifted**: `no_excerpts` is called at `engine.rs:5031` and
   `:4402` (PRD `:4311`, `:4931`); the "no writer" comments are at `:5028-5030` and `:5750-5754`
   (PRD `:4928`, `:5615`); `empty_excerpts` is `preview.rs:271`/`:301` (PRD `:266-285`).
6. **The PRD's record correction "the stale strip comment in `tests/settings.rs`"** was done in
   milestone 2 (`tests/settings.rs:945`); another stale comment remains at `:794`, fixed in T3.
7. **MOD-15's decision record says "every edit is a compare-and-set on `updated_at`"**, but
   `upsert_repo_box_path` and `upsert_workspace_box_path` are plain upserts
   (`pg/write.rs:1908-1921`). Informational; this milestone adds a CAS writer and leaves the manual
   one as it is.
8. **ANA-5 §4.5 (`:1013`) and MOD-2's record say `repo_box_path` has no reader and no writer**;
   `run_worker::repo_map` reads it and `SetRepoPath` writes it. Docs are the main thread's.
9. **ANA-5 open 8** (a scrubber-tripping excerpt) is decided here by OQ-30's default, not in ANA-5.
10. **Tier 2 of §4.5 ("MOD-4 supplies it")** was never wired: `changed_paths` stays empty (D122).
    Candidate follow-up for the main thread.

---

## Claims to verify

Every checkable fact this plan asserts, for the fact-check pass. Line numbers are at `9151eaa`.
Kept as the draft stated them; the verdicts and corrections are under "Verified claims" and in the
plan body.

1. `HEAD` is `9151eaa`; `graphify-out/` does not exist.
2. `crates/htui-store/.sqlx/` holds 267 files; `crates/htui/tests/snapshots` holds 87.
3. The migrations are `0001`..`0007`; `HANDOFF.md:31-34` says the next is `0008`.
4. `repo_box_path` is `(repo_id, box_id, local_path, updated_at)`, primary key `(repo_id, box_id)`, no source column (`0001_init.sql:202-208`); `workspace_box_path` is keyed `(workspace_id, box_id)` with `root_path` (`:173-179`); `repo` has `name`, nullable `remote_url`, `is_primary`, `UNIQUE (project_id, name)` (`:185-196`).
5. `repo_box_path` is not mirrored by the SQLite cache.
6. `StoreRequest::SetRepoPath` is served at `crates/htui/src/hierarchy.rs:303-321` through `canonical` (`:415-430`, `spawn_blocking` over `canonical_root`) and `writer.upsert_repo_box_path`.
7. `canonical_root` (`crates/htui-core/src/root_path.rs:40-61`) refuses a relative, missing, dangling or non-directory path and returns the canonical target of a link.
8. `WriteStore::upsert_repo_box_path` is at `traits.rs:604`, `repo_box_paths` at `:610`, `repos` at `:597`, `workspace_box_paths` at `:543`, `workspace_projects` at `:529`; `ReadStore::step_trees` at `:161`.
9. `State::upsert_repo_box_path` (`mem.rs:2292-2316`) replaces an existing row; `PgStore::upsert_repo_box_path` (`pg/write.rs:1908-1921`) is `ON CONFLICT … DO UPDATE`.
10. `WriteStore` has exactly five implementors: `MemStore` (`mem.rs:5255`), `PgStore` (`pg/write.rs:587`), `Writer` (`writer.rs:350`), `UsageSpy` (`htui-agent/src/conformance.rs:709`), `SpyStore` (`htui-agent/tests/recorder.rs:389`); `htui-orch` and `crates/htui` implement none.
11. The store `CASES` count is 76, pinned at `mem_store.rs:36-37` and `pg_conformance.rs:19` (`EXPECTED_CASES`) and nowhere else; `repo_round_trip_and_primary_flag` is at `conformance.rs:2872`.
12. `Engine` is `Engine<'a, S, G, I, V, C, A, K>` with `S: WriteStore` (`engine.rs:457-468`) and `parts.scrubber: &'a dyn Scrubber` (`:414`); `parts.box_id` and `parts.app` exist.
13. `phase_spec` is at `engine.rs:4926-5049`, calls `no_excerpts(caps)` at `:5031` under the "no writer" comment at `:5028-5030`, and receives `run`, `step`, `phase` and `item`.
14. `judge_prompts` is at `engine.rs:4269` and calls `no_excerpts(caps)` at `:4402`.
15. `no_excerpts` is at `engine.rs:5755` with its "no writer" doc at `:5750-5754`.
16. `walk_live_step` (`engine.rs:3091`) writes `upsert_step_tree` at `:3110` before `assemble_prompt` at `:3125`.
17. `drive_group` (`engine.rs:3408`) calls `assemble_prompt` at `:3446` before `run_candidate` (`:3477`), and `run_candidate`'s tree write is `upsert_step_tree` at `:3711`.
18. `htui-orch` depends on `htui-agent`, `gix` 0.87.1 (features `sha1`, `max-performance-safe`, `parallel`, `index`, `status`), `walkdir`, `tokio` with `rt`, and dev-depends on `tempfile` (`crates/htui-orch/Cargo.toml`, `Cargo.toml:110-118`).
19. `gix` 0.87.1 with that feature set exposes `Repository::remote_names` and `find_remote(..)` with a fetch URL (R-52).
20. `FakeIsolator` roots trees at `FAKE_TREE_ROOT = "/fake/trees"` (`fake.rs:47`, `:431`, `:445`), which does not exist.
21. `htui_core::prompt::excerpt::select` (`excerpt.rs:1050-1299`) has no production caller; the only non-test `select` call in `htui-core` is the skills `select` in `prompt/mod.rs:841`.
22. `htui_agent::excerpt::run_providers` (`crates/htui-agent/src/excerpt.rs:711`) and `FsRepoReader` have no production caller.
23. `select` records a `NoPath` root without listing and notes "no readable root"; a root whose listing fails is recorded with its source and the note "could not be listed" (`excerpt.rs:1085-1113`).
24. `RepoRoot { repo, root, source }` (`excerpt.rs:115-122`), `RootSource::{RunStepTree, RepoBoxPath, NoPath}` (`:160-167`), `ExcerptRequest` (`:391-414`) with `budget_tokens`, `caps`, `scan_cap`, `deadline`, `OwnedExcerptRequest` (`:422-445`) with `as_request`.
25. `PathPrefix::parse(touched, primary_repo)` (`excerpt.rs:68-88`) qualifies a bare glob with the primary repo.
26. `ExcerptSet` is `{ files, audit, notes }` (`excerpt.rs:1463-1472`) and `assemble` copies its notes into `trim_record.notes` and rebuilds `audit.files` from the surviving files (`prompt/mod.rs:924-933`).
27. `TrimRecord` has `target` and `estimated_after` (`crates/htui-core/src/prompt/trim.rs:183-211`).
28. Nothing in the tree computes `ExcerptRequest::budget_tokens` from a spec.
29. `prompt::parse(role, body)` and `ParsedTemplate { spans, used }` are public (`template.rs:256-263`, `:304`; re-exported at `prompt/mod.rs:46`).
30. `Placeholder::allowed_in(TemplateRole::Judge)` admits only `ItemKey`, `Phase`, `Skills`, `Task`, `Candidates` (`template.rs:188-212`).
31. Seven of the eight default phase bodies place `{{excerpts}}` and `verdict` does not; the `judge` and `handoff` bodies do not (`defaults.rs:29-212`).
32. `scrubbed_inputs` masks every excerpt's `repo`, `path` and `content` and a residue fails `assemble` with `AssembleError::Unmasked` (`prompt/mod.rs:706-851`, `:903-922`).
33. `MinimalScrubber` refuses any text containing `PRIVATE KEY` (`scrub.rs:43`, `:242`), and `crates/htui-core/src/scrub.rs` itself contains that string.
34. `DiffBlock` is `{ range, stat, diff }` with no repo-qualified path list (`prompt/mod.rs:159-166`).
35. `preview::build` is at `preview.rs:151-289`, sets `excerpts: empty_excerpts(caps)` at `:271`; `empty_excerpts` is at `:301` with its doc at `:292-300`; `EXCERPTS_NOTE` at `:104-107` is in `STAND_INS` (8 entries).
36. The preview runs on the agent runtime as deferred work, not on the UI task (`preview.rs:15-20`, `run_preview` at `:366`).
37. `Backend::writer()` answers `Some` on `Memory` and `Online` and `None` offline (`backend.rs:153-159`); `Backend::repo_paths(box)` exists (`:532`); there is no `Backend::repos`.
38. `prompt_preview.rs:143` asserts `trim.excerpts.roots.is_empty()` with the D110 message.
39. `excerpt_lines` renders `none · no_path (no run_step_tree, no repo_box_path)` for empty roots (`crates/htui/src/ui/tabs/backlog/detail/prompt.rs:234-262`, `:245`).
40. The demo fixture seeds no `repo` and no `repo_box_path` row (`mem.rs:8959-8964` asserts `repo_paths(ids::BOX)` empty; `conformance.rs:975-976` says it seeds no repo).
41. The three preview snapshots show the digest line and the `roots` line (`backlog__detail_prompt.snap:11`, `:15`; `prompt_preview__preview_ana_2.snap:11`, `:15`; `prompt_preview__preview_feat_1.snap:11`, `:15`).
42. No test pins a production `prompt_digest` value; the run tests assert `Some`, `None` or self-equality (`htui-orch/src/conformance.rs:1146`, `:3074`, `:4971`; `crates/htui/tests/runs_pg.rs:603`, `:745`).
43. No `htui-orch` or `crates/htui` test asserts a production step's `trim_record.notes` by equality or its `excerpts.roots` empty, other than `prompt_preview.rs:143` (`a_forward_with_nothing_to_render_degrades_to_trim_notes`, `engine.rs:6823`, uses `contains`).
44. `hierarchy::serve` is at `hierarchy.rs:169`, `snapshot` at `:113-163`, `box_id` at `:398`, `REQUEST_NAMES: [&str; 12]` at `:456`; `try_serve` routes the twelve hierarchy requests at `store_worker.rs:1073-1089`; `StoreRequest::name` has `SetRepoPath` at `:627`.
45. Every exhaustive `match` over `StoreRequest` or `StoreReply` is in `store_worker.rs` or `hierarchy.rs`; sections match `StoreReply` with a wildcard.
46. `StoreRequest` has 68 variants and `StoreReply` 39 (HANDOFF pins), and no test pins those counts.
47. Settings > Hierarchy: `HINT_BROWSE` at `settings/hierarchy.rs:46`, the `notice` field (`:225`) rendered at `:357-370`, repo rows print the path or `unset` (`:311-326`), `on_key` at `:1040` lists the global keys (`q`, `?`, digits, `ctrl-c`, `-`) and the tab's (`h`, `l`, `[`, `]`, arrows), and `i` is unbound in the section.
48. `hierarchy__demo.snap` shows `HINT_BROWSE` (`:32`); the hint with `· i infer` added is at most 98 columns.
49. `crates/htui/tests/hierarchy.rs` has `hierarchy_names_are_stable` (`:92`), `a_root_is_stored_canonical_and_a_link_is_refused` (`:333`) and at least one Postgres-backed case.
50. `run_worker::repo_map` (`run_worker.rs:1130-1159`) reads `Backend::repo_paths` and every workspace's projects' repos; `Shared::singletons` (`:859-905`) re-reads it on `StartRun` and refuses with `REPOS_MOVED` when it moved and a walk is live.
51. `isolate/real.rs:32-38` is `no_checkout_for_repo()`'s "no checkout for this repo on this box".
52. `crates/htui/tests/settings.rs:945` already reads "61 of the 100 columns" and `:794` still says "before MOD-7 and MOD-15 land".
53. `recover.rs` has tests with `#[tokio::test(start_paused = true)]` (`:561` onward).
54. `HANDOFF.md` records exactly six `cargo doc` baseline errors (`:43-47`).
55. T0 ∩ T1 = T0 ∩ T2 = T1 ∩ T2 = ∅ and T3 ∩ T4 = ∅ by the file lists above.
56. T1 and T2 compile against the base `htui-core` (neither names `infer_repo_box_path`); T4 compiles once T0 and T1 are merged; T3 once T2 is merged.

---

## Verified claims

Filled by the fact-check pass, 2026-09-26.

Tally: **56 claims: 46 verified, 10 amended, 0 falsified** (amended: 11, 13, 17, 26, 35, 39, 40, 43,
44, 49; claims 19, 25 and 53 were verified with notes that changed T1, D119 and R-49). Task
independence: the wave plan holds.

| Claim | Verdict | Evidence |
|---|---|---|
| 1 | verified | `git rev-parse --short HEAD` = `9151eaa`; `graphify-out/` absent |
| 2 | verified | `.sqlx` 267 tracked `query-*.json`; snapshots 87 tracked `*.snap` |
| 3 | verified | `0001_init`..`0007_skill_attachments`; `HANDOFF.md:34` says next `0008` (cache `0005`) |
| 4 | verified | `0001_init.sql:202-208`, `:173-179`, `:185-195` plus `uq_repo_primary` `:196`; no later migration alters them |
| 5 | verified | `cache/mod.rs:44` `MIRRORED_TABLES` has `repo`, not `repo_box_path`/`workspace_box_path` |
| 6 | verified | arm `hierarchy.rs:303` (ends `:322`); `canonical` `:415-429` (doc from `:406`), refusal → `Constraint` |
| 7 | verified | `root_path.rs:40-61`: Relative, Missing, Dangling, NotADirectory, then `Ok(canonical)` |
| 8 | verified | `traits.rs` `:161` (ReadStore), `:529`, `:543`, `:597`, `:604`, `:610` (WriteStore, `:259`) |
| 9 | verified | `mem.rs:2292-2316` replaces or pushes; `pg/write.rs:1908-1921` `ON CONFLICT … DO UPDATE`; `updated_at DEFAULT now()` (`0001_init.sql:206`) suits D104's insert |
| 10 | verified | grep `WriteStore\s+for`: exactly the five; no default bodies (`traits.rs:259-1318`); forwards at `writer.rs:599`, `conformance.rs:882`, `recorder.rs:577` |
| 11 | amended: pattern cite `:2872` → `:2778`; message clause and HANDOFF pin added | 76 counted (`conformance.rs:43-120`); pins `mem_store.rs:37` and `pg_conformance.rs:19` only; `repo_round_trip_and_primary_flag` at `:2778` (CASES `:72`, arm `:171`); doc pin `HANDOFF.md:42`; the `mem_store.rs` message needs a milestone 4 clause (T0) |
| 12 | verified | `engine.rs:457-470` (struct closes `:470`; amended in D106); `scrubber` `:414`, `app` `:417`, `box_id` `:421` |
| 13 | amended: D125 added; D107 scope join; T2 handoff test | lines correct; `phase_spec` is also called by `opening` (`:1270`, `strict=false`) and `promote::handoff_spec` (`promote.rs:81-106`) keeps `excerpts` via `..phase`; `run.repo_scope` is `Vec<RepoId>` (`model/run.rs:200`) |
| 14 | verified | `engine.rs:4269` `judge_prompts`; `:4402` `excerpts: no_excerpts(caps)` |
| 15 | verified | doc `engine.rs:5750-5754`; fn `:5755` |
| 16 | verified | `engine.rs:3091`; prepare `:3101-3104`; `upsert_step_tree` `:3110`; `assemble_prompt` `:3123-3125` |
| 17 | amended: `run_candidate` → `candidate_live` (OQ-32, disagreement 2) | `drive_group` `:3408`, `assemble_prompt` `:3446`, `run_candidate` `:3477`; the `:3711` write is in `candidate_live` (`:3669`), called by `run_candidate` (`:3626`) at `:3640` |
| 18 | verified | `htui-orch/Cargo.toml`; root `Cargo.toml:110`, `:117-118`; `Cargo.lock:1948-1949` gix 0.87.1 |
| 19 | verified (T1 notes added; R-52 retired) | compile probe printed both remotes' fetch URLs; `find_remote` needs a `&BStr` annotation; unborn HEAD opens; `url(Fetch)` keeps `user:tok@` |
| 20 | verified | `fake.rs:47` const, `:431` cwd, `:445` path; `/fake` absent |
| 21 | verified | `excerpt.rs:1050-1299`; every other `select(` call in tests (`:1474` on); only non-test htui-core `select` is `prompt/mod.rs:841` |
| 22 | verified | `htui-agent/src/excerpt.rs:711`, `:196`, `:219`; elsewhere only docs and the `lib.rs:148` re-export |
| 23 | verified | `excerpt.rs:1089-1099` NoPath note; `:1101-1113` 'could not be listed'; `FsRepoReader::walk` `:238-239` maps a failed `read_dir` to `Err` (D107 cite now `:1101-1113`) |
| 24 | verified | `excerpt.rs:115-122`, `:160-167`, `:391-414`, `:422-445`; `from_request` `:450`, `as_request` `:468` |
| 25 | verified (D119 primary rule amended) | `excerpt.rs:68-88`; D119 now uses `overlap.rs:43-54`'s rule (`is_primary` else `""`), not 'first scope repo' |
| 26 | amended: cite → `surviving_audit` `:924-976` (called `:510`), `notes()` `:978-984`; audit counters decided | `ExcerptSet` `excerpt.rs:1463-1472` correct; `surviving_audit` rebuilds only `files`, so a dropped file stays in `selected`; D119 accepts that (`ExcerptAudit` doc, `excerpt.rs:313`) |
| 27 | verified | `trim.rs:183-211`: `target` `:195`, `estimated_after` `:201`; set by `trim::record` `:1021-1052` |
| 28 | verified | `budget_tokens` appears only in definitions, conversions, `select` reads and test literals |
| 29 | verified | `template.rs:256-262`, `:304`; `prompt/mod.rs:46` re-export |
| 30 | verified | `template.rs:188`, Judge arm `:198-203`; fn ends `:215` (D109 cite now `:188-215`) |
| 31 | verified | 7 bodies place `{{excerpts}}`; absent from VERDICT (`:126-137`), JUDGE, HANDOFF (D109 cite now `:126-137`) |
| 32 | verified | `prompt/mod.rs:706-851` (masks repo, path, content, provider `:764-772`); `scrub_text` `:903-922`; called `:453` |
| 33 | verified | `scrub.rs:43` `PEM_MARKER`; `residue_rule` `:236-246`; string also at `:35-36`, `:42`, `:331`, `:447`, `:459-462` |
| 34 | verified | `prompt/mod.rs:159-166` `DiffBlock { range, stat, diff }` |
| 35 | amended: `build` `:151-291`; second `empty_excerpts` caller (test `:486`) added to T3 | `:271` call; doc `:292-300`, fn `:301-314`; `EXCERPTS_NOTE` `:104-107`; `STAND_INS: [&str; 8]` `:61` |
| 36 | verified | `preview.rs:15-20` doc; `run_preview` `:366`; `agent_worker.rs:1241` spawns it, `Served::Deferred` |
| 37 | verified | `backend.rs:153-159`; `repo_paths` `:532`; one `impl Backend` (`:73`), no `fn repos` |
| 38 | verified | `prompt_preview.rs:142-145` in `the_preview_assembles_from_real_store_reads` |
| 39 | amended: span `:235-264`; asserts `:119-122`, `:481` and feat_1 notes line `:36` added to T3 | literal at `prompt.rs:245`; the `no_path` frame asserts break with D120's copy; the feat_1 notes line is clipped |
| 40 | amended: cite `conformance.rs:9975`, not `:975-976` | `mem.rs:8959-8964`; `State::new` empties both (`mem.rs:242-243`); fixture creates no repo |
| 41 | verified | all three: digest `:11`, roots `:15`; `backlog__detail_prompt.snap` comes from `tests/backlog.rs` |
| 42 | verified | orch `conformance.rs:1146`, `:3074`, `:4971`; `runs_pg.rs:603`, `:745`; also `chat.rs:1267`, `agent_worker.rs:4357`; `render::excerpts` `render.rs:474` → `None` with no files |
| 43 | amended: second exception `preview.rs:488` (T3) | `the_empty_audit_registers_the_builtin_and_records_the_caps` (`:479`) asserts `roots` empty; other notes asserts use `contains`/`any`; `prompt_preview.rs:138-141` still holds |
| 44 | amended: `snapshot` `:113-153`; T4 rewords 'twelve' | `serve` `:169`, `box_id` `:398`, `canonical` `:415`, `reread` `:361`, `REQUEST_NAMES` `:456`; `try_serve` comment `:1073`, arms `:1078-1089`; name `:627`; `serve` doc `:165-167` says twelve |
| 45 | verified | compile probe: only E0004s at `store_worker.rs:586` (`name`) and `:1030` (`try_serve`); no exhaustive `StoreReply` match; `hierarchy::serve` has a catch-all (`:351`) |
| 46 | verified | 68 and 39 by script (`store_worker.rs:95`, `:668`); `HANDOFF.md:39`; name tests `tests/hierarchy.rs:92`, `:667` (T4 extends both) |
| 47 | verified | `HINT_BROWSE` `:46`, `notice` `:225`, `hint()` `:359-383`, repo rows `:310-327`, `on_key` `:1040` (comment `:1049-1051`); `i` bound only in `chat/mod.rs:509`, `settings/agents.rs:1140` |
| 48 | verified | `hierarchy__demo.snap:32`; 86 → 96 chars in a 98-column interior |
| 49 | amended: no Postgres case exists; T4 adds `tests/hierarchy_pg.rs` | `:92`, `:333` exist; every case is MemStore or Offline; no `HTUI_TEST_DATABASE_URL`/`testkit`; conventions from `box_probe_pg.rs:29-31` |
| 50 | verified | `run_worker.rs:1130-1159`; `singletons` `:859` (re-read `:886-892`, `REPOS_MOVED` `:894-896`) |
| 51 | verified | `isolate/real.rs:32-38` `no_checkout_for_repo()` |
| 52 | verified | `tests/settings.rs:794` and `:945` read as claimed |
| 53 | verified (R-49 downgraded) | 10 paused heartbeat tests in `recover.rs`; the walking ones are `engine.rs` (13) and `run_worker.rs` (2); tokio 1.53.1 does not auto-advance during `spawn_blocking` |
| 54 | verified | `HANDOFF.md:43-47` six entries; `cargo doc` at `9151eaa` emitted exactly those six |
| 55 | verified | file lists pairwise disjoint (see independence row) |
| 56 | verified | the five `impl WriteStore` blocks are outside htui-orch and crates/htui; `infer_repo_box_path` absent; `resolve_excerpt_caps` `settings.rs:732` |
| Task independence | holds, with three conditions | T0, T1, T2 pairwise disjoint; T3, T4 disjoint. `.sqlx`, store `CASES` → T0 only; `StoreRequest`/`StoreReply` and their two exhaustive matches → T4 only; snapshot names disjoint. Missing file: `htui-agent/src/lib.rs` (T2, optional). Conditions 1-3 under Tasks |
