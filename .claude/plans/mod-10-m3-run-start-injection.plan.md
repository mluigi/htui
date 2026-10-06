# Plan: MOD-10 M3 — Run-start injection

**Source PRD**: `.claude/prds/mod-10-secret-provider.prd.md`
**Selected Milestone**: 3 — Run-start injection
**Complexity**: Large
**Status**: COMPLETE 2026-10-06 (confirmed by the maintainer, OQ-A..D as recommended; fact-checked: 23 claims, 1 falsified; blueprint A-1..A-12, A-11 kept; implemented `7748b360`..`a4f9ab7e`; reviewed (rust-reviewer: 1 high graded medium, 9 low), all findings applied in R1)

## Summary

Graph runs and chats on a project whose `project.secret_provider` is set receive that project's
secrets in the agent's environment and nowhere else. The scrubber that guards every persisted
byte of the walk is built from **the same object** that hands out the env, so the two cannot
drift. Any resolution failure refuses the run with one typed reason before any agent starts, and
the verifier never sees a resolved value. The milestone also closes the scrubber carry-overs from
M1's review and M2: the 16 KiB flush seam, escaped token starts, trailing-newline normalisation,
and zeroizing what htui owns. It lands the two tests MOD-61 and MOD-62 owe.

Nothing here writes `project.secret_provider` / `secret_scope` or touches the Settings tab (M4).
Tests set the columns through the fixtures and the Postgres testkit.

Routed as PRD by `/handoff-run MOD-10` (accepted 2026-10-06, sandbox run `hr/MOD-10`, M3 only,
ultracode for the implement and review phases).

## Grounding (read on `hr/MOD-10`, base `94c434de`)

- **One choke point for every session of a graph run.** `Engine::drive_once`
  (`crates/htui-orch/src/engine.rs:6005`) builds every `SessionSpec` of the plain-step, candidate
  and judge paths ("exactly three call sites", its `#[expect]` reason). `env: BTreeMap::new()` at
  `:6099` carries the comment naming this item. It already reads the run's project at its top
  (`self.project(run.project_id)`), so the project's secret columns are in hand there.
- **The engine's scrubber is one borrowed part.** `EngineParts.scrubber: &'a dyn Scrubber`
  (`engine.rs:470`) is used at 11 production sites (prompt assembly, `to_value`, notes, the
  `Recorder::new` at `:5960`) and in 10 `EngineParts {` constructors (`engine.rs` ×7,
  `conformance.rs`, `tests/gix_isolator.rs`, `htui-worker/src/runtime.rs`). `Recorder<'a, S>`
  borrows it for `'a`, so a per-run scrubber must be owned by whoever owns the parts (the
  worker's `Kit`), not by the engine.
- **The worker builds a fresh `Kit` per task** (`crates/htui-worker/src/runtime.rs:927`
  `Kit::read`, 9 production call sites). Its scrubber is
  `MinimalScrubber::new(std::iter::empty())` (`:969`), so exact-match masking is inert on the run
  path (MOD-61). The tasks that walk sessions are `start_run` (`:2286`), `reclaim` (`:1856`),
  `resumed` (`:2146`), `on_run_unless_claimed` (`:2395`) and the resume case of `unblock`
  (`:2738`). `adopt`'s engine (`:1984`) only fences and adjudicates (`sweep_fenced`); it drives no
  session.
- **The verifier is a process singleton** built with its own empty `MinimalScrubber`
  (`runtime.rs:455-460`). It runs `sh -c` with htui's process environment
  (`crates/htui-orch/src/verify.rs:15`, plan D30) and never sees a `SessionSpec`.
- **htui cannot put a secret into its own process environment.** The workspace is edition 2024
  (`Cargo.toml:7`) with `unsafe_code = "forbid"` (`:183`), and every crate has
  `[lints] workspace = true`. `std::env::set_var` is `unsafe` in 2024: a plain call is E0133, and
  an `unsafe {}` block is refused by the lint (probed). No crate calls `set_var`. So the only
  route from a resolved map to a child process is `SessionSpec.env`.
- **`spec.env` is applied last and wins** over the agent row's environment
  (`crates/htui-agent/src/cli/mod.rs:401-419`, `acp/mod.rs:310-317`). htui's MCP token travels in
  the MCP server spec's own `env` (`htui-mcp/src/lib.rs:35` `ENV_TOKEN = "HTUI_MCP_TOKEN"`), not in
  the agent's.
- **The chat path** builds two `SessionSpec`s with `env: BTreeMap::new()`: promotion/resume
  (`crates/htui/src/agent_worker.rs:1089-1094`) and fresh chat (`:2150-2164`, whose comment names
  this item). Its scrubber is `MinimalScrubber::new(spec.env.values().cloned())` (`:4139`), so it
  bypasses `from_resolved`'s floor.
- **The provider seam exists** (`crates/htui-core/src/secret.rs`): `SecretProvider`
  (`kind`/`health`/`list_keys`/`resolve`), `SecretScope::parse` of the column, `ResolvedSecrets`
  (zeroized on drop, `as_map()`), `MachineIdentity`, and `SecretError` (15 variants, none carries
  a value). `htui-secrets` depends on `htui-core` only, and no crate depends on it yet. Keyring
  slots: `htui_store::secret::{get_infisical_url, get_machine_identity}` (`secret.rs:445`,
  `:482`).
- **`htui-core` has no `tokio`.** An async once-cell for the per-walk slot belongs in
  `htui-orch`, which has it.
- **Project rows carry the columns.** Postgres `project()` selects `secret_provider, secret_scope`
  (`crates/htui-store/src/pg/read.rs:620-632`), as does the cache (`cache/read.rs:895-909`). The
  Postgres testkit inserts both (`htui-store/src/testkit.rs:539`). `fixtures.rs:738` and
  `mem.rs:2480` default them to `None`.
- **Typed failure precedent.** `failure_text` (`engine.rs:6618`) maps exactly one error to a typed
  sentence: `RecordError::Unmasked` → `RunFailure::ScrubRefused` (`status.rs:108-125`). A driver
  that will not start fails the step and the run (`runtime.rs` `Kit::driver` doc, `RefusedDriver`).
- **The flush seam.** `CHUNK_FLUSH_BYTES = 16 * 1024` (`crates/htui-agent/src/record.rs:129`).
  The cut happens when the open text reaches the bound after a chunk is appended (`:998`), so a
  value split across two chunks can land in two rows.
- **Token start** is `(?:^|[^A-Za-z0-9_])` (`crates/htui-core/src/scrub.rs` `TOKEN_START`), so
  `\nsk-…`, `\u0022sk-…` and `%22sk-…` (letter or digit before the key) are not token starts (M1
  review L1). The audit query mirrors the rules (`scripts/scrub-audit.sql`).
- **`from_resolved`** (`scrub.rs:211`) masks values of at least `MIN_MASKED_LEN = 6` chars and
  returns the short keys; `MinimalScrubber` holds `secrets: Vec<String>` with no `Drop`.
- **Injection precedent.** The tool host reaches both runtimes the same way:
  `RunRuntime::with_tool_host` (`runtime.rs:1372`) and `AgentRuntime::with_tool_host`
  (`agent_worker.rs:659`), wired in the TUI (`crates/htui/src/lib.rs:156-173`
  `store_worker::spawn_hosted`) and the worker (`crates/htui/src/worker_cmd.rs:100-108`).
- **No live Infisical in this sandbox** (`HTUI_TEST_INFISICAL_*` unset). The M2 live test stays
  owed to a host run.

## Patterns to Mirror

| Category | Source | Pattern |
|---|---|---|
| Seam injection | `crates/htui-worker/src/runtime.rs:1372`, `crates/htui/src/agent_worker.rs:659` | `with_tool_host(Arc<dyn …>)` builder on both runtimes; wired once in `lib.rs` and `worker_cmd.rs` |
| Async seam | `crates/htui-core/src/secret.rs:21` | `SecretFuture<'a, T>` boxed future, dyn-compatible, no `async_trait` |
| Typed failure | `crates/htui-orch/src/status.rs:108-125`, `engine.rs:6618` | one `RunFailure` variant plus one `failure_text` arm; names the cause, never a value |
| Non-leaking errors | `crates/htui-core/src/secret.rs:282-390` | `thiserror` variants without values; a sentinel test over every variant |
| Keyring read | `crates/htui-store/src/secret.rs:445-520` | `get_*` free functions; `mock_keyring()` guard in tests (`testkit.rs:334`) |
| Tests (engine) | `crates/htui-orch/src/engine.rs:17485-17600` | MOD-10 D5 fake-parts cases: scripted turn, assert the typed failure sentence |
| Tests (worker) | `crates/htui-worker/src/runtime.rs:3735-3775` | `with_tool_host` cases: runtime built with a fake, assert what the engine received |
| Tests (recorder) | `crates/htui-agent/tests/recorder.rs:3518` | MOD-10 D8 flush cases over scripted chunks |

## Decisions (proposed; CONFIRM accepts or overrides)

Numbered from D11: M1 and M2 each used D1–D10, and code comments cite both as `MOD-10 Dn`.

**D11 — One per-walk secrets object that is both the env source and the scrubber.**
- New `htui_orch::secrets::RunSecrets`, owned by the worker's `Kit` (one per task). It holds an
  `Option<Arc<dyn SecretSource>>` and an async once-cell of
  `Result<(ProjectId, ResolvedSecrets, MinimalScrubber), SecretError>`.
- It implements `Scrubber`. Until it has resolved, it scrubs with the pattern rules only, exactly
  as today's empty `MinimalScrubber`. Once it has resolved, it masks with
  `from_resolved(resolved.as_map())`.
- `Kit::engine` passes `&self.secrets` as **both** `EngineParts.scrubber` and a new
  `EngineParts.secrets: &'a RunSecrets`. One object, one map: the two cannot drift.
- **Resolved lazily, at the first `drive_once` of the walk**, before `driver.start`. A walk that
  drives no session (a gate answer that only settles, a park, the sweep) never calls Infisical.
- Rejected: resolving eagerly in each of the five walking worker tasks. `unblock` builds its
  engine before it knows whether it walks, and five call sites are five chances to forget one.
  The choke point is the engine.

**D12 — When the env is filled and when the run is refused.** In `drive_once`, after the project
read:
- **`project.secret_provider` is `None`:** `env` stays empty and no provider is touched (no
  keyring read). The scope column is ignored: the provider column is the switch.
- **`Some("infisical")`:** `env = secrets.env_for(&project).await?`, which is the resolved map
  cloned.
- **Any other value, a missing or unparsable scope, or no `SecretSource` in this process:** a
  `SecretError::Config` sentence.
- **Single-project guard.** The slot binds to the first project it resolves for, and a call for a
  different project is refused (fail closed). One `Kit` walks one run, so this never fires
  legitimately.
- **The failure is cached** in the once-cell. Every later session of the same walk (fan-out
  candidates, the judge) meets the same refusal without a second login. That keeps D5's latch
  meaningful.

**D13 — One typed refusal: `RunFailure::SecretsRefused { cause }`.**
- New variant. Its `Display` is `secrets_refused: <SecretError's Display>`, which names the cause
  (no identity, unreachable, bad credentials, locked, not found, permission, invalid or reserved
  key, …) and never a value (M2 D4 guarantees the error carries none).
- New `EngineError::Secrets(SecretError)`; `failure_text` maps it like `ScrubRefused`.
- **The refusal fails the run on every path**, including the candidate and judge paths. One
  resolution per walk means every session of the walk would meet it, so failing a candidate
  (then `NoSurvivingCandidate`) or parking the judge would only hide the cause.
- **No agent process is started.** The refusal is raised before `driver.start`, so it takes the
  path a `RefusedDriver` takes today.
- **Transient causes fail the run too** (`Unreachable`, `RateLimited`, `LoginCoolingDown`). There
  is no automatic retry, so a cool-down or lockout is never chased (see OQ-A).
- Chats are refused before `start_chat_run` / before the promoted session starts, with the same
  sentence. No chat run row is left behind.

**D14 — Resolve once per walk; a resume re-resolves.** Values are never persisted, so a resumed,
retried or adopted walk (MOD-37 resume, sweep adoption, `r`, gate answers) resolves again in its
own `Kit`. Within one walk, every step, candidate and judge shares the one map. This settles the
PRD's "resolve-once vs per-step" question: per walk.

**D15 — One provider per process and identity (M2 carry-over).**
- New `htui_core::secret::SecretSource` trait:
  `fn provider(&self) -> SecretFuture<'_, Arc<dyn SecretProvider>>`.
- The production source lives in `crates/htui/src/secrets.rs`, the only crate that depends on
  both `htui-store` (keyring) and `htui-secrets` (HTTP). On every call it:
  - reads the keyring on a blocking thread: URL and machine identity;
  - compares `(normalised URL, client_id, client_secret)` with the cached provider's and rebuilds
    `InfisicalProvider` **only when one differs**, so M4 storing a new identity or URL takes
    effect at the next walk;
  - returns the cached `Arc` otherwise, so the 401 latch and the cool-down persist across walks
    and chats.
- One source is shared by the run runtime and the agent runtime of a process. The TUI has one,
  and `htui worker` has its own.
- **Missing keyring URL:** `Config("no Infisical base URL is stored in the OS keyring")`. Missing
  identity: `NoIdentity`. Keyring backend error: `Config` naming the keyring, never a value.

**D16 — Pure resolution helpers in `htui-core::secret`.**
- `project_scope(&Project) -> Result<Option<SecretScope>, SecretError>` covers D12's column rules.
- `check_env(&ResolvedSecrets) -> Result<(), SecretError>` refuses **reserved names**: any key
  with the `HTUI_` prefix (case-insensitive, R1 2026-10-06, Windows env names are
  case-insensitive; was case-sensitive). This adds the new variant
  `SecretError::ReservedKey { key }`: "the secret name `X` is reserved for htui; rename it in
  Infisical". htui's own variables (`HTUI_MCP_*`, `HTUI_LOG*`, `HTUI_TOOL_*`) must never be
  shadowed by a project secret that `spec.env` would apply last.
- A `test-support` `FakeSecretSource` / `FakeSecretProvider` with a scripted result per call and
  call counters, used by the orch, worker and htui tests. No keyring, no HTTP.

**D17 — Scrubber carry-overs (`htui-core::scrub`).**
- **Trailing newline** (M1, M2): `from_resolved` masks each value and, when it differs and is
  still at least the floor, its `trim_end_matches(['\r', '\n'])` form. Values are still injected
  byte for byte.
- **Escaped token starts** (M1 review L1): `TOKEN_START` also accepts a preceding `\n`, `\r`,
  `\t`, `\"` (backslash plus one of `nrtbf"/\`), `\uXXXX` and `%XX`. The same change is mirrored
  in `scripts/scrub-audit.sql`. It widens what fails closed, so the host audit runs before merge
  (OQ-D).
- **`(?-u:\b)`** (review N1) is not adopted; no benchmark is planned.
- **Zeroize** (M2 carry-over): `MinimalScrubber` gets a `Drop` that zeroizes its secret list.
  `RunSecrets` drops its `ResolvedSecrets`, which already zeroizes. **Documented residuals:**
  `SessionSpec.env` (a plain `BTreeMap`, moved into the driver) and the OS child-environment
  block that `std::process::Command` copies. Neither is htui's to wipe (OQ-C).
- **Short values** stay M1's rule: they are injected but not masked. The walk logs their **key
  names** once at `warn` (never values) so the operator can lengthen them.

**D18 — Flush seam (M1 review M2).**
- `Scrubber` gains one **defaulted** method, `fn hold_back(&self) -> usize { 0 }`. It is the
  number of trailing bytes a cut must keep open. `MinimalScrubber` returns
  `max(longest secret in bytes, PATTERN_HOLD_BACK) - 1`, where `PATTERN_HOLD_BACK` is a constant
  sized for the longest fixed-length rule; `RunSecrets` delegates.
- At the size trigger, the recorder cuts the open text at the last char boundary at least
  `hold_back` bytes before its end, and carries the tail into the next row. A complete occurrence
  is therefore always scrubbed whole in one row, and a partial one waits for its rest.
- Existing call sites are unchanged (defaulted method). The architect decides how a cut that
  would land inside an already-complete match moves back to the match's start.
- **Acceptance:** for a resolved secret or a pattern-shaped key split across the 16 KiB boundary
  by chunking, neither row contains it and **the two rows' text concatenated does not contain it**.

**D19 — The verifier stays outside (MOD-62).** No change to `ShellVerifier` or its pattern-only
scrubber. Grounding shows a resolved value cannot reach htui's environment (no `set_var`), and the
verifier never sees a `SessionSpec`. Update `verify.rs:15`'s comment, which made this
conditional on an empty `SessionSpec.env`.

## Open questions (decided 2026-10-06: all four as recommended)

- **OQ-A — Transient failures.** Fail the run on `Unreachable`/`RateLimited`/`LoginCoolingDown`
  like the permanent causes (**recommended**: one rule, no hidden retry loop near the lockout,
  and the maintainer re-runs), or leave the run where it is and refuse only the walk?
- **OQ-B — Reserved `HTUI_` prefix** refuses the run (**recommended**), or is it injected and
  htui's own variable wins?
- **OQ-C — Zeroize scope.** Wipe what htui owns (scrubber, resolved map) and document
  `SessionSpec.env` and the child env block as residuals (**recommended**). The alternative, a
  zeroizing wrapper type for `SessionSpec.env`, touches every `SessionSpec` constructor in
  tests.
- **OQ-D — Host audit for D17's escaped starts.** The maintainer runs `scripts/scrub-audit.sql`
  on the host before `scripts/hr collect MOD-10` merges (**recommended**; the sandbox cannot
  reach the host DB). The alternative is to skip it as M1 did.

## Files to Change

| File | Action | Why | Task |
|---|---|---|---|
| `crates/htui-core/src/scrub.rs` | UPDATE | D17 token start, newline, `Drop`; D18 `hold_back` | T1 |
| `scripts/scrub-audit.sql` | UPDATE | mirror D17's token start | T1 |
| `crates/htui-agent/src/record.rs` | UPDATE | D18 seam cut and carry | T2 |
| `crates/htui-agent/tests/recorder.rs` | UPDATE | D18 seam cases | T2 |
| `crates/htui-core/src/secret.rs` | UPDATE | D15 `SecretSource`; D16 helpers, `ReservedKey`, fakes | T3 |
| `crates/htui-orch/src/secrets.rs` | CREATE | D11 `RunSecrets` | T4 |
| `crates/htui-orch/src/lib.rs` | UPDATE | module and exports | T4 |
| `crates/htui-orch/src/engine.rs` | UPDATE | `EngineParts.secrets`, `drive_once` env (D12), `failure_text` arm, 7 constructors, tests | T4 |
| `crates/htui-orch/src/command.rs` | UPDATE | `EngineError::Secrets` | T4 |
| `crates/htui-orch/src/status.rs` | UPDATE | `RunFailure::SecretsRefused` | T4 |
| `crates/htui-orch/src/fake.rs`, `conformance.rs`, `tests/gix_isolator.rs` | UPDATE | constructors and fake parts | T4 |
| `crates/htui-orch/src/verify.rs` | UPDATE | D19 comment | T4 |
| `crates/htui-worker/src/runtime.rs` | UPDATE | `with_secret_source`, `Kit.secrets`, MOD-61/62 and refusal tests | T5 |
| `crates/htui/Cargo.toml`, `Cargo.lock` | UPDATE | depend on `htui-secrets` | T6a |
| `crates/htui/src/secrets.rs` | CREATE | D15 keyring + Infisical source | T6a |
| `crates/htui/src/agent_worker.rs` | UPDATE | chat env, `from_resolved` scrubber, refusal, `with_secret_source` | T6a |
| `crates/htui/src/lib.rs`, `worker_cmd.rs`, `store_worker.rs` | UPDATE | wire one source per process | T6b |
| `docs/htui-secrets.md` | UPDATE | "At run start" section: injection, refusals, reserved names, short values, residuals | T7 |

## Tasks

TDD throughout: each task writes its failing tests first.

### T1: Scrubber hardening carry-overs (`htui-core`)
- **Action**: D17 (escaped token starts, trailing-newline masks, `Drop` zeroize) and D18's
  defaulted `Scrubber::hold_back` with `MinimalScrubber`'s value; mirror the token start in
  `scripts/scrub-audit.sql`.
- **Tests**: `\nsk-ant-…`, `\u0022AKIA…`, `%22ghp_…` are refused; `subtask-…` and `x%2Fsk-learn`
  prose stays clean; a value `"abcdefgh\n"` masks both `abcdefgh\n` and `abcdefgh`; `hold_back`
  is 0 for the default impl and covers the longest secret.
- **Mirror**: `scrub.rs` rule tests (`each_rule_refuses_a_real_shaped_key_in_four_positions`).
- **Validate**: `cargo test -p htui-core --all-features scrub`

### T2: Flush seam (`htui-agent`), after T1
- **Action**: D18 cut-and-carry at the size trigger.
- **Tests**: a 40-char secret split across the boundary in two chunks; a pattern key split the
  same way; a secret entirely within `hold_back` of the boundary. Each asserts that no row, and
  no concatenation of adjacent rows, holds it; replay stays deterministic.
- **Mirror**: `tests/recorder.rs` MOD-10 D8 cases.
- **Validate**: `cargo test -p htui-agent --all-features --test recorder`

### T3: Secret seam additions (`htui-core`)
- **Action**: D15 `SecretSource`; D16 `project_scope`, `check_env`, `SecretError::ReservedKey`
  (extend the every-variant sentinel test), `test-support` fakes.
- **Tests**: every column case of D12; a reserved key is refused and named; the fakes count calls.
- **Validate**: `cargo test -p htui-core --all-features secret`

### T4: Engine injection (`htui-orch`), after T3
- **Action**: D11 `RunSecrets`; D12 in `drive_once`; D13 typed failure; D19 comment; update the
  10 `EngineParts` constructors.
- **Tests (engine fake parts)**:
  - a provider project's session gets the map in `spec.env`, and a provider-less project's gets
    none and never touches the source;
  - a fan-out walk resolves **once**;
  - each D12 refusal (no source, unknown kind, bad scope, provider error, foreign project) fails
    the run with `secrets_refused: …` on the plain, candidate and judge paths, and **the driver's
    `start` is never called**;
  - a trim-record string equal to a resolved value is stored `[REDACTED]` (MOD-61, engine
    level).
- **Validate**: `cargo test -p htui-orch --all-features --no-fail-fast` (grep for `SIGABRT`:
  stack headroom, memory note)

### T5: Worker wiring (`htui-worker`), after T4
- **Action**: `RunRuntime::with_secret_source`; `Kit` owns a `RunSecrets` and passes it per D11.
- **Tests**:
  - **MOD-61**: a walked step whose agent echoes the resolved value in text and in a trim-record
    string stores only `[REDACTED]`, read back from the store;
  - **MOD-62**: the step's verify command prints its environment (`env` / `set`), and the stored
    verify output holds no resolved value;
  - a runtime built without a source walks provider-less projects exactly as before;
  - a provider project without a source fails `secrets_refused`;
  - **R-SEC-2:** the agent's spec env holds only resolved keys, so no DSN, Qdrant key or identity.
- **Validate**: `cargo test -p htui-worker --all-features -- --test-threads=1`

### T6a: Production source and chat (`htui`), after T3; parallel with T4–T5
- **Action**: D15 `crates/htui/src/secrets.rs` (`KeyringInfisical`); add the `htui-secrets`
  dependency; on both chat paths, D12's column rules plus resolve, `check_env`, the env filled,
  the scrubber built by `from_resolved` over the same map, and a refusal before
  `start_chat_run` / the promoted session; `AgentRuntime::with_secret_source`.
- **Tests**:
  - the keyring source rebuilds on a changed identity and reuses the provider otherwise, so the
    401 latch survives two walks (`mock_keyring`, loopback stub from `htui-secrets` tests or a
    fake provider factory);
  - a chat on a provider project gets the env, and its stored transcript masks the value;
  - a refused chat writes no run row.
- **Validate**: `cargo test -p htui --all-features -- --test-threads=1 secrets agent_worker`

### T6b: Process wiring (`htui`), after T5 and T6a
- **Action**: build one `KeyringInfisical` per process. In the TUI, hand it to
  `store_worker::spawn_hosted` for both runtimes; in `htui worker`, give it to its `RunRuntime`.
- **Validate**: `cargo build -p htui`; the existing `run_worker` and `worker_cmd` suites.

### T7: Operator docs, after T4–T6a
- **Action**: `docs/htui-secrets.md` gains an "At run start" section: when resolution happens
  (D11, D14), what refuses and how it reads (D13), reserved names (D16), short values and
  newline masking (D17), residuals (D17), and the verifier boundary (D19).
- **Validate**: `bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh`

### Parallel lanes

| Lane | Tasks | Files (disjoint across lanes) |
|---|---|---|
| A | T1 → T2 | `htui-core/src/scrub.rs`, `scripts/scrub-audit.sql`, `htui-agent/src/record.rs`, `htui-agent/tests/recorder.rs` |
| B | T3 → T4 → T5 | `htui-core/src/secret.rs`, `htui-orch/src/{secrets,lib,engine,command,status,fake,conformance,verify}.rs`, `htui-orch/tests/gix_isolator.rs`, `htui-worker/src/runtime.rs` |
| C | T6a (after T3) | `htui/Cargo.toml`, `Cargo.lock`, `htui/src/secrets.rs`, `htui/src/agent_worker.rs` |
| — | T6b, then T7 | `htui/src/{lib,worker_cmd,store_worker}.rs`; `docs/htui-secrets.md` |

The file sets of A, B and C do not intersect (checked below). A and B both compile `htui-core`,
so concurrent lanes run in isolated worktrees, or a lane waits on the other's commit before its
gate (memory: shared-tree fan-out coupling). T4's `RunSecrets` uses `from_resolved` and
`Scrubber` as they exist today, so lane B does not wait on lane A. `hold_back` is defaulted.

## Validation

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo clippy --workspace -- -D warnings                      # featureless gate (memory)
cargo test -p htui-core --all-features
cargo test -p htui-agent --all-features
cargo test -p htui-orch --all-features --no-fail-fast        # grep SIGABRT
cargo test -p htui-secrets --all-features
cargo test -p htui-worker --all-features -- --test-threads=1
cargo test -p htui --all-features -- --test-threads=1        # HTUI_TEST_DATABASE_URL set: Postgres cases run
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```

On the host, before merge: `scripts/scrub-audit.sql` (OQ-D) and, once,
`crates/htui-secrets/tests/infisical_live.rs` against the self-hosted instance (M2 carry-over).

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| A walking path drives a session without resolving | Low | D11: resolution sits at `drive_once`, the single session choke point, so there is no per-task call to forget |
| A wider token start fails closed on real transcripts | Medium | OQ-D host audit before merge; rules mirrored in the audit SQL |
| The seam cut changes row boundaries in replay snapshots | Medium | `hold_back` is 0 when no secret is resolved and only the size trigger moves; full insta run (memory: snapshot impact) |
| Three runs lock the identity | Low | D15 one provider per process; D12 failure cached per walk |
| The `RunSecrets` once-cell in the walk future grows the stack | Medium | Box the resolution future (memory: `every_case_name_dispatches` headroom) |
| The keyring fake is process-wide, so tests flake under parallelism | Medium | Engine and worker tests use `FakeSecretSource`; only T6a's source test takes `mock_keyring`, and the gate runs `--test-threads=1` |
| A resolved value lands in `tracing` output | Low | Only key names are logged (D17); every error type is value-free (M2 D4) |

## Acceptance

- [ ] A provider project's graph sessions and chats receive the map in `spec.env`; provider-less
      projects are unchanged and never touch the keyring
- [ ] The run-path and chat-path scrubbers are built from the same map as the env (D11, T6a)
- [ ] Every D12 refusal fails the run with `secrets_refused: …` before any agent starts
- [ ] MOD-61 and MOD-62 tests pass; the seam test passes
- [ ] All validation passes; `docs/htui-secrets.md` updated
- [ ] Host steps recorded in the phase note: audit (OQ-D) and live Infisical test

## Verified claims

| Claim | Verdict | Evidence |
|---|---|---|
| `drive_once` builds every graph `SessionSpec`; env empty at `:6099` | VERIFIED | `engine.rs:6005-6110` read via Gortex |
| `drive_once` reads the run's project first | VERIFIED | `let project = self.project(run.project_id).await?;` first line |
| `EngineParts.scrubber` has 11 production uses; `Recorder` borrows it for `'a` | VERIFIED | grep `parts\.scrubber\b`; `record.rs:386` `scrubber: &'a dyn Scrubber` |
| 10 `EngineParts {` constructors | VERIFIED | grep: engine.rs 7, conformance.rs 1, tests/gix_isolator.rs 1, runtime.rs 1 |
| `Kit` built per task; run scrubber empty | VERIFIED | `runtime.rs:927-990`, `:969` |
| Verifier built once with its own empty scrubber | VERIFIED | `runtime.rs:455-460` |
| `adopt` drives no session | VERIFIED (doc) | `engine.rs:3002-3006`: "the sweep adopts and adjudicates but does not walk" |
| `unblock` builds its engine before knowing the case | VERIFIED | `runtime.rs:2738-2750` |
| `set_var` cannot compile in this workspace | VERIFIED (probe) | rustc 2024: E0133 bare; `forbid(unsafe_code)` refuses `unsafe {}`; all 8 crates `[lints] workspace = true`; no `set_var` call in `crates/` |
| `spec.env` applied last over the row env | VERIFIED | `cli/mod.rs:401-419`, `acp/mod.rs:310-317` |
| MCP token is in the MCP server spec env, not the agent's | VERIFIED | `acp/mod.rs:1085-1100` `acp_server` uses `spec.env` of `McpServerSpec`; `htui-mcp/src/lib.rs:35` |
| Chat builds two empty-env specs; scrubber via `new`, not `from_resolved` | VERIFIED | `agent_worker.rs:1089-1094`, `:2150-2164`, `:4139` |
| `htui-secrets` depends on `htui-core` only; nothing depends on it | VERIFIED | `crates/htui-secrets/Cargo.toml`; deps of htui, htui-worker, htui-orch |
| `htui-core` has no tokio | VERIFIED | `crates/htui-core/Cargo.toml` `[dependencies]` |
| Postgres and cache project reads include the secret columns | VERIFIED | `pg/read.rs:620-632`, `cache/read.rs:895-909` |
| `failure_text` types only the scrub refusal today | VERIFIED | `engine.rs:6618-6625` |
| Flush bound 16 KiB, cut after an appended chunk | VERIFIED | `record.rs:129`, `:998` |
| `TOKEN_START` misses escaped starts | VERIFIED | `scrub.rs` `TOKEN_START = (?:^|[^A-Za-z0-9_])` |
| `from_resolved` floor 6, returns short keys; no `Drop` on `MinimalScrubber` | VERIFIED | `scrub.rs:25`, `:185-222` |
| Keyring getters exist for URL and identity | VERIFIED | `htui-store/src/secret.rs:445`, `:482` |
| `with_tool_host` on both runtimes, wired in `lib.rs` and `worker_cmd.rs` | VERIFIED | `runtime.rs:1372`, `agent_worker.rs:659`, `lib.rs:156-173`, `worker_cmd.rs:100-108` |
| Live Infisical env present in sandbox | FALSIFIED | `HTUI_TEST_INFISICAL_*` unset → host step |
| Lanes A, B, C are file-disjoint | VERIFIED | intersection of the three file sets in "Parallel lanes" is empty; the shared crate `htui-core` is handled by worktrees or serial gates |
