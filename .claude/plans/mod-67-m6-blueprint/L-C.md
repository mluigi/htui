# MOD-67 M6 blueprint — Lane L-C: key prose (D4)

Scope: every key-naming sentence outside the queue overlay (L-A), the picker and Hierarchy (L-B)
and the two shared `P` sentences (T-close X-S): **53 sentences**, each taking its key from
`Keys::label` on the stack the named key acts in, the clause dropped when the act is unbound.
Branch `mod-67-m6-l-c`, worktree `target/wt/l-c`. Source, tests and snapshots: §2.

Lane rules: never edit `keys/`, `app/`, the registries, `testkit.rs`, `tests/keys*.rs`,
`ui/overlay/queue.rs`, `ui/path_picker.rs`, `settings/hierarchy.rs`, `tests/{queue_overlay,
hierarchy}.rs`; never change `backlog::queue_sentence` (X-S's). A missing act or stack goes back to
the main thread. Format only own paths. Gate with `env -u HTUI_TEST_DATABASE_URL` and
`--test-threads=1`. Commit incrementally, one family per commit group (§6).

Complete: sections 0-7.

## 0. The rule, the stack, the shape

- **The stack is the one the named key acts in**: usually the mode the sentence is shown in (a
  stale-save notice inside an editor → the editor's stack); for a sentence that sends the user
  elsewhere, the stack of the place it names (`press e` after a refused name → the browse stack;
  the switcher's `` `N` in Settings > Hierarchy`` → `HIERARCHY_BROWSE`).
- **Shape**: a `const` that names a key becomes a `fn` of the label(s) (`Option<&str>`, from
  `keys.label(stack, act).as_deref()`), returning the old text byte for byte with the defaults
  (`Ctrl+S` is the one spelling change, PA-7: `KeyChord::label` writes `Ctrl+s`). With the act
  unbound the key clause goes (the per-site "unbound" column; these branches are reachable only
  through a file that unbinds the act).
- **Where a function has no `ctx`**, it takes `keys: &Keys` from its caller (every caller has a
  `Ctx` or already threads `&Keys`, checked per site).
- The pattern to copy: `chat/mod.rs:509-512` (`ChatEnd` doubled from its label) and
  `kinds.rs:1955-1960` (`delete_question(.., answers)`).
- A shared helper in `settings/mod.rs` (L-C owns it):
  ```rust
  /// A question's answers as its sentence ends them (MOD-67 M6, PA-6): the first chords of
  /// `confirm.yes` and `confirm.no` through `stack`, `y / n` under the defaults; an unbound side
  /// drops, both unbound is empty.
  pub(crate) fn answers(keys: &Keys, stack: Stack<'_>) -> String
  ```
  Each question constant loses its trailing ` y / n`; the render appends `" "` and `answers(..)`
  when non-empty.

## 1. The sites

Family **S** (Settings), **K** (Skills, Requirements), **B** (Backlog, Chat, overlays), plus the
shared `editor.rs`.

| # | File:line | Today (key part) | Stack | Act | Becomes | Unbound |
|---|---|---|---|---|---|---|
| 1 | `editor.rs:23` `EDITED` (callers `item_form.rs:535`, `compose.rs:491`, `library.rs:693`, `templates.rs:513`) | `edited in $EDITOR — Ctrl+S saves` | the caller's editor: `ITEM_FORM`, `compose.stack()` (`DOCS_COMPOSE`/`NOTES_COMPOSE`), `LIBRARY_EDITOR`, `TEMPLATES_EDITOR` | `FormSave` | `pub fn edited(save: Option<&str>) -> String`; `ItemForm::on_external_edit(&mut self, outcome, keys: &Keys)`, `Compose::on_external_edit(&mut self, outcome, keys: &Keys)` (callers `backlog/mod.rs` `on_external_edit`, `documents.rs`/`notes.rs` `on_external_edit` pass `ctx.keys()`); Library/Templates use their `ctx` (`_ctx` → `ctx`) | `edited in $EDITOR` |
| 2 | `settings/agents.rs:150` `QUOTA_NOTE` | `r cannot refresh it` | `AGENTS_BROWSE` | `AgentsProbe` | `fn quota_note(probe: Option<&str>) -> String` (`hint` `:1458-1466` builds it; it has `keys`) | `quota latches per chat; a probe cannot refresh it` |
| 3 | `agents.rs:203` `REPROBES` (`:1856`) | `r probes now` | `AGENTS_BROWSE` | `AgentsProbe` | `fn reprobes(probe: Option<&str>) -> String` | ` · the next chat re-probes` |
| 4 | `agents.rs:1133` login pane | `p pastes the address if the browser cannot reach it` | `AGENTS_BROWSE` | `AgentsPasteRedirect` | `format!` with the label (the pane builder takes `keys`) | `redirect: {target}` |
| 5 | `agents.rs:1919` switched-on notice | `not probed yet · r probes` | `AGENTS_BROWSE` | `AgentsProbe` | label in the `format!` (`on_reply`, has `ctx`) | `` `{name}` switched on; not probed yet`` |
| 6 | `agent_settings.rs:558` `LITERAL_LAUNCH` (Agents `:2269`; worker `agent_worker.rs:1639`) | `e edits its command` | `AGENTS_BROWSE` | `Edit` | PA-8: the const becomes `"this row's launch is literal; edit its command"` (the worker's, keyless); new `pub fn literal_launch(edit: Option<&str>) -> String` (`…; {e} edits its command`); the Agents caller of `PathsForm::open` maps `Err(LITERAL_LAUNCH)` to `literal_launch(ctx.keys().label(views::AGENTS_BROWSE, Act::Edit).as_deref())` | the const |
| 7 | `settings/boxes.rs:151` `CHANGED_ELSEWHERE_QUIRKS` (`:373`, `:393`) | `ctrl-s retries against the current row` | `BOXES_EDITOR` | `FormSave` | `fn changed_elsewhere_quirks(save: Option<&str>)`; `on_stale(&mut self, snapshot, keys: &Keys)` (caller `on_reply` has `ctx`) | `changed elsewhere since you opened it — reloaded; save again to retry against the current row` |
| 8 | `boxes.rs:156` `CHANGED_ELSEWHERE_EXECUTOR` (`:397`) | `y retries against the current row` | `BOXES_EXECUTOR` | `ConfirmYes` | `fn changed_elsewhere_executor(yes: Option<&str>)` | `changed elsewhere since you opened it — reloaded` |
| 9 | `settings/connection.rs:131` `STORED_UNREADABLE` (`:962`) | `e replaces it` | `CONNECTION_BROWSE` | `Edit` | `fn stored_unreadable(edit: Option<&str>)`; the row builder threads `keys` | `stored — not readable by this build` |
| 10 | `connection.rs:140` `REBUILD_KEYS` (`:562`) | `press Enter or R` | `CONNECTION_BROWSE` | `ConnectionActivate`, `ConnectionRebuild` | `fn rebuild_keys(keys: &Keys) -> String`: `press {a} or {r}`, one bound `press {x}` | `—` |
| 11 | `connection.rs:144` `CONFIRM_CLEAR` (`:626`) | `… y / n` | `CONNECTION_CONFIRM` | `ConfirmYes`, `ConfirmNo` | tail dropped; `answers(keys, CONNECTION_CONFIRM)` appended at `:626` | no tail |
| 12 | `connection.rs:154` `confirm_rebuild` (`:628`) | `… y / n` | `CONNECTION_CONFIRM` | same | `confirm_rebuild(answers: &str)` | no tail |
| 13 | `settings/personas.rs:148` `CHANGED_ELSEWHERE_SAVE` (`:1120`, `:1132`) | `Ctrl+S retries against the current row` | `PERSONAS_EDITOR` | `FormSave` | `pub fn changed_elsewhere_save(save: Option<&str>)` | `changed elsewhere since you opened it — reloaded; save again to retry against the current row` |
| 14 | `settings/prompt.rs:86` `NOT_A_VALUE_ROW` (`:467`) | `` `e` edits a value row`` | `PROMPT_BROWSE` | `Edit` | `fn not_a_value_row(edit: Option<&str>)` (`` `{e}` edits a value row``) | `only a value row is edited` |
| 15 | `settings/queue.rs:67` `NOT_A_VALUE_ROW` (`:342`) | same | `QUEUE_BROWSE` | `Edit` (`["e", "enter"]`, first `e`) | `pub fn not_a_value_row(edit: Option<&str>)` | same |
| 16 | `settings/qdrant.rs:28` `CONFIRM_CLEAR` (`:607`) | `… y / n` | `QDRANT_CONFIRM` | confirm | tail dropped, `answers` appended | no tail |
| 17 | `settings/secrets.rs:94` `NOTHING_TO_EDIT` (`:515`, `:570`) | `t checks the provider` | `SECRETS_BROWSE` | `SecretsCheck` | `fn nothing_to_edit(check: Option<&str>)` | `nothing to edit on this row` |
| 18 | `secrets.rs:106` `HEALTH_GUIDE` (`:1039`) | `t logs in afresh: …` | `SECRETS_BROWSE` | `SecretsCheck` | `fn health_guide(check: Option<&str>)` | `a check logs in afresh: …` (rest verbatim) |
| 19 | `secrets.rs:111` `LATCHED` (`:1037`) | `(e on Identity)` | `SECRETS_BROWSE` | `Edit` | `fn latched(edit: Option<&str>)` | the parenthesis dropped |
| 20 | `secrets.rs:113` `CONFIRM_CLEAR_URL` (`:1050`) | `… y / n` | `SECRETS_CONFIRM` | confirm | tail dropped, `answers` | no tail |
| 21 | `secrets.rs:115` `CONFIRM_CLEAR_IDENTITY` (`:1051`) | `… y / n` | `SECRETS_CONFIRM` | confirm | same | no tail |
| 22 | `secrets.rs:166` `confirm_clear_scope` (`:1053`) | `… y / n` | `SECRETS_CONFIRM` | confirm | `confirm_clear_scope(slug, answers: &str)` | no tail |
| 23 | `skills/agent_help.rs:45` `HELP_OPEN` (`:351`) | `Esc leaves it first` | by state (PA-12) | asking: the field's `Esc` (literal); waiting `SkillsHelpCancel` (`HELP_WAITING`); proposal/answered `ConfirmNo` via `Hint::All` (`HELP_PROPOSAL`) | `fn help_open(&self, keys: &Keys) -> String` (`on_key` has `ctx`) | `agent help is open — close it first` |
| 24 | `agent_help.rs:66` `MASKED` (`:394`) | `Enter again replaces the body anyway` | `HELP_PROPOSAL` | `SkillsHelpAccept` (`["enter", "y"]`) | `fn masked(accept: Option<&str>)` (`on_act` has `ctx`) | `… where a masked value was — accept again to replace the body anyway` |
| 25 | `agent_help.rs:70` `ACCEPTED` (`library.rs:1298`, `templates.rs:975`) | `Ctrl+S saves it` | the caller's editor (`LIBRARY_EDITOR`/`TEMPLATES_EDITOR`) | `FormSave` | `pub(super) fn accepted(save: Option<&str>) -> String` | `proposal accepted` |
| 26 | `skills/attach.rs:470` detach question | `y detaches, any other key keeps it` | `ATTACH_CONFIRM` | `ConfirmYes` | label in the `format!` (`ask_detach` has `ctx`) | `` detach `{name}` from {target}?`` |
| 27 | `skills/library.rs:89` `SKILL_CHANGED_ELSEWHERE` (`:1533`) | `Ctrl+S saves it over the new row` | `LIBRARY_INFO` | `FormSave` | `fn skill_changed_elsewhere(save: Option<&str>)` | `… your text is kept` |
| 28 | `library.rs:93` `BINDING_CHANGED_ELSEWHERE` (`:1545`) | `Ctrl+S saves over it` | `ATTACH_FORM` | `FormSave` | `fn binding_changed_elsewhere(save: Option<&str>)` | `… your form is kept` |
| 29 | `library.rs:171` `version_changed_elsewhere` | `Ctrl+S saves it as v{n}` | `LIBRARY_EDITOR` | `FormSave` | `version_changed_elsewhere(head, save: Option<&str>)` | `… your draft is kept and saving writes it as v{n}` |
| 30 | `library.rs:185` `landed_unread` | `r reloads` | `LIBRARY_BROWSE` | `Reload` | `landed_unread(landed, why, reload: Option<&str>)` | `{landed} — the re-read failed: {why}` |
| 31 | `library.rs:1036` `named` | `select it and press e` | `LIBRARY_BROWSE` | `Edit` | `named(&mut self, field, keys: &Keys)` (caller has `ctx`) | `` `{name}` exists `` |
| 32 | `library.rs:1403` attach landed | `later edits kept, Ctrl+S saves them` | `ATTACH_FORM` | `FormSave` | label in the `format!` | `… later edits kept` |
| 33 | `library.rs:1423` unreachable arm | `r reloads` | `LIBRARY_BROWSE` | `Reload` | label | `` `{}` landed`` |
| 34 | `library.rs:1508` reverted/applied | `Ctrl+S saves them as v{n}` | `LIBRARY_EDITOR` | `FormSave` | label | `… later edits kept` |
| 35 | `skills/templates.rs:70` `OMITS_ITEM` (`:876`) | `Ctrl+S again saves anyway` | `TEMPLATES_EDITOR` | `FormSave` | `fn omits_item(save: Option<&str>)` | `… saving again saves anyway` |
| 36 | `templates.rs:127` `template_changed_elsewhere` | `Ctrl+S saves it as v{n}` | `TEMPLATES_EDITOR` | `FormSave` | `(head, save)` | as #29 |
| 37 | `templates.rs:136` `landed_unread` | `r reloads` | `TEMPLATES_BROWSE` | `Reload` | as #30 | as #30 |
| 38 | `templates.rs:780` `create` | `select it and press e` | `TEMPLATES_BROWSE` | `Edit` | `create(&mut self, project, name, keys: &Keys)` | `` `{name}` exists`` |
| 39 | `templates.rs:1089` saved | `Ctrl+S saves them as v{n}` | `TEMPLATES_EDITOR` | `FormSave` | label | `saved v{version} — later edits kept` |
| 40 | `requirements/mod.rs:137` `landed_unread` | `r reloads` | `REQUIREMENTS_BROWSE` | `Reload` | as #30 | as #30 |
| 41 | `requirements/mod.rs:146` `mint_failed` | `look for it there before Ctrl+S` | `REQUIREMENTS_FORM` | `FormSave` | `mint_failed(why, save: Option<&str>)` | `… look for it there first` |
| 42 | `requirements/mod.rs:156` `requirement_changed_elsewhere` | `Ctrl+S saves over it` | `REQUIREMENTS_FORM` | `FormSave` | `(head, save)` | `… your text is kept` |
| 43 | `backlog/divergence.rs:47` `HINT_NO_CONFLICT` (`hint` `:325`) | `m or t continues` | `ITEM_DIVERGENCE` | `ItemMine`, `ItemTheirs` | `fn no_conflict(keys: &Keys) -> String`: `no conflicts — {m} or {t} continues`, one bound `{x} continues` | `no conflicts` |
| 44 | `divergence.rs:76` `still_behind` (`item_form.rs:705`) | `Ctrl+S compares again` | `ITEM_FORM` (the form the view returns to) | `FormSave` | `still_behind(head, save: Option<&str>)` (`on_view_key` has `keys`) | `still behind v{head}` |
| 45 | `divergence.rs:82` `rebased_on` (`item_form.rs:363`) | `Ctrl+S saves the resolution` | `ITEM_FORM` | `FormSave` | `rebased_on(head, save)`; `ItemForm::open_resolution(context, resolved, keys: &Keys)` | `rebased on v{head}` |
| 46 | `backlog/item_form.rs:1144` `mint_may_have_landed` (`item_form.rs` tests; `backlog/mod.rs:663`) | `look for it before Ctrl+S` | `ITEM_FORM` (the form stays open) | `FormSave` | `mint_may_have_landed(why, save: Option<&str>)`; `backlog/mod.rs:663` passes `ctx.keys().label(views::ITEM_FORM, Act::FormSave)` | `… look for it first` |
| 47 | `detail/compose.rs:544` `not_written` (`documents.rs:290`, notes) | `Ctrl+S tries again` | `DOCS_COMPOSE`/`NOTES_COMPOSE` (the caller's) | `FormSave` | `not_written(why, save: Option<&str>)` | `{why} — the re-read shows it was not written` |
| 48 | `compose.rs:551` `could_not_check` (`documents.rs:585`, notes) | `Ctrl+S may write it twice` | same | `FormSave` | `could_not_check(why, reread, save)` | `… could not be re-read to check` |
| 49 | `detail/documents.rs:403` `cannot_tell` (`:288`) | `so Ctrl+S may write it twice` | `DOCS_COMPOSE` | `FormSave` | `cannot_tell(why, save)` | `… hides it from the check` |
| 50 | `chat/mod.rs:499` (`hint`, armed end) | `Esc again ends this chat` | `CHAT_LIVE` | `ChatEnd` (a resolved act: confirmed, `chat/mod.rs:606-626`) | `{end} again ends this chat` from `keys.label(stack, Act::ChatEnd)` | empty (unreachable: only `ChatEnd` arms it) |
| 51 | `chat/transcript.rs:545` folded thought | `(t to unfold)` | `CHAT_LIVE` live, `CHAT_REPLAY` replay (not `CAPTURE`: the transcript stays on screen while composing) | `ChatThoughts` | `Transcript::lines(&self, height, theme, unfold: Option<&str>)`, `resume_failed_lines(&self, theme, unfold)`, `render_row(.., unfold)`; `chat/mod.rs:819` passes the `CHAT_REPLAY` label, `:843` and the resume-failed call the `CHAT_LIVE` one | `thought  {summary}` |
| 52 | `overlay/concepts_search.rs:80` `NO_MATCHES` (`:259`) | `Ctrl+R indexes this scope if the index is empty` | `CONCEPTS_QUERY` | `ConceptsReindex` | `pub fn no_matches(reindex: Option<&str>) -> String` | `no matches` |
| 53 | `overlay/workspace_switcher.rs:123` `empty_text` | `` `N` in Settings > Hierarchy creates one`` | `HIERARCHY_BROWSE` (where `N` acts) | `HierarchyNewWorkspace` | `fn empty_text(&self, keys: &Keys) -> String` (render has `ctx`) | `no workspaces — create one in Settings > Hierarchy` |

## 2. Owned files

**Source (27)**: `editor.rs`, `agent_settings.rs`, `agent_worker.rs` (the test at `:16057` only:
its literal becomes the keyless const text), `ui/overlay/{concepts_search,workspace_switcher}.rs`,
`ui/tabs/backlog/{mod,divergence,item_form}.rs`, `backlog/detail/{compose,documents,notes}.rs`,
`chat/{mod,transcript}.rs`, `requirements/mod.rs`, `settings/{mod,agents,boxes,connection,personas,
prompt,qdrant,queue,secrets}.rs`, `skills/{agent_help,attach,library,templates}.rs`.
`backlog/mod.rs` is L-C's for the `mint_may_have_landed` caller (`:663`), `on_external_edit`'s
`keys`, and its unit tests (`EDITED`, `Ctrl+S`); `notes.rs` for the compose hedges' labels and its
`EDITED` tests.

**Tests**: the in-file tests of every source file above; `tests/{backlog,personas,skills,templates,
editor_pane,box_settings,chat,concepts_search,connection,settings,secrets_settings,requirements}.rs`,
`tests/backlog_keys/l_a.rs` (one `Ctrl+S` assertion, §4).

**Snapshots** (may move; nothing else): `skills__changed_elsewhere`,
`templates__changed_elsewhere`, `templates__missing_item_confirm` (`Ctrl+S` → `Ctrl+s` in the
notice row only, PA-7). Byte-identical and must not move: `connection__{confirm,editor,empty,
stored,unreadable}`, `settings__agents_*`, `probe__agents_probed_missing`, `secrets_settings__
{configured_health_ok,health_refused}`, `chat__{chat_streamed_turn,replay_fixture_step,
replay_streamed_turn}`, every other snapshot.

## 3. What stays literal, and why (the fact table)

| Site | Text | Why it stays |
|---|---|---|
| `settings/personas.rs:139` `UNSAVED` | `Esc again discards` | the body/rules `TextArea`'s `Cancel` (`personas.rs:655-656` → `cancel_body`; `:785-797`): a widget key (ANA-26 §6.1, M3 D13). Dropped from D4. |
| `skills/library.rs:80` `UNSAVED` | `Esc again discards` | the editor `TextArea`'s `Cancel` (`library.rs:1166-1178`). Dropped from D4. |
| `skills/templates.rs:73` `UNSAVED` | `Esc again discards` | the editor `TextArea`'s `Cancel` (`templates.rs:824-835`). Dropped from D4. |
| `agent_help.rs:45` asking state | `Esc` | the request field's `Cancel` (`agent_help.rs:330`): kept literal in that state only (PA-12) |
| `settings/{agents.rs:195, personas.rs:154, mod.rs:120, hierarchy.rs:91}` | `Enter retries` | the open form's `TextField` submit |
| `connection.rs:98`, `:105`, `:126`; `qdrant.rs:19`, `:21`, `:27` | `press Enter`, `Enter replaces …`, `Enter tries …` | guides under the open DSN/URL field: the field's `Enter` |
| `concepts_search.rs:71`, `:83` | `Enter searches` | the query field's `Enter` (`HINT` `:85-88` says so) |
| `ui/tabs/registry.rs:29` `CLOSE_THE_FIELD_FIRST` | `(Esc)` | an open field's `Esc` |
| `skills/attach.rs:1102` | `(Space cycles)` | the activation field's `Space`, now `FIXED` (main §4.5) |
| every `Hint::Text("Enter …")`/`("Esc …")`, `Text("m/l priority")`, `Text("←/→ …")`, `Text("1-9 answer")`, `Text("space toggle")` | — | widget or `FIXED` keys (M3 D13, M5 D5) |
| `agent_settings.rs:37-38`, `:270`, `:280`; `hierarchy.rs:681`, `:1136`; `kinds.rs:112`, `:1769`; `personas.rs:142`, `:166` | `(y/n)`, `is y or n` | a field's typed value, not a key |
| `lib.rs:284`, `worker_cmd.rs:165`, `provision/mod.rs:881` | `press Enter` | stderr prompts before the TUI (plan D4: out) |
| Runs (`detail/runs.rs`) | — | checked: no sentence names a key; M5 D11's "run sentences" were the replay miss, label-built since M5 (`backlog/mod.rs:1108-1117` `replay_miss`) |

## 4. Existing assertions that change (the `Ctrl+S` spelling, PA-7; everything else is byte-identical)

`Ctrl+S` → `Ctrl+s` in: `tests/backlog.rs` (13), `tests/templates.rs` (7), `tests/skills.rs` (3),
`tests/personas.rs` (1), `tests/backlog_keys/l_a.rs` (1), `tests/editor_pane.rs` (2, `edited in
$EDITOR`); in-file: `item_form.rs` (8), `backlog/mod.rs` (4), `compose.rs` (3), `documents.rs` (4),
`notes.rs` (3), `divergence.rs` (1), `requirements/mod.rs` (2), `library.rs` (4), `templates.rs`
(3). `Ctrl+R` → `Ctrl+r`: `tests/concepts_search.rs` (3). `ctrl-s retries` → `Ctrl+s retries`:
`tests/box_settings.rs` (2). `agent_worker.rs:16057`: the worker's literal → `this row's launch is
literal; edit its command`. Assertions that compare against a constant that becomes a function
(`EDITED`, `ACCEPTED`, `HELP_OPEN`, `HINT_NO_CONFLICT`, `NO_MATCHES`, `CHANGED_ELSEWHERE_SAVE`)
call the function with the default label (`Some("Ctrl+s")`, …). `tests/settings.rs:4950`'s local
`LITERAL_LAUNCH` const is the view's sentence (`e edits its command`): unchanged. Counts are the
scan's (`/tmp/m6arch`); the lane gate (`--no-fail-fast`) is the authority.

## 5. Pins (red first; one rebinding test per family, plus the unit pins)

| Family | Test (file) | File it loads | Asserts |
|---|---|---|---|
| shared | `edited_names_the_save_key_in_force` (`editor.rs` unit) | — | `edited(Some("F2"))` = `edited in $EDITOR — F2 saves`; `edited(None)` = `edited in $EDITOR` |
| S | `a_rebound_settings_key_is_the_one_the_sentence_names` (`tests/settings.rs`) | `[settings.agents]\nprobe = "P"\n` | the idle note reads `quota latches per chat, P cannot refresh it`; the switched-on notice `… not probed yet · P probes` |
| S | `a_rebound_answer_ends_the_question` (`tests/connection.rs`) | `[settings.connection]\nyes = "Y"\nrebuild = "B"\n` | the Rebuild row reads `press Enter or B`; the clear question ends `Y / n`; `y` is inert there |
| S | `a_rebound_check_key_is_named_on_health` (`tests/secrets_settings.rs`) | `[settings.secrets]\ncheck = "T"\n` | Health guide starts `T logs in afresh`; `e` on Health says `… T checks the provider` |
| S | `a_rebound_save_key_is_named_after_a_stale_quirks_save` (`tests/box_settings.rs`) | `[form]\nsave = "f2"\n` | the stale notice ends `F2 retries against the current row` |
| S | `settings::answers` unit (`settings/mod.rs`) | — | defaults `y / n`; `[confirm] no = []` → `y`; both unbound → empty |
| K | `a_rebound_save_key_is_named_by_the_skills_notices` (`tests/skills.rs`) | `[form]\nsave = "f2"\n` | `$EDITOR` return: `edited in $EDITOR — F2 saves`; a stale save: `… F2 saves it as v{n}` |
| K | `a_rebound_reload_and_edit_key_are_named` (`tests/skills.rs`) | `[skills.library]\nreload = "f5"\nedit = "o"\n` | `` `x` exists — select it and press o``; a landed-unread write `… F5 reloads: …` |
| K | `a_rebound_save_key_is_named_by_the_template_warning` (`tests/templates.rs`) | `[form]\nsave = "f2"\n` | `… F2 again saves anyway` |
| K | `a_rebound_detach_answer_is_named` (`attach.rs` unit) | `[skills.attach]\nyes = "Y"\n` | the question ends `Y detaches, any other key keeps it` |
| K | `help_open_names_the_way_out_of_each_state` (`agent_help.rs` unit) | defaults; `[skills.help]\ncancel = "x"\n` | asking `Esc`; waiting `x`; proposal `n/Esc` (PA-12) |
| K | `a_rebound_save_key_is_named_by_a_requirements_hedge` (`requirements/mod.rs` unit) | `[form]\nsave = "f2"\n` | `mint_failed(.., Some("F2"))` ends `before F2`; `requirement_changed_elsewhere` `F2 saves over it` |
| B | `a_rebound_save_key_is_named_by_the_item_form` (`tests/backlog.rs`) | `[form]\nsave = "f2"\n` | `$EDITOR` return `edited in $EDITOR — F2 saves`; the divergence `Esc`: `still behind v2; F2 compares again` |
| B | `a_rebound_side_key_is_named_with_no_conflict` (`divergence.rs` unit) | `[backlog.item]\nmine = "b"\ntheirs = "w"\n` | `no conflicts — b or w continues` |
| B | `a_rebound_save_key_is_named_by_the_compose_hedges` (`documents.rs` unit) | `[form]\nsave = "f2"\n` | `not_written` `F2 tries again`; `cannot_tell` `so F2 may write it twice` |
| B | `a_rebound_end_and_thoughts_key_are_named` (`tests/chat.rs`) | `[chat]\nend = "x"\nthoughts = "T"\n` | armed: `x again ends this chat`; a folded thought `(T to unfold)`; in a replay too |
| B | `a_rebound_reindex_key_is_named_with_no_matches` (`tests/concepts_search.rs`) | `[concepts]\nreindex = "ctrl-o"\n` | `no matches — Ctrl+o indexes this scope if the index is empty` |
| B | `the_empty_switcher_names_the_hierarchy_key` (`workspace_switcher.rs` unit) | `[settings.hierarchy]\nnew_workspace = "W"\n` | `` no workspaces — `W` in Settings > Hierarchy creates one`` |

Every unbound branch gets one unit assertion beside its family's pin (the function with `None`).

## 6. Commits (each compiles, clippy both ways, green)

1. No separate pins commit: each family's pins (§5) are written first inside its own commit
   group, run red against the old text (the rebound label is missing) and shown red in the
   workflow log before the code lands, so every commit compiles and is green.
2. `feat(mod-67): M6 shared and Settings key prose` — #1-#22, `settings::answers`.
3. `feat(mod-67): M6 Skills and Requirements key prose` — #23-#42.
4. `feat(mod-67): M6 Backlog, Chat and overlay key prose` — #43-#53, `backlog/mod.rs`, `notes.rs`.
5. `test(mod-67): M6 the Ctrl+s spelling in assertions and snapshots` — §4 and the 3 snapshots
   (if they cannot move separately from 2-4, accept each snapshot in the commit whose sentence
   moved it).

## 7. Gate

```bash
rustfmt --edition 2024 --check <own .rs paths>
cargo clippy --workspace --all-targets --all-features -- -D warnings && cargo clippy --workspace -- -D warnings
cargo test -p htui --lib ui::tabs ui::overlay editor agent_settings
env -u HTUI_TEST_DATABASE_URL cargo test -p htui --all-features --test backlog --test backlog_keys \
  --test skills --test templates --test settings --test connection --test secrets_settings \
  --test box_settings --test personas --test chat --test concepts_search --test editor_pane \
  --test requirements -- --test-threads=1
env -u HTUI_TEST_DATABASE_URL cargo test -p htui --all-features --no-fail-fast -- --test-threads=1
cargo insta test -p htui --all-features --check   # after accepting exactly the 3 snapshots of §2
```
`agent_worker.rs`'s test runs in the workspace gate at T-close too (`htui` lib tests cover it here).
