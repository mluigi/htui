# Blueprint: MOD-10 milestone 3 — Run-start injection

**Plan**: `.claude/plans/mod-10-m3-run-start-injection.plan.md`. D11–D19, OQ-A..D (all decided as
recommended), T1–T7 and the plan's "Verified claims" table are binding. They were re-checked
against the tree at `f8d26e35` (`hr/MOD-10`, plan confirmed) on 2026-10-06.
**PRD**: `.claude/prds/mod-10-secret-provider.prd.md`. **Earlier blueprints**:
`mod-10-m1-scrubber-hardening.blueprint.md`, `mod-10-m2-infisical-provider.blueprint.md`.
**Rule for the tree vs. the plan**: where they disagree, the tree wins. Each such point is
recorded under **Amendments** (A-n) with its evidence. Where the plan leaves a detail open, this
blueprint fixes it and says so. Anything not proven in source is marked **VERIFY — implementer
must check**.

Conventions inherited unchanged:
- toolchain and lint headers: MSRV 1.98, edition 2024 (`Cargo.toml:7-8`); workspace lints
  `unsafe_code = forbid`, `missing_debug_implementations`, `unused_qualifications`,
  `clippy::all` at `-D warnings`; `#![warn(missing_docs)]` in `htui-core`, `htui-orch`,
  `htui-agent`. `rustc 1.98.1` is the sandbox toolchain (probed);
- dyn-compatible async seams return a boxed future (`htui_core::secret::SecretFuture`); no
  `async_trait`;
- a type that holds a secret has a hand-written `Debug` that prints counts or key names only;
- no store-trait change, no migration, no `.sqlx` change. One tests-only `MemStore` setter (A-6);
- one test-helper set per file (the repo duplicates fixtures per file).

Probes run for this blueprint (scratch crate in `/tmp/m3probe`, `regex 1.13`, offline):
- D17's `TOKEN_START` (§B.1) compiles inside `RegexSet::new` and inside the `format!`-built
  `OPENAI_STRICT` / `SK_CANDIDATE` patterns, refuses `x\nsk-ant-…`, `\u0022AKIA…`, `%22ghp_…`,
  `a\tsk-ant-…`, `\"sk-ant-…`, `\\sk-ant-…`, and stays clean on `subtask-…`, `x%2Fsk-learn`,
  `nsk-ant-…`, `%2sk-ant-…`, `\u002sk-ant-…`, `%ZZsk-ant-…`, `src/sk-live.rs`. `SK_CANDIDATE`
  captures the body after an escaped start (`x\nsk-Ab3_…` → `Ab3_xY9-kLmN0pQrStUvWx`).
- `str::floor_char_boundary` is stable on 1.98 (used by D18).

---

## Amendments (plan ≠ tree, or plan underspecified)

| # | Plan says | Tree / analysis says | Resolution |
|---|---|---|---|
| A-1 | D11: "Resolved lazily, at the first `drive_once` of the walk, before `driver.start`." T4 test: "a trim-record string equal to a resolved value is stored `[REDACTED]` (MOD-61, engine level)." | Every live path persists scrubbed content **before** `drive_once`: stage 3 assembles the prompt with `parts.scrubber` (`engine.rs:5495`, `:4969-4970`), writes `trim_record` through `to_value(self.parts.scrubber)` + `set_step_prompt` (`:3566`, `:4232`, `:5099`), and `open_recorder` (`:5952`) writes the seq-0 prompt row (called at `:5910`, `:4246`, `:5112`), all before `drive_once` (`:5920`, `:4252`, `:5160`). Resolving at `drive_once` leaves `RunSecrets` pattern-only for all of that, so the plan's own MOD-61 trim-record test cannot pass on the first step of a walk | **Resolution is hoisted to the entry of each live path**, still lazy and still once per walk: `walk_live_step` (before `prepare`), `drive_group` (after its "no pending candidate" return, before `group_base`), `run_judge` (before its step 1). Each calls one boxed helper `Engine::secrets_ready(run)`. `drive_once` keeps D12's `env_for(&project)`, which reads the cached cell. D11's substance is unchanged: one object is env and scrubber, owned by the `Kit`, resolved once per walk before any `driver.start`, and a walk that reaches no live path (gate answer that only settles, park, sweep, `PromoteStep`, `AcceptArtifact`) never resolves |
| A-2 | D13: "The refusal is raised before `driver.start`, so it takes the path a `RefusedDriver` takes today" **and** "fails the run on every path, including the candidate and judge paths" | A `RefusedDriver` today fails the run on the plain path only. On the candidate path it fails **that candidate** (`run_candidate` → `fail_candidate`, `:4130-4141`) and a group retry (`Route::GroupFailed` + `may_attempt`, `:4426-4440`) or `NoSurvivingCandidate` follows; on the judge path it parks (`run_judge` `:4707` → `fail_judge`). The two sentences of D13 contradict each other on two of three paths | D13's run-failing intent wins. With A-1 the refusal is met at each path's entry and settled there: plain → `Err(EngineError::Secrets)` → `walk_step`'s existing `fail_hard` (the `RefusedDriver` path, as D13 says); candidates → the existing `fail_group_before_a_token(…, block = false)` (`:4059-4098`) with `RunFailure::SecretsRefused`, so no retry and no `NoSurvivingCandidate`; judge → new `refuse_judge_secrets` (§B.6). `drive_once`'s own `env_for` can then only answer the cached `Ok`; its error arm is defensive and `failure_text` still types it |
| A-3 | Risks: "`hold_back` is 0 when no secret is resolved and only the size trigger moves" | D18's formula `max(longest secret, PATTERN_HOLD_BACK) - 1` is never 0 for a `MinimalScrubber`: it is `PATTERN_HOLD_BACK - 1 = 75` with no secret at all. Every recorder over a `MinimalScrubber` (every graph run through `RunSecrets`, every chat) moves its 16 KiB seam. Two tests pin today's split `[16384, 1024]`: `crates/htui-agent/src/conformance.rs:3319-3355` `chunk_flush_at_16kib` (a conformance case run by every driver harness) and `crates/htui-agent/tests/recorder.rs:1830-1862` `chunks_flush_at_the_16_kib_bound`. The first is **not** in T2's file set | The formula stands (it is what protects a pattern key with no secret resolved, which D18's acceptance names). T2 rewrites both assertions in terms of `scrubber().hold_back()` (§D.2) and **T2's file set gains `crates/htui-agent/src/conformance.rs`**. A scrubber that keeps the default (`hold_back() == 0`) keeps today's cut exactly; a new test pins that. No other test builds a > 16 KiB run (`grep` of `repeat(` across `htui-agent`, `htui`, `htui-orch`, `htui-worker`); the merged-tree gate still runs the full insta suite (memory: snapshot impact) |
| A-4 | D18: "`PATTERN_HOLD_BACK` is a constant sized for the longest fixed-length rule" (sendgrid, 69 bytes) | What must stay open is the longest **not-yet-matching** prefix, which is the longest rule **minimum** less one, plus the token start in front of it. The longest minimum is `pypi_token` (20-byte prefix + 50 = 70), longer than sendgrid. After D17 the longest token start is `\uXXXX` (6 bytes). A cut between `\u00` and `22sk-ant-…` puts a `2` before `sk`, which is not a token start: neither row refuses and their concatenation holds the key | `pub const PATTERN_HOLD_BACK: usize = 76` (70 + 6), with the derivation in its doc and a test that checks it against every `one_short()` fixture (§D.1). `hold_back()` for an empty `MinimalScrubber` is 75 |
| A-5 | T3 file set: `htui-core/src/secret.rs` only; "extend the every-variant sentinel test" | `SecretError` is matched exhaustively, with no wildcard by design, in `crates/htui-secrets/tests/infisical.rs:1718-1736` (`variant`) and `crates/htui-secrets/tests/infisical_live.rs:41-58`. A new variant fails both to compile. `no_error_or_debug_carries_a_value_the_client_secret_or_the_token` (`infisical.rs:1838`) asserts the **provider produced** 15 distinct variants; the provider never produces `ReservedKey` | **T3's file set gains both htui-secrets test files.** Each `variant` gets `SecretError::ReservedKey { .. } => "ReservedKey"` with a comment ("raised by `htui_core::secret::check_env`, never by the provider"). The `== 15` count stays 15 |
| A-6 | "Tests set the columns through the fixtures and the Postgres testkit" | Engine tests run on `FakeOrchestrator::demo()` (`fake.rs:1668`), which builds `MemStore::demo()` with both columns `None` (`fixtures.rs:738`); no writer reaches the columns (`create_project` hard-codes `None`, `mem.rs:2480`; `ProjectPatch` has no field: M4 adds one). The worker tests can rebuild from `demo_data()`; the orch harness and the htui chat fixtures cannot | **T3 adds `MemStore::set_project_secret_columns`** beside the existing tests-only `set_project_settings` (`mem.rs:617-631`), same shape and the same "Tests only" doc. T3's file set gains `crates/htui-core/src/store/mem.rs` |
| A-7 | T6a (lane C) runs "after T3; parallel with T4–T5", and chats use "D12's column rules plus resolve, `check_env` … a refusal … with the same sentence" | The plan puts the resolution sequence in `RunSecrets` (T4) and the sentence in `RunFailure::SecretsRefused` (T4). Lane C would have to wait for T4, or duplicate both | **T3 hosts the shared pieces in `htui-core::secret`**: `resolve_project` (column rules → source → provider kind → `resolve` → `check_env`) and `SecretError::refusal()` (the `secrets_refused: …` sentence). `RunSecrets` (T4), `RunFailure::SecretsRefused`'s `Display` (T4), `EngineError::Secrets`'s `Display` (T4) and the chat (T6a) all call them. Lane C depends on T3 only, as the plan wants |
| A-8 | T6a creates `crates/htui/src/secrets.rs`; T6b owns `crates/htui/src/lib.rs` | A new module does not compile without its `mod` line in `lib.rs` | **T6a adds the one line `pub(crate) mod secrets;`** to `crates/htui/src/lib.rs`. T6b (strictly after T6a) edits the same file's wiring. No parallel conflict: `lib.rs` is in no other lane |
| A-9 | D19: "Update `verify.rs:15`'s comment" | `:14-15` (module doc) says the shell runs "with the process environment unchanged", which stays true. The comment that made this conditional on an empty `SessionSpec.env` is `verify.rs:323-325` in `spawn_and_wait` | T4 rewrites `:323-325` (§B.8). The module doc gains one clause ("which never holds a resolved secret: MOD-10 D19") |
| A-10 | T4: "update the 10 `EngineParts` constructors" | Confirmed 10 (`engine.rs` ×7 incl. `fake_parts` `:7102`; `conformance.rs:8593`; `tests/gix_isolator.rs:92`; `htui-worker/src/runtime.rs:1037`). But `fake_parts(orch, graphs, driver, scrubber: &dyn Scrubber)` (`:7076`) has **22 callers** (21 in `engine.rs`, 1 in `conformance.rs`), every one passing `MinimalScrubber::new([])` | `fake_parts`'s fourth parameter becomes `secrets: &'a RunSecrets` and fills **both** `scrubber` and `secrets` from it. Each caller's `let scrubber = MinimalScrubber::new([]);` becomes `let scrubber = RunSecrets::new(orch.secret_source());` (name kept, so the call line does not change). The 6 inline test literals in `engine.rs` and the two outside it add the field: fresh ones with a local `let secrets = RunSecrets::none();` (a temporary would not outlive the engine), copies with `secrets: parts.secrets` |
| A-11 | D19: "No change to `ShellVerifier` or its pattern-only scrubber" | `Engine::verify` persists `report.output` into `command_run.output` (`engine.rs:3750-3772`) exactly as the verifier's **pattern-only** scrubber left it. D19 covers the environment; it does not cover a verify command that prints a **file** the agent wrote with a resolved value in it (`cat .env`, a test log). That output is persisted unmasked: MOD-61's "exact-match masking is inert on the run path" stays true for one column | **Flag for the maintainer; included by default in T4**: `Engine::verify` re-masks `report.output` with `self.parts.scrubber` (the walk's `RunSecrets`) before `record_command_run` and before returning the report (§B.6). `ShellVerifier` and its scrubber are untouched, so D19 holds as written. Six lines, one test (§D.4). If the maintainer declines, drop that step and its test; nothing else depends on it |
| A-12 | D13: "Chats are refused before `start_chat_run` / before the promoted session starts" (T6a puts resolution on both chat paths) | `AgentRuntime::serve` is **awaited inline on the store loop's `select!` arm** (`store_worker.rs:2665`), and so are `start` (`agent_worker.rs:2036`) and `bind_promoted` (`:962`). The codebase keeps every network call off that arm (`R-NF-3`; e.g. `install_plan`'s doc `:1683-1685`, "the HEAD and the HTTP client happen in the task"). An Infisical resolution there (keyring read + login + list, 5 s connect / 20 s request timeouts, `htui-secrets/src/infisical.rs:23-28`) freezes every TUI store request for that long | **The chat resolves in its own task** (`run_chat`'s prelude, §B.10), which is spawned in production and awaited inline by the test harness (`:4236-4238`). On a provider project `start` defers `start_chat_run` into that prelude, after the resolution, so a refusal still leaves **no run row** (D13). A provider-less chat keeps today's sequence byte for byte (`start_chat_run` inline). The column checks (`project_scope`) stay inline: they do no I/O |

**12 amendments.** A-1, A-2 and A-12 change *where* the plan's decisions are realised, not what
they decide. A-11 is the only behaviour beyond the plan and is flagged.

---

## A. Per-file change table

| # | File | Action | Task | What changes (and what must **not**) |
|---|---|---|---|---|
| 1 | `crates/htui-core/src/scrub.rs` | UPDATE | T1 | `TOKEN_START` widened (D17); `from_resolved` adds the trailing-newline-trimmed form (D17); `impl Drop for MinimalScrubber` zeroizes (D17); `new`'s `dedup` zeroizes what it drops; `pub const PATTERN_HOLD_BACK`; `Scrubber::hold_back` defaulted; `MinimalScrubber`'s `hold_back`. **Not**: `PATTERN_RULES`, rule names, `mask`, `find_residue`, `Unmasked`, `residue_rule`'s order |
| 2 | `scripts/scrub-audit.sql` | UPDATE | T1 | Both token-start literals (`:78`, `:81`) mirror the new `TOKEN_START`; header comment `:10-11` updated |
| 3 | `crates/htui-agent/src/record.rs` | UPDATE | T2 | D18 cut-and-carry: `ChunkMark`, `Recorder.open_chunks`, `flush_at_seam`, free fns `seam_cut`, `scrubbed_text`, `split_marks`; the size trigger in `record` (`:1108`) calls `flush_at_seam`. **Not**: triggers 1, 2, 3, 5, `flush`'s body (except clearing `open_chunks`), `withhold_split_raw`, `release_held` |
| 4 | `crates/htui-agent/tests/recorder.rs` | UPDATE | T2 | Seam tests (§D.2); `chunks_flush_at_the_16_kib_bound` re-expressed (A-3) |
| 5 | `crates/htui-agent/src/conformance.rs` | UPDATE | T2 (A-3) | `chunk_flush_at_16kib` expects `[CHUNK_FLUSH_BYTES - hold, 1024 + hold]` |
| 6 | `crates/htui-core/src/secret.rs` | UPDATE | T3 | `INFISICAL`, `RESERVED_PREFIX`, `SecretSource`, `project_scope`, `check_env`, `resolve_project`, `SecretError::ReservedKey`, `SecretError::refusal`, `mod fake` (test-support); sentinel test → 16 variants |
| 7 | `crates/htui-core/src/store/mem.rs` | UPDATE | T3 (A-6) | `set_project_secret_columns` beside `set_project_settings` |
| 8 | `crates/htui-secrets/tests/infisical.rs`, `tests/infisical_live.rs` | UPDATE | T3 (A-5) | One `ReservedKey` arm in each `variant` |
| 9 | `crates/htui-orch/src/secrets.rs` | CREATE | T4 | `RunSecrets` (§B.5) and its unit tests |
| 10 | `crates/htui-orch/src/lib.rs` | UPDATE | T4 | `pub mod secrets;` and `pub use secrets::RunSecrets;` |
| 11 | `crates/htui-orch/src/command.rs` | UPDATE | T4 | `EngineError::Secrets(SecretError)` |
| 12 | `crates/htui-orch/src/status.rs` | UPDATE | T4 | `RunFailure::SecretsRefused { cause }`, `Display` arm, one row in `run_failure_display_is_ana2s_bytes` |
| 13 | `crates/htui-orch/src/engine.rs` | UPDATE | T4 | `EngineParts.secrets` + `Debug`; `secrets_ready`; the three entries (A-1); `drive_once` env (D12); `refuse_judge_secrets`; `failure_text` arm; `verify` re-mask (A-11); `fake_parts` + 4 `*_fake` fns + 6 inline test literals (A-10); tests |
| 14 | `crates/htui-orch/src/fake.rs` | UPDATE | T4 | `FakeOrchestrator.secret_source` + `set_secret_source` / `secret_source`; carried by `restarted()` |
| 15 | `crates/htui-orch/src/conformance.rs` | UPDATE | T4 | `EngineParts` literal at `:8593` gains `secrets: parts.secrets`; its `fake_parts` caller (A-10); fan-out and judge tests in `mod fanout_paths` (**never** `CASES`, H-1) |
| 16 | `crates/htui-orch/src/verify.rs` | UPDATE | T4 (A-9) | Comment `:323-325`, one clause in the module doc. No code |
| 17 | `crates/htui-orch/tests/gix_isolator.rs` | UPDATE | T4 | Macro `engine_as!` gains `let secrets = RunSecrets::none();` and `secrets: &secrets` |
| 18 | `crates/htui-worker/src/runtime.rs` | UPDATE | T5 | `Shared.secrets`, `RunRuntime::with_secret_source`, `Kit.secrets` replaces `Kit.scrubber`, `Kit::engine`; tests (MOD-61, MOD-62, R-SEC-2, refusal) |
| 19 | `crates/htui/Cargo.toml` | UPDATE | T6a | `htui-secrets = { workspace = true }` under `[dependencies]` |
| 20 | `Cargo.lock` | UPDATE | T6a | `htui`'s dependency list gains `"htui-secrets"`. No package moves. **Never `cargo update`** |
| 21 | `crates/htui/src/secrets.rs` | CREATE | T6a | `KeyringInfisical` (§B.9) and its tests |
| 22 | `crates/htui/src/lib.rs` | UPDATE | T6a (A-8), T6b | T6a: `pub(crate) mod secrets;`. T6b: build one source, pass it to `spawn_hosted` |
| 23 | `crates/htui/src/agent_worker.rs` | UPDATE | T6a | `AgentRuntime.secrets` + `with_secret_source`; `start` / `bind_promoted` inline column check; `ChatArgs.secrets`; `run_chat` prelude (A-12); scrubber via `from_resolved` (`:4139`); tests |
| 24 | `crates/htui/src/store_worker.rs` | UPDATE | T6b | `spawn_hosted` gains `secrets: Option<Arc<dyn SecretSource>>` and hands it to both runtimes; its test caller (`:5513`) passes `None` |
| 25 | `crates/htui/src/worker_cmd.rs` | UPDATE | T6b | `.with_secret_source(Arc::new(KeyringInfisical::new()))` on the worker's `RunRuntime` (`:105-107`) |
| 26 | `docs/htui-secrets.md` | UPDATE | T7 | New `## At run start` section after `## What htui refuses` (`:126`) |

No migration, no SQL beyond the audit script, no `.sqlx` change.

---

## B. Interfaces, exactly

### B.1 T1 — `htui-core::scrub` (D17, D18)

**Token start (D17, M1 review L1).** One constant, used by `PATTERNS`, `OPENAI_STRICT` and
`SK_CANDIDATE` exactly as today (all three interpolate it as a `format!` *argument*, so its braces
are not format-parsed):

```rust
/// A token start (MOD-10 D1, D17): the string start, one character that is not
/// `[A-Za-z0-9_]`, or a JSON / percent escape whose last character is a letter or digit: `\n`,
/// `\r`, `\t`, `\b`, `\f`, `\"`, `\/`, `\\`, `\uXXXX` and `%XX`. A key serialised inside an
/// escaped string (`…\nsk-ant-…`, `%22ghp_…`) is therefore a whole token, while `subtask-…` and
/// `x%2Fsk-learn` stay prose (the latter is too short for any rule). It widens what fails closed;
/// `scripts/scrub-audit.sql` mirrors it (OQ-D: the host audit runs before merge).
const TOKEN_START: &str = r#"(?:^|[^A-Za-z0-9_]|\\[nrtbf"/\\]|\\u[0-9A-Fa-f]{4}|%[0-9A-Fa-f]{2})"#;
```

- The `\"`, `\/` and `\\` alternatives are redundant with `[^A-Za-z0-9_]` (the backslash is
  already a non-word character before them) and are kept so the constant reads as the escape
  list D17 names.
- `(?-u:\b)` is **not** adopted (D17); no timing assert is added.
- `every_pattern_rule_compiles_alone` already compiles each rule with `TOKEN_START` and keeps
  doing so.

**Audit mirror** (`scripts/scrub-audit.sql`, plain `'…'` literals, standard conforming strings):

```sql
-- :78
        ON s.s ~ ('(^|[^A-Za-z0-9_]|\\[nrtbf"/\\]|\\u[0-9A-Fa-f]{4}|%[0-9A-Fa-f]{2})' || r.re)
-- :81
                 FROM regexp_matches(s.s,
                      '(?:^|[^A-Za-z0-9_]|\\[nrtbf"/\\]|\\u[0-9A-Fa-f]{4}|%[0-9A-Fa-f]{2})sk-([A-Za-z0-9_-]{20,})',
                      'g') AS m
```

In a Postgres ARE `\\` is a literal backslash inside and outside brackets, and `\\u` is a
backslash then `u` (not the ARE `\uXXXX` escape). The `:81` start group stays non-capturing so
`m[1]` is still the body. The header's `:10-11` sentence becomes: "JSON escapes such as `\n` are
token starts (MOD-10 D17), and matching the decoded string values (not `payload::text`) keeps one
level of escaping out of the haystack." **VERIFY — host only** (OQ-D): the maintainer runs the
script before `scripts/hr collect MOD-10`; the sandbox has no `psql` (memory).

**Trailing newline (D17).** `from_resolved` pushes, for each value at or above the floor, the
value itself and, when `value.trim_end_matches(['\r', '\n'])` differs and is still at least
`MIN_MASKED_LEN` characters, that trimmed form. Injection is untouched (the caller injects
`resolved` byte for byte). `short` is unchanged: a key is short only when its **full** value is
below the floor. Longest-first ordering in `new` already masks `abc…\n` before `abc…`.

**Zeroize (D17, OQ-C).**

```rust
impl Drop for MinimalScrubber {
    /// MOD-10 D17: the secret list is wiped when the scrubber goes (a walk's, a chat's).
    fn drop(&mut self) {
        for secret in &mut self.secrets {
            secret.zeroize();
        }
    }
}
```

`new`'s `secrets.dedup()` becomes
`secrets.dedup_by(|dropped, kept| if dropped == kept { dropped.zeroize(); true } else { false })`
(`dedup_by` hands the candidate for removal first). `use zeroize::Zeroize;` (already a
dependency, `htui-core/Cargo.toml`). Empty strings filtered by `new` hold nothing. There is no
test that reads freed memory (`unsafe_code = forbid`); the reviewer checks the impl.

**`hold_back` (D18).**

```rust
/// The longest not-yet-matching prefix any pattern rule can leave at the end of a text, plus the
/// longest token start in front of it (MOD-10 D18, blueprint A-4): `pypi_token`'s minimum match
/// is 70 bytes (`pypi-AgEIcHlwaS5vcmc` + 50) and `\uXXXX` is 6. A text cut at least
/// `PATTERN_HOLD_BACK - 1` bytes before its end therefore never splits a credential the rules
/// would have caught whole. A rule whose minimum grows must grow this.
pub const PATTERN_HOLD_BACK: usize = 76;

pub trait Scrubber: Send + Sync + core::fmt::Debug {
    fn scrub(&self, value: &mut Value) -> Result<(), Unmasked>;

    /// MOD-10 D18: how many trailing bytes of an open text run a size-triggered cut must keep
    /// open, so that a secret or a credential still arriving is never split across two rows.
    /// `0` (the default) keeps the recorder's cut at the bound exactly, as before MOD-10 M3.
    fn hold_back(&self) -> usize {
        0
    }
}

impl Scrubber for MinimalScrubber {
    fn scrub(&self, value: &mut Value) -> Result<(), Unmasked> { /* unchanged */ }

    /// `max(longest secret in bytes, PATTERN_HOLD_BACK) - 1`: 75 with no secret.
    fn hold_back(&self) -> usize {
        self.secrets
            .first() // sorted longest first by byte length (`new`)
            .map_or(0, String::len)
            .max(PATTERN_HOLD_BACK)
            - 1
    }
}
```

Every other `Scrubber` in the tree (`htui-agent/tests/recorder.rs:1239` `MaskKey`,
`htui-core/src/prompt/mod.rs:1496`, `htui-core/tests/prompt_hostname.rs:183`,
`htui-mcp/tests/tools_command.rs:202`, `:547`, `htui-mcp/tests/tools_backlog.rs:221`) keeps the
default and compiles unchanged.

### B.2 T2 — the flush seam (`htui-agent::record`, D18)

**State.** One recorder field and one private type; `PendingRow` is **not** changed (it has
literal constructions in several functions):

```rust
/// MOD-10 D18: one chunk of the open text run: where its text ends in the run, when it was
/// captured, and whether it pushed a `raw` entry.
#[derive(Debug, Clone, Copy)]
struct ChunkMark {
    end: usize,
    at: DateTime<Utc>,
    raw: bool,
}

// in `Recorder`:
    /// MOD-10 D18: the open text run's chunks, in order; empty unless `buffer_kind` is a chunk
    /// kind. Cleared by every `flush`.
    open_chunks: Vec<ChunkMark>,
```

- `new` initialises it to `Vec::new()`; `continuing` inherits it through `..Self::new`.
- `record`'s chunk arm, after the `push` (new run) or the `push_str` (open run), pushes
  `ChunkMark { end: <open text len after the append>, at: scrubbed.at, raw: false }`.
- The `raw` attachment block, for `RawTarget::Buffered` **when `chunk.is_some()`**, also sets
  `self.open_chunks.last_mut().raw = true`.
- `flush` clears it, next to `self.buffer_kind = None; self.open_message_id = None;`.
- The `Debug` impl is hand-written and may stay as it is (a private count is not owed).

**Trigger.** At `:1108`, `if turn_ended || reached_bound { flush; sync_step }` becomes:

```rust
if turn_ended {
    self.flush().await?;
    self.sync_step().await?;
} else if reached_bound {
    // MOD-10 D18: trigger 4 cuts at a seam and carries the tail; triggers 1, 2, 3 and 5 still
    // flush everything (a new message, a new kind, `done`, `finish`).
    self.flush_at_seam().await?;
    self.sync_step().await?;
}
```

`turn_ended` and `reached_bound` cannot both hold: `reached_bound` is set only in the chunk arm
and `turn_ended` only for `Done`.

**The cut.**

```rust
/// MOD-10 D18: how many earlier seams one cut may try before it gives up and flushes whole.
const SEAM_ATTEMPTS: usize = 4;

/// MOD-10 D18: where a size-triggered flush may cut `text`, keeping at least `hold_back` bytes
/// open. `None` is "no safe cut": the caller flushes the whole run (`hold_back == 0`, or a
/// residue anywhere in `text`, or no attempt found a seam).
///
/// A cut `c` is safe when scrubbing `text[..c]` and `text[c..]` apart gives, concatenated,
/// exactly what scrubbing `text` whole gives, and neither half is refused. That is the leak
/// criterion itself: every complete secret and every complete credential is masked or refused
/// identically whichever row it lands in. The first candidate is the last char boundary at least
/// `hold_back` bytes before the end; an unsafe candidate moves back by `hold_back` more bytes.
/// Because an occurrence of a secret is at most `hold_back + 1` bytes long, one step back always
/// clears the occurrence that made the previous candidate unsafe; the loop re-checks in case it
/// lands inside another.
fn seam_cut(scrubber: &dyn Scrubber, text: &str, hold_back: usize) -> Option<usize> {
    if hold_back == 0 || text.len() <= hold_back {
        return None;
    }
    let whole = scrubbed_text(scrubber, text)?;
    let mut cut = text.floor_char_boundary(text.len() - hold_back);
    for _ in 0..SEAM_ATTEMPTS {
        if cut == 0 {
            return None;
        }
        let (head, tail) = text.split_at(cut);
        if let (Some(head), Some(tail)) = (scrubbed_text(scrubber, head), scrubbed_text(scrubber, tail))
            && whole.len() == head.len() + tail.len()
            && whole.starts_with(&head)
            && whole.ends_with(&tail)
        {
            return Some(cut);
        }
        cut = text.floor_char_boundary(cut.saturating_sub(hold_back));
    }
    None
}

/// `text` as `scrubber` masks it, or `None` when a rule refuses it.
fn scrubbed_text(scrubber: &dyn Scrubber, text: &str) -> Option<String> {
    let mut value = Value::String(text.to_owned());
    scrubber.scrub(&mut value).ok()?;
    match value {
        Value::String(text) => Some(text),
        _ => None,
    }
}
```

Why each piece:
- **A cut inside an already-complete match moves back** (the plan leaves this to the
  architect). The recorder holds a `&dyn Scrubber` and cannot see match positions, and D18 allows
  one defaulted method only. The equality probe needs neither: a complete secret straddling `c`
  is masked in `whole` and not in either half, so the strings differ; a complete credential
  straddling `c` makes `whole` refuse, which ends the search before the loop (below). A cut that
  *creates* a refusal (a tail starting with `AKIA…` after a letter, so `^` becomes a token start;
  a head ending in `sk-learn-preprocessing-`, which `SK_PROSE` reads as non-prose) is also
  unsafe and moves back, so the seam never invents a failure the whole text did not have.
- **Char boundaries**: both `floor_char_boundary` calls land on a boundary at or before the
  byte offset, so `split_at` never panics and a multi-byte character is never split.
- **`whole` refused (`None`)**: a complete credential is already in the run. The cut cannot hide
  it, and carrying would only move it, so the run is flushed whole exactly as today: `flush`'s
  re-scrub turns it into one `scrub_residue` row (`flush`, `record.rs:1180-1240`) and `finish` answers
  `Unmasked`. A key whose matching prefix (≥ its rule's minimum) is in this run and whose tail is
  still arriving leaves that tail, without its prefix, in the next row. That is today's behaviour,
  the session already fails closed, and the tail alone is not the key (residual, documented in T7).
- **`text.len() <= hold_back`**: only when a resolved secret is longer than the run (a secret
  over 16 KiB). The run is **kept open** (no flush), so the row grows past the bound until it is
  longer than the longest secret plus one chunk; never a split secret.
- **No safe cut after `SEAM_ATTEMPTS`**: flush whole, as today, with
  `tracing::warn!(step = %self.step, bytes = text.len(), hold_back, "no safe seam was found; the text run is flushed whole")`
  (counts only). Reaching it needs four overlapping secret occurrences at the seam.
- **Cost**: three scrubs of at most 16 KiB + one chunk per seam (whole, head, tail), once per
  16 KiB of streamed text, versus one today. `MinimalScrubber::mask` is linear per secret.

**`flush_at_seam`.**

```rust
/// MOD-10 D18: trigger 4. Cuts the open text run at [`seam_cut`], flushes the head as one row and
/// re-opens the tail as the next run, with the same kind and grouping key.
async fn flush_at_seam(&mut self) -> Result<(), RecordError> {
    let hold_back = self.scrubber.hold_back();
    let Some(text) = self.open_text() else { return self.flush().await };
    match seam_cut(self.scrubber, text, hold_back) {
        Some(cut) => { /* split, below */ }
        None if hold_back > 0 && text.len() <= hold_back => return Ok(()),
        None => return self.flush().await, // + the warn when hold_back > 0 and whole was clean
    }
    ...
}
```

The split, in order:
1. `let message_id = self.open_message_id.clone(); let marks = core::mem::take(&mut self.open_chunks);`
2. Pop the open row (`self.buffer.pop()`; the buffer holds exactly one row while a chunk kind is
   open, because a chunk only `push`es when `buffer_kind` is `None`). Take its text, set the head
   row's `payload.text` to `text[..cut]`.
3. `split_marks(&marks, cut)` gives, for each chunk `i` spanning `[start_i, end_i)`
   (`start_0 = 0`, `start_i = end_{i-1}`):
   - **head raw**: the raw entries of chunks with `start_i < cut` (they are a prefix of
     `row.raw`, in order, counting only marks with `raw: true`);
   - **carry raw**: clones of the raw entries of chunks with `end_i > cut`. A chunk that
     **straddles** the cut (`start_i < cut < end_i`) is in **both**; a chunk wholly after the cut
     moves to the carry only;
   - **carry `at`**: the `at` of the first chunk with `end_i > cut`;
   - **carry marks**: those chunks, `end - cut`, same `at` and `raw`.
4. `let carry = PendingRow { kind, role: EventRole::Agent, tool_call_id: None, payload: json!({ "text": tail }), raw: carry_raw, at: carry_at };`
5. Push the head back, `let flushed = self.flush().await;` (this numbers the head with the next
   `seq`, runs `withhold_split_raw` on it and clears the open state).
6. **Re-open before propagating**: `self.push(carry); self.open_message_id = message_id;
   self.open_chunks = carry_marks; flushed`. A store error still leaves the carried text open, so
   `finish` can write it.

**Interactions.**
- **Trigger 1 / 2** (another kind, another `message_id`) and **3** (`Done`) and **5** (`finish`)
  call `flush`, which writes the carried tail whole. A secret split by a *message id change* is
  two messages and stays out of scope, as before.
- **`raw` (MOD-10 D8)**: the straddling chunk rides both rows, so each row's per-pointer join
  (`withhold_split_raw`) sees every byte of its own text. A secret that starts in the straddling
  chunk and ends in a later one is whole in the **carry's** join, which withholds the carry's raw
  (`raw_withheld += 1`); the head's raw holds at most its prefix. A secret wholly inside one chunk
  was masked at capture (`scrub_envelope`). No row's raw and no concatenation of two rows' raw
  holds the secret. Raw-only leaves at other pointers are not text and are not moved by the
  seam; D8's per-row check is unchanged.
- **Replay determinism**: the cut is a pure function of the chunk texts, their `at`s, their raw
  flags and the scrubber (its secrets fix `hold_back`), with no clock and no timer. The carried
  row's `at` is a captured chunk's own `at`. `replay.rs:30-43` ("a text run the bound split into
  two rows") stays true.
- **UI**: `send_ui` is untouched; the tab never sees row boundaries.
- **Held `edit_proposal` rows**: untouched (never a chunk kind).

### B.3 T3 — `htui-core::secret` (D15, D16; A-5, A-7)

```rust
use std::sync::Arc;
use crate::model::Project;

/// `project.secret_provider`'s value for Infisical (the only provider of this build). Equal to
/// `htui_secrets::InfisicalProvider::KIND`, which `htui-core` cannot name.
pub const INFISICAL: &str = "infisical";

/// MOD-10 D16: a resolved key with this prefix (case-sensitive) is refused: htui's own variables
/// (`HTUI_MCP_*`, `HTUI_LOG*`, `HTUI_TOOL_*`) must never be shadowed by `SessionSpec.env`, which
/// is applied last.
pub const RESERVED_PREFIX: &str = "HTUI_";

/// MOD-10 D15: where a process gets its one provider per identity. Production is `htui`'s
/// keyring-backed Infisical source; tests use [`fake::FakeSecretSource`].
pub trait SecretSource: Send + Sync + core::fmt::Debug {
    /// The provider for the identity stored now. Called once per walk and per chat; an
    /// implementation returns the **same** `Arc` while nothing it reads has changed, so the
    /// provider's login latch and cool-down persist across walks (M2 D5).
    fn provider(&self) -> SecretFuture<'_, Arc<dyn SecretProvider>>;
}

/// MOD-10 D12: the project's scope, or `None` when `secret_provider` is unset (the scope column is
/// then ignored: the provider column is the switch).
///
/// # Errors
/// [`SecretError::Config`]: an unknown provider (named with `{:?}`, so a control character is
/// escaped), a provider with no scope, or a scope [`SecretScope::parse`] refuses.
pub fn project_scope(project: &Project) -> Result<Option<SecretScope>, SecretError>;

/// MOD-10 D16: refuses the first key (map order) that starts with [`RESERVED_PREFIX`].
///
/// # Errors
/// [`SecretError::ReservedKey`].
pub fn check_env(resolved: &ResolvedSecrets) -> Result<(), SecretError>;

/// MOD-10 D12, D16 (blueprint A-7): the one resolution sequence the walk (`RunSecrets`) and the
/// chat share. `Ok(None)` for a provider-less project, **without touching `source`**. Otherwise,
/// in this order: [`project_scope`] (a column error never reads the keyring); a missing `source`
/// is `Config`; `source.provider()`; a provider whose `kind()` is not the column's is `Config`;
/// `provider.resolve(&scope)`; [`check_env`] (a refused map is dropped, so zeroized).
pub fn resolve_project<'a>(
    source: Option<&'a dyn SecretSource>,
    project: &'a Project,
) -> SecretFuture<'a, Option<ResolvedSecrets>>;
```

The exact `Config` sentences (each a `const` beside the function, so tests compare them):

| Case | Sentence (after `secret provider configuration: `) |
|---|---|
| unknown provider | `` project.secret_provider {provider:?} is not a provider this build knows (expected "infisical") `` |
| provider, no scope | `the project names a secret provider but has no secret_scope` |
| no source | `this process has no secret source, so the project's secrets cannot be resolved` |
| kind mismatch | ``the secret source answered a `{kind}` provider for a `{column}` project`` |

New variant and helper:

```rust
    /// A resolved key carries htui's reserved prefix (MOD-10 D16, OQ-B). Raised by
    /// [`check_env`], never by a provider. Key names are not secret; a provider has already
    /// refused a key that is not a valid environment name (`InvalidKey`).
    #[error("the secret name `{key}` is reserved for htui; rename it in Infisical")]
    ReservedKey {
        /// The key.
        key: String,
    },

impl SecretError {
    /// MOD-10 D13: the one refusal sentence a run (`RunFailure::SecretsRefused`), an engine error
    /// (`EngineError::Secrets`) and a chat print: `secrets_refused: <Display>`. Names the cause,
    /// never a value (M2 D4).
    #[must_use]
    pub fn refusal(&self) -> String {
        format!("secrets_refused: {self}")
    }
}
```

**Fakes** (`#[cfg(any(test, feature = "test-support"))] pub mod fake` at the end of `secret.rs`;
`htui-orch`, `htui-worker` and `htui` already enable `htui-core/test-support` for their tests):

```rust
/// A provider answering a script, one answer per `resolve`; the last answer repeats once the
/// script is spent; an empty script answers `Protocol { endpoint: "fake", detail: "no scripted answer" }`.
/// `list_keys` is `resolve`'s keys; `health` is `Ok` (`base_url: "fake://"`, `server_ok: true`).
/// `Debug` prints the kind and the counters, never a value.
pub struct FakeSecretProvider { /* kind, Mutex<VecDeque<Result<BTreeMap<..>, SecretError>>>, last, AtomicUsize resolves, Mutex<Vec<SecretScope>> scopes */ }
impl FakeSecretProvider {
    pub fn new(script: impl IntoIterator<Item = Result<BTreeMap<String, String>, SecretError>>) -> Self;
    pub fn resolving(pairs: &[(&str, &str)]) -> Self;   // one Ok answer
    pub fn failing(error: SecretError) -> Self;         // one Err answer
    #[must_use] pub fn with_kind(self, kind: &'static str) -> Self; // default INFISICAL
    pub fn resolves(&self) -> usize;
    pub fn scopes(&self) -> Vec<SecretScope>;
}

/// A source answering one fixed provider (or one fixed error) and counting its calls.
pub struct FakeSecretSource { /* Result<Arc<dyn SecretProvider>, SecretError>, AtomicUsize calls */ }
impl FakeSecretSource {
    pub fn new(provider: Arc<dyn SecretProvider>) -> Self;
    pub fn failing(error: SecretError) -> Self;
    pub fn calls(&self) -> usize;
}
```

Both are `Send + Sync` (`std::sync::Mutex`, atomics) and need no tokio, keyring or HTTP.

### B.4 T3 — `MemStore` setter (A-6)

```rust
/// Plants `project.secret_provider` and `project.secret_scope` without validation. **Tests
/// only**, like [`set_project_settings`](Self::set_project_settings): M4 owns the writer
/// (MOD-10 M3 blueprint A-6). A project that is not there is left alone.
pub fn set_project_secret_columns(&self, project: ProjectId, provider: Option<&str>, scope: Option<&str>);
```

Same body shape as `set_project_settings` (`mem.rs:623-631`), stamping `updated_at`.

### B.5 T4 — `htui_orch::secrets::RunSecrets` (D11, D12, D14)

```rust
//! MOD-10 D11: one walk's secrets: the environment its sessions receive and the scrubber that
//! guards every byte it persists, as **one object**, so the two cannot drift.

/// One walk's secrets (MOD-10 D11). Owned by `htui-worker`'s `Kit` (one per task) and lent to
/// the engine as both `EngineParts.scrubber` and `EngineParts.secrets`.
///
/// Resolves at most once (`tokio::sync::OnceCell`), for the first project it is asked about, on
/// the first live path of the walk (blueprint A-1). Until then, after a refusal, and for a
/// provider-less project it scrubs with the pattern rules only, exactly as the empty
/// `MinimalScrubber` it replaces. Dropping it zeroizes the resolved map and the scrubber's list.
pub struct RunSecrets {
    source: Option<Arc<dyn SecretSource>>,
    /// The pattern-only scrubber: `MinimalScrubber::new([])`.
    patterns: MinimalScrubber,
    /// Boxed so the struct stays small in every future that holds it (H-1).
    slot: tokio::sync::OnceCell<Box<Bound>>,
}

/// What the slot holds: the project it was resolved for (D12's single-project guard) and the
/// outcome, a refusal included (D12: "the failure is cached").
struct Bound {
    project: ProjectId,
    outcome: Result<Resolved, SecretError>,
}

struct Resolved {
    secrets: ResolvedSecrets,   // zeroized on drop (M2)
    scrubber: MinimalScrubber,  // `from_resolved(secrets.as_map())`; zeroized on drop (T1)
}

impl RunSecrets {
    /// A walk's secrets over `source` (`None`: a process without one; a provider project is then
    /// refused with `Config`).
    #[must_use]
    pub fn new(source: Option<Arc<dyn SecretSource>>) -> Self;

    /// `new(None)`: for engines that resolve nothing (tests, tools).
    #[must_use]
    pub fn none() -> Self;

    /// MOD-10 D12: makes this walk's secrets ready for `project`.
    ///
    /// 1. The slot is already bound: to another project → `Config(FOREIGN_PROJECT)` (fail
    ///    closed, provider-less or not); to this one → its cached outcome, no second call.
    /// 2. `project.secret_provider` is `None` → `Ok(())`, the slot stays unbound, the source is
    ///    never called.
    /// 3. Otherwise `slot.get_or_init(...)` runs [`htui_core::secret::resolve_project`] once for
    ///    every concurrent caller; the bound project is compared again after it (two callers
    ///    racing with two projects); the outcome is answered (a refusal cloned).
    ///
    /// On success the short keys of `from_resolved` are logged once at `warn` by **name**
    /// (`project`, `keys`), never by value (D17).
    pub fn prepare<'a>(&'a self, project: &'a Project) -> SecretFuture<'a, ()>;

    /// MOD-10 D12: [`Self::prepare`], then the env a session of `project` receives: the
    /// resolved map cloned, or empty for a provider-less project. Nothing else ever goes in
    /// (R-SEC-2).
    pub fn env_for<'a>(&'a self, project: &'a Project)
        -> SecretFuture<'a, BTreeMap<String, String>>;

    /// The scrubber in force: the resolved one once bound `Ok`, else `patterns`.
    fn active(&self) -> &MinimalScrubber;
}

/// `FOREIGN_PROJECT`: "this walk's secrets were resolved for another project".

impl Scrubber for RunSecrets {
    fn scrub(&self, value: &mut Value) -> Result<(), Unmasked> {
        self.active().scrub(value)
    }
    /// D18: delegates, so a long resolved secret widens the seam (H-20).
    fn hold_back(&self) -> usize {
        self.active().hold_back()
    }
}

impl core::fmt::Debug for RunSecrets {
    /// `RunSecrets { source: true, state: "unresolved" | "resolved" | "refused", keys: 3 }`: no
    /// value and no key name.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result;
}
```

The once-cell choices:
- **`tokio::sync::OnceCell::get_or_init`** (not `get_or_try_init`): the init future stores the
  `Result`, so a refusal is cached and every later session of the walk (fan-out candidates, the
  judge, the next step) meets it without a second login (D12, M2 D5's latch). `htui-orch`
  already depends on `tokio` with `sync`.
- A walk cancelled **while** the init future runs drops it; tokio's cell then stays empty and a
  later call starts again. The Infisical login itself runs on its own task (M2 D5), so nothing is
  half-recorded.
- `prepare` and `env_for` return `SecretFuture` (boxed), so a caller's state machine holds a
  pointer, not the init future (H-1).
- `Scrubber::scrub` and `hold_back` read the cell with `get()` (an atomic load), never await.

### B.6 T4 — engine (`engine.rs`, `command.rs`, `status.rs`)

**Parts.**

```rust
    /// MOD-10 D11: the walk's secrets. In production (`htui-worker`'s `Kit::engine`) the same
    /// object is `scrubber`, so the env a session gets and the mask its log is written through are
    /// one map. `RunSecrets::none()` resolves nothing.
    pub secrets: &'a RunSecrets,
```

`EngineParts`'s hand-written `Debug` adds `.field("secrets", &self.secrets)`.

**Errors and the sentence.**

```rust
// command.rs, beside `Record`
    /// MOD-10 D13: the walk's secrets could not be resolved for the run's project. Its `Display`
    /// is the run's `secrets_refused: …` sentence; it never carries a value.
    #[error("{}", .0.refusal())]
    Secrets(htui_core::secret::SecretError),

// status.rs
    /// MOD-10 D13: the run's secrets were refused before any agent started: no identity,
    /// unreachable, bad credentials, locked, cooling down, not found, permission, invalid or
    /// reserved key, or a configuration fault. Transient causes fail the run too (OQ-A).
    SecretsRefused {
        /// The provider's or the column check's error. Value-free by construction (M2 D4).
        cause: htui_core::secret::SecretError,
    },
// Display arm
            Self::SecretsRefused { cause } => f.write_str(&cause.refusal()),
```

`RunFailure` keeps `#[derive(Debug, Clone, PartialEq, Eq)]`; `SecretError` derives all four.

**`failure_text`** (`engine.rs:6618`) gains one arm, before `other`:

```rust
        EngineError::Secrets(cause) => RunFailure::SecretsRefused { cause: cause.clone() }.to_string(),
```

**The helper (A-1).**

```rust
    /// MOD-10 D11/D12 (blueprint A-1): this walk's secrets, ready for `run`'s project before a
    /// live path persists anything of its session (the trim record, the prompt row, the prompt's
    /// own masking). `Ok(Err(cause))` is a refusal the caller settles its own way; the outer
    /// error is the project read's. Call sites box it (H-1).
    async fn secrets_ready(&self, run: &Run) -> Result<Result<(), SecretError>, EngineError> {
        let project = self.project(run.project_id).await?;
        Ok(self.parts.secrets.prepare(&project).await)
    }
```

**The three entries.**

| Path | Where | Code | Outcome on refusal |
|---|---|---|---|
| Plain | `walk_live_step`, right after `let item = Self::item_of(run)?;` (`:3483`), **before** `prepare` | `Box::pin(self.secrets_ready(run)).await?.map_err(EngineError::Secrets)?;` | `Err(Secrets)` → `walk_step`'s catch-all (`:3463-3467`) → `fail_hard(run, &step, &failure_text(..))`: step `failed`, `run.failure` = the sentence, `cleanup_run`; the error is re-raised to the caller, as for `ScrubRefused`. No trees were prepared, no prompt row exists |
| Candidates | `drive_group`, right after `let Some(first) = pending.first() else { return Ok(None); };` (`:3908-3910`), **before** `group_base` | `if let Err(cause) = Box::pin(self.secrets_ready(run)).await? { return self.fail_group_before_a_token(run, phase, &pending, RunFailure::SecretsRefused { cause }, false).await.map(Some); }` | Every pending candidate `pending → running → failed` with the item note `fan-out candidate i of \`research\` attempt n: secrets_refused: …`; `finish_run(Failed, sentence)`; `cleanup_run`; `Rest { run: Failed, failure: Some(SecretsRefused) }`. `block = false`: the item mirrors `failed`, as on the plain path (not `blocked`). No retry: `select_stage` never runs |
| Judge | `run_judge`, first statement (before its `run_steps` read) | `if let Err(cause) = Box::pin(self.secrets_ready(run)).await? { return self.refuse_judge_secrets(run, phase, existing.as_ref(), cause).await.map(Some); }` | Below |

```rust
    /// MOD-10 D13 on the judge path: the run fails with the refusal, not the selection. A crash's
    /// `pending` judge row (`existing`) is moved `pending → running` and settled by `fail_hard`
    /// (`pending → failed` is illegal, `model/run.rs:113`); with no row, the run is failed and
    /// cleaned up directly, as `Route::GroupFailed`'s terminal arm does (`:4441-4452`).
    async fn refuse_judge_secrets(
        &self,
        run: &Run,
        phase: &SnapshotPhase,
        existing: Option<&RunStep>,
        cause: SecretError,
    ) -> Result<Rest, EngineError>;
```

It answers `Rest { run: RunStatus::Failed, position: Some(phase.position), failure: Some(RunFailure::SecretsRefused { cause }) }`.
A judge reached in the **same** walk as its candidates always finds the cell already `Ok` (the
candidates resolved it), so this arm fires only when the judge is the walk's first live path
(a resumed walk, or a project given a provider mid-walk; §D.4 uses the latter).

**`drive_once` (D12).** After the cancel check (`:6023-6025`) and before the policy:

```rust
        // MOD-10 D12: the walk's secrets, resolved by this live path's entry (blueprint A-1) and
        // read from the cell here; a provider-less project's env stays empty.
        let env = self
            .parts
            .secrets
            .env_for(&project)
            .await
            .map_err(EngineError::Secrets)?;
```

and `env: BTreeMap::new()` (`:6099`) with its `R-SEC-2` comment becomes `env,` with: "R-SEC-2,
MOD-10 D12: the resolved map, exactly; never htui's own environment (spec.env is applied last,
`cli/mod.rs:401-419`)." The `#[expect]` reason on `drive_once` is unchanged (no parameter added).
An error here is defensive only (the entry already answered the cached outcome) and travels the
existing arms (`fail_hard` / `fail_candidate` / `fail_judge`) with `failure_text`'s sentence.

**`verify` re-mask (A-11, flagged).** In `Engine::verify` after `let Some(report) = report else …`
(`:3746`):

```rust
        // MOD-10 (blueprint A-11): the verifier masks with the pattern rules only (D19); a
        // command that printed a file holding a resolved value is masked here with the walk's
        // scrubber before the output is persisted or read. A refusal drops the text, as the
        // verifier's own `scrubbed` does.
        let report = VerifyReport { output: remasked(self.parts.scrubber, report.output), ..report };
```

`remasked` is a private free fn: `Value::String` round trip, `Err` → `String::new()`.

**Test plumbing (A-10).**
- `fake_parts(orch, graphs, driver, secrets: &'a RunSecrets)` sets `scrubber: secrets` and
  `secrets`.
- `dispatch_fake`, `claim_fake`, `resume_fake`, `sweep_fake` (`:7011-7066`) and the 18 test
  callers build `let scrubber = RunSecrets::new(orch.secret_source());`. A fresh `RunSecrets` per
  call is one per walk, as the worker's `Kit` is (D14).
- `FakeOrchestrator` (`fake.rs`) gains `secret_source: Mutex<Option<Arc<dyn SecretSource>>>`,
  `pub fn set_secret_source(&self, source: Arc<dyn SecretSource>)` and
  `pub fn secret_source(&self) -> Option<Arc<dyn SecretSource>>`; `restarted()` carries it (a
  source is the machine's, like the keyring).
- The 6 inline `super::EngineParts {` test literals: `:7464` and `:7555` build fresh (add a
  local `let secrets = RunSecrets::none();` and `secrets: &secrets`); `:10257`, `:10359`,
  `:13271` and `:14389` copy from a `fake_parts` result read just above them
  (`:10253`, `:10355`, `:13268`, `:14383`) and take `secrets: parts.secrets`.
  `conformance.rs:8593` takes `secrets: parts.secrets`; `gix_isolator.rs`'s macro builds fresh.

### B.7 T5 — worker (`htui-worker/src/runtime.rs`)

```rust
// Shared<P>
    /// MOD-10 D15: the process's secret source, handed to every `Kit`'s `RunSecrets`; `None`
    /// refuses provider projects (`secrets_refused`) and leaves the rest untouched.
    secrets: Option<Arc<dyn SecretSource>>,

impl RunRuntime<H, P> {
    /// MOD-10 D15: a runtime whose walks resolve provider projects' secrets through `source`
    /// (`htui`'s `KeyringInfisical`, one per process, shared with the agent runtime).
    ///
    /// # Panics
    /// When called after the runtime has served.
    #[must_use]
    pub fn with_secret_source(mut self, source: Arc<dyn SecretSource>) -> Self {
        self.configure().secrets = Some(source);
        self
    }
}
```

- `assemble` initialises `secrets: None` (`:1157`, beside `tools: None`).
- `Kit.scrubber: MinimalScrubber` (`:892`) is replaced by
  `secrets: RunSecrets` (doc: "MOD-10 D11: this task's walk's secrets: the engine's scrubber
  and its env, one object"), built in `Kit::read` as `RunSecrets::new(shared.secrets.clone())`
  (`:969`). Building it does no I/O, so non-walking tasks pay nothing.
- `Kit::engine` (`:1036-1058`): `scrubber: &self.secrets, secrets: &self.secrets`.
- The verifier singleton (`:455-460`) is **unchanged** (D19).
- `use htui_core::scrub::MinimalScrubber;` (`:22`) stays (the verifier uses it).

### B.8 T4 — `verify.rs` comment (D19, A-9)

`:323-325` becomes:

```rust
        // The process environment is handed to the child unchanged (plan D30). ANA-2 `:493` asks
        // for "the agent's environment minus the secrets", and that is what this is: a resolved
        // secret reaches an agent only through `SessionSpec.env` (MOD-10 D12), never this process's
        // own environment, which htui cannot change (`set_var` is `unsafe`, `unsafe_code` is
        // forbidden; MOD-10 D19).
```

### B.9 T6a — `crates/htui/src/secrets.rs` (`KeyringInfisical`, D15)

```rust
//! MOD-10 D15: the process's one secret source: the OS keyring's Infisical URL and machine
//! identity, and one `InfisicalProvider` per (URL, identity).

/// Builds a provider for a normalised URL and an identity (production: `InfisicalProvider::new`).
type Build = dyn Fn(&str, MachineIdentity) -> Result<Arc<dyn SecretProvider>, SecretError> + Send + Sync;

pub struct KeyringInfisical {
    build: Box<Build>,
    /// Held across the keyring read and the build, so two walks starting together build once.
    cached: tokio::sync::Mutex<Option<Cached>>,
}

/// The provider and what it was built from. The client secret is kept only as a SHA-256 digest
/// (`sha2`, already a dependency) to compare; the provider holds the identity itself.
struct Cached {
    url: String,
    client_id: String,
    secret_digest: [u8; 32],
    provider: Arc<dyn SecretProvider>,
}

impl KeyringInfisical {
    /// Production: `InfisicalProvider::new(InfisicalConfig::new(url), identity)`.
    #[must_use]
    pub fn new() -> Self;
    /// Tests: any builder (a counting one returning `FakeSecretProvider`s).
    #[cfg(test)]
    fn with_builder(build: Box<Build>) -> Self;
}

impl SecretSource for KeyringInfisical {
    fn provider(&self) -> SecretFuture<'_, Arc<dyn SecretProvider>>;
}
```

`provider()`, in order:
1. Lock `cached`.
2. On `tokio::task::spawn_blocking`: `htui_store::secret::get_infisical_url()` and
   `get_machine_identity()` (the keyring is synchronous I/O; `R-NF-3`). A `JoinError` →
   `Config("the OS keyring read did not finish")`.
3. URL: `Ok(None)` → `Config("no Infisical base URL is stored in the OS keyring")` (D15);
   `Err(StoreError)` → `Config(format!("the OS keyring could not be read: {err}"))`
   (`get_*`'s messages name slots, never values: `htui-store/src/secret.rs:466-472`);
   `Ok(Some(raw))` → `htui_secrets::normalise_base_url(&raw)?`.
4. Identity: `Ok(None)` → `NoIdentity`; `Err` → the same keyring `Config`; `Ok(Some(identity))`.
5. If `cached` holds `(url, client_id, digest)` equal to the new one → return its `Arc` (the
   401 latch and the cool-down survive, M2 D5). Otherwise `build(&url, identity)?`, replace the
   cache, return the new `Arc` (M4's new identity or URL takes effect at the next walk).

`Debug`: `KeyringInfisical { cached: true }`. `Default` delegates to `new()`
(`clippy::new_without_default`).

### B.10 T6a — chat (`agent_worker.rs`, D12, D13, A-12)

**Runtime.** `AgentRuntime` gains `secrets: Option<Arc<dyn SecretSource>>` (`None` in its
constructor beside `tools: None`, `:652`) and:

```rust
    /// MOD-10 D15: every chat this runtime starts or binds on a provider project resolves its
    /// secrets through `source` (the TUI's `KeyringInfisical`, shared with its run runtime).
    #[must_use]
    pub fn with_secret_source(mut self, source: Arc<dyn SecretSource>) -> Self;
```

**Inline, on the serve arm (no I/O).** In `start` after `project_settings` (`:2111`) and in
`bind_promoted` after `project_caps_for` (`:1037`):

```rust
        let project = backend.project(project_id).await?.ok_or_else(|| StoreError::NotFound {
            entity: "project",
            id: project_id.to_string(),
        })?;
        // MOD-10 D12: a column fault refuses here, before anything is written or opened.
        let secrets = match htui_core::secret::project_scope(&project) {
            Ok(None) => None,
            Ok(Some(_)) => Some(ChatSecrets { source: self.secrets.clone(), project }),
            Err(cause) => {
                return Ok(Served::Reply(StoreReply::Failed {
                    request: "chat_start", // `PROMOTE_STEP` in `bind_promoted`
                    message: cause.refusal(),
                }));
            }
        };
```

Placed before the tool lease is opened, so a refused chat leaves no lease and no run.

**Deferred write (fresh chat on a provider project).** When `secrets.is_some()`, `start` does
**not** call `writer.start_chat_run(&chat)` (`:2146`); it builds `closed` as
`Arc::new(AtomicBool::new(true))` (so neither `binding.close` nor the panic answer's
`closing(..)` touches a run that does not exist yet) and passes `secrets` in `ChatArgs`. With
`secrets == None` the sequence is today's, byte for byte.

```rust
/// MOD-10 D12/D13 (blueprint A-12): what the chat task resolves before it writes or starts
/// anything: off the store loop's arm (`R-NF-3`).
struct ChatSecrets {
    source: Option<Arc<dyn SecretSource>>,
    project: Project,
}
// ChatArgs
    /// `Some` only for a provider project.
    secrets: Option<ChatSecrets>,
```

`ChatArgs`'s `Debug` adds `.field("secrets", &self.secrets.is_some())`.

**`run_chat` prelude**, first thing after destructuring (`:4120-4137`), `spec` rebound `mut`:

```rust
    if let Some(ChatSecrets { source, project }) = secrets {
        match htui_core::secret::resolve_project(source.as_deref(), &project).await {
            Ok(resolved) => {
                spec.env = resolved.as_ref().map(|r| r.as_map().clone()).unwrap_or_default();
            }
            Err(cause) => {
                // D13: the same sentence a run fails with; nothing was written, nothing started.
                let message = cause.refusal();
                frames.to_stream(StoreReply::Failed { request: binding.request(), message: message.clone() });
                frames.failed(message);
                return;
            }
        }
        if let ChatBinding::Fresh(chat, closed) = &binding {
            if let Err(err) = writer.start_chat_run(chat).await {
                let message = err.to_string();
                frames.to_stream(StoreReply::Failed { request: binding.request(), message: message.clone() });
                frames.failed(message);
                return;
            }
            closed.store(false, Ordering::Release);
        }
    }
    // MOD-10 D11 for chats: the scrubber is built from the very map the env holds.
    let (scrubber, short) = MinimalScrubber::from_resolved(&spec.env);
    if !short.is_empty() {
        tracing::warn!(step = %binding.step_id(), keys = ?short, "these secrets are shorter than the masking floor: injected, not masked");
    }
```

- `MinimalScrubber::new(spec.env.values().cloned())` (`:4139`) is replaced by the line above.
- The refusal's frames mirror the existing start-failure arm (`:4227-4234`) minus
  `binding.close` (no row for a fresh chat; a no-op for a promoted one, `:2404`). **VERIFY —
  implementer must check** that the tab clears its pending chat on `ChatFrame::Failed` with no
  `Ended`, or send `chat_failed`'s pair (`Failed` + `Ended`, `:4030-4036`) instead, as the
  start-failure arm's `frames.failed` does.
- The `ResolvedSecrets` drops (zeroized) at the end of the `Ok` arm; the clone in `spec.env` is
  OQ-C's documented residual.
- Short keys are logged by name only (D17).

### B.11 T6b — process wiring

- `crates/htui/src/lib.rs` (`:152-174`): before `spawn_hosted`,
  `let secrets: Arc<dyn SecretSource> = Arc::new(secrets::KeyringInfisical::new());` and
  `store_worker::spawn_hosted(started, request_rx, reply_tx, AgentRuntime::production().with_registration_probe(), tools, Some(secrets))`.
  Building it reads nothing (the keyring is read per walk).
- `store_worker::spawn_hosted` (`:2168`) gains `secrets: Option<Arc<dyn SecretSource>>`; before
  `host_the_runtimes`, `if let Some(source) = &secrets { runtime = runtime.with_secret_source(Arc::clone(source)); runs = runs.with_secret_source(Arc::clone(source)); }`.
  One source, both runtimes (D15). The test caller (`:5513`) passes `None`.
- `crates/htui/src/worker_cmd.rs` (`:105-107`): `.with_secret_source(Arc::new(crate::secrets::KeyringInfisical::new()))`
  after `.with_tool_host(..)`. `htui worker` has its own source (D15).

---

## C. Data flow

1. **Walk start** (any of `start_run`, `reclaim`, `resumed`, `on_run_unless_claimed`,
   `unblock`'s resume): `Kit::read` builds `RunSecrets::new(shared.secrets)` (no I/O);
   `Kit::engine` lends it as `scrubber` and `secrets`.
2. **First live path** (A-1): `walk_live_step` / `drive_group` / `run_judge` →
   `secrets_ready(run)` → `project` row → `RunSecrets::prepare`:
   provider-less → nothing; provider → `resolve_project` → `KeyringInfisical::provider()`
   (keyring on a blocking thread; cached `InfisicalProvider` unless the URL or identity changed)
   → `resolve(scope)` → `check_env` → `from_resolved` → cell `Ok(Resolved)`; any error → cell
   `Err(cause)`.
3. **Refusal**: plain → `fail_hard` with `secrets_refused: …`; group →
   `fail_group_before_a_token`; judge → `refuse_judge_secrets`. No `prepare`, no prompt row,
   no `driver.start`.
4. **Persistence** from here on (prompt masking, trim record, prompt row, recorder rows, notes,
   verify output via A-11): `RunSecrets::scrub` → the resolved `MinimalScrubber`.
5. **Session**: `drive_once` → `env_for` (cached) → `SessionSpec.env` = the map → driver
   (`spec.env` applied last).
6. **Recorder seam** (T2): at 16 KiB, `seam_cut` with `RunSecrets::hold_back()`; head flushed,
   tail carried.
7. **Walk end**: the `Kit` drops → `RunSecrets` → `ResolvedSecrets` and the scrubber zeroize.
8. **Verifier** (D19): `sh -c` with htui's environment, which never holds a resolved value.
9. **Chat**: serve arm → `project_scope` (inline) → `ChatArgs.secrets` → task prelude →
   `resolve_project` → env + `from_resolved` scrubber → (fresh) `start_chat_run` → recorder →
   `driver.start`.

---

## D. Tests, per task (written first; each must fail before its implementation)

Fixture rule for every lane: a resolved value that must be **masked** is long and not
pattern-shaped (e.g. `zq7-resolved-value-0123456789`, 29 chars), so a refusal can never stand in
for a mask. A pattern key used to test refusal is the M1 dual-valid one,
`sk-ant-api03-abcdefghijklmnopqrstuvwx`.

### D.1 T1 (`crates/htui-core/src/scrub.rs` `mod tests`)

- `an_escaped_token_start_is_a_token_start`: with `rules_only()`, each of
  `x\nsk-ant-api03-abcdefghijklmnopqrstuvwx` (literal backslash-n), `\u0022AKIAIOSFODNN7EXAMPLE`,
  `%22ghp_abcdefghijklmnopqrstuvwxyz0123456789`, and the `\r`, `\t`, `\b`, `\f`, `\/`, `\\`,
  `\"` forms of the Anthropic key, refuses with the right rule at `/t`. Build the backslash with
  `'\\'` in `format!` (a `\u0022` in source is a Rust escape error, and a JSON tool parameter
  decodes it: H-21).
- `an_escaped_start_reaches_the_openai_confirmation`: `x\nsk-Ab3_xY9-kLmN0pQrStUvWx` (a LiteLLM
  key after `\n`) refuses as `openai_api_key` (exercises `SK_CANDIDATE` and `OPENAI_STRICT`).
- `an_escape_lookalike_is_not_a_token_start`: `subtask-abcdefghijklmnopqrstuvwxyz`,
  `x%2Fsk-learn`, `nsk-ant-api03-…`, `%2sk-ant-api03-…` (one hex digit), `\u002sk-ant-api03-…`
  (three), `%ZZsk-ant-api03-…`, `src/sk-live.rs` are all `Ok`.
- The existing `prose_that_shares_a_prefix_is_not_a_credential`,
  `a_prefix_inside_a_word_is_not_a_credential`, `each_rule_refuses_a_real_shaped_key_in_four_positions`,
  `one_character_short_of_every_minimum_is_clean_under_all_rules` stay green unchanged.
- `from_resolved_masks_a_value_and_its_newline_free_form`: `{"K": "abcdefgh\n"}` masks both
  `"x abcdefgh\n y"` and `"x abcdefgh y"`; `"abcdefgh\r\n"` likewise; `short` is empty;
  `Debug` shows `secrets: 2`.
- `a_newline_free_form_below_the_floor_is_not_masked`: `{"K": "abcde\n"}` (6 chars) masks
  `abcde\n` but not a bare `abcde`, and `K` is not in `short`.
- `hold_back_is_zero_for_a_scrubber_that_does_not_say`: a local `struct Plain;` implementing
  only `scrub` answers `0`.
- `hold_back_without_secrets_is_the_pattern_bound`: `rules_only().hold_back() == PATTERN_HOLD_BACK - 1`.
- `hold_back_covers_the_longest_secret`: a 120-byte secret gives `119`; a 10-byte one gives
  `PATTERN_HOLD_BACK - 1`; byte length, not chars (`"é".repeat(60)` → `119`).
- `pattern_hold_back_covers_every_rule_minimum_and_the_longest_token_start` (A-4): for every
  `one_short()` value, `value.len() + 1 + 6 <= PATTERN_HOLD_BACK` (`6` named `LONGEST_TOKEN_START`
  in the test, with a comment pointing at `\\u[0-9A-Fa-f]{4}`).
- `the_seam_is_a_send_sync_trait_object` keeps passing (the default method keeps the trait
  dyn-compatible).

### D.2 T2 (`crates/htui-agent/tests/recorder.rs`, `src/record.rs`, `src/conformance.rs`)

Local helpers in `tests/recorder.rs`: `fn fill(n: usize) -> String` (`n` bytes of `x`), and
`fn texts(log) -> Vec<String>` (the `assistant_text` rows' `text`, in `seq` order). Every case
asserts its leak criterion on **each row** and on **`texts.concat()`**.

1. `a_resolved_secret_split_at_the_seam_is_never_split_across_rows`: scrubber
   `MinimalScrubber::new([SECRET40])` (40 chars, not pattern-shaped); chunks: 15 × `fill(1024)`,
   then `fill(1004) + SECRET40[..20]`, then `SECRET40[20..] + " tail"`, then `done`. No row and
   not the concatenation contains `SECRET40` or either half of it at the seam; the concatenation
   contains `[REDACTED]` once; `finish()` is `Ok`.
2. `a_pattern_key_split_at_the_seam_is_refused_whole`: `rules`-only scrubber
   (`MinimalScrubber::new([])`); the key split after its first byte (`"s"` | `"k-ant-…"`) across
   the trigger chunk and the next. The carried row becomes one `scrub_residue` row; no row and not
   the concatenation contains the key; `finish()` answers `Unmasked { rule: "anthropic_api_key" }`.
3. `a_pattern_key_already_matching_at_the_bound_flushes_whole_as_today`: the trigger chunk ends
   with `sk-ant-api03-abcdefghij` (already ≥ the minimum) and the next chunk carries the rest.
   The first run is one `scrub_residue` row (no cut: `whole` refused); the next row holds only the
   tail; the full key is in no row and not in the concatenation. Pins the documented residual.
4. `a_secret_entirely_within_hold_back_of_the_bound_is_carried_whole`: a 30-char secret ending
   10 bytes before the trigger's end: row 1 holds no part of it, row 2 holds `[REDACTED]`.
5. `a_complete_secret_straddling_the_first_candidate_moves_the_cut_back`: a 70-char secret
   ending 10 bytes before the end (so it spans `len - 75`): both rows clean, the concatenation
   holds `[REDACTED]` once, row 1 is shorter than `CHUNK_FLUSH_BYTES - hold_back` (the cut moved).
6. `a_seam_cut_never_splits_a_character`: runs of `é` (2 bytes) and `𝄞` (4 bytes) around the
   cut; every row is valid UTF-8 (it is a `String`), and the concatenation equals the input.
7. `the_seam_is_deterministic`: the same chunk stream into two fresh stores gives identical
   `(seq, text, at)` lists; row 1 is `CHUNK_FLUSH_BYTES - scrubber.hold_back()` bytes.
8. `the_carried_row_takes_its_first_chunk_s_capture_time`: chunk `at`s distinct; row 2's `at`
   is the `at` of the chunk the cut fell in.
9. `a_turn_end_writes_the_carried_tail`: after a seam, `done` flushes the carry as its own row
   (`texts.len() == 2`, total bytes preserved).
10. `a_new_message_id_writes_the_carried_tail`: after a seam, a chunk with another `message_id`
    flushes the carry first.
11. `a_straddling_chunk_s_raw_rides_both_rows` (`retain_raw = true`, the M1 `acp_chunk` helper):
    the chunk the cut falls in appears in row 1's raw array and as the first element of row 2's.
12. `a_secret_split_after_the_straddling_chunk_withholds_the_carry_s_raw`: `retain_raw = true`,
    the secret starts in the straddling chunk and ends in the next: row 2's `raw` is `NULL`,
    `raw_withheld() == 1`, and no row's raw (nor row 1's raw text joined with row 2's) holds it.
13. `a_scrubber_without_hold_back_cuts_at_the_bound_as_before`: the file's `MaskKey` scrubber
    (`:1239`, default `hold_back`) gives `[16384, 1024]` (A-3: the default path is today's).
14. `chunks_flush_at_the_16_kib_bound` (existing, `:1830`) re-expressed: `let hold = scrubber().hold_back();`
    row 1 is `CHUNK_FLUSH_BYTES - hold`, row 2 `1024 + hold`, seqs `0, 1`; the doc says "cut a
    hold-back before the bound".
15. `src/conformance.rs` `chunk_flush_at_16kib` (A-3): `vec![CHUNK_FLUSH_BYTES - hold, 1024 + hold]`
    with `let hold = scrubber().hold_back();` and its message updated.
16. `src/record.rs` `mod tests`: `seam_cut_is_none_without_hold_back`,
    `seam_cut_is_none_when_the_whole_text_is_refused`,
    `seam_cut_moves_back_over_a_complete_secret`, `seam_cut_lands_on_a_char_boundary`,
    `split_marks_puts_a_straddling_chunk_in_both_halves` (pure, no store).

### D.3 T3

**`crates/htui-core/src/secret.rs` `mod tests`:**
- `project_scope_is_none_without_a_provider`: `secret_provider: None` with `secret_scope` set to
  garbage is `Ok(None)` (the provider column is the switch, D12).
- `project_scope_parses_an_infisical_scope`.
- `project_scope_refuses_an_unknown_provider_and_escapes_it`: `"vault"` and `"inf\u{1b}[31m"`
  give the exact sentence; the second shows `\u{1b}` escaped.
- `project_scope_refuses_a_provider_without_a_scope` / `…_an_unparsable_scope`.
- `check_env_refuses_a_reserved_key_and_names_it`: `{"API_KEY", "HTUI_MCP_TOKEN"}` →
  `ReservedKey { key: "HTUI_MCP_TOKEN" }`; exact `Display`.
- `check_env_is_case_sensitive_and_needs_the_underscore`: `htui_x`, `HTUIX`, `MY_HTUI_X` pass.
- `check_env_accepts_an_empty_map`.
- `secret_error_refusal_prefixes_the_display`: `BadCredentials.refusal()` is
  `secrets_refused: Infisical refused the machine identity's login: …` byte for byte.
- `every_secret_error_variant_is_covered`: `variant()` gains `ReservedKey`; `every_variant()`
  gains `ReservedKey { key: "HTUI_LOG".into() }`; both counts become **16**; `ReservedKey` joins
  the key-carrying arm (names its key, no sentinel).
- `resolve_project_never_touches_the_source_without_a_provider`: `FakeSecretSource::calls() == 0`.
- `resolve_project_refuses_a_column_fault_before_the_source`: unknown kind, missing scope, bad
  scope → `Config`, `calls() == 0`.
- `resolve_project_refuses_without_a_source`: `None` → the no-source sentence.
- `resolve_project_refuses_a_provider_of_another_kind`: `FakeSecretProvider::with_kind("vault")`.
- `resolve_project_passes_a_provider_error_through`: `failing(BadCredentials)`.
- `resolve_project_refuses_a_reserved_key_from_the_provider`.
- `resolve_project_returns_the_map_and_the_scope_reached_the_provider`: `scopes() == [scope]`.
- `the_fakes_count_and_repeat_the_last_answer`: three resolves of a two-answer script.
- `the_fakes_never_print_a_value`: `Debug` of both fakes holds no scripted value.
- `a_source_is_usable_as_arc_dyn_and_send`.

**`crates/htui-core/src/store/mem.rs`** (its tests module): `set_project_secret_columns_plants_both`
(read back through `ReadStore::project`; an unknown project is a no-op).

**`crates/htui-secrets/tests/*.rs`**: no new test; both binaries compile and
`no_error_or_debug_carries_a_value_the_client_secret_or_the_token` still counts 15.

### D.4 T4

**`crates/htui-orch/src/secrets.rs` `mod tests`** (`#[tokio::test]`, `FakeSecretSource`):
- `a_provider_less_project_gets_nothing_and_never_touches_the_source` (`calls() == 0`, env empty,
  slot unbound: a later provider project still resolves).
- `a_provider_project_resolves_once_for_every_caller`: three sequential `env_for` and a
  `join_all` of three concurrent ones → `calls() == 1`, `resolves() == 1`, every env equal.
- `the_env_is_the_resolved_map_and_nothing_else` (R-SEC-2, unit).
- `before_resolution_only_the_pattern_rules_apply`: the value passes unmasked, an AWS key
  refuses, `hold_back() == PATTERN_HOLD_BACK - 1`.
- `after_resolution_the_values_are_masked_and_the_seam_widens`: a 200-byte value →
  `hold_back() == 199`.
- `a_refusal_is_cached_and_never_retried`: `failing(Unreachable)` → two `prepare`s, `calls() == 1`,
  equal errors; the scrubber is pattern-only.
- `a_second_project_is_refused`: bound to A; `prepare(B)` with a provider and `prepare(B')`
  provider-less both answer the foreign-project `Config`.
- `each_column_or_source_fault_refuses_with_its_sentence` (table: no source, unknown kind, no
  scope, bad scope, `ReservedKey`, kind mismatch).
- `short_values_are_injected_and_not_masked`: `{"PIN": "123"}` → env holds it, scrub leaves it.
- `the_debug_names_no_value_and_no_key`.

**`crates/htui-orch/src/status.rs`**: a row in `run_failure_display_is_ana2s_bytes`:
`SecretsRefused { cause: SecretError::NoIdentity }` →
`secrets_refused: no Infisical machine identity is stored in the OS keyring`; and
`secrets_refused_is_the_cause_s_refusal` over every `SecretError` the test can name.

**`crates/htui-orch/src/engine.rs` tests** (beside the MOD-10 D5 module, `:17485+`; a local
`fn provider_project(harness, map)` sets the columns with `set_project_secret_columns` and
`orch.set_secret_source(FakeSecretSource::new(FakeSecretProvider::resolving(map)))`):
- `failure_text_types_a_secrets_refusal` (sync): `EngineError::Secrets(NoIdentity)` →
  `RunFailure::SecretsRefused{..}.to_string()`.
- `a_provider_project_s_session_gets_the_resolved_env`: `orch.spec_for(&key).env == map`
  (keys equal exactly: R-SEC-2).
- `a_provider_less_project_s_session_gets_no_env_and_never_touches_the_source`.
- `a_refused_resolution_fails_the_plain_run_before_any_agent_starts`: one case per cause (no
  source, unknown kind, missing scope, bad scope, `BadCredentials`, `Unreachable` and
  `LoginCoolingDown` (OQ-A), `ReservedKey`): the walk answers `Err(EngineError::Secrets(_))`;
  `run.status == Failed`, `run.failure == Some(cause.refusal())`; the `prd` step is `Failed`;
  `orch.spec_for(&key)` is `None` (no `driver.start`); `orch.isolator.prepares() == 0`; the step
  has no events and no `trim_record`; the item is not `Blocked` (H-13 of M1).
- `mod61_a_trim_record_and_a_prompt_equal_to_a_resolved_value_are_stored_redacted`:
  `add_primary_repo_named(VALUE)` (`:7310`); the run walks; the step's `trim_record` and its seq-0
  prompt row hold `[REDACTED]` and not `VALUE`; the spec env holds `VALUE`. **Precondition** (M1
  H-15): the run did not end `prompt refused`.
- `mod61_an_agent_echoing_a_resolved_value_is_stored_redacted`: a scripted `AssistantChunk`
  holding `VALUE` → no `step_events` row holds it.
- `a_walk_resolves_once_across_its_steps`: an ungated multi-step walk in one dispatch →
  `resolves() == 1`.
- `a_resumed_walk_resolves_again` (D14): gated `prd` parks; `AnswerGate` approves in a second
  dispatch → `resolves() == 2` (each `dispatch_fake` is a fresh `RunSecrets`).
- `a_verify_output_holding_a_resolved_value_is_masked` (A-11, flagged):
  `orch.verifier.script_report(VerifyReport { output: format!("leaked {VALUE}"), .. })` → the
  `command_run.output` holds `[REDACTED]`, not `VALUE`.

**`crates/htui-orch/src/conformance.rs` `mod fanout_paths`** (never `CASES`, H-1):
- `a_fan_out_resolves_once_and_every_candidate_gets_the_env`: `fan_research(Gate::Never)`,
  `research_candidates(1)`, provider project: `resolves() == 1`; each candidate's spec env equals
  the map.
- `a_fan_out_whose_secrets_refuse_fails_the_run_before_any_candidate_starts`: source
  `failing(BadCredentials)`, `retry_limit` left as seeded: every candidate `Failed` with the note
  `` fan-out candidate {i} of `research` attempt 1: secrets_refused: … ``; `run.failure ==
  Some(cause.refusal())`; no candidate `spec_for`; no attempt 2 slot (no retry); `calls() == 1`.
- `a_judge_whose_secrets_refuse_fails_the_run_not_the_selection`: `fan_research(Gate::Never)`,
  `set_judge`, `research_candidates(1)`, the project **provider-less**, source `failing(..)`;
  `let (stalled, wake) = orch.suspend_after_done("research", 1, Some((2, 0)));` and
  `tokio::join!(orch.dispatch(start), async { stalled.notified().await; orch.store().set_project_secret_columns(project, Some("infisical"), Some(SCOPE)); wake.notify_one(); })`.
  Candidate 2 ends with no document (two survivors, so the route is the judge); the judge's
  entry is the walk's first resolution: `run.failure == Some(cause.refusal())`, run `Failed`
  (not `AwaitingApproval`), no judge `spec_for`, `calls() == 1`, the candidates' specs had empty
  env (they ran provider-less). **VERIFY — implementer must check** that a suspended candidate
  with no document fails `missing_output` and leaves exactly two passing.

**`gix_isolator.rs`**: mechanical only; its suite stays green.

### D.5 T5 (`crates/htui-worker/src/runtime.rs` `mod tests`)

A local `TransportBuilder` `Echoes { text, slot: SpecSlot }` builds
`FakeDriver::new(..).with_spec_slot(slot.clone())` whose turn emits one `AssistantChunk(text)`
then `done`. A local `provider_store(map_value) -> MemStore` is `seeded(MemStore::from_demo(data))`
with `data.projects` for `PROJECT_HTUI` given `secret_provider = Some("infisical")` and a scope.

- `with_secret_source_reaches_the_engine`: a claimed run's spec env equals the map.
- `mod61_a_walked_step_stores_a_resolved_value_only_redacted`: the agent echoes `VALUE`; the
  primary repo is named `VALUE` (trim record); read back **from the store**: `step_events` (all
  rows), the step's `trim_record` and the seq-0 prompt row hold `[REDACTED]` and never `VALUE`.
- `mod62_a_verify_command_printing_its_environment_shows_no_resolved_value`: a runtime over
  `FakeIsolator::new()` rooted at a `Scratch` dir (`root_trees_at`) and a real
  `ShellVerifier` (built as `verify.rs`'s `verifier_masking` helper builds one, `:952-970`, with
  an empty scrubber); the phase's `verify_command` is `env` (`set` on Windows); the
  `command_run` row's `output` contains `PATH` (the command did print the environment) and never
  `VALUE`; the agent's spec env did hold `VALUE` (the test is not vacuous). **VERIFY —
  implementer must check** how the worker test data sets a phase's `verify_command`
  (`demo_data().graphs` before `MemStore::from_demo`).
- `a_runtime_without_a_source_walks_a_provider_less_project_as_before`: reaches
  `AwaitingApproval`, empty env.
- `a_provider_project_without_a_source_fails_secrets_refused`: `run.failure ==
  Some(SecretError::Config(NO_SOURCE).refusal())`, no spec recorded.
- `r_sec_2_the_agent_env_holds_only_resolved_keys`: the env's key set equals the map's; no key
  starts with `HTUI_`, none is `DATABASE_URL`, `HTUI_TEST_DATABASE_URL`, a Qdrant or Infisical
  variable.
- `each_task_resolves_in_its_own_kit`: two claimed runs → `resolves() == 2`.

Gate: `--test-threads=1` (memory: the keyring fake is process-wide; these tests never touch it,
but the suite's green is scheduling-dependent).

### D.6 T6a

**`crates/htui/src/secrets.rs` `mod tests`** (`htui_store::testkit::mock_keyring().await`
guard in each; a counting builder returning `FakeSecretProvider`s):
- `no_stored_url_refuses_with_the_d15_sentence`.
- `no_stored_identity_is_no_identity`.
- `a_half_stored_identity_refuses_naming_the_keyring_only` (no client secret in the `Display`).
- `a_broken_keyring_refuses_naming_the_keyring` (`mock_keyring_broken`).
- `a_bad_stored_url_is_refused_by_normalisation` (`http://192.168.1.10`).
- `an_unchanged_keyring_reuses_the_provider`: two `provider()` calls → `Arc::ptr_eq`, builds 1.
- `an_equivalent_url_spelling_reuses_the_provider`: `https://x.example/` then `https://x.example`.
- `a_new_identity_or_url_rebuilds_the_provider`: `set_machine_identity` with another secret →
  builds 2, `!ptr_eq`; likewise for the URL.
- `the_login_latch_survives_two_walks`: the builder's provider is scripted
  `[Err(BadCredentials), Err(LoginRefusedEarlier)]`; two `RunSecrets` over one `KeyringInfisical`
  (two walks): walk 1 refuses `BadCredentials`, walk 2 `LoginRefusedEarlier`, builds 1.
- `the_source_never_prints_a_secret` (`Debug`).

**`crates/htui/src/agent_worker.rs` `mod tests`** (the M1 chat fixture; the project's columns via
`set_project_secret_columns`; `AgentRuntime::…with_secret_source(FakeSecretSource)`):
- `a_chat_on_a_provider_project_gets_the_env_and_masks_its_echo`: the adapter's `spec_handle`
  env equals the map; the agent echoes `VALUE`; `step_events` never hold it.
- `a_refused_chat_writes_no_run_and_answers_the_sentence`: source `failing(NoIdentity)`; the
  reply carries `NoIdentity.refusal()`; `store.runs(project)` count unchanged; no `run_step`.
- `a_column_fault_refuses_the_chat_on_the_serve_arm`: unknown provider → `Served::Reply(Failed)`
  directly, the source never called, no lease opened (`FakeToolHost::opened()` empty).
- `a_provider_less_chat_never_touches_the_source_and_writes_its_run_inline` (today's sequence).
- `chat_resolution_runs_in_the_chat_task_not_on_the_serve_arm` (A-12): a test-local
  `SecretSource` whose `provider()` awaits a `Notify`; `serve` returns `Served::Start` before the
  source answers.
- `the_chat_scrubber_is_built_from_the_resolved_map`: a short value is injected and not masked;
  a `"…\n"` value's newline-free echo is masked.
- `a_promoted_chat_on_a_provider_project_gets_the_env` and
  `a_refused_promotion_starts_no_session`. **VERIFY — implementer must check** the promoted-chat
  harness the MOD-37 M5 tests use, and reuse it.

Gate: `cargo test -p htui --all-features -- --test-threads=1 secrets agent_worker`.

### D.7 T6b, T7

- T6b: `store_worker`'s `spawn_hosted` test (`:5513`) passes `None`; the `run_worker`,
  `store_worker` and `worker_cmd` suites stay green.
- T7: `bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh` green.

---

## E. Hazards

- **H-1 Stack headroom (`htui-orch`).** `every_case_name_dispatches` (`conformance.rs:7794`)
  sits near the 2 MiB debug stack. `secrets_ready` is `Box::pin`ned at its three call sites;
  `prepare` / `env_for` return a boxed `SecretFuture`; `RunSecrets.slot` holds a `Box<Bound>`, so
  the `RunSecrets` each `*_fake` keeps in its frame is ~100 bytes (it replaces a 24-byte
  `MinimalScrubber`). New engine tests use `Box::pin(harness.dispatch(..))` as M1's do; new
  conformance tests go in `mod fanout_paths`. Gate:
  `cargo test -p htui-orch --all-features --no-fail-fast 2>&1 | tee /tmp/orch.log; grep -c SIGABRT /tmp/orch.log`
  must print 0. If one appears, box the dispatch future of the overflowing case first; never
  raise the stack.
- **H-2 `--all-features` / `testkit`.** Without it `tests/*.rs` run 0 tests and say `ok`
  (memory). Every gate carries `--all-features`.
- **H-3 Featureless clippy.** `mod fake` is `#[cfg(any(test, feature = "test-support"))]`;
  `KeyringInfisical::with_builder` is `#[cfg(test)]`; `MemStore::set_project_secret_columns` is
  unconditional like its sibling. Run `cargo clippy --workspace -- -D warnings` too (memory).
- **H-4 `Cargo.lock`.** Only T6a, and only `htui`'s dependency list (`htui-secrets` is a
  workspace member already). `--offline`; never `cargo update`. A lock diff in another lane is a
  leak from a shared tree.
- **H-5 Lane B needs T1's `hold_back` (hidden coupling).** `RunSecrets::hold_back` delegates to a
  trait method T1 adds. Lane B's T3 has no such need, but **T4 must start on a base that holds T1's
  commit** (merge or cherry-pick it into lane B's worktree). Without the override `RunSecrets`
  would compile against a later T1 and silently answer the default `0`: no seam protection on the
  run path. `after_resolution_the_values_are_masked_and_the_seam_widens` pins it.
- **H-6 Seam moves (A-3).** Every recorder over a `MinimalScrubber` cuts 75+ bytes earlier. Run
  the full workspace suite and `cargo insta test --workspace --all-features` on the merged tree;
  a pending `.snap.new` is investigated, not accepted blindly (memory: grepping `.snap` undercounts).
- **H-7 Byte-exact sentences.** Tests build expected text from `cause.refusal()` /
  `RunFailure::SecretsRefused{..}.to_string()`, never a retyped literal, except the one
  `status.rs` row and the `secret.rs` `refusal` test that pin the bytes. The candidate note
  carries `` fan-out candidate i of `phase` attempt n: `` before it.
- **H-8 No value in any log or error.** `tracing` fields are key **names**, counts, project and
  step ids. `RunSecrets`, `KeyringInfisical`, the fakes and `ChatArgs` have hand-written `Debug`s.
  `SecretError::Config` sentences built here quote only the provider column (`{:?}`) and the
  keyring's own slot-naming message.
- **H-9 Keyring fake is process-wide.** Only `crates/htui/src/secrets.rs` tests take
  `mock_keyring()`; the T4/T5/T6a chat tests use `FakeSecretSource`. The htui gate runs
  `--test-threads=1`.
- **H-10 `R-NF-3` (A-12).** Nothing on `AgentRuntime::serve`'s arm awaits the source. The
  `project` read and `project_scope` are the only additions there; the resolution, the keyring
  read and the deferred `start_chat_run` are in `run_chat`.
- **H-11 Deferred chat write (A-12).** `closed` starts `true` for a deferred fresh chat and flips
  to `false` only after `start_chat_run` lands, so `binding.close` and the panic answer never
  close a run that was never written. A provider-less chat is unchanged.
- **H-12 Single-project guard.** Checked **before** the provider column and **after**
  `get_or_init`. It never fires legitimately; a test that walks two projects through one
  `RunSecrets` is a test bug.
- **H-13 Item status.** The group refusal uses `block = false`, so the item mirrors `failed`, as
  `fail_hard` does on the plain path; a test asserts `!= Blocked`.
- **H-14 Judge reachability.** In one walk the judge always finds the cell resolved by its
  candidates; `refuse_judge_secrets` fires only for a judge that is its walk's first live path
  (§D.4's mid-walk provider switch, or a resumed walk with a `pending` judge).
- **H-15 Tokio in `htui-core`.** None added. `resolve_project` and the fakes are plain boxed
  futures; `htui-core`'s own async tests use its existing `tokio` dev-dependency.
- **H-16 `HTUI_` and the MCP token.** htui's token rides the MCP server spec's env
  (`htui-mcp/src/lib.rs:35`), not the agent's, so `check_env` is what keeps a project secret from
  shadowing `HTUI_*` in the agent's own environment; it is not a token-safety check.
- **H-17 Transient refusals (OQ-A).** No retry anywhere: a `LoginCoolingDown` or `RateLimited`
  fails the run like `BadCredentials`. The maintainer re-runs.
- **H-18 Resolution inside the step deadline.** `walk_live_step` resolves after `started_at`, so
  a slow Infisical (≤ 25 s: 5 s connect + 20 s request) is spent from the step's deadline. Accepted;
  documented in T7.
- **H-19 Worktrees.** Gortex `edit` writes to the primary checkout; edit with file tools inside a
  linked worktree, one `CARGO_TARGET_DIR` per worktree (~10 GB), `df -h .` (memory).
- **H-20 Two `conformance.rs` files.** Lane A's is `crates/htui-agent/src/conformance.rs`, lane
  B's is `crates/htui-orch/src/conformance.rs`. Different crates, no intersection.
- **H-21 Escapes in fixtures.** Build backslash fixtures with `'\\'` / `format!`: a `\u0022` in a
  Rust string literal is an escape error, and a JSON tool parameter decodes it before it reaches
  the file (observed while probing).

**21 hazards.**

---

## F. Build sequence, lanes, commits and gates

The plan's lanes are **confirmed**, with the file-set changes of A-3, A-5, A-6, A-7 and A-8 and
one ordering edge (H-5: T1 before T4):

| Lane | Order | Task | Touched files |
|---|---|---|---|
| **A** | T1 → T2 | T1 | `crates/htui-core/src/scrub.rs`, `scripts/scrub-audit.sql` |
| | | T2 | `crates/htui-agent/src/record.rs`, `crates/htui-agent/tests/recorder.rs`, `crates/htui-agent/src/conformance.rs` (A-3) |
| **B** | T3 → T4 → T5 | T3 | `crates/htui-core/src/secret.rs`, `crates/htui-core/src/store/mem.rs` (A-6), `crates/htui-secrets/tests/infisical.rs`, `crates/htui-secrets/tests/infisical_live.rs` (A-5) |
| | (T4 also after T1, H-5) | T4 | `crates/htui-orch/src/secrets.rs` (new), `crates/htui-orch/src/lib.rs`, `crates/htui-orch/src/engine.rs`, `crates/htui-orch/src/command.rs`, `crates/htui-orch/src/status.rs`, `crates/htui-orch/src/fake.rs`, `crates/htui-orch/src/conformance.rs`, `crates/htui-orch/src/verify.rs`, `crates/htui-orch/tests/gix_isolator.rs` |
| | | T5 | `crates/htui-worker/src/runtime.rs` |
| **C** | T6a (after T3) | T6a | `crates/htui/Cargo.toml`, `Cargo.lock`, `crates/htui/src/secrets.rs` (new), `crates/htui/src/agent_worker.rs`, `crates/htui/src/lib.rs` (one `mod` line, A-8) |
| — | after T5 and T6a | T6b | `crates/htui/src/lib.rs`, `crates/htui/src/worker_cmd.rs`, `crates/htui/src/store_worker.rs` |
| — | after T4, T5, T6a | T7 | `docs/htui-secrets.md` |

**Intersections.**
- Files: A ∩ B, A ∩ C, B ∩ C are all ∅. `crates/htui/src/lib.rs` is shared by T6a and T6b,
  which are serial (T6b waits on T6a). The two `conformance.rs` are different files (H-20).
- Crates: A and B both compile `htui-core` (different files); A's T2 and B's T4 never share a
  crate. B's T3 and C both build on `htui-core::secret`; C waits on T3.
- Hidden coupling: H-5 (T4 needs T1's `hold_back`); A-3/H-6 (T2 moves every seam: lanes B and C
  must not assert 16 KiB row splits); `Cargo.lock` is C's alone (H-4); no `.sqlx`, no migration.
  Concurrent lanes run in separate worktrees, or a lane waits for the other's commit before its
  gate (memory: shared-tree fan-out coupling).

**Within each task** (tests first; one commit per task unless noted; implementers commit as they
go, memory):
1. **T1**: §D.1 tests red → `TOKEN_START` → `from_resolved` newline form → `Drop` + `dedup_by` →
   `PATTERN_HOLD_BACK` + `hold_back` → audit SQL → `cargo test -p htui-core --all-features`, then
   `cargo test -p htui-agent --all-features` and `cargo test -p htui-orch --all-features verify`
   (a wider token start must not refuse an existing fixture). Commit
   `feat(mod-10): M3 T1 escaped token starts, newline masks, zeroize, Scrubber::hold_back`.
2. **T2**: §D.2 unit tests red → `ChunkMark` / `open_chunks` → `seam_cut`, `scrubbed_text`,
   `split_marks` → `flush_at_seam` and the trigger → integration tests red → green → the two
   pinned tests re-expressed (A-3) → `cargo test -p htui-agent --all-features --test recorder`,
   `cargo test -p htui-agent --all-features` (conformance and lib). Commit
   `feat(mod-10): M3 T2 recorder cuts at a hold-back seam and carries the tail`.
3. **T3**: §D.3 red → constants, `SecretSource`, `project_scope`, `check_env`, `ReservedKey`,
   `refusal`, `resolve_project`, `mod fake` → `MemStore` setter → htui-secrets arms →
   `cargo test -p htui-core --all-features secret`, `cargo test -p htui-core --all-features store::mem`,
   `cargo test -p htui-secrets --all-features`. Commit
   `feat(mod-10): M3 T3 SecretSource, project_scope, check_env, resolve_project, fakes`.
4. **T4** (base holds T1 and T3): two commits.
   - 4a, mechanical: `secrets.rs` + `lib.rs` + `EngineParts.secrets` + `fake_parts` /
     `*_fake` / the 8 literals + `FakeOrchestrator` source + `EngineError::Secrets` +
     `RunFailure::SecretsRefused` + the `verify.rs` comment, with `secrets.rs` and `status.rs`
     tests. The orch suite is green with no behaviour change (no project has a provider).
     Commit `feat(mod-10): M3 T4a RunSecrets and the engine's secrets part`.
   - 4b, behaviour: §D.4 engine and conformance tests red → `secrets_ready` and the three
     entries → `refuse_judge_secrets` → `drive_once` env → `failure_text` arm → A-11 re-mask →
     `cargo test -p htui-orch --all-features --no-fail-fast 2>&1 | tee /tmp/orch.log; grep -c SIGABRT /tmp/orch.log`
     (0). Commit `feat(mod-10): M3 T4b resolve at each live path's entry; secrets_refused fails the run`.
5. **T5**: §D.5 red → `Shared.secrets`, `with_secret_source`, `Kit.secrets`, `Kit::engine` →
   `cargo test -p htui-worker --all-features -- --test-threads=1`. Commit
   `feat(mod-10): M3 T5 the worker's Kit owns the walk's RunSecrets (MOD-61, MOD-62)`.
6. **T6a** (base holds T3): `Cargo.toml` + lock (`cargo build -p htui --offline`) →
   `secrets.rs` tests red → `KeyringInfisical` → `lib.rs` mod line → chat tests red →
   `AgentRuntime` source, inline column check, `ChatSecrets`, `run_chat` prelude, scrubber →
   `cargo test -p htui --all-features -- --test-threads=1 secrets agent_worker`. Commit
   `feat(mod-10): M3 T6a keyring Infisical source; chats resolve in their task`.
7. **T6b** (base holds T5 and T6a): wiring → `cargo build -p htui` →
   `cargo test -p htui --all-features -- --test-threads=1 run_worker store_worker worker_cmd`.
   Commit `feat(mod-10): M3 T6b one secret source per process`.
8. **T7**: the section (below) → `validate-workflow-docs.sh`. Commit
   `docs(mod-10): M3 T7 secrets at run start`.
9. **Merged tree** (memory: verify gates yourself, `--test-threads=1`):
   - `cargo fmt --all --check`;
   - `cargo clippy --workspace --all-targets --all-features -- -D warnings`;
   - `cargo clippy --workspace -- -D warnings` (featureless, H-3);
   - `cargo test --workspace --all-features --no-fail-fast -- --test-threads=1`, with the
     SIGABRT grep (H-1); `qdrant_live` timeouts re-run serially before calling them a regression
     (memory);
   - `cargo insta test --workspace --all-features` and no pending `.snap.new` (H-6);
   - `validate-workflow-docs.sh`.
   - **Host, before `scripts/hr collect MOD-10`**: `scripts/scrub-audit.sql` (OQ-D) and, once,
     `crates/htui-secrets/tests/infisical_live.rs` (M2 carry-over). Recorded in the phase note.

**T7 content** (`docs/htui-secrets.md`, new `## At run start` after `## What htui refuses`):
when resolution happens (once per walk, at the first live step, candidate group or judge; a
resumed, retried or adopted walk resolves again; chats resolve in their own task); what is
injected (exactly the resolved map, applied last over the agent row's env; nothing else, R-SEC-2);
what refuses and how it reads (`secrets_refused: <cause>`, the table of causes from §B.3 and M2
D4, transient causes included, no retry); reserved names (`HTUI_` prefix); short values
(injected, not masked, key names logged once); newline masking; the hold-back seam and its one
residual (a credential already matching at the bound leaves its tail, not the key, in the next
row; the session fails closed); what is wiped and what is not (OQ-C: `SessionSpec.env` and the
child's environment block); the verifier boundary (D19) and the verify-output re-mask (A-11, if
kept); a slow Infisical counts against the step deadline (H-18).

---

## G. Tree facts relied on (re-verified at `f8d26e35`)

| Fact | Where |
|---|---|
| `drive_once` reads the project first and builds the one `SessionSpec`; `env: BTreeMap::new()` | `engine.rs:6005-6110`, `:6018`, `:6099` |
| Stage 3 masks, persists `trim_record` and the prompt row before `drive_once` | `engine.rs:3511-3575`, `:4229-4246`, `:5097-5112`, `:5952-5980` |
| `walk_step` fails the run on any live-path error with `failure_text` | `engine.rs:3427-3470` |
| Candidates run under one `join_all`; a candidate's error fails that candidate | `engine.rs:3965-3976`, `:4106-4143` |
| `fail_group_before_a_token(…, block)` exists for "no candidate live yet" | `engine.rs:4059-4098` |
| The judge's prompts come before its row; a session error parks | `engine.rs:4625-4720` |
| `failure_text` types only `Unmasked` today | `engine.rs:6614-6625` |
| 10 `EngineParts {` literals; `fake_parts` has 22 callers | `engine.rs:7102`, `:7464`, `:7555`, `:10257`, `:10359`, `:13271`, `:14389`; `conformance.rs:8593`; `gix_isolator.rs:92`; `runtime.rs:1037` |
| `Kit` per task, empty `MinimalScrubber`; verifier singleton with its own | `runtime.rs:885-1058`, `:969`, `:455-460` |
| `with_tool_host` on both runtimes; `spawn_hosted` hosts both | `runtime.rs:1372`, `agent_worker.rs:659`, `store_worker.rs:2168-2202` |
| `AgentRuntime::serve` is awaited inline on the store loop | `store_worker.rs:2665` |
| Chat specs with empty env; chat scrubber from `spec.env` | `agent_worker.rs:1089-1104`, `:2146-2175`, `:4139` |
| `ChatBinding::close` is a no-op for `Promoted`; `request()` names `chat_start` / `PROMOTE_STEP` | `agent_worker.rs:2386-2405` |
| Size trigger after a chunk append; one open text row per chunk kind | `record.rs:986-1000`, `:1108` |
| Two tests pin `[16384, 1024]` | `htui-agent/src/conformance.rs:3319-3355`, `htui-agent/tests/recorder.rs:1830-1862` |
| `PATTERN_RULES` minima: pypi 70, sendgrid 69, npm 40, google 39, jwt 38 | `scrub.rs:35-63` |
| `SecretError` derives `Clone, PartialEq, Eq`; 15 variants; exhaustive matches in htui-secrets tests | `secret.rs:281-390`; `infisical.rs:1718`; `infisical_live.rs:41` |
| `MemStore::set_project_settings` is the tests-only precedent | `mem.rs:617-631` |
| Keyring getters, `mock_keyring` / `mock_keyring_broken` | `htui-store/src/secret.rs:445`, `:482`; `testkit.rs:334`, `:360` |
| `InfisicalProvider::new(InfisicalConfig::new(url), identity)`, `normalise_base_url`, `KIND` | `htui-secrets/src/infisical.rs:153-199`, `:605` |
| `htui` has `sha2`; `htui-orch` has `tokio` `sync`; `htui-core` has `zeroize` and no `tokio` | the three `Cargo.toml`s |
| `FakeDriver` records its started spec (`SpecSlot`); `FakeOrchestrator::spec_for` | `htui-agent/src/fake.rs:97-166`; `htui-orch/src/fake.rs:1788` |
| `suspend_after_done` returns `(stalled, wake)` and the candidate settles with no document | `htui-orch/src/fake.rs:1982-1998`, `:2147-2175` |
