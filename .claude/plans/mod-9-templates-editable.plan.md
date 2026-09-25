# Plan: MOD-9 milestone 1 — templates are editable

**Status: CONFIRMED by the maintainer 2026-09-25 with OQ-1..OQ-7 defaults as written.**

**Source**: `.claude/prds/mod-9-skill-library-templates.prd.md`, milestone 1 (Delivery Milestones
table, row 1): "Skills tab `Templates` view: list, body, diff between versions; `TextArea` and
`$EDITOR` editing; save runs `parse`, puts the cursor on the error, warns on a missing `{{item}}`,
and appends a version as a compare-and-set." Scope bullets "Skills tab, two views (D1)", "Multi-line
editor (D2)", "Template writer (D5)", "Version diff"; success-metric rows "Save gate", "Missing-item
warning", "Append-only versions", "Diff" (templates half), "`$EDITOR` round trip", "UI never
blocks". Design authority: the PRD's gate decisions **PRD D1–PRD D6** (settled, not reopened; PRD D5
is confirmed below as D1 with evidence); `docs/ANA-5.md` §4.1 (`:269-431`, the placeholder contract
and "Validation on save (MOD-9)" at `:398-404`), §4.6 (`:1215-1351`, reserved names and the two wire
contracts) and §6.2 (`:1841-1851`, "Skills tab, template editor" row).

**Requirements**: `R-PRM-4` (templates are versioned rows with a documented placeholder contract,
editable in the TUI), `R-TUI-7` (the Skills tab; the skills half is milestone 3), `R-NF-3` (every
read and write on the store worker; the `$EDITOR` child runs with the render loop suspended).

**Complexity**: Medium-large. No migration (templates already have version rows). One new
`WriteStore` method across five implementations plus three conformance cases; one new `.sqlx`
file; two `StoreRequest` and two `StoreReply` variants; a new reusable `TextArea` widget; an
external-editor runner with a terminal-suspension guard wired into the event loop; the Skills tab
rebuilt with a `Templates` view. **No new package in `Cargo.lock`**: `crates/htui` declares
`similar` (workspace, already compiled for `htui-agent`) and promotes `tempfile` from a dev- to a
normal dependency (already compiled via `gix`).

**Routing**: routed as PRD by `/handoff-run MOD-9`; ultracode for implementers. **Staffing: Opus
5.5 for every step — plan, fact-check, architect, implementers, verifiers and reviewer
(`rust-reviewer`, `.claude/workflow-config.json:2`); Fable is not used.** One ultracode workflow per
task; the architect and the reviewer stay plain agents.

**Numbering**: MOD-9's first plan. Its own decisions are **D1…D15**; the PRD's gate decisions are
always cited as **PRD D1…PRD D6**. Risks start at **R-1**, open questions at **OQ-1**, tasks at
**T1**. Milestone 2's plan continues from **D16**, **R-9**, **OQ-8**.

**Graphify / Gortex note**: `graphify-out/` does not exist and the Gortex MCP server is not
reachable in this session; every tree fact below was read with `grep`/`sed` at `da30927` and carries
a `file:line`.

---

## Open questions for the maintainer (read these first)

Each has a default this plan adopts so implementation is not blocked.

- [ ] **OQ-1 — How `$EDITOR` is chosen and invoked, Windows included** (PRD open question).
      **Default (D8):** `$VISUAL`, then `$EDITOR` (first non-blank), then `vi` on Unix and `notepad`
      on Windows. The value is handed to the platform shell exactly as a user would type it:
      Unix `sh -c '<value> "$1"' htui-editor <file>` (git's own shape; the path is a positional
      argument, never interpolated), Windows `cmd /S /C "<value> "<file>""` through
      `CommandExt::raw_arg`. No splitter and no new dependency. **Alternative:** split with
      `shell-words 1.1.1` (already in the lock via `agent-client-protocol`), which mangles an
      unquoted `C:\Tools\ed.exe` because it treats `\` as an escape.
- [ ] **OQ-2 — Scope selector's project or its own picker** (PRD open question). **Default (D6):**
      the view follows the scope and lists **every** project of the workspace, templates grouped
      under a project header row, as `Settings > Kinds` does. There is no per-project selector in
      the shell today (`Scope` is `workspace_id` + `project_ids`, `model/scope.rs:10-15`).
      **Alternative:** a project picker line at the top of the view.
- [ ] **OQ-3 — What `$EDITOR` returning does.** **Default (D11):** the edited text lands in the
      in-app `TextArea`, `parse` runs at once and the cursor goes to the error, but nothing is saved
      until `Ctrl+S` — one save path, one warning path. **Alternative:** save on return when `parse`
      passes (git-commit style), falling back to the `TextArea` only on an error.
- [ ] **OQ-4 — Does the store re-run `parse`?** **Default (D4):** yes — `append_prompt_template`
      refuses what `parse` refuses with `StoreError::Constraint`, so no caller (a future CLI, MOD-50)
      can write a row the assembler would fail on at stage 3. The UI still parses first, for the
      cursor. **Alternative:** UI-only validation, leaving ANA-5's stage-3 failure as the only
      backstop.
- [ ] **OQ-5 — Saving from an older version.** **Default (D1):** allowed and useful as a revert —
      `e` on v1 while v3 is the head opens v1's body and saves it as v4. The token is always the
      head, never the version shown. **Alternative:** `e` only on the head.
- [ ] **OQ-6 — `Esc` with unsaved changes.** **Default (D11):** asks once ("unsaved changes — Esc
      again discards"); no undo stack (PRD risk row 5). **Alternative:** discard silently.
- [ ] **OQ-7 — Diff against the compiled default.** **Default (D12):** `D` diffs the shown version
      against `htui_core::prompt::body_of(name)` when the name has one — the PRD's mitigation for the
      "saved template breaks a wire contract" risk. **Alternative:** version-to-version only.

---

## Summary

`prompt_template` has `UNIQUE (project_id, name, version)` and no writer beyond the project seed
(`crates/htui-store/src/pg/write.rs:4317-4338`, `crates/htui-core/src/store/mem.rs:2052-2062`) and
the demo loader (`crates/htui-store/src/pg/demo.rs:233-245`). The Skills tab is a 61-line stub
(`crates/htui/src/ui/tabs/skills.rs`), there is no multi-line widget (`ui/text_field.rs:1-7`) and
no `$EDITOR` handling anywhere in `crates/`.

**The writer (T1).** `WriteStore::append_prompt_template(new, expected: Option<i32>)` inserts version
`expected + 1` iff the head version of `(project, name)` is `expected` (`None` = the name has no row
yet), in one `INSERT … SELECT … WHERE head = $expected ON CONFLICT DO NOTHING RETURNING` statement
on Postgres and one locked closure on `MemStore`. A miss answers `CasOutcome::Stale(head)`. The
store also refuses a blank name and anything `parse` refuses. Three conformance cases prove it on
both stores; a Postgres-only case proves two concurrent saves of the same head yield one row.

**The widget (T2).** `ui::TextArea`: a byte-offset cursor over a `String`, insert, delete,
newline, arrows, Home/End, PgUp/PgDn, `set_cursor(byte)` (UTF-8 safe), viewport render. Nothing
more (PRD risk row 5); MOD-7 milestone 2's quirks editor reuses it (MOD-7 PRD D3).

**The handoff (T3).** `editor::run` writes the body to a named temp file, runs the resolved editor
command, reads the file back, and answers `Edited`, `Unchanged` or `Failed`. The event loop owns the
suspension: it drops crossterm's `EventStream` (whose reader thread would otherwise steal the
editor's keystrokes), leaves raw mode and the alternate screen through a guard that re-enters them
on every path out, runs the editor, recreates the stream and resets the ticker. A view asks through
`Action::EditExternally` and hears back through a new defaulted `Tab::on_external_edit`.

**The worker (T4).** `StoreRequest::Templates(Scope)` and `StoreRequest::SaveTemplate { .. }`
served by a new `crates/htui/src/templates.rs` (the `prompt_settings.rs` shape), answered with
`StoreReply::Templates` / `StoreReply::TemplatesStale`. `created_by` is the worker's
`Backend::this_user()`; the view never holds a `UserId`.

**The view (T5).** The Skills tab gets a `Skills | Templates` switch; `Templates` lists the scope's
templates, shows a version's body, diffs any two versions (and a version against the compiled
default), edits in the `TextArea` or `$EDITOR`, and saves through `parse` with the cursor on the
error and a confirm on a phase body without `{{item}}`. The tab title stays `Skills`.

## Design decisions (settled here, not in code review)

| # | Decision | Why / evidence |
|---|---|---|
| D1 | **PRD D5 confirmed: append-only, the head version is the CAS token, new names allowed, no delete.** Save inserts `head + 1`; the token is the head version the view saw when the editor opened (`expected: Option<i32>`, `None` for a name with no row), **not** the version shown — editing v1 while v3 is head saves v4 (OQ-5). A new name starts at version 1 with its role from `TemplateRole::of_name` (`template.rs:52`). No update of `body` in place and no delete. | Rows are referenced by version from three places that an in-place edit or a delete would silently rewrite: `step_graph_phase.template_version` (`0001_init.sql:243`, `model/kind.rs:206-207`), the resolved snapshot version (`model/run.rs:534`) and `trim_record.template` (`prompt/trim.rs:157`). `UNIQUE (project_id, name, version)` (`0001_init.sql:275`) makes `max(version)` a token only this writer and the seed move — the PRD constraint "keyed on a token only its own writes change". `updated_at` is useless as a token: rows are never updated. |
| D2 | **The writer goes on `WriteStore`; the reads stay inherent.** `async fn append_prompt_template(&self, new: NewPromptTemplate, expected: Option<i32>) -> Result<CasOutcome<PromptTemplate>>`, in a new `// prompt_template (MOD-9 milestone 1)` block after `phases` (`traits.rs:620`). | The reads are inherent because `ReadStore` is what `CacheStore` implements (`cache/read.rs:240`) and the mirror has no `prompt_template` (`traits.rs:25-27`, `backend.rs:335-340`). `WriteStore` has no `CacheStore` impl — its implementors are `PgStore` (`pg/write.rs:384`), `MemStore` (`mem.rs:4420`), `Writer` (`writer.rs:288`) and two spies (`htui-agent/src/conformance.rs:674`, `tests/recorder.rs:354`) — so the mirror argument does not reach writes. Precedent is exact: `upsert_agent_box` and `record_box_probe` write unmirrored tables (`agent_box`, `box_tool`) as trait methods while `agents()` stays inherent (`traits.rs:13-17`). The worker writes only through `Backend::writer()` (`backend.rs:6-9`, `:152-158`), and `store::conformance` is generic over `WriteStore` (`conformance.rs:105`), which is how the PRD's "conformance case over `MemStore` and `PgStore`" metric is met by one case. Cost: two one-line spy delegations. |
| D3 | **Postgres SQL shape.** `INSERT INTO prompt_template (id, project_id, name, version, body, created_by) SELECT $1, $2, $3, COALESCE($6::int, 0) + 1, $4, $5 WHERE COALESCE((SELECT max(version) FROM prompt_template WHERE project_id = $2 AND name = $3), 0) = COALESCE($6::int, 0) ON CONFLICT (project_id, name, version) DO NOTHING RETURNING …` as one `query_as!` (one new `.sqlx` file). Zero rows → re-read the head through the existing inherent `PgStore::prompt_template(project, name, None)` (`pg/read.rs:1401`, no new query): `Some(head)` → `Stale(head)`; `None` with `expected: Some(_)` → `StoreError::NotFound { entity: "prompt_template", id: "<project>/<name>" }`. A missing project or user surfaces as the FK's `23503` through `map_sqlx` → `Constraint`. | Probed on Postgres 16.13 at `localhost:5432` (scratch DB, dropped): sequential saves at the same head give `2|mine` then `INSERT 0 0`; a new name with `NULL` gives version 1, a second `NULL` gives `INSERT 0 0`; two concurrent transactions with the same token — the first holding for 2 s — give the first `3|A` and the second, after blocking on the unique index, `INSERT 0 0`. A gap in versions (hand-inserted rows) cannot fool it because the `WHERE` compares `max`, not `expected + 1`'s absence. |
| D4 | **Validation in the store, behind the CAS (OQ-4).** Order on both stores, mirroring `update_item_kind`'s (`pg/write.rs:1715-1729`, review M2): if the input is bad (name fails `PromptTemplate::name_is_valid` — non-empty, no leading/trailing whitespace, no `\n`/`\r` — or `parse(TemplateRole::of_name(name), body)` errs), read the head first: head ≠ expected → `Stale(head)`; else `Constraint(invalid_template_name(name))` or `Constraint(err.to_string())`. `MemStore` checks `require_user(created_by, "created_by")` (`mem.rs:1800`) and the project (`references_no_row`, `traits.rs:1226`) inside the same `write` closure. | One validator (`parse`, PRD constraint). A stale token answers `Stale` whether or not the input is also bad, so the two stores cannot disagree about which refusal a caller sees. `TemplateError`'s `Display` (`template.rs:377-411`) is the same string on both stores. |
| D5 | **Worker surface.** `StoreRequest::Templates(Scope)`; `StoreRequest::SaveTemplate { scope: Scope, project: ProjectId, name: String, body: String, expected: Option<i32> }`; `StoreReply::Templates(Box<TemplatesSnapshot>)`; `StoreReply::TemplatesStale(Box<TemplatesSnapshot>)`. New `crates/htui/src/templates.rs`: `TemplatesSnapshot { projects: Vec<ProjectTemplates> }`, `ProjectTemplates { project_id: ProjectId, templates: Vec<PromptTemplate> }` (scope order; `(name, version)` byte order as the reads return), `pub async fn serve(backend, request)`, `REQUEST_NAMES: [&str; 2] = ["templates", "save_template"]`, `READ_NAME`. The read calls `Backend::prompt_templates` per project (`backend.rs:348`; offline it refuses with `PROMPT_ON_SERVER_ONLY`); the save calls `backend.writer()` (`None` → `Unreachable(DATABASE_UNREACHABLE)`), `backend.this_user()` (`backend.rs:174`, as `hierarchy.rs:184`), the writer, then re-reads; `Applied` → `Templates`, `Stale` → `TemplatesStale` (`catalogue.rs:309-315`'s `cas`). Routing: one or-ed arm in `try_serve` beside `prompt_settings` (`store_worker.rs:1052-1054`). | The `prompt_settings.rs` / `catalogue.rs` pattern end to end, including its known residue (a failed re-read after an applied write answers `Failed`, `prompt_settings.rs:8-11`). `R-NF-3`: nothing on the render side touches a store. |
| D6 | **The view follows the scope (OQ-2).** One `Templates(scope)` read on activation and scope change; the list is a tree: project header rows (slug from `ctx.projects`, `ProjectRef`, `model/hierarchy.rs:210-219`) then one row per template **name** with its head version and role, e.g. `implement   v2  phase`. `on_scope_change` drops the snapshot and any open editor (PromptSection's rule, `settings/prompt.rs:798-805`). | No project selector exists in the shell; `Settings > Kinds` already renders every scope project (`catalogue.rs:24-37`). A picker would be a second scope concept. |
| D7 | **`TextArea` scope.** `crates/htui/src/ui/text_area.rs`: `pub struct TextArea { text: String, cursor: usize /* byte offset, always a char boundary */, top: usize, left: usize, goal_col: Option<usize> }`; `pub enum AreaOutcome { Consumed, Cancel, Pass }`; `new()`, `with_text(&str)`, `text() -> &str`, `into_text()`, `cursor() -> usize`, `set_cursor(byte)` (clamps to `len`, floors to a char boundary with `str::is_char_boundary`), `cursor_line_col() -> (usize, usize)` (chars), `on_key(KeyEvent, page: u16) -> AreaOutcome`, `lines(width, height, focused, &Theme) -> Vec<Line<'static>>` (scrolls `top`/`left` so the cursor is visible; cursor cell styled `theme.selected`). Keys: printable `Char` inserts; `Enter` inserts `\n`; `Backspace`/`Delete` (joining lines); `Left`/`Right` (crossing lines); `Up`/`Down` keep the goal column; `Home`/`End` line bounds; `PageUp`/`PageDown` by `page` lines; `Esc` → `Cancel`; any `CONTROL`/`ALT`/`SUPER`/`META`/`HYPER` chord, `Tab`, `BackTab`, `F(n)` → `Pass`. No wrap, no undo, no selection, no paste, no `Debug` of the text (hand-written `Debug` printing lengths, `text_field.rs:56-64`'s rule). | PRD risk row 5 caps it; `set_cursor` is what "puts the cursor on the error" needs. Width in `char`s like `TextField` (`text_field.rs:4-7`: no `unicode-width` is declared). `Tab` passes so the global `Tab` still switches tabs; the draft survives because the tab struct does. |
| D8 | **`$EDITOR` resolution and invocation (OQ-1).** `crates/htui/src/editor.rs`: `pub struct EditorCommand { value: String, fallback: bool }` with `EditorCommand::resolve(lookup: impl Fn(&str) -> Option<String>) -> Self` (`VISUAL`, then `EDITOR`, first non-blank; else `vi` / `notepad` by `cfg`) and `fn command(&self, file: &Path) -> std::process::Command`: `#[cfg(unix)]` `sh -c "<value> \"$1\"" htui-editor <file>`; `#[cfg(windows)]` `cmd` + `raw_arg(format!("/S /C \"{value} \"{file}\"\""))` (`/S` makes `cmd` strip exactly the outer pair of quotes). Converted with `tokio::process::Command::from` and awaited with `.status()`; stdio inherited. | The shell parses the value as the user's own shell would (git runs `$EDITOR` the same way), so `code --wait`, `"C:\Program Files\…\notepad++.exe" -multiInst` and `nvim -u NONE` all work with no splitter. The file is a positional argument, never spliced into the script. Verified on Linux by a compile probe (below). Windows is reviewed by eye; `cargo clippy --target x86_64-pc-windows-msvc` dies in `ring` on this box (TOOL-3, README `:451`), and `tokio::process::Command::raw_arg` exists in the locked tokio 1.53.1 (`process/mod.rs:420`). |
| D9 | **The runner and the suspension.** `pub async fn run(cmd: &EditorCommand, text: &str, stem: &str) -> ExternalEditOutcome`: `tempfile::Builder::new().prefix(&format!("htui-{stem}-")).suffix(".md").tempfile()`, write, `into_temp_path()` (closes the handle so Windows editors can write; the path is removed on drop, every path), run, `read_to_string`, `render::normalise_newlines` (`render.rs:111`: CRLF, lone CR, BOM), compare. `pub enum ExternalEditOutcome { Edited(String), Unchanged, Failed(String) }` — `Failed` for a spawn error ("could not start `vi`: … — set $VISUAL or $EDITOR"), a non-zero exit ("`vi` exited with 3; nothing was changed"), or non-UTF-8 content. **Suspension:** `pub trait Suspend { fn leave(&mut self) -> io::Result<()>; fn enter(&mut self) -> io::Result<()>; }`, implemented by `TerminalGuard` (`leave` = `ratatui::try_restore()`; `enter` = `enable_raw_mode()`, `execute!(stdout(), EnterAlternateScreen)`, `terminal.clear()`); `pub async fn run_suspended<S: Suspend>(term: &mut S, cmd, text, stem) -> ExternalEditOutcome` calls `leave`, then `run` under a `Resume<'_, S>` guard whose `Drop` calls `enter` unless `std::thread::panicking()`. A `leave` failure answers `Failed` without spawning. **Event loop** (`event_loop.rs:21-51`): after each select arm, `if let Some((tab, edit)) = app.take_external_edit()` → `drop(events)` → `run_suspended(term, …).await` → `events = EventStream::new()` → `ticker.reset()` → `app.finish_external_edit(tab, outcome)` (sets `dirty`; the next draw is full because `enter` cleared the buffer). | crossterm 0.29's `EventStream` parks a thread in `poll_internal(None, …)` that reads the tty (`crossterm-0.29.0/src/event/stream.rs:44-55`); only `Drop` shuts it down (`:140-145`), so the stream must go before the child starts. Not calling `ratatui::init()` again avoids stacking another panic hook (`ratatui-0.30.2/src/init.rs:397-403` sets one each call). `interval` defaults to `Burst`, so without `reset` a long edit replays a burst of ticks. The panic path is already covered: the hook restores (`terminal.rs:35-43`) and `TerminalGuard::drop` restores again (`:78-82`). Replies keep queueing in the unbounded channel while the loop waits (`R-NF-3`: the store worker is its own task). |
| D10 | **How a view asks and hears back.** `Action::EditExternally(ExternalEdit)` where `ExternalEdit { text: String, stem: String }` with a hand-written `Debug` (lengths only). `App::drain` stamps it like `Action::Store` (`state.rs:319-322`): from `Origin::Tab(id)` it sets `App.pending_edit = Some((id, edit))`; from any other origin it is an `Action::Error("only a tab can open the editor")`. `pub fn take_external_edit(&mut self) -> Option<(TabId, ExternalEdit)>`; `pub fn finish_external_edit(&mut self, tab: TabId, outcome: ExternalEditOutcome)` routes to `tabs.by_id_mut(tab)` with a `Ctx` and drains, as `on_reply` does (`update.rs:176-201`). `Tab` gains `fn on_external_edit(&mut self, _outcome: ExternalEditOutcome, _ctx: &mut Ctx<'_>) {}` — the trait's second default after `focus_section` (`registry.rs:57-59`), so no other tab changes. | The shell never names a concrete view (`app/mod.rs` doc); the event loop stays three `select!` arms plus one post-step, and the whole flow is testable in `Harness` without a terminal by calling `take_external_edit` / `finish_external_edit` directly. |
| D11 | **The save flow in the view.** `Ctrl+S` → `parse(TemplateRole::of_name(name), text)`. `Err(e)` with `at` → `area.set_cursor(at)`, notice = `e.to_string()`, no request; `MissingRequired` → cursor to `text.len()`. `Ok(p)` with role `Phase` and `p.omits_item()` and no pending confirm → notice "this phase template never places {{item}} — Ctrl+S again saves anyway", `confirm_item = true`; any edit clears it. Otherwise send `SaveTemplate { expected: self.token }` with `busy = Some("save_template")` (one write in flight, `settings/prompt.rs:255`, `:429-431`). `Templates` while `busy` → close the editor, select the saved name at its new head. `TemplatesStale` → keep the draft, set `token` to the new head, notice `TEMPLATE_CHANGED_ELSEWHERE` = "saved elsewhere since you opened it — v{N} is now the latest; your draft is kept and Ctrl+S saves it as v{N+1}". `Failed` for a `REQUEST_NAMES` entry → `busy = None`, notice. `Esc` unmodified → Browse; modified → "unsaved changes — Esc again discards" once (OQ-6). `$EDITOR` result (OQ-3): `Edited(text)` → Edit mode on that text with the token captured when `E` was pressed, then the same `parse` as `Ctrl+S` minus the send (cursor on the error or at the start); `Unchanged` → notice "no changes" (plus "use `code --wait`" when the editor exited in under a second); `Failed(msg)` → notice, draft untouched. | ANA-5 `:398-404` (four checks; warn, never refuse, on a missing `{{item}}`); `omits_item` is true for every judge/handoff body, hence the role guard. The existing `CHANGED_ELSEWHERE` names `Enter` (`settings/mod.rs:103`), which is not this editor's key, so the view carries its own sentence. |
| D12 | **Diff.** `crates/htui/Cargo.toml` declares `similar = { workspace = true }` (no new package: `Cargo.toml:75`, lock `similar 3.2.0`). New `crates/htui/src/ui/diff.rs`: `pub fn unified(old: &str, new: &str, old_label: &str, new_label: &str) -> String` (`TextDiff::from_lines(..).unified_diff().context_radius(3).header(..).to_string()`, the shape of `htui-agent/src/acp/fs.rs:112-118`) and `pub fn lines(diff: &str, theme: &Theme) -> Vec<Line<'static>>` using `diff_style`, **moved** from `chat/transcript.rs:627-639` and imported back there. Any two versions: `b` marks the shown version as the base, `,`/`.` move the shown version, `d` toggles the diff base → shown (default base: shown − 1); `D` diffs the compiled default (`prompt::body_of`, `defaults.rs:237`) → shown (OQ-7). Identical texts render "no differences". | PRD scope "rendered with the chat transcript's `diff_style`". Not reusing `htui_agent::acp::fs::unified_diff` keeps the Skills tab off an ACP module. |
| D13 | **Keys.** Global (unchanged): `q`, `Tab`, `Shift+Tab`, `1`–`9`, `?`, `w` (`keymap.rs:199-239`, `app/mod.rs:63-68`). Skills tab, Browse: `h`/`l`/`[`/`]`/`Left`/`Right` switch `Skills | Templates` (Settings' section keys, `settings/mod.rs:331-338`); Templates Browse: `j`/`k`/`Down`/`Up` row, `,`/`.` older/newer version, `b` base, `d` diff, `D` diff vs default, `e` edit in-app, `E` edit in `$EDITOR`, `n` new name (single-line `TextField` prompt; `Enter` opens an empty editor, or `body_of(name)` when the name has a default; a name already in the project → "exists — select it and press e"), `r` reload. Edit mode consumes every key the `TextArea` takes plus `Ctrl+S` save, `Ctrl+E` hand the draft to `$EDITOR`, `Esc` cancel; `Tab`/`Shift+Tab` pass. None collides with a global binding in Browse. | `Ctrl+S` reaches the app: crossterm's raw mode clears `IXON`. The chat composer's rule (`chat/mod.rs:485-495`): an open editor owns every key it uses. |
| D14 | **The tab.** `ui/tabs/skills.rs` becomes `ui/tabs/skills/mod.rs` (`SkillsTab`, the view switch, title `"Skills"`, `ID` unchanged) + `ui/tabs/skills/templates.rs` (`TemplatesView`). The `Skills` view renders one dim line: "Skills are edited here from MOD-9 milestone 3." — the stub's MOD-12 attribution (`skills.rs:1`, `:15`, `:53`) is corrected (PRD "Record corrections"). Edit mode layout: `TextArea` on the left, inline help on the right: the role, the tokens of `Placeholder::ALL` filtered by `allowed_in(role)` (`template.rs:128`, `:187`), each marked `scalar`/`section` (`is_section`) and `required` (`required_by`, `:233`), plus a wire-contract note for `review` (the three-line `verdict` front matter, ANA-5 `:1315-1330`) and `judge` (one fenced `json` block last, `:1332-1349`). `wants_requests` always returns `[Templates(scope)]`. | ANA-5 §6.2 (`:1848`): errors on the cursor, the placeholder list as inline help, the role from the name. There is no runtime description per placeholder (PRD Evidence), so help lists tokens, not prose. |
| D15 | **What milestone 1 leaves alone, by name.** Milestone 2's engine and preview wiring (`engine.rs:4310`, `:4925-4927`, `preview.rs:227`); every skill writer and the Skills view's content (milestone 3, ANA-22); `SKILL.md` import; MOD-50's agent help; syntax highlighting, bracketed paste, undo; any `app_setting` for the editor command; any migration; `htui-orch`; `Backend`'s read methods; `htui_core::prompt::template` (read-only use). `docs/**`, `HANDOFF.md` and the PRD are the main thread's. | PRD scope and D6 sequencing. |

## Patterns to Mirror

| Concern | Mirror | Where |
|---|---|---|
| A CAS writer across five implementations | `update_item_kind` | `traits.rs:544`; `mem.rs:2343`, `:4604`; `pg/write.rs:1730`; `writer.rs:546`; `htui-agent/src/conformance.rs:848`; `htui-agent/tests/recorder.rs:543` |
| `CasOutcome` | `Applied(T)` / `Stale(T)` | `traits.rs:1380-1395` |
| A conformance case and its two pins | `CASES` | `conformance.rs:37`, `:105`; `htui-core/tests/mem_store.rs:36-37`; `htui-store/tests/pg_conformance.rs:19` |
| A `created_by` check on `MemStore` | `State::require_user` | `mem.rs:1799-1808` |
| Worker-side serve module, `REQUEST_NAMES`, CAS reply | `prompt_settings.rs`, `catalogue.rs::cas` | `prompt_settings.rs:182-256`; `catalogue.rs:297-331` |
| Or-ed routing arm | prompt settings arm | `store_worker.rs:1052-1054` |
| `created_by` resolved by the worker | `hierarchy::serve` | `hierarchy.rs:184` |
| One write in flight, reply-driven editor close | `PromptSection::busy`, `on_settings` | `settings/prompt.rs:255`, `:429-456`, `:771-783` |
| A widget with a hand-written `Debug` and `on_key` outcome enum | `TextField`, `FieldOutcome` | `ui/text_field.rs:24-64`, `:113-164` |
| A trait default so one view's feature changes no other view | `Tab::focus_section` | `registry.rs:49-59` |
| Stamping an emitted action with its origin | `App::drain` | `state.rs:313-330` |
| Unified diff text | `acp::fs::unified_diff` | `htui-agent/src/acp/fs.rs:106-118` |
| Harness-driven tab tests and snapshots | `tests/prompt_settings.rs`, `tests/kinds.rs` | `crates/htui/tests/` |
| A Postgres case on a throwaway demo database | `pg_conformance.rs` | `htui-store/tests/pg_conformance.rs:29-42` (`testkit::demo_db`) |

## Files to Change

| File | Action | Task | Why |
|---|---|---|---|
| `crates/htui-core/src/model/kind.rs` | edit | T1 | `NewPromptTemplate { id, project_id, name, body, created_by }`; `PromptTemplate::name_is_valid` + unit tests |
| `crates/htui-core/src/model/mod.rs` | edit | T1 | re-export `NewPromptTemplate` (`:115-119`) |
| `crates/htui-core/src/store/traits.rs` | edit | T1 | `append_prompt_template`; `invalid_template_name(name)` message helper beside `reserved_phase_name` (`:1204`) |
| `crates/htui-core/src/store/mem.rs` | edit | T1 | `State::append_prompt_template` + trait arm; `templates` field doc (`:106`) names its writer |
| `crates/htui-core/src/store/conformance.rs` | edit | T1 | three cases appended to `CASES` and `run_case` |
| `crates/htui-core/tests/mem_store.rs` | edit | T1 | pin 56 → 59 (`:37`) |
| `crates/htui-store/src/pg/write.rs` | edit | T1 | `impl WriteStore for PgStore`: D3/D4 |
| `crates/htui-store/src/writer.rs` | edit | T1 | dispatch arm |
| `crates/htui-store/tests/pg_conformance.rs` | edit | T1 | `EXPECTED_CASES` 56 → 59 (`:19`) |
| `crates/htui-store/tests/prompt_template_cas.rs` | create | T1 | the concurrent-save case |
| `crates/htui-store/.sqlx/query-<hash>.json` | create | T1 | the one new `query_as!` |
| `crates/htui-agent/src/conformance.rs` | edit | T1 | `UsageSpy` delegation |
| `crates/htui-agent/tests/recorder.rs` | edit | T1 | `SpyStore` delegation |
| `crates/htui/src/ui/text_area.rs` | create | T2 | D7 + unit tests |
| `crates/htui/src/ui/mod.rs` | edit | T2 | `pub mod text_area; pub use text_area::{AreaOutcome, TextArea};` |
| `crates/htui/src/editor.rs` | create | T3 | D8, D9 + unit tests |
| `crates/htui/src/terminal.rs` | edit | T3 | `impl editor::Suspend for TerminalGuard` |
| `crates/htui/src/event_loop.rs` | edit | T3 | the post-step of D9 |
| `crates/htui/src/app/action.rs` | edit | T3 | `Action::EditExternally(ExternalEdit)` |
| `crates/htui/src/app/state.rs` | edit | T3 | `pending_edit`, drain stamping, `take_external_edit`, `finish_external_edit` + unit tests |
| `crates/htui/src/app/update.rs` | edit | T3 | `update()` arm for an unstamped `EditExternally` (origin `App`) → `Action::Error` |
| `crates/htui/src/ui/tabs/registry.rs` | edit | T3 | `Tab::on_external_edit` default |
| `crates/htui/src/lib.rs` | edit | T3, T4 | `pub mod editor;` (T3), `pub mod templates;` (T4) |
| `crates/htui/Cargo.toml` | edit | T3, T5 | `tempfile = "3"` moved to `[dependencies]` with a comment (T3); `similar = { workspace = true }` (T5) |
| `crates/htui/src/templates.rs` | create | T4 | D5 + unit tests over `Backend::memory(MemStore::demo())` |
| `crates/htui/src/store_worker.rs` | edit | T4 | two requests, two replies, two `name()` arms, one routing arm |
| `crates/htui/src/ui/tabs/skills.rs` | delete | T5 | becomes a directory module |
| `crates/htui/src/ui/tabs/skills/mod.rs` | create | T5 | `SkillsTab`, view switch (D14) |
| `crates/htui/src/ui/tabs/skills/templates.rs` | create | T5 | `TemplatesView` (D6, D11–D14) |
| `crates/htui/src/ui/diff.rs` | create | T5 | D12 |
| `crates/htui/src/ui/tabs/chat/transcript.rs` | edit | T5 | `diff_style` imported from `ui::diff` |
| `crates/htui/tests/templates.rs` | create | T5 | Harness tests + snapshots |
| `crates/htui/tests/snapshots/templates__*.snap` | create | T5 | the six snapshots of the test plan |
| `crates/htui/tests/templates_pg.rs` | create | T6 | end to end on Postgres |

**Not touched:** `Cargo.toml`, `Cargo.lock` (verified by `git diff --exit-code Cargo.lock`), every
migration, `htui-orch`, `backend.rs`, `pg/read.rs`, `prompt/template.rs`, `prompt/render.rs`,
`keymap.rs`, `app/mod.rs` (`SkillsTab::new()` keeps its path through `ui/tabs/mod.rs:13`), every
Settings section, every existing snapshot.

## Tasks

**Wave A: T1 ∥ T2 ∥ T3**, each in its own git worktree. **Wave B: T4** once T1 and T3 are merged
(it calls T1's trait method and shares `lib.rs` with T3). **Then T5** (needs T2, T3, T4). **Then
T6.** Merge order in Wave A: T1, T2, T3; re-run the touched crates' gates on the real tree after
each merge.

| Task | Files (complete list) | Parallel |
|---|---|---|
| T1 | `crates/htui-core/src/model/kind.rs`, `crates/htui-core/src/model/mod.rs`, `crates/htui-core/src/store/traits.rs`, `crates/htui-core/src/store/mem.rs`, `crates/htui-core/src/store/conformance.rs`, `crates/htui-core/tests/mem_store.rs`, `crates/htui-store/src/pg/write.rs`, `crates/htui-store/src/writer.rs`, `crates/htui-store/tests/pg_conformance.rs`, `crates/htui-store/tests/prompt_template_cas.rs`, `crates/htui-store/.sqlx/query-<hash>.json`, `crates/htui-agent/src/conformance.rs`, `crates/htui-agent/tests/recorder.rs` | Wave A |
| T2 | `crates/htui/src/ui/text_area.rs`, `crates/htui/src/ui/mod.rs` | Wave A |
| T3 | `crates/htui/src/editor.rs`, `crates/htui/src/terminal.rs`, `crates/htui/src/event_loop.rs`, `crates/htui/src/app/action.rs`, `crates/htui/src/app/state.rs`, `crates/htui/src/app/update.rs`, `crates/htui/src/ui/tabs/registry.rs`, `crates/htui/src/lib.rs`, `crates/htui/Cargo.toml` | Wave A |
| T4 | `crates/htui/src/templates.rs`, `crates/htui/src/store_worker.rs`, `crates/htui/src/lib.rs` | Wave B, after T1 + T3 |
| T5 | `crates/htui/src/ui/tabs/skills.rs` (delete), `crates/htui/src/ui/tabs/skills/mod.rs`, `crates/htui/src/ui/tabs/skills/templates.rs`, `crates/htui/src/ui/diff.rs`, `crates/htui/src/ui/mod.rs`, `crates/htui/src/ui/tabs/chat/transcript.rs`, `crates/htui/Cargo.toml`, `crates/htui/tests/templates.rs`, `crates/htui/tests/snapshots/templates__*.snap` | serial, after T2 + T3 + T4 |
| T6 | `crates/htui/tests/templates_pg.rs` | serial, last |

**Intersections, checked.** T1 ∩ T2 = T1 ∩ T3 = ∅ (T1 is `htui-core`, `htui-store`, `htui-agent`
only). T2 ∩ T3 = ∅ (`ui/text_area.rs`, `ui/mod.rs` vs. the nine T3 files). T3 ∩ T4 = `{lib.rs}` →
serial. T2 ∩ T5 = `{ui/mod.rs}`, T3 ∩ T5 = `{Cargo.toml}` → serial. **Build coupling:** T1 adds a
trait method and implements it in all five implementors inside the task, so its tree compiles
alone; `crates/htui` compiles against it unchanged (no caller yet). T2 and T3 add items nothing
calls until T5. `.sqlx` moves only in T1. No snapshot moves before T5, and T5 adds files only.

Every implementer prompt carries: PRD D1–D6 win over this plan; read the tree (no `graphify-out/`,
no Gortex); `parse` is the only validator; nothing sets `updated_at` by hand; every new `pub` item
has a doc comment and a `Debug` (each lib root warns on `missing_docs`, `htui/src/lib.rs:10`; the
workspace on `missing_debug_implementations`); commit the red tests first, then green; run the gate
with `--test-threads=1` on the real tree after the merge.

### Task 1: the writer on every store (D1–D4)
- **Files**: as tabled.
- **Tests first** — `conformance.rs`, appended to `CASES` in this order:
  `prompt_template_append_is_a_cas_on_the_head` (demo `PROJECT_HTUI`/`implement` is at v1: save
  with `Some(1)` → `Applied` v2 with the body and `created_by`; again with `Some(1)` → `Stale` whose
  row is v2 with the first body; `Some(2)` → `Applied` v3; `Some(7)` on a name with no row →
  `NotFound`); `prompt_template_new_name_starts_at_one` (`"triage"` with `None` → `Applied` v1;
  `None` again → `Stale` v1; `"judge"`-role check: a new name `judge` in a project that already has
  it → `Stale`); `prompt_template_refuses_what_parse_refuses` (`{{itme}}` in `implement` at the head
  → `Constraint` containing `unknown prompt placeholder`; a `judge` body without `{{candidates}}` →
  `Constraint` containing `must use candidates`; a blank name and `" plan"` → `Constraint`; the same
  bad body with a **stale** token → `Stale`, not `Constraint` (D4 order); then a valid save at the
  head → `Applied` head + 1, proving the refusals wrote nothing; an unknown project with `None` →
  `Constraint`). Pins: `mem_store.rs:37` and `pg_conformance.rs:19` → 59.
  `kind.rs` unit test `template_names_are_trimmed_single_line_and_non_empty`.
  `tests/prompt_template_cas.rs::two_saves_at_one_head_write_one_row` (`testkit::demo_db`; two
  `append_prompt_template` calls with `Some(1)` joined with `tokio::join!` on the pool; exactly one
  `Applied`, one `Stale`, `prompt_templates` holds v1 and v2 only).
- **Action**: D2's signature and doc (errors: `NotFound`, `Constraint`); `MemStore` inside one
  `write` closure (head by `max_by_key(version)` over `(project, name)`, D4 order,
  `require_user`, project check, push the row with `created_at = updated_at = now`); `PgStore`
  per D3/D4; `Writer` arm; two spy delegations. Regenerate `.sqlx` (Validation).
- **Validate**: `cargo test -p htui-core --all-features -- --test-threads=1`;
  `cargo test -p htui-store --all-features -- --test-threads=1` with `HTUI_TEST_DATABASE_URL`;
  `cargo test -p htui-agent --all-features -- --test-threads=1`; `cargo sqlx prepare --check`;
  `ls crates/htui-store/.sqlx | wc -l` = 235.

### Task 2: `TextArea` (D7)
- **Files**: as tabled.
- **Tests first** (`text_area.rs` unit tests): `typing_inserts_at_the_cursor`,
  `enter_splits_the_line_and_backspace_joins_it_again`, `delete_at_line_end_joins_the_next_line`,
  `left_and_right_cross_line_boundaries`, `up_and_down_keep_the_goal_column_through_a_short_line`,
  `home_and_end_stay_on_the_line`, `page_down_moves_by_the_page_and_clamps`,
  `set_cursor_floors_inside_a_multibyte_char` (`"é{{x"`: `set_cursor(1)` → 0, `set_cursor(2)` → 2),
  `set_cursor_past_the_end_clamps_to_len`, `cursor_line_col_counts_chars_not_bytes`,
  `control_chords_tab_and_function_keys_pass`, `esc_cancels`,
  `the_viewport_scrolls_to_keep_the_cursor_visible` (render 5×3 of a 10-line text after PgDn),
  `debug_prints_lengths_not_text`.
- **Action**: D7. `on_key` never allocates a new `String` per key beyond the edit itself.
- **Validate**: `cargo test -p htui --all-features --lib ui::text_area -- --test-threads=1`.

### Task 3: `$EDITOR` handoff, suspension, and the shell plumbing (D8–D10)
- **Files**: as tabled.
- **Tests first** — `editor.rs` unit tests: `visual_wins_over_editor`, `blank_values_fall_through`,
  `unix_falls_back_to_vi` / `windows_falls_back_to_notepad` (each `cfg`-gated),
  `the_unix_command_passes_the_file_as_dollar_one` (inspect `Command::get_program`/`get_args`);
  `cfg(unix)` round trips through scripts in a `tempfile::TempDir`:
  `a_fake_editor_that_appends_returns_edited`, `a_fake_editor_that_does_nothing_returns_unchanged`,
  `a_crlf_writing_editor_is_normalised`, `a_non_zero_exit_is_failed_and_names_the_code`,
  `a_missing_program_is_failed_and_names_visual_and_editor`,
  `the_temp_file_is_gone_after_every_outcome`; suspension over a `FakeTerminal` recording calls:
  `leave_then_enter_on_success`, `enter_runs_after_a_failed_editor`,
  `a_failed_leave_spawns_nothing_and_enters_nothing`,
  `a_dropped_future_still_enters` (poll once, drop). `state.rs` unit tests:
  `an_external_edit_from_a_tab_is_held_for_the_loop`,
  `an_external_edit_from_an_overlay_is_refused_on_the_status_line`,
  `finish_external_edit_reaches_only_the_asking_tab`.
- **Action**: D8–D10; `event_loop.rs` post-step with a comment naming the crossterm reader
  thread; `tempfile` moved to `[dependencies]` with a `# MOD-9 D9` comment.
- **Validate**: `cargo test -p htui --all-features --lib -- --test-threads=1`;
  `git diff --exit-code Cargo.lock`.

### Task 4: the worker surface (D5)
- **Files**: as tabled.
- **Tests first** (`templates.rs` unit tests over `Backend::memory(MemStore::demo())`):
  `the_read_answers_every_scope_project_in_scope_order`,
  `a_save_at_the_head_answers_templates_with_the_new_version`,
  `a_save_at_a_stale_head_answers_templates_stale_and_writes_nothing`,
  `a_refused_body_answers_failed_with_the_parse_message` (through `store_worker::serve`, so the
  `name()` arm is exercised), `request_names_match_the_name_arms` (`READ_NAME == REQUEST_NAMES[0]`,
  and each name equals `StoreRequest::name()` of a sample request).
- **Action**: D5. Comments on the or-ed arm say "the two template requests, or-ed for the reason
  the arms above are" (`store_worker.rs:1049-1051`).
- **Validate**: `cargo test -p htui --all-features --lib -- --test-threads=1`.

### Task 5: the Skills tab and the Templates view (D6, D11–D14)
- **Files**: as tabled.
- **Tests first** (`tests/templates.rs`, `Harness::demo()`, `2` then `l` to reach Templates):
  snapshots `templates__browse` (tree, `implement v1 phase`, body pane),
  `templates__edit_help` (`e` on `judge`: help lists `item_key phase task candidates`, marks
  `candidates` required, shows the judge wire note), `templates__unknown_placeholder_cursor`
  (type `{{itme}}` at a known offset, `Ctrl+S`: notice text and the highlighted cursor cell on the
  `{`), `templates__missing_item_confirm`, `templates__diff_two_versions` (save v2, `b` on v1, `.`,
  `d`), `templates__changed_elsewhere` (open the editor, write v2 to the `MemStore` directly, save:
  stale notice, draft kept); asserts: `ctrl_s_with_missing_required_puts_the_cursor_at_the_end`,
  `a_second_ctrl_s_saves_without_item_and_a_new_version_appears`,
  `judge_and_handoff_bodies_never_get_the_item_warning`, `a_refused_save_sends_no_request`,
  `saving_from_v1_while_v2_is_head_writes_v3`, `n_creates_a_new_name_at_version_one`,
  `n_refuses_an_existing_name`, `d_upper_diffs_against_the_compiled_default`,
  `external_edit_is_requested_with_the_shown_body` (`E`, then `app().take_external_edit()`),
  `an_edited_external_result_opens_the_editor_validated` (`finish_external_edit(Edited("{{bad}}"))`
  → cursor on the error, nothing sent), `a_failed_external_result_keeps_the_draft`,
  `tab_and_digits_while_editing` (`2` is typed, `Tab` leaves and `2` comes back to the draft),
  `the_strip_text_is_unchanged` (` 1 Backlog  2 Skills  3 Settings  4 Chat`).
- **Action**: D6, D11–D14; move `diff_style`; the title stays `Skills`.
- **Validate**: `INSTA_UPDATE=always cargo test -p htui --all-features --test templates`, review
  every new `.snap`, then the whole `htui` suite without the variable; no pre-existing `.snap`
  changes (`git status crates/htui/tests/snapshots crates/htui/src/snapshots`).

### Task 6: Postgres end to end (PRD hypothesis)
- **Files**: `crates/htui/tests/templates_pg.rs`.
- **Tests first**: `a_save_through_the_worker_lands_as_version_two_on_postgres` (Harness over a
  `testkit::demo_db` backend, as the existing `*_pg.rs` suites build it: edit `implement`, save, then
  `PgStore::prompt_template(project, "implement", None)` is v2 with the saved body and
  `created_by = this_user`); `the_preview_uses_the_new_head` (mirror `tests/prompt_preview.rs`'s
  setup: the preview's text contains a marker line only v2 has); `an_offline_save_is_refused_without_a_row`.
- **Validate**: `HTUI_TEST_DATABASE_URL=… cargo test -p htui --all-features --test templates_pg -- --test-threads=1`.

## Test plan

TDD per repo convention: every task's first commit is its failing tests, named above. **The first
red test of the milestone is T1's `prompt_template_append_is_a_cas_on_the_head`.**

**Store conformance** (both stores): three new cases (T1). **Postgres**: T1's concurrency case,
T6's three. **`htui` unit**: T2 (14), T3 (17), T4 (5). **`htui` harness**: T5's six snapshots and
thirteen asserts. The `$EDITOR` round trip is proven at two levels — the runner against real
scripts and the suspension against a fake terminal (T3), the view against injected outcomes (T5) —
because no test in this workspace owns a tty; the live check below covers the joint.

**Count pins that move:**

| Pin | Now | After | Where |
|---|---|---|---|
| Store `CASES` | 56 | 59 | `conformance.rs:37`; `htui-core/tests/mem_store.rs:36-37`; `htui-store/tests/pg_conformance.rs:19` |
| `READ_CASES` | unchanged | unchanged | `conformance.rs:229` |
| `StoreRequest` variants | 64 | 66 | `store_worker.rs:93-545` (counted) |
| `StoreReply` variants | 35 | 37 | `store_worker.rs:627-793` (counted) |
| `.sqlx` files | 234 | 235 | `crates/htui-store/.sqlx/` |
| Migrations / `TABLES` | unchanged | unchanged | no migration (PRD constraint) |
| Snapshots pinning the strip | 7, unchanged | 7 | `tests/integration.rs:58`; six `tests/snapshots/shell__*`/`integration__demo_shell`, one `src/snapshots/htui__testkit__tests__shell_empty.snap` |

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| **R-1** — crossterm's reader thread reads the editor's keystrokes | Certain without D9 | `drop(events)` before spawning, a fresh `EventStream` after; a comment in `event_loop.rs` names `stream.rs:44-55`; live check types in the editor |
| **R-2** — Windows invocation (`cmd /S /C`, `raw_arg`, `notepad`) is unverified here (TOOL-3) | Medium | Reviewed by eye; the resolution order is platform-independent and unit-tested; MOD-16 owns the Windows run |
| **R-3** — A GUI editor without a wait flag (`code`) returns at once and the edit is lost | High for VS Code users | `Unchanged` in under a second says "use `code --wait`"; nothing is overwritten |
| **R-4** — The body sits in a temp file while the editor runs | Low | `tempfile` creates it `0600` on Unix; removed on drop on every path; a `kill -9` of `htui` can leave one `htui-<name>-*.md` in the temp dir; template bodies are not secrets (`R-SEC` scrubbing applies at render, not here) |
| **R-5** — Wide characters misplace the cursor cell | Medium for CJK/emoji | Same limitation as `TextField` (`text_field.rs:4-7`); byte offsets from `parse` stay exact, only the drawn column is off |
| **R-6** — Existing tests that activate Skills (`tests/integration.rs:76`, `src/testkit.rs:795`) now issue a `Templates` read | Certain | Both assert only the active id or the strip; the Harness serves inline; `Harness::empty()`'s empty scope answers an empty snapshot |
| **R-7** — A save through a phase whose `template_version` is pinned does nothing visible to runs | Medium | Expected (ANA-5 §4.1 pinning); the view shows `pinned by <n> phases` only in milestone 2 if wanted — recorded, not built |
| **R-8** — Deviations the main thread must record: the store runs `parse` (D4, OQ-4); `$EDITOR` does not auto-save (OQ-3); `tempfile` becomes a runtime dependency of `htui`; `diff_style` moves to `ui::diff` | — | Listed here and in the disagreements section |

## Validation

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-features --all-targets -- -D warnings
HTUI_TEST_DATABASE_URL=postgres://postgres:htui@localhost:5432/postgres \
  cargo test --workspace --all-features -- --test-threads=1
git diff --exit-code Cargo.lock                     # no new package (D12, T3)
# sqlx-cli is NOT installed on this box (`cargo sqlx --version`: "no such command"). Once:
cargo install sqlx-cli --no-default-features --features postgres,sqlite --locked
psql postgres://postgres:htui@localhost:5432/postgres -c "CREATE DATABASE htui_sqlx;"
DATABASE_URL=postgres://postgres:htui@localhost:5432/htui_sqlx \
  cargo sqlx migrate run --source crates/htui-store/migrations
cd crates/htui-store && DATABASE_URL=postgres://postgres:htui@localhost:5432/htui_sqlx \
  cargo sqlx prepare -- --all-targets --all-features            # T1; then `prepare --check`
grep -rn 'EDITOR\|VISUAL' crates --include='*.rs' | grep -v 'src/editor.rs'   # expect only doc mentions
```

`--test-threads=1` is the repo's standing rule (the keyring fake is process-wide). The README and
`compose.yaml` name port 5439; this box's Postgres 16.13 answers on 5432, which is what the lines
above use. The Windows clippy line (README `:451`) dies in `ring` here (TOOL-3); the `cfg(windows)`
arm of D8 is reviewed by eye.

**Live check (after T5).** `cargo run -p htui -- --demo`, `2`, `l`: the demo scope's templates.
`e` on `implement`, type `{{itme}}`, `Ctrl+S`: the cursor lands on the `{`, the status names the
token. Fix it, delete every `{{item}}`, `Ctrl+S`: the confirm; `Ctrl+S` again: `implement v2`.
`b` on v1, `.`, `d`: the line diff. `EDITOR=nano`, `E`: nano opens on the body, type in it (every
key reaches nano, none reaches `htui` afterwards), save, quit: the editor shows the text, validated.
`EDITOR=false`, `E`: "`false` exited with 1; nothing was changed", the screen redraws whole.
`EDITOR=/nonexistent`: the spawn message. After each, `q` quits and the shell prompt is sane
(`stty -a` shows `icanon echo`).

## Acceptance

- [ ] Every save runs `parse`; a refused save sends no request and writes no row; the cursor lands
      on `at` for `UnknownPlaceholder`, `WrongRole`, `Unterminated`, and at the end for
      `MissingRequired`.
- [ ] A phase body without `{{item}}` saves only after a second `Ctrl+S`; judge and handoff bodies
      never warn.
- [ ] Save appends version head + 1; a save from a stale head is `TemplatesStale`, keeps the draft,
      and never overwrites; two concurrent saves at one head write one row on Postgres.
- [ ] New names start at version 1 with the role from the name; nothing deletes or updates a row.
- [ ] Any two versions, and a version against the compiled default, render as a line diff styled
      by the moved `diff_style`.
- [ ] `E` / `Ctrl+E` suspend the TUI, edit a temp file, return, validate; the terminal is restored
      on success, non-zero exit, spawn failure and a dropped future; the temp file is gone after
      each.
- [ ] Every template read and write goes through `StoreRequest` (`R-NF-3`); the view holds no store
      handle and no `UserId`.
- [ ] Strip text unchanged; no pre-existing snapshot moves; `CASES` 59 in three places; `.sqlx`
      235 and `prepare --check` clean; `Cargo.lock` unchanged; the workspace gate is green.

## Where the PRD, HANDOFF or tree disagree

1. **The PRD places `MemStore`'s seed "around `:1993`"**; the template loop is `mem.rs:2052-2062`
   (`:1988` is the doc line that names "ten templates").
2. **The Skills stub attributes the tab to MOD-12** (`skills.rs:1`, `:15`, `:53`); PRD D1 gives it
   to MOD-9. Corrected in T5 (D14).
3. **PRD Success Metrics: "`E` suspends the TUI, edits a temp file, returns, validates"** — this
   plan validates on return and saves on `Ctrl+S` (OQ-3, D11).
4. **`settings/prompt.rs:815-816` says the global table binds `ctrl-c` and `-`**; `default_global`
   binds neither (`keymap.rs:199-239`; they appear only in parse tests, `:330`, `:338`). Not this
   milestone's to fix; D13's collision check uses the table itself.
5. **`writer.rs:9-26` says `BufferedWriter` is "kept, compiling and `pub`"**; no
   `struct BufferedWriter` exists anywhere under `crates/` (grep count 0). Stale doc; not this
   milestone's.
6. **README (`:484`, `:504-509`) and `compose.yaml` use port 5439**; this box runs Postgres 16.13 on
   5432 with no container on 5439. Validation uses 5432.
7. **PRD Evidence says `similar` is "used by `htui-agent`" and cites the chat renderer** — true;
   this plan declares it in `crates/htui` too rather than calling `htui_agent::acp::fs::unified_diff`
   (D12).

---

## Verified claims

Checked at `da30927` with `grep -n`/`sed -n`, plus three probes: a scratch-crate compile probe
(`similar 3.2.0`, `tempfile 3.27.0`, `tokio 1.53.1`, built `--offline`), a SQL probe on Postgres
16.13 (`mod9_probe`, dropped), and a concurrent-transaction probe.

| Claim | Verdict | Evidence |
|---|---|---|
| `parse(role, body)` is at `template.rs:303` and documents itself as MOD-9's save gate | verified | `template.rs:294-303` |
| `TemplateError` has `at` on `UnknownPlaceholder`, `WrongRole`, `Unterminated`, none on `MissingRequired` | verified | `template.rs:377-411` |
| `omits_item()` is "MOD-9's warning" and is true for any body without `{{item}}`, whatever the role | verified | `template.rs:264-269` (no role check in the body) |
| `Placeholder::ALL`, `allowed_in`, `is_section`, `required_by`, `TemplateRole::of_name` are `pub` | verified | `template.rs:128`, `:187`, `:219`, `:233`, `:52` |
| `render::template_text` at `render.rs:260`, kept for MOD-9 | verified | `render.rs:248-262`; only test callers (`render.rs:1603`, `tests/prompt_render.rs:84`) |
| `render::normalise_newlines` is `pub` and strips CR and a BOM | verified | `render.rs:105-125` |
| `prompt_template` `UNIQUE (project_id, name, version)`, `created_by NOT NULL REFERENCES app_user` | verified | `0001_init.sql:266-276` |
| `prompt_template` has the `updated_at` trigger | verified | `0001_init.sql:577` |
| Reads inherent on PgStore/MemStore/Backend at the cited lines | verified | `pg/read.rs:1112`, `:1401`; `mem.rs:355`, `:532`; `backend.rs:348`, `:468` |
| Reads are off the traits because the tables are not mirrored | verified | `traits.rs:25-27`; `backend.rs:335-340`; `pg/read.rs:1094-1099` |
| Seed insert at `pg/write.rs:4317-4338` | verified | `pg/write.rs:4317` loop through `:4338` |
| `MemStore` seed "around `:1993`" | partial | loop at `mem.rs:2052-2062`; `:1988` is its doc line |
| `update_item_kind` at `traits.rs:544`, `mem.rs:2343`/`:4604`, `pg/write.rs:1730`, `writer.rs:546` | verified | each line opens the fn |
| Spy stores at `htui-agent/src/conformance.rs:674` and `tests/recorder.rs:354` | verified | `impl<S: WriteStore> WriteStore for UsageSpy` `:674`; `impl WriteStore for SpyStore` `:354` |
| `WriteStore` implementors are exactly PgStore, MemStore, Writer and the two spies | verified | `grep -rn 'WriteStore for'` over `crates/` |
| `CacheStore` implements `ReadStore` only | verified | `cache/read.rs:240`; no `WriteStore for CacheStore` |
| `CasOutcome` at `traits.rs:1380` | verified | `traits.rs:1380-1395` |
| `CASES` pinned at 56 in `mem_store.rs` and `pg_conformance.rs` | verified | `mem_store.rs:36-37`; `pg_conformance.rs:19` |
| `run_case` is generic over `WriteStore` | verified | `conformance.rs:105` |
| `StoreRequest` enum at `store_worker.rs:93`; 64 variants; `StoreReply` 35 | verified | counted over `:93-545` and `:627-793` |
| Catalogue handler `UpdateKind` at `catalogue.rs:153`; `cas` helper | verified | `catalogue.rs:153`, `:309-315` |
| Prompt-settings routing arm, or-ed | verified | `store_worker.rs:1049-1054` |
| `Backend::this_user` and its use by `hierarchy::serve` | verified | `backend.rs:174`; `hierarchy.rs:184`, `:230` |
| No `Backend::write*`; writes via `writer()` | verified | `backend.rs:6-9`, `:152-158` |
| `MemStore` has `require_user` for `created_by` | verified | `mem.rs:1799-1808` |
| Skills stub is 61 lines, registered at `app/mod.rs:48`, title `Skills` | verified | `wc -l`; `app/mod.rs:48`; `skills.rs:36-38` |
| `Tab` trait at `registry.rs:34-60` with one default (`focus_section`) | verified | `registry.rs:34`, `:57-59` |
| Strip text pinned by `tests/integration.rs:58` and seven snapshots | verified | `integration.rs:58`; `grep -rl '1 Backlog  2 Skills'` → six `tests/snapshots` + one `src/snapshots` |
| No snapshot renders the Skills stub body | verified | `grep -rl 'arrive with MOD-12'` → `skills.rs` only |
| `TextField` single-line, width in chars, no `unicode-width` | verified | `text_field.rs:1-7` |
| No `$EDITOR`/`$VISUAL` handling in code | verified | `grep -rn 'EDITOR\|VISUAL' crates` → none |
| Terminal setup/teardown: `terminal::init`, `event_loop::run`, `term.restore()` | verified | `lib.rs:106-108`; `terminal.rs:23`, `:70`, `:78` |
| Event loop is one `select!` over EventStream, replies, tick | verified | `event_loop.rs:27-51` |
| crossterm `EventStream` reader thread reads the tty until dropped | verified | `crossterm-0.29.0/src/event/stream.rs:44-55`, `:140-145` |
| `ratatui::try_init` sets a panic hook each call; `try_restore` = raw off + leave alt screen | verified | `ratatui-0.30.2/src/init.rs:397-403`, `:554-560` |
| `Terminal::clear` and `Interval::reset` exist in the locked versions | verified | `ratatui-core-0.1.2/src/terminal/buffers.rs:147`; `tokio-1.53.1/src/time/interval.rs:519` |
| `tokio::process::Command::raw_arg` exists (Windows) | verified | `tokio-1.53.1/src/process/mod.rs:420` |
| Global keys are `q`, Tab, BackTab, `1`–`9`, `?`, plus `w` | verified | `keymap.rs:199-239`; `app/mod.rs:63-68` |
| Settings uses `h`/`l`/`[`/`]`/arrows for sections | verified | `settings/mod.rs:331-338` |
| `similar = "3.2.0"` workspace dep, used by `htui-agent` only | verified | `Cargo.toml:75`; `htui-agent/Cargo.toml:35`; not in `crates/htui/Cargo.toml` |
| `TextDiff::from_lines(..).unified_diff().context_radius(3).header(..)` compiles and prints `---`/`+++`/`@@` | verified | compile probe output (`--- implement v1`, `@@ -1,3 +1,3 @@`, `\ No newline at end of file`) |
| `sh -c '<value> "$1"' name <file>` passes the file intact; exit codes propagate; `TempPath` drop deletes | verified | compile probe: `status=0 text="body\nadded\n"`, `failing code=Some(3)`, `removed=true` |
| Chat diff renderer `transcript.rs:552-570`, `diff_style` `:628-639` | verified | `transcript.rs:552`, `:567-570`, `:627-639` |
| `shell-words 1.1.1` and `shlex 2.0.1` in the lock, neither used by workspace code | verified | `Cargo.lock:5839`, `:5845`; `cargo tree -i` (via `agent-client-protocol`, `cc`) |
| `tempfile 3.27.0` in the lock; `htui` has it as a dev-dependency only | verified | `Cargo.lock:6327`; `crates/htui/Cargo.toml:71` |
| Settings prompt section: `busy`, single write, `CHANGED_ELSEWHERE` names `Enter` | verified | `settings/prompt.rs:255`, `:429-456`; `settings/mod.rs:103` |
| `Scope` is workspace + project ids; no project selector in the shell | verified | `model/scope.rs:10-15`; no `selected_project` in `crates/htui/src` |
| Preview picks a template's latest version by name | verified | `preview.rs:307-327` |
| Phase pins reference template versions; snapshots record the resolved version | verified | `model/kind.rs:206-207`; `model/run.rs:534`; `prompt/trim.rs:157` |
| Default bodies and `body_of` | verified | `defaults.rs:219-230`, `:237` |
| Demo projects hold the ten defaults at version 1 | verified | `fixtures.rs:677-749`; `seed.rs:268-286` (`version: 1`) |
| CAS insert shape: sequential, new-name and concurrent behaviour | verified | Postgres 16.13 probes (D3) |
| `sqlx-cli` installed | falsified | `cargo sqlx --version` → "no such command: `sqlx`"; install line added to Validation |
| Test Postgres at `localhost:5432` (brief) vs README's 5439 | verified (5432) | `psql -h localhost -p 5432` answers 16.13; 5439 refuses |
| `.sqlx` holds 234 files | verified | `ls crates/htui-store/.sqlx | wc -l` |
| `workflow-config.json` names `rust-reviewer` | verified | `.claude/workflow-config.json:2` |
