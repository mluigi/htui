# Plan: MOD-59 - A write's reply names itself, so a form never stays "in flight"

**Status: CONFIRMED by the maintainer 2026-09-30; implemented and closed out 2026-09-30 (`docs/decisions/mod/mod-59.md`). Blueprint deviations DV-1..DV-3 applied, DV-4 not; review findings in the write-up.**

**Source**: `HANDOFF.md:268-279` (MOD-59, from MOD-9 milestone 3 review, finding 3; found
2026-09-26; MOD-39 addendum). Requirements `R-TUI-7`, `R-NF-3`.

**Routing**: routed as plan by `/handoff-run MOD-59` (1 of C1-C4 fired: C3, the item names two
fixes without choosing one). Ultracode not needed. Staffing: session model (Opus 5.5) for every
agent, no `model:` override.

**Base**: `hr/MOD-59` @ `08e0880`. No migration, no `.sqlx` entry, no new dependency, no snapshot
(`insta`) change expected. Replies are notices and form state, not rendered frames.

**Numbering**: MOD-59's own plan. Decisions **D1…D7**, tasks **T1…T3**.

---

## The defect

Three views decide that their own write landed by searching the re-read snapshot for what they
sent:

| View | Predicate | Where |
|---|---|---|
| Skills library (create, new version, rename, attach, detach) | `LibraryView::land` | `crates/htui/src/ui/tabs/skills/library.rs:1199-1286` |
| Templates (save) | `TemplatesView::land_save` (milestone 1's D27) | `crates/htui/src/ui/tabs/skills/templates.rs:842-889` |
| Requirements (area, mint, amend, withdraw) | `RequirementsTab::land` | `crates/htui/src/ui/tabs/requirements/mod.rs:743-799` |

The worker answers an applied write with the same variant it uses for a plain read
(`StoreReply::Skills` / `Templates` / `Requirements`). So the view cannot tell its write's own reply
from a read served ahead of it, and it has to match on content. The content match fails in these
cases, and when it does, `busy` stays set and the form refuses `Esc` and `Ctrl+S` until the
workspace changes:

- **Another session writes the same row between the write and the worker's re-read.** The version
  body, the template body, the binding's fields or the requirement's text are someone else's, so
  the predicate never holds.
- **`MemStore` stamps two writes with the same instant.** A rename's `updated_at != token` check
  (`library.rs:1212-1225`) and an attach's `Some(row.updated_at) != *token` check
  (`library.rs:1227-1241`) never hold.
- **The Requirements tab's mint** needs a `known` id list and a body match to find the new row. When
  the write applied but only the re-read failed, the worker answers `Failed` under the write's
  name (every `serve` ends with `answer(…).await` behind `?`). So the tab re-reads to check
  (`verifying`, `CHECKING_MINT`, `mod.rs:1131-1223`) before it offers a retry that would mint a
  second requirement.

**Prior art in the tree.** MOD-23 already built the shape this item recommends, for the agent
registry. `StoreReply::AgentWritten { agents, outcome: AgentWrite }` (`store_worker.rs:1122-1130`,
`agent_settings.rs:485-520`) is documented as "Self-naming (MOD-59)". The Settings Agents section
lands a write on that variant alone, and a plain `Agents` reply never closes its form
(`ui/tabs/settings/agents.rs:2184-2206`). The import already works this way too:
`StoreReply::SkillImports` is "the import's own reply" (`store_worker.rs:1091-1095`). MOD-59 applies
the same pattern to the three remaining writers.

---

## Design decisions

| # | Decision | Why |
|---|---|---|
| D1 | **Self-naming variants, not "release on any reply to the request name".** There are three new `StoreReply` variants, each mirroring `AgentWritten`: `SkillWritten { snapshot, outcome: SkillWrite }`, `TemplateSaved { snapshot, project, name, version }` and `RequirementWritten { snapshot, outcome: RequirementWrite }`. Plain `Skills` / `Templates` / `Requirements` become read answers only. | The HANDOFF item recommends this, and MOD-23 already chose it. "Any reply to the request name" is not available anyway: an applied write's reply has no request name on it today. Only `Failed` carries `request`, and the envelope's `seq` is consumed by `App::is_fresh`, not passed to the tab. |
| D2 | **The outcome carries what the store returned** (every writer already returns the applied row, `htui-core/src/store/traits.rs:837-920, 1427-1480`). `SkillWrite::{Created { skill, name }, Versioned { skill, version }, Edited { skill, name }, Attached { key }, Detached { key }}`. `TemplateSaved` carries `project`, `name` and the stored `version`. `RequirementWrite::{Area { id, code }, Minted { id, key }, Amended { id, key, version }, Withdrawn { id, key }}`. Ids, names, keys and versions only; bodies stay in the snapshot. The enums live beside their `serve`, as `AgentWrite` does in `agent_settings.rs`. | The view's notice ("minted R-TUI-12", "saved v4") and cursor move come from the store's own answer, not from a search. The mint's `known` list and the amend's content comparison go away. |
| D3 | **The `*Stale` variants stay as they are.** `SkillsStale { snapshot, what }`, `TemplatesStale`, `RequirementsStale`. | Only a write produces them, so they already name themselves. Folding them into the outcome enum the way `AgentWrite::Stale` does would only churn three views' stale paths, which work. |
| D4 | **The landing gate is the write in flight, not content.** A `*Written` reply lands only when the view's `busy` is the request name of that outcome's write (`busy == Some("save_skill_version")` for `Versioned`, and so on). It is otherwise ignored for landing, and its snapshot is still taken if in scope. A plain read reply never lands and never clears `busy`. | The staleness gate is keyed `(Origin, Discriminant<StoreRequest>)` (`app/state.rs:164`, `:329-333`), so a newer write of the same kind drops an older reply before the view sees it, and a read never supersedes a write. All three views reset `busy`/`sent` on scope change (`library.rs:527-533`, `skills/templates.rs:331-337`, `requirements/mod.rs:1086-1088`), so a reply from a left workspace finds nothing in flight. The "later edits kept" rule still compares the editor's text with `Sent`'s body, which stays for that purpose alone. |
| D5 | **An applied write whose re-read failed still answers its own variant.** `snapshot` is `Result<Box<Snapshot>, String>`: `Err` carries the re-read's `StoreError` rendered through `Display`. The view lands the write (the form closes, the notice says what landed and that the re-read failed), keeps drawing the snapshot it holds, and sets `unavailable` only when it holds none. A write that was **refused** stays `Failed`. | This retires the Requirements tab's `verifying` / `CHECKING_MINT` round trip (the HANDOFF item says so). It also fixes the same lie in the Skills and Templates views, where an applied-but-unread write reads as "refused", the draft keeps its old token, and the retry goes stale. |
| D6 | **The Backlog detail's Reqs pane (`ItemCitations`) is out of scope.** | It already clears `busy` on any `ItemCitations` for its item (`ui/tabs/backlog/detail/requirements.rs:498-532`) and never matches content, so it cannot wedge. |
| D7 | **One task per writer, serial on one tree** (T1 Skills → T2 Templates → T3 Requirements). Each task adds its variant, its worker side and its view together, so every task commit is green. | All three tasks add a variant to `store_worker.rs`, so their file sets intersect and the parallel marking is stripped. Splitting worker from view instead (fact-check, V12) would leave the existing view tests red between commits. All three compile into the one `htui` crate anyway, and a worktree per task costs about 10 GB of `target/`. |

---

## Patterns to mirror

| Category | Source | Pattern |
|---|---|---|
| Self-naming reply | `store_worker.rs:1122-1130`, `agent_settings.rs:485-520` | `AgentWritten { agents, outcome }` + `AgentWrite` enum beside its `serve`; doc says a plain read never closes the form |
| View landing | `ui/tabs/settings/agents.rs:2184-2206` | the write's variant takes the rows, clears `busy`, calls `on_written(outcome)` |
| Worker re-read | `skills.rs:377-392` (`answer`), `requirements.rs:737-744`, `templates.rs:133-180` | one `answer` helper per module builds the reply after the write |
| Refusal | `library.rs:591-598`, `requirements/mod.rs:1204-1219` | `Failed { request }` with `REQUEST_NAMES` / `is_tab_write` frees the form, notice = message |
| Tests (worker) | `templates.rs:349-393` | `#[tokio::test]` against `Backend::memory(MemStore::demo())`, `match` on the reply variant, `panic!` on the wrong one |
| Tests (view) | `library.rs:1885-1963` | drive `on_key`/`on_reply` with a `Ctx` over `Emit`, `serve()` helper for real replies, assert `busy`, `captures_input`, `notice` |

---

## Files to change

| File | Task | Action | Why |
|---|---|---|---|
| `crates/htui/src/store_worker.rs` | T1, T2, T3 | UPDATE | one variant per task (D1); re-document `Skills`/`Templates`/`Requirements` as read answers |
| `crates/htui/src/skills.rs` | T1 | UPDATE | `SkillWrite`; `answer` builds `SkillWritten` (D2, D5); worker tests |
| `crates/htui/src/ui/tabs/skills/library.rs` | T1 | UPDATE | `on_reply` lands on `SkillWritten` (D4); `land` loses its predicate; tests |
| `crates/htui/src/ui/tabs/skills/attach.rs` | T1 | UPDATE (maybe) | `on_landed` keeps its signature; touched only if the key now comes from the outcome |
| `crates/htui/src/templates.rs` | T2 | UPDATE | `serve`'s applied arm builds `TemplateSaved` (D5); worker tests |
| `crates/htui/src/ui/tabs/skills/templates.rs` | T2 | UPDATE | `on_reply` / `land_save` land on `TemplateSaved`; tests |
| `crates/htui/src/requirements.rs` | T3 | UPDATE | `RequirementWrite`; the four tab writes build `RequirementWritten`; worker tests |
| `crates/htui/src/ui/tabs/requirements/mod.rs` | T3 | UPDATE | `land` from the outcome; `verifying`, `CHECKING_MINT`, `Sent::Mint.known` removed (D5); tests |
| `crates/htui/tests/requirements_pg.rs` | T3 | UPDATE | the `applied()` helper (`:73-78`) matches `StoreReply::Requirements` for every tab write |

Nine files. The integration suites `tests/{skills,skills_pg,templates,templates_pg,requirements}.rs`
drive the real worker through `App`, so they pick up the new variants unedited. They are in the
validation run, not the edit list, and any of them that does fail is fixed in the task that owns
its writer. Sets per task: T1 = {store_worker, skills, library, attach}; T2 = {store_worker,
templates, skills/templates}; T3 = {store_worker, requirements, requirements/mod, requirements_pg}.
`store_worker.rs` is in all three, so the tasks run serial (D7).

---

## Tasks

Each task is tests first (red commit), then the change (green commit), per the repo's TDD
convention. The tree is green at the end of every task.

### T1: the Skills library's writes name themselves

- **Tests first.** Worker (`skills.rs`): each of `CreateSkill`, `EditSkill`, `SaveSkillVersion` and
  `SetSkillBinding` (attach and detach) answers `SkillWritten` with the stored id, version or key.
  A refused write is still `Err`. A stale write is still `SkillsStale`. `StoreRequest::Skills` still
  answers `Skills`. View (`library.rs`):
  - (a) **The defect.** A version is saved, and another session saves over it before the re-read.
    The `SkillWritten` reply still closes the editor with "saved vN".
  - (b) A rename on a `MemStore` driven by a `TestClock` that is not advanced, where `updated_at`
    equals the token, still lands.
  - (c) `a_read_reply_does_not_close_the_attach_form_mid_save` keeps passing, now because a plain
    `Skills` never lands.
  - (d) `SkillWritten { snapshot: Err(..) }` lands, the notice names the re-read failure, and the
    held library stays drawn.
  - (e) A `SkillWritten` whose write is not the one in `busy` does not land.
- **Action**: add the `SkillWritten` variant and `SkillWrite`; `answer` takes the outcome and maps a
  re-read `Err` to `snapshot: Err(message)`. `on_reply` gains the arm (D4). `land` drops its content
  predicate and dispatches on the outcome. `landed_version` takes the skill id from the outcome.
- **Mirror**: `AgentWritten` / `AgentWrite`; `AgentsSection::on_reply`.
- **Validate**: `cargo test -p htui --all-features skills`

### T2: the Templates view lands on its save

- **Tests first.** Worker (`templates.rs`): an applied `SaveTemplate` answers `TemplateSaved` with
  the stored version. Stale is still `TemplatesStale`. View: the save lands on `TemplateSaved` even
  when the stored head's body is another session's. A plain `Templates` reply keeps `busy`. A
  re-read `Err` lands with the notice.
- **Action**: add the variant; `serve`'s `Applied` arm builds it. `on_reply` gains the arm.
  `land_save` takes `version` from the reply and drops the body comparison, keeping the "later edits
  kept" rule (`tests/templates.rs:485` pins its notice).
- **Validate**: `cargo test -p htui --all-features templates`

### T3: the Requirements tab lands on its write

- **Tests first.** Worker (`requirements.rs`): the four tab writes answer `RequirementWritten` with
  the stored id, key and version. A refused write is still `Err`. A diverged amend or withdraw is
  still `RequirementsStale`. View:
  - An amend lands on `RequirementWritten` although another session amended again before the
    re-read.
  - A mint whose re-read failed lands as "minted R-…" and sends no second `Requirements` request.
    The `CHECKING_MINT` path is gone.
  - A plain `Requirements` reply never closes the form.
- **Action**: add the variant and `RequirementWrite`; `answer` takes the outcome. `on_reply` gains
  the arm. `land` builds its message and row from the outcome. `verifying`, `CHECKING_MINT` and
  `Sent::Mint.known` are removed, and the `Failed` arm for a tab write frees the form directly.
  `tests/requirements_pg.rs`'s `applied()` follows the variant.
- **Validate**: `cargo test -p htui --all-features requirements`, which includes
  `requirements_pg` against `HTUI_TEST_DATABASE_URL`.

---

## Validation

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-features --all-targets -- -D warnings   # README.md:446
cargo test --workspace --all-features -- --test-threads=1              # memory: suite green is scheduling-dependent
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```

`--all-features` is load-bearing: without `testkit`, `tests/*.rs` run 0 tests and report ok. The
sandbox sets `HTUI_TEST_DATABASE_URL`, so the `_pg` suites run for real.

---

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| Some other consumer matched `Skills`/`Templates`/`Requirements` after a write | Low | The consumer census is below in "verified claims". The compiler finds any remaining `match` that should take the new variant only where a test drives it, so T1's worker tests assert the variant explicitly. |
| `Result` in a `Clone`/`Debug` reply leaks secrets through `Display` | Low | Same channel as `Failed { message }` today, which renders the same `StoreError`. No new data leaves the worker. |
| The "later edits kept" rule regresses when the body comparison goes | Medium | `Sent` keeps the body for exactly that rule; the existing "later edits kept" tests in `library.rs` and `skills/templates.rs` stay and must pass unchanged. |
| A reply arrives for a write the view already forgot (scope change) | Low | D4: every view resets `busy` on scope change, and the `(origin, kind)` gate drops superseded writes. |

---

## Acceptance

- [ ] No view decides that its write landed by matching content in a snapshot.
- [ ] A plain read reply never clears `busy` in the Skills, Templates or Requirements views.
- [ ] An applied write whose re-read failed closes its form and says so; the Requirements tab no
      longer re-reads to check a mint.
- [ ] The regression tests for the two HANDOFF scenarios (another session's write in between, same
      `MemStore` instant) fail on `08e0880` and pass after.
- [ ] Validation passes; the HANDOFF line is closed per `workflow-docs.md`.

---

## Verified claims

Plan fact-check (handoff-run step 3.5), 2026-09-30, against `hr/MOD-59` @ `08e0880`.

| # | Claim | Verdict | Evidence |
|---|---|---|---|
| V1 | Skills, Templates and Requirements land by matching sent content in the re-read snapshot | ✓ | `library.rs:1199-1286`, `skills/templates.rs:842-889`, `requirements/mod.rs:743-799` (read in full) |
| V2 | An applied write answers the same variant as a plain read | ✓ | `skills.rs:377-392` (`answer` → `Skills`), `templates.rs:133-180` (`Applied` → `Templates`), `requirements.rs:737-744` |
| V3 | A re-read failure after an applied write answers `Failed` under the write's name | ✓ | each `serve` ends `answer(…).await` behind `?`; `requirements/mod.rs:1204-1214` re-reads with `CHECKING_MINT` because of it |
| V4 | `AgentWritten` / `AgentWrite` is prior art citing MOD-59 | ✓ | `store_worker.rs:1122-1130` doc "Self-naming (MOD-59)"; `mod-23.md:45-46` "MOD-59's recommended shape" |
| V5 | Only `Failed` carries a request name; the tab never sees `seq` | ✓ | `StoreReply` (`store_worker.rs:928-1149`); `App::on_reply` calls `tab.on_reply(&reply, &mut ctx)` (`app/update.rs:241-313`) |
| V6 | The staleness gate is keyed per origin and request kind | ✓ (amended: the key is `Discriminant<StoreRequest>`) | `app/state.rs:164`, `:329-333` |
| V7 | All three views reset the write in flight on scope change | ✓ | `library.rs:527-533`, `skills/templates.rs:331-337`, `requirements/mod.rs:1086-1088` |
| V8 | Every writer returns the applied row (id, version, key available) | ✓ | `htui-core/src/store/traits.rs:837-841, 876, 884-920, 1427-1480`; `PromptTemplate.version` (`model/kind.rs:329`); `RequirementUpdate::Updated(Requirement)` (`model/requirement.rs:281-283`) |
| V9 | `MemStore` can stamp two writes with the same instant | ✓ | `TestClock` holds one instant until `advance` (`htui-core/src/clock.rs:72-110`); `MemStore::now` uses the injected clock (`store/mem.rs:830-831`) |
| V10 | The Backlog Reqs pane cannot wedge (out of scope) | ✓ | `backlog/detail/requirements.rs:498-532` clears `busy` on any `ItemCitations` for its item |
| V11 | `tests/requirements_pg.rs:75` and `:240` match a write's reply | ✗ **amended** | `:73-78` is the `applied()` write helper; `:240` answers a plain `Requirements` read and stays |
| V12 | Worker-first T1 then views T2-T4 leaves every commit green | ✗ **amended** | existing view tests (e.g. `library.rs:1885-1963`) serve a write and expect it to land; tasks re-cut per writer (D7) |
| V13 | No `insta` snapshot holds `CHECKING_MINT`'s text | ✓ | the sentence occurs only in `requirements/mod.rs:82` |
| V14 | No doc outside `docs/decisions/` names the changed variants | ✓ | grep of `docs/*.md`, `docs/**/*.md` for `SkillsStale\|TemplatesStale\|RequirementsStale\|CHECKING_MINT` |
| V15 | "Later edits kept" is pinned by tests | ✓ | `library.rs:1965-2062` (attach form), `tests/templates.rs:485` (template save) |
| V16 | Task independence | ✓ serial | T1-T3 file sets all hold `store_worker.rs` (Files to change); no parallel marking survives |
| V17 | Gate commands | ✓ | `README.md:440-446`; `--all-features` per memory (testkit) |
