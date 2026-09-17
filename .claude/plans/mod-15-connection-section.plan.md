# Plan: MOD-15 milestone 6 — a box with no DSN can fix itself

**Source**: `.claude/prds/mod-15-hierarchy-management.prd.md`, milestone 6 only (`:254`). Design
authority: the PRD's Scope (`:191-200`), Constraints (`:225-240`), **D9** (the DSN is a startup
requirement and a box without one is redirected rather than stranded), **D10** (`Settings > Rebuild
cache` is built here) — cited as **PRD Dn**; `docs/ANA-10.md` **§4.9** (the in-app DSN field, seven
sub-verdicts, `:1209-1420`) and §4.5 sub-verdict 3 (`:956-971`, the two frozen names), which are
this milestone's design document; milestone 1's plan (`.claude/plans/mod-15-hierarchy-seam.plan.md`,
**M1 Dn**), milestone 3's (`…-hierarchy-section.plan.md`, **M3 Dn**), milestone 4's
(`…-kinds-graphs.plan.md`, **M4 Dn**), milestone 5's (`…-prompt-settings.plan.md`, **M5 Dn**);
`HANDOFF.md:284-520`. This plan's own decisions are plain **Dn**.

**ANA-10's standing.** Its *verdict* is withdrawn (`HANDOFF.md:66-68`, MOD-25): everything about
local-only — `Backend::Local`, `LocalStore`, the first-run overlay, `local_setting`, the
`AckLocalOnlyNotice` request — is **stale and must not be built**. §4.9 is not part of the withdrawn
half: it is the in-app DSN field, which MOD-25 made *more* necessary by making `htui` online-only,
and PRD D9 adopts it by name. Where §4.9 names `MaskedField`, read `TextField::masked()` (M3
superseded it, `HANDOFF.md:317-319`). Where it names an overlay as the redirect's trigger, read
D6 below — the overlay it assumed does not exist.

**Requirements**: `R-STO-1` (Postgres is the only writable store; the connection string lives in the
OS keyring, `docs/REQUIREMENTS.md:122`), `R-TUI-8` (the Settings surface; "not echoed, not logged,
not written to any file", `:297-301`), `R-SEC-2` (`htui`'s own credentials are never exposed to a
session, `:254-256`), `R-NF-3` by ownership.

**Bare filenames below**: `store_worker.rs`, `connection.rs`, `hierarchy.rs`, `catalogue.rs`,
`prompt_settings.rs`, `testkit.rs`, `ui/**`, `app/**` are `crates/htui/src/…`; `connect.rs`,
`secret.rs`, `identity.rs`, `backend.rs`, `cache/mod.rs`, `dsn.rs` are `crates/htui-store/src/…`.

**Complexity**: High for this item — the first milestone that changes **process state** rather than
rows. One new `htui-store` module, one new worker module, one section, four `StoreRequest` and one
`StoreReply` variant, one new `TabAction` variant, and the in-process backend swap ANA-10 deferred
to a milestone that was never minted (§10.13/§9.1 "M9", `docs/ANA-10.md:2421`). **No seam change, no
migration, no new `query!`**: `CASES` stays 36, `EXPECTED_CASES` 36, `.sqlx/` byte-identical,
`0003_orchestration.sql` is still MOD-4's and still next.

**Routing**: routed as **PRD** by `/handoff-run MOD-15`; milestone 6 resumes at `plan`
(`/handoff-run MOD-15`, 2026-09-17, maintainer accepted the verdict). Reviewer: `rust-reviewer`
(`.claude/workflow-config.json`). Ultracode: **review only** — T1 → T2 → T3 → T4 are serial and
share `tests/connection.rs`, so the implement phase has no independent pair (see Tasks).

## Summary

Milestones 1–5 made every row this item owns writable from the app. One thing is still outside it:
the DSN. Today it enters through `htui --set-dsn`, which reads one stdin line **with echo
explicitly not suppressed** (`lib.rs:116-133`), stores it with `secret::set_dsn`
(`secret.rs:38`) and exits before the terminal is ever initialised. A box whose keyring is empty
starts `Offline` with `since: Some(now)`, `reconnect: None` (`connect.rs:189-208`) — the 30-second
ticker is guarded on `reconnect.is_some()` (`store_worker.rs:1162-1163`), so the process parks at
`offline · 0s` forever, pointing at a CLI flag the user cannot reach without quitting.

This milestone closes that, in four pieces.

1. **`htui_store::dsn`** — a `Dsn` newtype over `Zeroizing<String>` with a hand-written `Debug` that
   prints `Dsn(<redacted>)`, a `parse` that validates with `PgConnectOptions::from_str` and refuses
   with a **fixed vocabulary** rather than sqlx's text, and a `summary()` that reconstructs
   `postgres://user@host:5432/db · sslmode=require` from getters `PgConnectOptions` has — it has
   **no password getter at all**, so the summary is safe by construction. The DSN string never
   leaves this crate: `apply_dsn`/`forget_dsn` (also new here) do the keyring write, the mirror open
   and the `Reconnect` build, so the UI crate holds an opaque value and the worker holds none.

2. **`htui::connection`** — the worker half: a `ConnectionSnapshot` (is a DSN stored, its redacted
   summary, the backend label, the mirror's `CacheMeta`, the last attempt's outcome) and four served
   requests, `StoreRequest` **50 → 54**, `StoreReply` **28 → 29**.

3. **The in-process transition** — on `SetDsn` the worker writes the keyring off-task, opens
   `CacheStore` under the **new** fingerprint, closes the old handle, installs a fresh `Reconnect`
   and dials immediately. This is the seam ANA-10 §10.13 priced and deferred; PRD D9 pulls it in,
   and it is the whole reason this milestone is the riskiest.

4. **`ConnectionSection`** (`SectionId("connection")`, registered **after** `prompt`) — the first
   consumer of `TextField::masked()`, plus `TabAction::FocusSection(TabId, SectionId)` and
   `SettingsRegistry::focus`, so a launch with an empty keyring lands the user **on the field**
   rather than on a tab they have to find. `Rebuild cache` lands here too, with copy naming what
   survives and what goes.

## Design decisions (settled here, not in code review)

| # | Decision | Why |
|---|---|---|
| D1 | **The DSN's string stays inside `htui-store`.** `Dsn` (`dsn.rs`) exposes `parse`, `summary`, `fingerprint`, `Clone`, a redacting `Debug` — and **no public accessor for the text**. Everything that needs the text is a function in the same crate: `connect::apply_dsn(dsn, &ConnectContext) -> Result<Applied>` and `connect::forget_dsn()`. `Applied { cache: CacheStore, reconnect: Reconnect, fingerprint: String, summary: String }`. | `crates/htui` has **no `sqlx` dependency outside `[dev-dependencies]`** (`crates/htui/Cargo.toml`), so UI-side parsing per ANA-10 §4.9(5) is not even available there; and a `pub fn expose()` is the API shape that makes the next leak easy. Confining it means the audit is one module, not a crate. `Reconnect` already exists as a closure precisely so the worker never holds a DSN (`connect.rs:66-70`) — `apply_dsn` preserves that property instead of undoing it. |
| D2 | **Validation is `PgConnectOptions::from_str` and the refusal is a fixed vocabulary**, coined in `dsn.rs`: `not a URL` / `no host` / `unsupported sslmode` / `port out of range` / `unrecognised parameter`, each a `DsnError` variant with its own `Display`. sqlx's own string is **never** rendered, and nothing is logged on this path. | ANA-10 §4.9(5) verbatim, including its reasoning: today sqlx quotes its input in exactly one place (`ssl_mode.rs:48`, the `sslmode` value only) and nowhere quotes userinfo — "that guarantee is sqlx's, not `htui`'s, nothing in the tree asserts it, and it is one dependency bump from changing". M1 D7's rule one crate over: the sentence is coined once so two callers cannot disagree. |
| D3 | **`Dsn` holds `Zeroizing<String>`; `zeroize` is promoted from transitive to declared.** One line in `[workspace.dependencies]` + one in `crates/htui-store/Cargo.toml`. `TextField::masked()` additionally reserves `String::with_capacity(256)` and zeroizes its buffer in `clear()`/after `take()` when `masked`. | ANA-10 §4.9(6): `zeroize 1.9.0` is already compiled in as a dependency of `keyring 3.6.3` (`Cargo.lock`), so this declares an existing crate and compiles nothing new. The `with_capacity` is its named remedy for the honest residue it prices: `Zeroizing` wipes the *current* allocation, and a DSN typed one key at a time would otherwise leave residue at every intermediate capacity. **Priced, not claimed solved** — see Risks. |
| D4 | **Four requests, one reply.** `ConnectionInfo` (read), `SetDsn(Dsn)`, `ClearDsn`, `RebuildCache`; all four answer `StoreReply::Connection(ConnectionSnapshot)` or `Failed`. `REQUEST_NAMES` in `htui::connection` = `["connection_info", "set_dsn", "clear_dsn", "rebuild_cache"]`, matching `StoreRequest::name()`'s arms. | M3/M4/M5's shape, one milestone on: a write answers the **re-read snapshot** so the section never patches local state. `StoreRequest::name()` is what reaches `Failed` and the logs (`store_worker.rs:477-538`), and `"set_dsn"` is already the right string — ANA-10 §4.9(3) noted the existing mechanism does the right thing untouched. |
| D5 | **No compare-and-set, and this is not an exception to `R-ENT-10`.** The three writes are guarded only by the one-write-in-flight `busy` guard of M3 D7. There is no `*Stale` reply. | `R-ENT-10` governs entity rows with an `updated_at` the trigger maintains (PRD D8). The keyring is a single-slot OS secret with no token to compare — `keyring::Entry::set_password` replaces whatever is there (`secret.rs:33`) — and `CacheStore::rebuild` is idempotent. Inventing a token here would be inventing a fact. The honest guarantee is narrower and is what the section says: one write at a time, and the snapshot after it is the truth. |
| D6 | **The redirect is `App`-driven and one-shot, not overlay-driven.** `App::start` issues `ConnectionInfo` under `Origin::App` beside `StoreState` (`app/state.rs:231`); `on_app_reply` sees `Connection(snap)` with `snap.dsn_stored == Some(false)` and, **once per session** (`connection_redirect_done: bool`), emits `Action::Tab(TabAction::FocusSection(SettingsTab::ID, SectionId("connection")))`. | ANA-10's emitter was the local-only overlay, which MOD-25 withdrew; the names it froze survive and are used unchanged (§4.5 sub-verdict 3). The precedent for "App observes a reply and steers the UI once per session" is shipped and tested: `migration_prompt_shown` + `offer_migration_prompt` (`app/state.rs:184`, `app/update.rs:211-245`), which exists because `StoreState` is re-read every fourth tick. |
| D7 | **`TabAction::FocusSection(TabId, SectionId)` and `SettingsRegistry::focus(SectionId) -> bool`.** `update_tab` gains one arm: focus the tab, then — if the now-active tab is the addressed one — hand the `SectionId` down. The hand-down is a new `Tab` method with a default body: `fn focus_section(&mut self, _section: SectionId) -> bool { false }`, overridden by `SettingsTab` alone. | The two names are frozen by ANA-10 M0 (`:963-967`) and must not be renegotiated. `TabAction` is `Copy` and `SectionId` is `Copy` (`settings/mod.rs:117`), so the variant changes no derive. A default-bodied `Tab` method is `captures_input`'s own precedent one level down (`settings/mod.rs:142-144`) — it would be the `Tab` trait's first (`registry.rs:33-49`) — and it keeps `App::update_tab` from matching on a concrete tab type. `SectionId` stays at `settings/mod.rs:117` and is imported by `action.rs` and `registry.rs`: the frozen signature `FocusSection(TabId, SectionId)` already decides that coupling, and rehoming a frozen name buys nothing. |
| D8 | **The user lands on the field, and the section decides that — not a third action variant.** On a snapshot with `dsn_stored == Some(false)`, `ConnectionSection` opens its editor itself, once (`opened_for_empty: bool`, reset on `on_scope_change`). | ANA-10 §4.9(1) considered exactly these two arrangements and adopted this one: it keeps M0's names frozen and puts the decision in the component that owns the state, where the alternative widens a `Copy` enum for one caller. |
| D9 | **`ConnectionInfo` is served from `&Backend` in `try_serve`; the three writers live in the loop's own `match`.** The loop arm re-uses `connection::snapshot` and then fills `last_attempt` from worker-local state. `try_serve`'s arms for the three writers answer `Failed { message: "no connection worker in this build" }`. | `try_serve` is a free function with no access to `backend`-as-`&mut`, `reconnect`, `refresher` or the connect context — every writer needs at least two of them. The loop `match` is where `StoreState` and `ApplyMigrations` already live for the same reason (`store_worker.rs:1054-1081`), and the `Failed`-in-`try_serve` shape is the runtime variants' shipped precedent (`:901-916`, "no agent runtime in this build"). `try_serve` has **no wildcard arm**, so the four variants force four arms — a *guarded* arm would be `E0004` (M3 F-12). |
| D10 | **`Backend::Memory` answers `dsn_stored: None`, and the three writers refuse on it.** The snapshot's `dsn_stored` is `Option<bool>`: `None` means "this build has no keyring story". | `--demo` and every `Harness` test run on `Backend::memory` (`lib.rs:74-82`, `testkit.rs:96-116`), and `testkit::Harness::settle` serves requests **inline through `store_worker::serve`** (`testkit.rs:360-388`). Without this arm, a snapshot test would read — and a careless one would *write* — the developer's real keyring. `None` also correctly suppresses D6's redirect in demo mode. |
| D11 | **The `SetDsn` sequence is fixed, and its order is the whole point**: (1) `Dsn` already parsed on the UI side of the seam (it is a `Dsn`, so it parsed); (2) `spawn_blocking(secret::set_dsn)`; (3) `CacheStore::open(root, new_fingerprint, PgStore::schema_version())`; (4) abort the refresher if one is running; (5) `old_cache.close().await`; (6) `backend = Backend::Offline { cache: new, since: None }`; (7) `reconnect = Some(applied.reconnect)`; (8) spawn one immediate dial onto `events_tx`. Any failure before (6) leaves the backend untouched and answers `Failed`. | Every step is forced by a checked fact. The keyring first, because a mirror opened for a DSN that was never stored is a lie the next launch inherits. The new mirror before the swap, because `go_online` moves `backend.cache()` across **without re-opening** (`store_worker.rs:1196-1205`) — a stale handle here would answer this database's reads from another database's cache (PRD's own named risk, `:373`). `close()` before dropping, because "on Windows an open handle makes the file undeletable … dropping the last handle closes the pool too, but asynchronously, which is exactly the race this avoids" (`cache/mod.rs:216-224`). `since: None` renders `connecting` (`backend.rs:93`), which is what the top bar should say while the dial is in flight. |
| D12 | **`--offline` is honoured: `SetDsn` stores and re-opens but does not dial**, and the notice says so — `stored; this session was started with --offline, so it takes effect on the next launch`. `ConnectContext` carries the flag. | `connecting = !opts.offline && dsn.is_some()` (`connect.rs:189`) is the shipped rule and `--offline` is a deliberate session choice. Silently overriding a flag the user typed is the kind of helpfulness this repo's constraints keep refusing (PRD `:219-223`, `:99-103`). |
| D13 | **`ClearDsn` clears the keyring and drops `reconnect`; it does not tear down a live connection.** Notice: `the DSN is gone from the keyring — this session keeps its current connection until you quit`. | Dropping `reconnect` is required: the closure captures the old DSN by move (`connect.rs:198-208`), so leaving it armed re-dials with a credential the user just deleted. Tearing down a live `Online` backend is *not* required by anything and would throw away a working pool and refresher to no security end — the pool is not the secret, and `R-SEC-2` is about exposure to agent sessions, which this path never crosses. |
| D14 | **`RebuildCache` is `backend.cache()` → `rebuild()` → re-snapshot, behind one `y`/`n` confirmation** whose copy names both sides: **survives** the file, `schema_version`, `db_fingerprint`, `built_at`, and the `pending/` buffer; **goes** the 16 `MIRRORED_TABLES`, `cache_cursor`, `last_full_refresh_at`. It is never extended to clear anything else. | PRD D10 and `HANDOFF.md:321-323`. The copy is derived from `rebuild`'s own doc comment and body (`cache/mod.rs:166-194`), not from memory. One confirmation, not two: nothing here is irreversible — the next refresh pass refills from zero — which is exactly why M3 D13's typed-slug ceremony would be theatre. |
| D15 | **The section is read-only about connection *state*; it never derives it.** The label comes from `Backend::label()` inside the snapshot; the section parses no string and infers nothing from `offline · <age>`. | `store_worker.rs:209-216`'s shipped rule, quoted in ANA-10 §4.5(2): no view may derive a backend fact from the top bar's string. |
| D16 | **Rows and keys.** Rows: `Status` (label + last attempt), `DSN` (`stored — <summary>` / `not stored`), `Mirror` (fingerprint's first 12 chars, `built_at`, `last_full_refresh_at`, `schema_version`), `Rebuild cache` (an action row). Keys: `e` enter/replace the DSN (masked editor), `c` clear the DSN (confirm), `R` rebuild the mirror (confirm), `r` re-read, `j`/`k` move, `Esc` clears a notice. | M5's key set with one addition; `R` is upper-case because `r` is the shipped re-read on every section and a destructive action must not sit one shift away from a reflex. |
| D17 | **`captures_input` is `!matches!(self.mode, Mode::Browse)`**, the same one-liner the other three sections carry, so the masked editor keeps `h`/`l`/`[`/`]` as characters. | `settings/mod.rs:300-306` checks it before the cycle match; a DSN containing `l` or `[` is not exotic. This is ANA-10 §4.9(2)'s named fix, shipped by M3 and here getting the consumer it was built for. |
| D18 | **Tests**: `crates/htui/tests/connection.rs`, worker half through `store_worker::serve` and a driven worker task, section half through `SectionBench` + `Harness`, `connection__*.snap` snapshots. Keyring is faked by a new `htui_store::testkit::mock_keyring()` (installs `keyring::mock::default_credential_builder()` under a `Once`), never the real one. The headline test is **Postgres-gated**: empty keyring → `SetDsn(live DSN)` → the worker reaches `label() == "online"` and the mirror directory is the new fingerprint's. | M3 D14 / M4 D18 / M5 D16, one file across. The Postgres gate is what makes "reaches `Online` without a restart" (PRD `:254`) a verified claim rather than a hope: the `--demo` smoke cannot be run in the agent environment (no TTY), which M3, M4 and M5 all recorded. |
| D19 | **Registered after `prompt`**, title `Connection`, so the strip reads `Agents  Hierarchy  Kinds  Prompt  Connection`. PRD open question 2 (does `agents` stay first) stays open. | M4 D17 / M5 D15: registration order is strip order and appending moves no existing line. Five titles are **46** of the pinned 100-column harness — the pin at `tests/settings.rs:942-957` is re-run with the fifth section, never relaxed; its own doc says the fifth is what it exists to catch. |
| D20 | **`Started` gains `pub connect: Option<ConnectContext>`** (`config_root`, `connect_timeout`, `offline`), `None` for `Started::detached`. | The worker needs the root and the timeout to open a mirror and build a `Reconnect`, and it has neither today (`Started`'s six fields, `connect.rs:103-119`). Calling `identity::config_root()` in the worker instead would reach the developer's real config directory from every test. Cost is exactly three sites: the literal at `connect.rs:224`, `detached` at `:138`, the destructure at `store_worker.rs:1016` — there is no other `Started { … }` in the tree. |

## Patterns to Mirror

| Concern | Pattern | Where |
|---|---|---|
| Worker module: snapshot type, `snapshot()`, `serve()`, `REQUEST_NAMES` | `htui::prompt_settings` | `prompt_settings.rs` (M5), `catalogue.rs:25-108` |
| Or-ed `try_serve` arm, **no guard** (a guarded arm is `E0004`) | the three prompt-settings patterns | `store_worker.rs:949-951` |
| A request the loop keeps for itself because it needs loop state | `StoreState`, `ApplyMigrations` | `store_worker.rs:1055-1081` |
| "Not available in this build" refusal from `try_serve` | the thirteen runtime variants | `store_worker.rs:901-916` |
| Blocking OS call served off the UI task | `hierarchy::canonical`, `spawn_blocking` | `hierarchy.rs:405-429` |
| Backend swap owned by the worker loop | `go_online` / `go_offline` | `store_worker.rs:1190-1240` |
| Section skeleton: `Row`, `Editor`, `Mode`, `busy`, `Notice`, hint line, `on_reply` routing by `REQUEST_NAMES` | `PromptSection` (single-field editor) | `ui/tabs/settings/prompt.rs:93-258`, `:465-565`, `:853-912` |
| Confirm stage inside a section | `KindsSection`'s delete confirm / `DeleteStage` | `kinds.rs:309`, `hierarchy.rs:193-209` |
| Hand-written `Debug` that prints labels, never a buffer | `Editor` / `Mode` impls; `TextField` | `prompt.rs:138-169`, `text_field.rs:44-52` |
| One-shot, session-guarded UI steer from a reply | `migration_prompt_shown` + `offer_migration_prompt` | `app/state.rs:184`, `app/update.rs:211-245` |
| Section tests: bench, keys, replies, snapshots | `tests/prompt_settings.rs` | `crates/htui/tests/prompt_settings.rs:1-13`, `:559-607` |
| Multi-phase HANDOFF paragraph | "Milestone N landed (…)" appended to the one checklist line | `HANDOFF.md:324`, `:352`, `:376`, `:415`, `:460`; `.claude/rules/workflow-docs.md` lifecycle 4 |

## Files to Change

| File | Action | Task | Why |
|---|---|---|---|
| `crates/htui-store/src/dsn.rs` | **new** | T1 | D1/D2/D3: `Dsn`, `DsnError`, `parse`, `summary`, redacting `Debug` |
| `crates/htui-store/src/connect.rs` | edit | T1 | D1/D20: `reconnect_for`, `apply_dsn`, `forget_dsn`, `Applied`, `ConnectContext`, `Started.connect` |
| `crates/htui-store/src/lib.rs` | edit | T1 | `pub mod dsn;` + re-export beside `:26` |
| `crates/htui-store/src/testkit.rs` | edit | T1 | D18: `mock_keyring()` |
| `crates/htui-store/Cargo.toml`, `Cargo.toml` | edit | T1 | D3: `zeroize` promoted to a declared workspace dependency |
| `crates/htui-store/tests/dsn.rs` | **new** | T1 | D2/D3: the vocabulary, the summary, the redacting `Debug`, the keyring round trip under the mock |
| `crates/htui/src/connection.rs` | **new** | T2 | D4/D9/D10: `ConnectionSnapshot`, `snapshot()`, `serve()`, `REQUEST_NAMES` |
| `crates/htui/src/lib.rs` | edit | T2 | `pub mod connection;` |
| `crates/htui/src/store_worker.rs` | edit | T2 | D4/D9/D11/D12/D13/D14: four variants + `name()` arms, one reply variant, one `try_serve` arm group, the loop arms, `let mut reconnect`, `last_attempt` |
| `crates/htui/tests/connection.rs` | **new** | T2 (worker), T3 (section) | D18 |
| `crates/htui/src/ui/tabs/settings/connection.rs` | **new** | T3 | D8/D14/D16/D17: the section |
| `crates/htui/src/ui/tabs/settings/mod.rs` | edit | T3 | D7/D19: `pub mod connection;`, re-export, `SettingsRegistry::focus` |
| `crates/htui/src/ui/tabs/registry.rs`, `ui/tabs/mod.rs` | edit | T3 | D7: `Tab::focus_section` default body |
| `crates/htui/src/app/action.rs` | edit | T3 | D7: `TabAction::FocusSection(TabId, SectionId)` |
| `crates/htui/src/app/update.rs` | edit | T3 | D6/D7: the `update_tab` arm; `on_app_reply` redirect |
| `crates/htui/src/app/state.rs` | edit | T3 | D6: `ConnectionInfo` at start, `connection_redirect_done` |
| `crates/htui/src/app/mod.rs` | edit | T3 | D19: register after `prompt` (`:51`) |
| `crates/htui/src/ui/text_field.rs` | edit | T3 | D3: `with_capacity(256)` + zeroize on `clear()`/`take()` when masked |
| `crates/htui/tests/settings.rs` | edit | T3 | D19: strip pin now covers five sections (`:942-957`) |
| `crates/htui/tests/snapshots/connection__*.snap` | **new** | T3 | D18 |
| `README.md` | edit | T4 | PRD `:201`: the in-app DSN path beside `--set-dsn` |
| `HANDOFF.md` | edit | T4 | "Milestone 6 landed" paragraph **and** the correction at `:289` (below) |
| `.claude/prds/mod-15-hierarchy-management.prd.md` | edit | T4 | milestone table `:254`: row 6 → complete, plan link |

**A correction T4 owes, discovered by this plan's fact-check.** `HANDOFF.md:289` says
`Settings > Rebuild cache` calls `CacheStore::rebuild()`, and the PRD's Evidence (`:99-103`) says
`rebuild`'s "only callers are tests". **Both are wrong in different directions**: there is no
Settings action (correct in the PRD, which is why D14 builds one), but `rebuild` already has **two
production callers** — `hierarchy.rs:339` (project delete, M3) and `catalogue.rs:172` (kind delete,
M4). The close-out corrects the HANDOFF line rather than repeating either claim.

Not changed: `crates/htui-core/**` (no seam method, `CASES` stays 36), `crates/htui-store/src/pg/**`,
`migrations/**`, `.sqlx/**`, `crates/htui-agent/**`, `cli.rs` (ANA-10 §4.9(3): "never argv — the
field is not a CLI path at all; `cli.rs` is untouched"), `chat/composer.rs`.

## Tasks

**T1 → T2 → T3 → T4, serial.** T2 compiles against T1's `Dsn`, `ConnectContext` and `apply_dsn`;
T3 against T2's requests and snapshot; and the file sets T2 = {`connection.rs`, `lib.rs`,
`store_worker.rs`, `tests/connection.rs`} and T3 = {`settings/connection.rs`, `settings/mod.rs`,
`tabs/registry.rs`, `app/*.rs`, `text_field.rs`, `tests/connection.rs`, `tests/settings.rs`,
snapshots} **intersect on `tests/connection.rs`**. No independent pair exists — which is why the
routing verdict recommends ultracode for the review phase only.

Each implementer commits its own work incrementally — uncommitted subagent work does not survive the
session. Before blaming a Postgres failure in any gate, check `df -h /`: `target/` fills the disk on
this box.

TDD per task: tests first, red, then code. Every implementer prompt carries: PRD D9/D10 and
ANA-10 §4.9 win over this plan where they disagree; ANA-10's **local-only half is withdrawn** and
must not be built; graphify-first for codebase questions; **no `WriteStore` change, no migration, no
new `query!`**; a section holds no store handle; no `Debug` prints a DSN or a field buffer; nothing
logs on a validation failure; `unsafe_code = "forbid"`, MSRV 1.98.

### Task 1: the DSN type and the connect seam (`htui-store`)

- **Files**: `crates/htui-store/src/dsn.rs` (new), `connect.rs`, `lib.rs`, `testkit.rs`,
  `crates/htui-store/Cargo.toml`, workspace `Cargo.toml`, `crates/htui-store/tests/dsn.rs` (new).
- `Dsn::parse(&str) -> Result<Dsn, DsnError>`; `DsnError` = the five-variant fixed vocabulary (D2)
  with `Display`, `Debug` derived (it carries no input); `Dsn::summary()`, `Dsn::fingerprint()`,
  `Clone`, hand-written `Debug`. No public text accessor (D1).
- `ConnectContext { config_root: PathBuf, connect_timeout: Duration, offline: bool }`;
  `Started.connect: Option<ConnectContext>` (D20), filled by `start`, `None` in `detached`.
- `connect::reconnect_for(dsn: &Dsn, root: PathBuf, timeout: Duration) -> Reconnect` — the factory
  ANA-10 §10.13 named, extracted from `start`'s closure at `connect.rs:198-208` so `start` uses it
  too (one shape, not two).
- `connect::apply_dsn(dsn: Dsn, ctx: &ConnectContext) -> Result<Applied>` — `spawn_blocking` keyring
  write, then `CacheStore::open` under the new fingerprint, then `reconnect_for`.
- `connect::forget_dsn() -> Result<()>` — `spawn_blocking(secret::clear_dsn)`.
- `testkit::mock_keyring()` — `Once` + `keyring::set_default_credential_builder(keyring::mock::…)`
  (D18). The existing `#[cfg(test)]` installer at `secret.rs:117-122` keeps its `"htui-test"` slot;
  this one is for other crates' integration tests and uses the real `SERVICE`/`USER` under the mock
  backend.
- **Tests (red first)**: each `DsnError` variant from a real input; a valid DSN's `summary` carries
  host/port/database/user/sslmode and **not** the password (assert the password substring is absent);
  `format!("{:?}", dsn)` is exactly `Dsn(<redacted>)`; `apply_dsn` under the mock keyring stores,
  returns a cache whose `dir()` ends in the new fingerprint, and leaves the old directory on disk;
  `forget_dsn` on an empty keyring is `Ok(())`; two DSNs differing only in credentials produce the
  **same** fingerprint (`identity.rs:223-228`'s rule, restated at this level).

### Task 2: the worker half and the in-process transition

- **Files**: `crates/htui/src/connection.rs` (new), `lib.rs`, `store_worker.rs`,
  `crates/htui/tests/connection.rs` (new).
- `ConnectionSnapshot { label: String, writable: bool, dsn_stored: Option<bool>, summary:
  Option<String>, mirror: Option<MirrorInfo>, last_attempt: Option<Attempt> }`;
  `MirrorInfo` from `CacheStore::meta()` + `dir()`; `Attempt { at, outcome }`.
- `connection::snapshot(&Backend) -> Result<ConnectionSnapshot>` (keyring read in `spawn_blocking`,
  skipped entirely on `Memory` — D10); `connection::serve` for the read; `REQUEST_NAMES`.
- `store_worker.rs`: four `StoreRequest` variants + `name()` arms (50 → 54), `StoreReply::Connection`
  (28 → 29), one `try_serve` arm for the read and one or-ed arm refusing the three writers (D9),
  `let mut reconnect`, `let mut last_attempt`, set from the `ConnEvent` arms (`:1133-1155`) and from
  `go_offline`.
- The `SetDsn` arm implements D11's eight steps exactly, D12's `--offline` branch and D13's clear.
  `RebuildCache` per D14.
- **Tests (red first)**: `connection_names_are_stable`; a `Memory` backend answers
  `dsn_stored: None` and refuses all three writers; `SetDsn` on a worker whose `connect` context is
  `None` answers `Failed` and leaves the backend untouched; **the headline, Postgres-gated**: from an
  empty mock keyring, `SetDsn(live DSN)` drives the worker to `label() == "online"` **without a
  restart**, and the live mirror's directory is `<root>/cache/<db_fingerprint(dsn)>`; a second
  `SetDsn` to a different database swaps the mirror directory again and the old file still exists;
  `ClearDsn` empties the keyring and leaves an `Online` backend online; `RebuildCache` empties the
  mirrored tables and keeps `schema_version`/`db_fingerprint`/`built_at`.

### Task 3: the section, the focus action and the redirect

- **Files**: `ui/tabs/settings/connection.rs` (new), `settings/mod.rs`, `ui/tabs/registry.rs`,
  `ui/tabs/mod.rs`, `app/action.rs`, `app/update.rs`, `app/state.rs`, `app/mod.rs`,
  `ui/text_field.rs`, `tests/connection.rs`, `tests/settings.rs`, snapshots.
- `ConnectionSection` per D16/D17, mirroring `PromptSection`'s skeleton; masked editor via
  `TextField::masked()`; submit parses through `Dsn::parse` and renders `DsnError`'s sentence
  verbatim, emitting nothing on a refusal (D2).
- `TabAction::FocusSection` + `Tab::focus_section` default body + `SettingsRegistry::focus` (D7);
  `App` redirect once per session (D6); `TextField` capacity + zeroize (D3).
- Registration after `prompt` and the strip pin re-run at five sections (D19).
- **Tests (red first)**: the field renders `•` per char plus a count and **never** the text
  (`SectionBench` render + a `format!("{:?}")` of the section); a refused DSN emits **no**
  `Action::Store` at all; a valid one emits `SetDsn` and the section goes `busy`; `l`, `h`, `[` typed
  into the editor stay characters (`captures_input`); `c` and `R` each need their confirmation;
  the rebuild copy names both lists; `FocusSection` selects the section from any tab and is a no-op
  for an unknown `SectionId`; the redirect fires once and not again after the fourth-tick re-read;
  `dsn_stored: None` (demo) never redirects; snapshots `connection__{empty,stored,editor,confirm}`.

### Task 4: docs and bookkeeping

- README's connection section beside `--set-dsn`; `HANDOFF.md` milestone-6 paragraph **and** the
  `:289` correction; PRD milestone row 6 → complete with the plan link. Close-out per
  `references/lifecycle.md` P2 (this is the item's last milestone): the whole MOD-15 entry archives
  as **one** write-up at `docs/decisions/mod/mod-15.md` with **one** `DECISIONS.md` index line
  covering all six milestones.

## Validation

1. `cargo fmt --all -- --check`
2. `cargo clippy --workspace --all-features --all-targets -- -D warnings`
3. `cargo test --workspace --all-features` (README `:457`) — **and** the Postgres-live run,
   `HTUI_TEST_DATABASE_URL=postgres://postgres:htui@localhost:5439/postgres cargo test --workspace
   --all-features` (README `:471`), which is the only run where T2's headline test executes.
4. `cargo check -p htui-store` and `cargo tree -p htui-store -i zeroize` — the promoted dependency
   compiles no new crate (D3).
5. `bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh` at close-out.
6. **The `--demo` smoke is the maintainer's**: the agent environment has no TTY, which M3, M4 and M5
   each recorded. What it would show here — an empty keyring landing on the connection section with
   the field open — is pinned by tests instead (D6/D8), and the pin is named in the close-out.

## Risks

| Risk | Mitigation |
|---|---|
| The mirror swap leaves the shell reading the old database's cache | D11's ordering, and T2's test asserting the live mirror directory equals the new fingerprint's after `SetDsn` |
| `Zeroizing` is claimed to erase more than it does | D3 prices it in the doc comment: the current allocation is wiped, `with_capacity(256)` removes the realloc trail for realistic DSNs, and the terminal's own input buffering is out of reach. ANA-10 §11 risk 19, carried forward unchanged |
| A valid-but-wrong DSN now produces far more `ConnEvent::Failed` traffic, which reaches the status line **and** `tracing::warn!(%why)` → the `--log` file (`store_worker.rs:1153`) | Pre-existing path, unchanged here, and `why` is sqlx's connection error, not the DSN. ANA-10 §11 risk 20; the section shows `last_attempt` so the user sees it in the app rather than in a log |
| The keyring is absent on a headless Linux box; a real `get_dsn` failure is **fatal at startup** (`connect.rs:181` → `lib.rs:77-81`) | Unchanged by this milestone and not made worse: the new read path is on the worker, where a failure is a `Failed` reply, not an exit. Real-backend behaviour stays MOD-16's (PRD `:239-240`) |
| Five sections overflow the pinned 100-column strip | 46 of 100 columns after this milestone; the pin at `tests/settings.rs:942-957` is re-run, never relaxed |
| `Started` grows a field and a test constructs it positionally | There is exactly one `Started { … }` literal and one destructure in the tree (D20) |
| A section test reads or writes the developer's real keyring | D10 (`Memory` never touches it) plus D18's mock installed by `htui_store::testkit::mock_keyring()` |
| The redirect fights the user, re-focusing Settings every fourth tick | D6's one-shot session guard, which is the defect `migration_prompt_shown` exists to prevent, with its shipped test as the precedent |

## Verified claims (fact-check, 2026-09-17)

| # | Claim | Verdict | Evidence |
|---|---|---|---|
| F-1 | `crates/htui` has no `sqlx` in `[dependencies]`, so ANA-10 §4.9(5)'s UI-side parse cannot live there — D1 moves it into `htui-store` | **true** | `crates/htui/Cargo.toml`: `sqlx` appears under `[dev-dependencies]` only |
| F-2 | `keyring::mock::default_credential_builder()` is reachable from a normal (non-`cfg(test)`) dependency build, so D18's helper can live in `htui-store::testkit` | **true** | compile probe against this tree's toolchain, `cargo check -p htui-store --test probe_tmp --all-features` — green |
| F-3 | `PgConnectOptions` exposes `get_username`/`get_host`/`get_port`/`get_database`/`get_ssl_mode` and **no password getter**, so D2's summary is safe by construction | **true** | same probe compiled every getter; no `get_password` exists in sqlx 0.9.0's option set |
| F-4 | `identity::config_root() -> Result<PathBuf>` exists and is public | **true** | same probe; `identity.rs:45` |
| F-5 | `Backend::Offline { cache, since }` has public fields and is constructible from the `htui` crate | **true** | `backend.rs:46-68`; `store_worker.rs:1205` already builds `Backend::Online { … }` |
| F-6 | `try_serve`'s match has **no wildcard**, so four new variants force four arms | **true** | `store_worker.rs:873-956`; the runtime group's `Failed` precedent at `:901-916` |
| F-7 | `StoreRequest` is 50 variants and `StoreReply` 28 today → 54 / 29 after this milestone | **true** | `store_worker.rs:75-475`, `:542-690`, `name()` at `:477-538` |
| F-8 | `zeroize` is compiled today only as a transitive dependency, absent from `[workspace.dependencies]` | **true** | `Cargo.lock:5124`; no `zeroize` in the workspace or crate manifests |
| F-9 | `CacheStore::rebuild()` **already has two production callers** — the PRD's "only callers are tests" is wrong | **refuted** | `hierarchy.rs:339`, `catalogue.rs:172`; `.claude/prds/…prd.md:99-103`, `HANDOFF.md:289`. T4 corrects the HANDOFF line |
| F-10 | `CacheStore::close()` exists and its doc names "the store worker before it hands the directory to something else" | **true** | `cache/mod.rs:216-224` |
| F-11 | `Started` has exactly one literal construction and one destructure in the tree, so D20 costs three sites | **true** | `connect.rs:224`, `connect.rs:138` (`detached`), `store_worker.rs:1016`; grep for `Started {` finds no other |
| F-12 | `reconnect` is an immutable binding and is never reassigned — a new DSN cannot re-arm the dial today | **true** | `store_worker.rs:1022`, ticker guard `:1162-1171` |
| F-13 | `SettingsRegistry` has no `focus`/`select` — only `cycle_next`/`cycle_prev` — so D7 adds the first addressable jump | **true** | `settings/mod.rs:179-228` |
| F-14 | `TabAction::FocusSection`, `SectionId("connection")` and any DSN request have **zero** occurrences in `crates/` | **true** | grep over `crates/`; hits are confined to `docs/ANA-10.md`, the PRD and plans |
| F-15 | `testkit::Harness::settle` serves requests inline through `store_worker::serve`, so a careless snapshot test would reach the real keyring without D10 | **true** | `testkit.rs:360-388` |
| F-16 | Five section titles cost 46 of the pinned 100 columns | **true** | `" Agents "` 8 + `" Hierarchy "` 11 + `" Kinds "` 7 + `" Prompt "` 8 + `" Connection "` 12; pin at `tests/settings.rs:942-957` |
| F-17 | `db_fingerprint` ignores user, password and query parameters — replacing only a password keeps the same mirror | **true** | `identity.rs:132-143`, asserted at `:223-228`. D11's swap is therefore a no-op for a password rotation, which T2's test states rather than assumes |
| F-18 | `TextField::masked()` has no consumer in `src/` today; `text()` answers `None` when masked and `take()` is the only read | **true** | `text_field.rs:61-75`, `:156-164`; mask used only in its own unit tests |
| F-19 | The tree has **no CI**; `tests/connection.rs` under `#![cfg(feature = "testkit")]` runs only under `--all-features` | **true** | no `.github/` in the tree (M5 live coordinate 8); `crates/htui/Cargo.toml` `testkit = []` |
| F-20 | ANA-10's verdict is withdrawn, so only §4.9 and §4.5's two frozen names may be built from it | **true** | `HANDOFF.md:66-68`, `docs/decisions/mod/mod-25.md` |
| F-21 | The `Tab` trait has **no** default-bodied method today, so D7's `focus_section` is its first | **true** | `registry.rs:33-49` (seven required methods); the same move `SettingsSection::captures_input` made one level down, `settings/mod.rs:142-144` |
| F-22 | `Backend::label()` renders `"connecting"` for `Offline { since: None }`, which is what D11 step (6) relies on | **true** | `backend.rs:89-101` |

## Acceptance

- A launch with an empty keyring opens the Settings tab on `Connection` with the masked field open,
  and the top bar reads `offline · 0s` until a DSN is typed.
- A typed, valid DSN is stored in the keyring, opens the mirror under its own fingerprint, arms the
  reconnect and reaches `online` **in the same process** — asserted against a live Postgres.
- A typed, invalid DSN emits no request, stores nothing, logs nothing, and shows one of five fixed
  sentences.
- No frame, no `Debug` output, no log line and no file in the tree carries the DSN text; the summary
  carries no password because the type it is built from cannot produce one.
- `Clear` empties the keyring, disarms the reconnect and says what it did not do; `Rebuild cache`
  empties the 16 mirrored tables plus the cursors and names both sides before acting.
- `CASES` 36, `EXPECTED_CASES` 36, `.sqlx/` byte-identical, no new migration.
- `cargo fmt`, `cargo clippy -D warnings` and the Postgres-live `cargo test --workspace
  --all-features` are green; `validate-workflow-docs` is green at close-out.
