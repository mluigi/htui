# Blueprint: MOD-66, per-box manual tool paths in Settings > Agents

**Status**: PROPOSED 2026-10-01 by the code-architect, from the plan the maintainer confirmed the same
day. Decisions B1–B18 (§8), hazards H-1–H-24 (§7) and findings F-1–F-8 (§0.2) are this
blueprint's and are numbered locally (the MOD-31 convention). They are cited elsewhere as "MOD-66 B4".
Where §0.2 amends the plan, this file wins. Where it only records a defect for the maintainer (F-3),
the plan's text is built as written.

**Plan**: `.claude/plans/mod-66-manual-tool-paths.plan.md` (CONFIRMED, fact-checked: 18 claim
groups). Its D1–D12 are binding and are not reopened here. This blueprint makes the three calls the
plan left open. **D9**: a new `pub` entry point, not a synthesized `existing` (B1). **D11**: the
field type (B7, B8). **D8**: how `probe()` learns that a tool-paths write is running (B4).

**Verified at**: HEAD `82597da` on `hr/MOD-66`. The plan was written at this commit, so its line
numbers are HEAD's. **Line numbers here are pre-edit.** After a task's first commit, a citation into
a file that task edits has moved. Counts at HEAD: `StoreRequest` **92** variants (arms of `name()`,
`store_worker.rs:844-953`), `crates/htui/tests/snapshots/` **126** files, `ProbeSnapshot { … }`
literals **10** (9 without `..`). `df -h /`: 57 GB free (87 % used). `pg_isready -h localhost -p
5439`: accepting. `cargo-insta` 1.48.0 is installed.

**Tooling**: Gortex answered (`search`/`read`) despite its INACTIVE banner, as project memory says it
does. Every behaviour-critical body was read in full with `sed -n`. That covers `probe_tools`,
`resolve_tool`, `is_file`, `ProbeSnapshot`, `recorded_launch`, `probe_agent`, `snapshot_for`,
`agent_box_row`, `launch_from`, `AgentRuntime::{serve, probe, probe_box, auth_start,
claim_is_free, sweep_finished, finish_background}`, `Background`/`Writes`, `ReprobeClaims`,
`run_reprobe`, the staleness spawn in `start`, `try_serve`, the store-loop runtime list,
`Harness::drive`, `agent_settings::{serve, REQUEST_NAMES, AgentWrite, Refusal}` and
`AgentsSection::{on_box_cell, hint, refuse_write, send, on_editor_key, submit, on_written, on_key,
on_reply, on_paste, captures_input, render_table}`, plus `Editor::lines`.

**Coupling verdict**: the plan's waves stand. **T1 ∥ T2 → T3 ∥ T4.** The file sets are re-verified
in §6, and no file moves between tasks. One file is added to T1, which T2 does not touch:
`crates/htui-agent/src/launch.rs`, for documentation only (F-1).

**Scope at a glance**:
- No migration, no `.sqlx`, no store trait, no conformance case, no `htui-orch` change (D1, D12).
- `ProbeSnapshot` +1 trailing field (`manual`). `ToolReport` +1 field (`manual`, B2). Two new `pub`
  functions in `htui_agent::probe`: `probe_tools_with` (D4) and `probe_snapshot` (B1). `is_file`
  becomes `pub` (D2).
- `StoreRequest` 92 → **93** (`SetToolPaths`). `StoreReply` is unchanged. `AgentWrite` +1
  (`ToolPaths`). `agent_settings::REQUEST_NAMES` **stays 3** (D10).
- Snapshots 126 → **127**. Seven move (hint line only) and one is new
  (`settings__agents_tool_paths_form`).

**House style (carried from MOD-23)**: `unsafe_code = "forbid"`. `missing_docs` applies on lib
roots. Clippy `all` runs at `-D warnings`, with pedantic off. **Rustdoc denies broken and private
intra-doc links** (`Cargo.toml` `[workspace.lints.rustdoc]`), so a `pub` item's doc must not link a
private item (H-3). `max_width = 100`. Every new `pub` item has a doc and `Debug`. Red tests come
first. A `todo!()` body goes only where no existing path calls it. Every commit compiles. No test is
loosened, and a moved pin names its reason in the assertion message. Implementers stage their own
paths only: never `git add -A`, never `stash`, never `--amend`. Nothing branches on an agent's name
(`R-AGT-5`), and no new file, test or comment may contain the word `zeta`
(`htui-agent/tests/extensibility.rs` sweeps the tree for it).

---

## 0. Environment, findings

### 0.1 Gates (this sandbox)

`HTUI_TEST_DATABASE_URL` and `USERNAME` are already set by the sandbox. `.cargo/config.toml` sets
`SQLX_OFFLINE=true`. No task touches SQL. Every test command runs `--test-threads=1`, because the
htui suite's green depends on scheduling (the keyring fake is process-wide). Integration tests need
`--all-features`: without `testkit`, `tests/*.rs` run 0 tests and report ok.

| Crate | Command |
|---|---|
| htui-agent | `cargo test -p htui-agent --all-features -- --test-threads=1` |
| htui (lib) | `cargo test -p htui --all-features --lib -- --test-threads=1 <filters…>` (libtest takes several filters) |
| htui (one binary) | `cargo test -p htui --all-features --test settings -- --test-threads=1` |
| all | `cargo fmt --all -- --check`; `cargo check --workspace --all-features --all-targets`; `cargo clippy --workspace --all-features --all-targets -- -D warnings` |

### 0.2 Findings: where the plan is wrong or silent

| # | Severity | Task | Plan says | Tree | Fix |
|---|---|---|---|---|---|
| F-1 | Minor | T1 | D12: "documented on `launch_from`". `Files to Change` omits `launch.rs`. | `launch_from` is at `htui-agent/src/launch.rs:527`. The module doc (`:6`) and `ToolMap`'s doc (`:58`) repeat the stale claim "filled from `agent_box.probe.tools`", the same claim `agent.rs:42` makes. | T1 adds `crates/htui-agent/src/launch.rs` (docs only). T2 does not touch it, so the wave stays disjoint. |
| F-2 | **Major** (gate) | T1 | D2: "`probe::is_file`, made `pub`". | `is_file`'s doc (`probe.rs:575-588`) links ``[`exists`]``, a private fn, and says "`pub(crate)` for [`crate::acp::AcpDriver::launch_for`]". Once the item is `pub`, `cargo doc` fails `private_intra_doc_links = "deny"`. | T1 rewrites the doc (§2.1 (d)): `exists` becomes a plain code span, and the visibility sentence names both callers. The T1 gate runs `cargo doc -p htui-agent --no-deps --all-features`. |
| F-3 | **Major** (UX, escalated) | T4 | D10: "the `on this box` cell appends ` (manual)` for every status". | The column is a fixed `Length(13)` at **every** width (`agents.rs:1676-1686`, MOD-2 D89's donor column). Measured cells: `missing (manual)` 16 → `missing (manu`. `failed (manual)` → `failed (manua`. `0.48.0 (manual)` → `0.48.0 (manua`. `unauthenticated (manual)` → `unauthenticat`, so **no marker shows**. `0.48.0 (off) (manual)` → `0.48.0 (off)`, so **no marker shows**. | Built as D10 says (settled). The tests assert through the suite's `as_drawn` helper (`tests/settings.rs:1248`), whose doc already treats clipping as D89's stated price. **Maintainer call recorded, not taken**: either accept, or follow up with a shorter marker or a re-packed column. |
| F-4 | Minor | T2 | T2 Validate: `--lib store_worker` only. | T2's tests also live in `agent_settings.rs` and `testkit.rs` `mod tests`. | Gate: `--lib -- --test-threads=1 store_worker agent_settings testkit` (§3.6). |
| F-5 | Minor | T2→T3 | T2's routing test: "`SetToolPaths` reaches the agent runtime". | `AgentRuntime::serve` ends in `other => Failed { "not a chat request" }` (`agent_worker.rs:1175-1178`). Between T2 and T3 the runtime answers with that sentence. After T3 it answers with the handler's own sentence. | The test is written to hold in both commits (B13): a negative test on the two `no agent runtime in this …` sentences, plus a positive test through a harness with **no** runtime. |
| F-6 | Minor | T3 | D9: "a synthesized minimal `existing` carrying the map" (one of two options). | `ProbeSnapshot::from_row` needs `transport`, `tools` and `status` (no `serde(default)`, `probe.rs:1091-1118`). A "minimal" document that missed one would parse to `None`, and the map would be dropped **silently**. | B1 takes the other option the plan allows. |
| F-7 | Minor | T3 | D8: the claim set. | `claim_is_free`'s background sentence is "a probe or a re-probe is already writing this box; **install** once it has finished" (`:1764-1768`). A login, a box probe or a tool-paths write refused by a running tool-paths write would read that. | B5 gives the tool-paths write its own sentence, checked first. |
| F-8 | Trivial | T4 | "`app/mod.rs:121`" | The Backlog `m` binding is at `:123`. | — |

The rest of the plan checked out: the 10/9 literal sites, the 8+1+1 `probe_tools` call sites, the
three routing places (one exhaustive, two wildcard), `on_written` as the only exhaustive
`AgentWrite` match (all other uses are tests), `REQUEST_NAMES`/`:2207`, `HINT_IDLE` at 79 → 89
columns, the 7 snapshots (the only `*.snap` files in the tree containing `a authenticate`), `m`
unbound in Settings, and `p` bound only during a login.

---

## 1. Build order and validation at a glance

| Task | Files (§6) | Commits | Gate |
|---|---|---|---|
| T1 probe: manual tier, rule | htui-agent ×6, htui-core ×1, htui ×3 (literals only) | 3 (§2.5) | §2.6 |
| T2 request, outcome, routing | htui ×4 | 2 (§3.5) | §3.6 |
| merge wave 1 | — | T1, then T2 | `cargo check --workspace --all-features --all-targets`; the T1 and T2 gates on the merged tree |
| T3 runtime handler | `agent_worker.rs` | 2 (§4.6) | §4.7 |
| T4 Settings form | `agents.rs`, `tests/settings.rs`, snapshots | 2 (§5.6) | §5.7 |
| merge wave 2 | — | T3, then T4 | §9 workspace gate |

---

## 2. T1: probe, manual tier and resolution rule (D1–D6; B1–B3, B11; F-1, F-2)

**First failing test**: `tests/probe.rs::a_manual_path_resolves_a_tool_nothing_else_finds`.

**Files**: `crates/htui-agent/src/probe.rs`, `src/lib.rs`, `src/launch.rs` (docs, F-1),
`tests/probe.rs`, `tests/acp_driver.rs`, `tests/auth.rs`; `crates/htui-core/src/model/agent.rs`
(doc); `crates/htui/src/agent_worker.rs` (**the literal at `:7241` only**),
`crates/htui/tests/auth.rs` (`:205`), `crates/htui/tests/settings.rs` (`:1909`).

### 2.1 `probe.rs`

**(a) `ToolReport` (`:241-248`)**: a third field, after `missing` (B2):

```rust
    /// Names that resolved through this box's **manual path** (MOD-66 D2), in `BTreeMap` order:
    /// a subset of `found`'s keys. Non-empty makes the snapshot's `source`
    /// [`ProbeSource::Manual`] (D4): a human decided at least part of this launch.
    pub manual: Vec<String>,
```
`ToolReport` derives `Default`, and no literal exists outside `probe.rs`. The two `ToolReport`
constructions are `ToolReport::default()` (`:431`) and the test's `assert_eq!(report,
Default::default())` (`tests/probe.rs:1187`). Both stay valid.

**(b) `ProbeSource` doc (`:316-327`)**: replace the type doc and the `Manual` doc:

```rust
    /// `probe.source` (plan D45): who decided the launch. `manual` when at least one tool resolved
    /// through this box's manual path (MOD-66 D4), or in a row a human wrote by hand (ANA-4
    /// §4.6's recipe, honoured by the same rules). A `manual` row survives a probe that finds
    /// nothing (plan D51; MOD-66 D5).
    …
        /// At least one tool came from `probe.manual`, or the row was hand-written.
        Manual => "manual",
```

**(c) The tier walk.** `probe_tools` keeps its signature and its doc's first paragraph, and becomes
a delegate:

```rust
pub async fn probe_tools(discovery: Option<&Discovery>, env: &ProbeEnv) -> Result<ToolReport> {
    probe_tools_with(discovery, &BTreeMap::new(), env).await
}

/// [`probe_tools`] with this box's manual paths (MOD-66 D2, D3). Per tool, in this order:
///
/// 1. `HTUI_TOOL_<NAME>`, unchanged: checked with `exists`, recorded with no version, no args.
/// 2. `manual[name]`, when present: it must be an **absolute** path (B3) to a **file**
///    ([`is_file`]). Then it is found with `version: None`, `below_min: false`, and, for a
///    [`ToolProbe::Glob`] tool, the matching platform's `args` (D3). Otherwise it is
///    **missing**, with a `warn!` naming the tool. It never falls through to step 3, because
///    that would silently replace the human's choice.
/// 3. The tool's own `discovery` probe ([`resolve_tool`]), unchanged.
///
/// A key of `manual` that `discovery.tools` does not declare is ignored here, and carried by the
/// snapshot as-is (D4).
///
/// # Errors
/// As [`resolve_tool`].
pub async fn probe_tools_with(
    discovery: Option<&Discovery>,
    manual: &BTreeMap<String, String>,
    env: &ProbeEnv,
) -> Result<ToolReport>
```

The body is today's `probe_tools` body. After the override block's `continue`, insert:

```rust
        if let Some(value) = manual.get(name) {
            let path = PathBuf::from(value);
            if path.is_absolute() && is_file(&path).await {
                let args = match probe {
                    ToolProbe::Glob { platform, .. } => platform
                        .get(&env.platform)
                        .map(|entry| entry.args.clone())
                        .unwrap_or_default(),
                    ToolProbe::Path { .. } | ToolProbe::NodePackage { .. } => Vec::new(),
                };
                report.found.insert(name.clone(), ToolResolution { path, version: None, args, below_min: false });
                report.manual.push(name.clone());
            } else {
                warn!(tool = name, path = value, "the manual path is not an absolute path to a file on this box");
                report.missing.push(name.clone());
            }
            continue;
        }
```
The `match` is exhaustive on purpose, so a fourth `ToolProbe` kind has to decide its args here.
A path is not a secret (`R-SEC-2`). The override's `warn!` already logs one.

**(d) `is_file` (`:575-591`)**: `pub(crate)` becomes `pub`. Doc rewrite (F-2): keep the
substance, write `exists` as a plain code span (no link), and replace the visibility sentence with
"`pub` for its three callers: D58's disk check in `launch_from`, the manual tier of
[`probe_tools_with`] (MOD-66 D2), and the `SetToolPaths` worker check in `htui` (MOD-66 D9)."
`launch_from` stays a code span: it is `pub(crate)`, which is a private link from a `pub` item.

**(e) `ProbeSnapshot` (`:1091-1118`)**: one field, **after `source`**, so that it is last (H-1):

```rust
    /// MOD-66 D1: `${tool}` name → the absolute path a human gave this box for it. Written by
    /// `SetToolPaths`, read by every probe as a resolution tier ([`probe_tools_with`]), and
    /// carried forward by every snapshot the probe writes. Absent in older documents, and absent
    /// when empty, so a row with no manual path keeps its exact bytes.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub manual: BTreeMap<String, String>,
```

**(f) `recorded_launch` (`:1145-1186`)**: delete the `source` check (`:1175-1177`). With two
variants, accepting both is no check at all. The doc's first rule becomes:

> - `source` is either. Since MOD-66 (D6) a `manual` snapshot is one the probe wrote over a human's
>   path. It resolved, and, where tier 2 ran, it answered, exactly as a `probe` one did. A
>   hand-written `manual` row (ANA-4 §4.6's recipe) is honoured by the same two rules below.
>   MOD-2 blueprint H-5 is superseded.

"Three rules" becomes "two rules". The fourth paragraph (the disk check) is unchanged.

**(g) `ProbeOutcome::Kept` doc (`:1295-1301`)**: "Plan D51: the stored row is `source: manual` and
this probe **resolved nothing**". Since MOD-66 such a row is usually one `SetToolPaths` wrote, so a
manual path that vanished, with nothing else resolving, keeps the last good row and its map (D5).
`SetToolPaths` itself never reaches this arm, because it calls [`probe_snapshot`] (B1). The
sentence about `probed_at` stays.

**(h) `snapshot_for` → `probe_snapshot` (B1)**:

```rust
/// Steps 1–7 of [`probe_agent`] over `manual`, with no stored row: the whole answer, and never
/// [`ProbeOutcome::Kept`]. A caller that was **asked** to write gets a row
/// ([`agent_box_row`]): `htui`'s `SetToolPaths` (MOD-66 D9). Every snapshot carries `manual`
/// verbatim. `source` is [`ProbeSource::Manual`] exactly when the tool walk used a manual path
/// (D4), and [`ProbeSource::Probe`] for the three failures that end before the walk.
pub async fn probe_snapshot(
    agent: &Agent,
    manual: &BTreeMap<String, String>,
    ctx: &ProbeContext,
    tier2: &dyn Tier2,
) -> ProbeSnapshot
```

Body changes, and nothing else:
- The `blank` closure gains a `source: ProbeSource` parameter, and its literal carries
  `manual: manual.clone()`. The three pre-walk calls (launch parse `:1385`, credential `:1406`,
  `probe_tools` error `:1420`) pass `ProbeSource::Probe`. The two `Missing` calls (`:1434`, `:1444`)
  pass `source`.
- `probe_tools(launch.discovery.as_ref(), &ctx.env)` becomes
  `probe_tools_with(launch.discovery.as_ref(), manual, &ctx.env)`.
- Right after the report:
  `let source = if report.manual.is_empty() { ProbeSource::Probe } else { ProbeSource::Manual };`
- The three full literals (`:1454`, `:1467`, `:1477`) take `source` and `manual: manual.clone()`.

**(i) `probe_agent` (`:1334-1358`)**: the signature is unchanged (D4: no caller changes).

```rust
    let stored = existing.and_then(ProbeSnapshot::from_row);
    let manual = stored.as_ref().map(|s| s.manual.clone()).unwrap_or_default();
    let snapshot = probe_snapshot(agent, &manual, ctx, tier2).await;
    if snapshot.resolved.is_none()
        && stored.is_some_and(|stored| stored.source == ProbeSource::Manual)
    { return ProbeOutcome::Kept { reason: MANUAL_KEPT }; }
    ProbeOutcome::Row(agent_box_row(agent, box_id, &snapshot, ctx.now))
```
Steps 3 and 8 of the doc gain "(over `existing`'s `manual`, MOD-66 D4)". Step 8's last sentence
becomes "A refreshed row says `source: probe`, or `manual` when a manual path decided a tool".

### 2.2 `lib.rs`, `launch.rs`, `agent.rs`

- `lib.rs:173-177`: add `probe_snapshot, probe_tools_with` to `pub use probe::{…}`. **Not**
  `is_file` (B11): the bare name at the root says nothing, the convention `lib.rs:135-157` spells
  out, and `htui_agent::probe::is_file` is reachable because `pub mod probe` is.
- `launch.rs:6` and `:58`: "filled from `agent_box.probe.tools`" becomes "filled by the probe
  (`ToolReport::tool_map`) or by `tools::resolve`". `launch_from`'s doc (`:499-525`) gains a
  paragraph: "**No manual tier here** (MOD-66 D2, D12). A box's manual paths reach a spawn through
  the recording, which the probe made over them. When the recording is gone or no longer a file,
  the fallback is `tools::resolve`, which walks the `HTUI_TOOL_*` override and the discovery
  tiers only. A recording that vanished usually means the manual path vanished too."
- `htui-core/src/model/agent.rs:41-42`: "resolved per box from `agent_box.probe.tools`" becomes
  "resolved per box: a spawn uses the launch the probe recorded in `agent_box.probe.resolved`
  (the box's manual paths included, MOD-66), else resolves the row's tools then and there". This is
  a plain doc with no link, because `htui-core` does not depend on the driver crate.

### 2.3 The nine literals elsewhere

Add `manual: BTreeMap::new()` (or `std::collections::BTreeMap::new()`, matching the `tools` line
beside it) at: `tests/probe.rs:2426`, `tests/acp_driver.rs:529`, `tests/auth.rs:1218`,
`crates/htui/src/agent_worker.rs:7241`, `crates/htui/tests/auth.rs:205`,
`crates/htui/tests/settings.rs:1909`. `acp_driver.rs:833` uses `..snapshot(…)` and needs nothing.
The four in `probe.rs` are §2.1 (h).

### 2.4 Tests (first)

`crates/htui-agent/tests/probe.rs` gets a new section, `// 21. MOD-66: the manual tier`, after
section 20. It adds one helper, `manual_row(tools: Value, handshake: bool) -> Agent`, which is
`synthetic_row` with `launch = {"command": "${first}", "args": [], "env": {}, "discovery":
{"tools": tools, "handshake": handshake}}`, plus `fn map(pairs: &[(&str, &Path)]) ->
BTreeMap<String, String>`.

| Test | Asserts |
|---|---|
| `a_manual_path_resolves_a_tool_nothing_else_finds` | `path_probe(["htui-no-such-binary-66"])`, manual → a plain file under `tmp` (absolute). `found["node"].path` is that file, `version == None`, `args` is empty, `report.manual == ["node"]`, `missing` is empty. |
| `the_env_override_beats_a_manual_path` | Override → file A, manual → file B. `found` is A, and `report.manual` is empty (D2's order). |
| `a_manual_path_that_is_gone_a_directory_or_relative_is_missing_and_never_falls_through` | `PATH` holds an executable `node` the discovery tier **would** find. Manual → (i) a non-existent absolute path, (ii) a directory, (iii) `"bin/node"` (relative, B3). Each gives `missing == ["node"]`, no `found["node"]`, and empty `manual`. |
| `a_manual_glob_tool_keeps_its_platform_args` | `agy_discovery()`, manual `agy_acp_server` → a file outside every seed pattern. On `env()` (linux-x86_64), `args == ["--uid="]`. On `platform: "darwin-aarch64"`, `args` is empty (D3). |
| `probe_tools_is_probe_tools_with_no_manual_paths` | Over the same discovery and env, the two reports are `==`. |
| `probe_agent_carries_the_stored_map_and_marks_the_row_manual` | `manual_row` with `first` unresolvable and `handshake: false`. `existing` is a row whose snapshot has `manual = {first: file}` (built through a literal and `agent_box_row`). The result is a `Row` with `status: ready`, `source: manual`, `manual == {first: file}`, `resolved.command == file`, `enabled`. |
| `source_is_manual_only_when_a_manual_path_decided_a_tool` | Three snapshots with `probe_snapshot`. A map naming a tool the override also names gives `Probe`, with the map still carried. An empty map over a resolvable row gives `Probe`, with no `manual` key in `to_value()`. A used manual path gives `Manual`. |
| `a_vanished_manual_path_with_nothing_else_resolving_is_kept` | `existing` is `source: manual`, `manual = {first: <deleted file>}`, and `first` resolves nowhere: the result is `Kept` (D5). The same `existing` with the file present gives a `Row`. |
| `probe_snapshot_never_keeps` | `probe_snapshot` over a map to a deleted file gives a `Missing` snapshot that carries the map, with `resolved: None` (B1: there is no `existing` to keep). |
| `a_snapshot_without_manual_round_trips_byte_for_byte` | A **current-shape** document text in ANA-4 order with `"credential":null` and no `manual` key: `serde_json::to_string(&from_str(text)) == text`. (A pre-`credential` document gains `credential:null` whatever this item does, which is why the input carries it.) |
| `an_empty_map_adds_no_key_and_a_full_one_sorts_last` | An empty map: `!text.contains("\"manual\"")`. A non-empty map: `"manual":` appears **after** `"source":` (the sequential-`find` style of `:1972`). |
| `recorded_launch_applies_d58s_row_side_rules` (renamed from `…_three_…`, `:2453`) | The `manual` arm flips to `Some(&recorded())`, with the message `MOD-66 D6: a manual snapshot is the probe's own, over a human's path`. The other arms are unchanged. |

`a_probe_snapshot_orders_keys_as_ana4_does` (`:1972`) and
`a_manual_entry_survives_a_probe_that_finds_nothing_and_is_refreshed_by_one_that_does` (`:1784`)
must stay green unchanged. They are the D5 and H-1 pins.

`crates/htui-agent/tests/acp_driver.rs`:

| Test | Asserts |
|---|---|
| `a_manual_snapshot_is_used_as_a_launch` (renamed and flipped, `:782`) | `launch.command == recorded.command` (the plain file), and `args` contains `MARKER`. The doc cites MOD-66 D6 and drops H-5. |
| `a_manual_path_reaches_the_launch_through_the_probe` | `unresolvable_row()` (its tool is on no `PATH`). A local `struct Answers;` implements `Tier2` and answers `Handshake { auth_methods: vec![], .. }`. `probe_snapshot(&agent, &{tool: plain_file}, &ctx, &Answers)` then `agent_box_row`. `AcpDriver::from_row_with_probe(…).launch_for(spec)` gives `command == plain_file`. This is the plan's acceptance criterion "a per-box manual path reaches a spawn". |

### 2.5 Commits (T1)

1. **(a) field and fix-ups, green**: `ProbeSnapshot.manual` with serde attrs, every literal (§2.3
   plus the four in `probe.rs` as `BTreeMap::new()`), `is_file` `pub` with its doc (F-2),
   `ToolReport.manual` (always empty for now).
   `docs(mod-66): …`-style messages are not used for code: `feat(mod-66): ProbeSnapshot.manual and literal fix-ups`.
2. **(b) red**: every test in §2.4, plus `probe_tools_with` and `probe_snapshot` with their final
   signatures and `todo!()` bodies. No existing path calls them yet, so nothing that is green goes
   red except the two flipped pins and the new cases.
3. **(c) green**: the bodies, `probe_tools` and `probe_agent` delegating, `recorded_launch`, and
   every doc in §2.1 and §2.2.

### 2.6 Gate (T1)

```bash
cargo fmt --all -- --check
cargo test -p htui-agent --all-features -- --test-threads=1
cargo check --workspace --all-features --all-targets
cargo test -p htui --all-features --test settings --test auth -- --test-threads=1   # the literal sites run
cargo clippy -p htui-agent -p htui-core --all-features --all-targets -- -D warnings
cargo doc -p htui-agent --no-deps --all-features                                     # F-2: private_intra_doc_links
```

---

## 3. T2: request, outcome and routing (D7, D10's names; B10, B12, B13; F-4, F-5)

**First failing test**: `agent_settings::tests::parse_tool_path_refuses_empty_and_relative_naming_the_tool`.

**Files**: `crates/htui/src/store_worker.rs`, `src/testkit.rs`, `src/agent_settings.rs`,
`src/ui/tabs/settings/agents.rs` (the compile-only arm).

### 3.1 `store_worker.rs`

The variant goes right after `SetAgentOnBox` (`:283-292`):

```rust
    /// Set this box's manual tool paths for one agent (MOD-66 D7): `agent_box.probe.manual`, by a
    /// probe of the row over them. Served by the **agent runtime's own task**, because the probe
    /// may spawn tier 2. Answered exactly once, with [`StoreReply::AgentWritten`] (`ToolPaths`),
    /// or with [`StoreReply::Failed`] before anything is spawned. The write always lands: no
    /// manual-row rule can swallow it (D9). An empty map clears every manual path. Offline:
    /// `REGISTRY_ON_SERVER_ONLY`.
    SetToolPaths {
        /// The agent.
        agent_id: AgentId,
        /// `${tool}` name → an absolute path to a file on this box, for names the row's
        /// `discovery.tools` declares. Paths, not secrets (`R-SEC-2`).
        paths: BTreeMap<String, String>,
    },
```

- `name()` (`:873`): add `Self::SetToolPaths { .. } => agent_settings::SET_TOOL_PATHS,` after the
  three `agent_settings::REQUEST_NAMES` arms, with the comment "served by the agent runtime, not
  `agent_settings::serve` (MOD-66 D7)" (B12, the `PROMPT_PREVIEW` precedent at `:860`).
- `try_serve` (`:1469-1493`): add `| StoreRequest::SetToolPaths { .. }` to the "no agent runtime in
  this build" group. The comment at `:1469-1474` says "the probes, the preview, the three install
  requests, MOD-21's four login ones, MOD-22's delivery **and MOD-66's tool-paths write** … all
  seventeen". This `match` has no wildcard, so the compiler forces the arm (H-4).
- Store loop (`:2129-2144`): add `| StoreRequest::SetToolPaths { .. }` to the runtime list, and
  update the comment at `:2121-2128` ("the probes, the tool-paths write, the installs and the
  logins"). **There is a wildcard below (`other => try_serve`), so forgetting this line compiles
  and answers "no agent runtime in this build" (H-4).**
- `StoreReply::AgentWritten` doc (`:1179-1182`): "The answer to every agent registry write (MOD-23
  D240) **and to `SetToolPaths` (MOD-66 D7)**".

### 3.2 `testkit.rs`

Add `| StoreRequest::SetToolPaths { .. }` to the runtime-served tuple (`:280-295`). In
`drive`'s doc (`:248-252`), name MOD-66's tool-paths write beside MOD-20 and MOD-21 ("without it,
`Settings > m` … would be answered 'no agent runtime in this harness' by a harness that has one").
**Wildcard below**: forgetting it compiles, and the request falls through to `store_worker::serve`,
which answers "no agent runtime in this build" (H-4).

### 3.3 `agent_settings.rs`

- Module doc (`:1-8`): add a sentence: "MOD-66 adds `SetToolPaths`'s outcome
  ([`AgentWrite::ToolPaths`]) and its one pure rule ([`parse_tool_path`]). The request is
  served by the agent runtime, not by [`serve`]."
- After `REQUEST_NAMES` (`:480`):

```rust
/// `StoreRequest::SetToolPaths`'s name (MOD-66 D7, D10). **Not** in [`REQUEST_NAMES`]: those three
/// are served by [`serve`] in the store loop and pinned by `store_worker`'s naming test. This one
/// is the agent runtime's. The section's `Failed` match accepts it beside them.
pub const SET_TOOL_PATHS: &str = "set_tool_paths";

/// One manual path as the form and the worker check it (MOD-66 D9, D10): trimmed, non-empty and
/// absolute. The refusal is one sentence naming the tool, `` `<tool>`: <reason> `` (the
/// [`Refusal`] shape, with a runtime name, D11). Pure: whether the path is a **file** is the
/// worker's check, because it is I/O.
///
/// # Errors
/// `` `<tool>`: the path is empty `` or `` `<tool>`: the path must be absolute ``.
pub fn parse_tool_path(tool: &str, text: &str) -> Result<String, String>
```

- `AgentWrite` (`:485-522`) gains a last variant, and the type doc gains "and the probe's status
  word":

```rust
    /// This box's manual tool paths for the agent were written, and the row was probed over them
    /// (MOD-66 D7). No path here (`R-SEC-2` in spirit: the reply carries no launch).
    ToolPaths {
        /// The agent.
        id: AgentId,
        /// The agent's name, from the re-read.
        name: String,
        /// The probe's verdict over the new paths (`agent_box.probe.status`).
        status: htui_agent::probe::ProbeStatus,
    },
```
`AgentWrite` derives `Eq`, and `ProbeStatus` derives it too (`wire_enum!`, `lib.rs:70-72`).

### 3.4 `agents.rs` (compile only)

`on_written` (`:1465`) is exhaustive, so T2 compiles only with an arm (H-5). Add it, plus a
private helper that T4 keeps:

```rust
            AgentWrite::ToolPaths { name, status, .. } => {
                self.notice = Some(tool_paths_saved(name, *status));
            }
…
/// What `AgentWritten::ToolPaths` says (MOD-66 D10): the row and the probe's word, never a path.
fn tool_paths_saved(name: &str, status: ProbeStatus) -> String {
    format!("tool paths saved for `{name}` \u{b7} this box: {status}")
}
```
`ProbeStatus` is already imported (`agents.rs:65`). T4 adds the form close to this arm (§5.3).

### 3.5 Tests (first) and commits (T2)

| Test (file) | Asserts |
|---|---|
| `agent_settings::tests::parse_tool_path_trims_and_keeps_an_absolute_path` | `parse_tool_path("t", &format!("  {abs}  ")) == Ok(abs)`. `abs` is built from `std::env::temp_dir().join("x")`, which is absolute on every platform (H-17). |
| `agent_settings::tests::parse_tool_path_refuses_empty_and_relative_naming_the_tool` | `""` and `"   "` give ``Err("`demo_server`: the path is empty")``. `"bin/x"` gives ``Err("`demo_server`: the path must be absolute")``. |
| `store_worker::tests::set_tool_paths_is_named_and_refused_without_a_runtime` | `name() == SET_TOOL_PATHS == "set_tool_paths"`. `serve(&demo(), &SetToolPaths{..})` is `Failed { "set_tool_paths", "no agent runtime in this build" }`. This pins the `try_serve` arm. |
| `store_worker::tests::the_loop_hands_set_tool_paths_to_the_agent_runtime` | `spawn_with(Started::detached(Backend::memory(MemStore::demo())), …, AgentRuntime::new(DriverFactory::new()))`, then one `SetToolPaths` with a random `AgentId`. Exactly one reply, at `seq`: `Failed { request: "set_tool_paths", message }` with `message != "no agent runtime in this build"`. This holds before T3 ("not a chat request") and after it ("agent `…` not found"), per B13. |
| `store_worker::tests::agent_requests_are_named_as_agent_settings_lists_them` (`:3950`) | **Unchanged**: `REQUEST_NAMES` stays the three (H-18). |
| `testkit::tests::a_harness_without_an_agent_runtime_refuses_set_tool_paths_by_name` | `Harness::demo()`, then `app().update(Action::Store(SetToolPaths{..}))`, then `drive()`. `app().status == Some("set_tool_paths: no agent runtime in this harness")`. The harness-specific sentence proves the request is on the runtime list. |
| `testkit::tests::a_harness_with_an_agent_runtime_serves_set_tool_paths` | `.with_agent_runtime(AgentRuntime::new(DriverFactory::new()))`. The status starts with `set_tool_paths: ` and contains neither `no agent runtime in this`. |

Commits: **(a)** the variant, `name()`, the three routing edits and comments, `SET_TOOL_PATHS`,
`AgentWrite::ToolPaths` plus the `agents.rs` arm and helper, `parse_tool_path` with a `todo!()`
body (no caller yet), and every test (the routing ones are green, the parser ones red). **(b)** the
`parse_tool_path` body.

### 3.6 Gate (T2)

```bash
cargo fmt --all -- --check
cargo test -p htui --all-features --lib -- --test-threads=1 store_worker agent_settings testkit
cargo test -p htui --all-features --test settings -- --test-threads=1   # agents.rs arm; nothing moves
cargo clippy -p htui --all-features --all-targets -- -D warnings
```

---

## 4. T3: the agent runtime handler (D8, D9; B1, B4–B6, B15; F-6, F-7), after T1 and T2

**First failing test**: `agent_worker::tests::a_valid_map_writes_a_manual_snapshot_and_answers_tool_paths`.

**File**: `crates/htui/src/agent_worker.rs` only.

### 4.1 The claim tag (B4)

`Writes` (`:528-534`) gains a variant. `Background` (`:516-578`) gains a constructor and a reader:

```rust
    /// A `SetToolPaths` task for this agent (MOD-66 D8): it writes `agent_box`, so it holds the
    /// claim exactly as [`Writes::AgentBox`] does, **and** `ProbeAgents` refuses beside it. The
    /// agent is named so the refusal can say whose write to wait for.
    ToolPaths(AgentId),
…
    /// A `SetToolPaths` task (MOD-66 D8, B4). In `background`, not in a slot of its own, so the
    /// sweep, [`finish_background`](AgentRuntime::finish_background),
    /// [`shutdown`](AgentRuntime::shutdown) and `writing_background_len` all cover it unchanged.
    fn writing_tool_paths(task: JoinHandle<()>, agent_id: AgentId) -> Self
    /// The agent of a running `SetToolPaths`, or `None` for every other task.
    fn tool_paths(&self) -> Option<AgentId>
```
`writes_agent_box` becomes `matches!(self.writes, Writes::AgentBox | Writes::ToolPaths(_))`.
`Writes` stays `Copy` (`AgentId: Copy`).

### 4.2 Refusals (D8; B5)

```rust
/// What `claim_is_free` and `ProbeAgents` refuse with while a `SetToolPaths` write runs (MOD-66 B5).
fn tool_paths_running(agent_id: AgentId) -> String {
    format!("a tool-paths write is running for agent {agent_id}; try again once it has finished")
}
```
- `claim_is_free` (`:1724`): just **before** the generic background check (`:1764`), add `if let
  Some(agent_id) = self.background.iter().find_map(Background::tool_paths) { return
  Err(StoreError::Backend(tool_paths_running(agent_id))); }`. The function doc names the fifth
  holder.
- `probe()` (`:1287`): the same check, after the install check (`:1305-1310`) and before
  `box_probe_running` (`:1312`), with a comment naming D8. `probe()` consults no background writer
  today, and it still consults no other one (`r` beside a staleness re-probe stays allowed, as
  before).

### 4.3 `serve` arm and the handler (D9; B6)

In `serve`, after the `ProbeBox` arm (`:1114`):

```rust
            StoreRequest::SetToolPaths { agent_id, paths } => {
                match self.set_tool_paths(backend, replies, addr, *agent_id, paths).await {
                    Ok(served) => served,
                    Err(err) => Served::Reply(failed(SET_TOOL_PATHS, &err)),
                }
            }
```
**`serve` ends in a wildcard** (`other => "not a chat request"`, `:1175`), so a missing arm compiles
(H-4). The `Served::Deferred` doc (`:381-383`) and the `background` field doc (`:420-429`) name the
tool-paths task.

```rust
    /// The [`StoreRequest::SetToolPaths`] path (MOD-66 D7–D9). Every refusal comes **before
    /// anything is spawned**, in `auth_start`'s order (B6): writer (offline:
    /// `REGISTRY_ON_SERVER_ONLY`), registered box, the box claim, this row's re-probe claim, the
    /// row exists, it is `enabled`, its `launch` parses, every key is a tool its
    /// `discovery.tools` declares, and every value passes `parse_tool_path` and `is_file`. A
    /// store refusal is an `Err`. A refused field is `Ok(Served::Reply(Failed))` carrying its own
    /// sentence, never behind a `StoreError` prefix (MOD-23 D250's precedent).
    ///
    /// Awaited on the loop's arm: `box_info()`, `agents()` and one `metadata` per path, which is
    /// the same class of cost the probe's and the login's arms already pay (`R-NF-3`). The probe
    /// runs in the task.
    async fn set_tool_paths(
        &mut self,
        backend: &Backend,
        replies: &mpsc::UnboundedSender<ReplyEnvelope>,
        addr: ReplyAddr,
        agent_id: AgentId,
        paths: &BTreeMap<String, String>,
    ) -> Result<Served, StoreError>
```

The sequence:
1. `recording_writer(backend)?` → `registered_box(backend).await?` → `self.claim_is_free()?` → the
   `reprobe_claims.claim((agent_id, box_id))` refusal, using `auth_start`'s sentence (`:1603-1607`)
   → `row_for(backend, agent_id).await?`.
2. `!summary.agent.enabled` → ``refuse(format!("`{name}` is disabled in the registry; nothing to set a path for"))``.
3. `serde_json::from_value::<AgentLaunch>(…)` fails → ``refuse(format!("`{name}`'s launch does not parse; nothing declares a tool"))``.
   No serde text, because it can quote an `env` value (`R-SEC-2`, the `checked_launch` rule).
4. For each `(tool, text)` in `paths` (`BTreeMap` order, so the first failure is deterministic):
   undeclared → ``"`{tool}` is not a tool `{name}` declares"``. Then
   `agent_settings::parse_tool_path(tool, text)`; on `Err(s)`, refuse with `s`. Then
   `!htui_agent::probe::is_file(Path::new(&path)).await` →
   ``"`{tool}`: `{path}` is not a file on this box"``. Collect the trimmed map.
5. `let (env, _) = self.probe_env()?;`, then
   `let frames = Frames::new(replies.clone(), addr); let answer = frames.answer(tool_paths_failed);`.
6. `self.background.push(Background::writing_tool_paths(tokio::spawn(answering("tool paths",
   run_tool_paths(ToolPathsArgs { … }), Some(answer))), agent_id)); Ok(Served::Deferred)`.

`refuse(message)` is `Ok(Served::Reply(StoreReply::Failed { request: SET_TOOL_PATHS, message }))`.

### 4.4 The task (B1, B15)

```rust
/// Everything a `SetToolPaths` task owns (MOD-66 D7). Reads go through a [`Backend`] clone
/// (`BoxProbeArgs`'s precedent: the reply carries the registry re-read), and the write through
/// the [`Writer`] taken at spawn. `claim` is the row's re-probe claim, **held to the end** and
/// released by its `Drop`, aborted or not (D8, H-10).
struct ToolPathsArgs {
    backend: Backend,
    writer: Writer,
    box_id: BoxId,
    agent: Agent,
    paths: BTreeMap<String, String>,
    env: ProbeEnv,
    frames: Frames,
    claim: ReprobeClaim,
}
// impl Debug: writer label, agent name, `paths.len()`; finish_non_exhaustive.

/// One probe of one row over the requested map, written whatever it found (MOD-66 D9):
/// [`probe_snapshot`], never `probe_agent`, so D51's `Kept` cannot swallow the edit (B1). Exactly
/// one reply at the request's address: `AgentWritten { agents, ToolPaths { id, name, status } }`,
/// or `Failed { "set_tool_paths" }` when the write fails. A re-read that fails after an applied
/// write also answers `Failed` (`agent_settings::serve`'s known residue).
async fn run_tool_paths(args: ToolPathsArgs)

/// A panicked `SetToolPaths`'s last word: the failure that clears the Agents section's `busy`
/// (MOD-53's shape).
fn tool_paths_failed(message: String) -> Vec<StoreReply>
```
Body: `let ctx = ProbeContext { env, now: Utc::now() }; let snapshot =
probe_snapshot(&agent, &paths, &ctx, &SpawnTier2::default()).await; let row =
agent_box_row(&agent, box_id, &snapshot, ctx.now);` then `writer.upsert_agent_box(&row)`, then
`backend.agents()`. `upsert_agent_box` applies `enabled AND NOT user_off`, so the re-read is right
for a switched-off row without code here. Imports: `htui_agent::launch::AgentLaunch`,
`htui_agent::probe::{agent_box_row, probe_snapshot}`, `crate::agent_settings::{SET_TOOL_PATHS,
parse_tool_path, AgentWrite}`, `std::collections::BTreeMap`, `std::path::Path`.

### 4.5 Tests (first): `agent_worker.rs` `mod tests`, a block `// MOD-66: SetToolPaths`

The fixtures sit beside `fake_env` (`:9280`). `paths_row(id) -> Agent` is `acp`, enabled, with
`launch = {"command": "${first}", "args": ["${second}"], "env": {}, "discovery": {"tools":
{"first": {"kind": "path", "names": ["htui-no-such-binary-66a"]}, "second": {"kind": "path",
"names": ["htui-no-such-binary-66b"]}}, "handshake": false}}`. With `handshake: false` a resolved
row is `ready` and **nothing is spawned**. `paths_fixture(tmp)` is `MemStore::demo()` plus
`upsert_agent(paths_row)`, then `AgentRuntime::new(DriverFactory::new()).with_probe_env(fake_env(tmp),
fake_hardware())`. `plain_file(path) -> String` writes a file. `set_paths(runtime, backend, tx, seq,
agent_id, pairs) -> Served`. `one_reply(runtime, rx) -> StoreReply` does `finish_background(5 s)`,
then drops `tx` and collects exactly one reply. The box is `ids::BOX`.

| Test | Asserts |
|---|---|
| `set_tool_paths_offline_refuses_before_spawning_anything` | `Backend::Offline` over a `CacheStore` (the `:6035` staging): `Failed { "set_tool_paths", m }` with `m.contains(REGISTRY_ON_SERVER_ONLY)`, and `background_len() == 0`. |
| `set_tool_paths_on_an_unregistered_box_is_refused` | `Backend::memory(MemStore::new())`: `m.contains("not registered")`, `background_len() == 0`. |
| `set_tool_paths_refuses_an_unknown_agent_and_a_disabled_one` | A random id gives `m.contains("not found")`. `paths_row` with `enabled: false` gives the disabled sentence. Both leave `background_len() == 0`. |
| `set_tool_paths_refuses_a_tool_the_row_does_not_declare` | `{"third": file}` gives exactly ``"`third` is not a tool `paths-fixture` declares"``. |
| `set_tool_paths_refuses_a_relative_path_a_directory_and_a_missing_file_naming_the_tool` | `{"first": "bin/x"}` → ``"`first`: the path must be absolute"``. `{"first": <tmp dir>}` and `{"first": <absent>}` → ``"`first`: `…` is not a file on this box"``. Nothing is spawned. |
| `set_tool_paths_is_refused_while_a_writer_holds_the_box` | Stage, one at a time: (i) `background.push(Background::writing(spawn(pending())))`, which gives `"is already writing this box"`; (ii) `box_probe = Some(spawn(pending()))`, which gives `BOX_PROBE_RUNNING`; (iii) `Background::writing_tool_paths(spawn(pending()), other)`, which gives `tool_paths_running(other)`. Abort or shut down between stagings. |
| `auth::set_tool_paths_while_a_login_runs_is_refused` (in `mod auth`, beside `a_probe_while_a_login_runs_is_refused`, `:7719`, copying its staging) | `m.contains("a login is already running")`. |
| `set_tool_paths_while_an_install_runs_is_refused` (beside `a_probe_and_an_install_never_write_the_same_row_at_once`, `:6382`, copying its staging) | `m.contains("an install is already running")`. |
| `set_tool_paths_is_refused_while_a_re_probe_holds_the_row` | `let _held = runtime.reprobe_claims.claim((agent_id, ids::BOX))`. The sentence names the agent and "re-probe". `background_len() == 0`. |
| `probe_agents_is_refused_while_a_tool_paths_write_runs` | `Background::writing_tool_paths(spawn(pending()), agent_id)`, then `ProbeAgents` gives `Failed { "probe_agents", tool_paths_running(agent_id) }` (as `failed()` renders it: `contains`). `background_len() == 1` (nothing added). |
| `a_valid_map_writes_a_manual_snapshot_and_answers_tool_paths` | Both tools map to plain files. The answer is `Deferred`, and `writing_background_len() == 1`. Exactly one reply, at `seq` and origin: `AgentWritten { agents, ToolPaths { id, name: "paths-fixture", status: Ready } }`. The stored row has `probe.source == "manual"`, `probe.manual == map`, `probe.resolved.command == first`, `args == [second]`, `enabled`, and `probed_at` set. Afterwards `reprobe_claims.claim((agent_id, ids::BOX)).is_some()` (the claim was released). |
| `a_stored_manual_row_with_an_incomplete_report_is_still_written` | A stored row with `source: manual`, old `probed_at`, and a map to both files. Request `{first: file}` only: `second` is missing. The status is `Missing`, the row **was written** (`probed_at` is new), and `manual == {first}`. `source` is `manual`, because `first` was manual. This is plan amendment 7, the case `probe_agent` would have kept. |
| `a_row_with_no_agent_box_row_is_written` | The demo has no `agent_box` for `paths_row`. A full map gives `Ready`, and the row exists afterwards. |
| `an_empty_map_clears_to_a_probe_snapshot_even_when_missing` | A stored row with `source: manual` and a map. `{}` gives `ToolPaths { status: Missing }`. The stored `probe.source == "probe"`, and there is no `manual` key (`probe.get("manual").is_none()`). |
| `a_switched_off_row_stays_off_under_set_tool_paths` | `store.set_agent_box_enabled(agent_id, ids::BOX, false)`, then a full map. The reply says `status: Ready`. The re-read summary has `user_off`, `!on_box.enabled`. |
| `a_probe_after_set_tool_paths_keeps_the_map` | A full map, then `ProbeAgents`, then `finish_background`. The row still has `source: manual`, the same `manual`, and `ready` (D4: every probe path carries the map). |

The T2 routing tests (`store_worker`, `testkit`) are re-run in this gate. They now reach the
handler and must still pass (B13).

### 4.6 Commits (T3)

**(a) red**: the fixtures and every test above. They compile against T1 and T2's public items and
fail on `"not a chat request"`. **(b) green**: §4.1–§4.4 and the doc updates.

### 4.7 Gate (T3)

```bash
cargo fmt --all -- --check
cargo test -p htui --all-features --lib -- --test-threads=1 agent_worker store_worker testkit
cargo clippy -p htui --all-features --all-targets -- -D warnings
```

---

## 5. T4: the Settings form (D10, D11; B7–B9, B14, B16, B17; F-3), after T1 and T2

**First failing test**: `tests/settings.rs::m_opens_the_tool_paths_form_with_one_field_per_declared_tool`.

**Files**: `crates/htui/src/ui/tabs/settings/agents.rs`, `crates/htui/tests/settings.rs`,
`crates/htui/tests/snapshots/settings__agents_{switched_off,unknown_row,empty,probed,demo,quota}.snap`,
`probe__agents_probed_missing.snap` (moved), and `settings__agents_tool_paths_form.snap` (new).

> **Amendment, maintainer 2026-10-01 (F-3 answered: "short marker `*`").** This replaces
> ` (manual)` everywhere in §5. The rest of §5 is unchanged.
>
> - `MANUAL_SUFFIX` is `"*"`. A manual row's cell reads `ready*`, `missing*`, `failed*` or
>   `0.48.0*`. `unauthenticated*` (16 characters) is clipped like `unauthenticated` already is.
>   B17's scope is unchanged: no marker on install, login, probing, switched-off or not-probed
>   cells.
> - When **any** row's `probe.source` is `manual`, the note line under the keys (the line carrying
>   `QUOTA_NOTE`) gains ``" · * manual path"`` (`MANUAL_NOTE`), within `NOTE_WIDTH` 98.
> - The test `a_manual_row_reads_manual_in_the_on_this_box_cell` expects `0.48.0*`, `missing*` and
>   `0.48.0` (no `as_drawn` clipping needed), and asserts the note appears only when a manual row
>   exists.
> - The module doc and the `on_box_cell` doc say `*`, not ` (manual)`.

### 5.1 Constants (after `UNCHANGED`, `agents.rs:132`)

```rust
const HINT_IDLE: &str = "j/k select \u{b7} n new \u{b7} e edit \u{b7} m paths \u{b7} t this box \u{b7} r probe \u{b7} i install \u{b7} a authenticate";
/// What the `on this box` cell appends when `probe.source` is `manual` (MOD-66 D10). Clipped by
/// the 13-wide column on most cells (blueprint F-3).
const MANUAL_SUFFIX: &str = " (manual)";
/// `m` on a row with no `discovery.tools` (D10).
const LITERAL_LAUNCH: &str = "this row's launch is literal; e edits its command";
/// `m` on a row whose `launch` does not parse (B14).
const LAUNCH_UNREADABLE: &str = "this row's launch does not parse; nothing declares a tool";
```
`HINT_IDLE`'s doc says "(89 of the 98 columns, MOD-23 D245, MOD-66 D10)". `m` sits with the other
write keys (B9). The module doc gains a "Since MOD-66" paragraph: `m`, the form, `SetToolPaths`
served by the runtime, and ` (manual)`.

### 5.2 State (D11; B7, B8)

```rust
enum Mode {
    #[default] Browse,
    Editing(Editor),
    /// `m`'s form (MOD-66 D10): one path per tool the row declares. Its own type, so the
    /// create and edit forms keep their `&'static str` labels (D11).
    Paths(PathsForm),
}

/// The tool-paths form: its own row identity (MOD-23 F-14), never an index into the table.
#[derive(Debug)]
struct PathsForm {
    agent_id: AgentId,
    name: String,
    /// The stored `probe.manual`, restricted to the declared tools: the prefill, and what `Enter`
    /// compares against ("unchanged closes").
    opened: BTreeMap<String, String>,
    fields: Vec<PathField>,
    focus: usize,
}

/// One input, labelled with a tool name: a runtime string (D11).
#[derive(Debug)]
struct PathField {
    tool: String,
    /// Its `Debug` never prints the text.
    input: TextField,
}

impl PathsForm {
    /// The form over `summary`, or the refusal sentence: `LAUNCH_UNREADABLE`, or `LITERAL_LAUNCH`
    /// for no `discovery` or empty `tools`. Fields in `discovery.tools` (`BTreeMap`) order,
    /// prefilled from `ProbeSnapshot::from_row(on_box).manual`. An unreadable snapshot prefills
    /// nothing (H-21).
    fn open(summary: &AgentSummary) -> Result<Self, &'static str>;
    /// `Ok(None)` for an unchanged map. `Ok(Some(SetToolPaths))` with the trimmed non-empty
    /// entries (empty map = clear, D9). `Err((field index, sentence))` from `parse_tool_path`.
    fn request(&self) -> Result<Option<StoreRequest>, (usize, String)>;
    /// `tool paths for {name} · empty = no manual path`.
    fn header(&self) -> String;
    /// `Editor::lines`' shape. The label column is `min(longest tool name, width / 3)` (B8), and
    /// a longer name is cut to `column - 1` characters plus `…`.
    fn lines(&self, width: u16, theme: &Theme) -> Vec<Line<'static>>;
}
```

### 5.3 Behaviour

- **Key arm** in `on_key`'s match, after `'t'` (`:2101`):
  `KeyCode::Char('m') => { if !self.refuse_write(ctx) { self.open_paths(ctx); } Handled::Consumed }`.
  `refuse_write` already says busy, install, login and probe (D10, "as `n`/`e`/`t` are").
  `open_paths` covers no selection (`"no agent row is selected"`), the `PathsForm::open` refusals
  as `Action::Error`, and on success `self.mode = Mode::Paths(form); self.notice = None`. The
  free-keys comment (`:2068-2071`) gains `m`.
- **Routing** (H-12): `on_key`'s first test (`:2052`) also routes `Mode::Paths(_)` to a new
  `on_paths_key`. This is `on_editor_key`'s body over `form.fields[form.focus].input`. `Submit`
  calls `submit_paths`. `Cancel` sets `Browse` and clears the notice.
  `captures_input` becomes `matches!(self.mode, Mode::Editing(_) | Mode::Paths(_)) || …`. In
  `on_paste`, the last `if let Mode::Editing` block gains a `Mode::Paths(form)` twin (paths are
  pasted).
- **`submit_paths`**: `submit`'s shape. The busy refusal comes first. `Ok(Some(req))` clears the
  notice and calls `self.send(req, ctx)` (busy = `"set_tool_paths"`). `Ok(None)` sets `Browse` and
  `UNCHANGED`. `Err((index, sentence))` sets the focus to `index` and the notice to the sentence.
  **The form stays open** until the reply.
- **`pane`** (`:1249`): a `Mode::Paths(form)` branch, the header then `form.lines(width, theme)`.
- **`hint`** (`:1283`): the tuple match's first arm becomes `(Mode::Editing(_) | Mode::Paths(_), _)
  => HINT_EDITING`. That `match` is exhaustive, and the compiler forces the change (H-5).
- **`on_written`**: the `Stale` arm's `match &mut self.mode` (`:1528-1531`) is exhaustive and needs
  `Mode::Paths(_) => Vec::new()` (H-5). The `ToolPaths` arm (T2's) becomes:
  `if matches!(&self.mode, Mode::Paths(form) if form.agent_id == *id) { self.mode = Mode::Browse; }`
  followed by T2's notice (B16, B18).
- **`on_reply`** (`:2207`): the guard becomes
  `REQUEST_NAMES.contains(request) || *request == agent_settings::SET_TOOL_PATHS` (H-6). The body is
  unchanged, so busy clears, the notice is the message, and the form stays open.
- **`on_box_cell`** (`:1178-1213`, B17): the first five early returns are unchanged (install,
  login, probing, switched off, not probed). The tail becomes:

```rust
        let probe = row.probe.as_ref();
        let status = probe.and_then(|p| p.get("status")).and_then(Value::as_str);
        let cell = if let Some(status @ ("missing" | "unauthenticated" | "failed")) = status {
            status.to_owned()
        } else {
            let version = row.version.as_deref().unwrap_or(NONE);
            if row.enabled { version.to_owned() } else { format!("{version} (off)") }
        };
        if probe.and_then(|p| p.get("source")).and_then(Value::as_str) == Some("manual") {
            format!("{cell}{MANUAL_SUFFIX}")
        } else {
            cell
        }
```
  The doc gains: "A `manual` row's verdict ends in ` (manual)`, whatever it is (MOD-66 D10)."

### 5.4 Tests (first): `tests/settings.rs`, a block `// MOD-66 T4: m and the tool-paths form`

Edit `IDLE_KEYS` (`:3600`) to the new `HINT_IDLE` text. Helpers:
`paths_row(name, tools: &[&str], stored: &[(&str, &str)], source: ProbeSource) -> AgentSummary`
is `registry_row`'s shape, with one `{"kind": "path", "names": [tool]}` per tool, and an `on_box`
built through a `ProbeSnapshot` literal plus `agent_box_row`, as `login_row` does (`:1908`). A
hand-written document might not parse back. `paths_field_of(rendered, label, width)` is
`field_of` with a given label width (H-9). Paths are built from `std::env::temp_dir()` (H-17).

| Test | Asserts |
|---|---|
| `m_opens_the_tool_paths_form_with_one_field_per_declared_tool` | Tools `alpha_tool`, `beta_tool`, stored `{alpha_tool: A}`. After `m`: `captures_input()`, the header line, `alpha_tool` → `A`, `beta_tool` → empty. Typing `l` then `h` lands in the field (the section is not cycled). |
| `m_on_a_literal_row_is_refused` | A row with `launch = {"command": "/bin/x", "args": [], "env": {}}` gives `errors_of == [LITERAL_LAUNCH]`, no request, and `!captures_input()`. A `launch = json!("nonsense")` row gives `LAUNCH_UNREADABLE`. |
| `m_is_refused_while_a_write_or_a_probe_is_in_flight` | After `t`, `m` gives ``"`set_agent_on_box` is still in flight"``. After `r` (and its `Failed`), `m` gives `"a probe is running; edit afterwards"`. Nothing is asked. |
| `enter_sends_set_tool_paths_with_trimmed_non_empty_entries` | Type `"  {B}  "` into `beta_tool`, leave `alpha_tool` as is, press Enter: exactly `[SetToolPaths { agent_id, paths: {alpha_tool: A, beta_tool: B} }]`. A second Enter gives ``"`set_tool_paths` is still in flight"``. |
| `clearing_every_field_sends_an_empty_map` | Stored `{alpha_tool: A}`. Backspace ×64, then Enter: `SetToolPaths { paths: {} }`. |
| `an_unchanged_map_closes_with_unchanged` | `m`, Enter: the form is closed, `note_line == UNCHANGED`, and nothing was requested. |
| `a_relative_path_is_refused_locally_naming_the_tool` | `"bin/x"` in `beta_tool` gives note ``"`beta_tool`: the path must be absolute"``, `beta_tool` focused, nothing requested. |
| `agent_written_tool_paths_closes_the_form_and_notes_the_status` | `written(rows, ToolPaths { id, name: "alpha", status: Ready })` gives a closed form and note ``"tool paths saved for `alpha` · this box: ready"``. A `ToolPaths` for **another** id leaves the form open (B18). |
| `a_failed_set_tool_paths_clears_busy_and_keeps_the_form` | `Failed { request: "set_tool_paths", message }`: the form is open, the note is the message, and the next Enter sends again. |
| `a_paste_lands_in_the_focused_tool_path` | `bench.paste(section, B)` into an open form, then Enter: the map holds `B`. |
| `a_manual_row_reads_manual_in_the_on_this_box_cell` | Rows: `source: manual` `ready` with version `0.48.0`, `manual` `missing`, and `probe` `ready`. Cells are `as_drawn("0.48.0 (manual)")`, `as_drawn("missing (manual)")` and `"0.48.0"` (F-3: expectations written whole, clipped the way the frame clips). |
| `the_tool_paths_form_sizes_its_labels_to_the_longest_tool` | Tools `demo_agent_server` (17) and `x`: the label column is 17, and `x` is padded. A 40-character tool name at width 100: the label is 33 characters ending in `…`. |
| `the_tool_paths_form_renders_under_the_table` | Two rows, cursor on the second, `m`, `render_section`, `insta::assert_snapshot!("agents_tool_paths_form", rendered)`. |
| `the_idle_keys_and_the_quota_note_are_two_lines` (`:4783`) | Unchanged code. It now pins the new `IDLE_KEYS`. |

### 5.5 Snapshots

`cargo insta test -p htui --all-features --test settings --test probe -- --test-threads=1`, then
`cargo insta review` (or `accept` after reading each `.snap.new`). The six `settings__agents_*`
files and `probe__agents_probed_missing` must differ **only** in the keys line (`m paths` inserted).
The new `settings__agents_tool_paths_form.snap` is reviewed by eye. `ls
crates/htui/tests/snapshots | wc -l` must be **127**, and `cargo insta pending-snapshots` must be
empty.

### 5.6 Commits (T4)

**(a) red**: the helpers, the tests, and the `IDLE_KEYS` edit. They fail on the hint, on `m`
unbound, and on the cell. **(b) green**: §5.1–§5.3 and the accepted snapshots (7 moved, 1 new, all
staged by path).

### 5.7 Gate (T4)

```bash
cargo fmt --all -- --check
cargo test -p htui --all-features --test settings --test probe --test auth -- --test-threads=1
cargo insta pending-snapshots                       # nothing pending
git diff --stat HEAD~2 -- crates/htui/tests/snapshots   # 7 modified (1 line each), 1 added
cargo clippy -p htui --all-features --all-targets -- -D warnings
```
`--test auth` is there because `tests/auth.rs:514` asserts `contains("a authenticate")` on a live
render, which still holds.

---

## 6. File sets, re-verified disjoint within each wave

| Task | Files |
|---|---|
| T1 | `htui-agent/src/{probe.rs, lib.rs, launch.rs}`, `htui-agent/tests/{probe.rs, acp_driver.rs, auth.rs}`, `htui-core/src/model/agent.rs`, `htui/src/agent_worker.rs` (`:7241` only), `htui/tests/{auth.rs, settings.rs}` (`:205`, `:1909` only) |
| T2 | `htui/src/{store_worker.rs, testkit.rs, agent_settings.rs, ui/tabs/settings/agents.rs}` |
| T3 | `htui/src/agent_worker.rs` |
| T4 | `htui/src/ui/tabs/settings/agents.rs`, `htui/tests/settings.rs`, `htui/tests/snapshots/` (8 files) |

- **T1 ∩ T2 = ∅.** Hidden coupling was checked. No `ProbeSnapshot` literal lives in a T2 file. T2
  uses `ProbeStatus` only (already public). T1 renders nothing (an empty `manual` adds no key), and
  T2 renders nothing new (the arm fires only on a reply no test sends before T4), so no snapshot
  moves in wave 1.
- **T3 ∩ T4 = ∅.** T3 changes nothing a `SectionBench` renders. `probe__agents_probed_missing`
  runs through the runtime's `ProbeAgents`, and T3's new refusal fires only while a tool-paths task
  runs, so only T4's hint moves it.
- **Sequential overlaps** (not parallel): T3 builds on T1's `agent_worker.rs`. T4 builds on T1's
  `tests/settings.rs` and T2's `agents.rs`.
- **Cross-task contract**: T3 and T4 rely on `htui_agent::probe::{probe_snapshot, is_file,
  agent_box_row}`, `ProbeSnapshot::manual`, `crate::agent_settings::{SET_TOOL_PATHS,
  parse_tool_path, AgentWrite::ToolPaths}` and `StoreRequest::SetToolPaths { agent_id, paths }`,
  exactly as §2 and §3 spell them.

---

## 7. Hazards

| # | Hazard | Guard |
|---|---|---|
| H-1 | **Serde key order and round trip.** `ProbeSnapshot`'s field order is its key order (`probe.rs:1088-1090`). `manual` must be **last**, `#[serde(default)]` (old documents), and `skip_serializing_if = "BTreeMap::is_empty"`, so that every pin and every existing row keeps its bytes. JSONB does not keep key order, so the order pin is struct-level only. There is no `deny_unknown_fields`, which is why an older binary reading a new row ignores the key. | §2.4's two serde tests, plus `:1972` unchanged. |
| H-2 | **Every `ProbeSnapshot` literal.** There are 10 sites, and 9 need the field (there is no `Default`). Inside `probe.rs` the `blank` closure also needs a `source` parameter: hard-coding `Probe` there would mark a `Missing` snapshot that used a manual path `probe`, which breaks D4 and `a_stored_manual_row_with_an_incomplete_report…`. | §2.1 (h), §2.3, and `cargo check --workspace --all-targets` in T1's gate. |
| H-3 | **Rustdoc private links.** `is_file` turning `pub` makes its link to private `exists` a hard `cargo doc` error. New `pub` docs must not link `launch_from` (`pub(crate)`) or any private helper. | F-2, and `cargo doc -p htui-agent` in T1's gate. |
| H-4 | **Three routing sites, two of them wildcards, plus a fourth wildcard.** `try_serve` is exhaustive (it compiles only with an arm). The store-loop list (`store_worker.rs:2129-2144`) and `testkit.rs:280-295` fall through to "no agent runtime in this build" when the line is forgotten. `AgentRuntime::serve` ends in `other => "not a chat request"`. | T2's four routing tests (B13), and T3's tests through `serve`. |
| H-5 | **Exhaustive matches.** `on_written` (`agents.rs:1466`) does not compile in T2 without a `ToolPaths` arm. `Mode::Paths` breaks `hint`'s tuple match (`:1284`) and the `Stale` arm's `match &mut self.mode` (`:1528`) at compile time, which is good. The `matches!`/`if let` sites (`captures_input`, `on_key`, `on_paste`, `pane`) do **not** break, and must be found by hand (H-12). | §3.4, §5.3. |
| H-6 | **`busy` clearing on `Failed`.** `send` sets `busy = request.name()`, which is `"set_tool_paths"`, and only a `Failed` whose name is in `REQUEST_NAMES` clears it (`:2207`). Without the `SET_TOOL_PATHS` clause, a refused write locks `m n e t r i a` for the session. Do **not** add it to `REQUEST_NAMES` (H-18). | §5.3, and `a_failed_set_tool_paths_clears_busy_and_keeps_the_form`. |
| H-7 | **`on_box_cell`'s early return** for the three status words (`:1203-1205`) skips everything after it. The suffix must be applied after both branches. The fixed 13-wide column clips it (F-3). | §5.3 code, and the `as_drawn` test. |
| H-8 | **Snapshot and `IDLE_KEYS` moves.** Seven files carry the idle hint, and `IDLE_KEYS` is a hand-written pin. 89 ≤ `NOTE_WIDTH` 98. `contains("a authenticate")` checks (`settings.rs:2627,2772,2788`, `auth.rs:514`) still hold. | §5.5, and the diff must be the keys line only. |
| H-9 | **`LABEL_WIDTH` 13 is a test constant** (`tests/settings.rs:3597`), not the section's. `Editor::lines` sizes to its own longest label. The paths form's column is its tools' (B8), so `field_of` cannot read it. | `paths_field_of(…, width)`. |
| H-10 | **Claims.** `claim_is_free` cannot see the D60 re-probe, which runs inside the chat task. Only `ReprobeClaims` can (`:2461`). The claim must be **moved into** the task's args and dropped with it (RAII, aborted or not). Dropping it on the arm would let a D60 re-probe race the write. The task must be pushed into `background` (B4). A new runtime slot would be invisible to `sweep_finished`, `finish_background` (a harness would photograph a write in flight) and `shutdown`. | §4.1, §4.4, and the claim-released assertion in the valid-map test. |
| H-11 | **`probe()` checked no background writer.** The section's `probing`/`busy` guard covers its own keys only, and any `StoreReply::Agents` clears `probing` (`agents.rs` module doc). The runtime has to refuse `ProbeAgents` itself. | §4.2, and `probe_agents_is_refused_while_a_tool_paths_write_runs`. |
| H-12 | **Typed and pasted input.** A path holds `h`, `l`, `q` and digits. If `captures_input` misses `Mode::Paths`, `SettingsTab::on_key` (`settings/mod.rs:333-352`) cycles sections on `h` and `l`. If `on_paste` misses it, a pasted path is dropped. | The `l`/`h` and paste tests in §5.4. |
| H-13 | **D8 residual (named, not fixed).** A D60 re-probe uses the `existing` read at `ChatStart`. If a `SetToolPaths` lands between that read and the re-probe's write, the re-probe rewrites the row **without** the new map. The fix is a re-read in `run_reprobe`, which is out of scope. The handoff write-up must record it. | — |
| H-14 | **Spawn-time fallback.** `tools::resolve` and `resolve_now` have no manual tier (D12). A manual recording whose file is gone falls back to the override and the discovery tiers. | Documented on `launch_from` (§2.2). |
| H-15 | **`R-AGT-5`.** No vendor name in a `src/` comment or branch (write `the glob tool's platform args`, not a tool name), and the word `zeta` nowhere. | `cargo test -p htui-agent --test extensibility` (in T1's gate). |
| H-16 | **The 2 MiB test stack.** Project memory's case is `htui-orch`'s `every_case_name_dispatches`. `htui-orch` is untouched (D12) and does not depend on `htui`. `AgentRuntime::serve`'s future gains one arm (`set_tool_paths`: three awaits, small). If a `serve`-driving test ever reports `SIGABRT`/stack overflow, wrap that arm's call in `Box::pin(…)`. Never raise the stack. | Run gates with `--no-fail-fast`, and grep the output for `SIGABRT`. |
| H-17 | **Absolute paths across platforms.** `"/opt/x"` is not absolute on Windows. | Tests build absolute paths from `tempdir()` or `temp_dir()`, and relative ones as `"bin/x"`. |
| H-18 | **`REQUEST_NAMES` is pinned** to the three loop-served names in `StoreRequest` order (`store_worker.rs:3950`). | `SET_TOOL_PATHS` is a separate const (§3.3). |
| H-19 | **Error rendering.** `failed()` renders `StoreError` with its prefix (`store unreachable: …`, `store backend error: …`). The field refusals answer `Failed` directly, so they carry no prefix. Tests on `StoreError` paths use `contains`. | §4.3. |
| H-20 | **Kept for the probe's own manual rows (D5, settled).** Rows written by `SetToolPaths` say `source: manual`. If the manual file later vanishes and nothing else resolves, every later probe is `Kept`: the row keeps `ready`, the recording and the old `probed_at`, and a chat falls back (H-14) and fails. | Accepted by D5. It is the reason `SetToolPaths` must bypass `Kept` (B1). |
| H-21 | **Unparseable stored `probe`.** `from_row` answers `None`. `probe_agent` then carries **no** map (it is dropped on the next refreshing probe) and the form prefills nothing. This is consistent with the column's existing tolerance. | Named. No code. |
| H-22 | **Sizes.** `ProbeOutcome`'s `#[expect(clippy::large_enum_variant)]` is about `AgentBox`, which is unchanged. `AgentWrite::ToolPaths` (16 + 24 + 1 bytes) is smaller than `Switched`. `StoreRequest::SetToolPaths` (16 + 24) is far below `InstallConfirm`'s boxed plan. No `#[allow]`, ever. | Clippy in every gate. |
| H-23 | **Cross-version.** An older `htui` on the **same** box drops the map on its next refreshing probe, because it does not know the field. `agent_box` is per box, so other boxes are unaffected. | Named. |
| H-24 | **Integration suites need `--all-features`.** Without `testkit`, `tests/*.rs` run 0 tests and say ok. | Every gate passes `--all-features`. |

---

## 8. Decisions

| # | Decision | Why |
|---|---|---|
| B1 | D9's open choice: **a `pub` entry point**, `probe_snapshot(agent, manual, ctx, tier2) -> ProbeSnapshot` (the former private `snapshot_for`). The handler builds the row with the existing `pub agent_box_row`. There is no synthesized `existing`. | `from_row` needs a full document, so a "minimal" one silently loses the map (F-6). A returned snapshot hands over `status` with no re-parse. "Always writes" is in the type, because `ProbeOutcome` is not involved. |
| B2 | `ToolReport.manual: Vec<String>` records which tools a manual path decided, mirroring `missing`. | `source` (D4) needs it, and the type already lists names this way. |
| B3 | The probe tier counts a **relative** manual path as missing, as well as a non-file one. | `is_file` on a relative path would be resolved against the process's cwd, not `env.cwd`, and only a hand-edited row can hold one. |
| B4 | D8's open choice: a third `Writes::ToolPaths(AgentId)` tag on a `background` entry. `writes_agent_box` counts it, and `probe()` and `claim_is_free` look it up. | No new slot means the sweep, `finish_background`, `shutdown` and `writing_background_len` cover it unchanged (H-10), and the tag names the agent for the refusal. |
| B5 | `claim_is_free` names a running tool-paths write with its own sentence, checked before the generic background sentence. `probe()` uses the same sentence. | The generic sentence says "install once it has finished" (F-7). |
| B6 | Handler order follows `auth_start`: writer, box, box claim, row claim, then D9's row, enabled, launch, keys and paths checks. | It is the one other per-row `agent_box` writer that takes both claims. A claim refusal costs no `agents()` read, and D9's relative order is kept. |
| B7 | D11's open choice: `Mode::Paths(PathsForm)` with `PathField { tool: String, input: TextField }`. `Editor`, `Field` and `Target` are untouched. | The create and edit forms keep their `&'static str` labels, `FIELD_LABELS` and `Refusal` focus logic byte for byte. |
| B8 | The label column is `min(longest tool, width / 3)`, and a longer name is cut to `column - 1` plus `…`. | D11's cap, made exact. Ellipsis truncation matches `TextField`'s left clip. |
| B9 | The hint is `… e edit · m paths · t this box …` (89 columns). | `m` sits with the other write keys. |
| B10 | `parse_tool_path(tool, text) -> Result<String, String>`. The sentences are ``"`{tool}`: the path is empty"`` and ``"`{tool}`: the path must be absolute"``. The worker adds the `is_file` sentence. | `Refusal`'s display shape with a runtime name (D11). The section and the worker share one rule, as MOD-23 D247 does. |
| B11 | `is_file` is `pub` but not re-exported at the root. The root gains `probe_snapshot` and `probe_tools_with`. | The bare name says nothing at the root (`lib.rs` convention), and `htui_agent::probe::is_file` is reachable. |
| B12 | `name()` returns `agent_settings::SET_TOOL_PATHS`, and the const is not in `REQUEST_NAMES`. | `PROMPT_PREVIEW` precedent. H-18. |
| B13 | T2's routing tests are written to stay green through T3 (negative assertion plus a no-runtime harness positive). | F-5. |
| B14 | `m` on a row whose `launch` does not parse gets `LAUNCH_UNREADABLE`. "No declared tools" gets D10's sentence. | "Literal" would misdescribe a broken document. |
| B15 | The task re-reads through a `Backend` clone. A failed re-read after an applied write answers `Failed`. | `BoxProbeArgs` and `agent_settings::serve` precedent. |
| B16 | The notice is ``"tool paths saved for `{name}` · this box: {status}"``. T2 defines it and T4 keeps it. | It names the row and the verdict, never a path. One definition across two tasks. |
| B17 | The `(manual)` suffix is applied after the status or version decision, never to install, login, probing, switched-off or not-probed cells. | Those cells are not the snapshot's verdict. D10's "every status" means every probe status. |
| B18 | `AgentWritten::ToolPaths` closes the form only when it is the same agent's. | MOD-23 F-14: forms hold their own row identity. |

---

## 9. Merge order and the workspace gate

Wave 1: merge T1, then T2. On the merged tree, run `cargo check --workspace --all-features
--all-targets`, T1's §2.6 and T2's §3.6. Wave 2 branches from it: merge T3, then T4, and re-run
§4.7 and §5.7. End:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features --no-fail-fast -- --test-threads=1   # Postgres suites run, not SKIP; grep SIGABRT (H-16)
cargo doc -p htui-agent --no-deps --all-features
ls crates/htui/tests/snapshots | wc -l                                       # 127
```
Before believing a Postgres failure, check `df -h /` and re-run the case alone (project memory:
disk pressure crash-loops the dev Postgres).
