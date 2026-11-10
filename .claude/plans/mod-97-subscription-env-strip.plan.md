# Plan: MOD-97 — strip inherited vendor API keys from subscription-billed agent spawns

**Source**: `HANDOFF.md` MOD-97 (from ANA-30 §2.3, §8 call 7). `R-AGT-5`, `R-AGT-7`, `R-AGT-9`, `R-SEC-2`.
**Routed**: plan path (`/handoff-run`, maintainer accepted 2026-11-10; C3 fired, C2 borderline).
**Complexity**: Small–Medium (htui-agent spawn path, two seeds, one data migration, tests).
**Status**: awaiting CONFIRM.

## Summary

Every agent child inherits `htui`'s whole environment (`.envs(&launch.env)`, no `env_clear`), and
Claude Code in `-p` mode always uses `ANTHROPIC_API_KEY` when present. A `billing = subscription`
row therefore silently bills the API whenever `htui` runs where the key is exported (`hr attach
--api-key`/`--shell`) or a project secret carries that name. The fix: a registry row names the
variables that move it onto API billing (row data, `R-AGT-5`); on a `subscription` row the driver
removes them from the spawn — inherited, row-set and project-secret alike. `--bare` behaviour is
pinned by a seed test and a host-owed live test, since the CLI has no `--no-bare` flag to pin it
explicitly.

## Decisions (settled here, confirm at CONFIRM)

- **D1 — strip, not refuse.** The row's billing says what the operator wants; the hazard is an
  ambient variable nobody chose for this row. Refusing would make every claude run fail inside an
  `hr attach --api-key` sandbox, and would let a stale shell export block work. Stripping keeps the
  row honest: it runs on the subscription login or fails auth as a subscription row should. Each
  strip of a variable that was actually present logs one `warn!` naming the variable and the agent
  (name only, never the value). A per-token run is a `per_token` row, by design (ANA-30 §2.3 (c)).
- **D2 — the list is row data: `agent.settings.api_billing_env: [String]`.** Top-level in
  `AgentSettings` (both transports, like `user_extensions`), `#[serde(default)]` empty. Applied
  only when `agent.billing = subscription`; a `per_token` row carrying the same list keeps the
  variables, so copying `claude-cli` and flipping billing to `per_token` (ANA-30 route (c)) works
  without editing the list. No vendor name in Rust source: a row without the key strips nothing.
  Not `discovery.credential.env`: that block means "this variable counts as being logged in"
  (agy's `GEMINI_API_KEY`), the opposite meaning.
- **D3 — seeded on `claude` and `claude-cli`** with `["ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN"]`.
  `agy` is **not** seeded: its `credential.env` lists `GEMINI_API_KEY` as a login tier, so
  stripping it would make the probe report ready and the session fail. (Open to the maintainer:
  whether agy + `GEMINI_API_KEY` is per-token billing is unverified; leave as a follow-up.)
- **D4 — existing databases get it by migration `0028_seed_api_billing_env.sql`**, mirroring
  `0018_seed_permission_mode.sql`: seeds are insert-once by name, so a box seeded earlier would
  otherwise keep the hazard. Patches `claude` (acp) and `claude-cli` (cli) only while the key is
  absent (`settings ? 'api_billing_env'` false); an explicit value, `[]` included, is left alone;
  second run matches nothing. No cache migration (mirror replaces the agent table whole).
- **D5 — mechanics.** `ResolvedLaunch` gains `env_remove: Vec<String>`
  (`#[serde(default, skip_serializing_if = "Vec::is_empty")]`, so a probe snapshot round-trips
  byte for byte and the recorded launch never carries it). Each driver computes its strip list in
  `from_row_with_probe` (billing + parsed settings). In `launch_for`, after
  `resolved.env.extend(spec.env)`, a shared `launch::strip_billing_env` removes the names from
  `resolved.env` (row env and project secrets) and sets `resolved.env_remove`. `spawn_supervised`
  calls `env_remove` for each, after `.envs` and beside the `HTUI_OWN_ENV` loop (MOD-158 D5),
  so nothing re-adds them. Probes and the login flow (`launch::spawn` from `probe.rs`, `auth/run.rs`)
  are out of scope: they do not run billed turns.
- **D6 — `--bare` pin.** Claude Code 2.1.322 has no `--no-bare`; `--bare` sets
  `CLAUDE_CODE_SIMPLE=1` and makes auth strictly `ANTHROPIC_API_KEY`/`apiKeyHelper`. Pinned by:
  (a) a unit test that the seeded `claude-cli` row's built argv has no `--bare` and its env sets no
  `CLAUDE_CODE_SIMPLE`; (b) an `#[ignore]` live case in `cli_live.rs` that spawns the seeded row
  through `CliDriver::launch_for` with a bogus `ANTHROPIC_API_KEY` in the parent environment and
  asserts the turn authenticates with `system/init.apiKeySource == "none"`. When the CLI makes
  `--bare` the `-p` default, (b) fails loudly on the host. (b) is owed on the host (spends tokens;
  the sandbox has no login).

## Patterns to Mirror

| Category | Source | Pattern |
|---|---|---|
| Env removal at spawn | `crates/htui-agent/src/launch.rs:1201-1206` | `HTUI_OWN_ENV` loop of `command.env_remove` after `.envs` (MOD-158 D5) |
| Per-session env step | `crates/htui-agent/src/launch.rs:621-623`, `cli/mod.rs:608-610`, `acp/mod.rs:324-327` | `mark_session` after `spec.env` in both `launch_for` |
| Settings field | `crates/htui-agent/src/launch.rs:286-306` | top-level `AgentSettings` field, `#[serde(default)]`, both transports (`user_extensions`) |
| Seed-patch migration | `crates/htui-store/migrations/0018_seed_permission_mode.sql` | guarded `UPDATE agent SET settings = jsonb_set(...)` by name/transport, idempotent |
| Migration test | `crates/htui-store/tests/migrations.rs:1458` | 0018's pre-seeded-row test |
| Inherited-env test | `crates/htui-orch/src/verify.rs:781-827` | re-run the test binary as a child with the variable set, assert the grandchild never sees it |
| No-vendor sweep | `crates/htui-agent/tests/extensibility.rs:312-347` | `production_half` scan of `src/` for vendor literals |
| Errors/logging | `launch.rs` `warn!(field = %..., "...")` | names only, never values (`R-SEC-2`) |

## Files to Change

| File | Action | Why |
|---|---|---|
| `crates/htui-agent/src/launch.rs` | UPDATE | `AgentSettings.api_billing_env`; `ResolvedLaunch.env_remove` (+ `Debug`); `strip_billing_env`; `spawn_supervised` removal |
| `crates/htui-agent/src/cli/mod.rs` | UPDATE | `CliDriver` strip list from row; apply in `launch_for` |
| `crates/htui-agent/src/acp/mod.rs` | UPDATE | `AcpDriver` strip list from row; apply in `launch_for` |
| `crates/htui-agent/tests/launch.rs` | UPDATE | inherited variable removed by `spawn` (child-rerun pattern) |
| `crates/htui-agent/tests/cli_driver.rs` | UPDATE | `launch_for`: subscription strips (project secret + `env_remove`), per_token keeps; seeded argv has no `--bare` |
| `crates/htui-agent/tests/acp_driver.rs` | UPDATE | same two cases for ACP |
| `crates/htui-agent/tests/extensibility.rs` | UPDATE | sweep: `ANTHROPIC_` absent from every `src/` production half |
| `crates/htui-agent/tests/cli_live.rs` | UPDATE | `#[ignore]` live case D6(b) |
| `crates/htui-core/seeds/agent_claude.json`, `agent_claude_cli.json` | UPDATE | seed `api_billing_env` |
| `crates/htui-store/migrations/0028_seed_api_billing_env.sql` | CREATE | D4 |
| `crates/htui-store/tests/migrations.rs`, `tests/connect.rs` | UPDATE | 0028 behaviour test; move the `Pending(27)` / "twenty-seven" pins to 28 |
| insta snapshots showing agent settings JSON (if any) | UPDATE | found by a full insta run, not by grep |
| `docs/decisions/mod/mod-97.md`, `HANDOFF.md`, `DECISIONS.md` | CREATE/UPDATE | close-out |

## Tasks (serial — T1 and T2 share the seed/settings coupling; small enough not to split)

### T1 — strip at spawn (htui-agent)
- **Tests first**: `launch_for` on a subscription row with `api_billing_env = ["HTUI_TEST_VENDOR_KEY"]`
  and the same name in `spec.env` → absent from `resolved.env`, present in `env_remove`; on a
  `per_token` row → kept, `env_remove` empty; on a row with no key → nothing stripped. Both
  drivers. `spawn` with `env_remove` and the variable inherited → child's `env` lacks it. A
  `ResolvedLaunch` without `env_remove` serialises with no such key (probe round-trip). Sweep test.
  Test names use a neutral variable (`HTUI_TEST_VENDOR_KEY`), not a vendor name.
- **Action**: D5.
- **Validate**: `cargo test -p htui-agent --all-features -- --test-threads=1`.

### T2 — seeds and migration
- **Tests first**: 0028 patches a pre-seeded `claude`/`claude-cli` row lacking the key, leaves an
  explicit value and other names alone, is idempotent; fresh database gets it from the seeds.
- **Action**: D3, D4; move the count pins.
- **Validate**: `cargo test -p htui-store --all-features -- --test-threads=1` against the sandbox
  Postgres (`HTUI_TEST_DATABASE_URL`); `cargo test -p htui-core`; full insta run.

### T3 — `--bare` pin
- **Action**: D6 (a) in `cli_driver.rs`, (b) in `cli_live.rs`.
- **Validate**: (a) in the suite; (b) `cargo test -p htui-agent --test cli_live -- --ignored
  <case>` **on the host**.

### T4 — docs and close-out
- `docs/decisions/mod/mod-97.md` (D1–D6), a line in `docs/hr-sandbox.md` that `htui` inside
  `hr attach --api-key` does not bill subscription rows to the key, HANDOFF/DECISIONS per
  `workflow-docs.md`.

## Validation

```bash
bash scripts/gate.sh
cargo clippy --workspace -- -D warnings          # featureless clippy (memory)
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| A sandbox with only an API key and no login loses working claude rows | Medium (by design) | `warn!` names the stripped variable; doc line in `hr-sandbox.md`; a `per_token` row is the route |
| Another in-flight branch also mints migration `0028` | Medium | rename at `scripts/hr collect`, moving the count pins (as MOD-163's 0026→0027) |
| Unreadable `agent.settings` falls back to default → empty list → no strip | Low | same fallback every other setting uses; noted in the decision record |
| Claude Code honours other API-billing variables (`CLAUDE_CODE_USE_BEDROCK`, `apiKeyHelper` in user settings) | Low | data: a user extends the list; isolated rows already pass `--setting-sources ""` |
| `--bare` default flip lands before the live case runs | Low | the live case is the canary; owed on host each release check |

## Verified claims (step 3.5)

| Claim | Verdict | Evidence |
|---|---|---|
| `spawn_supervised` uses `.envs(&launch.env)` with no `env_clear`, then removes `HTUI_OWN_ENV` | ✓ | `launch.rs:1184-1206` (read) |
| `SessionSpec.env` carries resolved secrets into the child | ✓ | `driver.rs:283-284`; both `launch_for` `extend(spec.env)` |
| Both drivers spawn sessions only through `launch_for` | ✓ | `acp/mod.rs:383`, `cli/mod.rs:662` are the only `self.launch_for(` sites; other `launch::spawn` src callers are probe and login |
| Probe's recorded `ResolvedLaunch` is reused verbatim, so the strip must be per session | ✓ | `launch_from` returns `recorded.clone()` (`launch.rs:584-602`) |
| Drivers can see billing at construction | ✓ | `from_row_with_probe(agent: &AgentRow)`, `AgentRow = htui_core::model::Agent` with `billing: Billing` |
| `AgentSettings` tolerates an unknown/new key | ✓ | `#[serde(default)]`, no `deny_unknown_fields` (`launch.rs:286-288`) |
| Seeds are insert-once by name, existing boxes need a migration | ✓ | `0018_seed_permission_mode.sql` header; same precedent |
| Next migration ordinal is 0028; pins at `Pending(27)` ×6 plus "twenty-seven" prose | ✓ | `ls migrations/` ends at 0027; `connect.rs:140,242`, `migrations.rs:3919,4020,4029,4052` |
| No `ANTHROPIC_` literal in any `src/` production half today | ✓ | only `htui-worker/src/runtime.rs:5859`, inside `mod tests` from `:3570` |
| Claude Code has no `--no-bare`; `--bare` sets `CLAUDE_CODE_SIMPLE=1`, auth strictly key/helper | ✓ | `claude --help`, 2.1.322 in the sandbox |
| Stream-json `system/init` reports `apiKeySource` (`"none"` on a login) | ✓ (live, earlier) | `crates/htui/tests/chat_live_cli.rs:75-76` records it from a host run |
| `agy`'s `GEMINI_API_KEY` is a login tier, not an API-billing marker | ✓ | `seeds/agent_agy.json` `discovery.credential.env`; `CredentialProbe` docs `launch.rs` |
| T1/T2 independent | ✗ → serial | T2's seeds parse through T1's `AgentSettings`; seeds are a known hidden coupling |
