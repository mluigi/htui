# Plan: MOD-4 milestone 3 — work happens in a real tree

**Source**: `.claude/prds/mod-4-orchestrator-manual-mode.prd.md`, milestone 3 (`:307`). Design
authority: the PRD's D1–D8 (cited as **PRD Dn**; this plan's own decisions are plain **Dn**,
milestone 1's are **M1 Dn** and milestone 2's **M2 Dn**), `docs/ANA-2.md` §2 invariants 4 and 6,
§4.2 (verification), §4.6, §4.7 (what this milestone must leave alone), §4.9 (what milestone 5 will
read out of this milestone's rows), §8, §9 build step 5, §10.4/§10.7, §11 risks 2 and 3, §12
criteria 11, 12 and 13; `docs/ANA-5.md` §4.6's forwarded set; `docs/ANA-10.md:1116` and
`:2207-2210`; `docs/REQUIREMENTS.md` `R-ORCH-7`/`R-ORCH-8`/`R-ORCH-9`/`R-ORCH-11`.

**Requirements**: `R-ORCH-8` (all four modes), `R-ORCH-11` (the commit hashes), `R-ID-4` (no
`htui` file inside a managed repository), `R-ENT-3` (one tree per repo of a multi-repo project);
`R-ORCH-9` by *not* touching it (D44); `R-NF-3` by construction (no UI); `R-SEC-3` on the verify
output (D30).

**Complexity**: Large (a git library enters the workspace; four isolation modes over it; four
`git` CLI verbs behind a process runner, because `gix` ships no worktree mutation (OQ-1, resolved);
a second process runner for `verify_command`; two seam writers with their conformance leg; three of
ANA-2's validation criteria, all of which need a real repository on disk).

**Routing**: routed as **PRD** by `/handoff-run MOD-4`; the PRD and its milestone table exist, so
this milestone enters the chain at `plan`. Ultracode was recommended for the implement phase and
**the maintainer scoped it to implement only**, as in milestones 1 and 2. Reviewer: `rust-reviewer`
(`.claude/workflow-config.json:2`). Milestone 1's plan predicted the fan-out would first pay off
here (`mod-4-orch-seam.plan.md:137`): this plan has **two parallel pairs** (T1 ∥ T2, then T3 ∥ T4).

**Numbering**: milestone 2's table runs to **D21**, not D20 — D21 is the blueprint's H-8 amendment
(`mod-4-orch-engine.plan.md:72`) — so this plan starts at **D22**.

**Status**: draft, not yet fact-checked against a blueprint. Branch `mod-4-m2-close-out`; nothing
is committed by this plan.

**Maintainer decisions (2026-09-22)**: **OQ-1 resolved to the alternative** — the maintainer's
words were "use git worktrees, if gix does that use that, don't create new code from scratch";
`gix` 0.87.1 demonstrably does not, so `isolate/git.rs` shells out to the `git` CLI for exactly
`worktree add --lock`, `worktree remove`, `merge --no-ff` and `reset --hard`, and keeps `gix` for
every read and every ref write. D22, D23, D25, D35, D38, D39, D40, D42, T2, the mode table, the
Files table, Validation, Risks, Acceptance and the ledgers are amended below where the decision
touches them; everything else stands as drafted. **OQ-2, OQ-3, OQ-4 and OQ-5 are accepted at their
adopted defaults** (D31, D26, D32, D25's range copy). The ANA-2 amendment the decision requires
(`docs/ANA-2.md:1777` and `:2055`) is the main thread's to write, not this plan's.

---

## Open questions for the maintainer (read these first)

Each has a default this plan adopts so implementation is not blocked; each is a place where the
analysis and the library disagree, or where the library's surface could not be verified. **OQ-1
was answered against the default and the plan is amended for it; OQ-2 to OQ-5 stand at their
defaults** (see "Maintainer decisions" above).

- [x] **OQ-1 — `gix` 0.87.1 has no `git worktree add`, `remove` or `prune`, and no
      "checkout this tree into an existing working directory".** Verified by reading the crate
      source, not docs: `Repository::worktrees()`, `worktree_proxy_by_id`, `main_repo`, `worktree`,
      `is_bare`, `worktree_stream`, `worktree_archive` are the *whole* worktree surface
      (`gix-0.87.1/src/repository/worktree.rs:46-160`); `worktree::Proxy` is read-only
      (`src/worktree/proxy.rs:48-103`); `Repository::checkout_options` is the only `checkout`
      symbol on the repository (`src/repository/checkout.rs:8`), and the only checkout *operation*
      in the crate is the clone path's `gix_worktree_state::checkout` over a fresh, empty directory
      (`src/clone/checkout.rs:113-146`). ANA-2 §4.6 writes every row of its mode table in `git`
      CLI verbs (`docs/ANA-2.md:914-916`) and §8 rejects shelling out in as many words
      (`:1777`); §10.7 picked `gix` over `git2` (`:2055`) and PRD D2 adopted it (`:339`).
      **Resolved by the maintainer, 2026-09-22: the alternative.** The maintainer's words — "use
      git worktrees, if gix does that use that, don't create new code from scratch" — and the
      evidence above resolve to: shell out to the `git` CLI for exactly `worktree add --lock`,
      `worktree remove`, `merge --no-ff` and `reset --hard`, behind the same `isolate/git.rs`
      seam, keeping `gix` for every read (`head_id`, `is_dirty`, `submodules`, `worktrees`,
      `open_index`, object reads, OQ-5's `rev_walk` range copy) and every ref write (D26's label
      with `reference(..., PreviousValue::MustNotExist, ...)`, and `commit_as` where the tests
      mint history). The evidence paragraph is now the *justification*: there is nothing in `gix`
      to use, so the choice was between writing ~350 lines that re-implement four `git` verbs and
      spawning the `git` that already implements them. Three consequences, none absorbed silently:
      (1) **ANA-2 §8 `:1777` is contradicted** ("Shelling out to `git` is the third option and is
      rejected") and §10.7 `:2055` restates it ("shelling out to `git` is rejected in both cases");
      both need an amendment, which the main thread writes (the text it must carry is in the
      close-out report, not here); (2) **a `git` binary becomes a runtime dependency** of the
      `worktree` mode and of every non-identity `reconcile`, and a **test dependency** of T2, T5
      and T6's git-backed cases (D40); `local` and `shared_serialized` stay `gix`-only; (3)
      **MOD-16's Windows surface grows**: `git.exe` on `PATH`, its `core.autocrlf` and filter
      behaviour, and `CREATE_NO_WINDOW` on a fourth spawn site (PRD `:296`). **Considered and not
      taken: `git2`/libgit2.** ANA-2 §10.7 rejected it (`:2055`, "pure-Rust build with no
      `libgit2` or OpenSSL linkage on Windows"), it pulls a C build into every `cargo build`, and
      it would leave two git libraries in the workspace beside `gix`, which the reads still need.
      The former default (hand-built layout, restricted-index checkout) survives only where the
      decision does not reach: D35's reset of a dirty *copy* is still `gix`'s full-index checkout,
      the one remaining consumer of `worktree-mutation` (D22, D35).
- [ ] **OQ-2 — Where the verify command's output lives.** ANA-2 §4.4 and ANA-5 both say the
      review loop forwards "the stored `command_run.output` of the previous attempt"
      (`docs/ANA-2.md:730`, `docs/ANA-5.md:335`), and `command_run` exists with an `output TEXT
      -- scrubbed` column (`crates/htui-store/migrations/0001_init.sql:537-551`) — but **no seam
      method writes or reads it**: `traits.rs` names `command_run` only in the `Counts` struct
      (`crates/htui-core/src/store/traits.rs:1322-1327`), `MemStore` counts it as a constant `0`
      (`crates/htui-core/src/store/mem.rs:2836`), and no `CommandRun` model type exists. ANA-2 §8
      assigns `command_run` to MOD-11 (`docs/ANA-2.md:1704`), yet §4.2 has MOD-4 running the
      verify command directly until MOD-11 lands (`:504-506`). **Default adopted (D31):** this
      milestone adds `record_command_run` and `command_runs(step)` to `WriteStore` — the twentieth
      and twenty-first methods, with the read on `WriteStore` by M1 D1's shipped precedent for
      `repos`/`phases` — so an `unavailable` verify leaves a readable reason (invariant 7) and the
      loop's forwarded set has its durable home. **Alternative:** keep only the two existing
      `run_step` columns (`verify_outcome`, `verify_exit_code`) and let MOD-11 add the row later;
      cheaper by one conformance case and six impl arms, but a failed verify would leave *no
      output anywhere* until then, and D32 could never be unwound.
- [ ] **OQ-3 — `shared_serialized` does not switch the managed tree's `HEAD` to
      `htui/<step_id>`.** ANA-2 `:916` says siblings commit "to `htui/<step_id>`" and reconciliation
      is "the winner's branch is checked out", which reads as a branch switch at step start. A
      switch needs the *previous* branch name to switch back at cleanup and after a crash, and no
      column carries it (`run_step_tree` is `mode, path, base_ref, dirty`, `0003_orchestration.sql:92-100`; PRD
      `:272` forbids a `0004`). **Default adopted (D26):** the agent commits on whatever branch the
      tree has checked out; `htui/<step_id>` is created at capture as a *label* on `after_hash`,
      which keeps "the losers' branches remain" (`:916`) true for milestone 4 and makes criterion
      13's "no `htui/` branch checked out anywhere" (`:2121`) hold by construction. The deviation
      is recorded rather than silently made.
- [ ] **OQ-4 — `previous_diff` and `verify_failure` are not wired into the loop prompt here.**
      `engine.rs:1045-1048` says "Both are milestone 3's". This plan moves the *prompt* half to
      milestone 4 (D32) and keeps the *durability* half here. If the maintainer wants the sections
      rendered in milestone 3, T6 grows a unified-diff renderer and the two `PromptSpec` fields;
      the renderer's library (`gix` `blob-diff` versus `similar`, `Cargo.toml:65`) is then a further
      choice. `similar` is **not** idle, contrary to this plan's first draft: `htui-agent` already
      depends on it (`crates/htui-agent/Cargo.toml:35`) and builds ACP's `edit_proposal.diff` with
      it (`crates/htui-agent/src/acp/fs.rs:106-118`), so the root manifest's "no crate consumes it
      yet" comment is stale and should be corrected whichever way OQ-4 is answered.
- [ ] **OQ-5 — The `copy` mode's "fetch from the copy" is an object walk, not a fetch.** ANA-2
      `:981` reconciles a copy with `git fetch <copy_path>`. `gix`'s fetch needs
      `blocking-network-client` and, for a local path, spawns `git-upload-pack`
      (`gix-0.87.1/Cargo.toml:111-117` pulls `gix-transport`, the decisive `"dep:gix-transport"` at `:114`), which is shelling out by another
      name. **Default adopted (D25):** copy the commit range `before_hash..after_hash` from the
      copy's object store into the primary's with `rev_walk` + `find_object` + `write_object`
      (`src/repository/revision.rs:174`, `object.rs:55`, `:250`). Bounded by the step's own
      commits; no transport.

---

## Summary

Milestone 2 made a graph walk against an `Isolator` that invents paths and hashes. This milestone
gives that seam its production implementation and stops every walk from lying about the
filesystem: `GixIsolator` in `crates/htui-orch/src/isolate/real.rs` implements the four `R-ORCH-8`
modes over `gix` 0.87.1, writes the `run_step_tree` row per repo that milestone 1's
`upsert_step_tree` already persists (`crates/htui-core/src/store/traits.rs:841-848`), reports the
`before_hash` that `record_commits` already stores (`:850-857`), captures `after_hash` at stage 5,
reconciles the single winner back into the primary tree, and removes what it created when the run
is terminal. `verify.rs` runs `step_graph_phase.verify_command` in the primary repo's tree between
the session and the gate and lands ANA-2's three outcomes in the two columns milestone 1 added
(`crates/htui-core/src/model/run.rs:254-258`). The engine gains the three call sites it is missing
today — `reconcile` and `cleanup` have **no production caller** (grep over `engine.rs` finds
`prepare` at `:794` and `capture` at `:842` only) — and a `CancelRun` command so criterion 13 is
reachable. Nothing here touches admission (D44), nothing adds a column (PRD `:272`), and no `.snap`
moves.

Two things cross back into the seam (T1): `command_run` gets a writer and a reader (D31, OQ-2),
and `upsert_step_tree` starts writing `run_step.isolation_path`, which nothing writes today (D33).
Both are the milestone-1 shape — case first, both stores, refusing `BufferedWriter` arm, `.sqlx`
regenerated — and both are parallel with the crate work because their file sets are disjoint.

The single largest risk was **library surface**: `gix` 0.87.1 does not implement `git worktree
add` (OQ-1). The maintainer resolved it by spawning `git` for the four verbs `gix` lacks (the
"Maintainer decisions" note), so the risk moves from "our layout diverges from `git`'s" to "`git`'s
behaviour is inherited, and its binary and version are ours to check" (Risks). The plan is honest
about what it verified by reading the crate source, what it verified by compile probe, what it
verified by running `git` 2.43.0 on this box, and what it could not verify at all (the last
section).

## Design decisions (settled here, not in code review)

| # | Decision | Why |
|---|---|---|
| D22 | **`gix = "0.87.1"`, `default-features = false`, features `["sha1", "max-performance-safe", "parallel", "index", "status", "worktree-mutation"]`, declared once in `[workspace.dependencies]` and consumed only by `htui-orch`.** Every `gix` call runs under `tokio::task::spawn_blocking` inside a sync function in `isolate/git.rs` that takes owned `PathBuf`s and returns owned values; no `gix::Repository` is ever held across an `.await`. The four `git` verbs of OQ-1's resolution run as `tokio::process` children through the same file's `Cli` (T2) and are the **only** subprocesses under `isolate/`. | 0.87.1 is the current crates.io release (`cargo search gix`, 2026-09-22; `rust-version: 1.85`, under the workspace's 1.98 pin, `rust-toolchain.toml:2`). The feature list was **recomputed for OQ-1's resolution (2026-09-22)** by naming the call that needs each feature: `sha1` is **required** and is a default feature that vanishes under `default-features = false` (`gix-0.87.1/Cargo.toml:132-139`, `:228-233`) — the first probe without it failed inside `gix-hash` with `E0004: type &Kind is non-empty`; `parallel` is what makes `Repository: Send` at all (`src/types.rs:155-158`); `status` gates `is_dirty` (`src/lib.rs:492-493`, `src/status/mod.rs:168`) — D24, D25's guards, D27, D29 — and its closure pulls `dirwalk`, `index`, `blob-diff`, hence `attributes` and `excludes` (`Cargo.toml:241-247`, `:140-149`, `:79-82`, `:60-68`), which is what gates `submodules()` for the `worktree` refusal (`src/repository/mod.rs:62-63`); `index` gates `open_index` (`src/repository/mod.rs:43-44`, `src/repository/index.rs:25`), which D25 reads for the conflict path list after a failed `git merge`, and `State::from_tree`/`File::from_state` for D35's reset; `worktree-mutation` gates the `gix_worktree_state` re-export (`:259-262`, `src/worktree/mod.rs:8`) and its **only** remaining consumer is D35's full-index reset of a dirty copy — if the maintainer extends the shell-out to `git reset --hard` (ANA-2 `:915`'s own verb) it and `index`'s second use go too; `max-performance-safe` (`:183`, `= max-control`) is the crate's default object-cache profile and gates no call this plan names — kept so OQ-5's `copy_range`, which reads every object of a step's range, runs with the pack cache the defaults assume; it is the one entry not tied to a call. **Removed:** `merge` — freed by D25's move from `merge_commits`/`tree_merge_options`/`merge_base` to `git merge --no-ff`; `blob-diff` — freed by D25's removal of `diff_tree_to_tree` (the restricted-index working-tree update is `git merge`'s now), still present transitively through `status`; `dirwalk` — never a named call, only `status`'s dependency, still present transitively; `revision` — freed by D25's removal of `merge_base`, and OQ-5's `rev_walk` does **not** need it (`src/repository/revision.rs:174` and `src/revision/mod.rs:9` `pub mod walk;` carry no `cfg`; the `revision` gates at `revision.rs:2-160` cover `rev_parse`, `merge_base*` and `describe`). Net effect on the compiled closure: `gix-merge` and `gix-revision`'s `describe`/`merge_base` leave; nothing enters. **Not re-probed by compile in this revision** — the earlier probe compiled a superset; T2's first commit runs `cargo tree -p htui-orch -i gix --edges features` and is the check. `Repository` is `Send` and **not `Sync`** (`types.rs:148`), and `IsolatorFuture` is `Send` by its alias (`crates/htui-orch/src/isolate.rs:32`), so the only shape that compiles and stays honest is blocking work handed owned inputs. `htui-core` and `htui-store` gain no `gix` edge (ANA-2 §8's dependency-weight argument, `docs/ANA-2.md:1646-1652`). |
| D23 | **The `worktree` mode is one `git` invocation, run in the managed checkout (`current_dir = local_path`): `git worktree add --lock --reason "htui run <run_id>" -b htui/<step_id> <path> <before_hash>`.** Flag spelling and argument order are `git worktree --help`'s synopsis on this box (`git worktree add [-f] [--detach] [--checkout] [--lock [--reason <string>]] [--orphan] [(-b \| -B) <new-branch>] <path> [<commit-ish>]`, git 2.43.0) and were exercised end to end (fact ledger). `-b` creates `refs/heads/htui/<step_id>` at `<before_hash>`, so the former step (1) — `Repository::reference(..., PreviousValue::MustNotExist, ...)` — **goes away for this mode** (it survives only as D26's label for `shared_serialized`); `--lock --reason` writes the `locked` file with `htui run <run_id>` (ANA-2 `:914`'s `--lock`); the checkout is `git`'s own. **Nothing is parsed from its output.** Success is exit status 0 plus a `gix` post-condition: `gix::open(<path>)?.head_id()? == before_hash` and `Repository::worktrees()` on the main repo (`src/repository/worktree.rs:46`) lists a `Proxy` whose `base()?` (`src/worktree/proxy.rs:48`) is `<path>` and whose `is_locked()` is true; stdout (`Preparing worktree (new branch 'htui/<step_id>')`, `HEAD is now at <short> <subject>`) is discarded and stderr is kept only as error text. **The admin entry's id is not ours to choose**: `git` names `<git_dir>/worktrees/<basename of path>` (`main`, then `main1`, … on collision — verified), so the id is `<repo.name>`-derived, `worktree_proxy_by_id` is not usable, and every lookup is by `base()` path. **Failure modes → `IsolateError`** (each verified on 2.43.0 under `LC_ALL=C`): branch exists → exit 255, `fatal: a branch named 'htui/<step_id>' already exists` — unreachable after D38's idempotence check, and `Git(...)` if reached; path exists and is non-empty → exit 128, `fatal: '<path>' already exists` → `Git`; bad start point → exit 128, `fatal: invalid reference: <hash>` → `Git`; ref lock held → exit 255, `fatal: cannot lock ref 'refs/heads/htui/<step_id>': Unable to create '<git_dir>/refs/heads/htui/<step_id>.lock': File exists.` → D39's classifier, retried; a timeout (T2) → `Git("git worktree add timed out after 120s")`; a missing binary → `Refused("git not on PATH")` before any spawn. Every `Git` payload is `git worktree add: <last non-empty stderr line>`. **Remove** = `git worktree remove --force --force <path>` — one `--force` for a dirty tree, the second for a locked one (`git worktree --help`: "To remove a locked working tree, specify `--force` twice"); it deletes the directory and the admin entry, and it succeeds when the directory is already gone (verified), which is the crash-recovery case; an unknown path is exit 128, `fatal: '<path>' is not a working tree`, which cleanup treats as already removed. **Prune** = `git worktree prune`, run only as a backstop when a `worktrees()` read *after* the removes still lists an entry whose `base()` is under the scratch root; `git` never prunes a locked entry (`git worktree --help`, `lock`: "prevent its administrative files from being pruned"; verified), so ours are prune-immune while their `locked` file stands and the backstop fires only for an entry someone unlocked by hand. The `htui/<step_id>` branch survives `remove` (verified) — the label ANA-2 `:916` wants kept for milestone 4. | OQ-1, resolved. ANA-2's own oracle for criterion 11 — "`git worktree list` in the managed repo shows them" (`docs/ANA-2.md:2117`) — is now true by construction, because the same binary wrote the entry. Prune's scoping to `htui-*` entries is **lost** — `git worktree prune` clears every unlocked stale entry, the user's own included — so `R-ID-4`'s spirit is kept by not calling it unless our own entry outlived its `remove`; that is the inherited-behaviour risk, named under Risks. `git`'s `post-checkout` hook runs on `worktree add` (since 2.17; verified on 2.43.0) — inherited, also under Risks. |
| D24 | **`dirty` means `Repository::is_dirty()`: a change in the index against `HEAD` or in the working tree against the index, submodules included, untracked files excluded.** | It is the one dirty predicate the crate ships and its doc is explicit that untracked files do not count (`gix-0.87.1/src/status/mod.rs:156-168`). The consumer of `dirty` is milestone 5's sweep, whose action is a reset to `before_hash` (`docs/ANA-2.md:1298-1299`), and a reset never deletes untracked files — so an untracked-only tree is exactly as safe to reset as a clean one. A stricter predicate would be a status walk of our own (`status().untracked_files(…)`, `src/status/platform.rs:32`) and is a knob, not a v1 need. |
| D25 | **Reconciliation of one winner is: refuse unless the primary tree is clean *and* still at the step's `before_hash` (both `gix` reads: `is_dirty()`, `head_id()`); then `git merge --no-ff --no-edit -m "htui: reconcile <step_id>" <after_hash>` in the primary tree, under the `htui` signature passed as `GIT_AUTHOR_NAME`/`GIT_AUTHOR_EMAIL`/`GIT_COMMITTER_NAME`/`GIT_COMMITTER_EMAIL`; exit 0 → post-condition via `gix` (`head_commit()?.parent_ids()` are exactly `[before_hash, after_hash]`, `src/object/commit.rs:154`) and the new `HEAD` is recorded as the winner's `after_hash`; exit ≠ 0 → classified (below), and a conflict is refused with the path list.** Where in the walk: one engine helper, `reconcile_done_step`, called after a step reaches `done` — `gate::apply`'s `(OnFailure \| Never, Ok)` arm (`crates/htui-orch/src/gate.rs:383-388`) and `answer_gate`'s `Approved \| Skipped` (`crates/htui-orch/src/engine.rs:376-377`) — and before `Landing::Advance` / `run_to_rest` continues. A refusal parks the **run** (step stays `done`) through a new `park_run(reason)` in `engine.rs`: run `running → awaiting_approval`, item `in_progress → awaiting_approval`, `item_note` naming the path list — the blueprint's H-10 three-write order, with R-5 still carried. `local` is the identity; `shared_serialized` is the identity when the tree's `HEAD` equals the winner's `after_hash` (the only milestone-3 case; D26); `copy` first walks `before..after` from the copy's odb into the primary's with `gix` (OQ-5) so `<after_hash>` resolves in the primary before `git merge` runs. **The conflict case is detected from the exit status, not from `gix`'s merge machinery**: `git merge` exits **1** on a conflict (verified: `CONFLICT (content): Merge conflict in <path>` / `Automatic merge failed; fix conflicts and then commit the result.`) and leaves `MERGE_HEAD` plus stage-1/2/3 index entries behind; `git.rs` then (a) reads the path list with `gix` — `open_index()` (`src/repository/index.rs:25`) entries whose `stage() != Stage::Unconflicted` (`gix-index-0.55.0/src/entry/mod.rs:3-11`) — (b) restores the primary with `git merge --abort` (verified exit 0, tree clean, `MERGE_HEAD` gone), and (c) refuses. Exit 1 is **also** what an `index.lock` collision produces (`error: Unable to write index.` then the same `Automatic merge failed` line, `MERGE_HEAD` written, tree untouched — verified), so the exit-1 branch consults D39's classifier *first*: a lock signature aborts and retries; otherwise it is a conflict. Any other non-zero exit is `IsolateError::Git("git merge: <last stderr line>")` after an abort attempt (`git merge --abort` with no merge in progress is exit 128, `fatal: There is no merge to abort (MERGE_HEAD missing).`, which is swallowed). `Already up to date.` (exit 0, no commit) cannot occur — `after_hash == before_hash` is `None` at capture and reconcile is the identity — and the two-parent post-condition would catch it if it did. | The steps are ANA-2 `:975-988` in order; `:978` names `dirty_primary_tree`, `:983` `merge_conflict` with the path list, `:987-988` the `after_hash` update — **the refusal sentences are unchanged** (`dirty_primary_tree`; `merge_conflict: <path list>`; this plan's `primary_moved: <hash>`). "Still at `before_hash`" is this plan's addition: `:948-953` branches every phase from the primary's *current* HEAD so later phases see earlier commits, and a primary that moved under a step is a three-way merge whose working-tree update no longer has a known base; parking with `primary_moved: <hash>` is the honest refusal. OQ-1's resolution makes `git merge` the working-tree update — the restricted-index checkout, `merge_commits`, `tree_merge_options`, `diff_tree_to_tree` and `commit_as` for the merge commit are gone from this row (D22 frees `merge`, `blob-diff`, `revision` accordingly), and with them the unverified populated-directory behaviour and the cold-cache fallback: `git merge` touches only the paths it changes. The identity goes through the environment rather than `-c user.name=…` because a managed repo may carry no `user.*` and the env form was verified to commit as `htui <htui@localhost>` with `HOME=/nonexistent` and no config at all. `--no-edit -m` is what keeps `git` from opening an editor; always `--no-ff` even when fast-forwardable, per `:980` (verified: a descendant merged with `--no-ff` yields a two-parent commit). |
| D26 | **`shared_serialized` and `local` run the agent on the branch the managed tree already has checked out. `shared_serialized` creates `htui/<step_id>` as a label on `after_hash` at capture; `local` creates no branch at all.** `before_hash` is `HEAD` in both, `dirty` is D24's predicate in both, and the tree row's `path` is `repo_box_path.local_path`. | OQ-3 records the deviation from `:916`'s "committing to `htui/<step_id>`". The label keeps every property the analysis needs from the branch — the losers' branches remain for milestone 4 (`:916`), the step's range is `before_hash..after_hash` (`:970-973`), criterion 13's "no `htui/` branch checked out" (`:2121`) — without a persisted "branch to restore" that no column can carry. `local` gets no label because invariant 4 says the two managed-tree modes "run the agent there rather than writing `htui`'s own files" (`docs/ANA-2.md:119-123`) and `:917` says `local` has "nothing to reconcile"; `:916` sanctions the branch for `shared_serialized` explicitly and nothing sanctions it for `local`. |
| D27 | **A `worktree` that produced no commit is removed at step end only when it is also not dirty.** `capture` checks `after_hash == before_hash` *and* `is_dirty() == false` before removing; a dirty no-commit worktree is kept until the run is terminal like every other tree. | ANA-2 `:997-999` adopts Claude Code's step-end removal of a no-commit worktree as "a special case", and `:2065` (risk 2) names the failure that rule invites — "an auto-cleanup then destroyed the uncommitted work". An agent that edited without committing is the *common* failed step, and its edits are the only artefact a human can recover a retry from. Narrowing the special case to genuinely untouched trees keeps `:997`'s intent (no litter) and `:2065`'s mitigation (never destroy work) at once. |
| D28 | **The scratch root is `identity::config_root()/trees`, validated against every repo path at construction and at every `prepare`.** Tree path = `<root>/<run_id>/<step_id>/<repo.name>/`; `cwd` for `worktree`/`copy` is `<root>/<run_id>/<step_id>/` (the common parent) and for `shared_serialized`/`local` the primary repo's checkout; every other tree goes in a new `Prepared.extra_dirs: Vec<PathBuf>`, which the engine hands to `SessionSpec.extra_dirs` (today hard-coded `Vec::new()`, `crates/htui-orch/src/engine.rs:1097`). A root inside any repo, or a repo inside the root, is `IsolateError::Refused`. | ANA-2 `:905` fixes `<config_dir>/trees/<run_id>/<step_id>/<repo_slug>/` and invariant 4 fixes the validation (`:119-123`); `config_root()` is the shipped `<dirs::config_dir()>/htui` (`crates/htui-store/src/identity.rs:36-52`). `Prepared.cwd` is already documented as "their common parent" for a two-repo run (`crates/htui-orch/src/isolate.rs:69-73`); `extra_dirs` is the field `SessionSpec` reserves for "ACP `additionalDirectories`, `claude --add-dir`" (`crates/htui-agent/src/driver.rs:257-258`), and a second repo that the agent cannot read is a tree it cannot work in. `repo.name` is unique per project (`crates/htui-core/src/model/hierarchy.rs:129-130`), so it is the slug. |
| D29 | **A dirty `shared_serialized` tree is recorded, not refused, in this milestone.** `prepare` writes `dirty = true` with `before_hash = HEAD` and proceeds; the refusal in `:916`'s "Refused when" cell applies to the *reset* a second fan-out sibling needs and is milestone 4's. | §4.6 disagrees with itself: the mode table refuses a dirty `shared_serialized` tree "the user has not accepted the reset" for (`docs/ANA-2.md:916`), while the commit-capture paragraph says a dirty `local` *or* `shared_serialized` step records `dirty = true` and continues (`:963-968`). With one candidate nothing is reset, so the two readings only conflict when a sibling starts, and no acceptance UI exists yet. This is a **§4.6-internal** disagreement, not a third §4.2/§4.3 one; recorded here so milestone 4 reads `:916` as its own rule. |
| D30 | **`verify_command` runs through the platform shell (`sh -c` on Unix, `cmd /C` on Windows) in the tree of the repo that `is_primary`, with the process environment unchanged, stdout and stderr merged and tail-capped at 64 KiB, masked by the engine's `Scrubber` before persistence, under a class semaphore keyed `verify`, and with a timeout equal to the step deadline's remainder when the phase has one and none otherwise.** Spawn shape mirrors `launch.rs` (process group on Unix, job object + `CREATE_NO_WINDOW` on Windows); a timeout kills the group. A scope with no primary-repo tree yields `unavailable` with the reason `no primary tree`. | ANA-2 `:491-493` fixes the tree ("the step's own tree for the project's primary repo") and the environment ("the agent's environment minus the secrets"); the walk's `SessionSpec.env` is empty this milestone (`crates/htui-orch/src/engine.rs:1098-1099`), so the process environment *is* that. The column is documented as `output TEXT -- scrubbed` (`0001_init.sql:546`) and the engine already holds one `Scrubber` for the assembler and recorder (`engine.rs:192-194`). The semaphore is `:504-506`'s "same in-process semaphore", sized from `app.command_limits.verify` (default `1`, `0003:1988`) and owned by a `ShellVerifier` the caller builds once per process — process state, which M2 D16 does not forbid. The shell is a decision because ANA-2 never says how a `TEXT` becomes an argv; `cargo test --workspace` is the shape every seeded template assumes and a shell is the only reading under which quoting works. The deadline is M2 D8's "verify deadline belongs to `verify.rs`" made concrete: `:515` lists "deadline elapsed" among the `unavailable` causes, and the only deadline a phase has is `deadline_seconds` (`model/run.rs:508-510`). Mirror: `crates/htui-agent/src/launch.rs:1110-1160`. |
| D31 | **Two seam methods on `WriteStore`: `record_command_run(NewCommandRun) -> CommandRun` and `command_runs(step) -> Vec<CommandRun>`; `WriteStore` 61 → 63.** `CommandRun` mirrors `0001_init.sql:537-551` field for field with a `CommandRunStatus` `str_enum!`; `MemStore` gains a `command_runs` map; one conformance case `verify_run_is_recorded`; `CASES` 48 → 49; both `BufferedWriter` arms refuse with the one sentence; `UsageSpy` and `SpyStore` gain delegating arms. **No mirror column, no migration.** | OQ-2. The table exists and is unmirrored (`MIRRORED_TABLES` does not list it; ANA-2 `:1704`), so by M1 D1 its read would be `Backend`-inherent and unreachable from a conformance case; M1 D1 also records that `repos`/`phases` sit on `WriteStore` "so its conformance suite could read back what it wrote" (`crates/htui-core/src/store/traits.rs:489`) and that the shipped placement wins. Putting the read beside the write follows that precedent and is also what milestone 4's loop prompt will call (D32). `WriteStore` is 61 `async fn` today (counted over `traits.rs`'s `impl` block; `finish_run` at `:915` was the sixty-first). |
| D32 | **The loop's `verify_failure` and `previous_diff` prompt sections are milestone 4's, not this milestone's.** This milestone guarantees their inputs are durable — `command_run.output` (D31) and `before_hash..after_hash` per repo (`record_commits`) — and leaves `PromptSpec.verify_failure`/`previous_diff` at `None` (`engine.rs:1047-1048`). | OQ-4. The prompt's role block is next opened when the judge prompt lands (`crates/htui-core/src/prompt/mod.rs:104-110`: `verify_failure`, `previous_diff`, `judge`, `handoff` are one field group), and `previous_diff` needs a unified-diff renderer that nothing in this milestone's deliverable otherwise needs. Milestone 3 per the PRD is "the `Isolator` over `gix` … and `verify_command` with its three outcomes" (`:307`); the forwarded set is ANA-5 §4.6's and belongs to whichever milestone next touches `assemble`. The `engine.rs:1045-1048` comment is corrected by T6 to say so. |
| D33 | **`upsert_step_tree` also writes `run_step.isolation_path`**: the `path` of the row whose repo `is_primary`, else the first row's, else the column is left. Both stores; the `trees_and_commits_round_trip` case gains the assertion; `.sqlx` regenerated. | ANA-2 `:903` adopts the tree table on the condition that "`isolation_path` keeps the primary repo's path", and `0003` comments the column the same way (`:1942-1943`). **No writer exists**: both `MemStore` inserts set it `None` (`crates/htui-core/src/store/mem.rs:1564`, `:3354`), `NewRunStep` has no such field (`model/run.rs:384-400`), and the only Postgres mention is a `SELECT` column (`crates/htui-store/src/pg/write.rs:2724`). The step is created before its trees exist, so the tree writer is the only site that knows the path. |
| D34 | **`GixIsolator::new(IsolatorConfig)` is built by the caller with the repo map resolved: `repos: BTreeMap<RepoId, RepoCheckout { name, local_path, is_primary }>`, `scratch_root`, `copy_exclude`, `copy_max_total_bytes`, `box_id`.** A repo in `scope` and absent from the map is `Refused("no checkout for this repo on this box")`. Milestone 6's `run_worker` resolves the map from `repos(project)` and `repo_paths(box)`; this milestone's tests resolve it from `MemStore`. | The two reads the map needs are unreachable from the engine: `repos` is a `WriteStore` method (`traits.rs:489`) but `repo_paths(box)` is inherent on `Backend` (`crates/htui-store/src/backend.rs:529-535`) and `MemStore` (`mem.rs:537-545`) — exactly the shape M2 D19/A-2 handled by passing `app` as a map (`engine.rs:195-197`). The refusal sentence is the one the fake's own test already pins (`engine.rs:2606`, `:2638`). `is_primary` and `name` are `Repo` fields (`hierarchy.rs:130`, `:136`); `local_path` is `RepoBoxPath`'s (`:189`). |
| D35 | **`copy` measures first, refuses above the cap, copies with an exclusion list, refuses a source with no `.git` directory or with a `.git` *file*, and resets a dirty copy to `HEAD` with a full checkout.** `DEFAULT_COPY_EXCLUDE` is ANA-2's seven entries, applied when `ProjectSettings.copy_exclude` is empty; matching is by path component with one trailing `*` allowed; size is measured with `walkdir` over what would be copied; the cap is `app.copy_max_total_bytes` × copies (one, this milestone). | `:919-926` fixes the list and calls it mandatory; `ProjectSettings::default()` has `copy_exclude: Vec::new()` (`crates/htui-core/src/model/kind.rs:292`) and `0003` seeds no `copy_exclude` key (`:1986-1999`), so the default has to be a crate constant. `:930-932` fixes the measured refusal and `:1993` the default cap. "No glob crate" is `:1783`. A `.git` *file* means the source is itself a linked worktree, whose copy would point back at the original's `worktrees/` entry; a missing `.git` is the PRD's open question, adopted as **refused** (`:378-380`). `walkdir` is already a workspace dependency (`Cargo.toml:101`). The reset is `:915`'s `git reset --hard <before_hash>` **composed in `gix`** — `State::from_tree(HEAD)` → `File::from_state` → `gix::worktree::state::checkout` with `overwrite_existing = true` → `index.write` — because OQ-1's resolution scoped the shell-out to four verbs and `reset` is not among them; a fresh copy has no warm cache to protect, so the full-index form is the right one. **After OQ-1 this is the only consumer of `worktree-mutation` and of `index`'s write half (D22), and the only place the probe's unproven "checkout over a populated directory" still matters** (a tracked file the user deleted is restored, a modified one overwritten; a file the user *staged as new* stays in the copy's working tree, which `git reset --hard` would have removed — harmless to a scratch copy the agent then works in). A fifth verb, `git reset --hard <before_hash>` in the copy, would delete this composition and two features; that is a further maintainer call, recorded in the close-out report, not taken here. |
| D36 | **`Isolator::cleanup` is called by the engine after every terminal `finish_run`, through one `Engine::cleanup_run(run)` that gathers every step's `step_trees` and calls the isolator once; cleanup errors are `tracing::warn`ed and never raised.** The same helper is `pub` for milestone 6's UI and is what `CancelRun` (D45) calls. | `cleanup` has **no caller today** (grep: `engine.rs` calls `prepare` at `:794` and `capture` at `:842`, nothing else; `gate.rs` none). The terminal sites are `engine.rs:415`, `:556`, `:724`, `:912` and `gate.rs:542`, `:651`, `:675`, `:688`. The rule is invariant 6 and `:994-1000`: never at step end, never while a step is `awaiting_approval`. Warn-only because the run is already terminal when cleanup runs, a failed `rm` must not un-terminate it, and milestone 5's sweep is the retry. |
| D37 | **A real isolator's failure reaches the operator through milestone 2's `fail_hard`, unchanged.** `prepare` is the first call after `pending → running` (`engine.rs:791-795`); its `Err` escapes `walk_live_step`, `walk_step` catches it (`:767-776`), `fail_hard` moves the step `running → failed` and the run through `finish_run(Failed, Some("isolation refused: …"))` (`:895-915`), the item mirrors, and the error is re-raised as `EngineError::Isolate` (`command.rs:248`). `capture` and `reconcile` errors at stages 5 and 6 take the same path. The operator sees `run.failure` = `isolation refused: <reason>` or `git: <reason>` — `IsolateError`'s own `Display` (`isolate.rs:44`, `:47`) — in the Runs tab at milestone 6. | Nothing to decide; this row exists because the close-out asked the plan to say how the path is reached. The `refuse_prepare` case already pins the sentence end to end (`engine.rs:2599-2646`). Each mode's refusal reasons are named in the mode table below so the sentences are fixed before code. |
| D38 | **`GixIsolator::prepare` is idempotent for `worktree` and `copy` even though the trait promises nothing and the engine calls it once.** If `<root>/<run>/<step>/<repo>` exists with a `.git` entry, `prepare` reuses it and reports `before_hash` = the target of `refs/heads/htui/<step_id>` (not the current `HEAD`). `shared_serialized` and `local` are idempotent by construction. The trait doc gains one sentence saying so; the fake stays non-idempotent (it mints fresh hashes, `fake.rs:98-109`). | The engine threads `cwd` down one call frame precisely because a second `prepare` "is a second set of trees and a second `before_hash`" (`engine.rs:1069-1073`), and `a_walk_prepares_each_step_exactly_once` pins one call (`:2550-2588`). What that does not cover is a crash *between* `prepare` and `upsert_step_tree` (`:791-801`): milestone 5's sweep re-derives from rows (M2 D16; invariant 9, `docs/ANA-2.md:139-142`), finds no tree row, and re-runs the step — whose second `prepare` would then hit `git worktree add -b`'s own branch check (`fatal: a branch named 'htui/<step_id>' already exists`, exit 255 — verified; D23) on an existing branch and strand the step. Idempotence costs one `exists()` and one `gix` ref read (`find_reference("refs/heads/htui/<step_id>")`) and removes a whole class of unrecoverable states; it is also what keeps the branch-exists failure of D23 unreachable in practice. |
| D39 | **Every git write is wrapped in `git::with_retry`: three retries at 200, 400 and 800 ms when the error is classified as a lock error. The classifier has two sources.** (1) `gix`'s own: an `io::ErrorKind::AlreadyExists` from `gix::lock`, or an error whose `Display` contains `.lock` or `cannot lock` (`PermanentlyLocked`, `gix-lock-24.0.0/src/acquire.rs:44-52`) — raised by D26's label write, D35's index write and OQ-5's object writes. (2) The `git` CLI's: a non-zero exit whose captured stderr contains any of `Unable to create '` … `.lock': File exists` (a ref lock on `worktree add`, exit 255: `fatal: cannot lock ref 'refs/heads/htui/<step>': Unable to create '<git_dir>/refs/heads/htui/<step>.lock': File exists.`; an `index.lock` on reset-shaped commands, exit 128: `fatal: Unable to create '<git_dir>/index.lock': File exists.`), `cannot lock ref`, or `Unable to write index` (`git merge` under a held `index.lock`, exit **1** — the conflict status — with `MERGE_HEAD` left behind, so the merge retry runs `git merge --abort` before sleeping; D25). All three wordings were produced on this box with `LC_ALL=C`, which T2's `Cli` pins in the child environment so the text is not localised. Reads are not retried; a timed-out command is not retried. | ANA-2 `:935-942` and PRD `:389`. `gix::lock` is re-exported (`gix-0.87.1/src/lib.rs:141`) and its `acquire::Fail::AfterDurationWithBackoff(Duration)` is a *quadratic* backoff (docs.rs, gix-lock 24.0.0), not the 200/400/800 the analysis fixes — so the schedule is ours and sits above the library. The message-based half of the classifier is a weak point and is listed under Risks; it exists because `gix`'s error enums are per-operation and this plan could not enumerate every lock-bearing variant from the tree, and — after OQ-1 — because a subprocess offers nothing *but* its exit status and text, which is the very objection ANA-2 `:1777` raised ("the failure modes (`index.lock`) are easier to classify through an API") and the maintainer accepted. |
| D40 | **Tests build real repositories with `gix::init` + `commit_as` under `tempfile::TempDir`; every test that exercises the `worktree` mode, a non-identity `reconcile`, or `git.rs`'s `Cli` needs the `git` binary the implementation uses and, on a box without one, prints `skipped: git not on PATH` and passes — one `pub const SKIP_GIT: &str` in `git.rs`'s test support, in the style of `htui-store`'s `SKIP` (`crates/htui-store/src/testkit.rs:35`, `"skipped: HTUI_TEST_DATABASE_URL not set"`). A `git` older than 2.33.0 prints `skipped: git <version> is older than 2.33.0` and passes likewise.** The `git worktree list --porcelain` oracle of criteria 11 and 13 is no longer a when-available nicety — it is the same binary the implementation called, so it runs whenever the test runs. Tests that touch no `git` verb (`head`, `is_dirty`, `copy_range`, `with_retry`'s `gix` half, all of `copy.rs`, all of `verify.rs`, the `local` and `shared_serialized` mode cases, the eighteen fake `CASES`) run on a box without `git`. No Postgres anywhere in the isolator tests; the `command_run` writer's Postgres twin is the only Postgres-gated test and sits behind `HTUI_TEST_DATABASE_URL` like every other. **CI**: there is no CI configuration in this repository (no `.github/`, no `.gitlab-ci.yml`; `ls -a` at the root, 2026-09-22) — `README.md:510` calls `cargo sqlx prepare --check` "the CI form" and the gate runs on the maintainer's box, which has `git version 2.43.0` at `/usr/bin/git`; so the gate as it exists today runs the git-backed tests, not the skips. | `gix::init` (`src/lib.rs:332`) and `commit_as` (`object.rs:361`) are enough to mint a repository with history. `tempfile = "3"` is a dev-dependency in every other crate (`crates/htui-core/Cargo.toml:31`, `htui-agent:70`, `htui-store:45`, `htui:64`). The skip sentence is the README's own convention for a missing server (`README.md:477-480`), so the workspace gate stays green on a box with neither. The skip is decided once per binary by the same `which::which("git")` + `git --version` probe `GixIsolator::new` runs (T2), so a test can never pass on a box where production would refuse. |
| D41 | **The real isolator gets its own integration binary, `crates/htui-orch/tests/gix_isolator.rs`, driving `GixIsolator` through the `Isolator` trait per mode and one end-to-end walk through `Engine` built over `EngineParts` with the real isolator; the fifteen fake cases stay fake.** `verify.rs` defines a `Verifier` trait with `ShellVerifier` (real) and `fake::FakeVerifier` (scripted outcomes), so the fake suite pins the three outcomes' settle interplay without a process. | M2 D18 promised the real isolator could be "pinned against the same cases"; `FakeOrchestrator` holds a concrete `FakeIsolator` field (`fake.rs:608`) and `dispatch_fake` assembles from it (`:821-826`), so binding `CASES` to `GixIsolator` means making the harness generic over its isolator — churn with no criterion behind it. `EngineParts`' fields are all public (`engine.rs:170-206`), so a test can assemble an engine directly. The `Verifier` seam is M2 D6's pattern (a seam now, implementations by kind) applied to the one other side effect this milestone introduces. |
| D42 | **`isolate.rs` keeps the trait and grows a directory: `isolate/git.rs` (the `git` CLI wrapper for the four verbs, the sync `gix` reads and ref writes, retry), `isolate/copy.rs` (walker, size, excludes), `isolate/real.rs` (`GixIsolator`, four modes, the per-`(box, repo)` lock); `verify.rs` is new at the crate root.** `fanout.rs`, `select.rs`, `overlap.rs`, `recover.rs`, `queue.rs` stay uncreated. | ANA-2 §8 names `isolate.rs` for all of "the four modes, scratch root, git ops with backoff, run_step_tree, before/after hash capture, winner reconciliation, cleanup" and `verify.rs` separately (`docs/ANA-2.md:1667-1670`); the crate's `lib.rs` already lists the not-created set (`crates/htui-orch/src/lib.rs:12-13`). Splitting `isolate.rs` into a directory is what lets T3 and T4 touch disjoint files; the public path `htui_orch::isolate::GixIsolator` is unchanged by it. |
| D43 | **`shared_serialized`'s serialization is an in-process `tokio::sync::Mutex<()>` per `(box_id, repo_id)` held inside `GixIsolator` for the whole step — not a Postgres advisory lock.** | ANA-2 `:916` says advisory lock; the engine cannot issue SQL (invariant 10, `:143-146`) and an advisory lock would be a seam method with six impl arms and a `MemStore` semantics to invent. In v1 the contention the lock must serialise is only fan-out siblings inside one run: `claim_run` already refuses a second run whose repo set intersects on the box (M1's R-2, `HANDOFF.md:205-212`), and `lease_owner` stops a second process adopting a live run (`:1277-1282`). Milestone 5 owns admission and may replace the lock behind the same method when R-2 is answered — **not foreclosed** (D44). |
| D44 | **What this milestone must not foreclose (R-2, mirroring M2 D6): no overlap or admission reasoning anywhere in `isolate/*` or `verify.rs`; rule L's "another non-terminal run holds any tree in this repo" refusal for `local` (`:917`, `:1082-1087`) is *not* implemented here; no per-repo `isolated`/`local`/`paths` fact is persisted anywhere new.** `run_step_tree.mode` per repo is the only per-repo isolation fact this milestone writes, and it is ANA-2's own table. | R-2 is milestone 5's by the PRD's table (`:309`) and its option (a) needs a `0004` the PRD forbids (`:272`); M2 D6 kept `claim_run` "exactly as milestone 1 shipped it" (`mod-4-orch-engine.plan.md:60`). Today's predicate — repo-set overlap on one box — *subsumes* rule L, so implementing L here would be dead code that milestone 5 then has to reconcile with §4.7's `RunScope`. `run_step_tree.mode` is precisely what R-2's option (b) will read to evaluate rules I and P outside the critical section. |
| D45 | **`Command::CancelRun { run }` lands now: legal from `queued`, `running` and `awaiting_approval` (`RunStatus::can_move_to`, `model/run.rs:60-68`); every non-terminal step is moved to `cancelled`; `finish_run(Cancelled, None)`; then `cleanup_run` (D36). A `running` step is cancelled only if no session is live, which in this milestone's synchronous walk is always true.** | Criterion 13 is this milestone's and is stated as "cancelling a run" (`docs/ANA-2.md:2120-2121`); `command.rs:1-3` reserves `CancelRun` for "milestones 3 to 6", and 3 is in range. The one seeded run that criterion needs freeing is already cancelled by the fake's own prologue through `finish_run(Cancelled)` (`engine.rs:1668-1680`), so the store side is exercised; what is missing is the command and the cleanup after it. Killing a live session is milestone 6's, where a session can outlive a dispatch. |
| D46 | **`git worktree prune` is never run.** Our entries are created `--lock`ed and `git worktree prune` refuses locked entries by construction, so the verb can only ever act on entries this orchestrator did not create — the maintainer's own stale worktrees. Cleanup therefore runs `git worktree remove --force --force <path>` per tree and, if a `worktrees()` read still lists an entry under the scratch root afterwards, **reports it** in the cleanup error rather than pruning: `stale worktree entry <path>; run \`git worktree prune\` yourself`. | The revision's own finding: prune never touches a locked entry, and it cannot be scoped to a subset. A verb whose only reachable effect is on the user's data is not a cleanup step. `R-ID-4`'s scoping requirement survives by not running the unscopable verb at all, which is cheaper than mitigating it. |
| D47 | **`git reset --hard` is the fifth shelled verb, and D35's dirty-copy reset uses it.** The hand-composed `gix` full-index checkout over a populated directory goes away with it, and with it the `worktree-mutation` feature and `index`'s write half — recompute the D22 list in T2's first commit and drop what no named call needs. | Same maintainer directive as OQ-1: do not re-implement a `git` verb from primitives. The composed reset was the last place the unproven "checkout over a populated directory" behaviour survived (a staged-new file is not removed by it), so this deletes an unverified behaviour rather than adding a dependency — `git` is already required by D23. |

## The four modes, one row each

Every mode: `run_step_tree(run_step_id, repo_id, mode, path, base_ref, dirty)` per repo in scope at
stage 2, `run_step_commit(before_hash, NULL)` beside it, `after_hash` at stage 5
(`docs/ANA-2.md:957-960`). `before_hash` is `head_id()` of the repo the row describes
(`gix-0.87.1/src/repository/reference.rs:211`), taken before the agent starts (`:962`). Refusal
sentences are `IsolateError::Refused`'s payload and are fixed here.

| | `worktree` | `copy` | `shared_serialized` | `local` |
|---|---|---|---|---|
| **Tree** | `<root>/<run>/<step>/<name>/`, branch `htui/<step_id>` at `before_hash`, created and checked out by D23's `git worktree add --lock --reason … -b …`, `locked` | `<root>/<run>/<step>/<name>/`, a filesystem copy including `.git/` minus `copy_exclude`, reset to `HEAD` if the source was dirty (D35) | `repo_box_path.local_path` itself; the per-`(box, repo)` mutex held from `prepare` to `cleanup` (D43) | `repo_box_path.local_path` itself, nothing held |
| **`run_step_tree` row** | `path` = the worktree, `base_ref` = `before_hash`, `dirty = false` | `path` = the copy, `base_ref` = `before_hash`, `dirty = false` | `path` = the checkout, `base_ref` = `HEAD`, `dirty` = D24 | same as `shared_serialized` |
| **`before_hash` / `after_hash`** | primary's `HEAD` at prepare / the worktree's `HEAD` at capture, `None` when equal to `before_hash` | source's `HEAD` / the copy's `HEAD`; after reconcile, the merge commit in the primary (D25) | `HEAD` / `HEAD`; the label `htui/<step_id>` is created on `after_hash` (D26) | `HEAD` / `HEAD`; no label (D26) |
| **Dirty tree** | cannot be at prepare (fresh); at capture a dirty no-commit tree is kept (D27) | source dirtiness is erased by the reset; the row says `false` | recorded `true`, step proceeds (D29); milestone 4's sibling reset is what refuses | recorded `true`, step proceeds (`:963-968`); milestone 5 never resets it (`:1299`) |
| **Cleanup (run terminal)** | `git worktree remove --force --force <path>` per tree; an entry a `worktrees()` read still lists under the scratch root afterwards is **reported, never pruned** (D46) | delete the directory | release the mutex | nothing |
| **Refused when** | the repo has submodules (`submodules()` yields any, `src/repository/submodule.rs:93`) → `submodules: worktree isolation is not supported`; `<name>` collides between two repos of one project → `duplicate repo name`; no usable `git` (D40, T2) → `git not on PATH` or `git <version> is older than 2.33.0` | no `.git` directory → `not a git checkout`; `.git` is a file → `source is a linked worktree`; measured size × copies > cap → `copy would need N bytes; cap is M` | never in this milestone (D29) | never in this milestone (`fan_out > 1` is already refused at snapshot time, `crates/htui-orch/src/graph.rs:425-429`; rule L is milestone 5's, D44) |
| **Any mode** | a repo not in the config map → `no checkout for this repo on this box`; a tree path inside a repo, or a repo inside the root → `scratch root … is inside a managed repository` (invariant 4) | | | |

`fan_out > 1` is milestone 4's; this milestone's `reconcile` sees one winner and `Prepared.trees`
has one entry per repo, never per candidate.

## `verify_command`'s three outcomes, against ANA-2's text

`verify.rs` runs after the session and before the settle, in stage 5 (`docs/ANA-2.md:491`), and
the engine hands its result to `gate::settle` through the `SettleInput.verify_outcome` field that is
hard-coded `None` today (`crates/htui-orch/src/gate.rs:219-220`, `engine.rs:853-855`) and to
`finish_step` through the two `StepOutcome` fields that are hard-coded `None` (`engine.rs:869-870`).

| Situation (`:510-515`) | `verify_outcome` | `verify_exit_code` | `command_run` row (D31) | Settle (M2 D2) |
|---|---|---|---|---|
| phase has no `verify_command` (every seeded phase, `crates/htui-core/src/seed.rs:233`) | `NULL` | `NULL` | none | unaffected — `NULL` is the normal case |
| exit 0 | `pass` | `0` | `status = done, exit_code = 0, output` | unaffected |
| nonzero exit | `fail` | the code | `status = done, exit_code = N, output` | **`failed`** — `gate.rs:256` already tests `Some(Fail)` |
| the command could not run: binary or shell missing, spawn refused, no primary tree, killed by signal with no code, timed out on the step deadline's remainder (D30) | `unavailable` | `NULL` | `status = failed, exit_code = NULL, output = <the reason>` | unaffected — "`unavailable` never fails a step" (`:443`); **a timeout still settles `failed` one line later**, because `settle` compares `now` with `started_at + deadline_seconds` (`gate.rs:212-216`) and the timeout elapsed exactly that deadline |

A *missing command* is the `unavailable` row: the binary named by `verify_command` is not on `PATH`,
the shell reports it, and the reason is what the operator reads in `command_run.output`. There is
no fourth outcome; `VerifyOutcome` is the shipped three-variant enum
(`crates/htui-core/src/model/run.rs:154-164`).

## Patterns to Mirror

| Concern | Mirror | Where |
|---|---|---|
| A seam behind a `dyn`-safe boxed future | `IsolatorFuture`'s alias shape, reused for `Verifier` | `crates/htui-orch/src/isolate.rs:32` |
| Refusal wording owned by the error, not the caller | `IsolateError::Refused(String)` with `Display` `isolation refused: …` | `crates/htui-orch/src/isolate.rs:40-52` |
| Blocking work off the async runtime | `spawn_blocking` with owned inputs; the shape `launch.rs` uses for process I/O | `crates/htui-agent/src/launch.rs:1110-1160` |
| Child process discipline | `process_wrap::tokio::CommandWrap` + `ProcessGroup::leader()` / `JobObject` + `CREATE_NO_WINDOW` | `crates/htui-agent/src/launch.rs:1111-1156` |
| Seam writer | case first, `MemStore` closure, `PgStore` single statement + `map_sqlx`, `Writer` delegates, `BufferedWriter` refuses, `.sqlx` committed with the query | M1 D5/D6, M2 T1 (`mod-4-orch-engine.plan.md:145-154`) |
| Engine hard failure from a live step | `walk_step` → `walk_live_step` → `fail_hard` | `crates/htui-orch/src/engine.rs:744-777`, `:895-915` |
| Caller-resolved config passed as a plain map | `EngineParts.app: BTreeMap<String, Value>` | `crates/htui-orch/src/engine.rs:195-197` |
| Deterministic fake | scripted FIFO, counter, no filesystem | `crates/htui-orch/src/fake.rs:42-215` |
| Skip sentence on a missing external | `skipped: HTUI_TEST_DATABASE_URL not set`, one `pub const SKIP` printed byte for byte | `crates/htui-store/src/testkit.rs:35`; `README.md:477-480` |
| Locating a binary on `PATH` | `which::which(...)` (workspace `which = "8.0.6"`) | `Cargo.toml:66`; `crates/htui-agent/Cargo.toml:32` |
| Not-created modules stated in the crate doc | `lib.rs`'s list | `crates/htui-orch/src/lib.rs:12-13` |

## Files to Change

| File | Action | Task | Why |
|---|---|---|---|
| `crates/htui-core/src/model/run.rs` | edit | T1 | `CommandRun`, `NewCommandRun`, `CommandRunStatus` (D31) |
| `crates/htui-core/src/model/mod.rs` | edit | T1 | re-export the three |
| `crates/htui-core/src/store/traits.rs` | edit | T1 | `record_command_run`, `command_runs` (D31); `upsert_step_tree`'s doc gains the `isolation_path` sentence (D33) |
| `crates/htui-core/src/store/mem.rs` | edit | T1 | `State.command_runs`, the two impls, `upsert_step_tree`'s `isolation_path` write, `counts()` stops hard-coding `0` (`:2836`) |
| `crates/htui-core/src/store/conformance.rs` | edit | T1 | `verify_run_is_recorded`; `trees_and_commits_round_trip` asserts `isolation_path`; `CASES` 48 → 49 |
| `crates/htui-core/tests/mem_store.rs` | edit | T1 | the `CASES` pin (`:36`) and its ledger message |
| `crates/htui-store/src/pg/write.rs` | edit | T1 | the two writers; `upsert_step_tree` gains the `UPDATE run_step SET isolation_path` inside its transaction |
| `crates/htui-store/src/pg/rows.rs` | edit | T1 | the `CommandRun` row, fields appended (M1 D10) |
| `crates/htui-store/.sqlx/query-*.json` | add/regenerate | T1 | offline data for every touched query |
| `crates/htui-store/src/writer.rs` | edit | T1 | `Writer` delegation and `BufferedWriter` refusal for both (M1 D5) |
| `crates/htui-store/tests/writer_buffered.rs` | edit | T1 | the one-sentence assertions |
| `crates/htui-store/tests/pg_conformance.rs` | edit | T1 | `EXPECTED_CASES` 48 → 49 (`:19`) |
| `crates/htui-store/tests/pg_criteria.rs` | edit | T1 | the Postgres twin (column-level: `output` round-trips, `status` `CHECK`) |
| `crates/htui-agent/src/conformance.rs` | edit | T1 | `UsageSpy`'s two delegating arms (its `impl WriteStore` is exhaustive; `:1004` is the `upsert_step_tree` arm) |
| `crates/htui-agent/tests/recorder.rs` | edit | T1 | `SpyStore`'s two arms (`:698-720`) |
| `Cargo.toml` | edit | T2 | `[workspace.dependencies]` gains `gix` (D22); `walkdir`, `process-wrap`, `windows`, `dirs`, `which` are already there (`:51`, `:59-64`, `:66`, `:101`) |
| `crates/htui-orch/Cargo.toml` | edit | T2 | `gix`, `walkdir`, `process-wrap`, `windows` (cfg windows), `dirs`, `which`; dev `tempfile = "3"`; `tokio` features `["sync", "rt", "process", "io-util", "time", "fs"]` |
| `crates/htui-orch/src/isolate.rs` | edit | T2 | `pub mod git;`; `Prepared.extra_dirs` (D28); the idempotence sentence on `prepare` (D38); `pub use real::GixIsolator;` lands in T5 |
| `crates/htui-orch/src/isolate/git.rs` | create | T2 | the `git` CLI wrapper (`Cli`: locate, version-check, spawn, scrub, capture, timeout; `add_worktree`, `remove_worktree`, `prune_worktrees`, `merge_no_ff`, `abort_merge`) plus the sync `gix` reads and ref writes (open, head, dirty, submodules, worktree lookup by path, branch label, conflict paths, odb range copy, D35's full-index reset), and retry over both (D22, D23, D25, D35, D39) |
| `crates/htui-orch/src/fake.rs` | edit | T2 | `extra_dirs: Vec::new()` in `FakeIsolator::prepare` |
| `crates/htui-orch/src/verify.rs` | create | T3 | `Verifier`, `ShellVerifier`, `VerifyRequest`, `VerifyReport`, the semaphore, outcome mapping (D30) |
| `crates/htui-orch/src/lib.rs` | edit | T3 | `pub mod verify;` and its re-exports |
| `crates/htui-orch/src/isolate/copy.rs` | create | T4 | `DEFAULT_COPY_EXCLUDE`, the matcher, `measure`, `copy_tree` (D35) |
| `crates/htui-orch/src/isolate.rs` | edit | T4 | `pub mod copy;` — one line, after T2 |
| `crates/htui-orch/src/isolate/real.rs` | create | T5 | `GixIsolator`, `IsolatorConfig`, `RepoCheckout`, the four modes, the locks (D26, D27, D28, D29, D34, D38, D43) |
| `crates/htui-orch/src/isolate.rs` | edit | T5 | `pub mod real; pub use real::{GixIsolator, IsolatorConfig, RepoCheckout};` |
| `crates/htui-orch/src/lib.rs` | edit | T5 | the crate-root re-export of the three |
| `crates/htui-orch/src/engine.rs` | edit | T6 | `EngineParts.verifier`; stage 5 runs verify and fills `SettleInput`/`StepOutcome`; `reconcile_done_step` + `park_run` (D25); `cleanup_run` at every terminal site (D36); `extra_dirs` into `SessionSpec` (D28); `CancelRun` dispatch (D45); the `:1045-1048` comment (D32) |
| `crates/htui-orch/src/command.rs` | edit | T6 | `Command::CancelRun`, its enabling guard, `CommandOutcome::Cancelled` |
| `crates/htui-orch/src/gate.rs` | edit | T6 | `SettleInput.verify_outcome`'s "always `None`" doc (`:219`) and `StepFailure::VerifyFailed`'s "never produced" doc (`:131`) corrected; no logic change |
| `crates/htui-orch/src/fake.rs` | edit | T6 | `FakeVerifier` (D41); `FakeOrchestrator.verifier`; `dispatch_fake` wires it |
| `crates/htui-orch/src/conformance.rs` | edit | T6 | `verify_fail_settles_failed`, `verify_unavailable_never_fails`, `cancel_cleans_up_once`; `CASES` 15 → 18 |
| `crates/htui-orch/tests/fake_conformance.rs` | edit | T6 | `cases_len_is_fifteen` → eighteen (`:14-17`) |
| `crates/htui-orch/tests/gix_isolator.rs` | create | T6 | criteria 11, 12, 13; the per-mode table; reconcile; idempotent prepare; the `git worktree list --porcelain` oracle over the same binary the isolator spawned; `SKIP_GIT` at the top of every git-backed case (D40, D41) |

**Not touched, on purpose:** no migration and no column (PRD `:272`; D31 and D33 write into
columns `0001` and `0003` already have); `crates/htui/**` (milestone 6 wires `GixIsolator` and
`ShellVerifier` in `run_worker.rs`); `claim_run`, `overlap.rs`, `recover.rs`, `fanout.rs`,
`select.rs`, `queue.rs` (D44); `crates/htui-store/Cargo.toml` and `crates/htui-core/Cargo.toml`
(`gix` enters `htui-orch` only, D22); `crates/htui-agent/src/**` except the spy arm; every `.snap`
in the workspace; `docs/ANA-2.md` (the §4.6 self-disagreement of D29 is recorded here, not edited
there; the OQ-1 amendment to `:1777` and `:2055` is the main thread's, outside this plan's file
set).

## Tasks

**T1 ∥ T2, then T3 ∥ T4, then T5, then T6.** Independence is decided by intersecting the file sets
above, not by prose:

- T1 ⊂ `crates/htui-core/**`, `crates/htui-store/**`, `crates/htui-agent/{src/conformance.rs,
  tests/recorder.rs}`; T2 ⊂ `Cargo.toml`, `crates/htui-orch/{Cargo.toml, src/isolate.rs,
  src/isolate/git.rs, src/fake.rs}`. Intersection empty → **T1 ∥ T2**.
- T3 ⊂ `crates/htui-orch/{src/verify.rs, src/lib.rs}`; T4 ⊂ `crates/htui-orch/{src/isolate.rs,
  src/isolate/copy.rs}`. Intersection empty → **T3 ∥ T4**; both need T2's manifest, so both
  follow T2. T3 also reads nothing from T1 (`verify.rs` returns a report; the engine persists it).
- T5 touches `isolate.rs` (T4's) and `lib.rs` (T3's) and calls `git.rs` and `copy.rs` → after both.
- T6 touches `engine.rs`, `command.rs`, `gate.rs`, `fake.rs` (T2's) and `conformance.rs`, and
  needs T1's writers, T3's verifier and T5's isolator → last.

That is **two parallel pairs**, which is wider than milestone 2 and is the scale the ultracode
recommendation was made for; the orchestration is the maintainer's call at the CONFIRM gate.

TDD per task: the test that fails for the stated reason comes first.

Every implementer prompt carries: PRD D1–D8 win over this plan where they disagree; graphify-first
for codebase questions, with every graph-derived fact re-verified against the tree; `.sqlx`
regenerated and committed with any query change; nothing sets `updated_at` by hand; no new refusal
constant; **no second migration**; **the only `git` subprocess in `src/` is `isolate/git.rs`'s
`Cli`, and it spawns exactly `worktree add`, `worktree remove`, `merge --no-ff`, `merge --abort`
and `reset --hard`
and `merge --abort` — every other git fact is a `gix` read** (OQ-1, resolved to the alternative);
commit incrementally, because uncommitted work does not survive the session.

### Task 1: `htui-core` + `htui-store` — `command_run`'s writer and reader, `isolation_path`
- **Test first**: `verify_run_is_recorded` in `conformance.rs` — `record_command_run` for a step
  returns the row with `id`, `class = "verify"`, `status`, `exit_code` and `output` as given;
  `command_runs(step)` returns it in `queued_at` order; a row naming a step that does not exist is
  `NotFound { entity: "run_step" }`. It fails to compile. Then extend
  `trees_and_commits_round_trip`: after `upsert_step_tree` with two rows, one `is_primary`,
  `run_steps(run)` shows `isolation_path == Some(primary.path)`.
- **Action**: the three model types; the two trait methods with docs; `MemStore` (`State`
  gains `command_runs: Vec<CommandRun>`; `counts()` reports the real length); `PgStore`'s two
  statements and the `isolation_path` `UPDATE` inside `upsert_step_tree`'s existing transaction;
  `pg/rows.rs`'s row (appended fields); `Writer`/`BufferedWriter`; the two spies; the three count
  pins; the `pg_criteria` twin; `cargo sqlx prepare` from `crates/htui-store`, committed with the
  query.
- **Mirror**: M1 D5/D6/D14; `traits.rs:841-857`'s doc shape; `pg/rows.rs:114-120`'s append rule.
- **Validate**: `cargo test -p htui-core --all-features`, then `USERNAME=htui-ci
  HTUI_TEST_DATABASE_URL=… cargo test -p htui-store --all-features -- --test-threads=1`, then
  `cargo sqlx prepare --check -- --all-targets --all-features` from `crates/htui-store`, then
  `cargo clippy -p htui-core -p htui-store -p htui-agent --all-targets --all-features -- -D warnings`.

### Task 2: `htui-orch` — the manifest and `isolate/git.rs`: a `git` CLI wrapper plus the `gix` reads
*(Amended 2026-09-22 for OQ-1's resolution. File set unchanged: `Cargo.toml`,
`crates/htui-orch/{Cargo.toml, src/isolate.rs, src/isolate/git.rs, src/fake.rs}`.)*
- **Test first**: in `git.rs`'s `#[cfg(test)]`, every git-backed case opening with the `SKIP_GIT`
  check (D40): `init_commit_and_head_round_trip` (a repo made with `gix::init` + `commit_as`
  reports `head()` equal to the commit it made — no `git`);
  `is_dirty_ignores_untracked_and_sees_a_modified_tracked_file` (D24 — no `git`);
  `parses_git_version_and_refuses_below_2_33` (the `git version 2.43.0` line, and a scripted
  `2.32.1` → the refusal sentence); `add_worktree_is_listed_by_gix_and_by_git` (D23: after
  `add_worktree`, `Repository::worktrees()` on the main repo lists a proxy whose `base()?` is the
  path, `is_locked()` with reason `htui run <run_id>`, and `gix::open(path).head_id()` is
  `before_hash`; `git worktree list --porcelain` over the same binary names the path with a
  `locked htui run <run_id>` line; the branch `htui/<step>` exists at `before_hash`);
  `add_worktree_on_an_existing_branch_is_a_git_error_naming_the_branch`;
  `remove_clears_a_locked_dirty_worktree_and_a_vanished_one` (dirty the tree → remove → no
  directory, no admin entry; `rm -rf` a second tree → remove still exits 0 — the crash case);
  `a_stale_entry_that_survives_remove_is_reported_and_never_pruned`;
  `merge_no_ff_of_a_descendant_makes_a_two_parent_commit` (parents `[before, after]`, author
  `htui`, the step's file present in the primary working tree, `MERGE_HEAD` absent);
  `a_conflicting_merge_is_refused_with_the_path_and_aborted` (exit 1 → `merge_conflict: f`,
  then the primary is clean at `before_hash` with no `MERGE_HEAD`);
  `a_held_index_lock_inside_merge_is_aborted_and_retried` (the test plants `index.lock`, the
  first attempt fails with `Unable to write index`, the test removes the lock during the 200 ms
  sleep, the second attempt commits); `git_env_is_scrubbed` (the test process sets
  `GIT_DIR=/nonexistent` before calling `add_worktree`; it still succeeds — verified to fail
  `fatal: not a git repository: '/nonexistent'` without the scrub); `copy_range_moves_every_object_between_odbs`
  (OQ-5 — no `git`); `with_retry_retries_three_times_on_a_lock_error_and_not_on_others` (D39,
  both sources: a `gix` `PermanentlyLocked` and a `Cli` stderr carrying `cannot lock ref`). Every
  test builds its repository under `tempfile::TempDir`.
- **Action**: the workspace and crate manifest lines (D22) in the **first** commit, so the crate
  compiles against `gix` before any code uses it; `Prepared.extra_dirs` and the fake's
  `Vec::new()`; the idempotence sentence on the trait; then `git.rs` in two halves. **(a) The
  `git` CLI wrapper**, `pub(crate) struct Cli { binary: PathBuf, version: (u32, u32, u32) }`,
  built once by `Cli::locate() -> Result<Cli, IsolateError>` with `which::which("git")`
  (workspace `which = "8.0.6"`, `Cargo.toml:66`, consumed by `htui-agent` at
  `crates/htui-agent/Cargo.toml:32`; `htui-orch` adds the edge) — absent → `Refused("git not on
  PATH")` — then `git --version` parsed from `git version <maj>.<min>.<patch>[…]` and refused
  below **2.33.0** (the floor `--reason` with `add --lock` sets; ledger). `async fn run(&self,
  cwd: &Path, args: &[&str], env: &[(&str, &str)]) -> Result<Output, IsolateError>` spawns
  through the workspace's **`process-wrap` 10.0.0** (`Cargo.toml:59-60`, features `tokio1`,
  `creation-flags`, `job-object`, `process-group`; `Cargo.lock:3803-3805`) exactly as
  `launch.rs:1110-1160` does — `tokio::process::Command` → `CommandWrap::from`, `.wrap(ProcessGroup::leader())`
  on Unix, `.wrap(CreationFlags(CREATE_NO_WINDOW))` + `.wrap(JobObject)` on Windows — with
  `stdin(Stdio::null())`, `stdout`/`stderr` **both `piped()` and read to end concurrently**
  (`tokio::join!` over two `read_to_end`s so neither pipe can fill and stall the child), each
  tail-capped at 64 KiB like D30; under `tokio::time::timeout(Duration::from_secs(120))`, on
  expiry the process group is killed and the error is `Git("git <verb> timed out after 120s")`.
  **Environment**: the parent's environment is inherited (so `PATH`, `HOME` and the user's
  `~/.gitconfig` apply — the inherited-behaviour risk) **minus** every variable that redirects
  repository discovery — `env_remove` for `GIT_DIR`, `GIT_WORK_TREE`, `GIT_INDEX_FILE`,
  `GIT_COMMON_DIR`, `GIT_OBJECT_DIRECTORY` — **plus** `LC_ALL=C` (D39's classifier matches
  English) and `GIT_TERMINAL_PROMPT=0`, and, for `merge_no_ff` only, the four identity
  variables of D25. Verbs: `add_worktree(repo, path, step_id, run_id, before_hash)`,
  `remove_worktree(repo, path)`, `reset_hard(repo)`, `merge_no_ff(primary, after_hash,
  step_id) -> Result<MergeResult { commit } | Conflict { paths }, _>`, `abort_merge(primary)`.
  Nothing is parsed from any command's stdout; success is exit 0 plus the `gix` post-condition
  D23/D25 name; failure text is the last non-empty stderr line. **(b) The sync `gix` half**,
  functions over `&Path` returning owned values, each called under `spawn_blocking`: `open`,
  `head`, `is_dirty`, `has_submodules`, `create_branch` (D26's label; `reference(...,
  PreviousValue::MustNotExist, ...)`), `branch_target`, `worktree_by_path` (`worktrees()` filtered
  on `base()`), `conflicted_paths` (`open_index()`, `stage() != Unconflicted`),
  `checkout_tree_over` (**D35's full-index reset only**; doc comment says so and names the
  populated-directory caveat), `copy_range`, and `with_retry` over both halves (D39). Each public
  function documents the `gix` call or `git` verb it wraps with the `gix-0.87.1/src/...` line or
  the `git worktree --help` synopsis this plan cites, so a fact-checker can re-verify without
  recall.
- **Mirror**: `launch.rs:1110-1160` for the spawn and the blocking discipline; `isolate.rs:40-52`
  for errors; `testkit.rs:35` for `SKIP_GIT`.
- **Validate**: `cargo test -p htui-orch --all-features` then `cargo clippy -p htui-orch
  --all-targets --all-features -- -D warnings`; `cargo tree -p htui-orch -i gix --edges features`
  once, to confirm the feature set is the one D22 names — six features, no `merge`, no
  `revision`, nothing enabled `blocking-network-client` — and `grep -rn 'Command::new' crates/htui-orch/src/`
  once, to confirm `isolate/git.rs` is the only hit.

### Task 3: `htui-orch` — `verify.rs`
- **Test first**: `exit_zero_is_pass_with_code_zero`, `nonzero_exit_is_fail_with_the_code`,
  `a_missing_binary_is_unavailable_with_a_reason`, `a_timeout_is_unavailable_and_kills_the_group`
  (uses `sleep 30` behind a 200 ms remainder), `output_is_tail_capped_and_masked` (a 100 KiB
  stdout keeps its last 64 KiB; a scripted secret is masked), `the_verify_semaphore_admits_one`
  (two concurrent requests with `command_limits.verify = 1` serialise, observed by timestamps in
  the reports); `no_verify_command_yields_no_report`. All run `sh -c` (Unix) and are
  `#[cfg(unix)]` where they depend on `sleep`/`sh`; the Windows twin is MOD-16's (PRD `:296`).
- **Action**: `trait Verifier { fn run(&self, req: VerifyRequest) -> VerifierFuture<'_, Option<VerifyReport>> }`
  with `VerifyRequest { command: Option<String>, cwd: PathBuf, remaining: Option<Duration>,
  step: StepId }` and `VerifyReport { outcome: VerifyOutcome, exit_code: Option<i32>, output: String,
  started_at, finished_at, reason: Option<String> }`; `ShellVerifier::new(limits: &BTreeMap<String,u32>,
  scrubber)` holding the `verify` semaphore (D30); `lib.rs`'s module line and re-exports.
- **Mirror**: `launch.rs:1110-1160`; `IsolatorFuture`'s alias for `VerifierFuture`.
- **Validate**: as T2.

### Task 4: `htui-orch` — `isolate/copy.rs`
- **Test first**: `default_excludes_match_ana2s_seven_entries` (the constant, verbatim);
  `an_empty_project_list_falls_back_to_the_default` (D35); `cmake_build_star_matches_by_prefix`;
  `measure_counts_only_what_would_be_copied` (a `target/` of 1 MiB is not counted);
  `copy_tree_reproduces_the_layout_minus_excludes_and_keeps_dot_git`;
  `a_source_with_a_dot_git_file_is_refused`; `a_source_without_dot_git_is_refused`.
- **Action**: `DEFAULT_COPY_EXCLUDE`, `Exclude::matches(component)`, `measure(src, &excludes)
  -> u64` over `walkdir`, `copy_tree(src, dst, &excludes)` (files, dirs, symlinks re-created as
  symlinks; permissions preserved on Unix), all sync, all called under `spawn_blocking` by T5.
- **Mirror**: `crates/htui-store/src/cache/**`'s use of `walkdir` for a bounded walk.
- **Validate**: as T2.

### Task 5: `htui-orch` — `isolate/real.rs`, `GixIsolator`
- **Test first** (each over a `TempDir` with one or two `gix::init` repos and a config map, D34):
  `worktree_prepare_writes_one_tree_per_repo_outside_every_repo` (criterion 11's first half:
  two repos → two `PreparedTree`s, both paths under the scratch root, both `before_hash`es equal
  to the repos' `HEAD`s, `cwd` is the common parent, `extra_dirs` is empty because both trees
  are under `cwd`); `worktree_capture_reports_the_new_head_or_none`;
  `a_no_commit_clean_worktree_is_removed_at_capture_and_a_dirty_one_is_kept` (D27);
  `local_records_dirty_true_and_head_as_before_hash` (criterion 12's first half, D24);
  `shared_serialized_labels_after_hash_and_holds_the_lock` (D26, D43: a second `prepare` on the
  same `(box, repo)` does not resolve until the first `cleanup`); `copy_resets_a_dirty_source`
  (D35); `prepare_is_idempotent_for_worktree_and_copy` (D38: two calls, one tree, the same
  `before_hash` even after the primary moved); `a_repo_with_submodules_is_refused_for_worktree`;
  `a_scratch_root_inside_a_repo_is_refused` (invariant 4); `worktree_prepare_is_refused_without_git`
  (a config whose `Cli` failed to locate → `isolation refused: git not on PATH`, while a `local`
  step on the same config prepares fine); `cleanup_removes_every_tree_and_leaves_
  the_repo_untouched` (criterion 13's isolator half: the `git worktree list --porcelain` oracle
  over the binary the isolator used and `Repository::worktrees()` both show no entry under the
  scratch root, a user's own linked worktree made by the test survives, the managed tree's `HEAD`
  and dirtiness unchanged, no `refs/heads/htui/*` checked out anywhere — `head()` of every repo
  is compared);
  `reconcile_merges_the_winner_no_ff_and_updates_the_primary_tree` (D25: after reconcile the
  primary's `HEAD` has two parents, its working tree contains the step's file, the winner's
  `after_hash` in the returned commits is the merge commit); `reconcile_refuses_a_dirty_primary`
  and `reconcile_refuses_a_moved_primary` (the exact sentences).
- **Action**: `IsolatorConfig`, `RepoCheckout`, `GixIsolator` with `locks: Mutex<BTreeMap<(BoxId,
  RepoId), Arc<tokio::sync::Mutex<()>>>>` and `held: Mutex<BTreeMap<StepId, Vec<OwnedMutexGuard>>>`;
  `impl Isolator` dispatching per `Isolation` to one private `async fn` per mode, each of which
  calls T2/T4's sync functions through `spawn_blocking`; `isolate.rs` and `lib.rs` exports.
- **Mirror**: `fake.rs:143-215` for the trait impl shape; `mem.rs:3-7` for "no `.await` under a
  `std` lock".
- **Validate**: as T2.

- **Fact-check addition (2026-09-22):** correct `Isolator::reconcile`'s trait doc in
  `isolate.rs` while you are in the file. It still says "Fan-out is milestone 4's, so this
  milestone's only winner is the single step at `fanout_index = 0` and reconciliation is an
  identity" — D25 makes it a real no-ff merge with a two-parent commit for `worktree` and `copy`,
  so the sentence would contradict `GixIsolator` the moment this task lands. No other task's file
  set covers it.

### Task 6: `htui-orch` — the engine's three missing call sites, verify at stage 5, `CancelRun`, criteria
- **Test first**: in `conformance.rs` over the fake (`CASES` 15 → 18):
  `verify_fail_settles_failed` (a `FakeVerifier` scripted `Fail(1)` on a `never`-gated phase →
  step `failed`, `verify_outcome = fail`, `verify_exit_code = 1`, one `command_run` row);
  `verify_unavailable_never_fails` (scripted `Unavailable` → step `done`, `verify_outcome =
  unavailable`, `verify_exit_code` NULL, the reason in `command_run.output`);
  `cancel_cleans_up_once` (a parked run cancelled → run `cancelled`, every step `cancelled`, the
  fake's cleanup counter is 1, the item `open`). Then in `tests/gix_isolator.rs` the end-to-end
  half of criteria 11, 12 and 13 through an `Engine` over `GixIsolator` + `ShellVerifier` +
  `FakeDriver` + `MemStore`: a two-repo `worktree` walk writes two `run_step_tree` and two
  `run_step_commit` rows per step and `run_step.isolation_path` is the primary tree (D33); a
  `local` step on a dirty tree records `dirty = true` (the sweep's refusal is milestone 5's and
  is not asserted here); `CancelRun` on a parked run leaves no tree and no `htui/` branch checked
  out. The `git worktree list --porcelain` oracle runs over the same binary the isolator
  spawned; the `worktree`-mode half of the binary prints `SKIP_GIT` and passes without one (D40).
- **Action**: `EngineParts.verifier: &'a V` (one more generic, defaulted nowhere — every
  constructor names it); stage 5 calls `verifier.run` with the primary tree's path and the
  deadline remainder, records the `command_run` row, and passes the outcome into `SettleInput`
  and `StepOutcome`; `reconcile_done_step` and `park_run` (D25); `cleanup_run` after every
  terminal `finish_run` (D36) and at `CancelRun` (D45); `extra_dirs` into `SessionSpec`;
  `FakeVerifier` and the harness field; the doc corrections in `gate.rs` and `engine.rs`.
- **Mirror**: `walk_step`/`fail_hard` for the error region; `answer_gate`'s unpark order for
  `park_run`'s reverse (`engine.rs:385-396`); blueprint H-10's step → run → item order.
- **Validate**: as T2, then the full workspace gate below.

## Validation

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
USERNAME=htui-ci HTUI_TEST_DATABASE_URL=postgres://postgres:htui@localhost:5439/postgres \
  cargo test --workspace --all-features -- --test-threads=1
cargo doc --workspace --no-deps
cd crates/htui-store && \
  DATABASE_URL=postgres://postgres:htui@localhost:5439/htui_sqlx cargo sqlx prepare --check -- --all-targets --all-features
cargo tree -p htui-orch -i gix --edges features    # once: the D22 feature set and nothing more
```

`--test-threads=1` is not optional (M2 plan `:207-208`: the keyring fake is process-wide). `cargo
doc` exits 101 at HEAD on six pre-existing `htui-store` intra-doc errors (CLEAN-3); this milestone
adds **zero** — and `gix`'s own docs are `--no-deps`-excluded, so its `broken_intra_doc_links`
cannot leak in. Without `HTUI_TEST_DATABASE_URL` the store suites print `skipped:` and pass
(`README.md:477-480`); without `git` on `PATH` (or with one older than 2.33.0) every git-backed
case — T2's `Cli` and worktree/merge tests, T5's `worktree` and `reconcile` cases, T6's criteria 11
and 13 — prints `skipped: git not on PATH` (or `skipped: git <version> is older than 2.33.0`)
and passes (D40); everything else in this milestone runs on a box with neither. This box has
`git version 2.43.0`, so the gate here runs them all.

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| ~~**`gix` has no `worktree add`, and the hand-built layout diverges from `git`'s**~~ — **retired 2026-09-22**: OQ-1 resolved to the alternative, `git` writes the layout | — | Superseded by the four `git` rows below; kept so the record shows why they exist |
| **`git` version skew across boxes**: `worktree add --lock` needs 2.13, `worktree remove` 2.17, `--reason` with `add --lock` **2.33.0** (absent from v2.32.0's `git-worktree.txt`, present in v2.33.0's), and `remove --force` twice for a locked tree is documented at 2.33.0 — so the floor is **2.33.0** (Aug 2021); an older `git` on a second box would fail `add` with an unknown-option error at the first `worktree` step | Medium | `Cli::locate` parses `git --version` once and refuses below 2.33.0 with `git <version> is older than 2.33.0`, before any spawn; tests skip with the same sentence (D40); this box is 2.43.0 |
| **The `git` binary is absent** on a box (or is a shim that is not `git`): the `worktree` mode and every non-identity `reconcile` are unusable there | Medium | `Refused("git not on PATH")` at `prepare` for `worktree` and at `reconcile` for `worktree`/`copy`, reaching the operator through `fail_hard` (D37); `local` and `shared_serialized` need no `git` and keep working; milestone 6's `run_worker` can surface the refusal before a run is claimed |
| **Output-parsing brittleness**: nothing is parsed on success, but D39's CLI classifier and D25's conflict branch rest on three stderr phrases (`Unable to create '…lock': File exists`, `cannot lock ref`, `Unable to write index`) and on exit 1 meaning "conflict or index lock" — each verified on 2.43.0 only, and `git` may reword them | Medium | `LC_ALL=C` in the child env pins the language; a missed classification degrades to a `Git(...)` error, never to silent success, because every success path has a `gix` post-condition (two-parent HEAD; locked proxy at the path); the retry wrapper logs the unclassified stderr at `warn`, so a rewording is a one-line fix |
| **`git`'s behaviour is inherited, not controlled**: the user's `~/.gitconfig` and the repo's config apply to every verb (`core.autocrlf`, `filter.*`, `merge.conflictstyle`, `merge.renames`, `core.hooksPath`); `post-checkout` runs on `worktree add` (verified) and `pre-merge-commit`/`prepare-commit-msg`/`commit-msg`/`post-merge` on `merge`; `git worktree prune` clears the user's own *unlocked* stale entries, not just ours; `git`'s submodule handling in worktrees is the "incomplete" one ANA-2 `:914` cites | Medium | This is the trade the maintainer made — the hand-built path would have had to *re-implement* these to be correct, and MOD-16 (PRD `:296`) verifies `git.exe`'s behaviour instead of ours; submodule repos are still refused before any spawn (mode table); prune is a backstop that fires only when our own entry survived its `remove` (D23); hooks are the repo owner's and run with the scrubbed environment; a hook that fails fails the verb, which is a `Git(...)` error the operator reads |
| D35's full-index `gix` checkout — now the only `worktree-mutation` consumer — misbehaves on the populated *copy* it resets (a staged-new file left behind, a filter not applied) | Low | The copy is scratch; T4/T5's `copy_resets_a_dirty_source` asserts a deleted tracked file is restored and a modified one overwritten; a fifth verb (`git reset --hard`) is the maintainer's call if the composition proves wrong, recorded in the close-out |
| `is_dirty()` is `O(tree)` and runs on every `prepare` and every reconcile of a managed tree, on the maintainer's own checkout with a large `target/` | Medium | `target/` is untracked and ignored, and the walk respects excludes (`status` pulls `dirwalk` + `excludes`, `gix-0.87.1/Cargo.toml:241-245`); measured on this repo in T5 and recorded in the close-out |
| The lock classifier of D39 matches on message text — `gix`'s `PermanentlyLocked` and the `git` CLI's stderr — and misses a lock error, so a contended write fails without retrying | Medium | Named in D39 as a weak point; the retry wrapper logs the classified and unclassified error kinds at `warn`, so the first miss in the field is a one-line fix; PRD `:389`'s eight-of-thirteen figure is for parallel siblings, which are milestone 4's |
| ANA-2 §4.6 disagrees with itself about a dirty `shared_serialized` tree (`:916` versus `:963-968`) and milestone 4 reads `:916` and refuses what this milestone records | Medium | D29 records which reading ships and why; milestone 4's sibling reset is where `:916` applies |
| The engine's new `park_run` is three compare-and-sets, not one transaction, on Postgres | Certain | Same as the gate park today (blueprint H-10, R-5 carried); ordered step-stays-done → run → item so the forbidden direction cannot occur |
| A `copy` of the maintainer's checkout hits the size cap or takes minutes on a full `target/` that a stale `copy_exclude` does not name | Low | D35's default list is applied when the project's is empty; the refusal names the measured size and the cap |
| D45's `CancelRun` on a `running` run with a live session — impossible in this milestone's synchronous walk — becomes possible at milestone 6 and cancels a step out from under its recorder | Low | The guard checks "no live session" and milestone 6 owns the session registry; `command.rs`'s doc says so |
| `spawn_blocking` inside an `IsolatorFuture` needs a Tokio runtime with a blocking pool, and a `#[tokio::test]` with `flavor = "current_thread"` still has one, but a `futures::executor::block_on` caller does not | Low | Every test and the fake harness already run under `#[tokio::test]`; `run_worker` at milestone 6 runs under the app's runtime |
| A third §4.2/§4.3 disagreement | — | **None found.** The only tension this milestone hit is D30's timeout, where `:515` (`unavailable`) and `:439` (deadline → `failed`) both apply and compose rather than conflict; D29's is internal to §4.6 |

## Acceptance

- [ ] `gix` 0.87.1 is in `Cargo.lock` with exactly D22's six-feature set (`cargo tree -p
      htui-orch -i gix --edges features`; no `gix-merge`, no `gix-revision` `describe`/`merge_base`),
      consumed by `htui-orch` only; `htui-core`'s and `htui-store`'s manifests show a zero diff;
      `git2`/`libgit2` is nowhere in `Cargo.lock`.
- [ ] `GixIsolator` implements all four modes; each mode's row of the table above is pinned by a
      test in `tests/gix_isolator.rs` or `isolate/real.rs`, including every refusal sentence.
- [ ] ANA-2 §12 criteria 11, 12 (the record half) and 13 pass end to end against `GixIsolator` +
      `ShellVerifier` + `FakeDriver` + `MemStore`, with no Postgres; the `git worktree list
      --porcelain` oracle runs over the same binary the isolator spawned, and every git-backed
      case prints `skipped: git not on PATH` (or the version sentence) and passes without one
      (D40).
- [ ] `verify_command`'s three outcomes land in `run_step.verify_outcome`/`verify_exit_code` and a
      `command_run` row; `fail` settles the step `failed`, `unavailable` never does, and the
      seeded phases (`verify_command: None`) write neither column — pinned over the fake by
      three new `CASES` (15 → 18).
- [ ] `reconcile` and `cleanup` have production callers in `engine.rs`; `cleanup_run` is called
      after every terminal `finish_run` and by `CancelRun`; the fake's cleanup counter is exactly
      1 on a cancelled run.
- [ ] `record_command_run`, `command_runs` and the `isolation_path` write pass on both stores;
      `CASES` is 49 and both count pins agree; `writer_buffered.rs` pins the two new refusals.
- [ ] `cargo fmt --all -- --check` clean; `cargo clippy --workspace --all-targets --all-features
      -- -D warnings` clean.
- [ ] `cargo test --workspace --all-features -- --test-threads=1` green with Postgres up; green
      minus the `skipped:` lines with Postgres down; green minus the `skipped: git …` lines on a
      box without `git` 2.33.0 or newer; on this box (2.43.0) no git-backed case skips.
- [ ] `cargo sqlx prepare --check` clean, with every new query file committed alongside its query.
- [ ] `cargo doc --workspace --no-deps` adds **no new** errors over CLEAN-3's six.
- [ ] No migration, no `.snap` re-recorded, the **only** `git` subprocess under
      `crates/htui-orch/src/` is `isolate/git.rs`'s `Cli` (grep `Command::new` over `src/` hits
      that file and `verify.rs`'s shell only), spawning exactly `worktree add`, `worktree remove`,
      `merge --no-ff`, `merge --abort` and `reset --hard`; no overlap or admission logic
      anywhere in this milestone's files (D44).

## Verified against the tree and against `gix` (fact ledger for the blueprint's checker)

**Tree facts this plan rests on, each read on 2026-09-22 at `e66455e`:**

| Claim | Where |
|---|---|
| `Isolator` has four verbs; `Prepared { trees, cwd }`; `IsolatorFuture` is `Send` | `crates/htui-orch/src/isolate.rs:32`, `:75-80`, `:92-131` |
| The engine calls `prepare` once and `capture` once per step and **never** `reconcile` or `cleanup` | `crates/htui-orch/src/engine.rs:794`, `:842`; grep of `.reconcile(`/`.cleanup(` over `src/` is empty |
| `walk_step` → `walk_live_step` → `fail_hard` is the live-step error path | `engine.rs:744-777`, `:895-915` |
| `SettleInput.verify_outcome` and `StepOutcome.verify_*` are hard-coded `None` | `engine.rs:853-855`, `:869-870`; `gate.rs:219-220` |
| `gate.rs` already tests `verify_outcome == Some(Fail)` | `gate.rs:256` |
| `SessionSpec.extra_dirs` is hard-coded empty; `env` is empty | `engine.rs:1097-1099` |
| Steps reach `done` at two sites | `gate.rs:383-388`; `engine.rs:376-377` (`answer_gate`) |
| Terminal `finish_run` sites | `engine.rs:415`, `:556`, `:724`, `:912`; `gate.rs:542`, `:651`, `:675`, `:688` |
| `CancelRun` is reserved for "milestones 3 to 6" | `crates/htui-orch/src/command.rs:28-30` |
| `RunStatus::can_move_to` admits `Cancelled` from `queued`, `running`, `awaiting_approval` | `crates/htui-core/src/model/run.rs:60-68` |
| `RunStepTree { run_step_id, repo_id, mode, path, base_ref, dirty }`; `RunStepCommit { …, before_hash: String, after_hash: Option<String> }` | `model/run.rs:424-439`, `:572-583` |
| `VerifyOutcome { Pass, Fail, Unavailable }`; `RunStep.verify_outcome`/`verify_exit_code`/`isolation_path` | `model/run.rs:154-164`, `:248-258` |
| `SnapshotPhase.isolation: Isolation` (resolved), `verify_command: Option<String>`, `deadline_seconds: Option<u32>`, `command_queue` | `model/run.rs:502-510` |
| `SnapshotSettings` carries no `copy_exclude` | `model/run.rs:557-570` |
| `Isolation` has the four variants with ANA-2's strings | `crates/htui-core/src/model/kind.rs:24-33` |
| `ProjectSettings.copy_exclude: Vec<String>`, default empty | `kind.rs:276`, `:292` |
| `Repo { name, is_primary, … }`; `RepoBoxPath { local_path, … }` | `model/hierarchy.rs:124-141`, `:184-191` |
| `upsert_step_tree`, `record_commits`, `create_repo`, `repos`, `upsert_repo_box_path`, `repo_box_paths`, `finish_run` on `WriteStore`; 61 methods | `store/traits.rs:841-857`, `:470`, `:489`, `:496`, `:502`, `:915`; `awk` over the `impl` block |
| `repo_paths(box)` is inherent on `Backend` and `MemStore` | `crates/htui-store/src/backend.rs:529-535`; `crates/htui-core/src/store/mem.rs:537-545` |
| `run_step.isolation_path` has no writer | `mem.rs:1564`, `:3354`; `pg/write.rs:2724` is a `SELECT` |
| `command_run` table exists; no model type, no seam method; `MemStore` counts `0` | `migrations/0001_init.sql:537-551`; `traits.rs:1322-1327`; `mem.rs:2836` |
| `config_root()` is `<dirs::config_dir()>/htui` | `crates/htui-store/src/identity.rs:36-52` |
| `graph.rs` refuses `local` at `fan_out > 1` at snapshot time | `crates/htui-orch/src/graph.rs:139-145`, `:425-429` |
| Every seeded phase has `verify_command: None` and `isolation: None` | `crates/htui-core/src/seed.rs:231-233` |
| `CASES` 48 pinned at two sites; `htui-orch` `CASES` 15 pinned once | `crates/htui-store/tests/pg_conformance.rs:19`; `crates/htui-core/tests/mem_store.rs:36`; `crates/htui-orch/tests/fake_conformance.rs:14-17` |
| `UsageSpy` and `SpyStore` are exhaustive `WriteStore` impls (M2's `finish_run` needed arms) | `crates/htui-agent/src/conformance.rs:1004`; `crates/htui-agent/tests/recorder.rs:698-720` |
| Workspace `tokio` already has `process`, `io-util`, `fs`; `process-wrap`, `windows`, `walkdir`, `dirs`, `which` are workspace deps; `similar` is declared at `:65` but is **not** idle — `htui-agent` consumes it | `Cargo.toml:32-33`, `:51`, `:59-64`, `:65`, `:66`, `:101`; `crates/htui-agent/Cargo.toml:35`, `crates/htui-agent/src/acp/fs.rs:106-118` |
| `process-wrap` is `10.0.0` with features `tokio1`, `creation-flags`, `job-object`, `process-group`; one copy in the lock; consumed by `htui-agent` only today | `Cargo.toml:59-60`; `Cargo.lock:3803-3805`; `crates/htui-agent/Cargo.toml:30`; `htui-orch/Cargo.toml` has no `process-wrap`, `which`, `gix`, `walkdir` or `dirs` edge |
| `htui-orch`'s manifest: deps, `test-support`, `[lints] workspace = true` | `crates/htui-orch/Cargo.toml:14-15`, `:21-27`, `:34-35` |
| `process-wrap` spawn pattern | `crates/htui-agent/src/launch.rs:1110-1160` |
| `PromptSpec.verify_failure`/`previous_diff`; `VerifyFailure { exit_code, output }`; `DiffBlock { range, stat, diff }` | `crates/htui-core/src/prompt/mod.rs:104-106`, `:144-149`, `:151-165` |
| ANA-5 says `verify_failure` is `command_run.output` | `docs/ANA-5.md:335`, `:583`; `docs/ANA-2.md:730` |
| `git` 2.43.0 is on this box at `/usr/bin/git` — ~~test oracle only~~ **a runtime dependency since OQ-1's resolution** (2026-09-22) | `which git`; `git --version` |
| No CI configuration in the tree; the gate runs on the maintainer's box | `ls -a` at the root (no `.github/`, no `.gitlab-ci.yml`); `README.md:510` ("the CI form") |
| Milestone 1's R-2 sentence | `HANDOFF.md:205-212` |

**`gix` 0.87.1 facts, read from the crate source in the local registry
(`~/.cargo/registry/src/index.crates.io-*/gix-0.87.1/`) after `cargo info gix` downloaded it:**

| API | Where in the crate |
|---|---|
| `open`, `discover`, `init` | `src/lib.rs:418`, `:262`, `:332` |
| `worktrees()`, `worktree_proxy_by_id`, `main_repo`, `worktree`, `is_bare` — and nothing that adds/removes/prunes | `src/repository/worktree.rs:46`, `:65`, `:80`, `:93`, `:101`; whole file `:46-160` |
| `Proxy::{base, git_dir, id, is_locked, lock_reason, into_repo}` | `src/worktree/proxy.rs:48-103` |
| `head_id`, `head_commit`, `head_tree_id`, `head` | `src/repository/reference.rs:211`, `:253`, `:263`, `:187` |
| `reference(name, target, PreviousValue, log)`, `edit_reference`, `find_reference` | `src/repository/reference.rs:79-87`, `:136`, `:323` |
| `is_dirty()` and its untracked-files caveat | `src/status/mod.rs:156-168` |
| `merge_commits`, `merge_trees`, `tree_merge_options`, `merge_base` | `src/repository/merge.rs:181-188`, `:126`, `:84`; `src/repository/revision.rs:58` |
| `merge::tree::Outcome { tree: Editor, conflicts, … }`, `has_unresolved_conflicts(TreatAsUnresolved)` | `src/merge.rs:116-135` |
| `commit`, `commit_as`, `find_object`, `find_tree`, `write_object` | `src/repository/object.rs:461-467`, `:361-370`, `:55`, `:103`, `:250` |
| `rev_walk`, `rev_parse_single` | `src/repository/revision.rs:174`, `:42` |
| `diff_tree_to_tree` | `src/repository/diff.rs:50` |
| `submodules()` | `src/repository/submodule.rs:93` |
| `git_dir`, `workdir`, `index_path`, `checkout_options` | `src/repository/location.rs:13`, `:97`, `:44`; `src/repository/checkout.rs:8` |
| The clone checkout sequence (`State::from_tree` → `File::from_state` → `gix_worktree_state::checkout` → `write`) | `src/clone/checkout.rs:113-146` |
| Re-exports `gix::index`, `gix::lock`, `gix::refs`, `gix::worktree::state` | `src/lib.rs:140-141`, `:151`; `src/worktree/mod.rs:6-8` |
| `Repository: Send`, not `Sync`; `parallel` required for `Send` | `src/types.rs:148-158` |
| Feature gates: `sha1` (in `default`, lost under `default-features = false`), `status`, `merge`, `worktree-mutation`, `parallel` | `Cargo.toml:132-139`, `:228-233`, `:241-245`, `:184-188`, `:259-262`, `:192` |
| Feature closures and gates re-read for D22's recomputation (2026-09-22): `status = [gix-status, dirwalk, index, blob-diff, gix-diff/index]`; `dirwalk = [gix-dir, attributes, excludes]`; `blob-diff = [gix-diff/blob, attributes]`; `merge = [blob-diff, gix-merge, attributes]`; `worktree-mutation = [attributes, gix-worktree-state]`; `revision = [gix-revision/describe, gix-revision/merge_base, index]`; `index = [gix-index]`; `max-performance-safe = [max-control]`; `mod submodule` is under `attributes`; `mod index` under `index`; `mod merge` under `merge`; `mod revision` and `pub mod walk` (hence `rev_walk`) are **ungated** | `Cargo.toml:241-247`, `:140-144`, `:79-82`, `:184-188`, `:259-262`, `:201-205`, `:167`, `:183`; `src/repository/mod.rs:43-44`, `:50-51`, `:59`, `:62-63`; `src/revision/mod.rs:9`; `src/repository/revision.rs:174` (no `cfg`) |
| `open_index()` and `Entry::stage()` / `Stage::{Unconflicted, Base, Ours, Theirs}` — D25's conflict path list | `src/repository/index.rs:25`; `gix-index-0.55.0/src/entry/mod.rs:3-11`, `:90` |
| `Commit::parent_ids()` — D25's two-parent post-condition; `Proxy::base()` — D23's lookup by path | `src/object/commit.rs:154`; `src/worktree/proxy.rs:48` |
| Pinned sub-crates: `gix-lock ^24.0.0`, `gix-ref ^0.67.1`, `gix-index ^0.55.0`, `gix-worktree-state ^0.34.1`, `gix-status ^0.34.1` | `Cargo.toml:360-361`, `:408-409`, `:356-357`, `:465-467`, `:427-430` |
| `gix-lock::acquire::Fail::{Immediately, AfterDurationWithBackoff(Duration)}` is quadratic backoff | docs.rs, gix-lock 24.0.0 (fetched) |
| `gix-ref::transaction::RefEdit { change, name, deref }` | docs.rs, gix-ref 0.67.1 (fetched) |
| MSRV 1.85; crates.io current 0.87.1 | `cargo info gix`, `cargo search gix` |

**Compile probe** (`/tmp/gixprobe`, `cargo check` on rustc 1.98.1 against `gix = "=0.87.1"`,
`default-features = false`, D22's feature set, `#![forbid(unsafe_code)]`): the first run without
`sha1` failed inside `gix-hash 0.26.2` (`E0004` on an empty `Kind`), which is how D22's list
gained `sha1`. The second and third runs **type-checked every call this plan names**, and a
deliberately injected type error in the same file failed as expected (so a clean check is not a
skipped one). Verified by compile, not by reading:

| Call, as compiled |
|---|
| `gix::open`, `gix::init`, `gix::discover`; `head_id()?.detach()`, `head_tree_id`, `is_dirty()?`, `is_bare`, `git_dir`, `workdir`, `index_path`, `submodules()?` |
| `worktrees()?` → `Proxy::{id, is_locked, lock_reason, base()?, git_dir}`; `main_repo()?` |
| `reference("refs/heads/htui/x", id, PreviousValue::MustNotExist, "…")?`; `find_reference` |
| `commit_as(SignatureRef, SignatureRef, "refs/heads/…", msg, tree, [parents])?` — **without** the `command` feature |
| `tree_merge_options()?.into()` → `merge_commits(ours, theirs, Labels::default(), opts)?`; `out.tree_merge.has_unresolved_conflicts(TreatAsUnresolved::default())`; `out.tree_merge.tree.write()?.detach()`; `merge_base` |
| `Conflict { ours, theirs, … }` where both are `gix::diff::tree_with_rewrites::ChangeDetached` with `.location() -> &BStr` — D25's path list |
| `diff_tree_to_tree(Some(&old_tree), Some(&new_tree), None)?` |
| `gix::index::State::from_tree(&tree_id, &repo.objects, Default::default())?`; `State::remove_entries(\|idx, path, entry\| …)`; `entries()` — so D25's **restricted-index checkout is expressible** |
| `gix::index::File::from_state(state, repo.index_path())`; `file.write(Default::default())?` |
| `repo.checkout_options(attributes::Source::IdMapping)?` with fields `destination_is_initially_empty`, `overwrite_existing`, `keep_going` |
| `gix::worktree::state::checkout(&mut file, workdir, repo.objects.clone().into_arc()?, &gix::progress::Discard, &gix::progress::Discard, &AtomicBool, opts)?` → `checkout::Outcome` |
| `rev_walk([tip]).with_hidden([base]).all()?` yielding `info?.id` — OQ-5's range walk |
| `gix::lock::acquire::Fail::AfterDurationWithBackoff(Duration)` |

*After OQ-1's resolution (2026-09-22)* the probe rows for `reference(... MustNotExist ...)` (now
D26's label only), `merge_commits`/`tree_merge_options`/`has_unresolved_conflicts`/`merge_base`,
`Conflict { ours, theirs }`, `diff_tree_to_tree`, `State::remove_entries` and the
`destination_is_initially_empty = true` checkout describe calls the plan **no longer makes**;
they stay in the table as the record of what was verified. Still load-bearing: `gix::init`,
`head_id`, `is_dirty`, `submodules`, `worktrees()` → `Proxy::{base, is_locked, lock_reason}`,
`commit_as` (tests), `State::from_tree` → `File::from_state` → `checkout` with
`overwrite_existing = true` → `write` (D35 only), `rev_walk`, `find_object`/`write_object`. The
recomputed six-feature list of D22 was **not** re-probed; T2's first commit is the check.

**`git` CLI facts, produced on this box (`git version 2.43.0`, `LC_ALL=C`, throwaway
repositories under `mktemp -d`) on 2026-09-22:**

| Fact | How |
|---|---|
| `git worktree add --lock --reason "htui run r1" -b htui/step1 <path> <hash>` exits 0; stdout `Preparing worktree (new branch 'htui/step1')` / `HEAD is now at <short> <subject>`; `git worktree list --porcelain` shows `worktree <path>` / `HEAD <hash>` / `branch refs/heads/htui/step1` / `locked htui run r1`; the `locked` file holds the reason | run; synopsis `git worktree add [-f] [--detach] [--checkout] [--lock [--reason <string>]] [--orphan] [(-b \| -B) <new-branch>] <path> [<commit-ish>]` from `git worktree --help` |
| The admin entry is named after the path's basename (`main`, then `main1` for a second tree with the same basename), not after the branch | `ls .git/worktrees` after two adds |
| Branch exists → exit **255**, `fatal: a branch named 'htui/step1' already exists`; non-empty path → exit 128, `fatal: '<path>' already exists`; bad start point → exit 128, `fatal: invalid reference: deadbeef` | run |
| Ref lock held → exit 255, `fatal: cannot lock ref 'refs/heads/htui/s6': Unable to create '<git_dir>/refs/heads/htui/s6.lock': File exists.` | planted `refs/heads/htui/s6.lock`, run |
| `git worktree remove <path>` on a locked tree → exit 128, `fatal: cannot remove a locked working tree, lock reason: htui run r1` / `use 'remove -f -f' to override or unlock first`; `--force` once → same; `--force --force` on a locked **and dirty** tree → exit 0, directory and admin entry gone; `--force --force` when the directory was already `rm -rf`ed → exit 0, entry gone; unknown path → exit 128, `fatal: '<path>' is not a working tree` | run |
| `git worktree prune -v` leaves a **locked** entry whose directory is gone untouched (exit 0, no output); after `git worktree unlock` it prunes it (`Removing worktrees/main1: gitdir file points to non-existent location`) | run |
| The `htui/<step>` branch survives `remove` and `prune` | `git branch --list 'htui/*'` |
| `git merge --no-ff --no-edit -m "<msg>" <after>` under `env -i PATH HOME=/nonexistent GIT_AUTHOR_NAME=htui GIT_AUTHOR_EMAIL=htui@localhost GIT_COMMITTER_NAME=htui GIT_COMMITTER_EMAIL=htui@localhost` → exit 0, `Merge made by the 'ort' strategy.`, a two-parent commit by `htui <htui@localhost>` with no `user.*` config anywhere | run; `git log --format='%H %p %an <%ae>' -1` |
| Conflict → exit **1**, `Auto-merging f` / `CONFLICT (content): Merge conflict in f` / `Automatic merge failed; fix conflicts and then commit the result.`; `git diff --name-only --diff-filter=U` lists `f` (the index carries the stage entries `gix` reads instead); `git merge --abort` → exit 0, tree clean, `MERGE_HEAD` gone | run |
| Held `index.lock` during `merge` → exit **1**, `error: Unable to write index.` / `Automatic merge failed; fix conflicts and then commit the result.`, `MERGE_HEAD` written, working tree untouched — indistinguishable from a conflict by status alone | planted `.git/index.lock`, run |
| Held `index.lock` on a reset-shaped command → exit 128, `fatal: Unable to create '<git_dir>/index.lock': File exists.` | `git reset --hard`, run |
| `git merge --abort` with no merge in progress → exit 128, `fatal: There is no merge to abort (MERGE_HEAD missing).`; `git merge --no-ff` of an ancestor → exit 0, `Already up to date.`, no commit | run |
| `GIT_DIR=/nonexistent` in the parent environment breaks `worktree add` (`fatal: not a git repository: '/nonexistent'`, exit 128) — the scrub of T2 is necessary | run |
| `post-checkout` runs on `worktree add` (a hook printing `HOOK-RAN` did) | run |
| **Minimum version 2.33.0**: `add --lock` since 2.13.0 (RelNotes: "`git worktree add --lock` allows to lock a worktree immediately after it's created"); `remove` since 2.17.0 (`git-worktree.txt` at v2.16.0 lists it only under BUGS as a wish, v2.17.0 documents it; v2.17.0 RelNotes: "`git worktree add` learned to run the post-checkout hook"); `--reason` with `add --lock` absent from v2.32.0's synopsis (`[--lock] [-b <new-branch>]`) and present in v2.33.0's (`[--lock [--reason <string>]]`, "With `lock` or with `add --lock`, an explanation why the working tree is locked"); "To remove a locked working tree, specify `--force` twice" is in v2.33.0's `-f` text and not in v2.17.0's | `raw.githubusercontent.com/git/git/<tag>/Documentation/{RelNotes/2.13.0.txt, RelNotes/2.17.0.txt, git-worktree.txt}` for tags v2.13.0, v2.16.0, v2.17.0, v2.32.0, v2.33.0, v2.34.0 (fetched) |
| `--diff-filter=U` ("Unmerged") exists — not used by the plan (the path list is a `gix` index read) but is the CLI oracle T2's conflict test can cross-check against | `git diff --help` |

## Claims this plan could not verify (open until T2's first commit settles them)

Five of the seven items the first draft listed here were settled by the probe above (the
`checkout::Options` fields, the restricted index, `from_tree`'s third argument, `commit_as`
without `command`, and `Conflict`'s path accessor). Two remain:

**Both were settled by the independent fact-check of 2026-09-22 and are no longer open:**

1. **`gix::index::File::write` does take `<index>.lock`** — it calls
   `gix_lock::File::acquire_to_update_resource(&self.path, gix_lock::acquire::Fail::Immediately,
   None)?` (`gix-index-0.55.0/src/file/write.rs:67-81`), the suffix is `.lock`
   (`gix-lock-24.0.0/src/lib.rs:46`), and `PermanentlyLocked`'s `Display` contains the string
   `.lock` (`gix-lock-24.0.0/src/acquire.rs:44-52`), so D39's message classifier does fire on our
   own index writes. **One correction to D39:** the acquire is `Fail::Immediately`, so `gix` never
   retries for us even transiently — the 200/400/800 ms schedule must come entirely from
   `git::with_retry`. `Fail`'s own `Display` writes `"immediately"` / `"after {secs}s"`
   (`acquire.rs:22-29`), which is not `.lock`-shaped, so that half of the classifier only ever
   fires from `PermanentlyLocked`.
2. **`edit_reference` with `Target::Symbolic` for `HEAD` compiles** as OQ-3's alternative would
   need it (`RefEdit { change: Change::Update { expected: PreviousValue::Any, new:
   Target::Symbolic("refs/heads/htui/<step>".try_into()?), .. }, name: "HEAD".try_into()?, deref:
   false }`), verified by compile probe, not by reading. ~~D23 still writes the linked worktree's
   `HEAD` file directly, so the default path does not depend on it.~~ *After OQ-1's resolution
   nothing in this plan writes a worktree `HEAD` — `git worktree add` does; the probe stands as
   a record.*

Two things the probe did **not** prove and T2's tests were to pin: ~~that the hand-written
worktree layout of D23 is one `git worktree list` accepts~~ — **moot since 2026-09-22**, `git`
writes the layout (D23) — and that `checkout` over an index behaves as expected on an
already-populated directory (`overwrite_existing = true`, `destination_is_initially_empty =
false`), which **now matters only for D35's full-index reset of a dirty copy** and is pinned by
`copy_resets_a_dirty_source` (T5) rather than by a reconcile test. One new thing no probe proves
and T2's tests must: that the three stderr phrases D39 classifies and the exit-1 conflict branch
of D25 hold for the `git` on the box under test — pinned by
`a_conflicting_merge_is_refused_with_the_path_and_aborted` and
`a_held_index_lock_inside_merge_is_aborted_and_retried`, both run against the real binary.

## Verified claims (independent fact-check, 2026-09-22)

Four checkers re-verified this plan against the tree, the pinned toolchain (`rustc 1.98.1`) and the
vendored `gix` sources, independently of the ledger above. The plan was amended where a row says
so. `gix` symbols were re-probed in a fresh crate, with the check proved live by injecting
deliberate type errors and by removing `sha1` to reproduce the documented failure.

| Claim | Verdict | Evidence |
|---|---|---|
| `gix = "0.87.1"`, `default-features = false`, D22's feature list resolves and compiles | confirmed — **obsolete as stated since 2026-09-22**: that was the ten-feature list; the six-feature list is a subset whose closure loses `gix-merge` and `gix-revision`'s `describe`/`merge_base` and is not yet probed | fresh probe crate green; `cargo tree -e features -i gix` adds nothing beyond D22's closure |
| `sha1` is required; omitting it fails inside `gix-hash` | confirmed | `error[E0004]: non-exhaustive patterns: type &Kind is non-empty`, `gix-hash-0.26.2/src/kind.rs:147` |
| `gix` 0.87.1 has no `worktree add`/`remove`/`prune` and no checkout into an existing directory (OQ-1's premise) | confirmed | `gix-0.87.1/src/repository/worktree.rs:46,65,80,93,101` is the whole surface; `src/worktree/proxy.rs:48-103` is read-only; `src/repository/checkout.rs:8` is the only `checkout` symbol; `src/clone/checkout.rs:114-143` runs with `destination_is_initially_empty = true` |
| Every symbol D23/D25/D35 intend to call exists with the claimed shape | confirmed — **partly obsolete since 2026-09-22**: `merge_commits`, `Conflict.ours.location()` and `State::remove_entries` are no longer called (D25 is `git merge`); `Repository::reference` is D26's label only; the rest stand | all compiled under D22's feature set: `gix::init`, `Repository::reference`, `State::from_tree`, `gix_worktree_state::checkout` + its `Options` fields, `index::File::write`, `is_dirty()`, `commit_as` without `command`, `merge_commits`, `Conflict.ours.location()`, `rev_walk(..).with_hidden(..)`, `find_object`/`write_object`, `State::remove_entries` |
| `from_tree`'s third argument is reachable from outside `gix` | confirmed with correction | `Repository::config` is `pub(crate)` (`gix-0.87.1/src/types.rs:172`), so `repo.config.protect_options()` — what `gix`'s own clone path uses — is **not** callable. Use `Default::default()` or a hand-built `gix_validate::path::component::Options`, which is what the ledger row already writes |
| `Conflict.ours` is `gix::diff::tree_with_rewrites::ChangeDetached` | confirmed, naming imprecise — **obsolete since 2026-09-22** (no `gix` merge is performed) | same underlying type, re-exported as `gix::object::tree::diff::ChangeDetached` (`object/tree/diff/mod.rs:11`); `.location() -> &BStr` at `gix-diff-0.67.1/src/tree_with_rewrites/change.rs:541` |
| OQ-5: `gix` fetch needs `blocking-network-client` and spawns `git-upload-pack` for a local path | confirmed, citation corrected | feature block is `gix-0.87.1/Cargo.toml:111-117` (`"dep:gix-transport"` at `:114`), not `:107-113`; `gix-transport-0.59.2/src/client/blocking_io/file.rs:393,427` builds argv containing `git-upload-pack`. **Amended in OQ-5.** |
| `gix::index::File::write` takes `<index>.lock` (previously open) | confirmed, D39 corrected | `gix-index-0.55.0/src/file/write.rs:67-81` acquires with `Fail::Immediately`; `.lock` suffix at `gix-lock-24.0.0/src/lib.rs:46`; `PermanentlyLocked`'s message contains `.lock` (`acquire.rs:44-52`). `gix` never retries for us, so the whole 200/400/800 ms schedule is `git::with_retry`'s. **Amended in the closing section.** |
| `edit_reference` with `Target::Symbolic` for `HEAD` (previously open) | confirmed | compile-probed with the exact `RefEdit` shape OQ-3's alternative needs. **Amended in the closing section.** |
| OQ-2: `command_run` exists with `output TEXT` and no seam method touches it; no `CommandRun` model type | confirmed | `crates/htui-store/migrations/0001_init.sql:537-551` (`output TEXT` at `:546`); `crates/htui-core/src/store/traits.rs:1322-1327` names it only in `Counts`; `MemStore::counts` hard-codes `command_runs: 0` (`crates/htui-core/src/store/mem.rs:2836`); only the `CommandRunId` newtype exists (`model/ids.rs:113`) |
| D31: `WriteStore` has 61 methods, so the two new ones make 63 | confirmed | trait spans `traits.rs:194-952`, 61 `async fn`; `finish_run` is the 61st at `:915` |
| D33: `run_step.isolation_path` exists and `upsert_step_tree` does not write it | confirmed | column `0001_init.sql:490`; `MemStore::upsert_step_tree` `mem.rs:3617-3627` and `PgStore`'s `INSERT` `pg/write.rs:3187-3223` both omit it |
| OQ-3/D26: `run_step_tree` is `mode, path, base_ref, dirty` plus keys | confirmed, citation corrected | real location `0003_orchestration.sql:92-100`; the file is 140 lines, so the drafted `:1951-1959` could not exist. **Amended in OQ-3.** |
| D45: `RunStatus::can_move_to` admits `cancelled` from `queued`/`running`/`awaiting_approval`; no `Command::CancelRun` yet | confirmed | `model/run.rs:60-68`; `Command` has three variants (`command.rs:37-65`) and reserves the rest at `:28-30` (not `:1-3`, **amended**) |
| D28: `identity::config_root()` exists and returns a per-user root | confirmed | `crates/htui-store/src/identity.rs:45-52`, `dirs::config_dir().join("htui")`, created on call |
| D30: `verify_outcome`/`verify_exit_code` exist and nothing writes a non-`None` value today | confirmed | columns `0003_orchestration.sql:67-69`; every caller passes `None` (`engine.rs:855`, `:869-870`; `gate.rs:219-220`'s doc says "Always `None` this milestone") |
| T1: the two `htui-agent` spies are exhaustive `WriteStore` impls needing an arm per new method | confirmed | `UsageSpy` `conformance.rs:673-1039` and `SpyStore` `recorder.rs:353-733`, 61 `async fn` each; `finish_run`'s arm at `recorder.rs:713` is the M2 precedent |
| D40: `tempfile` is available and no test needs a `git` binary | partial — **the second half is obsolete since 2026-09-22**: git-backed tests need the binary and skip without it (D40 as amended) | `tempfile = "3"` is a **per-crate** dev-dependency (`htui-core/Cargo.toml:31`, `htui-store:45`, `htui:64`, `htui-agent:70`), **not** a `[workspace.dependencies]` entry, and `htui-orch` has none — T2 adds it the same way the other crates did. No test spawns a `git` process today |
| `similar` is an idle workspace dependency | falsified | declared at `Cargo.toml:65`, but `htui-agent` depends on it (`crates/htui-agent/Cargo.toml:35`) and `unified_diff` uses it (`src/acp/fs.rs:106-118`). The manifest's "no crate consumes it yet" comment is stale. **Amended in OQ-4 and the ledger.** |
| D37: a failing `prepare` reaches the operator through `fail_hard` unchanged | confirmed, citation corrected | `prepare` at `engine.rs:791-795` is the first call after the move; `walk_step` catches at `:769-775` and `fail_hard` runs `:905`/`:912`; the pin is `an_error_after_the_running_move_fails_the_step_and_the_run` at `engine.rs:2599-2646` (**amended** from `:2603-2640`), asserting `failure == Some("isolation refused: no checkout for this repo on this box")` |
| D36: `reconcile` and `cleanup` have no production caller | confirmed, and stronger | crate-wide there is **no** call site at all — not in `engine.rs`, not in any test, not in `fake.rs`'s own tests. They exist only as trait methods (`isolate.rs:121-130`) and `FakeIsolator`'s impl (`fake.rs:199-209`) |
| OQ-4: `engine.rs:1045-1048` says both prompt sections are milestone 3's | confirmed | exact line match, including the comment "Both are milestone 3's" |
| D38's premise: a crash between `prepare` and `upsert_step_tree` leaves no row | confirmed, not conflated — **the second half is superseded since 2026-09-22**: the branch-exists failure D38 avoids is now `git worktree add -b`'s (exit 255), `PreviousValue::MustNotExist` remains D26's label | `upsert_step_tree` has no precondition on either store (`mem.rs:3617-3627` plain insert; `pg/write.rs:3187-3223` `ON CONFLICT … DO UPDATE`); `PreviousValue::MustNotExist` is the `gix`-ref concept D23's branch creation uses (`gix-ref-0.67.1/src/transaction/mod.rs:50`), which is what D38 actually attributes it to |
| D30/stage 5: there is no verify call in the walk today, and the hook lands between stages 4 and 5 | confirmed | stage comments at `engine.rs:753`, `:814`, `:833`, `:841`, `:876`; `verify_outcome: None` hard-coded at `:853-855` |
| D43: a `tokio::sync::Mutex` guard held across an await keeps the future `Send` as the trait demands | confirmed | `IsolatorFuture` is `+ Send` (`isolate.rs:32`); `Isolator: Send + Sync` (`:92`); the engine holds `&'a I` (`engine.rs:183`) across every await |
| D41: `Isolator` is public without `test-support`, so a new integration binary needs no feature | confirmed | `pub mod isolate;` is ungated (`lib.rs:24`) unlike `fake` (`:20`) and `conformance` (`:18`); current binaries are `fake_conformance.rs`, `fixtures.rs`, `review_loop.rs` |
| Count pins: orch `CASES` is 15 with two pins | confirmed | `conformance.rs:138-173`; `tests/fake_conformance.rs:14-17` (`cases_len_is_fifteen`) and `conformance.rs:1748-1760` (`cases_are_unique_and_fifteen`) |
| **Task independence — `T1 ∥ T2`** | holds | `T1 ∩ T2 = ∅`: T1 is `htui-core`/`htui-store`/`htui-agent`, T2 is `htui-orch` + root `Cargo.toml` |
| **Task independence — `T3 ∥ T4`** | holds | `{verify.rs, lib.rs} ∩ {isolate.rs, isolate/copy.rs} = ∅` |
| No hidden coupling across the two parallel pairs | confirmed | three distinct count pins (core `CASES` 48→49 and store `EXPECTED_CASES` are T1's, orch `CASES` 15→18 is T6's); only T1 regenerates `.sqlx`; no task re-records a `.snap`; `isolate.rs` is edited by T2, T4 and T5 and `lib.rs` by T3 and T5, but never by two tasks the plan runs in parallel |
| Serial edges are genuine | confirmed with one qualification | T3/T4 need T2's manifest (`htui-orch/Cargo.toml` has no `gix`/`walkdir`/`process-wrap`/`dirs` today); T5 calls T4's `copy.rs` functions; T6 needs T1, T3 and T5. The **T3 → T5 edge is file-adjacency only** — both edit `lib.rs` — not a functional dependency |
| Every task declares its file set | confirmed | all six declare one; T5's and T6's come from the Files-to-Change table rather than the `⊂` prose |
| The plan's file sets are complete | one gap | `Isolator::reconcile`'s trait doc still calls reconciliation "an identity", which D25 falsifies, and no task was assigned to fix it. **Amended: folded into T5's `isolate.rs` edit.** |

**Claims added by the OQ-1 revision (2026-09-22), each verified on this box or against the
tree before it was written:**

| Claim | Verdict | Evidence |
|---|---|---|
| `git worktree add --lock --reason <string> -b <new-branch> <path> <commit-ish>` is the flag spelling and argument order (D23) | confirmed | `git worktree --help` synopsis on this box; the exact invocation ran and produced a locked worktree on `htui/step1` at the given hash (git 2.43.0) |
| `git worktree remove --force --force <path>` removes a locked, dirty, or already-deleted tree; `git worktree prune` never touches a locked entry (D23) | confirmed | run; `git worktree --help`: "To remove a locked working tree, specify `--force` twice", `lock`: "prevent its administrative files from being pruned" |
| The admin entry id is the path basename, not `htui-<step_id>` (D23 — falsifies the drafted `worktree_proxy_by_id("htui-<step>")` lookup) | confirmed | `ls .git/worktrees` → `main`, `main1` |
| `git merge --no-ff --no-edit -m … <hash>` exits 1 on conflict and 1 on a held `index.lock`, 0 on success with a two-parent commit under the env identity and no `user.*` config; `--abort` restores (D25, D39) | confirmed | run under `env -i` and with a planted `.git/index.lock` |
| The minimum `git` for D23's exact invocation is **2.33.0** (`--reason` with `add --lock`); `add --lock` alone is 2.13, `remove` is 2.17 | confirmed | v2.32.0 vs v2.33.0 `Documentation/git-worktree.txt`; RelNotes 2.13.0 and 2.17.0; v2.16.0 `git-worktree.txt` (`remove` under BUGS only) |
| `process-wrap` is a workspace dependency at `10.0.0` with `tokio1`/`creation-flags`/`job-object`/`process-group`, consumed by `htui-agent`, not by `htui-orch` (T2) | confirmed | `Cargo.toml:59-60`; `Cargo.lock:3803-3805`; `crates/htui-agent/Cargo.toml:30`; `crates/htui-orch/Cargo.toml` (no such line) |
| `which = "8.0.6"` is a workspace dependency consumed by `htui-agent` (T2's binary lookup) | confirmed | `Cargo.toml:66`; `crates/htui-agent/Cargo.toml:32` |
| D22's feature list reduces from ten to six: `merge` freed by `merge_commits`/`tree_merge_options`/`merge_base`'s removal, `blob-diff` by `diff_tree_to_tree`'s, `revision` by `merge_base`'s (with `rev_walk` ungated), `dirwalk` never a named call; `index`, `status`, `worktree-mutation`, `sha1`, `parallel`, `max-performance-safe` each keep a named consumer | confirmed by reading, **not by compile** | `gix-0.87.1/Cargo.toml` feature blocks (ledger row above); `src/repository/mod.rs:43-44`, `:50-51`, `:59`, `:62-63`; `src/revision/mod.rs:9`; `src/repository/revision.rs:174`; T2's `cargo tree` is the compile check |
| `open_index()` needs `index`; `Stage::Unconflicted` is the stage-0 discriminant (D25's conflict path list) | confirmed | `src/repository/mod.rs:43-44`; `src/repository/index.rs:25`; `gix-index-0.55.0/src/entry/mod.rs:3-11` |
| `submodules()` needs `attributes`, which `status`'s closure supplies (mode table's refusal) | confirmed | `src/repository/mod.rs:62-63`; `Cargo.toml:241-247` → `:140-144` → `:60-68` |
| `GIT_DIR` inherited from the parent breaks `worktree add` in the managed checkout (T2's scrub) | confirmed | `GIT_DIR=/nonexistent git worktree add …` → `fatal: not a git repository: '/nonexistent'` |
| `post-checkout` runs on `worktree add` (Risks) | confirmed | a hook printing `HOOK-RAN` fired on this box; RelNotes 2.17.0 |
| `htui-store`'s skip constant is `pub const SKIP: &str = "skipped: HTUI_TEST_DATABASE_URL not set"` at `testkit.rs:35` (D40's mirror) | confirmed | `crates/htui-store/src/testkit.rs:35` |
| No CI configuration exists in the repository (D40) | confirmed | `ls -a` at the root; `README.md:510` is the only "CI" mention |
| ANA-2 rejects shelling out at exactly two lines (OQ-1's consequence 1) | confirmed | `docs/ANA-2.md:1777` (§8 crate table, "Shelling out to `git` is the third option and is rejected") and `:2055` (§10.7 row 7, "shelling out to `git` is rejected in both cases") |
| **Task independence — `T1 ∥ T2`** after the revision | still holds | T2's file set is unchanged (`Cargo.toml`, `crates/htui-orch/{Cargo.toml, src/isolate.rs, src/isolate/git.rs, src/fake.rs}`); `T1 ∩ T2 = ∅` |
| **Task independence — `T3 ∥ T4`** after the revision | still holds | neither T3 nor T4 touches `git.rs`; `{verify.rs, lib.rs} ∩ {isolate.rs, isolate/copy.rs} = ∅`; the only file-set change anywhere is *within* `git.rs`'s and `tests/gix_isolator.rs`'s row descriptions, plus `which` on two manifests T2 already owns |
