# MOD-15 - Workspace, project, repo and kind management (done, 2026-09-17)

**MOD-15 - Workspace, project, repo and kind management** (from MOD-1). `R-ENT-1..4`,
`R-ENT-6`, `R-BOX-4`, `R-TUI-8`. Create and edit workspaces, projects (seeded kinds, graphs and
templates per `docs/ANA-9.md` §5.10), repos with primary flag and per-box paths, workspace root
paths per box; item kind editor with the prefix-change warning (§10); Settings tab sections for
kinds and step graphs per project. Not blocked (MOD-6 landed, `docs/decisions/mod/mod-6.md`;
`Settings > Rebuild cache` calls `CacheStore::rebuild()`). Seed graphs per `docs/ANA-2.md` §4.1
(`review` in `implement`/`fix` `input_kinds`, `gate_hard` on the **feature** graph's `prd` and
`plan` and the **analysis** graph's `verdict` and **none elsewhere** — this line previously read
"`prd`, `plan` and `verdict`", which would have hardened `plan` in the refactor and tooling
graphs too and taken MOD-12's first unattended targets away; PRD D3 and `docs/ANA-2.md` §10
item 5 rule the narrow reading, and milestone 2 shipped it. `is_override = false` needs no
write: the column arrives with MOD-4's `0003` defaulting to `false`);
whichever of MOD-15 and MOD-4 lands second owns the seed amendment. Per
ANA-5 (`docs/ANA-5.md` §5.3, §5.4, §9): seed ten `prompt_template` rows per project (eight phase
names plus reserved `judge` and `handoff`, amending `docs/ANA-9.md` §5.10), the phase editor
refuses the two reserved names, and the Settings tab exposes the ten `app_setting` keys.
**MOD-2 shipped the read half of those ten and none of the write half** (closed 2026-09-15,
`docs/decisions/mod/mod-2.md`): `prompt::settings::{DEFAULTS, resolve_budget, resolve_reserve_bp,
resolve_hops, resolve_max_skill_tokens, resolve_excerpt_caps}` resolve each key through
phase → `project.settings` → `app_setting` → the compiled table, and `trim_record.budget_source`
records which rung answered. There is **no `app_setting` writer on the store seam**, so today the
only way to change `token_budget` (default **120 000**, with a 10% response reserve) or any other
of the ten is SQL against the database — this item is what makes them editable from the app. The
phase rung is the finest-grained one and has no editor either; `ResolvedPhase.token_budget` is
read but never written. **Creating a workspace on a box with no server does not exist** (MOD-25
closed 2026-09-16, `docs/decisions/mod/mod-25.md`): `htui` is online-only, so every create path
here is a server-backed path and "Not blocked" holds without qualification. This item builds and
registers the Settings connection **section** (`SectionId("connection")`, named by ANA-10 M0 so
neither item has to guess) — **and now owns the masked DSN field inside it**, which MOD-25 handed
back rather than building scaffolding it does not own (MOD-25 close-out, maintainer decision
2026-09-16). Nothing exists yet: `SectionId("connection")`, `MaskedField` and
`TabAction::FocusSection` have zero occurrences in `crates/`. Also:
no ad-hoc focus mechanism instead of M0's `TabAction::FocusSection` /
`SettingsSection::captures_input` names — **`MaskedField` is superseded**: milestone 3 shipped
one `TextField` with a mask flag rather than a masked type, so the connection section builds on
`TextField::masked()` (`crates/htui/src/ui/text_field.rs`); no second persistence location for a
"shown once" marker (ANA-10 §5.4's `local_setting` is withdrawn with the mode); and
`Settings > Rebuild cache` stays as written — it is safe because the local store is a different
file that `MIRRORED_TABLES` never names, so it must not be "helpfully" extended to clear local
data, and its confirmation copy should say what it does and does not delete.
**Milestone 1 landed (`0d48a71`..`806f6b6`, 2026-09-16): the seam can write the hierarchy.**
Routed as PRD (`.claude/prds/mod-15-hierarchy-management.prd.md`, six milestones, maintainer
decisions D1–D13); plan at `.claude/plans/mod-15-hierarchy-seam.plan.md`, blueprint beside it.
`WriteStore` grew **31 methods** — 21 writers, `delete_reach`, 9 readers — for `workspace`,
`workspace_project`, `workspace_box_path`, `project`, `repo`, `repo_box_path`, `item_kind`,
`step_graph` and `step_graph_phase`, plus a typed `SettingKey`/`SPECS` registry in
`htui_core::prompt::settings` and the rung-aware `set_setting`/`clear_setting`/`setting` over
`App | Project | Phase`. **No migration**: every table was already in `0001_init.sql`, so
`0003_orchestration.sql` is still MOD-4's and still next. Conformance `CASES` **23 → 35**,
`READ_CASES` unchanged at 6; **1032 passed, 0 failed, 30 ignored** workspace-wide with Postgres
live. Every edit is a compare-and-set on `updated_at` (no `version` column exists and none was
added); `project.settings` is merged key by key, never round-tripped through a struct, so the
keys MOD-4 and MOD-12 will add cannot be erased; validation refuses what the reader would clamp
or ignore rather than clamping. **Three live coordinates open items need.** (1) `WriteStore` now
has **six** implementors, not four — `MemStore`, `PgStore`, `Writer`, `BufferedWriter`, plus
`UsageSpy` (`crates/htui-agent/src/conformance.rs:645`) and `SpyStore`
(`crates/htui-agent/tests/recorder.rs:323`), both invisible to `cargo check -p htui-store`; the
trait has no default bodies, so a new seam method costs six impls. (2) `project.settings` holds
`upstream_hops` under a **different name** than the `app_setting` key `prompt_upstream_hops` —
`SettingSpec::project_key` carries the mapping, and a writer that ignores it writes a key the
reader drops. (3) `run_step_commit.repo_id REFERENCES repo(id)` has **no cascade**
(`0001_init.sql:503`), so a project holding one cannot be deleted at all — latent because nothing
writes that table yet, and the fix is a migration, which makes it MOD-4's to carry in `0003`.
Reviewed by `rust-reviewer`: no CRITICAL or HIGH; 3 MEDIUM and 6 LOW all fixed
(`18459a5`..`806f6b6`), the MEDIUMs being a `delete_project` count/delete snapshot race
(now `REPEATABLE READ` with a bounded retry), a MemStore/PgStore divergence on
stale-token-plus-invalid-input, and a self-confirming cascade assertion (now measured against
`count(*)` per table).
**Milestone 2 landed (`e0aa62a`..`0e11c42`, 2026-09-16): a created project is a working
project.** `create_project` now seeds, on both stores and inside the transaction milestone 1
shaped for it, the 35 rows every project is born with: 5 `step_graph`, 15 `step_graph_phase`,
5 `item_kind` and the 10 `prompt_template` rows of `DEFAULT_TEMPLATES` at version 1. Their one
source is the new **`htui_core::seed`** module (`crates/htui-core/src/seed.rs`, `KINDS` plus four
row constructors), which the demo fixture now builds from as well — `fixtures.rs` lost
`KindSpec`, `KIND_SPECS` and `TEMPLATE_NAMES`, so a seed change cannot land in the product and
miss the fixture, or the reverse. ANA-2 §4.1's two amendments are applied at seed time and in the
fixture: `implement.input_kinds = ['plan','review']` on feature, refactor and tooling (PRD D5
extends ANA-2's feature-only wording), `fix.input_kinds = ['reproduce','review']` on bug, and the
three `gate_hard` flags of the corrected sentence above. **The seed is not parameterised**: there
is no unseeded create, `NewProject` and the `WriteStore` signature are unchanged, and all six
implementors compile untouched. `item_key_counter` is still never seeded (the first mint on a
fresh project is `FEAT-1`), `project.settings` is still `set_setting`'s alone, and no seed
statement binds `created_at`/`updated_at` — note `step_graph_phase` and `item_kind` have **no
`created_at` column at all**. No migration: `0003_orchestration.sql` is still MOD-4's and still
next. Conformance `CASES` **35 → 36** (`project_create_seeds_the_catalogue`, which asserts the
amended table as a literal independent of `KINDS`), `READ_CASES` unchanged at 6, three
per-backend twins added on each store, and 4 new `.sqlx` files; **36/36 conformance cases ran
against Postgres, none skipped**. Reviewed by `rust-reviewer`: no CRITICAL, one HIGH — a public
doc comment linking the private `seed_project`, which `rustdoc::private_intra_doc_links = "deny"`
turned into a broken `cargo doc` for the whole crate (`077cd0d`) — and two LOW, one fixed
(`0e11c42`), one deferred with reason (`seed_project`'s length, whose shape the blueprint fixed
and which matches the crate's existing writers).
**Milestone 3 landed (`171cf3c`..`524f069`, 2026-09-17): the app can take typed input.** Plan at
`.claude/plans/mod-15-hierarchy-section.plan.md`, blueprint beside it. Three things shipped.
(1) **`TextField`** (`crates/htui/src/ui/text_field.rs`): one single-line field with a char
cursor, insertion, a render-time window that leads with `…`, and an optional mask — the widget
PRD D1 owes MOD-22 and MOD-23. Its `Debug` is **hand-written and never prints the buffer**, and
`text()` answers `None` when masked (`take()` is the only read of a masked field); the mask has
**no consumer until milestone 6**. (2) **`SettingsSection::captures_input`**, the trait's first
default body, checked in `SettingsTab::on_key` before the `h`/`l`/`[`/`]` cycle
(`settings/mod.rs`) — ANA-10 §4.9's named fix, so a section taking text gets `l` as a letter.
(3) **`HierarchySection`** (`ui/tabs/settings/hierarchy.rs`), registered **after** `agents`:
workspaces, projects, repos with the primary flag (`p` moves it, nothing unsets it) and this
box's paths, all edited through milestone 1's CAS seam; a CAS miss reloads, keeps the typed text
and retries only on `Enter` (PRD D8, never auto-retry); delete is `d` → counts from
`delete_reach` → `y` → **typed slug** (PRD D13). **No seam change, no migration, no `query!`**:
`CASES` stays 36, `EXPECTED_CASES` 36, `.sqlx/` byte-identical. **Live coordinates.** (1) The
twelve hierarchy `StoreRequest` variants (`StoreRequest` 26 → 38, names in
`htui::hierarchy::REQUEST_NAMES`) are served by `hierarchy::serve` through **one `try_serve` arm
of twelve or-ed patterns** — `try_serve`'s `match` has no wildcard, so a *guarded* arm is `E0004`
(the plan's fact-check caught this by compile probe before any code was written). (2) Identity is
worker-side: `created_by` from `Backend::this_user`, `box_id` from `Backend::box_info`; **no view
holds a `UserId` or `BoxId`** and `HierarchySnapshot` deliberately carries neither. (3) The
per-box root guard is `htui_core::root_path::canonical_root` (new `crates/htui-core/src/root_path.rs`,
`tempfile` dev-dep added): relative / missing / dangling / not-a-directory refused in that order,
a link **to** a directory stored canonical, and no refusal ever names a link's target (PRD D11;
MOD-7 is the second caller and reuses it). (4) A **project** delete rebuilds the mirror from the
worker (`CacheStore::rebuild`, M1 D5), a **workspace** delete does not — so PRD D13's stale-mirror
hole is closed here rather than waiting for milestone 6's button. (5) `htui::testkit::SectionBench`
(was `Bench` in `tests/settings.rs`) is the shared section bench for milestones 4–6.
**1102 passed, 0 failed, 30 ignored**; 7 new `hierarchy__*.snap`, the two switcher snapshots moved
with their copy, the five `settings__agents_*.snap` byte-identical. The `--demo` smoke was **not**
run (no TTY in the agent environment) — interactive behaviour is pinned by tests only. Reviewed by
`rust-reviewer`: no CRITICAL, one HIGH, three MEDIUM, nine LOW, **all twelve fixed**
(`d566cd6`..`524f069`) — the HIGH being an editor `submit()` with no in-flight guard, where a
second `Enter` created the row, lost the first reply to the staleness index and then reported
"duplicate slug" for a write that had landed. Two residues recorded, neither a defect: a write
and its re-read are still two seam calls, so a connection lost between them reports `Failed` for
a write that applied (closing it needs a seam method this item does not add); and
`chat/composer.rs` was **not** rebuilt on `TextField` (plan D3) — a CLEAN candidate for this
item's close-out.
**Milestone 4 landed (`5e0f24a`..`b1672b8`, 2026-09-17): kinds and graphs are editable.** Plan at
`.claude/plans/mod-15-kinds-graphs.plan.md`, blueprint beside it. Two things shipped.
(1) **`htui::catalogue`** (`crates/htui/src/catalogue.rs`): one scope-wide snapshot per read —
every project of the scope with its kinds, its graphs and each graph's phases — and nine served
requests (`StoreRequest` 38 → **47**, names in `htui::catalogue::REQUEST_NAMES`, three new
replies taking `StoreReply` to 26), routed through **one `try_serve` arm of nine or-ed patterns**
as milestone 3's twelve are. The read is scope-wide and **not** one request per project because
the staleness index is keyed by `(Origin, Discriminant<StoreRequest>)` (`app/state.rs:158`): N
requests of one variant would leave only the newest reply delivered. (2) **`KindsSection`**
(`SectionId("kinds")`, `ui/tabs/settings/kinds.rs`), registered **after** `hierarchy`: the tree is
project → each kind with the phases of its default graph → each graph no kind points at; kind
create/edit/delete, graph create/edit and phase create/edit all land through milestone 1's CAS
methods; a prefix change is read before it is written (PRD D12, a modal stage naming the surviving
keys and counter); a kind delete asks once and rebuilds the mirror, because `item_kind` is one of
the 16 `MIRRORED_TABLES` and cursor-based refresh propagates no delete. **No seam change, no
migration, no `query!`**: `CASES` stays 36, `EXPECTED_CASES` 36, `.sqlx/` byte-identical, and
`0003_orchestration.sql` is still MOD-4's and still next. **`token_budget` is the app's first
settings writer**, scoped to one key on the `Phase` rung through
`set_setting`/`clear_setting` (an empty field clears, so the project/app rung answers); milestone
5 widens it to the ten keys on `App` and `Project`. **Live coordinates.** (1) `catalogue::snapshot`
is bound `ReadStore + WriteStore`: `item_kinds`, `step_graphs` and `phases` live on **`WriteStore`**
(`traits.rs:472`, `:508`, `:537`), not on `ReadStore`, which ends at `:126`. (2) A phase created
from the app is `seed::phase_row`'s row with four text columns overwritten — `PhaseSeed`'s fields
are `&'static str`, so runtime text cannot flow through it, and ANA-2's eight frozen columns stay
named in `seed.rs` alone. (3) An out-of-range budget refuses with
``constraint violated: `token_budget` = N is outside 1..=2147483647 tokens``:
`prompt::settings::validate` runs **before** the `i32` cast, so `mem.rs`'s "does not fit … which is
INTEGER" branch is unreachable for a validated value. (4) `step_graph_phase.output_kind` is set at
create (`= name`) and **never followed on a rename** — `PhasePatch` has no field for it; whether a
rename should follow it is **MOD-4's** call (M4 open item O-1). (5) A graph every kind points at has
no row of its own, so **`g`** on a kind or phase row opens the owning graph's editor (M4 D19, added
after the blueprint found the hole). (6) In this section `Browse` with a write in flight is
unreachable — the editor consumes every printable key — so the "a read's reply is taken for the
write's" hazard has exactly two open doors, a scope change and a tab re-activation, both argued at
`on_catalogue`. **1154 passed, 0 failed, 30 ignored** workspace-wide with Postgres live;
`tests/kinds.rs` is 51 tests, 8 new `kinds__*.snap`, and **no existing snapshot moved** (the
hierarchy and agents tests build their own two-section tab). The `--demo` smoke was **not** run (no
TTY in the agent environment). Reviewed by `rust-reviewer`: no CRITICAL, one HIGH, two MEDIUM, four
LOW; the HIGH, one MEDIUM and three LOW fixed (`2debc42`..`2016911`) — the HIGH being the two-write
budget chain rewriting its own comparison baseline before the second write landed, so a `Stale`
follow-up silently dropped the budget. Two recorded rather than fixed: an `r` issued mid-chain can
be taken for the write's reply (refusing `r` while busy would wedge a section whose reply was
overtaken, which is why milestone 3 allows it — documented, plus a notice that says nothing was
written), and a re-read that fails **after** an applied write still answers `Failed`, which
`hierarchy::serve` does too and which needs a seam method this milestone does not add.
**Milestone 5 landed (`ec54b6e`..`a0dc3e8`, 2026-09-17): the prompt is tunable from the app.**
Plan at `.claude/plans/mod-15-prompt-settings.plan.md`, blueprint beside it. Two things shipped.
(1) **`htui::prompt_settings`** (`crates/htui/src/prompt_settings.rs`): one `SettingsSnapshot` per
read — the ten `app_setting` keys on the `App` rung each with its own compare-and-set token, plus
every project of the scope with the keys its spec admits — and three served requests
(`StoreRequest` 47 → **50**, names in `htui::prompt_settings::REQUEST_NAMES`, two new replies
taking `StoreReply` to 28), routed through **one `try_serve` arm of three or-ed patterns** as
milestones 3 and 4's twelve and nine are. Every stored value is read through
`WriteStore::setting`, never by indexing `project.settings` here, so M1's live coordinate 2 — the
`project_key` spelling — is applied by the seam and cannot drift. (2) **`PromptSection`**
(`SectionId("prompt")`, `ui/tabs/settings/prompt.rs`), registered **after** `kinds`: rows, labels,
units, ranges, doc lines and accepted rungs all read from `SettingKey::ALL` / `SPECS` — **no key
name is spelled in the section**, and a test greps the source to keep it that way. Each row shows
`stored | effective (source)`: the effective number is the reader's own
(`resolve_budget`, `resolve_hops`, `resolve_max_skill_tokens`, `resolve_excerpt_caps`) and the
label is `BudgetSource`'s own spelling, the one `trim_record.budget_source` already writes. `e`
opens a one-field editor; an **empty field clears** so the rung below answers (PRD D7), and a
rung that holds nothing sends no request and says so. The section parses **shape only** — integer
or finite fraction — and every range, `not_above` and Phase-narrowing refusal arrives as the
seam's own sentence, verbatim. **No seam change, no migration, no `query!`**: `CASES` stays 36,
`EXPECTED_CASES` 36, `.sqlx/` byte-identical, and `0003_orchestration.sql` is still MOD-4's and
still next. **Live coordinates milestone 6 needs.** (1) On the `App` rung a token over a row
**cleared elsewhere** answers `NotFound`, **not** `Stale` — `CasOutcome::Stale` needs a row to
carry — so the reload-and-retry habit does not run on that one path; the refusal names
``app_setting `token_budget` not found``, recovery is `Esc`, `r`, `e`, and closing it is a seam
change (open item **O-3**). (2) Every demo project is born holding `token_budget: 120000` in its
`settings` blob (`fixtures.rs:644-648`) — the blob is **not** `{}` — and `DEFAULTS.token_budget`
is `120_000`, so in the demo world `prompt_upstream_hops` is the only genuinely unset
`Project`-rung row. (3) `MemStore` starts with **no** `app_setting` rows while a migrated Postgres
holds all ten (`0002_agent_probe.sql:68-79`), which is what makes `set_setting`'s
`expected: None` insert-after-clear path real on one store and not the other; both are tested.
(4) `resolve_reserve_bp` is **private** (the reserve comes from `resolve_budget(..).reserve_bp`),
`resolve_hops` takes a third `notes: &mut Vec<String>` argument, and none of the resolvers are
re-exported from `htui_core::prompt` — they are reached through `prompt::settings`.
(5) `SettingKey`'s `Display` is `f.write_str`, so it **ignores format width**: pad `key.key()`,
not the key. (6) `settings/mod.rs` now also owns `CHANGED_ELSEWHERE`, `CHANGED_ELSEWHERE_CLOSED`,
`DELETED_ELSEWHERE` (promoted out of `kinds.rs`) and `wrapped` (which was on its third
byte-identical copy) — milestone 6's section inherits both rather than copying. (7) `SetPhaseBudget`
was **not** folded into `SetSetting`: the two differ in their *reply* — a catalogue tree versus a
settings snapshot — not in their request (**O-2**). (8) **This repo has no CI**: no `.github/`
was ever committed, and `crates/htui/tests/prompt_settings.rs` is `#![cfg(feature = "testkit")]`,
so a plain `cargo test` runs none of it; `cargo test --workspace --all-features` (README `:457`)
is the only thing that does. **1198 passed, 0 failed, 30 ignored** workspace-wide with Postgres
live; `tests/prompt_settings.rs` is 42 tests, 6 new `prompt_settings__*.snap`, and **no existing
snapshot moved at all** (the strip tests build their own section vectors). The `--demo` smoke was
**not** run (no TTY in the agent environment) — the provenance flip it would show is pinned by
`a_project_row_flips_to_project_and_back` instead. Reviewed by `rust-reviewer`: no CRITICAL, no
HIGH, three MEDIUM and seven LOW; the three MEDIUM and four LOW fixed (`6835c73`..`a0dc3e8`).
Three LOW deferred with reasons: the tree still shows a value cleared elsewhere until `r` after
an `App` `NotFound` (honest residue, an auto re-read is a behaviour change this milestone did not
design); the seam's refusal echoes the submitted **number** into `Notice`, which derives `Debug`
(numbers are not secrets, but milestone 6 should give `Notice` the hand-written `Debug` that
`Editor` and `Mode` already have, independently of the DSN newtype); and a write's reply arriving
after a scope change installs the old scope's tree for one event, which `kinds` and `catalogue`
do too and which belongs in the shell's reply filter. One lesson worth carrying: the shell echoes
`set_setting: {message}` on the status line, so a naive "the seam's sentence reached the screen"
assertion passes even when the section does nothing with it — the test filters the echo out and
was mutation-checked against exactly that.
**Milestone 6 landed (`8217749`..`d3ec338`, 2026-09-17): a box with no DSN can fix itself.**
Plan at `.claude/plans/mod-15-connection-section.plan.md`, blueprint beside it. Four things
shipped. (1) **`htui_store::dsn`** (`crates/htui-store/src/dsn.rs`): `Dsn`, a `Zeroizing<String>`
newtype whose `Debug` is `Dsn(<redacted>)`, whose text is reachable only through a `pub(crate)`
`as_str`, and which has no `Display`; `DsnError`'s five fixed sentences (`not a URL`, `no host`,
`unsupported sslmode`, `port out of range`, `unrecognised parameter`) carry **none** of the input —
the type is `Copy`, so it cannot. `Dsn::parse` **pre-scans the string before sqlx sees it**, which
is not decoration: sqlx-postgres 0.9.0 `options/parse.rs:107` does not reject an unrecognised query
parameter, it runs `tracing::warn!(%key, %value, …)`, so an unvalidated DSN's parameter value would
reach the `--log` file. `summary()` rebuilds `postgres://user@host:port/db · sslmode=mode` from
getters `PgConnectOptions` has — it has **no password getter at all**, so the summary is safe by
construction rather than by discipline. (2) **`connect::{apply_dsn, forget_dsn, reconnect_for}` and
`ConnectContext`**, so the DSN's text never leaves `htui-store`: the UI crate holds an opaque `Dsn`
and the worker holds none, which is what `connect.rs:66-70`'s "the store worker never has to hold a
DSN" already bought and this milestone preserves. (3) **`htui::connection`** — one
`ConnectionSnapshot` per read (backend label, DSN state, the mirror's `cache_meta`, the last dial)
and four requests, `StoreRequest` 50 → **54**, `StoreReply` 28 → **29**. The read is served from
`try_serve`; the three writers live in the worker loop's own `match`, because each rewires loop
state `try_serve` cannot see. (4) **`ConnectionSection`** (`SectionId("connection")`,
`ui/tabs/settings/connection.rs`), registered **after** `prompt` and the first consumer of
`TextField::masked()`: `e` opens a masked field (one `•` per char plus a count, no reveal toggle),
`c` clears behind a confirmation, `R` rebuilds the mirror behind a confirmation naming both lists.
`TabAction::FocusSection(TabId, SectionId)` and `SettingsRegistry::focus` arrived with it, under
**ANA-10 M0's frozen names**, plus `Tab::focus_section` — the `Tab` trait's **first** default body.

**The in-process transition is the milestone's real cost**, and it is the seam `docs/ANA-10.md`
§10.13 priced and deferred to a milestone that was never minted. `SetDsn` runs eight ordered steps —
keyring write, open the mirror under the **new** fingerprint, abort the refresher, close the old
cache, swap the backend, install the reconnect, dial — with **everything fallible before the swap**,
so a failure leaves `backend`, `refresher`, `health`, `held` and `reconnect` exactly as they were.
`Settings > Rebuild cache` is built here too (PRD D10).

**Live coordinates other items need.** (1) **The event channel is now `(u64, ConnEvent)`**
(`connect.rs`, `connect::LAUNCH_GENERATION` = 0): a dial in flight across a `SetDsn` must not
deliver, or `go_online` installs the old server's `PgStore` over the **new** mirror and the
refresher writes one database's rows into another's `cache.sqlite` — silently, persistently, and
only `RebuildCache` clears it. The generation is checked **at consumption, never at send**: a
`select!` arm's precondition is evaluated before the future is polled and so cannot see the
generation arriving with the message, and a stale event left at an `mpsc`'s head blocks every later
event for the session. A dropped `Online`/`MigrationsPending` has its `PgStore` pool **closed**, not
dropped — dropping a `PgPool` closes asynchronously. A test that injects on `started.events_tx` is
now a generation-0 injection and is dropped after any `SetDsn`. (2) **An unreadable keyring is not
an empty one.** `secret::get_dsn`'s mapping is unchanged (`NoEntry` and blank are `Ok(None)`,
everything else is `Err`), but `connect::start` no longer propagates it: before this milestone, a
Linux box with a session bus and no unlocked collection answered `NoStorageAccess` and **the binary
did not launch at all** — pre-existing, and the exact opposite of what `connect.rs:218-220` promises
and of what this milestone is for. `ConnectionSnapshot` carries `DsnState { NotApplicable, Stored,
NotStored, Unreadable(String) }`, the section shows the reason, and the first-launch redirect fires
on **`NotStored` only** — telling a user whose keyring is merely locked that they have no DSN would
invite them to retype a credential into a store that cannot hold it. `--set-dsn` / `--clear-dsn`
still exit non-zero on a keyring error, deliberately: an explicit command to write the keyring must
report that it could not. (3) `PgConnectOptions::from_str` reads env and, when the DSN carries no
password, `~/.pgpass` — so validation does one small synchronous file read on the UI task, on
`Enter` only (priced, not fixed). (4) **`keyring::mock` cannot round-trip through this seam**: it is
entry-local (`persistence() == EntryOnly`) and `secret.rs` opens a fresh `Slot` per call, so
`set_dsn` then `get_dsn` answers `None` under it. `secret.rs` therefore carries a
`#[cfg(feature = "test-support")]` process-wide fake (`Slot` / `Broken` states), installed only by
`htui_store::testkit::{mock_keyring, mock_keyring_broken}` under one process-wide lock;
`cargo check -p htui-store` with no features is the proof it is compiled out. **Any `Harness` over a
non-`Memory` backend reaches the keyring** through `App::start`'s `ConnectionInfo`, so such a test
takes a guard first; `Backend::Memory` never consults it. (5) `CacheStore::rebuild` had **two**
production callers before this milestone (`hierarchy.rs`, `catalogue.rs`) and now has a third, the
`RebuildCache` request — `HANDOFF.md`'s old claim that `Settings > Rebuild cache` already called it
was false, and the PRD's Evidence claim that its only callers were tests was false in the other
direction. (6) `PgStore` has no `close()`; a discarded one's pool closes on drop (open item).

**No seam change, no migration, no `query!`**: `CASES` stays 36, `EXPECTED_CASES` 36, `.sqlx/`
byte-identical, and `0003_orchestration.sql` is still MOD-4's and still next. `zeroize` moved from a
transitive dependency to a declared one (three manifest lines, no new crate in the lock), and
`TextField` now holds a `Zeroizing<String>` whose masked constructor reserves 256 bytes so an
ordinary DSN never reallocates — a reduction in residue, not an elimination. **1271 passed, 0
failed, 30 ignored** workspace-wide with Postgres live **and at `--test-threads=1`**, which is the
strict condition here: the suite's greenness was briefly scheduling-dependent, because an unguarded
test passed only when a sibling happened to have the fake keyring installed. `tests/connection.rs`
is 51 tests, five `connection__*.snap`. The `--demo` smoke was **not** run (no TTY in the agent
environment); the milestone's headline is pinned instead by a Postgres-gated worker test,
`set_dsn_goes_online_without_a_restart`, which starts from an empty keyring, sends one `SetDsn` and
drives the worker to `label == "online"` in the same process.

Reviewed by `rust-reviewer`: **one HIGH, one MEDIUM, four LOW**, and the HIGH blocked the milestone.
Every finding was then verified by two independent adversarial passes before it was applied, which
changed three verdicts. The HIGH was the stale-dial guard: the generation counter added during
implementation covered neither the **launch** dial (`connect::start` spawns its own and never
consulted the counter — a window of the full 10-second connect timeout, and a blackholed SYN is both
what burns it and what sends a user to Settings to change the DSN) nor an event **queued before the
bump**; the fix is coordinate (1) above, proved by sabotage — with the consumption check removed the
new tests fail showing `online` over the new DSN's mirror, while the pre-existing dial test still
passes, which is the direct evidence it could never have caught the launch path. The MEDIUM was
tests reaching the developer's real keyring; verification found **half of its named list wrong** (
five of ten call `serve` directly or use `Backend::Memory`) but also found one case **red on this
box**, which the reported suite had masked. Two LOW were fixed (comments claiming sqlx logs an
unclosed `options[`, which it silently drops instead). Two LOW were **deferred with reasons**: the
`apply_dsn(dsn.clone())` copy, because `apply_dsn` clones again internally and both copies are
`Zeroizing`, so moving at the call site removes zero net exposure; and a failed **read** clearing
`busy`, because the same arm sets `unavailable` and the section's `blocked()` gates on both. The
reviewer's `select!`-fairness argument was judged decorative — nothing dropped the event either way.
