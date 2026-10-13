# Blueprint: MOD-91 — session isolation (T3–T6)

**Plan**: `.claude/plans/mod-91-session-isolation.plan.md` (confirmed; D1 confirmed by T1, D5 resolved
to candidate (a) by T2, D8 added after T1 case 1a, `isolation_live.rs` moved to T6).
**Scope**: T3 (CLI default + D8), T4 (ACP `_meta`), T5 (Settings > Agents field), T6 (docs +
`isolation_live.rs`). No source file is written by this blueprint.

## Design decisions (blueprint-level, on top of D1–D8)

- **B1 — One place per lever.** The CLI levers are one `pub const ISOLATION_ARGS` in
  `cli/mod.rs`; the ACP content is one `pub fn isolation_meta()` in `acp/mod.rs`. Every test that
  lists an argv composes it from `ISOLATION_ARGS`, and exactly one test per transport pins the
  literal (`isolation_args_are_the_probed_levers`, `isolation_meta_is_the_probed_options`). A
  future lever change touches the constant/function and its one pin test, nothing else. D8's
  predicate is tied to `disableAllHooks` by name (`HOOKS_OFF`), documented beside the constant.
- **B2 — `argv` gains a sixth parameter `user_extensions: bool`.** `argv` takes `&CliSettings`,
  not `AgentSettings` (see Flags, F2), so the opt-in has to arrive as a parameter. A bool mirrors
  the stored field name (`false` = isolated), and 6 parameters stays under clippy's
  `too_many_arguments` threshold (7). Only production caller: `CliDriver::io_in`.
- **B3 — `HTUI_SESSION` is inserted in `launch_for`, not in `io`/`io_in`.** On both drivers
  `launch_for` is called only by `io`/`io_in` in production, so this is D4 to the letter in effect
  (resolved per-session launch env; not `launch::spawn`, not the probe, not `launch.env`). It makes
  the variable observable from a public seam without a spawn (`io_in` is private, so the plan's
  "`io_in` env test" is not writable as stated). It is inserted **after** `spec.env`, so a row's
  `launch.env` cannot unset it; `spec.env` can never carry it anyway (`RESERVED_PREFIX = "HTUI_"`,
  `htui-core/src/secret.rs:453`). Logins (`auth_source` → `launch_in`) do not get it: a login is
  not a session.
- **B4 — `refuse_settings_override` is a sibling of `refuse_widening`, not a branch of it.** Both
  are pure `pub fn(…) -> Result<()>`, both called in `CliDriver::start` before `self.io(…)`,
  widening first (its message is the more specific one for a tool-less turn). Unlike
  `refuse_widening`, the D8 message **never quotes the argument**: a `--settings` value is JSON
  that may carry an `env` block with secrets (`R-SEC-2`), and `--settings=<json>` is one token.
- **B5 — The form writes `settings.user_extensions` only when it changes the effective value.**
  `merge_settings` reads the stored effective value (`settings.user_extensions` as bool, anything
  else = `false`, the driver's default). If the draft agrees, the stored document is returned
  untouched (byte-for-byte, including a non-object document); otherwise the key is inserted. This
  is D6's "leaves every other settings key untouched" in the strongest form: an edit that does not
  touch the field never rewrites the JSONB, and the seven existing tests that pin `settings`
  unchanged stay valid (F7).
- **B6 — The refusal for an unmergeable settings document names the form label** `mods
  (y/n)` (not a new `SETTINGS_FIELD`), so "every refusal names a form label" holds and the focus
  lands on the field that caused it. It only fires when the draft changes the value over a stored
  document that is not a JSON object.
- **B7 — The create form defaults `mods` to `n` even when `settings_from` copies a row
  that opted in (D6 default).** The copy is the safe direction; the README says so.
- **B8 — The live test stub MCP server is a Python fixture script, not a re-exec of the test
  binary.** libtest prints its `running N tests` header to stdout, which is the MCP stdio channel
  of a re-exec'd binary; avoiding it would need a second `harness = false` target. `python3` is
  on the box, the probe's `stub_mcp.py` is proven, and the test is `#[ignore]`d.

---

## T3 — CLI default and D8

### Files written

| File | Change |
|---|---|
| `crates/htui-agent/src/launch.rs` | `AgentSettings.user_extensions`, `SESSION_VAR`, `mark_session` |
| `crates/htui-agent/src/cli/mod.rs` | `ISOLATION_ARGS`, `HOOKS_OFF`, `argv` 6th param + rule 6⅞, `refuse_settings_override`, `CliDriver::start`, `CliDriver::launch_for`, `CliDriver::io_in` |
| `crates/htui-agent/tests/cli_driver.rs` | updated + new tests below |
| `crates/htui-agent/tests/launch.rs` | default + read-back of the new key (not in the plan's list) |

### New and changed symbols

`crates/htui-agent/src/launch.rs`

```rust
// in AgentSettings, after `usage` (struct is already #[serde(default)]):
/// Whether this row lets the user's own Claude Code extensions — plugins and their mods, settings
/// hooks, skills and slash commands — into the sessions `htui` drives (MOD-91 D3). `false`, the
/// default and every pre-MOD-91 row's value, isolates: the CLI gets `cli::ISOLATION_ARGS`, an ACP
/// session gets `acp::isolation_meta()`. Top-level rather than under `cli` because it applies to
/// both transports. An unreadable settings document falls back to `default()`, the isolated side.
pub user_extensions: bool,

/// The variable every agent session `htui` starts carries, set to `1` (MOD-91 D4): how a mod the
/// user let back in (`user_extensions`) tells it is running under `htui` and can stand down.
pub const SESSION_VAR: &str = "HTUI_SESSION";

/// Inserts [`SESSION_VAR`]` = 1` into a session's resolved environment (MOD-91 D4). Called by both
/// drivers' `launch_for`, **after** `spec.env`, so a row's `launch.env` cannot unset it. Never by
/// `spawn` (which the probes use too) and never into `launch.env` (user data, `R-SEC-2`).
pub(crate) fn mark_session(env: &mut BTreeMap<String, String>)
```

`crates/htui-agent/src/cli/mod.rs`

```rust
/// The isolation levers (MOD-91 D1, probed in T1 case 1): `disableAllHooks` stops settings hooks
/// and every plugin's mods (user-scope and skills-directory), `--disable-slash-commands` drops
/// skills and slash commands. Auth, `htui`'s `--mcp-config` server and `--permission-prompt-tool`
/// keep working under both. `--settings` takes one value, so the pair is spaced, as probed.
/// Plugin agents and plugin MCP servers survive (T1 case 5, D7). Changing this changes
/// `isolation_args_are_the_probed_levers` and, if `disableAllHooks` goes, [`HOOKS_OFF`]'s check.
pub const ISOLATION_ARGS: [&str; 3] = [
    "--settings",
    r#"{"disableAllHooks":true}"#,
    "--disable-slash-commands",
];

/// The settings key D8 requires an operator's own `--settings` to set (`true`): a later
/// `--settings` **replaces** ours rather than merging (T1 case 1a).
const HOOKS_OFF: &str = "disableAllHooks";

pub fn argv(
    row_args: &[String],
    cli: &CliSettings,
    spec: &SessionSpec,
    session_id: &str,
    mcp_config: Option<&Path>,
    user_extensions: bool, // NEW, last
) -> Vec<String>

/// Refuses an isolated session (`user_extensions` false) on a row whose `extra_args` carry a
/// `--settings` that would undo [`ISOLATION_ARGS`] (MOD-91 D8): they come last on [`argv`] and the
/// CLI keeps the last `--settings` whole (T1 case 1a). Spaced or `=`-joined. Allowed only when the
/// value is inline JSON whose object sets `"disableAllHooks": true`; a path is refused, because
/// `htui` does not read the operator's file. A row that opts back in is never checked.
/// Refused rather than rewritten, for `refuse_widening`'s reason.
///
/// # Errors
/// [`DriverError::Transport`] naming the flag and both fixes, never the value (`R-SEC-2`: a
/// settings value can carry an `env` block). Never [`DriverError::Spawn`]: nothing was spawned.
pub fn refuse_settings_override(cli: &CliSettings, user_extensions: bool) -> Result<()>
```

`refuse_settings_override` body shape (mirrors `refuse_widening`'s loop): return `Ok` when
`user_extensions`; iterate `extra_args` with `peekable`; `split_once('=')` → `(flag, joined)`;
skip unless `flag == "--settings"` exactly (so `--setting-sources` never matches); value =
`joined`, else consume `args.next()`; keep going only if
`serde_json::from_str::<Value>(value).ok().and_then(|v| v.get(HOOKS_OFF).and_then(Value::as_bool)) == Some(true)`
(a non-object, a path, a dangling `--settings`, `false` all fail). Message, one sentence, e.g.:
``an isolated session refuses this row's `--settings` extra argument: a later `--settings` replaces htui's own, so it must set `"disableAllHooks": true` itself, or set `mods` to y in Settings > Agents``.

`argv` rule list — insert between 6¾ and 7 (D2):

```text
///    6⅞. the isolation levers [`ISOLATION_ARGS`] unless the row opts back in
///    (`settings.user_extensions`, MOD-91 D1–D3): after the narrowing, on **every** session —
///    tool-less help turns and resumes included — and before 7, so the operator's `extra_args`
///    still win. That is also why a later `--settings` that drops `disableAllHooks` is refused
///    before the spawn ([`refuse_settings_override`], D8) rather than allowed to win silently;
```

Code position: after the `--disallowedTools=` push, before `args.extend(cli.extra_args…)`:
`if !user_extensions { args.extend(ISOLATION_ARGS.map(ToOwned::to_owned)); }` with a
`// MOD-91 D1, D2:` comment.

`CliDriver::start` — after the existing `refuse_widening(&self.cli_settings(), &spec.tools)?;`:
`refuse_settings_override(&self.cli_settings(), self.settings.user_extensions)?;` with a
`// MOD-91 D8: before the spawn, so a refused row starts nothing.` comment.

`CliDriver::io_in` — `argv(…, config.as_ref().map(McpConfigFile::path), self.settings.user_extensions)`.

`CliDriver::launch_for` — after `resolved.env.extend(spec.env.clone());`:
`crate::launch::mark_session(&mut resolved.env);` and one doc sentence (B3). `refuse_widening`
is unchanged (plan T3).

### Tests (TDD order: write and see them fail, then implement)

`crates/htui-agent/tests/launch.rs`

1. `empty_settings_document_yields_every_documented_default` — add `assert!(!settings.user_extensions)`.
2. NEW `user_extensions_is_read_from_the_settings_document` — `{"user_extensions":true,"cli":{…}}`
   parses to `true` with `cli` intact; `{}` → `false`; a serialise → parse round trip keeps it.

`crates/htui-agent/tests/cli_driver.rs` — existing tests that change:

| Test | Why it changes | Change |
|---|---|---|
| every `argv(…)` call (≈20) and `narrowed_argv` | new parameter | pass `false` (the default, isolated) |
| `argv_is_the_ana4_line_in_order` | exact list | `ISOLATION_ARGS` before `"--x"` |
| `persona_flags_follow_the_pairs_and_precede_extra_args` | exact list | `ISOLATION_ARGS` before `"--foo"` |
| `argv_without_mcp_is_unchanged` | exact list | `ISOLATION_ARGS` before `"--x"` (doc: "flag for flag" now includes the levers) |
| `deny_kinds_edit_yields_the_four_claude_names` | `args.last()` assumed the narrowing | `without_levers(&args).last()` |
| `command_run_exposure_denies_the_heavy_bash_prefixes` | same | same |
| `no_exposure_no_bash_denials` | same | same |
| `a_default_exposure_adds_no_flag` | `args.last() == "minted-id"` | same |

New helper: `fn without_levers(args: &[String]) -> &[String]` — asserts `args.ends_with(&ISOLATION_ARGS)`
and returns the prefix. Exact lists build as `[&[…][..], &ISOLATION_ARGS, &["--x"]].concat()`.
Unchanged by content (only the extra `false`): `argv_with_mcp_has_one_joined_config_before_tools`,
`extra_args_stay_last_with_mcp`, `argv_names_the_prompt_tool_only_with_a_port`,
`no_tools_disables_every_tool_and_every_mcp_server`, `no_tools_off_adds_neither_flag`, the
prompt/resume/budget cases.

New tests:

3. `isolation_args_are_the_probed_levers` — `ISOLATION_ARGS == ["--settings", r#"{"disableAllHooks":true}"#, "--disable-slash-commands"]` (T1 case 1). The one literal pin.
4. `argv_isolates_by_default` — grid over specs (fresh, resuming, tool-less, full + MCP + prompt):
   `ISOLATION_ARGS` appears as one contiguous window, exactly once.
5. `argv_opt_in_drops_the_levers` — `user_extensions = true`: no `--settings`, no
   `--disable-slash-commands`; `argv(…, true)` equals `argv(…, false)` minus the window.
6. `the_levers_precede_extra_args` — full spec, MCP, prompt, `extra_args = ["--y"]`: the window
   starts right after `--disallowedTools=…` and ends right before `--y`.
7. `a_tool_less_session_is_isolated_too` — `tool_less()`: window after `--strict-mcp-config` and
   the deny list, before `--x` (D2).
8. `an_isolated_row_refuses_a_settings_that_drops_the_hooks_lever` — grid: spaced and joined
   `{"env":{"TOKEN":"s3cret"}}`, a path (spaced and joined), `{"disableAllHooks":false}`, `[1]`,
   a dangling `--settings`, one after an unrelated flag. Each: `Transport`, message contains
   `--settings`, `disableAllHooks` and `mods`, and never `s3cret` / the path.
9. `a_settings_that_keeps_the_hooks_lever_passes` — spaced, joined, and whitespace-laden
   `{ "disableAllHooks" : true, "env": {…} }`; no `--settings` at all; `--setting-sources user`
   (not the same flag).
10. `an_opted_in_row_is_never_checked_for_settings` — `user_extensions = true` with a path value: `Ok`.
11. `#[cfg(unix)] async an_isolated_start_with_an_overriding_settings_is_refused_before_the_spawn`
    — mirror of `a_tool_less_start_with_widening_extra_args_is_refused_before_the_spawn`: row
    command absent, `extra_args = ["--settings", "/op/settings.json"]`, expect `Transport` (a
    spawn-first driver would answer `Spawn`).
12. `#[cfg(unix)] async launch_for_marks_the_session_whatever_the_setting` — `CliDriver::from_row`
    over `scripted_row` with `settings.user_extensions` false and true, and with
    `launch.env.HTUI_SESSION = "0"`: `launch_for(&spec).env[SESSION_VAR] == "1"` every time.
13. `#[cfg(unix)] async a_started_session_spawns_the_levers_and_marks_its_environment` — new
    `SESSION_SCRIPT` (`<RECORD>` then `printf '%s' "${HTUI_SESSION-unset}" > "$HTUI_GO_FILE.session"`,
    then `TURN_SCRIPT`'s loop). Isolated row: the recorded argv (one arg per line) contains the
    window; opted-in row: it does not; both: `go.session == "1"`.

**Validate**: `cargo test -p htui-agent --all-features --test cli_driver --test launch -- --test-threads=1`;
`cargo clippy --workspace --all-features -- -D warnings`; `cargo clippy --workspace -- -D warnings`.

---

## T4 — ACP `_meta` on new, resume and load

### Files written

| File | Change |
|---|---|
| `crates/htui-agent/src/acp/mod.rs` | `isolation_meta`, `session_meta`, `session_main`, `restore_session`, `AcpDriver::launch_for`, import `Meta` |
| `crates/htui-agent/tests/acp_driver.rs` | updated + new tests below |

### New and changed symbols

```rust
use agent_client_protocol::schema::v1::Meta; // re-exported from schema v1 (`pub use ext::*`)

/// What an isolated session sends as `_meta` on `session/new`, `session/resume` and
/// `session/load` (MOD-91 D5, probed in T2 (a)): `claudeCode.options`, which `claude-agent-acp`
/// spreads over its own SDK options (`acp-agent.js:5931-5936`) and whose `extraArgs` it merges
/// (`:5974-5977`). `disableAllHooks` stops settings hooks and plugin mods and leaves the adapter's
/// own programmatic SDK hooks firing (T2: `current_mode_update` still arrives);
/// `disable-slash-commands` empties the command list. Sent from the transport, never on the
/// agent's name (`R-AGT-5`): ACP requires an agent to ignore `_meta` keys it does not know.
/// `options.settings` replaces any adapter-derived settings object (risk row,
/// `CLAUDE_MODEL_CONFIG`, which htui never sets). The CLI's counterpart is `cli::ISOLATION_ARGS`.
pub fn isolation_meta() -> Meta   // Meta::from_iter([("claudeCode".into(), json!({"options":{
                                  //   "settings":{"disableAllHooks":true},
                                  //   "extraArgs":{"disable-slash-commands":""}}}))])

/// `None` when the row opts back in (`settings.user_extensions`), else [`isolation_meta`].
pub fn session_meta(settings: &AgentSettings) -> Option<Meta>
```

`session_main` — before step 2: `let meta = session_meta(&options.settings);`. `session/new`:
`NewSessionRequest::new(…).additional_directories(…).mcp_servers(…).meta(meta.clone())`
(`.meta` takes `impl IntoOption<Meta>`, and `Option<Meta>` satisfies it). The `Some(previous)`
arm passes `meta` to `restore_session`.

`restore_session` — new last parameter `meta: Option<Meta>` (7 parameters: at clippy's threshold,
no `#[expect]` needed). Both `ResumeSessionRequest` and `LoadSessionRequest` chains gain
`.meta(meta)` (move into whichever arm runs). Doc: one sentence, "the same `_meta` as
`session/new`: the adapter's `getOrCreateSession` forwards `params._meta` (`acp-agent.js:5699-5720`,
T2)". **`restore_route` is unchanged**: it picks the request, not its parameters.

`AcpDriver::launch_for` — after `resolved.env.extend(spec.env.clone());`:
`crate::launch::mark_session(&mut resolved.env);` plus a doc sentence (B3; the adapter passes
`process.env` to the CLI it spawns, `acp-agent.js:5915-5925`).

### Tests (TDD order)

Harness changes: `session_new_params(mcp)` becomes a wrapper over new
`session_new_params_with(mcp, settings: AgentSettings)`; `open_restoring(…, session, …)` becomes a
wrapper over new `open_restoring_with(…, settings: AgentSettings, …)` (the old one builds
`AgentSettings { acp: AcpSettings { session, ..Default::default() }, ..Default::default() }`).
`options()` keeps `AgentSettings::default()`, so every existing case now runs isolated.

1. `isolation_meta_is_the_probed_options` — `Value::Object(isolation_meta())` equals the literal
   `{"claudeCode":{"options":{"settings":{"disableAllHooks":true},"extraArgs":{"disable-slash-commands":""}}}}`.
   The one literal pin.
2. `session_new_carries_the_isolation_meta_by_default` — `params["_meta"] == Value::Object(isolation_meta())`.
3. `session_new_sends_no_claude_code_meta_when_opted_in` — `user_extensions = true`:
   `params.get("_meta").and_then(|m| m.get("claudeCode")).is_none()` (robust to `_meta` absent or `null`).
4. `a_resume_and_a_load_carry_the_same_meta` — grid: caps `{resume}` → `session/resume`, caps
   `{loadSession}` only → `session/load`; × settings isolated/opted-in. Isolated: the captured
   params' `_meta` equals `isolation_meta()`; opted in: no `claudeCode`. `assert_restore_params`
   still holds.
5. `launch_for_marks_the_session_whatever_the_setting` — same setup as
   `a_usable_snapshot_is_what_the_driver_would_spawn_marker_included`, row settings with
   `user_extensions` false and true: `env[SESSION_VAR] == "1"`.
6. Changed: `a_usable_snapshot_is_what_the_driver_would_spawn_marker_included` — its exact `env`
   map gains `("HTUI_SESSION", "1")`; doc sentence: the marker is inserted after the spec's env.

No other existing ACP test asserts a whole `session/new` params object or a whole env
(`acp_conformance.rs` answers `session/new` without reading params; auth tests use `launch_in`).

**Validate**: `cargo test -p htui-agent --all-features --test acp_driver -- --test-threads=1`;
both clippy runs.

---

## T5 — Settings > Agents `mods (y/n)`

### Files written

| File | Change |
|---|---|
| `crates/htui/src/agent_settings.rs` | label, draft, parse, merge, `check_draft`, `apply_draft`, `new_agent`, `draft_of`; inline `mod tests` |
| `crates/htui/src/ui/tabs/settings/agents.rs` | `Editor::create`, `Editor::request`, `prefill`, `Editor::lines` doc |
| `crates/htui/tests/agent_settings.rs` | `draft()` literal + new cases |
| `crates/htui/tests/agent_settings_pg.rs` | literal at `:262` + one new case |
| `crates/htui/tests/settings.rs` | `LABEL_WIDTH`, `VALID_CREATE`, literal at `:3823`, refusal grid, prefill list, new cases (not in the plan's list) |
| `crates/htui/tests/snapshots/settings__agents_create_form.snap`, `settings__agents_edit_form.snap` | re-accepted |

`serve` and `store_worker.rs` are untouched: `CreateAgent`/`EditAgent` already route through
`new_agent`/`apply_draft`.

### Threading the field

`crates/htui/src/agent_settings.rs`

- `FIELD_LABELS: [&str; 9]` — append `"mods (y/n)"` (index 8). Doc: MOD-91 D6.
- `const USER_EXTENSIONS: &str = FIELD_LABELS[8];`
- `AgentDraft` — `pub user_extensions: bool` after `enabled`, doc ``settings.user_extensions``
  (MOD-91 D6): the one key of `settings` the form edits. Update the struct doc ("everything but
  `name`, `settings` …" → "… `settings` other than `user_extensions` …").
- `DraftFields` — `pub user_extensions: &'a str` (doc: `mods (y/n)`).
- NEW `pub fn parse_user_extensions(text: &str) -> Result<bool, Refusal>` —
  `yes_or_no(text).ok_or_else(|| Refusal::new(USER_EXTENSIONS, "is y or n"))`, as `parse_enabled`.
- `draft_from_fields` — `let user_extensions = parse_user_extensions(fields.user_extensions)?;`
  after `enabled` (tab order = refusal order).
- `check_draft` — `user_extensions: if draft.user_extensions { "y" } else { "n" }`.
- NEW private `fn stored_user_extensions(settings: &Value) -> bool` —
  `settings.get("user_extensions").and_then(Value::as_bool).unwrap_or(false)`; the driver's
  reading of an absent key; shared by `draft_of` and `merge_settings`.
- NEW `pub fn merge_settings(stored: &Value, user_extensions: bool) -> Result<Value, Refusal>` —
  B5/B6: equal effective value → `Ok(stored.clone())`; differs and `stored` is an object →
  clone, `insert("user_extensions", Value::Bool(user_extensions))`; differs and not an object →
  `Refusal::new(USER_EXTENSIONS, "the stored settings document is not a JSON object; edit it with SQL")`.
  Never quotes the document. The key literal `"user_extensions"` lives only here (+ the
  cross-crate test below pins it against `htui_agent::launch::AgentSettings`).
- `apply_draft` — `let settings = merge_settings(&stored.settings, draft.user_extensions)?;` and
  `settings` in the struct literal. Doc: "`settings` is kept but for `user_extensions` (MOD-91 D6)".
- `new_agent` — `let settings = merge_settings(&settings, draft.user_extensions)?;` after
  `check_draft`. Doc: "`settings` is the caller's (OQ-5), with `user_extensions` from the draft".
- `draft_of` — `user_extensions: stored_user_extensions(&agent.settings)`.
- `launch_changed` — unchanged (the opt-in is not something a probe checked; no relaunch).

`crates/htui/src/ui/tabs/settings/agents.rs`

- `Editor::create` — `texts` gains a ninth `"n"` (B7); doc "all eight fields" → "all nine".
- `Editor::edit` — unchanged in code (zips `FIELD_LABELS[1..]` with `prefill`).
- `Editor::request` — `DraftFields { …, user_extensions: self.text(FIELD_LABELS[8]) }`. The
  "unchanged closes" check (`draft == *opened`) and `relaunch` need nothing: `AgentDraft`
  equality covers the new field, and `relaunch` reads only transport/command/args.
- `prefill` — `-> [String; 8]`, appending `if draft.user_extensions { "y" } else { "n" }`; the
  `Stale` rebase picks the field up through it.
- `Editor::lines` doc and `settings.rs` `LABEL_WIDTH` — **unchanged**: the label `mods (y/n)` (10
  columns) fits today's 13-column label column (F6, resolved).

### Tests (TDD order)

Inline `crates/htui/src/agent_settings.rs` `mod tests` (where the parser tests live today):

1. Changed: `fields()` gains `user_extensions: "n"`; `draft_from_fields_parses_every_field`,
   `draft_from_fields_returns_the_first_refusal_in_field_order`,
   `draft_of_prefills_from_the_row_and_round_trips_through_the_form` gain the field in their
   literals.
2. Changed: `every_refusal_names_a_form_label` — add `parse_user_extensions("")`; still `== FIELD_LABELS`.
3. NEW `user_extensions_is_y_or_n` — y/yes/n/no as `yes_or_no` takes them; `"?"` refused on the label.
4. NEW `merge_settings_writes_only_user_extensions` — stored with `cli`, `acp`, `permission`,
   `quota`, `usage` and an unknown key; `true` → result minus the key equals stored.
5. NEW `merge_settings_leaves_the_document_untouched_when_the_value_is_unchanged` — `{}`+false,
   `{"user_extensions":true}`+true, `json!([])`+false: each `== stored`.
6. NEW `merge_settings_refuses_a_non_object_it_would_have_to_change` — `json!([])`+true → refusal
   field `mods (y/n)`.
7. NEW `apply_draft_sets_user_extensions_and_keeps_every_other_settings_key` — and setting it back
   to `n` stores `false`.
8. NEW `new_agent_writes_user_extensions_into_the_given_settings` — `true` over `{cli:…}`; `false`
   over a source with `true` stores `false`.
9. NEW `draft_of_reads_user_extensions_from_settings` — `true` read; absent, non-bool and
   non-object settings read `false`.
10. Changed: `launch_changed_sees_only_transport_command_and_args` — toggling `user_extensions` alone is `false`.
11. NEW `the_merged_settings_read_back_as_the_drivers_setting` — merge over the seeded
    `claude-cli` row's settings with `true`, parse as `htui_agent::launch::AgentSettings`:
    `user_extensions` is `true` and `cli` equals the seed's. Pins T3's key name across crates.

Unchanged under B5 (they pin `settings` unchanged with a `false` draft):
`apply_draft_keeps_name_settings_and_both_stamps`, `new_agent_has_a_blank_launch_and_the_given_settings`.

`crates/htui/tests/agent_settings.rs` (MemStore):

12. Changed: `draft()` literal gains `user_extensions: false`. `create_agent_*` settings pins
    (`:130`, `:164`, `:186`) and `edit_agent_applies_and_keeps_name_env_discovery_and_settings`
    (`:320`) stay green under B5.
13. NEW `edit_agent_writes_user_extensions_and_keeps_the_other_settings_keys` — over the demo
    `claude-cli` row: `y` → stored settings minus the key equal the old; `n` again → `false`.
14. NEW `create_agent_merges_user_extensions_into_the_source_settings`.

`crates/htui/tests/agent_settings_pg.rs`:

15. Changed: literal at `:262` gains the field; `created.settings == {}` (`:287`) stays green.
16. NEW `user_extensions_round_trips_through_postgres` — edit to `y`, re-read, `draft_of` → `true`.

`crates/htui/tests/settings.rs`:

17. Changed: `LABEL_WIDTH` 13 → 21 (and its doc), `VALID_CREATE: [&str; 9]` ending `"n"`, the
    `AgentDraft` literal at `:3823` gains `user_extensions: false`.
18. Changed: `a_bad_field_is_refused_locally_by_name_and_sends_nothing` — add `(8, "maybe", "mods (y/n)")`.
19. Changed: `n_opens_the_create_form_and_captures_input` — field reads `n`;
    `e_prefills_the_highlighted_row_without_a_name_field_and_sends_its_token` — list gains `("mods (y/n)", "n")`.
20. NEW `e_prefills_user_extensions_from_settings_and_sends_that_change` — row with
    `settings.user_extensions = true` prefills `y`; changing it to `n` sends `EditAgent` with
    `draft.user_extensions == false` and no relaunch.

Snapshots that move: `settings__agents_create_form.snap` and `settings__agents_edit_form.snap`.
**Every form line moves** (labels re-pad from 13 to 21) and a ninth line appears; the blank region
above the form shrinks by one. A grep finds the label in no other snapshot, but run the full
`htui` insta suite (`cargo insta test -p htui --features testkit --accept`) and review every
accepted file.

**Validate**: `cargo test -p htui --features testkit --test agent_settings --test settings -- --test-threads=1`;
`cargo test -p htui --features testkit --lib agent_settings -- --test-threads=1`;
`agent_settings_pg` with `HTUI_TEST_DATABASE_URL`; both clippy runs.

---

## T6 — Docs and `isolation_live.rs`

### Files written

| File | Change |
|---|---|
| `docs/htui-mcp.md` | new section + stale line fix |
| `README.md` | "Setting up agents" + "Claude Code without ACP" |
| `crates/htui-agent/tests/isolation_live.rs` | CREATE (moved here by the plan's amendment) |
| `crates/htui-agent/tests/fixtures/isolation/**` | CREATE |

### Docs

- `docs/htui-mcp.md`: new `## Isolation from your Claude Code setup` (and a contents entry):
  what is off (settings hooks, plugin mods user-scope and skills-directory, skills and slash
  commands; on ACP the whole command list), what stays (Claude Code's built-in plugins, plugin
  agents and plugin MCP servers (T1 case 5), your own MCP servers, the repo's `CLAUDE.md`), the
  levers per transport, the opt-in, `HTUI_SESSION=1` (always, both transports, for a mod you let
  back in), and D8 (an `extra_args` `--settings` must set `"disableAllHooks": true` or the row
  must opt in; a `--settings` in the row's launch `args` comes before htui's and is replaced by it).
  Fix line 34: `--mcp-config=<json>` → `--mcp-config=<path>`, a private `0600` file (MOD-79).
- `README.md`: in "Setting up agents", the `mods (y/n)` field: default `n`, only
  matters for Claude Code rows, takes effect for sessions started after the save, a new row
  copied from an opted-in row still starts at `n`. In "Claude Code without ACP", one bullet on D8.

### `crates/htui-agent/tests/isolation_live.rs` (`#[ignore = "spends model tokens…"]`)

Purpose: prove the **shipped** levers (`cli::argv` through `CliDriver`, `acp::session_meta`)
against real `claude` 2.1.294 and `claude-agent-acp` 0.75.0, under a scratch config, reproducing
T1 rows 0, 1, 1a and T2 (a). Run with
`cargo test -p htui-agent --features test-support --test isolation_live -- --ignored --nocapture --test-threads=1`.

Fixtures (`crates/htui-agent/tests/fixtures/isolation/`), copied from the `/tmp/mod91` harness:

- `marketplace/.claude-plugin/marketplace.json` (`mod91-probe`, one plugin `probe-user`);
  `marketplace/plugins/probe-user/.claude-plugin/plugin.json`, `hooks/hooks.json`
  (`{"modules":["./register.ts"]}`), `hooks/register.ts` (empty `register`).
- `skills-dir/probe-proj/` (same three files; copied to `<checkout>/.claude/skills/probe-proj/`).
- `skills/probe-skill/SKILL.md` (project skill), `user-skills/probe-uskill/SKILL.md` (copied to `<cfg>/skills/`).
- `stub_mcp.py` (B8) — the probe's stub, with the log path from `argv[1]` instead of a hardcoded
  `/tmp` path, and one added line logging `HTUI_SESSION` from its environment.

Setup (one `Scratch` struct holding the tempdirs, dropped at the end):

1. Preconditions, failing loudly (an explicitly requested ignored run must not pass vacuously):
   `claude` on `PATH`, `python3`, `git`; for the ACP cases `HTUI_LIVE_NODE` and
   `HTUI_LIVE_ACP_ADAPTER` (the adapter's `dist/index.js`).
2. Scratch `CLAUDE_CONFIG_DIR` (tempdir): read `~/.claude/.credentials.json` **read-only**, drop
   `claudeAiOauth.refreshToken`, write `0600`. Never write under `~/.claude` (ANA-29 §3.3).
   `.claude.json` = `{"hasCompletedOnboarding": true, "projects": {"<checkout>": {"hasTrustDialogAccepted": true}}}`
   — mirror whatever else the main thread's `/tmp/mod91/cfg/.claude.json` seed actually needed
   (it holds `oauthAccount`; confirm whether that was seeded or written by `claude`, see F12).
   `settings.json`: a `SessionStart` command hook `sh -c 'printf %s "${HTUI_SESSION-unset}" > <marker>'`.
3. Plugin install, with the scrubbed env below: `claude plugin marketplace add <fixtures>/marketplace`
   then `claude plugin install probe-user@mod91-probe` (spends no tokens).
4. Checkout: tempdir, `git init -q`, one commit (`-c user.name/-c user.email`), skills copied in.
5. Env scrub, as `/tmp/mod91/cc` does: the row's command is `env` and its `args` are
   `-u CLAUDECODE -u CLAUDE_CODE_SESSION_ID -u CLAUDE_CODE_CHILD_SESSION -u CLAUDE_PID -u CLAUDE_CODE_MESSAGING_SOCKET -u CLAUDE_CODE_MESSAGING_TOKEN -u CLAUDE_CODE_SESSION_ATTENDED -u CLAUDE_CODE_ENTRYPOINT -u CLAUDE_CODE_EXECPATH -u CLAUDE_EFFORT claude`
   — `argv`'s rule 1 keeps a wrapper's leading arguments first, which is exactly this. `launch.env`
   carries `CLAUDE_CONFIG_DIR`.

CLI cases — through the **full** `CliDriver::start` (factory + `ClaudeStreamAdapter`), so D8, the
levers, `HTUI_SESSION` and the MCP config file are all the shipped code. Spec: checkout cwd,
`mcp = [stub server]`, `prompt = Some(port)` from `prompt_bridge::bridge()` (the stub answers
`permission_prompt` itself), `retain_raw = true` (the banner envelope's `raw` is `system/init`),
`extra_args = ["--debug-file", <path>]`. Prompt: "Run this exact Bash command: touch <checkout>/touched. Then reply DONE."

- `cli_opted_in_is_the_positive_baseline` (row 0, `user_extensions: true`): `Done(EndTurn)`;
  both mods "loaded" in the debug file; hook marker reads `1` (a let-back-in extension sees
  `HTUI_SESSION`); probe skills in `init.skills`/`slash_commands`; stub logged a `CALL`; `touched` exists.
- `cli_isolated_by_default` (row 1): `Done(EndTurn)`; stub connected and called; `touched`
  exists; no probe mod "loaded"; no hook marker; no probe skill in `init`; stub log shows
  `HTUI_SESSION=1`.
- `cli_an_overriding_settings_is_refused` (row 1a, no tokens): `extra_args` gains
  `--settings '{"env":{}}'` → `start` answers `Transport` naming `--settings`; nothing spawned.

ACP cases — `AcpDriver::launch_for` (shipped `HTUI_SESSION`) over a row whose command is the same
`env -u …` wrapper around `HTUI_LIVE_NODE <adapter>` with `CLAUDE_CONFIG_DIR` and
`CLAUDE_CODE_EXECUTABLE` in `launch.env`; then `launch::spawn` + `ChildIo::from_spawned` and a
small line-JSON-RPC client in the test (mirroring `/tmp/mod91/acp_client.py`): `initialize`,
`session/new` with the stub as a stdio `mcpServers` entry and `_meta` =
`acp::session_meta(&settings)` with `claudeCode.options.extraArgs["debug-file"]` merged in (the
one thing the driver path cannot inject; the driver's own `_meta` plumbing is T4's unit tests),
prompt "call EnterPlanMode, then reply DONE", every permission request allowed.

- `acp_opted_in_is_the_positive_baseline`: mods loaded, hook fired, probe commands listed.
- `acp_isolated_by_default` (T2 (a)): `end_turn`; stub saw `initialize` + `tools/list`; no
  probe mod loaded; no hook; zero `available_commands`; a `current_mode_update` with `plan`
  arrives (the adapter's SDK hooks still fire).
- Optional, recommended: `acp_isolated_on_resume` — a second adapter process `session/resume`s
  the first id with the same `_meta`; its debug file shows no probe mod.

Print every observation (as `cli_live.rs` does) before asserting, so a failing run is a record.

**End-to-end** (plan T6): re-run `chat_live_cli` (`--ignored`) and `chat_live` on the shipped
argv/`_meta`; `chat_live` needs Node and the adapter (F13). Then the full gate, serial.

---

## Independence, from the actual file sets

| Task | Writes |
|---|---|
| T3 | `htui-agent/src/launch.rs`, `htui-agent/src/cli/mod.rs`, `htui-agent/tests/cli_driver.rs`, `htui-agent/tests/launch.rs` |
| T4 | `htui-agent/src/acp/mod.rs`, `htui-agent/tests/acp_driver.rs` |
| T5 | `htui/src/agent_settings.rs`, `htui/src/ui/tabs/settings/agents.rs`, `htui/tests/{agent_settings,agent_settings_pg,settings}.rs`, two `.snap` files |
| T6 | `docs/htui-mcp.md`, `README.md`, `htui-agent/tests/isolation_live.rs`, `htui-agent/tests/fixtures/isolation/**` |

- **T3 → T4: serial, confirmed.** T4 reads `AgentSettings.user_extensions`, `SESSION_VAR` and
  `mark_session` from `launch.rs`.
- **T3 → T5: serial, confirmed, and stronger than the plan says.** Test 11 compiles against
  `AgentSettings.user_extensions`, so T5 does not build before T3 lands.
- **T4 ∥ T5: confirmed on file sets (empty intersection), with one hidden coupling.** `htui`
  depends on `htui-agent`, so in a **shared tree** a half-edited `acp/mod.rs` breaks T5's
  `cargo test -p htui` build. Run them in **separate worktrees** (branched after T3's commit), or
  let T5's validate gate wait for T4 to be green. No other coupling: no `.sqlx`, no seed, no
  migration, no shared snapshot (T4 touches no `htui` snapshot; no `htui` test asserts ACP
  request params).
- **T6 after all, and not "docs only".** It now creates `isolation_live.rs` (needs T3's
  `CliDriver` + D8 and T4's `session_meta`) and the fixtures.

## Flags — where the plan and the code disagree

- **F1** Claim 9 cites `agent-client-protocol-schema` 1.7.0 `v1/agent.rs:114` as
  `NewSessionRequest::meta`; `:114` is `InitializeRequest::meta`. The builders are `:856`
  (`NewSessionRequest`), `:1022` (`LoadSessionRequest`) and `:1354` (`ResumeSessionRequest`). The
  claim holds; the citation is wrong.
- **F2** "Implement the levers in `argv`" understates it: `argv` takes `&CliSettings`, which has
  no `user_extensions`, so its signature changes (B2). `verify_change`: one production caller
  (`io_in`), about 20 test calls, none outside `cli_driver.rs` (`cli_live.rs` builds its own argv).
- **F3** The plan's T3 "`io_in` env test" cannot be written: `io_in` is private. B3 moves the
  insert to `launch_for`, which only `io`/`io_in` call in production.
- **F4** Four more argv tests break than the three exact-list tests the plan names. Each assumes
  the narrowing is the last argument: `deny_kinds_edit_yields_the_four_claude_names`,
  `command_run_exposure_denies_the_heavy_bash_prefixes`, `no_exposure_no_bash_denials` and
  `a_default_exposure_adds_no_flag`.
- **F5** The plan's file table misses three files: `crates/htui-agent/tests/launch.rs` (T3),
  `crates/htui/tests/settings.rs` (T5: `LABEL_WIDTH`, `VALID_CREATE`, a literal, the refusal grid)
  and the inline `mod tests` of `crates/htui/src/agent_settings.rs`. The last one holds most of
  the parser and merge tests, including `every_refusal_names_a_form_label`.
- **F6** The label `mods (y/n)` is 21 columns wide against today's 13. So both form
  snapshots change on every line, not by "a line". `Editor::lines`' doc (13 wide, 83 columns of
  text) and `settings.rs` `LABEL_WIDTH` both change, and the form's text room drops from 83 to 75
  columns. **Resolved (maintainer, 2026-10-08): the label is `mods (y/n)`.** It is 10 columns, fits the
  existing 13-column label column, and only the added line moves in the two form snapshots. The
  README says the field covers mods, settings hooks and skills.
- **F7** "`apply_draft` sets `settings.user_extensions`" collides with seven existing pins of
  `settings` being unchanged:
  - `tests/agent_settings.rs:130`, `:164`, `:186` and `:320`;
  - `agent_settings_pg.rs:287`;
  - the unit tests `apply_draft_keeps_name_settings_and_both_stamps` and
    `new_agent_has_a_blank_launch_and_the_given_settings`.

  Writing the key on every save would break all seven and rewrite every row's JSONB on unrelated
  edits. B5 (write only on change) keeps them green.
- **F8** The independence row "T6 ∩ others: docs only" is stale since the amendment. T6 now writes
  a test and fixtures under `crates/htui-agent/tests/`.
- **F9** "T4 ∩ T5: none → Parallel" holds for files, not for builds, because `htui` depends on
  `htui-agent`. See Independence.
- **F10** D8 says it "follows the shape of `refuse_widening`", but `refuse_widening`'s message
  quotes the whole argument (`` `{arg}` ``). For `--settings=<json>`, that would print a settings
  value, which can carry an `env` block. The D8 message names the flag only (B4).
- **F11** D8 covers `extra_args` only. A `--settings` in the row's `launch.args` comes **before**
  the levers (rule 1), so ours wins and the operator's file is silently replaced. That is the safe
  direction, so it is documented (T6) rather than refused.
- **F12** The probe's scratch `.claude.json` holds `oauthAccount`, `userID` and other keys besides
  `hasTrustDialogAccepted`. The plan says to seed only trust. The main thread should confirm
  whether `oauthAccount` had to be copied from the real file for auth, then mirror that in
  `isolation_live.rs`.
- **F13** T6's `chat_live` (ACP) and the ACP live cases need Node ≥ 22 and the adapter, which the
  sandbox has only under `/tmp` (claim 17). `/tmp` does not survive a sandbox restart (memory), so
  T6 must re-check `/tmp/node22` and `/tmp/acp-run` first.
- **F14** `agy` now receives `_meta.claudeCode` on every ACP session. ACP requires an agent to
  ignore unknown keys, but nothing proves `agy_acp_server` does. If `agy` is installed when T6
  runs, run `agy_live`/`chat_live_agy`; otherwise record it as unverified.

## Build sequence

1. **T3** (one implementer):
   1. tests in `tests/launch.rs` and `tests/cli_driver.rs`, in red;
   2. `launch.rs`: the field, `SESSION_VAR` and `mark_session`;
   3. `cli/mod.rs`: `ISOLATION_ARGS`, the `argv` parameter and rule 6⅞, `refuse_settings_override`,
      then `start`, `launch_for` and `io_in`;
   4. green, both clippy runs, then commit.
2. **T4** and **T5**, in separate worktrees branched from T3's commit:
   - **T4**: tests in `acp_driver.rs`, then `acp/mod.rs`, then green and commit.
   - **T5**: the inline tests and `tests/*.rs` cases, then `agent_settings.rs`, then `agents.rs`.
     Then run the insta suite in full, review it, and commit.
3. Merge T4 and T5, then run the workspace gate serially.
4. **T6**:
   1. the fixtures, then `isolation_live.rs`, then a live run (record the output in the plan's
      Probe results);
   2. the docs;
   3. `chat_live_cli` and `chat_live`;
   4. the full gate (fmt, both clippy runs, `--no-fail-fast --test-threads=1`, grep for `SIGABRT`
      and stack overflows), then the validator.
