# MOD-76 - Orchestrator carried risks after MOD-37 (done, 2026-10-04)

**Requirements:** `R-ORCH-3`, `R-ORCH-8`, `R-TUI-4`.
**Origin:** MOD-37 (`docs/decisions/mod/mod-37.md` "Carried"): four risks re-deferred with reasons,
R-32, R-44, R-53 and R-55.
**Artifacts:**
- plan [`.claude/plans/mod-76-carried-risks.plan.md`](../../../.claude/plans/mod-76-carried-risks.plan.md): D1-D5, with its verified-claims table;
- blueprint `.claude/plans/mod-76-carried-risks.blueprint.md`: T1, T2-a/b/c, amendment B-1.

Decision numbers are local to MOD-76 (the MOD-31 convention).

Routed as **plan** (1 criterion fired: C3, the options R-32 and R-44 named). Run in a TOOL-7 sandbox
(`hr/MOD-76`). T1 and T2 had disjoint file sets and ran in parallel.

**Decisions (maintainer, 2026-10-04):**
- route accepted, no ultracode;
- scope per risk: R-32 closed as accepted, R-44 abbreviate the model, R-53 closed as accepted, R-55
  fixed now;
- plan confirmed as written and fact-checked (21 claims, 2 amended);
- review: H-1 and N-1-N-4 applied; L-2 fixed with a test; L-1 and L-3 accepted and recorded below.

**Commits:**
- plan and blueprint: `c40fe97e`, `d59c36ef`;
- T1 `who()` (R-44): `55aecb42` (red), `8d9bb36c`;
- T2 limits re-read (R-55): `a0e6cd85` (seam), `4f05045e` (red), `5a433ce6`, `33031d8a` (docs);
- review fixes: `a286a6c3` (H-1, N-1-N-4), `75c84524` (L-2).

---

## What was decided and built

### R-32 - closed as accepted, no code (D1)

A D93 park whose sweep crashed between `interrupt_step` and `park_interrupted` is completed by
D131's `settle_failed` with the reason `PARK_CUT_SHORT` (or `DIRTY_AT_START` for a dirty tree),
not the refusal text the sweep held in memory. The note still lists every tree's path and
`before_hash`, so recovery loses nothing, only the sentence explaining it. Keeping that sentence
needs a `run_step` column (migration 0015, cache migration 0006, `.sqlx`, and the `interrupt_step`
signature across two trait declarations and nine implementations) or a change to `gate_note`,
which `settle_failed` and four tests match exactly. That costs far more than a diagnostic in a
two-write crash window is worth, and 0015 would race the migrations of branches in flight.

### R-53 - closed as accepted, no code (D2)

`ItemActions` is as of the last `Runs` reply, but the engine re-checks every action with the same
admission function (D184) and the pane re-reads after each action (D171). What is left is the
interval between that reply and the key press, which D184 guards.

### R-44 - the model is abbreviated before it is fitted (D3)

Line 2 of a Runs step row gives `agent/model` a 24-column tail, so `claude/claude-sonnet-4-5-20250929`
was always cut. `runs.rs` `who(agent, model)` now builds that text: a trailing `-YYYYMMDD` (eight
ASCII digits) is dropped from the model, then a leading `{agent}-`, each skipped when it would
leave the model empty; a missing agent or model is still `—`. So the example reads
`claude/sonnet-4-5`, and fits beside the widest indicator (`~36k ! `) exactly. The agent name is
never dropped, an id still too long is cut with `…` as before, and row height, the indent and
D197's columns are unchanged. No snapshot moved (their ids, `claude/sonnet` and the like, match no
rule). Tests: `the_model_loses_its_date_and_its_agent_prefix`,
`a_model_no_rule_matches_or_would_empty_is_kept`, `a_missing_agent_or_model_is_a_dash`,
`an_abbreviated_model_fits_beside_the_widest_indicator`, and a new `extremes()` step under
`every_step_row_fits_forty_three_columns`.

### R-55 - the verifier follows a `command_limits` edit (D4, D5, B-1)

`Shared::singletons` already re-read the repo map at every walking `StartRun` and every worker
sweep, but read `box.settings.command_limits` only when it built the parts. Now `Built.limits`
holds the **raw stored value** the verifier was built from, and every such call re-reads it
(`stored_limits`):
- changed and no walk of the process live: the verifier is rebuilt (`parse_limits`, which warns on
  a bad value);
- changed while a walk is live: the cached verifier stays and nothing is refused; the next call
  with no walk live applies it. Two verifiers at once would be two `verify` semaphores
  (`ShellVerifier`), so the class could exceed its limit;
- the repo-map rebuild keeps its `REPOS_MOVED` refusal (D202), which still returns before the
  limits read; a failed read assigns nothing (D216); a server switch clears `limits` with the parts.

B-1 (session amendment of the architect's flagged edge case): comparing the raw `Value` rather than
the parsed map means a value that does not parse warns once per distinct stored value, as it did
once per build before, instead of every 5 s sweep tick. serde_json objects compare by key
(`preserve_order` is on, and `IndexMap` equality ignores order).

So the TUI applies an edit at its next walking `StartRun`, and `htui worker` at its next sweep with
no run walking, with no restart either way (`docs/htui-worker.md` updated). No editor was added
(D5): R-55's trigger had not arrived, and a future editor needs nothing beyond its write.

`MemStore::set_box_setting` (`test-support`) is the test seam, since no `BoxEdit` field writes
`command_limits`. `RunRuntime::verifier_builds` sits beside `isolator_builds` as D156's test hook.
Tests: `a_worker_sweep_applies_a_limits_change_alone`,
`a_worker_sweep_with_the_same_limits_keeps_its_verifier`,
`a_limits_change_under_a_live_walk_waits_for_the_rest`, `a_walking_start_run_applies_a_limits_change`,
`a_bad_stored_limits_value_is_parsed_once` (`htui-worker` `role_gate`),
`a_box_setting_is_set_on_every_clone` (`htui-core`), and
`a_walking_start_run_applies_a_command_limits_edit` (`htui` `run_worker`, L-2: two real
`StartRun`s around an edit; mutation-checked, it fails when "a walk is live" is forced).

### Review

`rust-reviewer`: approve-with-fixes.
- **H-1** (fixed, `a286a6c3`): `singletons` reading `stored_limits`/`parse_limits` directly left the
  private `command_limits` dead in a featureless build, so `cargo clippy --workspace -- -D warnings`
  failed (the `--all-features` gate hid it). `testing::command_limits` composes the two itself now.
- **N-1-N-4** (fixed): `parse_limits` takes `Option<&Value>`, so the comparison path never clones;
  a test-helper doc; a `# Panics` note on the seam; over-width lines.
- **L-2** (fixed, `75c84524`): the direct `Kit::read` test could not catch a `start_run` that minted
  its walk first; the real-`StartRun` case above does.

## Gates

`cargo fmt --all -- --check`, `cargo clippy --workspace -- -D warnings` and
`cargo clippy --workspace --all-targets --all-features -- -D warnings` clean. Before the review
fixes: `htui-core` 606, `htui` 2198 (`--all-features --test-threads=1 --no-fail-fast`), `htui-worker`
19 passed, 0 failed. After them: `htui-worker` 19, `htui` `run_worker` 74 passed. No migration, no
`.sqlx`.

## Carried

Accepted with the maintainer, recorded here only:
- **L-1** (LOW): a TUI `StartRun` reads its parts in `Kit::read` and mints its walk only after
  `enqueue`. A second `StartRun` in that window sees no live walk and can rebuild the verifier, so
  two `verify` semaphores overlap for the first walk. `REPOS_MOVED` (D202) has the same window for
  the isolator, and limits are throughput, not correctness. A real fix mints the walk under the
  `built` lock.
- **L-3** (LOW): the limits read is one more `box_row` read right after `Kit::read`'s own; removing
  it means passing the settings into `singletons`.
- A server switch with old-server walks still live can hold a new server's verifier beside theirs
  (pre-existing, unchanged by MOD-76).
