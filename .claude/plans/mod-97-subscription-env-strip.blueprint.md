# Blueprint: MOD-97 (code-architect, 2026-11-10)

Source plan: `.claude/plans/mod-97-subscription-env-strip.plan.md`.


## Architecture: MOD-97, strip API-billing variables from subscription-billed agent spawns

The two things most likely to go wrong are already-written tests, not the new code. Two existing migration tests will break once 0028 exists, and 27 struct literals of `ResolvedLaunch` will stop compiling.

### Design decisions (only what D1–D6 leave open)

- **Driver field `strip_env: Vec<String>`**, computed once in `from_row_with_probe` by a helper that matches on `Billing` with no wildcard arm. A new billing kind then has to be decided explicitly.
- **The helper drops `SESSION_VAR` from the list.** `spawn_supervised` runs `env_remove` after `.envs`, so a row listing `HTUI_SESSION` would silently remove the session marker. MOD-91 D4 says a row cannot unset it.
- **`strip_billing_env` overwrites `resolved.env_remove`** with all listed names, present or not. Removing an absent variable does nothing, and tests can assert the exact list. The `warn!` fires only when the name was in `resolved.env` or in the process environment (`std::env::var_os`). It logs the agent name and the variable name, never the value.
- **`Debug` prints `env_remove` only when it is non-empty.** This keeps two exact `Debug` assertions byte-identical (`tests/launch.rs:627` and `:640`, both `... env: {} }`), and mirrors `skip_serializing_if`. The names are not secret.
- **`AgentSettings.api_billing_env` needs no field attribute.** The struct already has `#[serde(default)]` (`launch.rs:285`), and nothing in production serialises `AgentSettings` back to the database. Mirror `user_extensions`.
- **Windows:** `BTreeMap::remove` is case-sensitive, but `Command::env_remove` after `.envs` is case-insensitive on Windows. The removal at spawn is the authoritative one.

### 1. Rust shapes

**`crates/htui-agent/src/launch.rs`**

- `AgentSettings` (struct at :286, add after `user_extensions` :305): `pub api_billing_env: Vec<String>,`. Its doc comment **must not spell `ANTHROPIC_`**; the sweep reads every line of `launch.rs` because the file has no `#[cfg(test)]\nmod tests {` block. Describe the seeds instead.
- `ResolvedLaunch` (:458–465) gains:
  ```rust
  /// Names `spawn` removes from the child's inherited environment, after `.envs` (MOD-97 D5).
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub env_remove: Vec<String>,
  ```
  Derives stay `Clone, PartialEq, Eq, Serialize, Deserialize`. In `Debug` (:467–475), switch to `let mut s = f.debug_struct(..)`, add the three existing fields, add `if !self.env_remove.is_empty() { s.field("env_remove", &self.env_remove); }`, then `s.finish()`.
- New helpers, placed after `mark_session` (:621):
  ```rust
  pub(crate) fn billing_env_to_strip(billing: htui_core::model::Billing, settings: &AgentSettings) -> Vec<String>
  //   Subscription => settings.api_billing_env without SESSION_VAR; PerToken => Vec::new()

  pub(crate) fn strip_billing_env(resolved: &mut ResolvedLaunch, names: &[String], agent: &str)
  //   for name: let named = resolved.env.remove(name).is_some();
  //             if named || std::env::var_os(name).is_some() {
  //                 warn!(agent = %agent, variable = %name, "...stripped from a subscription-billed session")
  //             }
  //   resolved.env_remove = names.to_vec();
  ```
- `spawn_supervised` (:1184): add `for key in &launch.env_remove { command.env_remove(key); }` right after the builder chain ends at :1199 and before the `HTUI_OWN_ENV` loop (:1202). Add a short MOD-97 D5 comment.

**`crates/htui-agent/src/cli/mod.rs`**

- Add the field `strip_env: Vec<String>` to `CliDriver` (:501–514). Leave the `Debug` impl at :516 unchanged.
- In `from_row_with_probe` (:559), hoist the settings parse out of the literal at :573:
  - `let settings: AgentSettings = serde_json::from_value(agent.settings.clone()).unwrap_or_default();`
  - `let strip_env = crate::launch::billing_env_to_strip(agent.billing, &settings);`
- In `over()` (:627, test-support), set `strip_env: Vec::new()`. A prepared transport never spawns.
- In `launch_for` (:598), the order becomes `extend(spec.env)` (:609), then `strip_billing_env(&mut resolved, &self.strip_env, &self.name)`, then `mark_session` (:610). `io_in` only reassigns `resolved.args` (:679), so `env_remove` survives to `launch::spawn` (:687).

**`crates/htui-agent/src/acp/mod.rs`**: the same three edits.

- Struct at :190; `from_row_with_probe` at :253 with the literal at :269–278.
- `over()` at :369.
- `launch_for` at :323–328: strip after :325, before `mark_session` at :326.
- `launch_in` and `auth_source` (login) stay unstripped (D5).

The ACP adapter passes its own `process.env` to the CLI, so removing the variable from the adapter process is enough.

#### Struct-literal breakage list: all 27 sites need `env_remove: Vec::new(),`

None of them uses `..`. Nothing outside htui-agent names `ResolvedLaunch`, and there are no destructuring patterns.

- **src (6):**
  - `box_probe/hardware.rs:247`, `:266`
  - `box_probe/mod.rs:216`
  - `launch.rs:537` (`resolve`, the production constructor)
  - `probe.rs:604`, `:1119`
- **tests (21):**
  - `acp_driver.rs` (12): `:306`, `:361`, `:431`, `:863`, `:924`, `:1008`, `:1047`, `:1096`, `:1135`, `:1241`, `:1278`, `:1602`
  - `auth.rs` (2): `:907`, `:1137` (`:1193` mutates an existing value and needs nothing)
  - `cli_driver.rs:2063`
  - `estimator_live.rs:639`
  - `launch.rs`: `:574`, `:752`
  - `probe.rs`: `:1047`, `:1522`, `:2444`

**No `CliDriver {` or `AcpDriver {` literal exists outside the two `Self {` sites per driver.** All `AgentSettings {` literals already end in `..AgentSettings::default()`: `acp/mod.rs:2199`, and `acp_driver.rs:1541`, `:2233`, `:2264`, `:2336`.

### 2. Tests (write first)

**T1, `crates/htui-agent/tests/launch.rs`**

- `debug_names_env_remove_only_when_set`: with an empty list the output stays `... env: {} }`; with a non-empty list it contains `env_remove: ["HTUI_TEST_VENDOR_KEY"]`.
- `a_resolved_launch_without_env_remove_serialises_three_keys`:
  - `to_value` keys are exactly `["command","args","env"]`;
  - JSON without the key parses with an empty `env_remove`;
  - a non-empty list round-trips.
- `env_remove_beats_the_launch_env` (`cfg(unix)`): `launch.env` sets `HTUI_TEST_VENDOR_KEY` and `PLAIN=kept`, with `env_remove = [HTUI_TEST_VENDOR_KEY]`. The child prints `rc=1` and `kept`. This proves the removal runs after `.envs`.
- `an_inherited_variable_in_env_remove_never_reaches_the_child` (`cfg(unix)`): copy the in-file precedent `an_inherited_htui_qdrant_api_key_never_reaches_an_agent` (:1175–1210) with its own `CHILD` constant.
  - The parent re-runs itself with `HTUI_TEST_VENDOR_KEY=SENTINEL-VK`.
  - The child asserts the variable is inherited, then spawns `printenv HTUI_TEST_VENDOR_KEY; echo "rc=$?"; printenv PLAIN`.
  - Add a `spawn_sh_with(script, env, env_remove, cwd)` beside `spawn_sh_with_env` (:747), or build the literal inline.
- `the_seeded_subscription_rows_name_their_api_billing_env`: modelled on :426. Parse the seeds through `AgentSettings`:
  - `claude` and `claude-cli` carry `["ANTHROPIC_API_KEY","ANTHROPIC_AUTH_TOKEN"]`;
  - `agy` carries an empty list;
  - `{}` parses to an empty list.

**T1, `crates/htui-agent/tests/cli_driver.rs`** (`cfg(unix)`; use `scripted_row` at :1326, `spec` at :59, and `launch_for_marks_the_session_whatever_the_setting` at :2610 as the model)

- `launch_for_strips_the_billing_env_on_a_subscription_row`:
  - the row's `launch.env` has `HTUI_TEST_VENDOR_KEY`;
  - the settings set `api_billing_env = ["HTUI_TEST_VENDOR_KEY","HTUI_TEST_VENDOR_TOKEN"]`;
  - `spec.env` has `HTUI_TEST_VENDOR_TOKEN` (a project secret) and `OTHER`;
  - expect both names gone from `env`, `OTHER` and `HTUI_SESSION=1` kept, and `env_remove` equal to the list in order.
- `launch_for_keeps_the_billing_env_on_a_per_token_row`: `row.billing = Billing::PerToken`, so both variables are kept and `env_remove` is empty.
- `launch_for_strips_nothing_without_the_list`, plus an unreadable-settings case (`settings = json!("x")`) that also strips nothing.
- `a_billing_list_cannot_strip_the_session_marker`: the list names `HTUI_SESSION`; it stays `1` and is absent from `env_remove`.
- **T3 (D6a):** `the_seeded_cli_row_never_asks_for_bare`.
  - Do not call `launch_for` here: it would resolve `${claude}` on PATH and fail on CI.
  - Build the seeded `claude-cli` row from `seed_rows`, parse `AgentLaunch` and `AgentSettings`, then call `argv(&launch.args, &cli, &spec, "sid", None, settings.user_extensions)` (`cli/mod.rs:159`).
  - Assert: no `--bare` in the argv, none in `extra_args`, and no `CLAUDE_CODE_SIMPLE` in `launch.env`.

**T1, `crates/htui-agent/tests/acp_driver.rs`**: model on :917, using `unresolvable_row`, `snapshot`, `on_box`, `plain_file` and `MARKER`.

- `launch_for_strips_the_billing_env_on_a_subscription_row`: the recorded `env` holds `HTUI_TEST_VENDOR_KEY` and `spec.env` holds `HTUI_TEST_VENDOR_TOKEN`. Both are stripped and `env_remove` is set. Then `driver.launch_in(cwd)` (the login path) still contains the recorded variable with an empty `env_remove`, which pins the D5 scope.
- `launch_for_keeps_the_billing_env_on_a_per_token_row`.

**T1, `crates/htui-agent/tests/extensibility.rs`**: `the_spawn_path_names_no_billing_variable`, a copy of :312–338 with `VOCABULARY = ["ANTHROPIC_"]`, scanning `production_half`. It should pass already; the only current hit, `htui-worker/src/runtime.rs:5859`, sits after the `mod tests` marker at :3569. Do not add the literal `zeta` anywhere.

**T3, `crates/htui-agent/tests/cli_live.rs` (D6b)**

- Refactor `Run::start` (:312) into "resolve" plus a new `Run::spawn(resolved: ResolvedLaunch, cwd)`, which takes over :322–348.
- New case `case_11_a_subscription_row_ignores_an_inherited_api_key`, marked `#[ignore = "spends model tokens..."]`.
  - It must re-run itself, because `set_var` is unsafe and the workspace forbids `unsafe_code`. Child arguments: `[NAME, "--exact", "--ignored", "--test-threads=1", "--nocapture"]`, environment `CHILD=1` and a bogus `ANTHROPIC_API_KEY` (optionally `ANTHROPIC_AUTH_TOKEN` too). The parent asserts `"1 passed"`.
  - The child:
    - takes `ONE_AT_A_TIME` and asserts the key is inherited;
    - builds `CliDriver::from_row` on the seeded `claude-cli` row and calls `launch_for(&spec)`, asserting `env_remove` contains `ANTHROPIC_API_KEY`;
    - sets `resolved.args = cli::argv(...)` with a UUIDv7 session id, as case 2 does;
    - calls `Run::spawn`, sends "reply with exactly the word ok", and runs `read_result`;
    - asserts the `system/init` line has `apiKeySource == "none"` and the result passes `is_success`.
- Update the file header (:13–17, which says "never through `src/cli/`") and the case list (:67–88). This case deliberately goes through the driver.
- This case only runs on the host.

**T2, `crates/htui-core/src/model/agent.rs`**: in `seed_rows_match_ana4_5_3` (:214; the test half starts at :206, so the literal is safe), assert the list on `rows[0]` and `rows[2]`, and assert `rows[1].settings.get("api_billing_env").is_none()`.

### 3. Migration 0028

`crates/htui-store/migrations/0028_seed_api_billing_env.sql`. Use 0018's header style: `-- 0028_seed_api_billing_env.sql - MOD-97 (plan D3, D4).` then `-- Forward-only (R-STO-5).` The explanation should cover:
- insert-once seeds;
- run before the seed pass, so a fresh database is a no-op;
- an explicit value, `[]` included, is left alone;
- the trigger moves `updated_at`;
- no cache migration (refers to `cache_migrations/0002_agent_mirror.sql`).

```sql
UPDATE agent
   SET settings = settings || '{"api_billing_env": ["ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN"]}'::jsonb
 WHERE ((name = 'claude' AND transport = 'acp') OR (name = 'claude-cli' AND transport = 'cli'))
   AND jsonb_typeof(settings) = 'object'
   AND NOT (settings ? 'api_billing_env');
```

`build.rs` already reruns on the `migrations/` directory, and unchecked queries need no `.sqlx` prepare.

**Tests in `crates/htui-store/tests/migrations.rs`.** Use `run_to(28)`, not `run`, so a future 0029 cannot break them.

- `migration_0028_seeds_the_billing_env_once`:
  - `run_to(27)`, then `insert_agent` (:1413) for `claude` (acp, `old_acp_settings()` plus `permission_mode: "default"`), `claude-cli` (cli, `old_cli_settings()`), `agy`, and `my-claude`;
  - after `run_to(28)`, `claude` and `claude-cli` equal their old settings plus the list, with `updated_at > at`;
  - `agy` and `my-claude` are byte-identical, `updated_at` included;
  - a `rerun(28)` changes nothing. Generalise `rerun_0018` (:1444) into `rerun(pool, version)`.
- `migration_0028_leaves_explicit_and_other_rows_alone`: one database per case.
  - `claude` with `[]`, with `["MY_VAR"]`, and with an explicit JSON `null`;
  - a `claude` row on transport `cli`;
  - a `claude-cli` row on transport `acp`.
- `a_fresh_database_seeds_the_billing_env`: `common::fresh_db()` (migrated and seeded); `SELECT name, settings->'api_billing_env' FROM agent ORDER BY name` returns `agy` → NULL, and `claude` and `claude-cli` → the list.

**Must-fix existing tests.** Both 0018 tests run `MIGRATOR.run` after `run_to(17)`, so they will now apply 0028 too and see the patched `claude` and `claude-cli` rows.
- `migration_0018_pins_the_old_claude_row_once` at `migrations.rs:1480`: change to `MIGRATOR.run_to(18, &db.pool)` and fix the doc sentence at :1465 ("plain `run` afterwards applies 0018 alone").
- `migration_0018_leaves_edited_and_other_rows_alone` at `migrations.rs:1546`: same change.
- I checked every other `run_to(N)` + `run` test (0019–0026): none inserts agent rows.

**Count pins from 27 to 28.** The plan listed only the `Pending(27)` sites; the vector at :116, the bare `27` at `connect.rs:156` and the count at :4204 are additional.

- `crates/htui-store/tests/connect.rs`:
  - `:140`, `:242` `Pending(27)`;
  - `:156` the bare `pending, 27`;
  - prose at `:141–142`, `:157–158`, `:243–244`: "twenty-seven" becomes "twenty-eight", and "through MOD-163's 0027_trim_record_v5.sql" becomes "through MOD-97's 0028_seed_api_billing_env.sql".
- `crates/htui-store/tests/migrations.rs`:
  - `:116` add `28` to the version vector, and extend the ordinal prose at `:129` ("…and MOD-163 M2's 0027_trim_record_v5.sql and MOD-97's 0028_seed_api_billing_env.sql");
  - `:3919`, `:4020` `Pending(27)`;
  - `:4029`, `:4052` `MigrationsPending(27)`;
  - `:4204` the bare `27` count;
  - prose at `:3920`, `:4021`, `:4030`, `:4053`, `:4205–4206`.

**Not pins, leave alone:**
- `migrations.rs:4957` `[(&str, usize); 27]` counts tables;
- the `TABLES` constant (:32), since 0028 adds no table;
- `MigrationsPending(2/3/5)` in `crates/htui/src` and `tests/connection.rs`, which are synthetic counts.

The commit that added 0027 (`db8dcfd0`) touched only these two test files.

### 4. Seeds

- `crates/htui-core/seeds/agent_claude.json`: add `"api_billing_env": ["ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN"]` as a top-level key of `settings`.
- `crates/htui-core/seeds/agent_claude_cli.json`: the same.
- `agent_agy.json`: unchanged.

**What else moves:**
- No `.snap` file renders agent settings. The only hits on `model_usage` and `acceptEdits` are `cli_map__*.snap`, which are stream mappings.
- `old_cli_settings` and `old_acp_settings` (`migrations.rs:1381`, `:1392`) are deliberately the old settings and stay as they are.
- `fixtures.rs`, the in-memory store and the conformance suite re-stamp `seed_rows`, so they follow the seeds automatically.
- `merge_settings` in the Settings form (`crates/htui/src/agent_settings.rs:380`) keeps unknown keys, so `api_billing_env` survives an edit there.
- Still run the full insta suite to confirm.

### 5. Risks and gotchas

1. The two 0018 tests above will fail as soon as 0028 lands, and the plan does not mention them.
2. The vendor sweep reads every line of any `src/` file that lacks the exact `\n#[cfg(test)]\nmod tests {` marker. `launch.rs` has none, so a doc comment saying `ANTHROPIC_API_KEY` there fails the sweep. `cli/mod.rs` (test half from :1942), `acp/mod.rs` (:2117) and `htui-core/src/model/agent.rs` (:206) are safe only below those lines.
3. The exact `Debug` assertions at `tests/launch.rs:627` and `:640` force the print-only-when-set `Debug`.
4. Probe round-trips: `ProbeSnapshot` serialises `ResolvedLaunch` into `agent_box.probe`. Without `skip_serializing_if`, every recording would gain `"env_remove":[]`, and the snapshot fixtures in `probe.rs` and `acp_driver.rs` (built through `agent_box_row`) would move.
5. The live case needs the re-run pattern with `--ignored` in the child arguments, or the child filter matches nothing. The existing `"1 passed"` assertion catches that.
6. Integration tests run zero tests without `--all-features`. Use `cargo test -p htui-agent --all-features -- --test-threads=1`, plus a featureless `cargo clippy --workspace -- -D warnings`.
7. The `warn!` path has no capture harness in htui-agent (no tracing-subscriber dev-dependency). Leave the log line itself untested; behaviour is pinned by `env`/`env_remove` at the driver level.
8. No other branch carries a 0028 (`git log --all` shows nothing), so there is no ordinal clash today.

### 6. Build sequence

1. **T1 red:** the `tests/launch.rs` cases (Debug, serde, the two spawn cases), then the CLI and ACP driver cases, then the sweep.
2. **T1 green:**
   - `launch.rs`: field, `Debug`, helpers, `spawn_supervised`;
   - the 27 literal sites;
   - the CLI and ACP driver fields, constructors and `launch_for`.
3. **T2 red:** the 0028 tests, the seed asserts in `agent.rs` and `tests/launch.rs`, and the fresh-database test.
4. **T2 green:** the two seeds, the SQL file, the count pins and prose, and the two 0018 tests switched to `run_to(18)`.
5. **T3:** `the_seeded_cli_row_never_asks_for_bare`; the `Run::spawn` refactor; `case_11` plus the header and case-list docs. `case_11` itself waits for a host run.
6. **T4:**
   - `docs/decisions/mod/mod-97.md`: header in the `mod-96.md` style; D1–D6; the unreadable-settings → empty-list → no-strip note; that 0028 and the 0018 tests now use `run_to`; commit hashes.
   - The plan's one line in `docs/hr-sandbox.md` near :161 (`--api-key`): `htui` does not bill subscription rows to the forwarded key.
   - Optionally, an "Amended by MOD-97" note on ANA-4 §5.2, which lists the `agent.settings` keys.
7. **Gate:** `bash scripts/gate.sh`, featureless clippy, and the store suite against `HTUI_TEST_DATABASE_URL` with `--test-threads=1`.

Key files:
- `/home/mluigi/projects/htui/crates/htui-agent/src/launch.rs`
- `/home/mluigi/projects/htui/crates/htui-agent/src/cli/mod.rs`
- `/home/mluigi/projects/htui/crates/htui-agent/src/acp/mod.rs`
- `/home/mluigi/projects/htui/crates/htui-agent/tests/launch.rs`
- `/home/mluigi/projects/htui/crates/htui-agent/tests/cli_driver.rs`
- `/home/mluigi/projects/htui/crates/htui-agent/tests/acp_driver.rs`
- `/home/mluigi/projects/htui/crates/htui-agent/tests/extensibility.rs`
- `/home/mluigi/projects/htui/crates/htui-agent/tests/cli_live.rs`
- `/home/mluigi/projects/htui/crates/htui-core/seeds/agent_claude.json`
- `/home/mluigi/projects/htui/crates/htui-core/seeds/agent_claude_cli.json`
- `/home/mluigi/projects/htui/crates/htui-core/src/model/agent.rs`
- `/home/mluigi/projects/htui/crates/htui-store/migrations/0018_seed_permission_mode.sql`
- `/home/mluigi/projects/htui/crates/htui-store/tests/migrations.rs`
- `/home/mluigi/projects/htui/crates/htui-store/tests/connect.rs`
- `/home/mluigi/projects/htui/docs/hr-sandbox.md`