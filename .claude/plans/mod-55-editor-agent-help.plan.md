# Plan: Agent help in the Skills editors (MOD-55)

**Source PRD**: `.claude/prds/mod-55-editor-agent-help.prd.md`
**Selected Milestone**: 1 — Agent help in the Skills editors
**Complexity**: Medium (about 13 files across `htui-core`, `htui-store`, `htui`)
**Status**: DONE (2026-10-06) — confirmed 2026-10-06; write-up `docs/decisions/mod/mod-55.md`

## Summary

A new editor action (`Ctrl+G`) in the Templates and Library editors opens a small request prompt.
The request, the body being edited and (for templates) the role's placeholder table are sent as one
**help turn**: a new store request served by the agent runtime. It reuses the chat path's run pair,
driver and recorder, but scrubs the prompt **before** it leaves, exposes no tools, denies every
permission, and closes the session once the first turn is done. The reply's fenced block becomes a
proposed body, shown as a diff against what was sent. Accepting it replaces the editor buffer, and
saving stays the existing `Ctrl+S` gate.

## Design decisions (proposed, maintainer may amend at CONFIRM)

| # | Decision | Why |
|---|---|---|
| P1 | **A new request, `StoreRequest::EditHelp { project_id, agent_id, prompt: HelpPrompt }`**, served by `AgentRuntime` through the same `start` → `run_chat` code with a `ChatMode::{Conversation, Help}` switch, not a second session runner | Reuses identity, registry, driver, run pair, recorder, cap and frame plumbing (PRD D1, D2); one place stays responsible for a session's life |
| P2 | **Scrub before send, before the run is minted.** Help mode assembles the prompt in `htui-core`, scrubs it with the session's scrubber (`MinimalScrubber` over `spec.env`, which MOD-10 will fill), and refuses with `StoreReply::Failed` naming the section and rule (never the text) **before** `start_chat_run`, so a refusal leaves no run | PRD D3, `R-ID-7` fail closed; mirrors the MCP lease, which is opened before the run is written so a refusal leaves no run behind |
| P3 | **Help sessions expose nothing and permit nothing:** no MCP lease; `PermissionPolicy { default: Deny, rules: [], remembered: [] }`; `ToolExposure.deny_kinds` covering the write, delete, move and execute kinds | The agent row's own policy defaults to `Ask`, which would park a request nobody answers. The ACP `fs/write_text_file` handler would write under the process cwd. A help turn proposes text; it never acts (PRD constraint, `R-ID-5`) |
| P4 | **One turn:** in help mode, `run_chat` ends the session after the first `Done`, exactly as a `ChatCancel` between turns does (run closed `Done`, `ChatFrame::Ended` sent) | `run_chat` waits for follow-ups indefinitely by design; there is no end-after-first-turn today |
| P5 | **Help runs are marked** by `run_step.phase_name = 'edit_help'` (still `run.kind = 'chat'`): a `phase_name` field on `ChatRunSpec`, defaulting to `"chat"`, set by a `ChatRunSpec::for_edit_help()` builder; both stores bind it | No migration (`phase_name` has no CHECK); keeps every existing `mint` caller unchanged; makes help runs countable for the PRD metric |
| P6 | **The fixed instruction and the reply parser live in `htui-core`** (`prompt::edit_help`): `HelpPrompt { target: HelpTarget::{Template(TemplateRole), Skill}, body, request }`, `assemble(&HelpPrompt) -> String` (instruction text, placeholder table for templates only, body in a fenced block, the request), and `proposal(reply: &str) -> Option<String>` (the last fenced block, else `None`) | PRD open questions 2 and 3; pure functions, unit-testable without a session; `R-ID-5` (the instruction is htui's text) |
| P7 | **The project is the editor's for templates** (`Editor.project`; templates are project-scoped) and **the active project for skills** (`ctx.projects.first()`, as the Chat tab does); no project means the action is refused with a notice | PRD open question 1, refined: a template already belongs to a project, so no guess is needed there |
| P8 | **One shared UI component, `ui/tabs/skills/help.rs`**, embedded by both editors as `help: Option<Help>`. States: `Asking` (a `TextField` for the request and an agent picker over enabled agents, using the Chat tab's `StoreRequest::Agents` read), `Waiting { step_id, sent_body }` (`Esc` sends `ChatCancel`), `Proposal { proposed, diff }` (`Enter`/`y` accept, `Esc`/`n` discard), `Answered { text }` (reply without a fenced block, nothing to accept). While `help` is `Some`, the buffer is locked: editor keys go to the component | PRD open question 4: the diff is always against the body that was sent |
| P9 | **A proposal containing the mask marker `[REDACTED]` is flagged** in the diff pane's title and needs a second accept | PRD risk 1: a masked secret coming back as the marker would otherwise overwrite a real value |
| P10 | **The key is `Ctrl+G`, hard-coded like `Ctrl+E`/`Ctrl+S`**, added to both editors' hint lines | **PRD constraint amended:** MOD-67's catalogue does not exist yet (open, not started; the Skills views do not read `keymap.rs`). A one-line note is appended to MOD-67's HANDOFF entry so its milestone 4 (Skills) picks the action up |
| P11 | **Help is unavailable offline** with chat's own sentence (`DATABASE_UNREACHABLE`), from the same `backend.writer()` check | PRD open question 5 |

## Patterns to Mirror

| Category | Source | Pattern |
|---|---|---|
| Request shape | `crates/htui/src/store_worker.rs:196` (`StoreRequest::ChatStart`) | Doc-commented struct variant; reply is `ChatAccepted` then `Chat` frames |
| Session start | `crates/htui/src/agent_worker.rs:2036` (`AgentRuntime::start`) | Refusals as `StoreError` before `start_chat_run`; lease opened before the run is written |
| Session loop | `crates/htui/src/agent_worker.rs:4130` (`run_chat`), cancel between turns | Closing path records `Done` and sends `Ended` |
| Scrubbing | `crates/htui-core/src/prompt/mod.rs:1276` (`scrub_text`), `crates/htui-core/src/scrub.rs:138` | Refusal is typed (`Unmasked { path, rule }`), never carries the text |
| Editor keys | `crates/htui/src/ui/tabs/skills/templates.rs:687` (`on_editor_key`), `library.rs:1020` | Chord check before handing the key to `TextArea`; refused while `busy` with the in-flight notice |
| Diff | `crates/htui/src/ui/diff.rs` (`unified`, `lines`), call site `templates.rs:1039` | `unified(old, new, old_label, new_label)` then `diff::lines` |
| Chat-run rows | `crates/htui-store/src/pg/write.rs:1915`, `crates/htui-core/src/store/mem.rs:2136` | Both stores mirror each other; conformance case in `htui-core/src/store/conformance.rs:1862` |
| Errors | `StoreReply::Failed { request, message }` | Request name is the variant's snake case (`edit_help`) |
| Tests | `htui_agent::fake::{FakeAdapter, FakeDriver}` + `Script::one_turn` (`htui-agent/src/conformance.rs:96`); `agent_worker.rs` `fixture` (~5146); `crates/htui/tests/chat.rs:77` harness; `templates.rs` `Bench`; snapshots `crates/htui/tests/snapshots/{skills,templates}__*.snap` | TDD: failing test first per task |

## Files to Change

| File | Action | Why | Task |
|---|---|---|---|
| `crates/htui-core/src/prompt/edit_help.rs` | CREATE | `HelpPrompt`, `HelpTarget`, `assemble`, `proposal`, instruction text (P6) | T1 |
| `crates/htui-core/src/prompt/mod.rs` | UPDATE | `pub mod edit_help;`; expose a `pub` string-scrub helper (wrap of `scrub_text`) returning a typed refusal (P2) | T1 |
| `crates/htui-core/src/model/run.rs` | UPDATE | `ChatRunSpec.phase_name` (`#[serde(default)]` → `"chat"`), `for_edit_help()` (P5) | T2 |
| `crates/htui-core/src/store/mem.rs` | UPDATE | Bind `phase_name` from the spec; test | T2 |
| `crates/htui-core/src/store/conformance.rs` | UPDATE | Case: an edit-help chat run stores `phase_name = 'edit_help'` | T2 |
| `crates/htui-store/src/pg/write.rs` | UPDATE | Bind `phase_name` as a parameter | T2 |
| `crates/htui-store/.sqlx/query-*.json` | CREATE/DELETE | Regenerated offline entry for the changed insert | T2 |
| `crates/htui/src/store_worker.rs` | UPDATE | `StoreRequest::EditHelp`; routing to the agent runtime | T3 |
| `crates/htui/src/agent_worker.rs` | UPDATE | `ChatMode`; help-mode scrub-before-send, policy, exposure, no lease, one turn (P1–P4, P11); tests | T3 |
| `crates/htui/src/ui/tabs/skills/help.rs` | CREATE | The shared component (P8, P9) | T4 |
| `crates/htui/src/ui/tabs/skills/mod.rs` | UPDATE | `mod help;` | T4 |
| `crates/htui/src/ui/tabs/skills/templates.rs` | UPDATE | `Ctrl+G`, embed `Help`, render, hint, project (P7, P10) | T4 |
| `crates/htui/src/ui/tabs/skills/library.rs` | UPDATE | Same for the Library editor | T5 |
| `crates/htui/tests/templates.rs`, `crates/htui/tests/skills.rs` + `snapshots/` | UPDATE | End-to-end help turn over `FakeDriver`; hint-line snapshots | T4, T5 |
| `HANDOFF.md` | UPDATE | MOD-67 note (P10); MOD-55 phase/close-out | T6 |

## Tasks

Wave 1 runs **T1 ∥ T2** (disjoint files). Then **T3**, **T4**, **T5** and **T6** run in sequence: T3
needs T1+T2, T4 needs T3's request variant, and T5 reuses T4's component.

### Task 1: Help prompt and reply parser (`htui-core`)
- **Action**: Tests first: `assemble` includes the placeholder table only for templates, puts the body
  in a fence long enough not to collide with fences inside it, and includes the request.
  `proposal` returns the last fenced block, `None` without one, and handles the body's own nested
  fences. The public scrub helper masks a known value and refuses a pattern with section and rule.
  Then implement.
- **Mirror**: `prompt/mod.rs` `scrub_text`; `template.rs` `Placeholder::ALL`/`allowed_in`/`required_by`.
- **Validate**: `cargo test -p htui-core prompt::edit_help`

### Task 2: Mark help runs (`ChatRunSpec.phase_name`)
- **Action**: Test first (conformance case, mem unit test). Then add the field and builder, bind it in
  both stores, and regenerate the sqlx offline entry against a migrated scratch DB
  (`docs/hr-sandbox.md`, `localhost:5439`).
- **Mirror**: `start_chat_run` in both stores; `start_chat_run_mints_chat_rows`.
- **Validate**: `cargo test -p htui-core store`; `cargo test -p htui-store --all-features` (Postgres
  conformance); `SQLX_OFFLINE=true cargo check -p htui-store`

### Task 3: `EditHelp` in the agent runtime
- **Action**: Tests first in `agent_worker.rs` over `FakeDriver`:
  - (a) a help turn records `prompt`, the reply events, and closes the run `Done` after one turn
    without a `ChatCancel`;
  - (b) a body holding a key pattern is refused with no run minted and no driver start;
  - (c) the session spec carries no MCP server, `default: Deny`, and the denied kinds;
  - (d) the step is `phase_name = 'edit_help'`;
  - (e) offline is refused with the chat sentence.

  Then implement `ChatMode` through `start` and `run_chat`.
- **Mirror**: `AgentRuntime::start`, `run_chat`, the cancel-between-turns branch.
- **Validate**: `cargo test -p htui --all-features agent_worker -- --test-threads=1`

### Task 4: Help component + Templates editor
- **Action**: Tests first:
  - unit tests on `Help` state transitions: chunks accumulate, `Done`/`Ended` leads to `Proposal` or
    `Answered`, accept replaces the buffer, `[REDACTED]` needs a second accept, and `Esc` while
    waiting sends `ChatCancel`;
  - a `templates.rs` `Bench` test: `Ctrl+G` → type request → `EditHelp` dispatched with the editor's
    project and role;
  - an integration test in `tests/templates.rs` over `FakeDriver`: the proposal diff renders,
    accepting it and pressing `Ctrl+S` runs `parse`;
  - snapshot updates.

  Then implement.
- **Mirror**: `templates.rs` editor modes and notices; Chat tab frame handling (`chat/mod.rs:550-603`).
- **Validate**: `cargo test -p htui --all-features templates -- --test-threads=1`; `cargo insta test -p htui --all-features` (full run, review every changed snapshot)

### Task 5: Library editor
- **Action**: Same wiring in `library.rs`. Tests first: dispatch with the active project and
  `HelpTarget::Skill`; refused without a project; accept, then save through `skill_body_refusal`.
- **Mirror**: Task 4.
- **Validate**: `cargo test -p htui --all-features library -- --test-threads=1`; full insta run

### Task 6: Docs
- **Action**: Append the MOD-67 note (P10). Mention the `edit_help` phase name in the schema comment
  (`0001_init.sql:478` is a comment in a shipped migration and is **not** edited, because migrations
  are immutable; the note goes in the write-up instead).
- **Validate**: `bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh`

## Validation

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo clippy --workspace -- -D warnings            # featureless gate
cargo test --workspace --all-features -- --test-threads=1
cargo insta test -p htui --all-features            # review, never blanket-accept
SQLX_OFFLINE=true cargo check --workspace --all-features
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| `run_chat` is long and concurrency-sensitive; a help branch could disturb chat or promotion | Medium | Mode switch at the few points that differ (scrub, policy, lease, end of turn); all existing chat tests run unchanged |
| A CLI transport ignores `deny_kinds` for some tool names, so the agent still writes | Low | `deny_kinds` is inverted to tool names on `claude-cli`'s argv (MOD-26 D11); T3 test (c) asserts the spec, and the blueprint checks the CLI argv mapping |
| The chat path itself still sends unscrubbed (pre-existing `R-ID-7` gap) | Exists today | Out of scope; offered to the maintainer as a new item at close-out |
| `Help` frames reach both Skills views (`SkillsTab::on_reply` forwards to both) | Medium | Each `Help` matches frames on its own `step_id` / request seq |
| A global `Action::Error` fires on every `Failed`, double-reporting a scrub refusal | Low | Accept the status-line line as the one report; the component shows the same sentence |
| Snapshot churn from hint-line changes | High | Full `cargo insta test` run, every changed snapshot reviewed |

## Amendments after the blueprint (accepted by the maintainer 2026-10-06)

The blueprint (`.claude/plans/mod-55-editor-agent-help.blueprint.md`) is authoritative where it
differs from the sections above. Accepted: **A-1** to **A-11** as written there, including:

- **A-1:** frames are told apart by `App::is_fresh` and one help per tab, not by `step_id`/seq.
- **A-2 (M-3):** an `Esc` before `ChatAccepted` defers the cancel; a help that is cancelling cannot be abandoned.
- **A-3:** the `serde` default is `"chat"`.
- **A-4:** `HelpTarget::{Template { name }, Skill { name }}`.
- **A-5:** the component is `agent_help.rs`/`AgentHelp`.
- **A-6 (M-2):** all ten `ToolKind`s are denied.
- **A-7:** `Failed` comes before `Ended` when the recorder refuses, and only `Ended { EndTurn }` proposes.
- **A-8:** `testkit.rs` and the `tests/templates.rs` hint assertion are added to the file list.
- **A-9:** a scrub refusal is a `Served::Reply(Failed)`.
- **A-10:** `start_chat_run_mints_chat_rows` is extended instead of adding a case.
- **A-11:** `scrub::REDACTED` becomes public.

- **M-1:** accept/discard is **not recorded**. Help runs are counted by `phase_name = 'edit_help'`;
  the accept rate comes from the maintainer's self-report in the close-out write-up (PRD metric
  amended).
- **H-6 (known limit, accepted):** on `claude-cli` with no lease the deny-all policy is not
  consulted. Only `--disallowedTools` binds, so `Task`, `TodoWrite`, user-configured MCP servers and
  an ACP agent's non-asking tools stay reachable. This is recorded in the write-up and not filed as
  an item.

## Verified claims

| Claim | Verdict | Evidence |
|---|---|---|
| MOD-67's action catalogue exists to register the action in | **FALSE → PRD constraint amended (P10)** | `HANDOFF.md` MOD-67 open, no milestone landed; `keymap.rs` holds only the global table; Skills views hard-code `KeyModifiers::CONTROL` chords |
| `Ctrl+G` is free in both editors and globally | TRUE | `TextArea::on_key` passes every chord but `Ctrl+S` (`text_area.rs:191-247`); editors take only `Ctrl+E`; global: `Ctrl+C` (`keymap.rs:205-254`), `Ctrl+F`, `Ctrl+W` (`app/mod.rs:95,106`) |
| Chat sends the prompt unscrubbed (scrubs only the persisted row) | TRUE | `driver.start(spec, prompt)` at `agent_worker.rs:4170` precedes `record_prompt` at `:4273`; scrubber built at `:4139` |
| A chat session never ends after one turn by itself | TRUE | `run_chat` blocks on `commands.recv()` after a turn (`agent_worker.rs:4345`); tests queue `ChatCancel` up front |
| The agent row's policy defaults to `Ask` (a help turn could park) | TRUE | `PermissionPolicy.default` docs "Defaults to `PermissionDefault::Ask`" (`htui-agent/src/driver.rs:192`) |
| A deny-all policy and kind denial are expressible without new types | TRUE | `PermissionDefault::Deny` (`driver.rs:121`); `ToolExposure.deny_kinds` is honoured by the relay, ACP `fs/*` and `claude-cli` argv (`driver.rs:212-223`) |
| `ChatRunSpec` is only built through `mint` (a new field breaks no literal) | TRUE | Every construction site is `ChatRunSpec::mint(` (20 sites; no struct literal outside `impl`) |
| `phase_name` of a chat step is read nowhere (renaming to `edit_help` breaks nothing) | TRUE | Hard-coded at `pg/write.rs:1936`, `mem.rs:2184`; no `phase_name == "chat"` comparison; item Runs panes show only item runs (`item_id NULL` for chats) |
| `phase_name` has no CHECK constraint (no migration needed) | TRUE | `0001_init.sql:478` `phase_name TEXT NOT NULL` with a comment only; `run.kind` CHECK is `('graph','chat')`, unchanged |
| Changing the insert needs a new sqlx offline entry | TRUE | The offline hash is the literal query (memory: sqlx offline hash = literal query); `.sqlx` lives at `crates/htui-store/.sqlx` |
| Templates are project-scoped (the editor knows its project) | TRUE | `Editor.project: ProjectId` (`templates.rs:206-226`) |
| Skills have no placeholder table and no `parse`-style gate | TRUE | Library save checks `skill_body_refusal` (blank/NUL) and head-identical only (`library.rs:1064-1111`, `htui-core/src/store/traits.rs:2195`) |
| A non-Chat view can receive a request's chat frames | TRUE | Replies route by `Origin::Tab(id)` (`app/update.rs:279-351`); every frame carries the `ChatStart` seq (`agent_worker.rs:5450-5458`) |
| `diff::unified(old, new, old_label, new_label) -> String` and `diff::lines` exist | TRUE | `crates/htui/src/ui/diff.rs` (~:20, ~:47); used at `templates.rs:1039-1047` |
| A fake driver with a one-turn script exists for tests | TRUE | `htui_agent::conformance::Script::one_turn` (`conformance.rs:96`); `FakeDriver`; `tests/chat.rs:77-130` |
| No public string-scrub helper exists today (T1 must add one) | TRUE | `scrub_text` is private (`prompt/mod.rs:1276`); only `Scrubber::scrub(&mut Value)` is public |
| **T1 ∥ T2 are independent** | TRUE | T1: `prompt/edit_help.rs`, `prompt/mod.rs`. T2: `model/run.rs`, `store/mem.rs`, `store/conformance.rs`, `pg/write.rs`, `.sqlx/`. Intersection empty |
| T3, T4 and T5 independent of each other | **FALSE → serial** | T3 defines the `StoreRequest::EditHelp` that T4 sends; T4 creates `help.rs`, which T5 embeds |

## Acceptance

- [ ] All tasks complete, tests written first
- [ ] Validation passes (featureless clippy included, `--test-threads=1`)
- [ ] Every changed snapshot reviewed
- [ ] Reviewer (`rust-reviewer`) findings applied or deferred with the maintainer
- [ ] Patterns mirrored, not reinvented
