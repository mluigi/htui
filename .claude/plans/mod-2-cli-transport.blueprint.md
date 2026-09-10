# Blueprint: MOD-2 milestone 8 — the degraded CLI transport

**Plan**: `.claude/plans/mod-2-cli-transport.plan.md` (D79–D90, T49–T58 binding; its "Verified
claims" table taken as read and re-checked against the tree on `main` at `f5b37ad`, 2026-09-10 —
every row held except the ones marked **plan ≠ tree** below, and the plan carries **errors** that
go back to the maintainer before a line is written: section E-0..E-8).
**Design authority**: `docs/ANA-4.md` §4.3 (`:544-554`), §4.4 (`:631-662`), §6.2 (`:1033-1050`),
§7 (`:1077-1150`), §8 (`:1154-1237`), §11 criteria 1–8, 11 (`:1338-1369`), §11.14 (`:1372-1393`).
Where the plan and the tree disagree the tree wins and the point is marked **plan ≠ tree**; where
the plan and the *installed binary* disagree the binary wins and the point is an **error**. Anything
not verified in source is **UNVERIFIED — implementer must check**, and anything only a live run can
settle is **T55/T56 decides** and named as such.

Conventions inherited unchanged from the milestone-5/6/7 blueprints: `#![warn(missing_docs)]`,
`unsafe_code = "forbid"` (so SIGINT goes through process-wrap's safe `ChildWrapper::signal`, B.2),
MSRV 1.98, one `thiserror` type per crate (no new `DriverError` variant — B.3 picks an existing
one), **no new `DriverEvent` variant** (eleven, pinned by `tests/driver_contract.rs:284,312` — which
is exactly why D85 needs E-1), no lock across an `.await`, `htui-store` never depends on
`htui-agent`, the scrubber stays inside `Recorder`, nothing keyed on `agent.name` (`R-AGT-5`; the
sweep vocabulary is listed in H-25 — `claude` is **not** in it, so `cli/claude.rs` and
`claude_stream_json` are lawful identifiers, and every new comment is written against that list),
caps in USD micros (D70), the H-7 fixture rule (only the `*_live.rs` files may touch an unmodified
seed row), `CASES` stays at **15** names.

The Windows clippy line the plan lists (`cargo clippy --target x86_64-pc-windows-msvc …`) **does not
run on this box**: `HANDOFF.md` TOOL-3, re-confirmed by the milestone-7 blueprint H-16 (it dies in
`ring`'s build script before any `htui` code compiles). Every `#[cfg(windows)]`/`#[cfg(unix)]` line
below is reviewed by eye and named in the phase note; the runtime fact is MOD-16's (section H).

---

## E. Errors in the plan, for the maintainer

These are the findings the plan asked this step to produce. Each names the evidence, the
consequence if built as written, and the resolution this blueprint assumes. **E-0, E-1 and E-4 need
a maintainer decision; the rest are corrections the implementer applies.**

| # | Plan says | What is true | Consequence / resolution assumed below |
|---|---|---|---|
| **E-0** | D79: the `claude-cli` seed passes `extra_args: ["--bare"]`, is `billing: subscription`, `quota.source: cli_rate_limit_event`; §4.4 (written against 2.1.261) calls `--bare` "the documented recommendation for scripted callers" | `claude --help` on **this** box (2.1.267, 2026-09-10) says of `--bare`: "**Anthropic auth is strictly `ANTHROPIC_API_KEY` or apiKeyHelper via `--settings` (OAuth and keychain are never read)**", and it "Sets `CLAUDE_CODE_SIMPLE=1`". This box holds a subscription (OAuth) login and no API key (`chat_live.rs`, `agy_live.rs` run on it) | **Built as written, every live task (T55, T56, T57) fails to authenticate, and a row that is `subscription` + `cli_rate_limit_event` + `--bare` contradicts itself** — under an API key there is no subscription allowance to latch. Two coherent rows exist; the maintainer picks one: **(a) drop `--bare`** (recommended; keeps `subscription` and the rate-limit source honest; costs the supervisor a pre-`init` buffer because hook events then precede `system/init`, §4.4 `:649-650`, B.2 step 3 and H-2), or **(b) keep `--bare`** and make the row `billing: per_token`, `quota.source: none`, with the maintainer setting `ANTHROPIC_API_KEY` for the live runs. This blueprint is written for **(a)**; under (b) T55 case 2 and the pre-init buffer are dropped and the seed's two fields change. T55 case 1 runs *both* forms once, so the choice is recorded from evidence rather than from help text |
| **E-1** | D85: the transport "synthesizes `permission_answer` rows with `by: "policy"` and `denied: true`" | `permission_answer` is one of the **three kinds `htui` authors itself**: `DriverEvent` has no variant for it (`event.rs:155-184`), `driver_contract.rs:284,312` pins eleven variants, `conformance::driver_rows` filters it out as non-driver (`conformance.rs:322-331`), and every row the recorder writes from a `DriverEvent` is stamped `role: agent` (`record.rs:763,787,838`) while a policy answer must be `role: htui` (`AnsweredBy::Policy.role()`, `record.rs:218-223`; asserted at `conformance.rs:929-933`) | **Unreachable from a transport as written.** Resolution (B.5): the transport emits the denial as `DriverEvent::Other { update: "permission_denied", body }` — the vendor shape, verbatim, §6.2's own `other` rule — and **`Recorder::record` converts that one `update` into the `permission_answer` row** D85 describes (role `htui`, `by: policy`, `denied: true`, `option_id: null`, `request_id` = the tool-use id, the wire line as `raw`). One `htui`-owned constant, `record::PERMISSION_DENIED`, is the contract; `SESSION_STARTED` is the precedent for an `other.update` every transport shares. The zero-seam-change alternative — leave it an `other` row and amend D85's kind — is stated so the maintainer can take it instead; the twelfth-variant alternative is rejected for the three reasons in the middle column. **`record.rs` joins the file table (A.12).** |
| **E-2** | D86: `cost_micros` is the delta, `cost_micros_total` cumulative; the four token fields are "summed across models" | `result.total_cost_usd` is documented by the SDK as the session's cumulative cost, and `modelUsage` is a session-level map; nothing in the plan or the ANA says whether its **token** figures are per-turn or cumulative. The recorder **sums** the four token keys across rows (`usage.rs:48-54`) | If `modelUsage` tokens are cumulative, a two-turn chat double-counts tokens in `run_step.usage` on turn 2 and criterion 7 fails silently for tokens. The mapper is written **delta-for-all-five** from the previous `result`'s totals (B.4); **T55 case 4 is a two-turn run whose fixture settles it** — if the figures prove per-turn, the token deltas are switched off by one flag and the finding is recorded. Either way `cost_micros` is the delta and the sum is exact |
| **E-3** | D86: `rate_limit_event` "drives the quota latch through the existing `normalize(QuotaSource::CliRateLimitEvent, …)`" | `htui_core::model::quota::normalize` maps `CliRateLimitEvent | CliStatusLine | None => None` for the blob (`quota.rs:187-189`), and its test at `:540-541` pins that the two CLI sources publish nothing; the milestone-7 blueprint said so in its "does not touch" list ("normalize to no status, no windows until a reader exists") | Built as written, the latch fires and publishes **spend only**: no windows, no status, the Settings `quota` cell reads `$x spent` and MOD-4's skip rules never see a full window. `quota.rs` gains the `CliRateLimitEvent => raw` arm (the CLI blob is "the same rate-limit blob", §7 `:1086`) and its test flips; **`crates/htui-core/src/model/quota.rs` joins the file table (A.11)** |
| **E-4** | D80 names three capability-dependent cases | The `claude` dialect carries **cost on the terminal `result` only** — `assistant` messages carry `message.usage` tokens, never cost — so a harness cannot put a scripted `usage { cost_micros: 100 }` on the wire mid-turn without inventing a message the dialect does not have, which is the fabrication D80's own rationale forbids and the amended `usage_deltas_sum_to_step_usage` comment (`conformance.rs:1148-1153`) already refuses in the other direction. Three more cases are therefore transport-dependent: `usage_deltas_sum_to_step_usage` (asserts **3** usage rows, `:1185-1191`), `quota_blob_latches_agent_box` (asserts 3 rows and **2** latch calls, `:1338-1360`), and `run_cap_breach_cancels_within_one_event` (its (a)/(d) scripts end in `ExpectCancel` with the breach mid-turn) | **Six cases need a second arm, not three, and there is no `DriverCaps` predicate to gate the three usage cases on.** Resolution (B.1, B.6): `DriverCaps` gains `usage_mid_turn: bool` (ACP and the fake `true`, CLI `false`, `Default` false), and the three usage cases assert the turn-end form: **one** `usage` row per turn whose `cost_micros` equals the sum of the script's deltas and whose `cost_micros_total` equals the same number (criterion 7 in its strongest form), **one** latch call carrying the blob, and a breach detected on the turn's `usage` row with the transport's own `done` withheld (H-4 of milestone 7, now exercised by a real transport). This is a seam change (**D91 candidate**) and needs the maintainer's yes |
| **E-5** | D84: the banner sources `protocol_version: null` | `session_banner_is_first_other_row` asserts `body.protocol_version == json!(1)` — "the banner reports the negotiated protocol version" (`conformance.rs:1986-1993`) | The CLI binding fails the case as written. The assertion becomes "the key is present, and is `null` or a positive integer" (B.6) — both existing bindings unchanged in behaviour, and a transport that *forgot* the key still fails |
| **E-6** | Validation lines: `cargo test -p htui --features demo,test-support …` (T49, T49b, T57) | `crates/htui/Cargo.toml:19-23` declares one feature, `testkit`; `demo` is a `htui-core`/`htui-store` feature already on through the dependency lines | The lines are `cargo test -p htui --features testkit …` — the same P-6 the milestone-6 blueprint caught |
| **E-7** | D89's stated price is `unauthenticated` → `unauthenticat` | `choose a method` is **also** 15 (`agents.rs:1040`; `tests/settings.rs:924`, `:1896`), and at 13 it renders **`choose a meth`** — a word that reads as something else. The `tests/settings.rs` position constants `NAME_WIDTH = 8`, `DEFAULT_AT = 39`, `QUOTA_AT = 69`, `ON_BOX_AT = 83`, `ON_BOX_WIDE` (`:412-432`) all move, and `the_on_box_column_holds_the_whole_unauthenticated_verdict` (`:565-590`) **inverts** rather than re-accepts | Not a solver error (section C confirms the arithmetic) but an **understated price**; the maintainer's override clause in D89 is the place to name a different cell text (`pick a method` is 13, and would leave only `unauthenticated` clipped). This blueprint keeps D89 as confirmed and lists every moved assertion (F-T49b) |
| **E-8** | Smaller factual slips, corrected in place | (i) T54: "filtered to `claude-p`" — the row is `claude-cli` (D79). (ii) D90: "construction sites … `fake.rs`, `acp/`" — neither constructs a `SessionSpec`; the six sites are `conformance.rs:250`, `extensibility.rs:491`, `driver_contract.rs:133`, `acp_driver.rs:89`, `agy_live.rs:799`, `agent_worker.rs:1144`, plus the hand-written `Debug` at `driver.rs:276-292`. (iii) File table: `htui-store/src/pg/demo.rs` needs **no** change — it deletes `data.agents.iter().map(name)` (`demo.rs:141`), which follows the fixture. (iv) T50: stderr "captured to the run log as the ACP path does" — the ACP path keeps a 64-line tail on `Spawned` and appends it to an error (`launch.rs:574-584`); there is no run log. (v) `uuid` is not a dependency of `htui-agent` (`crates/htui-agent/Cargo.toml`), so D84's mint is one manifest line (A.1). (vi) `fixtures::agents()` zips `seed_rows` with a **two**-element id array (`fixtures.rs:394-398`), so a third seed is **silently dropped** by `zip` unless `ids::AGENT_CLAUDE_CLI` is added (H-10) | Applied in the tables below |

---

## Plan ≠ tree, resolved in the tree's favour

| # | Plan says | Tree says | Resolution |
|---|---|---|---|
| P-1 | Patterns: `ChildGuard` and `run_bounded` own the child "on every exit path" | True, and the ACP session task's shape is `Arc<Mutex<ChildGuard>>` swapped out under the lock and reaped outside it (`acp/mod.rs:977`, `:1736-1742`) | The CLI task copies that exact shape (B.2); no new guard type |
| P-2 | T50: "`send_follow_up` writing one NDJSON user message per line to stdin"; §6.2 row 1: the prompt is "argv or the first stdin line" | The conformance harness has **no argv** (`AcpDriver::over` takes an in-process pair, `acp/mod.rs:474-482`), and a prompt on argv is visible in `ps` on the box | **The prompt always travels on stdin**, as the first user message, and never on argv (B.2 step 2). T55 case 1 proves `-p` with no positional prompt accepts it |
| P-3 | T50: the banner "with the same key set milestone 3 established" | `acp/mod.rs:1226-1241` — `session_id`, `protocol_version`, `agent_name`, `agent_version`, `models`; the fake writes the same five (`fake.rs:198-207`) | Same five keys, sourced per B.2 step 3; `protocol_version: null` under E-5 |
| P-4 | T50/T51: `Stamp` and the `AcpIo` pair are "mirrored" | Both are defined in `acp/mod.rs` (`:112-134`, `:144-176`); a `cli` module importing `crate::acp::Stamp` would make the second transport depend on the first | `Stamp`, `SESSION_STARTED` and `TRANSPORT_CLOSED` move to `event.rs`; `AcpIo` moves to `launch.rs` as `ChildIo`. Every existing path keeps resolving through `pub use … as` in `acp/mod.rs` and `fake.rs` (P-6 of milestone 7 is the pattern; the twelve `AcpIo` users and four `Stamp` users listed in A.3 move nothing) |
| P-5 | T50: `--max-budget-usd` "from `SessionSpec.budget_micros`" | `start()` computes `project_caps` **before** it builds the spec (`agent_worker.rs:1114-1128` then `:1144`), so the value is in scope at the literal | `budget_micros: project_caps.run_micros` at `:1144`; no plumbing |
| P-6 | T55 "independent"; T54 numbered before T55 | T54's scripted agent has to answer a stdin close the way the real binary does (an interrupted `result`, or EOF), and only T55 says which | **T55 → T56 run first**, before T51/T50/T54 (section E build order). They are independent of T49 and of the seam lifts, not of the CLI code |
| P-7 | T53: "`run_case` reads `driver.caps()`" | `run_case` never sees the driver; `open_case` builds it and returns `(chat, session)` (`conformance.rs:269-292`) | `open_case` returns `(chat, session, caps)` with `caps = driver.caps()` read before `start`; `run_case`'s signature is unchanged |
| P-8 | Risk table: "`which` with `PATHEXT` is already the resolver" | `launch.rs:829` — `which::which` on `spawn_blocking`; `claude` on Windows is a `.cmd` shim which `std::process::Command` runs through `cmd.exe` since Rust 1.77 | Unchanged; MOD-16's runtime fact (section H) |
| P-9 | T57 mirrors `chat_live.rs` and greps `pgrep -f` | `chat_live.rs:158-167` greps `claude-agent-acp`; a `pgrep -f claude` would match the test binary `chat_live_cli` itself | The pattern is the argv the CLI alone carries: `--output-format stream-json` (F-T57) |

---

## A. Per-file change table

| # | File | Action | Task | What changes (and what must **not**) |
|---|---|---|---|---|
| 1 | `crates/htui-agent/Cargo.toml` | UPDATE | T50a | `uuid = { workspace = true }` (E-8 v). The workspace pins `v7` + `serde` (`Cargo.toml:15`); the mint is `Uuid::now_v7()`, the same call `htui_core::model::ids` makes (`ids.rs:32`). H-12 |
| 2 | `crates/htui-agent/src/event.rs` | UPDATE | T50a | `Stamp` (moved from `acp/mod.rs:106-134`, unchanged), `SESSION_STARTED`, `TRANSPORT_CLOSED` (P-4). **Not**: any `DriverEvent` variant |
| 3 | `crates/htui-agent/src/launch.rs` | UPDATE | T50a, T50 | T50a: `ChildIo` (was `AcpIo`, `acp/mod.rs:144-176`, unchanged body; `from_spawned` with it). T50: `Spawned::interrupt()` (`#[cfg(unix)]`, B.2), `pub(crate) async fn launch_from(launch, recorded, cwd)` lifted from `AcpDriver::launch_in` (`acp/mod.rs:394-419`) so D58's rules have one body. `ChildGuard`'s doc (`:722-730`) gains the fourth holder |
| 4 | `crates/htui-agent/src/acp/mod.rs` | UPDATE | T50a | `pub use crate::event::{SESSION_STARTED, Stamp, TRANSPORT_CLOSED}; pub use crate::launch::ChildIo as AcpIo;` in place of the definitions; `launch_in` becomes a call to `launch_from`. **Not**: the session task, cancel sequence, mapper |
| 5 | `crates/htui-agent/src/fake.rs` | UPDATE | T50a, T53 | T50a: `pub use crate::event::SESSION_STARTED;` replaces `:55`; `full_caps` gains `usage_mid_turn: true`. T53: the `ScriptEvent::PolicyDenied` arm (B.6) |
| 6 | `crates/htui-agent/src/driver.rs` | UPDATE | T50a | `SessionSpec.budget_micros: Option<i64>` after `resume` (`:273`) + the `Debug` field (`:290`); `DriverCaps.usage_mid_turn` after `usage` (`:333`), B.1 |
| 7 | `crates/htui-agent/src/registry.rs` | UPDATE | T50a, T52 | T50a: `caps_from` — `usage_mid_turn: true` in the ACP arm, `false` in the CLI arm, with the §7 sentence as the comment. T52: `with_acp` → `production`, registers `crate::cli::ADAPTER_ID`; the doc at `:60-63` rewritten past tense |
| 8 | `crates/htui-agent/src/cli/mod.rs` | CREATE | T50 | B.2: `ADAPTER_ID`, `STREAM`, `ClaudeStreamAdapter`, `CliDriver`, `IoSource`, `SessionOptions`, `Command`, `CliSession`, `open_session`, `run_session`, `TaskState`, `argv`, `usd`, `stdin_line`, the pre-init buffer, cancel per D81 |
| 9 | `crates/htui-agent/src/cli/claude.rs` | CREATE | T51 | B.4: `Mapper`, `kind_of`, `update_name`, `body_of`, `tool_kind`, `stop_reason`, `locations_of`, `output_of`, `denials_of`. Imports `serde_json`, `crate::event`, `crate::launch::UsageScope` and nothing that names a process |
| 10 | `crates/htui-agent/src/lib.rs` | UPDATE | T50 | `pub mod cli;` after `auth` (`:87`); `pub use cli::{ADAPTER_ID as CLI_ADAPTER_ID, ClaudeStreamAdapter, CliDriver, CliSession};` beside the `acp` re-exports (`:104`) |
| 11 | `crates/htui-core/src/model/quota.rs` | UPDATE | T51 | E-3: `QuotaSource::AcpMetaRateLimit | QuotaSource::CliRateLimitEvent => raw` at `:187-189`; the doc at `:168-176` names both; the in-module test at `:540-541` moves `CliRateLimitEvent` to the publishing side and keeps `CliStatusLine` on the empty side (no reader exists for a status line) |
| 12 | `crates/htui-agent/src/record.rs` | UPDATE | T53 | E-1 / B.5: `pub const PERMISSION_DENIED`; the conversion arm in `record` (`:733` neighbourhood, before the chunk logic); `record_policy_denial` (private). Module doc item list gains one line. **Not**: `record_permission_answer`'s signature, `pump`, `enforce_breach` |
| 13 | `crates/htui-agent/src/conformance.rs` | UPDATE | T53 | B.6: `ScriptEvent::PolicyDenied(String)`; `open_case` returns caps (P-7); the six two-arm cases; `session_banner_is_first_other_row`'s `protocol_version` assertion (E-5). **Not**: `CASES` (15), `run_case`'s signature, any case name |
| 14 | `crates/htui-agent/tests/acp_conformance.rs` | UPDATE | T53 | `wire_update`'s panic arm gains `PolicyDenied` (`:392-394`); count guard stays 15 |
| 15 | `crates/htui-agent/tests/cli_conformance.rs` | CREATE | T54 | B.7: `CliHarness`, `scripted_cli`, `wire_lines`; third binding |
| 16 | `crates/htui-agent/tests/cli_map.rs` + `tests/snapshots/cli_map__*.snap` | CREATE | T51 | F-T51: one `insta` debug snapshot per fixture, the `acp_map.rs:24-42` reading shape over `{"direction","line"}` records |
| 17 | `crates/htui-agent/tests/cli_live.rs` | CREATE | T55, T56 | F-T55, F-T56: `#[ignore]`, drives the binary **directly** (`launch::spawn` over `tools::resolve` of the seed row), records fixtures, asserts no survivor by pid |
| 18 | `crates/htui-agent/tests/fixtures/claude_stream_json_{turns,sigint,sigterm,stdin_close,thinking,subagent,budget,denied}.jsonl` | CREATE | T55, T56 | Recorded, redacted by hand (session ids → `<session>`, `$HOME` → `~`, `cwd` → `/scratch`) |
| 19 | `crates/htui-agent/tests/{driver_contract,acp_driver,auth_live}.rs`, `crates/htui/tests/auth.rs`, `crates/htui/src/agent_worker.rs:365,5353` | UPDATE | T52 | D87: the eight `with_acp` sites (verified: `registry.rs:65` def; `acp_driver.rs:664`; `auth_live.rs:820,944` and its comment `:817`; `driver_contract.rs:426,438`; `auth.rs:263`; `agent_worker.rs:365,5353`) |
| 20 | `crates/htui-agent/tests/extensibility.rs` | UPDATE | T52 | New test `the_production_factory_holds_two_adapters_for_three_rows`: `DriverFactory::production().adapter_ids() == ["acp", "cli/claude_stream_json"]` and `seed_rows(..).len() == 3`. `:177` untouched. `every_seed_row_deserialises_into_the_launch_types` already passes for the new row (`settings.acp` defaults; `install.is_some() == (name == "agy")` holds) |
| 21 | `crates/htui-core/seeds/agent_claude_cli.json` | CREATE | T49 | Section C.1 |
| 22 | `crates/htui-core/seeds/agent_claude.json` | UPDATE | T49 | The `cli` block (`:34-35`) removed; nothing else |
| 23 | `crates/htui-core/src/model/agent.rs` | UPDATE | T49 | Third `include_str!` (`:131-134`); doc "two `agent` rows" → three (`:104`, `:112`); `seed_rows_match_ana4_5_3` (F-T49) |
| 24 | `crates/htui-core/src/fixtures.rs` | UPDATE | T49 | `ids::AGENT_CLAUDE_CLI: AgentId = (class::AGENT, 2)` after `:122`; `agents()` zips three ids (H-10); the `:118` doc line |
| 25 | `crates/htui-store/src/pg/mod.rs` | UPDATE | T49 | D88: the `.filter(|_| empty)` at `:299` goes; the doc at `:223-231` rewritten: the empty-table guard was for edits, and `ON CONFLICT (name) DO NOTHING` already protects those — a top-up inserts only names the table lacks. Keep the function name; a separate `seed_missing_agents` would be a second loop over the same rows |
| 26 | `crates/htui/src/agent_worker.rs` | UPDATE | T50a, T52 | T50a: `budget_micros: project_caps.run_micros` at `:1144-1164` (P-5) with a two-line D83/D90 comment. T52: the two renames. **Not**: `run_turn`, `run_chat` — a session that never parks needs nothing from the worker (`run_turn`'s `parked` stays `None` for the whole turn, `:2470-2604`) |
| 27 | `crates/htui/src/ui/tabs/settings/agents.rs` | UPDATE | T49b | `Constraint::Min(10)` first, `Constraint::Length(13)` last (`:1070-1077`); the comment block `:1021-1066` rewritten per D89 (section C.3); `render_table`'s doc "eight-column" unchanged |
| 28 | `crates/htui/tests/settings.rs` | UPDATE | T49b | F-T49b: five constants, one inverted test, one new test, the `as_drawn` expectations |
| 29 | `crates/htui/tests/snapshots/settings__agents_{demo,empty,probed,quota,unknown_row}.snap`, `probe__agents_probed_missing.snap` | RE-ACCEPT | T49, T49b | A third row (`claude-cli cli subscription 0 — yes — not probed`) and the D89 widths. Reviewed line by line: only the `name`/`on this box` widths and the new row may differ |
| 30 | `crates/htui/tests/chat_live_cli.rs` | CREATE | T57 | F-T57 |
| 31 | `README.md`, `HANDOFF.md:77`, `.claude/prds/mod-2-agent-driver-chat.prd.md:157,186`, this plan | UPDATE (docs) | T58 | README: a `### The CLI row` subsection under `### Chat tab` (`:190`) — what it cannot do, the banner, `--max-budget-usd`, and E-0's auth consequence; HANDOFF phase-8 note carrying D79/D88's ANA-4 amendments **and** E-0's `--bare` finding as an ANA §4.4 amendment; PRD row 8 `complete`, `:186`'s open items closed |

---

## B. Interfaces, exactly

### B.1 The seam additions (T50a; E-4, D90)

`driver.rs`, `DriverCaps` after `usage` (`:333`):

```rust
/// The transport reports usage **while a turn is open**, so a cap can be reached mid-turn and a
/// `usage` row can carry a rate-limit blob before the turn's `done`. `false` means one report per
/// turn, on the message that ends it — the `claude` CLI's shape, where cost exists only on
/// `result` (`docs/ANA-4.md` §7, plan E-4): the recorder's per-run cap then fires on the turn's
/// closing row and cancels a turn that has already ended, and the conformance suite asserts the
/// turn-end form of criterion 7 rather than the per-report one.
pub usage_mid_turn: bool,
```

`SessionSpec` after `resume` (`:273`):

```rust
/// `project.settings.per_token_cap_run` in USD micros, when the project sets one (plan D90). The
/// recorder enforces the same figure client-side (D70); a transport that has a server-side knob
/// passes it too, so the two caps read **one** number — `claude --max-budget-usd` (D83). ACP has
/// no such knob and ignores it.
pub budget_micros: Option<i64>,
```

`registry.rs` `caps_from`: ACP arm `usage_mid_turn: true` ("`usage_update` arrives whenever the
adapter reports, §3"); CLI arm `usage_mid_turn: false` with the §7 sentence. `FakeDriver::full_caps`
`usage_mid_turn: true`. The six `SessionSpec` literals gain `budget_micros: None` except
`agent_worker.rs:1144` (P-5).

### B.2 `crates/htui-agent/src/cli/mod.rs` (T50; D81, D83, D84)

Module doc, first paragraph: the second transport of `docs/ANA-4.md` §8, over the `claude` CLI's
headless NDJSON stream (§4.4, §6.2). Shape copied from `crate::acp`: one task per session owning the
child through a `ChildGuard`, a handle holding channel endpoints and nothing else, and a mapper
(`claude.rs`) that is pure. What differs is stated once: there is no protocol layer — a line in is a
JSON value, a line out is a user message — so there is no handshake beyond the first `system/init`,
no permission channel (§4.3: `DriverCaps { permission_requests: false, … }`), and cancellation is a
signal, not a notification.

```rust
/// The adapter id this transport registers under (plan D12): `cli/<settings.cli.stream>`.
pub const ADAPTER_ID: &str = "cli/claude_stream_json";
/// The `settings.cli.stream` value that selects it — half of [`ADAPTER_ID`], and what a registry
/// row declares. A dialect name, not an agent name (`R-AGT-5`): two rows may declare it.
pub const STREAM: &str = "claude_stream_json";
/// How long the first `system/init` may take. The CLI's login refusal prints to stderr and exits,
/// which is EOF and is reported at once; this bounds an agent that hangs before saying anything.
pub const INIT_TIMEOUT: Duration = Duration::from_secs(60);
/// Depth of the session task's event channel; `crate::acp::EVENTS_CAPACITY`'s reason.
pub const EVENTS_CAPACITY: usize = 256;
/// The grace a dropped handle gives the child; `crate::acp::DROP_GRACE`'s reason.
pub const DROP_GRACE: Duration = Duration::from_secs(1);
/// `SIGINT`'s POSIX number. A literal rather than `libc::SIGINT` because `libc` is not a direct
/// dependency and `unsafe_code = "forbid"` rules out calling it anyway; process-wrap's
/// `ChildWrapper::signal(i32)` does the `killpg` (`process_group.rs:113`, `:230`).
#[cfg(unix)]
const SIGINT: i32 = 2;

/// The [`TransportBuilder`] registered under [`ADAPTER_ID`].
#[derive(Debug, Default, Clone, Copy)]
pub struct ClaudeStreamAdapter;
impl TransportBuilder for ClaudeStreamAdapter {
    fn build(&self, agent: &AgentRow, on_box: Option<&AgentBox>, caps: DriverCaps) -> Result<Box<dyn AgentDriver>> {
        Ok(Box::new(CliDriver::from_row_with_probe(agent, on_box, caps)?))
    }
}

/// The driver for a `cli` row whose stream is [`STREAM`]: one per row, holding no process.
pub struct CliDriver {
    name: String,
    settings: AgentSettings,        // `settings.cli` is what `argv` reads; `settings.usage.scope` is the mapper's
    caps: DriverCaps,
    io: IoSource,                   // the `acp::IoSource` shape: `Spawn { launch, recorded }` | `Prepared(Mutex<Option<Box<ChildIo>>>)` under `test-support`
    stamp: Stamp,
    /// `agent_box.version` — the probe's `claude --version` capture — the banner's `agent_version`
    /// fallback when `system/init` names no version (B.2 step 3).
    box_version: Option<String>,
}
// hand-written Debug: name, caps, stamp, `recorded: bool` — exactly `AcpDriver`'s (`acp/mod.rs:270-290`)

impl CliDriver {
    pub fn from_row(agent: &AgentRow, caps: DriverCaps) -> Result<Self>;                                  // = from_row_with_probe(agent, None, caps)
    pub fn from_row_with_probe(agent: &AgentRow, on_box: Option<&AgentBox>, caps: DriverCaps) -> Result<Self>;
    //   `agent.launch` parses or `DriverError::Transport("agent.launch does not parse: …")`;
    //   `recorded` = on_box → ProbeSnapshot::from_row → filter(transport == agent.transport) → recorded_launch().cloned()
    //   (byte-for-byte `acp/mod.rs:344-364`; a `cli` row's snapshot is `Ready`/`Probe` with `resolved: Some`, `probe.rs:1452-1459`,
    //   so D58's rules admit it); `box_version = on_box.and_then(|b| b.version.clone())`.
    pub async fn launch_for(&self, spec: &SessionSpec) -> Result<ResolvedLaunch>;                          // `launch::launch_from(launch, recorded, &spec.cwd)` then `env.extend(spec.env)`
    #[cfg(feature = "test-support")]
    pub fn over(io: ChildIo, agent: &AgentRow, caps: DriverCaps, stamp: Stamp) -> Self;                   // the harness constructor; `box_version: None`
}

/// The argv of `docs/ANA-4.md` §4.4, assembled from the row and the spec. Pure and unit-tested.
///
/// Order: the row's own resolved `args` first (the seed has none), then the fixed flags, then the
/// row's mode, then the session id **or** the resume id (exactly one — `--session-id` mints,
/// `--resume` continues, and the CLI refuses both), then the spec's model, directories and budget,
/// then `settings.cli.extra_args` **last** so an operator's flag wins a repeated one.
pub fn argv(row_args: &[String], cli: &CliSettings, spec: &SessionSpec, session_id: &str) -> Vec<String>;
//   row_args…, "-p", "--output-format", "stream-json", "--input-format", "stream-json", "--verbose",
//   "--include-partial-messages",
//   ["--permission-mode", cli.permission_mode] if !cli.permission_mode.is_empty(),
//   spec.resume.map_or(["--session-id", session_id], |r| ["--resume", r.as_str()]),
//   ["--model", m] if spec.model, ["--add-dir", d] per spec.extra_dirs (lossy display),
//   ["--max-budget-usd", usd(b)] if spec.budget_micros, cli.extra_args…

/// USD micros as the decimal `--max-budget-usd` takes: integer arithmetic, six places, no float.
/// `300` → `"0.000300"`, `1_500_000` → `"1.500000"`. A negative value never reaches here
/// (`ProjectCaps::from_settings` refuses it); `debug_assert!`ed.
pub fn usd(micros: i64) -> String;   // format!("{}.{:06}", micros / 1_000_000, micros % 1_000_000)

/// One `--input-format stream-json` user message as a line. The `session_id` is included because
/// the scripted agent of the conformance harness has no argv to read it from (P-2) and the SDK
/// includes it too. **T55 case 1 fixes the minimal accepted shape**; this is the hypothesis.
fn stdin_line(text: &str, session_id: &str) -> String;
//   {"type":"user","message":{"role":"user","content":[{"type":"text","text":<text>}]},"session_id":<id>}\n

/// What the handle asks the task to do. No `AnswerPermission`: there is no request to answer.
pub enum Command { FollowUp(String), Cancel { grace: Duration, done: oneshot::Sender<()> } }

/// A live CLI session: channel endpoints and handle-side bookkeeping only.
pub struct CliSession {
    session_ref: AgentSessionRef,               // the minted id (D84)
    events: mpsc::Receiver<DriverEnvelope>,
    commands: mpsc::UnboundedSender<Command>,
    pending: VecDeque<DriverEnvelope>,          // drained while `cancel` waited, served first
    turn_open: bool,                            // false between a handed-out `done` and the next accepted follow-up
    ended: bool,
    task: Option<JoinHandle<()>>,
}
```

**`AgentSession`'s five obligations, and how a stream with no permission channel meets them:**

| Method | Behaviour | Against `DriverError` |
|---|---|---|
| `session_ref` | `Some(&self.session_ref)` — the id `htui` minted and passed as `--session-id`, which is what a later `--resume` takes (D84). If `system/init.session_id` disagrees, the task logs `warn!` and the banner still carries **ours** (H-17) | — |
| `next_event` | `pending` first; `Ok(None)` once `ended`; else `events.recv()`, noting `Done` → `turn_open = false`. No parked check: nothing can park | `Transport` only from the task's own error rows; never from the handle |
| `send_follow_up` | `Closed` if ended; `Transport("a follow-up must have text")` on empty; `Transport("a follow-up before the turn's done would interleave two turns")` while `turn_open`; else `Command::FollowUp`, `turn_open = true` — byte for byte `acp/mod.rs:648-667`, which is what `done_precedes_next_follow_up` asserts | `Closed`, `Transport` — the trait's own two |
| `answer_permission` | `Closed` if ended; otherwise **`Err(DriverError::Unsupported("answer_permission"))`**. Chosen over `Transport("no parked request …")` because the latter says the *id* is unknown while the truth is that the *operation* does not exist on this transport — which is precisely what `Unsupported` was added for (`error.rs:36-40`: "the name is the trait method's, so … the runtime can branch on the variant rather than matching on a message"), and `caps.permission_requests == false` is the predicate that pairs with it, the same pairing `authenticate`/`caps.authenticate` already has. `Closed` first because the trait's contract for every operation is "`Closed` once the session has ended" (`driver.rs:405,411`), and a closed session should not claim to lack an operation it never had the chance to refuse. The worker never calls it for a CLI session (`run_turn`'s `parked` stays `None`); the conformance suite calls it only under the `permission_requests` arm. The trait doc gains one line naming `Unsupported` | `Closed`, then `Unsupported` |
| `cancel` | If ended, `Ok(())`. Else send `Command::Cancel { grace, done }`, drain `events` into `pending` until the ack (the `acp/mod.rs:702-713` loop), then join the task so "cancel returned" means "the tree is gone" (criterion 11). **Not** `ended = true` here — the cancel's own rows are still in the channel (`:715-718`'s reason) | `Ok` even when the kill path was taken; a `warn!` names it — the same reading `enforce_breach` gives a failed cancel (`record.rs:1711-1713`) |

**The task**, `run_session(io, spec, prompt, options, ready, events, commands)`, owns
`Arc<Mutex<ChildGuard>>` exactly as `acp::run_session` does (`:977`), and a `kill(&child)` that swaps
the guard out under the lock (`:1736-1742`) — copied, not shared, because the two are three lines and
a shared helper would take a `Mutex<ChildGuard>` parameter that names neither transport's task. The
reader is `BufReader::new(reader)` driven by `read_until(b'\n')` + `from_utf8_lossy` with `\r`
stripped — the stderr reader's rule (`launch.rs:852-878`) for the same reason: one stray byte must
not end the stream (H-23). Steps, in order:

1. **Spawn** (`Spawn` arm): `launch_for(&spec)` → `resolved.args = argv(&resolved.args, &settings.cli, &spec, &session_id)` → `launch::spawn(&resolved, &spec.cwd)` → `ChildIo::from_spawned`. The id is minted **before** the spawn (`Uuid::now_v7().to_string()`), on the session task, once per `start`. A `Prepared` pair skips the argv (P-2) and mints all the same.
2. **The prompt goes out first**, as `stdin_line(&prompt, &session_id)`. A write failure is `Transport` with the stderr tail, answered to `ready`, then `kill`.
3. **Wait for `system/init`** under `options.init_timeout`, buffering every earlier line in `TaskState.pre_init: Vec<Value>` (hook events precede `init` without `--bare`, §4.4 `:649-650`, E-0). On `init`: the banner is emitted **first** — `Other { update: SESSION_STARTED, body }` with `raw = the init line` — then the buffered lines go through the mapper in arrival order (they land as `other` rows *after* the banner, which is what `session_banner_is_first_other_row` measures). The banner's keys and sources:

   | Key | Source |
   |---|---|
   | `session_id` | the minted id (D84); `init.session_id` compared and logged (H-17) |
   | `protocol_version` | `null` — the stream has none (E-5) |
   | `agent_name` | `options.agent_name` — the row's name; the stream carries no agent identity |
   | `agent_version` | `init.claude_code_version` if the fixture shows the key (**T55 decides**; read through a one-element candidate list the fixture fixes), else `options.box_version` (the probe's `--version` capture), else `""` — the same three-way fallback shape `acp/mod.rs:1231-1238` uses |
   | `models` | `init.model` as a one-element list when present, else the row's `models` (D84 said "the configured row"; the stream's own answer is more honest and the row's is the fallback — recorded as a D84 clarification, not a reversal) |

   EOF before `init` → `Err(Transport(stderr tail))` (this is where an unauthenticated box's refusal
   surfaces, E-0); timeout → `task.abort()` + await, exactly `open_session`'s arm 1 (`acp/mod.rs:894-917`).
4. **Answer `ready`**, open turn 0 (`turn_open = true` was set before the prompt went out, so a
   failure between 2 and 4 still owes a `done` — `acp/mod.rs:1287-1299`'s rule).
5. **The loop**: `select!` over the next line, `commands.recv()`. A line → `claude::Mapper::map` →
   each event through `emit` (the `acp::emit` rules copied: a `ToolResult` for a settled call is
   dropped; `note` keeps `open_calls`/`settled_calls`; `envelope` stamps `at` and synthesizes
   `raw` for rows with no line under `retain_raw`, `acp/mod.rs:1083-1097`). A `Done` from the mapper
   → `turn_open = false`. `Command::FollowUp` → `turn_open = true`, write `stdin_line`; failure →
   `error{TRANSPORT_CLOSED}` + `close_turn(Cancelled)`. **EOF with `turn_open`** → `error{TRANSPORT_CLOSED, stderr tail}`
   + `close_turn(Cancelled)` (synthesized `failed/cancelled` results for `open_calls`, then exactly one `done`) — the
   milestone-3 defect class, closed the same way `Step::Closed` closes it (`acp/mod.rs:1374-1386`). EOF between turns → `ended`.
6. **Cancel** (D81), `Command::Cancel { grace, done }`:
   1. drop the stdin writer (end of input, §4.4);
   2. `#[cfg(unix)]` `child.interrupt()` — `Spawned::interrupt(&self) -> Result<()>` = `self.child.signal(SIGINT)`, which is `killpg` on the group leader (`process_group.rs:113`); on Windows this step is absent (no `SIGINT`; the trait method itself is `#[cfg(unix)]`, `core.rs:227-230`);
   3. **the grace window is a drain**: keep reading lines until a `result` line (→ the mapper's own `done`, recorded as the turn's real end, exactly as the ACP grace step records a `StopReason`, `acp/mod.rs:1689-1713`), or EOF, or the deadline. Whatever arrives is emitted in order. This is the ordering that keeps "a stream ending before its `done`" from being recorded as a finished turn (`682a423`): the synthesized `done` is written **only after** the read side is exhausted or timed out, never before;
   4. `close_turn(Cancelled)` if the turn is still open (no-op otherwise: cancelling between turns ends the session, not a turn — `fake.rs:422-432`'s rule);
   5. `kill(&child)` (tree kill + reap), then `done.send(())`, return.
   A dropped handle (`Command(None)`) runs the same with `DROP_GRACE`. **What `result` says after SIGINT is T55 case 3's finding** (`subtype`, `is_error`, exit status); the mapper's `stop_reason` table (B.4) is written from it, and step 3's "a `result` is the real end" holds whatever it says.

**Windows, compile-checkable from Linux**: `interrupt()` and `SIGINT` are `#[cfg(unix)]`; step 6.2
is inside a `#[cfg(unix)]` block with a one-line `#[cfg(windows)]` comment stating the omission;
nothing else in the module is platform-specific. The `.cmd` shim, the job-object kill of a
`node`-hosted CLI, and CRLF on the wire are MOD-16's runtime facts (section H).

### B.3 `error.rs` — nothing

No variant. A malformed line is not an error: it lands in `other { update: "<unparsed>", body: { "line": <text> } }`
(B.4), because §6.2's wildcard rule is "stored verbatim", and a transport that refused a line the
vendor added would be the drift the fixtures exist to catch. `answer_permission` uses `Unsupported`
(B.2). The plan's "only if a new named cause is needed" resolves to "none is".

### B.4 `crates/htui-agent/src/cli/claude.rs` (T51; D82, D85, D86)

Module doc: §6.2 as code, one stdout line in, zero or more `DriverEvent`s out; JSON read by key,
never a typed decode, for `acp/map.rs:1-14`'s reason. Carried state is listed in one sentence: the
current message id, which messages streamed, the previous `result`'s five totals, and a rate-limit
blob waiting for the turn's `usage` row.

```rust
/// `other.update` of a denial the CLI applied itself; `Recorder` turns it into the
/// `permission_answer` row of `docs/ANA-4.md` §6.2 (plan D85 as amended by blueprint E-1).
pub use crate::record::PERMISSION_DENIED;

/// Per-session mapping state.
#[derive(Debug)]
pub struct Mapper {
    scope: UsageScope,                        // `agent.settings.usage.scope`: what the token sum means (`usage_scope` key)
    message: Option<String>,                  // `message_start`'s `message.id`: the chunk grouping key (§4.1)
    streamed: BTreeSet<String>,               // ids whose text/thinking already arrived as deltas (H-8)
    last: Totals,                             // the previous `result`'s five figures, for the deltas (E-2)
    pending_quota: Option<Value>,             // the last `rate_limit_event` blob, attached to the next `usage`
    tokens_are_cumulative: bool,              // E-2's flag; `true` is the hypothesis, T55 case 4 confirms or flips it
}
#[derive(Debug, Default, Clone, Copy)] struct Totals { cost_micros: i64, input: i64, output: i64, cache_read: i64, cache_write: i64 }

impl Mapper {
    pub fn new(scope: UsageScope) -> Self;
    /// One line → events. `system/init` is the supervisor's (the banner) and maps to `other`
    /// here so a fixture replay shows it; a `user` line with no `tool_result` block maps to
    /// nothing (§6.2 rows 1–2: the prompt and follow-up rows are `htui`'s).
    pub fn map(&mut self, line: &Value) -> Vec<DriverEvent>;
}
/// `(type, subtype)` of a line, `("<missing>", None)` when there is no `type`.
pub fn kind_of(line: &Value) -> (&str, Option<&str>);
/// `other.update` text: `type`, or `type/subtype` when there is one — `system/init`, `rate_limit_event`.
pub fn update_name(line: &Value) -> String;
/// The line minus `type` and `subtype`: §6.2's verbatim body.
pub fn body_of(line: &Value) -> Value;
/// §6.2's tool-name → kind table. The names are the dialect's tool vocabulary (a wire fact of the
/// stream this file parses), not an agent's name; unknown → `Other`.
pub fn tool_kind(name: &str) -> ToolKind;
//   Read → Read; Edit | Write | MultiEdit | NotebookEdit → Edit; Bash → Execute; Glob | Grep → Search;
//   WebFetch | WebSearch → Fetch; _ → Other   (TodoWrite → Other: §6.2 says `plan` is not produced)
/// `result.subtype` (+ `is_error`) → `done.stop_reason`. **T55/T56 decide the rows marked ?**.
pub fn stop_reason(subtype: Option<&str>, is_error: bool) -> StopReason;
//   "success" → EndTurn; "error_max_turns" → MaxTurnRequests; "error_max_budget_usd"? → EndTurn (the
//   error row says why); "error_during_execution"? after SIGINT → Cancelled; anything else → EndTurn
/// `input.file_path | path | notebook_path` → one `ToolLocation`; nothing else is a path.
pub fn locations_of(input: &Value) -> Vec<ToolLocation>;
/// A `tool_result` block's `content`: a string as is, an array's text blocks joined by `\n`.
pub fn output_of(content: &Value) -> Option<Value>;
/// `result.permission_denials[]` → one `Other { update: PERMISSION_DENIED, body }` each, body
/// `{ tool_call_id: tool_use_id, tool_name, tool_input }` (the vendor keys renamed to the row's
/// column names, and nothing dropped).
pub fn denials_of(result: &Value) -> Vec<DriverEvent>;
```

The `map` table (the rows of §6.2, in the order the mapper checks them):

| Line | Events | Rule |
|---|---|---|
| `stream_event` / `message_start` | none | `message = message.id` |
| `stream_event` / `content_block_delta` `text_delta` | `AssistantChunk { text, message_id }` | `streamed.insert(id)` |
| `stream_event` / `content_block_delta` `thinking_delta` | `ThoughtChunk { thinking, message_id }` | D82; `signature_delta`, `input_json_delta` → none (the full `tool_use` comes on `assistant`) |
| `stream_event` (any other) | none | `message_stop`, `content_block_start/stop`, `message_delta` — bookkeeping, not rows; **the whole line is still the `raw` of whatever it produced** |
| `assistant` | per `content[]` block: `text` → `AssistantChunk` **only if** `message.id ∉ streamed` (H-8); `thinking` → `ThoughtChunk` under the same rule; `tool_use` → `ToolCall { tool_call_id: id, title: name, tool_kind: tool_kind(name), input, locations: locations_of(input) }` | `parent_tool_use_id` present → the same, unchanged (we do not pass `--forward-subagent-text`, so it does not arise) |
| `user` with `tool_result` blocks | `ToolResult { tool_call_id: tool_use_id, status: is_error ? Failed : Completed, output: output_of(content), locations: [], terminal_reason: None }` per block | a real failure the agent reported leaves `terminal_reason` `None` (`event.rs:262-264`) |
| `user` without | none | §6.2 rows 1–2 |
| `rate_limit_event` | `Other { update: "rate_limit_event", body }` | and `pending_quota = body.rate_limit_info` (or `body` — **T56 decides** the nesting), `is_object`-filtered as `acp/map.rs:116-120` |
| `system` / `init` | `Other { update: "system/init", body }` | the supervisor intercepts this kind before `map` and emits the banner instead (B.2 step 3); the mapper's arm exists for `cli_map.rs` |
| `system` / `api_retry` | `Error { code: "api_retry", message }` | §6.2's row, as written; H-20 |
| `system` / other | `Other { update: "system/<subtype>", body }` | hooks, `compact_boundary`, plugins |
| `result` | in order: `denials_of(result)…`, `Usage(..)`, `Error { code: subtype, message: result.error ‖ result.result }` **iff** `is_error`, `Done { stop_reason }` | one `usage` row per turn (§6.2); the `Done` is last so a cap breach on the `usage` row finds the `done` **after** it, which `enforce_breach` withholds |
| anything else, and a line that is not JSON | `Other { update: update_name ‖ "<unparsed>", body }` | §6.2's wildcard |

**D86, field by field**, from `result` (`m = result.modelUsage`, a map of per-model objects;
`sum(k)` = the sum of `m[*][k]` over models, `None` when `m` is absent or empty — an empty map
answers `null` rather than `0`, so a harness that sends no `modelUsage` yields the five-null shape
`usage_deltas_sum_to_step_usage` expects):

| `UsageEvent` key | Derivation |
|---|---|
| `cost_micros` | `total = round(result.total_cost_usd × 1e6)` (saturating, `acp/map.rs:125-136`'s helper lifted or copied); `Some(total − last.cost_micros)`; `None` when `total_cost_usd` is absent |
| `cost_micros_total` | `Some(total)` |
| `input_tokens` / `output_tokens` / `cache_read_tokens` / `cache_write_tokens` | `sum(inputTokens)` / `sum(outputTokens)` / `sum(cacheReadInputTokens)` / `sum(cacheCreationInputTokens)`, **minus** `last.*` when `tokens_are_cumulative` (E-2); `last` updated to the new sums either way |
| `context_used`, `context_size` | `None`, `None` — the stream has no occupancy figure; `modelUsage[*].contextWindow` is a model's window, not this turn's use, and is left on `raw` |
| `cost_amount`, `cost_currency` | `None` — the CLI reports USD only |
| `usage_scope` | `Some(scope.as_str())` — `"model_usage"` for the seed (§7 `:1102-1103`) |
| `quota` | `pending_quota.take()` — the last `rate_limit_event`'s blob rides the turn's one `usage` row, which is where `Recorder::latch_quota` reads it (`record.rs:1098-1100`); `nothing_to_say` already treats `CliRateLimitEvent` as an allowance-reporting source (`:1171-1173`), so the first turn without a blob publishes nothing (H-3 of milestone 7 holds here too) and E-3's `normalize` arm makes the windows real |

The delta rule and D78: the recorder's per-token publish rule reads `billing` first
(`record.rs:1153-1176`), so a `claude-cli` row switched to `per_token` publishes spend on every
costed row — the CLI path needs **nothing** the ACP path did not, once E-3's arm exists. What the
CLI path needs that ACP did not is only the `tokens_are_cumulative` flag (E-2).

### B.5 `record.rs` — the denial conversion (T53; E-1)

```rust
/// `other.update` of a permission the transport's own policy denied (`docs/ANA-4.md` §6.2, plan
/// D85 as amended by blueprint E-1). A `DriverEvent::Other` carrying it is recorded as the
/// `permission_answer` row §6.2 describes rather than as an `other` row: the kind is one `htui`
/// authors, the role is `htui`, and `denied: true` is the added key that tells it apart from an
/// answer a human or a rule gave. `SESSION_STARTED` is the precedent: an `other.update` every
/// transport agrees on, owned by this crate, matched by nothing keyed on an agent's name.
pub const PERMISSION_DENIED: &str = "permission_denied";
```

In `record`, after `scrubbed` is built (`:733-737`) and before the chunk logic:

```rust
if let DriverEvent::Other(other) = &scrubbed.event && other.update == PERMISSION_DENIED {
    return self.record_policy_denial(other, scrubbed.raw.take(), scrubbed.at).await.map(|()| None);
}
```

`record_policy_denial` (private): `flush`; push `PendingRow { kind: PermissionAnswer, role: EventRole::Htui,
tool_call_id: body.tool_call_id, payload: { request_id: body.tool_call_id, option_id: null, by: "policy",
cancelled: false, denied: true, tool_name: body.tool_name }, raw: [raw] if Some, at }`; `flush`;
`send_ui` the **original** `Other` envelope (the UI channel carries `DriverEnvelope`, which has no
answer variant — the tab renders the verbatim denial live and the answer row on replay; H-3 names
this and T57's snapshot pins it). The key set is `record_permission_answer`'s four plus two added
keys (`:664-670`), so the transcript's `by` reader (`transcript.rs:347`) needs nothing.

### B.6 `conformance.rs` — the second arms (T53; D80, E-4, E-5)

`ScriptEvent` gains, additively (its own doc says milestone 3 may):

```rust
/// This call was denied by the **transport's own** policy, with no request `htui` could answer:
/// the CLI's `--permission-mode` refusing a tool (`docs/ANA-4.md` §4.3, §6.2). A transport that
/// *has* a permission channel never sees this marker — the cases that use it take the
/// no-capability arm — so the ACP harness refuses it by name, as it refuses a scripted
/// `permission_request`. The fake plays it as the denied call's synthesized `failed/rejected`
/// result behind an `other { permission_denied }` marker, which is what `Recorder` turns into the
/// row (B.5).
PolicyDenied(String),
```

`open_case` → `(ChatRunSpec, Box<dyn AgentSession>, DriverCaps)`; every case destructures the
third. The arms, each documented beside its assertion the way `fake.rs:9-27` documents the rules:

| Case | Gate | With the capability | **Without** — the negative, then the surviving positives |
|---|---|---|---|
| `cancel_answers_parked_permissions` | `caps.permission_requests` | unchanged | Script `chunk, tool_call(call-1), ExpectCancel` (no `park`: a transport that cannot park has nothing to park). Pull and record to the `tool_call` (a small `pump_to_kind` helper, or `pump_to_permission` generalised); `cancel(0)`; `pump` → `stop == Cancelled`. Assert **no** `permission_request` row and **no** `permission_answer` row (the negative: a transport claiming no permission channel put no permission row in the log); `call-1` has exactly one `tool_result`, `status == failed`, `terminal_reason == cancelled` (criterion 5's third clause survives); last row `done`, exactly one `done`, `stop_reason == cancelled` |
| `rejected_tool_gets_failed_result` | `caps.permission_requests` | unchanged | Script `tool_call(call-9), PolicyDenied(call-9), tool_result(call-9, "should never be recorded"), done(EndTurn)`. `pump` to done. Assert no `permission_request` row; exactly one `tool_result` for `call-9`, `status == failed`; the string `should never be recorded` absent (the transport's own late result for a settled call is dropped — §4.3's rule survives); one `permission_answer` row with `by == policy`, `role == htui`, `option_id == null`, `denied == true`, `request_id == "call-9"` and `tool_call_id == Some("call-9")` (D85's row, via B.5) |
| `edit_proposal_deduped_per_call_and_path` | `caps.edit_proposals` | unchanged | Same script. Assert **zero** `edit_proposal` rows; **three** `tool_call` rows with `tool_kind == edit` (one per scripted proposal — they are calls, and calls do not dedupe), whose `locations[0].path` are `src/a.rs`, `src/b.rs`, `src/a.rs` in script order (§4.3 `:545-546`: "surfaces them only as post-hoc `Edit`/`Write` tool calls"); each has a `tool_result`; the `assistant_text` row sits between the second and third; `seq` gapless; `done` last |
| `usage_deltas_sum_to_step_usage` | `caps.usage_mid_turn` | unchanged | Assert **one** `usage` row; `summary.usage == expected` (the five-key document with `cost_micros: 351`, the other four `null`); `spy.calls().last().usage == expected`; the row's `cost_micros == 351` **and** `cost_micros_total == 351` (the turn-end form: delta and total coincide on a first turn); the digest clauses unchanged |
| `quota_blob_latches_agent_box` | `caps.usage_mid_turn` | unchanged | Script A: one `usage` row, `payload.quota == blob`; **one** latch call equal to `normalize(AcpMetaRateLimit, Subscription, Some(&blob), Some(351), reports[0].at)` (the case's own source, as today — the CLI blob has the ACP blob's shape, §7); `quota_at == reports[0].at`; windows `["five_hour","seven_day"]`. Script B: one row, one call, spend `351` |
| `run_cap_breach_cancels_within_one_event` | `caps.usage_mid_turn` | unchanged | Scripts (a) and (d) end in `done(EndTurn)` instead of `ExpectCancel` (a turn-end transport cannot be cancelled before its report arrives, and the report ends the turn); (b), (c) unchanged. Assertions unchanged **verbatim**: `stop == Cancelled`, `cap_breach == Some { 300, 350, at }`, `log[breaching+1] == Error`, last two `error, done`, exactly one `done` — the transport's own `done{end_turn}` is withheld by `enforce_breach` (milestone 7 H-4, `record.rs:1714-1719`), which this arm proves on a real transport for the first time |
| `session_banner_is_first_other_row` | — | E-5: `body.protocol_version` is present and is `null` or a positive integer | same assertion |

Every other case is unchanged. **Inert for the fake and ACP bindings**: the fake's `full_caps` and
`caps_from`'s ACP arm are all-true, so every gate takes the left column; `fake_conformance` and
`acp_conformance` must stay green **unchanged** (the plan's T53 validation), and that is the check.

### B.7 `tests/cli_conformance.rs` (T54)

`CliHarness { row: seed_rows().find(name == "claude-cli"), sessions: AtomicU64 }` — the
`acp_conformance.rs:39-80` shape over `CliDriver::over(ChildIo { reader, writer, child: None }, &row,
caps_for(&row), Stamp::Fixed { epoch })` and `tokio::io::duplex(DUPLEX_BYTES)` (same 256 KiB, same
reason). `scripted_cli(stream, script, n)`: reads stdin lines; the first user message → emit
`system/init { session_id: <the line's session_id>, model: "sonnet", tools: [], … }` (echoing ours,
P-2), then plays turn 0; each later user message plays the next turn; **its cancel behaviour is
copied from the T55 fixture** (`claude_stream_json_stdin_close.jsonl`: whatever the binary wrote
after stdin closed mid-turn, the scripted agent writes the same shape, then EOF). `wire_lines(event,
&mut Totals) -> Vec<Value>` — the inverse of B.4, panicking by name on `ParkPermission` and
`PermissionRequest`/`EditProposal`/`Plan` (no shape in this dialect), and on `Done` **only when a
`Usage` was folded into it**:

| `ScriptEvent` | Lines |
|---|---|
| `AssistantChunk { text, message_id }` | `stream_event/message_start { message: { id } }` when the id changes, then `content_block_delta { delta: { type: text_delta, text } }`; on the next id change or the turn's end, the full `assistant { message: { id, content: [ { type: text, text: <all deltas> } ] } }` (so H-8's dedup rule is exercised on every run) |
| `ThoughtChunk` | the same with `thinking_delta` / `{ type: thinking, thinking }` |
| `ToolCall` | `assistant { message: { id: current, content: [ { type: tool_use, id, name: title, input } ] } }` — `title` doubles as the tool name so `tool_kind` round-trips `read` → `Read` (the harness maps the `ToolKind` back to a canonical name: `Read`, `Edit`, `Bash`, `Glob`, `WebFetch`, else `Other`) |
| `ToolResult` | `user { message: { content: [ { type: tool_result, tool_use_id, content: <output text>, is_error: status == failed } ] } }` |
| `EditProposal { tool_call_id, path, diff }` | a `ToolCall`-shaped `Edit` with `id = "<call>#<n>"`, `input: { file_path: path, old_string: "", new_string: diff }`, followed by its `tool_result` — no diff on the wire, ever |
| `PolicyDenied(call)` | `user`/`tool_result { tool_use_id: call, is_error: true, content: <the fixture's denial text> }` now, and the call is appended to the turn's `permission_denials[]` for its `result` |
| `Usage { cost_micros, quota }` | folded: `cost += cost_micros`; a `quota` → a `rate_limit_event` line **now** (a real mid-turn shape); nothing else on the wire |
| `Other { update, body }` | `{ type: update, ..body }` (an object body's keys spread, else `body`) |
| `Done { stop_reason }` | `result { subtype: "success" ‖ …, is_error: false, total_cost_usd: cost / 1e6, num_turns, permission_denials, duration_ms: 0 }` — **no** `modelUsage` (the five-null token shape) |
| `ExpectCancel` | stall until stdin closes, then the fixture's shape |

`the_case_list_is_the_shared_one` asserts 15; `the_cli_transport_passes_every_case` runs `run_all`.

---

## C. The documents and the screen, worked

### C.1 `crates/htui-core/seeds/agent_claude_cli.json` (T49; D79 under E-0 (a))

```json
{
  "name": "claude-cli",
  "transport": "cli",
  "billing": "subscription",
  "models": [],
  "default_model": null,
  "launch": {
    "command": "${claude}",
    "args": [],
    "env": {},
    "discovery": {
      "tools": {
        "claude": { "kind": "path", "names": ["claude"],
                    "version": { "args": ["--version"],
                                 "pattern": "^(\\d+\\.\\d+\\.\\d+) \\(Claude Code\\)$" } }
      },
      "handshake": false
    }
  },
  "settings": {
    "cli": { "stream": "claude_stream_json", "permission_mode": "acceptEdits", "extra_args": [] },
    "permission": { "default": "ask", "rules": [], "remembered": [] },
    "quota": { "source": "cli_rate_limit_event" },
    "usage": { "scope": "model_usage" }
  }
}
```

`extra_args` is `[]` under E-0 (a) and `["--bare"]` under (b), where `billing`/`quota.source` also
change. `permission.default: ask` is inert on this row (nothing asks) and kept so the block parses
identically to its siblings. The probe on this row: tier 1 resolves `claude` and captures the
version; tier 2 declines (`probe.rs:1444-1448`); the snapshot is `Ready`/`Probe`/`resolved: Some`
and `agent_box.version` is the capture — which is the banner's `agent_version` fallback (B.2).

`seed_rows_match_ana4_5_3` after T49: `rows.len() == 3`; `rows[2].name == "claude-cli"`,
`transport == Cli`, `billing == Subscription`, `settings.cli.stream == "claude_stream_json"`,
`permission_mode == "acceptEdits"`, `extra_args == []`, `quota.source == "cli_rate_limit_event"`,
`usage.scope == "model_usage"`, `models.is_empty()`; the per-row loop's `transport == Acp` becomes
`rows[..2]`; and `rows[0].settings.get("cli").is_none()` pins D79's removal.

### C.2 A recorded turn, as rows (the shape T57 asserts)

```
seq 0  turn 0  prompt            role htui
seq 1  turn 0  other             session_started { session_id: <ours>, protocol_version: null, agent_name: "claude-cli", agent_version: "2.1.267", models: ["…"] }
seq 2  turn 0  other             system/hook_started …            (E-0 (a) only; buffered until init, emitted after the banner)
seq 3  turn 0  assistant_text    "ok"                             (deltas coalesced on message.id)
seq 4  turn 0  usage             { input_tokens, …, cost_micros: Δ, cost_micros_total, usage_scope: "model_usage", quota? }
seq 5  turn 0  done              { stop_reason: "end_turn" }
seq 6  turn 1  follow_up         role htui
…
```

`run_step.usage.cost_micros == Σ usage.cost_micros == last cost_micros_total` — criterion 7, both
clauses, which T57 asserts through `UsageTotals::from_rows` over `step_events` and the spy-free
route `chat_usage_pg.rs` established.

### C.3 D89, verified independently

`ratatui 0.30.2` (`Cargo.lock`); `Table`'s default `flex` is `Flex::Start`
(`ratatui-widgets-0.3.2/src/table.rs:289`); a `Min(n)` under any non-legacy flex gets
`size ≥ n` at `MIN_SIZE_GE` (strong) **and** `size == area` at `FILL_GROW` (medium), while the
trailing spacer of `Flex::Start` grows only at `GROW` = medium/10 and every `Length` is pinned at
`LENGTH_SIZE_EQ` (`ratatui-core-0.1.2/src/layout/layout.rs:959-964`, `:1075-1082`, `:1350-1357`) —
so the one `Min` column absorbs every spare cell **wherever it sits**, and moving it from last to
first changes nothing. The default highlight symbol is empty, so no selection column is reserved
(the existing 98 → 15 arithmetic in the comment block already proves that). Computed with the real
solver in a scratch crate (`Layout::horizontal(..).spacing(1)`), widths as `[name, transport,
billing, models, default, enabled, quota, on this box]`:

| Area | Today (`Min(11)` last) | D89 (`Min(10)` first, `Length(13)` last) |
|---|---|---|
| 98 (100-column terminal, bordered) | `[8, 9, 12, 6, 21, 7, 13, 15]` | `[10, 9, 12, 6, 21, 7, 13, 13]` — `claude-cli` whole |
| 100 (the snapshot suite's unbordered width) | `[…, 17]` | `[12, …, 13]` |
| 118 (120-column terminal) | `[…, 35]` | `[30, …, 13]` |
| 158 (160-column terminal) | `[…, 75]` | `[70, …, 13]` |
| 90 (below the guard) | `[8, 9, 9, 6, 20, 7, 13, 11]` | `[10, 9, 9, 6, 16, 7, 13, 13]` |

**The solver does what the plan predicts** at 98, 100, 118 and 158: 10, 12, 30, 70. The one
behaviour the plan does not describe is *below* the guard width, where `Flex::Start` shrinks the
`Length` columns (`billing`, `default`) proportionally and keeps `name` at its floor — a squeeze the
old layout had too (`default` 21 → 20 → 10 at 90 and 80), so it is not new damage, but the rewritten
comment should say the ranking holds **at and above 98** rather than everywhere. `Flex::Legacy`
would instead zero `on this box` at 80; the table does not use it and must not start to.

The rewritten comment block (`agents.rs:1021-1066`) records: D89's ranking (`name` first: it is the
one column whose content now exceeds 8 — `claude-cli` is 10 — and every cell wider is a name the
operator chose; then `default`; then `quota`; `on this box` last, fixed at 13, paying 2), the
price named in full (`unauthenticated` → `unauthenticat`, `choose a method` → `choose a meth`,
E-7), the arithmetic (`9 + 12 + 6 + 21 + 7 + 13 + 13 = 81` fixed, 7 gaps, `name` draws `area − 88`,
10 at 98), and the "at and above 98" clause. It must not spell any string in H-25's list — the
current block avoids `amp-acp` deliberately (`:1046-1048`) and the rewrite keeps that sentence.

---

## D. Data flow and ownership

### D-1. `ChatStart` on the `claude-cli` row

```
AgentRuntime::start  (worker loop; no spawn, no fs I/O)
  backend.agents() → summary { agent: claude-cli, on_box }
  factory.driver_for(&agent, on_box)                       registry: transport cli, settings.cli.stream → "cli/claude_stream_json"
    └ ClaudeStreamAdapter::build → CliDriver::from_row_with_probe (launch parsed; recorded = snapshot.recorded_launch(); box_version)
  project_caps read (D70) → spec.budget_micros = run_micros (P-5)
  ChatAccepted { caps: driver.caps() }  → the tab's caps_banner names permission requests, edit proposals, plans
run_chat  (its own task)
  driver.start(spec, prompt)
    └ launch_for → launch_from(launch, recorded, cwd) → env.extend(spec.env)
    └ resolved.args = argv(row args, settings.cli, spec, minted id)
    └ launch::spawn (which → group leader / job object → piped stdio → stderr tail task) → ChildIo
    └ open_session: task spawned; prompt on stdin; pre-init lines buffered; init → banner → ready
  run_turn: next_event … Usage → Recorder (add_payload, latch_quota → normalize(CliRateLimitEvent, …) → set_agent_box_quota, check_cap) … Done
```

### D-2. Cancel, and why the drain comes before the synthesized `done`

```
handle.cancel(grace) → Command::Cancel
  task: drop stdin ─▶ interrupt() (unix) ─▶ read lines until result | EOF | deadline ─▶ close_turn(Cancelled) if still open ─▶ kill+reap ─▶ ack
                                             │ a `result` here is the turn's real end (its own stop_reason)
                                             │ EOF here: error{transport_closed} is NOT written — the cancel is the cause, and the synthesized done says so
handle: drains events into `pending` until the ack; joins the task; Ok(())
caller: pump/run_turn → pending → … → done{cancelled | the real one}; then Ok(None)
```

The milestone-3 defect ("a stream ending before its `done` was recorded as a finished turn") had
two halves: a `done` written while lines were still in flight (the drain closes it — nothing is
synthesized until the read side is exhausted), and an EOF taken as a clean end (step 5's
EOF-with-`turn_open` closes it — an unexpected EOF is an `error` plus a `done{cancelled}`, never a
silent `Ok(None)`).

### D-3. A denial (E-1)

```
CLI: user/tool_result{is_error} for tool_use X … result{permission_denials: [X]}
mapper: ToolResult{X, failed} … then at result: Other{permission_denied, {tool_call_id: X, tool_name}} · Usage · Done
recorder: tool_result row (agent) … permission_answer row (htui, by: policy, denied: true, request_id: X) · usage · done
tab, live: the Other envelope (verbatim denial) · replay: the permission_answer row
```

---

## E. Build order and file-set intersections

Order: **T55 → T56 ∥ T49 → T49b ∥ T50a** — then **T51 → T50 → T53 → T54 → T52 → T57 → T58**.

| Pair | Intersection | Parallel? |
|---|---|---|
| T55/T56 × T49/T49b × T50a | ∅ (`cli_live.rs` + fixtures · seeds, `agent.rs`, `fixtures.rs`, `pg/mod.rs`, `settings/agents.rs`, `settings.rs`, snapshots · `event.rs`, `launch.rs`, `acp/mod.rs`, `fake.rs`, `driver.rs`, `registry.rs`, `agent_worker.rs:1144`, `Cargo.toml`, the six spec literals) | **yes** — three fronts |
| T49 × T49b | the five `settings__agents_*` snapshots | serial: T49 first (the row), T49b second (the widths), each re-accepting once |
| T51 × T55/T56 | the fixtures are T51's **inputs** | serial by need: T51 is red until the fixtures exist |
| T50 × T50a | `launch.rs`, `acp/mod.rs` (the lifts) | serial: T50a first |
| T50 × T51 | `src/cli/`, `lib.rs` | serial pair, one implementer (the plan's note) |
| T53 × T50/T51 | `record.rs` (B.5) is T53's; `cli/claude.rs` imports `record::PERMISSION_DENIED` | T53's constant lands **before** T51 compiles, or T51 defines it and T53 moves it — pick the first: T50a adds the constant with its doc and no arm |
| T54 × everything above | the binding needs T50, T51, T53 and the T55 fixture | serial, last of the code |
| T52 × T50 | `agent_worker.rs` (the two renames), `registry.rs` | after T50 |

Checkpoints: `cargo fmt --all -- --check` first, every time. Per task the plan's lines with E-6
applied. The Windows clippy line is listed and **marked unavailable** (TOOL-3) in every commit that
touches a `cfg` — T50 and T50a. Commits, one per task, in the house style:
`test(agent): live claude CLI probes — cancellation, stdin, session id, cumulative usage (T55)` ·
`test(agent): thinking, subagent usage, budget and denial fixtures (T56)` ·
`feat(core,store): the claude-cli row, a third fixture id, and the name-keyed seed top-up (D79, D88)` ·
`fix(tui): Settings name column is the flexible one; on this box fixes at 13 (D89)` ·
`refactor(agent): lift Stamp, ChildIo and the banner constants; SessionSpec.budget_micros; DriverCaps.usage_mid_turn (D90, E-4)` ·
`feat(agent): the claude stream-json mapper, from recorded fixtures (D82, D86)` ·
`feat(agent): the CLI supervisor — spawn, stdin follow-ups, cancel by stdin-close/SIGINT/kill, the banner (D81, D83, D84)` ·
`feat(agent): capability arms in the conformance suite and the policy-denial row (D80, D85)` ·
`test(agent): the CLI transport passes every case (T54)` ·
`refactor(agent,tui): DriverFactory::production registers acp and cli/claude_stream_json (D87)` ·
`test(tui): a live chat over the CLI row (T57)` · `docs(mod-2): milestone 8 close-out`.

---

## F. Test plan, per task, TDD order

### F-T55 — `crates/htui-agent/tests/cli_live.rs` (CREATE, `#[ignore]`, first)

Module doc mirrors `agy_live.rs`: what it spawns (the `claude` the seed's tier-1 probe resolves,
through `tools::resolve` + `launch::resolve` + `launch::spawn` — **never through `src/cli/`**, which
does not exist yet and must not be what the probe proves), that every case spends a few tokens, the
run line `cargo test -p htui-agent --features test-support --test cli_live -- --ignored --nocapture`,
the fixture variable `HTUI_CLI_FIXTURE_DIR` (read, never written; each case writes
`claude_stream_json_<case>.jsonl` there as `{"direction":"stdin"|"stdout","line":<json|text>}` records
plus a final `{"direction":"exit","status":…,"signal":…}`), and the redaction rule (A.18). Every
case: spawn with the §4.4 argv **minus** anything under test, read stdout to EOF on a task, assert
by **pid** that nothing survives (`Spawned::pid`, `/proc/<pid>` absent or state `Z`, the
`acp_driver.rs` helper's parse), and print the answer it exists for.

1. `case_1_auth_and_the_first_stdin_message` — **E-0**: two spawns of `-p --output-format stream-json --input-format stream-json --verbose`, one with `--bare` and one without, each fed one user message on stdin ("reply with exactly the word ok") and stdin closed after the `result`. Records both transcripts; prints whether each produced `system/init` + an `assistant` + a `result{success}` or an auth refusal on stderr. Also answers: is `init` emitted before the first stdin line is read (send the prompt after a 500 ms wait and see whether `init` preceded it), do hook lines precede `init` without `--bare`, and does `-p` with no positional prompt accept the stdin form (P-2). **Fails loudly, not silently**, when neither form authenticates: the message names `ANTHROPIC_API_KEY` and the OAuth login.
2. `case_2_session_id_is_ours` — `--session-id <Uuid::now_v7()>`: `init.session_id` equals it (H-12); then a second spawn with `--resume <that id>` and a one-word follow-up, asserting the reply arrives under the same id (D84's resume half). Under E-0 (b) the same with `--bare`.
3. `case_3_cancellation_semantics` — **D81**: three spawns, each given a prompt that runs long ("count slowly from 1 to 200, one number per line"), and after the first `stream_event` text delta: (a) stdin closed only; (b) stdin closed then `signal(SIGINT)` to the group (`process_wrap::tokio::ChildWrapper::signal` through a new `Spawned::interrupt`, which this case therefore needs — **the one production line T55 lands ahead of T50**, in `launch.rs`); (c) `SIGTERM` (`signal(15)`). Records each terminal envelope (or its absence), the exit status and the wall time to EOF. The finding fills B.4's `stop_reason` rows marked `?` and B.7's cancel shape.
4. `case_4_usage_across_two_turns` — **E-2**: one process, two user messages (a reply, then "and one more word"), both `result`s printed side by side: `total_cost_usd`, `usage`, `modelUsage[*]`. The test **asserts** the relation it finds (`turn2.modelUsage.inputTokens >= turn1's` as a smell of cumulative, or `<` as per-turn) and prints the verdict `tokens_are_cumulative = …`, which sets B.4's flag.
5. `case_5_stdin_eof_between_turns_exits_clean` — after a `result`, close stdin, assert exit status 0 and EOF within 5 s (the "not a cancel" path of D-2).

### F-T56 — same file (serial after T55)

6. `case_6_thinking_blocks` — D82: a prompt that invites reasoning ("think step by step, then answer: what is 17 × 23?") with `--include-partial-messages`; records `claude_stream_json_thinking.jsonl`; prints the `content_block_start` type and the delta types seen; asserts a `thinking` block or a `thinking_delta` arrived, else prints "thinking was off in this run" without failing (the fixture is the answer either way).
7. `case_7_subagent_makes_model_usage_and_usage_disagree` — a prompt that uses the `Task`/`Agent` tool once; asserts `Σ modelUsage.inputTokens != result.usage.input_tokens` or prints that no subagent ran; the D86 evidence.
8. `case_8_budget_trips_server_side` — `--max-budget-usd 0.000001` and `0.000000`: records both `result`s (their `subtype`, `is_error`); fills B.4's budget row and H-16.
9. `case_9_a_denied_tool` — `--permission-mode default` and "run `ls` with the Bash tool": records the denied `user`/`tool_result` text and `result.permission_denials[]` — B.4's `denials_of` and B.7's `PolicyDenied` line are written from this fixture (`claude_stream_json_denied.jsonl`). Prints whether a `rate_limit_event` appeared in any run of this file and its nesting (B.4's `pending_quota`).

### F-T49 — `crates/htui-core`, `crates/htui-store`

1. **Red**: `seed_rows_match_ana4_5_3` amended per C.1 (fails: `len() == 2`). `fixtures.rs`: `agents().len() == 3` and `agents()[2].id == ids::AGENT_CLAUDE_CLI` in the existing fixture test (fails: two, and no such id — H-10). `pg/mod.rs`: the existing `seed_if_empty_as` test gains "a table holding `claude` and `agy` gains `claude-cli` on the next pass and keeps an edited `enabled = false`" (fails: the filter).
2. The seed, the removal, the `include_str!`, the fixture id and zip, the filter removal and its doc.
3. `cargo test -p htui-core --all-features`; the Postgres line for `htui-store`; `cargo test -p htui --features testkit --test settings` and `--test probe` → **re-accept** the row additions only (`cargo insta review`, every hunk read).

### F-T49b — `crates/htui/tests/settings.rs`

1. **Red**: `the_name_column_holds_claude_cli_whole_at_the_bordered_width` — a `probed_row("claude-cli", …)` rendered at `SECTION_BORDERED`, `name_cell == "claude-cli"` (fails: `claude-c`). `the_name_column_absorbs_the_slack` — the same row at 160, the `transport` cell starts at column 71 (`name` 70 + 1). `the_on_box_column_holds_the_whole_unauthenticated_verdict` **inverts** to `the_on_box_column_is_fixed_at_thirteen_and_says_so`: `on_box_cell == "unauthenticat"` with a message quoting D89's price and E-7's second word.
2. The two constraints and the comment block (C.3).
3. Constants: `NAME_WIDTH` → a doc saying it is now the floor (10) and that `row_line` matches the first 10; `DEFAULT_AT = 43`, `QUOTA_AT = 73`, `ON_BOX_AT = 87` at `SECTION_WIDE` (name draws 12 there); `ON_BOX_WIDE = 13` as a literal (it is a `Length` now, the same at 98 and 100). `as_drawn` keeps taking `ON_BOX_WIDE`; its doc and the nine call sites' expectations (`:1288, :1505, :1694, :1896, :2147, :2182, :2232, :2281, :2361`) are re-read — `downloading 42%` → `downloading 4`, `choose a method` → `choose a meth`. Five snapshots re-accepted (`INSTA_UPDATE=always` then without, every hunk read: only the `name`/`on this box` widths and the third row may move).

### F-T50a — the seam lifts

1. **Red**: `tests/driver_contract.rs` — a `DriverCaps::default()` has `usage_mid_turn == false` and `FakeDriver::full_caps().usage_mid_turn` (fails: no field); `caps_for(&claude_row).usage_mid_turn` and `!caps_for(&zeta-shaped cli row).usage_mid_turn` in `extensibility.rs::an_unknown_agent_reaches_a_driver_from_its_row_alone` (`:162-170`); a `SessionSpec { budget_micros: Some(300), .. }` literal in `driver_contract.rs::spec()` (fails: no field); `agent_worker.rs` in-module: `fixture_with_project_settings(.., {"per_token_cap_run": 300})` → the spec the driver received has `budget_micros == Some(300)` (a `FakeAdapter` records the spec it was started with — **UNVERIFIED — implementer must check** whether the fake exposes the spec; if not, assert through `ChatArgs` in the existing `a_chat_over_its_run_cap_is_cancelled_and_its_run_fails` fixture).
2. `event.rs`, `launch.rs` (`ChildIo`, `launch_from`), `acp/mod.rs` re-exports, `fake.rs`, `driver.rs`, `registry.rs`, `record.rs` (`PERMISSION_DENIED` constant only), the seven spec sites, `Cargo.toml`.
3. `cargo test --workspace --all-features` green unchanged; `cargo doc --workspace --no-deps` (the moved items' intra-doc links).

### F-T51 — `crates/htui-agent/tests/cli_map.rs` (CREATE) and `cli/claude.rs` in-module

1. **Red**, `cli_map.rs`: `a_recorded_turn_maps_to_the_rows_it_did_when_it_was_captured` over `claude_stream_json_turns.jsonl` (the two-turn fixture): `insta::assert_debug_snapshot!("cli_map__turns", events)`; `usage_is_one_row_per_turn_with_cost_deltas_summing_to_the_last_total` — exactly two `Usage` events, `Σ cost_micros == last cost_micros_total`, `UsageTotals::add_payload` over both equals the last totals (criterion 7 on the mapper alone, and E-2's proof once the flag is set from T55 case 4); `text_streams_once_not_twice` — over the `thinking` fixture the `AssistantChunk` texts concatenated equal the `assistant` message's text exactly once (H-8); `thinking_maps_to_thought` (D82); one snapshot each for `sigint`, `denied`, `budget`, `subagent` (the regression net: a future dialect change shows as a snapshot diff, `acp_map.rs:13-16`'s sentence). **Fails because** the module does not exist.
2. **Red**, in-module (`cli/claude.rs` `mod tests`, exempt from the sweep by `production_half`): `tool_kind` table; `stop_reason` table (from T55 case 3); `locations_of` the three keys; `output_of` string vs array; `an_unparsed_line_lands_in_other`; `a_user_line_without_tool_results_maps_to_nothing`; `denials_of_renames_the_vendor_keys`; `an_empty_model_usage_answers_null_not_zero`; `a_rate_limit_blob_rides_the_next_usage_row_and_only_that_one`.
3. The mapper; `quota.rs`'s arm (E-3) with its in-module test flipped — red first: `normalize(CliRateLimitEvent, Subscription, Some(&blob), ..)` has two windows (fails: none).

### F-T50 — `crates/htui-agent/tests/cli_driver.rs` (CREATE; the `acp_driver.rs` shape)

1. **Red**: `argv_is_the_ana4_line_in_order` — `argv(&["--row-arg"], &cli { permission_mode: "acceptEdits", extra_args: ["--x"] }, &spec { model: Some("sonnet"), extra_dirs: [a, b], budget_micros: Some(300), resume: None }, "id")` equals the B.2 sequence with `--max-budget-usd 0.000300` and `--x` last; with `resume: Some("r")` → `--resume r` and no `--session-id`; `usd(1_500_000) == "1.500000"`, `usd(0) == "0.000000"`. **Fails because** no module.
2. **Red**, `#[cfg(unix)]`, over `launch::spawn` of a shell script the test writes (the `acp_driver.rs:456-460` technique): `a_session_over_a_scripted_binary_streams_a_turn_and_ends_clean` — `sh` script that echoes a canned `init`, reads one stdin line, echoes an `assistant`, a `result`, then reads until EOF and exits 0; through `DriverFactory::new()` + `register(cli::ADAPTER_ID, ClaudeStreamAdapter)` + `driver_for(&claude_cli_seed_row_with_command_overridden, None)` — wait, a seed row must not be modified (H-7): use a **synthetic** `cli` row naming `${tool}` with `HTUI_TOOL_*`-free discovery pointing at the script (`acp_driver.rs`'s `synthetic_row` shape). Asserts banner first (with `agent_version` from the script's `init`), `AssistantChunk`, `Usage`, `Done{EndTurn}`, then `send_follow_up` → the script's second turn; `cancel(0)` between turns → `Ok(None)`; the pid is gone.
   `an_eof_mid_turn_is_an_error_and_a_cancelled_done` — script exits after the `assistant`: rows `error{transport_closed}`, `done{cancelled}`, in that order, and **no** `done` before the `error` (D-2). `cancel_mid_turn_drains_before_it_synthesizes` — script that, on SIGINT, writes a `result` after 200 ms (`trap`): with grace 1 s the `done` carries the script's stop reason; with grace 0 the `done` is `cancelled` and the script's later `result` never lands (the guard killed it). `a_prompt_never_reaches_argv` — the script dumps `$@` to a file; the prompt text is absent from it (P-2). `init_timeout_kills_the_child` — a script that sleeps: `Err(Transport(.. "system/init" ..))` within the test's `INIT_TIMEOUT` override (`SessionOptions.init_timeout`, the milestone-6 P-1 pattern) and the pid is gone.
3. The supervisor. Proof: `cargo test -p htui-agent --features test-support --test cli_driver`; the Windows line marked unavailable.

### F-T53 — `conformance.rs`, `record.rs`, `tests/recorder.rs`

1. **Red**, `tests/recorder.rs`: `a_permission_denied_marker_becomes_a_policy_answer_row` — record `Other { update: PERMISSION_DENIED, body: { tool_call_id: "x", tool_name: "Bash" } }` with `retain_raw`: one `permission_answer` row, role `htui`, the six keys, `tool_call_id` column `"x"`, `raw` present; and `an_other_row_with_another_update_is_still_other`. **Fails because** the arm does not exist.
2. **Red**, the arms (B.6) — proven red by running `fake_conformance` with `FakeDriver::full_caps()` temporarily answering `permission_requests: false` in a scratch build (the arm's negative assertions must fail on a fake that still parks), then restored. The count guards stay 15.
3. B.5, B.6; `fake_conformance` and `acp_conformance` green **unchanged** — the plan's own check that the ACP side lost nothing.

### F-T54 — `tests/cli_conformance.rs`

`the_case_list_is_the_shared_one` (15); `the_cli_transport_passes_every_case`. **Red** for the honest reason: the harness exists and the supervisor refuses a prepared pair until `IoSource::Prepared` lands — which is T50's, so this file is written against T50 and turns green with it.

### F-T52 — the rename

`extensibility.rs::the_production_factory_holds_two_adapters_for_three_rows` (**red**: no `production`), then the eight sites. `cargo test --workspace --all-features`.

### F-T57 — `crates/htui/tests/chat_live_cli.rs` (CREATE, `#[ignore]`)

`chat_live.rs:29-154` verbatim with: the `claude-cli` summary; `pgrep -f -- "--output-format stream-json"` (P-9); `ChatAccepted.caps` equal to `caps_for(&row)` and its three `false`s; a turn ("reply with exactly the word ok"), a follow-up, `ChatCancel` → `Ended`; assertions: the banner's `protocol_version` is `null` and `agent_version` non-empty; `run_step.usage` (through the `UsageSpy`-free route: `UsageTotals::from_rows(&step_events)` equals what `chat_usage_pg.rs` would read — the Postgres variant is one more `#[ignore]` case in `chat_usage_pg.rs` if the maintainer wants it, not owed here); the tab's banner line (`render` contains `this agent cannot: permission requests, edit proposals, plans`, via a `ChatTab` snapshot the `chat.rs` harness already knows how to take); no survivor. Run line: `HTUI_KEEP_RAW_EVENTS=1 cargo test -p htui --features testkit --test chat_live_cli -- --ignored --nocapture`.

### F-T58

README, HANDOFF, PRD, this plan's answer table (the §11.14 items: cancellation semantics from T55 case 3, thinking shape from T56 case 6, plus E-0 and E-2 as findings); `bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh`.

---

## G. Hazards

| # | Failure mode | Detected by | Closed by |
|---|---|---|---|
| H-1 | **`--bare` never reads OAuth** (E-0): every live run on a subscription box fails auth, and the row's `billing`/`quota.source` are wrong for the auth it forces | F-T55 case 1's paired spawn | The maintainer's E-0 choice; C.1 written for (a) |
| H-2 | Without `--bare`, hook lines precede `system/init`, and a supervisor that mapped them as they arrived would put an `other` row before the banner — failing `session_banner_is_first_other_row` on the live path only | F-T50 case 1 with a script that prints a hook line before `init`; F-T55 case 1 prints the order | B.2 step 3's `pre_init` buffer: banner first, then the buffered lines in order |
| H-3 | **`permission_answer` is `htui`-authored** (E-1): a transport cannot write it; a naive `Other` row would render as `other` on replay; the live UI copy is an `Other` envelope either way | F-T53 recorder case; T57's tab snapshot shows the live rendering | B.5's recorder conversion; the live/replay difference is named in the module doc and pinned by the snapshot — the maintainer may instead take the `other`-only alternative |
| H-4 | **Cumulative `modelUsage` tokens** double-count in `run_step.usage` on turn 2 (E-2) | F-T55 case 4; F-T51 `usage_is_one_row_per_turn…` over the two-turn fixture | `tokens_are_cumulative` set from the fixture; deltas either way |
| H-5 | `normalize`'s CLI arm publishes spend only (E-3) | F-T51 step 3's red `quota.rs` test | The arm |
| H-6 | **Cost exists only on `result`** (E-4): three usage cases cannot be met mid-turn; a cap breach on the CLI is detected on the turn's closing row, so the "cancel" cancels a turn that has ended — the run still ends `cancelled`/`Failed` with criterion 8's two rows, but the *next* turn was never prevented by the transport, only by the recorder's `cap_breached` and the worker's `TurnEnd::CapExceeded` break | B.6's arm for `run_cap_breach…`; `run_chat`'s `CapExceeded` arm already breaks before a follow-up (`agent_worker.rs:2354-2358`) | `DriverCaps.usage_mid_turn` (D91 candidate) and the arms; the README's cap paragraph says the CLI cap is turn-granular and `--max-budget-usd` is the mid-turn guard (D83) |
| H-7 | `protocol_version == 1` pinned (E-5) | the CLI binding | B.6's last row |
| H-8 | `--include-partial-messages` delivers text **twice**: as deltas and inside the full `assistant` message | F-T51 `text_streams_once_not_twice` over the thinking fixture; F-T54 `coalesce_across_message_id` (the harness sends both, B.7) | `streamed` set keyed on `message.id`; a message with no deltas (partials off by a user config) still maps from the full message |
| H-9 | **A stream ending before its `done`** (the `682a423` class), in two forms: EOF mid-turn taken as a clean end; a synthesized `done{cancelled}` written while a real `result` is still in the pipe after SIGINT | F-T50 `an_eof_mid_turn…` and `cancel_mid_turn_drains_before_it_synthesizes` | B.2 step 5's EOF arm; D-2's drain-then-synthesize order |
| H-10 | `fixtures::agents()` **`zip`s two ids** and silently drops a third seed (`fixtures.rs:394-398`) — the demo store would carry two agents while `seed_rows` carries three, and every fixture-based test would pass over the wrong registry | F-T49 step 1's `agents().len() == 3` | `ids::AGENT_CLAUDE_CLI` and a three-element array; `zip` replaced by an `assert_eq!(seeds.len(), ids.len())` before it |
| H-11 | D89 clips `choose a method` to `choose a meth` (E-7) and moves every position constant in `tests/settings.rs` | F-T49b; the inverted test's message | Named as the price; the maintainer's override hook is a different cell text |
| H-12 | `--session-id` "must be a valid UUID": a v7 id has version nibble `7`; a validator that insists on v4 would refuse it | F-T55 case 2 | v7 (no new `uuid` feature); if refused, `v4` joins the workspace feature list and the mint changes — one line either way |
| H-13 | SIGINT under `forbid(unsafe_code)` and without `libc` | compile | `Spawned::interrupt` over `ChildWrapper::signal(2)` (`#[cfg(unix)]`); the constant's doc says why it is a literal |
| H-14 | A follow-up written while the child is not reading stdin blocks the task on a full pipe | Not reachable for a one-line message (< 64 KiB pipe buffer); a pathological prompt is bounded by the write's `await` on the task, never the UI (`R-NF-3`) | Documented; the stdout reader keeps draining so the child cannot deadlock on its own output |
| H-15 | The scripted CLI agent and the supervisor write to each other over one duplex | `DUPLEX_BYTES` = 256 KiB (`acp_conformance.rs:27-29`'s reason); `chunk_flush_at_16kib` is the stress | Same constant |
| H-16 | `--max-budget-usd 0.000000`: the CLI may read `0` as "no budget" | F-T56 case 8 | If so, `argv` omits the flag at `0` and the recorder's cap (which cancels on the first costed row at `0`, milestone-7 H-5) is the whole guard; recorded in D83's call-site comment |
| H-17 | `system/init.session_id` disagrees with the minted id (a `--resume` the CLI reassigned, or a validator that regenerated) | F-T55 case 2 | The banner carries ours and the task logs the CLI's; `--resume` uses ours — if T55 shows the CLI rewrites it, `session_ref` follows the wire and the finding is recorded |
| H-18 | `--session-id` and `--resume` together are refused by the CLI | F-T50 `argv` case | Exactly one, from `spec.resume` |
| H-19 | Subagent lines (`parent_tool_use_id`) or an echoed user prompt | B.4's `user`-without-`tool_result` rule; no `--forward-subagent-text` | Dropped / mapped unchanged |
| H-20 | `system/api_retry` → an `error` row per transient retry, per §6.2 | T57's rows on a rate-limited afternoon | Follow the ANA; if it proves noisy, the row is `other` by a one-arm change and an ANA amendment — recorded, not done |
| H-21 | D88 resurrects a deleted row | Plan-known; stated at CONFIRM | `ON CONFLICT (name) DO NOTHING` |
| H-22 | The CLI harness must not fabricate (D80's rationale) | `wire_lines` panics by name on `ParkPermission` | B.7's table; `PolicyDenied` is the dialect's real shape from T56 case 9 |
| H-23 | CRLF on stdout (Windows) or a stray non-UTF-8 byte ends `lines()` | F-T50 with a script printing `\r\n` | `read_until` + lossy + `\r` strip, the stderr reader's rule |
| H-24 | The Settings `agents.rs` comment rewrite or a new doc comment spells a swept string | `extensibility.rs` (three sweeps) | H-25's list checked before commit |
| H-25 | **The sweep vocabulary**, so nobody guesses: `zeta` (every `.rs`/`.json` under `crates/` except the test itself, `extensibility.rs:108-130`); `antigravity`, `amp-acp`, `dl.google.com`, `agy_acp_server` (production halves of `src/` files, `:275-302`); `oauth-personal`, `oauth-business`, `gemini-api-key`, `agent-platform`, `accounts.google.com`, `antigravity-acp`, `GEMINI_HOME`, `GEMINI_API_KEY` (`:313-350`). `claude`, `claude-cli`, `claude_stream_json`, `Bash`, `Read` are **not** swept | the three tests | — |
| H-26 | `open_case` gains a return value; every case destructures | compile | P-7 |
| H-27 | `pgrep -f claude` matches the test binary | P-9 | the argv pattern |
| H-28 | A box without `claude` on `PATH`: the seed row is `enabled` (registry) while `agent_box.enabled` is false; a chat start fails at spawn | D60's re-probe (`agent_worker.rs` `run_chat` `Err` arm) already covers it | Nothing new |
| H-29 | The Windows clippy target is unavailable (TOOL-3): `interrupt`'s `cfg`, the `cfg(windows)` comment in step 6 and `spawn_supervised`'s untouched job-object path are reviewed by eye | The phase note | MOD-16 (section H) |
| H-30 | `AcpIo` → `ChildIo` and `Stamp` → `event.rs`: twelve and four files import through the old paths | `pub use … as` in `acp/mod.rs`; compile | P-4 |

---

## H. What this milestone does NOT touch

- **MOD-11**: `--permission-prompt-tool`, an MCP prompt tool, real `permission_request` rows over
  the CLI; `caps.permission_requests` stays `false` and `answer_permission` stays `Unsupported`.
- **MOD-16**: every Windows runtime fact — the `.cmd` shim through `cmd.exe`, whether the job object
  reaches a `node`-hosted CLI's children, CRLF on the pipe, stdin close as the only cancel signal
  there, and whether `claude` on Windows reads a keychain at all. Compile-side, this milestone adds
  one `#[cfg(unix)]` method and one `#[cfg(unix)]` block, both listed in H-29.
- **MOD-4**: the orchestrator's gate on `DriverCaps` (§4.3 `:554`: "gates that require an inline
  approval must not be scheduled onto a CLI-only agent") — `usage_mid_turn` is readable by it and
  is not read by it here.
- **Milestone 9**: the prompt assembler; `record_prompt`'s `sections` stay the suite's constant.
- **MOD-12**: the batch cap; `budget_micros` is the run cap only.
- **MOD-10**: `SessionSpec.env` stays empty; the CLI inherits the row's resolved env plus nothing.
- **MOD-13 / MOD-7**: `cwd` is the process's own; `--add-dir` is wired from `extra_dirs`, which
  nothing fills yet.
- **MOD-23**: the registry editor; D88's top-up is the only way an existing box gains the row.
- **The `agy` CLI** (§6.3): not built; `QuotaSource::CliStatusLine` still normalizes to nothing.
- **The chat tab**: no code — `caps_banner` (`chat/mod.rs:320-333`) already renders the banner from
  `DriverCaps`; the only tab-visible novelty is the live `other{permission_denied}` envelope (H-3).
- **`Recorder::new`, `pump`, `enforce_breach`, `record_permission_answer`**: signatures unchanged;
  B.5 is one arm and one private method.
- **`event_loop.rs`, `store_worker.rs`, migrations, `.sqlx/`**: nothing.
- **The ACP transport's behaviour**: re-exports only (P-4); `acp_conformance` and every ACP test
  green unchanged is the proof.
