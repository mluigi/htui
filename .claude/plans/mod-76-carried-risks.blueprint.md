# Blueprint: MOD-76 carried risks (T1 R-44 `who`, T2 R-55 limits re-read)

**Plan**: `.claude/plans/mod-76-carried-risks.plan.md` (confirmed 2026-10-04). Produced by `code-architect`; one
amendment by the session (B-1) settling the architect's flagged edge case 1.

## T1: `who` (`crates/htui/src/ui/tabs/backlog/detail/runs.rs`)

- Placement: directly after `fn tail`, before `fn step_lines`.
- `fn who(agent: Option<&str>, model: Option<&str>) -> String`, doc: "MOD-76 D3 (R-44): `agent/model`, the model
  shortened before it is fitted: a trailing `-YYYYMMDD` goes, then a leading `{agent}-`, each skipped when it would
  leave the model empty. A missing agent or model is [`PENDING`], and the rules run only when both are present."
- Flow: `let (Some(agent), Some(model)) = (agent, model) else { return format!("{}/{}", agent.unwrap_or(PENDING),
  model.unwrap_or(PENDING)); };` → date rule via `model.rsplit_once('-')` keeping the head when the tail is 8 ASCII
  digits and the head is non-empty → prefix rule `model.strip_prefix(agent).and_then(|r| r.strip_prefix('-'))
  .filter(|r| !r.is_empty())`, else the model as it was → `format!("{agent}/{model}")`.
- `tail`: doc "Line 2's tail: D106's indicator, then [`who`]."; `let who = who(step.agent_name.as_deref(),
  step.model.as_deref());`; `match indicator(step)` unchanged.
- Tests first (pure ones beside `the_figure_is_thousands_with_a_bang_when_trimmed`):
  1. `the_model_loses_its_date_and_its_agent_prefix`: `claude`+`claude-sonnet-4-5-20250929` → `claude/sonnet-4-5`;
     `x`+`sonnet-4-5-20250929` → `x/sonnet-4-5`; `claude`+`claude-sonnet-4-5` → `claude/sonnet-4-5`.
  2. `a_model_no_rule_matches_or_would_empty_is_kept`: `claude/sonnet`, `agy/default`, `scripted/sonnet` unchanged;
     `claude`+`claude-` → `claude/claude-`; `claude`+`-20250929` → `claude/-20250929`; tails `-2025092`,
     `-202509290`, `-2025092x` kept.
  3. `a_missing_agent_or_model_is_a_dash`: `(None, Some("m"))` → `—/m`; `(Some("a"), None)` → `a/—`; both → `—/—`.
  4. `an_abbreviated_model_fits_beside_the_widest_indicator` (`#[tokio::test]`): step with `prompt_tokens:
     Some(35_988)`, `trimmed: true`, agent `claude`, model `claude-sonnet-4-5-20250929`, `..feat_1_runs().await[0]
     .steps[0].clone()`; `step_lines(&step, from_ref(&step), false, &Theme::default())` second line: width `PANE`,
     `trim_end()` ends with `~36k ! claude/sonnet-4-5`, no ellipsis. Also append the same step as the **last**
     `steps.push` of `extremes()` (indices 4-6 stay valid).

## T2-a: `MemStore::set_box_setting` (`crates/htui-core/src/store/mem.rs`, after `command_rows`)

```rust
/// MOD-76 D4 (R-55): sets `settings[key]` on box `id`'s row, on this store and every clone of
/// it, as no `BoxEdit` field writes it (`command_limits`). `edit_version` does not move. Whether
/// the row exists.
#[cfg(feature = "test-support")]
pub fn set_box_setting(&self, id: BoxId, key: &str, value: serde_json::Value) -> bool
```
Body: `self.write(|state| state.boxes.get_mut(&id).map(|row| row.settings[key] = value).is_some())`. Tests
`assert!` it.

## T2-b: `crates/htui-worker/src/runtime.rs`

- **B-1 (session amendment of the architect's edge case 1):** re-reading every sweep tick would repeat the
  "does not parse" `warn!` every tick. So the read is split: `stored_limits(host, box_id) ->
  StoreResult<Option<Value>>` (the `box_row` read, D216 errors passed up) and `parse_limits(box_id, stored:
  Option<Value>) -> BTreeMap<String, u32>` (default `{"verify": 1}` + the `warn!` on a bad value). The existing
  `command_limits` stays as their composition (its `testing::command_limits` wrapper and the `run_worker.rs` test
  keep their signature). `Built.limits: Option<Option<Value>>` holds the **raw stored value** the verifier was
  built from; `singletons` compares raw, and parses (and so warns) only when it builds. One warning per distinct
  stored value, as before MOD-76 (once per build). serde_json objects compare by key, not order.
- `Built` gains, after `verifier`: `/// MOD-76 D4 (R-55): the stored `command_limits` [`Self::verifier`] was built
  from (`None` inside: no key).` `limits: Option<Option<Value>>`. The server-switch reset clears it with the parts.
- `singletons`, after the unchanged repo-map rebuild block (which still returns `REPOS_MOVED` before any limits
  read, and still sets `built.verifier = None`):
  ```rust
  // MOD-76 D4 (R-55): the limits are read wherever the repo map is, so an edit to them alone
  // rebuilds the verifier. Never under a live walk: two verifiers are two `verify` semaphores
  // (verify.rs `ShellVerifier`), so the swap waits for the next call with none live.
  let stored = stored_limits(host, box_id).await.map_err(|err| err.to_string())?;
  if built.limits.as_ref() != Some(&stored) && !self.any_live() {
      built.verifier = None;
  }
  if built.verifier.is_none() {
      let limits = parse_limits(box_id, stored.clone());
      built.verifier = Some(Arc::new(ShellVerifier::new(&limits, /* scrubber, clock unchanged */)));
      built.limits = Some(stored);
  }
  ```
  The old `command_limits` call inside the `is_none` block goes (one read per call). Paths: first build builds;
  repo moved + idle rebuilds from the same read; repo moved + live → `REPOS_MOVED` first, as today; limits changed
  + live → verifier and `built.limits` stay, nothing refused, next idle call rebuilds; verifier `None` + live (only
  after a failed read or a server reset) builds as today; a failed read assigns nothing.
- Doc edits: `singletons` ("Each such call also re-reads the box's `command_limits` and rebuilds the verifier when
  they changed and no walk is live; while one is, the cached verifier stays and nothing is refused (MOD-76 D4,
  R-55). A repo-map rebuild reads `copy_max_total_bytes` afresh."); `command_limits` (drop "read once per build …
  next process (R-55) …"; say read at every walking `StartRun` and every worker sweep, rebuilt once no walk is
  live, MOD-76 D4); the rebuild-block "MOD-41 review R-1 … refresh with the repo map" comment points to the D4 read;
  `sweep_once` (~1836-1838, "with the limits read afresh") adds that a limits change alone rebuilds the verifier.
- Tests (after `a_worker_sweep_under_a_live_walk_keeps_its_parts`; setup copied from the R-1 tests: `Scratch::new()`,
  `seeded(MemStore::demo())`, `set_executor(&store, Executor::Worker)`, `RunRuntime::new(factory).with_author(
  Arc::new(OutputAuthor)).with_role(Role::Worker).with_scratch_root(..)`, `factory.register("acp",
  Box::new(OneTurn))`, `Backend::memory(store.clone())`, `Timed::new()`; one tick = `runtime.sweep_with(&backend,
  &sink); assert!(runtime.settle(PATIENCE).await.is_empty());`; read back with `parts(&runtime, &backend)`; check
  the demo box's stored limits and pick a value that differs):
  1. `a_worker_sweep_applies_a_limits_change_alone` (red): tick, parts, set, tick → verifier new (`!Arc::ptr_eq`),
     isolator same, `isolator_builds() == 1`.
  2. `a_worker_sweep_with_the_same_limits_keeps_its_verifier`: tick, parts, tick → same verifier.
  3. `a_limits_change_under_a_live_walk_waits_for_the_rest` (red second half): tick, parts, `let walk =
     runtime.shared.walks.child(RunId::new());`, set, tick; direct `runtime.shared.singletons(&backend, &writer,
     true)` is `Ok`, verifier same, `isolator_builds()` unchanged; `drop(walk)`, tick → verifier new.
  4. `a_walking_start_run_applies_a_limits_change` (red): default `Role::Tui` (no `set_executor`), `Kit::read(
     &runtime.shared, &backend, true)` → parts, set, `Kit::read(.., true)` → verifier new; optionally `Kit::read(..,
     false)` keeps it.
  5. (B-1) `a_bad_stored_limits_value_is_parsed_once`: a stored value that does not parse builds once and is not
     rebuilt on the next tick (same verifier) — pins the raw comparison.
  Unaffected: `run_worker.rs` `the_isolator_is_built_once_per_process`, `command_limits_fail_with_the_store_and_
  default_a_missing_value`.

## T2-c: `docs/htui-worker.md` lines 45-51
- "the box's command limits and `copy_max_total_bytes` are read again at that point" → "`copy_max_total_bytes` is
  read again at that point".
- "A change to the limits alone reaches the worker at its next restart." → "A change to the box's command limits
  alone reaches the worker at its next sweep with no run walking, with no restart; while a run is walking, the
  worker keeps the limits it had, and refuses nothing."

## Edge cases (architect, recorded)
1. Repeated warning — settled by B-1.
2. D216 on the new read is not unit-testable in memory (`MemFault` covers writes only); the read sits beside
   `box_row` reads that already fail the same way.
3. Server switch with walks still live: pre-existing, out of scope.
4. `Some("")` agent strips a leading `-`; harmless, agent rows are named.
