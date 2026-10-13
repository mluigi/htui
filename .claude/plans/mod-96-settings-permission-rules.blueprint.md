# Blueprint: MOD-96 — settings-file permission rules and the permission mode (T1 note, T2–T5)

**Plan**: `.claude/plans/mod-96-settings-permission-rules.plan.md` (CONFIRMED 2026-10-13: Q1 neutralise,
Q2 retire `acceptEdits` from the seed, Q3 (b) migration `0018`, Q4 stay on plan).
**Precedent**: MOD-91 (`docs/decisions/mod/mod-91.md`, `.claude/plans/mod-91-session-isolation.blueprint.md`).
No source file is written by this blueprint. Line anchors are from `hr/MOD-96` at `c20bad15`.

## Plan corrections and gaps (read first)

| # | Plan claim / omission | Finding | Consequence |
|---|---|---|---|
| F1 | Files-to-change omits `crates/htui-agent/tests/acp_conformance.rs` | Its harness drives the **seeded `claude` row** (`claude_row()`, `:37-42`) and its scripted agent answers `session/new` with no `modes` (`:218-229`). Once T4 seeds `acp.permission_mode: "default"`, D4 refuses **every** ACP conformance case | T3 adds `modes` to that scripted agent (below). Without it T4 is red across the suite |
| F2 | Omitted: `crates/htui-store/tests/migrations.rs:100-111` | `migrations_apply_on_a_clean_database` pins the applied version list `[1..=17]` and names every file | T4 appends `18` and the file name |
| F3 | T2 "update the exact argv lists (two new elements)" | Lists compose from `ISOLATION_ARGS`, so the two lever elements are automatic. **Not** automatic: D3 adds `--permission-mode default` to every isolated, tool-carrying argv whose mode is `""`. That changes `persona_flags_follow_the_pairs_and_precede_extra_args` (`cli_driver.rs:282-313`) and falsifies `an_empty_permission_mode_passes_no_flag` (`:192-203`). D2 also falsifies two pass cases of `a_settings_that_keeps_the_hooks_lever_passes` (`:1074-1075`) | Listed in T2's test table |
| F4 | D6 "It bumps `updated_at` for the cache mirror" | `updated_at` is bumped by the `BEFORE UPDATE` trigger `trg_agent_updated_at` (`0001_init.sql:574-580`; "no write path may set updated_at by hand"). The mirror does not read it: `replace_agent` (`cache/refresh.rs:474-516`) does `DELETE FROM agent` and reinserts every Postgres row on **every** refresh pass, with no cursor (`cache_migrations/0002_agent_mirror.sql` header) | **Not a blocker.** The migration must not set `updated_at`. No cache migration is needed |
| F5 | Not in the plan: a `claude-cli` session **with no prompt port** (htui hosting no MCP tools; `README.md:466-467`) | Under `acceptEdits` edits ran. Under the seeded `default`, `claude -p` denies any call it would ask about, edits included (`cli_live.rs:1746-1762` probes exactly this). The new edit rule cannot help, because no permission request reaches htui | Behaviour change for that configuration only. T5 documents it. **Maintainer FYI**, not a stop |
| F6 | Patterns table anchors | `refuse_settings_override` is `cli/mod.rs:316-355`; rule 6⅞ is `:144-148`; the refusal tests are `cli_driver.rs:998-1090`; new/restore is `acp/mod.rs:1191-1247` | Use the anchors below |
| F7 | "resume responses carry modes" (asked) | TRUE. Schema 1.7.0 `ResumeSessionResponse.modes` and `LoadSessionResponse.modes` exist. SDK 2.1.0 `RestoreRequest::response_modes` (`session.rs:239-258`) and the new route (`:146-161`) both feed `ActiveSession.modes`, and `restore_session` returns that `ActiveSession` (`RestoredSession::into_session`). `session.modes()` is therefore valid on all three routes | D4 is implementable as planned |

Verified TRUE, no action needed:
- the edit rule's wire names: `PermissionRule { #[serde(rename = "match")] matcher, answer: PermissionOptionKind, reason }` (`driver.rs:162-172`), `PermissionMatch.tool_kind: Option<String>` (`:140-159`), and `PermissionOptionKind::AllowOnce => "allow_once"` (`event.rs:110-125`);
- the CLI prompt offers `AllowOnce`/`RejectOnce` (`cli/mod.rs:1521-1526`);
- `Edit|Write|MultiEdit|NotebookEdit → edit` (`cli/claude.rs:508-517`);
- `SetSessionModeRequest::new(session_id, mode_id)` (`schema v1/agent.rs:1986-1995`), method `session/set_mode`;
- `AcpSettings`'s only struct literal outside tests is its `Default` impl (`launch.rs:323-332`);
- no `.sqlx` change: no new `query!` anywhere.

## Design decisions (blueprint level, on top of D1–D7)

- **B1 — One literal per lever, still.** `ISOLATION_ARGS` grows to 5 and stays the only CLI lever source. `isolation_args_are_the_probed_levers` and `isolation_meta_is_the_probed_options` stay the only literal pins.
- **B2 — D3 is data plus one const.** `pub const ISOLATED_PERMISSION_MODE: &str = "default"` in `cli/mod.rs` is the value an isolated row with an empty mode gets. It is substituted at rule 3, so the push order does not move.
- **B3 — D2 is a third arm, not a new function.** `refuse_settings_override` matches `--setting-sources` exactly, spaced or `=`-joined, with any value or none, and refuses before it looks at the value. The value is never read, so it can never be quoted.
- **B4 — The ACP pin is a pure decision plus a thin async step.** `fn mode_to_set(...)` is unit-testable like `restore_route`. `async fn pin_mode(...)` sends the request. A refusal travels the **owed** path that `restore_session` uses (`acp/mod.rs:1239-1243`, `run_session` `:941-953`): the child is killed and reaped before `start` answers, on all three routes.
- **B5 — The pin runs before the banner.** A refused start records no `session_started` row, and the step is named `session/set_mode` via `at_step`, so a connection-side error names the right request.
- **B6 — The migration is name-, transport- and value-keyed, and strict.** It rewrites only rows whose values still equal the old seed: missing keys and edited values are left alone, and a second run matches nothing.

---

## T1 — live probe (main thread): where its pinned CLI case goes

`crates/htui-agent/tests/isolation_live.rs`.
- The existing CLI cases are `cli_opted_in_is_the_positive_baseline` (`:551`), `cli_isolated_by_default` (`:586`) and `cli_an_overriding_settings_is_refused` (`:635`). Helpers are `cli_row` (`:386`, a synthetic row with `permission_mode: "default"`), `cli_driver` (`:422`), `cli_turn` (`:473`) and `print_cli` (`:511`).
- Insert the pinned A4/B4 case **after `cli_an_overriding_settings_is_refused` and before `acp_row` (`:667`)**, as `#[ignore] cli_settings_file_allow_is_asked_under_isolation`. It needs T2, because the shipped argv must carry `--setting-sources ""`.
  - Setup: the scratch checkout's `.claude/settings.json` = `{"permissions":{"allow":["Bash"]}}`; the prompt is the `python3 -c "open('py_out.txt','w').write('x')"` command.
  - Isolated: the stub's `permission_prompt` is called once, and `py_out.txt` is absent.
  - Opted in (`user_extensions: true`): written, unasked (the positive baseline).
  - Run it on a `cli_row` copy with `permission_mode: ""` to pin D3 live: the init envelope's `permissionMode` is `"default"`.
- ACP: `AcpTurn` already has a `modes` field (`:856`). T1 cases 2–3 use the raw `AcpClient` (`:727`), not the production driver. The production D4 pin is proven live by `chat_live` in T5.
- If T1 falsifies `settingSources: []` (case 2), the fallback `extraArgs: {"setting-sources": ""}` replaces it **only** in `isolation_meta()` and its pin test. Nothing else in this blueprint changes.

---

## T2 — CLI (D1, D2, D3)

### Files written

| File | Symbols |
|---|---|
| `crates/htui-agent/src/cli/mod.rs` | `argv` doc rule 3 (`:119`) and 6⅞ (`:144-148`); `argv` mode push (`:186-192`); `ISOLATION_ARGS` + doc (`:249-259`); new `ISOLATED_PERMISSION_MODE`, `SETTING_SOURCES`; `refuse_widening` doc (`:280-290`); `refuse_settings_override` doc + body (`:316-355`) |
| `crates/htui-agent/tests/cli_driver.rs` | the tests below; import line `:26` gains `ISOLATED_PERMISSION_MODE` |

### Constants

```rust
/// The isolation levers. MOD-91 D1 (its T1 case 1): `disableAllHooks` stops settings hooks and every
/// plugin's mods (user-scope and skills-directory); `--disable-slash-commands` drops skills and slash
/// commands. MOD-96 D1 (probe B4): `--setting-sources ""` loads no user, project or local settings
/// file, so their `permissions` rules and `defaultMode` cannot settle a call before `htui` is asked.
/// The CLI still applies flag and policy settings (`flagSettings`/`policySettings`), so `htui`'s own
/// `--settings` keeps working. Auth, `htui`'s `--mcp-config` server and `--permission-prompt-tool`
/// keep working under all three (B4). Each flag takes one value, so both pairs are spaced, as probed;
/// `""` is one argv element. Plugin agents and plugin MCP servers survived MOD-91's levers (its T1
/// case 5); MOD-96 T1 re-checks them under `--setting-sources ""`. Changing this changes
/// `isolation_args_are_the_probed_levers`, and [`HOOKS_OFF`]'s or [`SETTING_SOURCES`]'s check.
pub const ISOLATION_ARGS: [&str; 5] = [
    "--settings",
    r#"{"disableAllHooks":true}"#,
    "--disable-slash-commands",
    "--setting-sources",
    "",
];

/// The flag MOD-96 D2 refuses in an isolated row's `extra_args`, whatever its value: the CLI keeps
/// the **last** `--setting-sources` (probe E1), so a later one would read the settings files again.
const SETTING_SOURCES: &str = "--setting-sources";

/// The permission mode an isolated, tool-carrying session gets when its row names none (MOD-96
/// D3): with no settings file read, the CLI otherwise picks its own, and probe F3 saw `auto`.
pub const ISOLATED_PERMISSION_MODE: &str = "default";
```

### `argv` (signature unchanged; `user_extensions` is already its 6th parameter)

Replace the block at `:186-192` (rule 3, first push, so the order is unchanged):

```rust
        // MOD-55 review M1: a tool-less session has nothing a mode could approve, so the row's mode
        // is dropped rather than refused (an `acceptEdits` row must not make help unusable).
        // MOD-96 D3: an isolated row that names no mode says `default` rather than leave the CLI to
        // pick one; an opted-in row's settings files still supply their `defaultMode`.
        let mode = match cli.permission_mode.as_str() {
            "" if !user_extensions => ISOLATED_PERMISSION_MODE,
            mode => mode,
        };
        if !mode.is_empty() && !spec.tools.no_tools {
            push("--permission-mode", mode);
        }
```

Doc rule 3 (`:119`) becomes:

```
/// 3. the row's `--permission-mode`, as written; when the row names none, [`ISOLATED_PERMISSION_MODE`]
///    on an isolated session (MOD-96 D3: no settings file is read, and the CLI's own default was
///    `auto`, probe F3) and nothing on one that opts back in, whose settings files' `defaultMode`
///    still applies. Omitted on a tool-less session (6¾);
```

In rule 6⅞, add "MOD-96 D1" after "MOD-91 D1–D3", and end it with: "That is also why a later
`--settings` that drops `disableAllHooks`, or any later `--setting-sources`, is refused before the
spawn ([`refuse_settings_override`], MOD-91 D8, MOD-96 D2) rather than allowed to win silently;".

`refuse_widening` doc (`:284-287`): replace "because the seeded `claude-cli` row carries `acceptEdits` and refusing it would refuse help on every default install" with "because a row may carry `acceptEdits` (the seed did until MOD-96), and refusing it would refuse help on that row".

### `refuse_settings_override` (`:326`, signature unchanged)

Loop body (replaces `:337-340`; the `--settings` arm keeps its current code and message byte for byte):

```rust
        if flag == SETTING_SOURCES {
            return Err(DriverError::Transport(format!(
                "an isolated session refuses this row's `{SETTING_SOURCES}` extra argument: a later \
                 `{SETTING_SOURCES}` replaces htui's own and would let your settings files' \
                 permission rules answer before htui, so remove it, or set `mods (y/n)` to `y` in \
                 Settings > Agents"
            )));
        }
        // Exactly the flag: `--setting-sources` shares the prefix and is matched above.
        if flag != "--settings" {
            continue;
        }
```

- The message must not contain the substrings `user`, `project` or `local`; the tests assert it. Write it exactly as above.
- Doc: replace the first sentence with "Refuses an isolated session (`user_extensions` false) on a row whose `extra_args` would undo [`ISOLATION_ARGS`]: a `--settings` that does not itself keep `disableAllHooks` on (MOD-91 D8), or any `--setting-sources` at all (MOD-96 D2: the CLI keeps the last one, probe E1, so even `\"\"` is refused rather than judged)."
- `# Errors` becomes "naming the flag and the fix, never the value".

### Tests (red first)

| Test | Status | Asserts | Red before because |
|---|---|---|---|
| `isolation_args_are_the_probed_levers` (`:854`) | UPDATE | the 5-element literal above | still 3 elements |
| `isolated_session_with_no_mode_says_default` | NEW, **replaces** `an_empty_permission_mode_passes_no_flag` (`:192-203`) | `argv(&[], cli_settings("", &[]), spec, "minted-id", None, false)` has `["--permission-mode", "default"]` once, exactly after `"--include-partial-messages"` and before `"--session-id"`; also `assert_eq!(ISOLATED_PERMISSION_MODE, "default")` | no flag today |
| `opted_in_session_with_no_mode_passes_none` | NEW | same with `true` → no `--permission-mode` | guard: green before and after |
| `tool_less_session_still_drops_the_mode` | NEW | `spec.tools = tool_less()`, mode `""`, isolated → no `--permission-mode`; also with `"acceptEdits"` | guard: fails if the `no_tools` check is lost |
| `an_operators_mode_passes_unchanged` | NEW | isolated, modes `"acceptEdits"` and `"plan"` → exactly that value once, never `"default"` | guard |
| `persona_flags_follow_the_pairs_and_precede_extra_args` (`:282`) | UPDATE | insert `"--permission-mode", "default"` after `"--include-partial-messages"` in the exact list | D3 changes the list |
| `argv_opt_in_drops_the_levers` (`:936`) | UPDATE | the lever list becomes `ISOLATION_ARGS.iter().filter(\|arg\| arg.starts_with("--"))`, so `--setting-sources` is checked absent | weakens otherwise |
| `an_isolated_row_refuses_a_setting_sources_override` | NEW | for each of `["--setting-sources","user"]`, `["--setting-sources=user,project,local"]`, `["--setting-sources",""]`, `["--setting-sources="]`, `["--setting-sources"]`, `["--x","--setting-sources","project"]`, `["--settings",r#"{"disableAllHooks":true}"#,"--setting-sources","s3cret-source"]`, and `["--setting-sources","s3cret-source"]`: `Err`, message starts `agent transport error: `, contains `--setting-sources` and ``set `mods (y/n)` to `y` in Settings > Agents``, and contains none of `s3cret`, `user`, `project`, `local` | today these pass |
| `a_settings_that_keeps_the_hooks_lever_passes` (`:1063`) | UPDATE | drop the two `--setting-sources` cases (`:1074-1075`); add `&["--setting-source", "user"]` (not the flag: exact match). Doc: drop ", and `--setting-sources`, which is a different flag" | after D2 those two refuse |
| `an_opted_in_row_is_never_checked_for_setting_sources` | NEW | `refuse_settings_override(&cli_settings("acceptEdits", &["--setting-sources", "user,project,local"]), true)` is `Ok` | guard |

Automatic (compose from the const, no edit): `argv_is_the_ana4_line_in_order`, `argv_without_mcp_is_unchanged`, `without_levers`, `levers_at` (the `""` lever occurs once on every grid argv), `argv_isolates_by_default`, `the_levers_precede_extra_args`, `a_tool_less_session_is_isolated_too`, and `a_started_session_spawns_the_levers_and_marks_its_environment` (`:2516`). `RECORD` writes `printf '%s\n' "$@"`, so `""` is an empty line and `text.lines()` yields `""`. `isolation_live.rs` uses `argv` itself.

**Validate**: `cargo test -p htui-agent --all-features --test cli_driver -- --test-threads=1`.

---

## T3 — ACP (D4), after T1

### Files written

| File | Symbols |
|---|---|
| `crates/htui-agent/src/launch.rs` | `AcpSettings` (`:308-321`) + `Default` (`:323-332`) |
| `crates/htui-agent/src/acp/mod.rs` | `isolation_meta` + doc (`:1095-1120`); `session_main` new step after `:1249`; new `mode_to_set`, `pin_mode` (place after `restore_session`, `:1921`); `use` block `:32-38`; `mod tests` (`:2022`) |
| `crates/htui-agent/tests/acp_driver.rs` | `restoring_agent` (`:1374`), new opener, `isolation_meta_is_the_probed_options` (`:2150`), new section |
| `crates/htui-agent/tests/acp_conformance.rs` | `scripted_agent` `session/new` reply (`:218-229`), per F1 |
| `crates/htui-agent/tests/launch.rs` | one new round-trip test |

### `AcpSettings` field (after `model_config_id`)

```rust
    /// The ACP session mode `htui` pins after `session/new`, `session/resume` and `session/load`
    /// (MOD-96 D4): `session/set_mode` when the agent reports another current mode, and a refused
    /// start when it reports no modes or none with this id. `None`, the default, sends nothing.
    /// The row's own setting, not an isolation lever: it applies whether or not the row opts back in
    /// (`user_extensions`). `claude-agent-acp` seeds its mode from the settings files' `defaultMode`,
    /// which `_meta` cannot override (MOD-96 plan, adapter `acp-agent.js:5827, 5952`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permission_mode: Option<String>,
```

The struct is already `#[serde(default)]`. `Default` gains `permission_mode: None`.

### `isolation_meta()` (`:1110-1120`)

`"options"` gains `"settingSources": []`, placed after `"extraArgs"`. Append to the doc: "MOD-96 D4: `settingSources: []` is the CLI's `--setting-sources \"\"`, so no user, project or local settings file is read. The adapter's own `settingSources: [\"user\",\"project\",\"local\"]` precedes the spread (`acp-agent.js:5933-5935`), so ours wins. The adapter's `SettingsManager` still reads `defaultMode` from those files (`:5827`), which is why the mode is pinned separately ([`AcpSettings::permission_mode`])."

### The pin

Imports: add `SessionModeId`, `SessionModeState` and `SetSessionModeRequest` to the `agent_client_protocol::schema::v1::{…}` list at `:32`.

```rust
/// MOD-96 D4: what pinning `wanted` takes. `Ok(None)` when it is already current; `Ok(Some(id))`
/// when the agent offers it and is elsewhere; `Err(why)` when the agent reports no modes, or none
/// with that id. Fails closed: an unknown starting mode is an unknown approval path.
fn mode_to_set(
    modes: Option<&SessionModeState>,
    wanted: &str,
) -> std::result::Result<Option<SessionModeId>, String> {
    let Some(modes) = modes else {
        return Err(format!(
            "cannot pin `settings.acp.permission_mode` `{wanted}`: the agent reports no session modes"
        ));
    };
    if &*modes.current_mode_id.0 == wanted {
        return Ok(None);
    }
    if modes.available_modes.iter().any(|mode| &*mode.id.0 == wanted) {
        return Ok(Some(SessionModeId::new(wanted)));
    }
    let offered: Vec<&str> = modes.available_modes.iter().map(|mode| &*mode.id.0).collect();
    Err(format!(
        "cannot pin `settings.acp.permission_mode` `{wanted}`: the agent offers no such mode \
         (it offers {offered:?})"
    ))
}

/// Pins `wanted` on `session` (MOD-96 D4), after any of the three opening requests. The step is
/// recorded first, as the opening requests' are, so a refusal answered from the connection names
/// `session/set_mode`. ANA-4 risk 11: `block_task` only here.
async fn pin_mode(
    cx: &ConnectionTo<Agent>,
    session: &agent_client_protocol::ActiveSession<'static, Agent>,
    wanted: &str,
    ready: &Mutex<ReadyCell>,
    child: &Mutex<ChildGuard>,
) -> Result<()> {
    let Some(mode_id) = mode_to_set(session.modes(), wanted).map_err(DriverError::Transport)? else {
        return Ok(());
    };
    at_step(ready, "session/set_mode");
    let request = SetSessionModeRequest::new(session.session_id().clone(), mode_id);
    cx.send_request(request)
        .block_task()
        .await
        .map(|_| ())
        .map_err(|err| handshake_error("session/set_mode", &err, child))
}
```

In `session_main`, directly after `let session_ref = …` (`:1249`) and before step 3 (the banner):

```rust
    // 2½. MOD-96 D4: the permission mode the row pins, on every route and before the banner, so a
    //     refused start records nothing. Owed, as a failed restore is (review L-4): `run_session`
    //     kills and reaps the child before `start` answers.
    if let Some(wanted) = options.settings.acp.permission_mode.as_deref()
        && let Err(err) = pin_mode(&cx, &session, wanted, ready, child).await
    {
        ready.lock().unwrap_or_else(PoisonError::into_inner).owed = Some(err);
        return;
    }
```

Not changed: the probe handshake (`acp/handshake.rs`) and the auth flows. They are not tool-running sessions. If the adapter answers the pin with a `current_mode_update`, it reaches the turn loop as an ordinary update; it is not drained.

### Test harness changes (`acp_driver.rs`)

- `restoring_agent` (`:1374`) gains a last parameter `modes: Option<Value>`:
  - the `session/new` result becomes `{"sessionId": FRESH_SESSION}`, plus `"modes": modes` when `Some`;
  - `answered(Answer::Ok | OkAfterUpdates)` gives `{}`, plus `"modes": modes` when `Some`;
  - a new arm `"session/set_mode" => json!({"jsonrpc":"2.0","id":id,"result":{}})` (the request is already sent to `seen` at `:1392`).
- New `open_against(cwd, spec: SessionSpec, caps, resume, load, modes: Option<Value>, settings, with_child) -> Restore` holds the body of today's `open_restoring_with` (`:1536-1596`), with `spec` and `modes` passed through. `open_restoring_with` keeps its signature and delegates with `spec_resuming(cwd, PRIOR_SESSION)` and `None`, so its 9 existing callers are untouched.
- New `fn agent_modes(current: &str) -> Value` = `{"currentModeId": current, "availableModes": [{"id":"default","name":"Default"},{"id":"acceptEdits","name":"Accept Edits"},{"id":"plan","name":"Plan"}]}`.
- New `fn pinned(mode: &str) -> AgentSettings` = default settings with `acp.permission_mode = Some(mode)`.
- `mcp_agent` (`:2020`) ignores unknown methods (`_ => continue`), so it would hang on a `set_mode`. It is only ever used with `permission_mode: None`; leave it and say so in its doc.

`acp_conformance.rs` `scripted_agent` (`:218-229`): the `session/new` result gains `"modes": {"currentModeId": "default", "availableModes": [{"id": "default", "name": "Default"}]}`. This is harmless before T4 (nothing is pinned) and required after it, because the seeded `claude` row then pins `default`, already current here, so no `set_mode` is sent.

### Tests (red first)

| Test | File | Asserts | Red before because |
|---|---|---|---|
| `acp_permission_mode_is_read_and_round_trips` | `tests/launch.rs` | `{}` → `acp.permission_mode == None` and the serialised `acp` has no `permission_mode` key; `{"acp":{"permission_mode":"default"}}` → `Some("default")`, and serialise→parse is equal | field absent (compile) |
| `mode_to_set_skips_sends_and_refuses` | `acp/mod.rs` `mod tests` | via `SessionModeState::new` / `SessionMode::new`: current = wanted → `Ok(None)`; offered → `Ok(Some("default"))`; `None` → `Err` containing `default` and `no session modes`; not offered → `Err` containing `default` and the offered ids | fn absent |
| `isolation_meta_is_the_probed_options` (`:2150`) | UPDATE | literal gains `"settingSources": []` | literal differs |
| `a_pinned_mode_is_set_after_session_new` | `acp_driver.rs` | spec without resume, `agent_modes("acceptEdits")`, `pinned("default")` → `open_session` is `Ok`; exactly one `session/set_mode` with params `{"sessionId": FRESH_SESSION, "modeId": "default"}`, after `session/new` and before `session/prompt` in `methods(&seen)` | no request sent |
| `a_pinned_mode_is_set_after_a_resume_and_a_load` | `acp_driver.rs` | the route grid of `a_resume_and_a_load_carry_the_same_meta` (`:2198`) with `agent_modes("acceptEdits")` → one `set_mode` `{"sessionId": PRIOR_SESSION, "modeId": "default"}` after the restore request, and the banner still carries `PRIOR_SESSION` | same |
| `a_mode_already_current_sends_no_set_mode` | `acp_driver.rs` | new + resume routes, `agent_modes("default")`, `pinned("default")` → `Ok`, no `session/set_mode` | guard (fails if implemented as "always send") |
| `no_pinned_mode_sends_no_set_mode` | `acp_driver.rs` | `AgentSettings::default()`, `agent_modes("acceptEdits")` → no `set_mode` (today's behaviour, `agy`'s) | guard |
| `an_agent_without_the_pinned_mode_is_refused` | `acp_driver.rs` | two worlds, `modes: None` and `agent_modes` with `"default"` removed from `availableModes` (current `"acceptEdits"`); `with_child: true`: `open_session` is `Err`; `transport_message` contains `` `default` `` and `settings.acp.permission_mode`; no `session/set_mode` and no `session/prompt` in `seen`; `assert_reaped_already(pid, …)` | session opens today |

**Validate**: `cargo test -p htui-agent --all-features --test acp_driver --test acp_conformance --test launch -- --test-threads=1` and `cargo test -p htui-agent --all-features --lib acp::`.

---

## T4 — seeds and migration (after T3)

### Files written

| File | Change |
|---|---|
| `crates/htui-core/seeds/agent_claude_cli.json` | `settings.cli.permission_mode` `"acceptEdits"` → `"default"`; `settings.permission.rules` → `[EDIT_RULE]` |
| `crates/htui-core/seeds/agent_claude.json` | `settings.acp` gains `"permission_mode": "default"` after `"model_config_id": null` |
| `crates/htui-core/src/model/agent.rs` | `seed_rows_match_ana4_5_3` (`:213-310`) |
| `crates/htui-store/migrations/0018_seed_permission_mode.sql` | CREATE |
| `crates/htui-store/tests/migrations.rs` | version pin (`:100-111`) + two new tests |
| `crates/htui-agent/tests/launch.rs` | one seed-through-policy test |

`EDIT_RULE` (byte-identical in the seed JSON, the migration and the tests; no apostrophe, because of SQL quoting):

```json
{"match": {"tool_kind": "edit"}, "answer": "allow_once",
 "reason": "edits are allowed and recorded by htui (MOD-96); a persona rule can refuse them"}
```

### `seed_rows_match_ana4_5_3` re-pin

- `:299` → `assert_eq!(rows[2].settings["cli"]["permission_mode"], "default");`
- Add `assert_eq!(rows[2].settings["permission"]["rules"], serde_json::json!([EDIT_RULE]));` (the literal), with the comment "MOD-96 D5 (Q2): edits proceed through htui's policy, recorded, instead of `acceptEdits`".
- Add `assert_eq!(rows[0].settings["acp"]["permission_mode"], "default");` and `assert!(rows[1].settings["acp"].get("permission_mode").is_none(), "agy pins nothing");`.
- **Red before**: the seed still says `acceptEdits`.

### `crates/htui-agent/tests/launch.rs` — `the_seeded_rows_pin_the_mode_and_allow_edits_by_policy` (new)

- Parse `htui_core::model::agent::seed_rows(now)`'s `claude-cli` and `claude` settings as `AgentSettings`. This proves the wire names.
- `cli.permission_mode == "default"`; `permission.rules == [PermissionRule { matcher: PermissionMatch { tool_kind: Some("edit".into()), ..Default }, answer: PermissionOptionKind::AllowOnce, reason: <literal> }]`.
- `htui_agent::permission::evaluate` with the CLI's two options (`allow`/AllowOnce, `deny`/RejectOnce):
  - an `Edit` `ToolCallEvent` (kind `edit`) → `Some`, kind AllowOnce, `PolicyStage::Rule`;
  - a Bash call (kind `execute`) → `None`.
- `claude`'s `acp.permission_mode == Some("default")`; `agy`'s is `None`.
- **Red before**: the seed lacks both.

### `0018_seed_permission_mode.sql` (exact)

```sql
-- 0018_seed_permission_mode.sql - MOD-96 (plan D5, D6; Q2, Q3 = b).
-- Forward-only (R-STO-5).
--
-- The seeds now state the permission mode: claude-cli runs in 'default' and allows edits through
-- one agent rule instead of acceptEdits, and claude (ACP) pins 'default' with session/set_mode.
-- Seeds are a name-keyed insert-once (seed_if_empty_as: ON CONFLICT (name) DO NOTHING), so a box
-- seeded earlier keeps the old values. This rewrites a seeded row only while it still holds them;
-- an operator's edit, a missing key or another name is left alone, and a second run matches
-- nothing. Runs before the seed pass (apply_migrations: migrate, then bootstrap), so a fresh
-- database is a no-op here and gets the new seeds.
--
-- updated_at moves through trg_agent_updated_at (0001_init.sql); it is never set here. No cache
-- migration: the mirror's agent table is replaced whole on every refresh pass
-- (cache_migrations/0002_agent_mirror.sql, cache/refresh.rs replace_agent).

UPDATE agent
   SET settings = jsonb_set(
           jsonb_set(settings, '{cli,permission_mode}', '"default"'),
           '{permission,rules}',
           '[{"match": {"tool_kind": "edit"}, "answer": "allow_once", "reason": "edits are allowed and recorded by htui (MOD-96); a persona rule can refuse them"}]')
 WHERE name = 'claude-cli'
   AND transport = 'cli'
   AND settings #>> '{cli,permission_mode}' = 'acceptEdits'
   AND settings #> '{permission,rules}' = '[]'::jsonb;

UPDATE agent
   SET settings = jsonb_set(settings, '{acp,permission_mode}', '"default"')
 WHERE name = 'claude'
   AND transport = 'acp'
   AND jsonb_typeof(settings -> 'acp') = 'object'
   AND settings #> '{acp,permission_mode}' IS NULL;
```

### Migration tests (`crates/htui-store/tests/migrations.rs`)

- **Pin** (`:100-111`): `vec![1, …, 17, 18]`, and the message gains "and MOD-96's 0018_seed_permission_mode.sql". Red after the file lands, so update it first. TABLES and the count are unchanged.
- Shared constants: `OLD_CLI_SETTINGS` = the pre-MOD-96 `agent_claude_cli.json` `settings` verbatim (`cli.permission_mode` `"acceptEdits"`, `extra_args []`, `permission {default ask, rules [], remembered []}`, `quota cli_rate_limit_event`, `usage model_usage`). `OLD_ACP_SETTINGS` = the pre-MOD-96 `agent_claude.json` `settings` verbatim.
- Helper `insert_agent(pool, name, transport, settings: Value, at)` does `INSERT INTO agent (name, transport, launch, billing, settings, created_at, updated_at) VALUES ($1,$2,'{}'::jsonb,'subscription',$3,$4,$4)`. It uses `sqlx::query` (unchecked: no `.sqlx` entry), and `at` is a fixed 2026-01-01 timestamp.
- Helper `rows(pool) -> Vec<(String, Value, DateTime<Utc>)>` selects `name, settings, updated_at` ordered by name.
- Helper `rerun_0018(pool)`: `sqlx::raw_sql(AssertSqlSafe(MIGRATOR.iter().find(|m| m.version == 18).expect("0018 is embedded").sql.as_str().to_owned())).execute(pool)`.
- Every case: `common::bare_db()`, then `MIGRATOR.run_to(17, &db.pool)`, then the inserts, then `MIGRATOR.run(&db.pool)` (applies 18 only; no seed pass), then the asserts, then `db.drop_db()`.

| Test | World (inserted at v17) | Asserts |
|---|---|---|
| `migration_0018_rewrites_the_old_seed_rows_once` | `claude-cli` cli `OLD_CLI_SETTINGS`; `claude` acp `OLD_ACP_SETTINGS` | `claude-cli` settings == `OLD_CLI_SETTINGS` with `cli.permission_mode = "default"` and `permission.rules = [EDIT_RULE]`, every other key equal; `claude` == `OLD_ACP_SETTINGS` with `acp.permission_mode = "default"`; both `updated_at > at` (the trigger); then `rerun_0018` leaves `rows()` identical, `updated_at` included |
| `migration_0018_leaves_edited_and_other_rows_alone` | `claude-cli` with `acceptEdits` but one custom rule; `claude` with `acp.permission_mode: "plan"`; `my-cli` cli `OLD_CLI_SETTINGS`; `agy`-named acp `OLD_ACP_SETTINGS` | `rows()` before == after, byte for byte, `updated_at` included |
| (same test, second db) | `claude-cli` with `permission_mode: "plan"`, rules `[]`; `claude` acp with settings `{}` (no `acp` object) | unchanged |

**Red before**: `MIGRATOR.run` applies nothing new, so the first test's rewrite asserts fail.

### Other pins checked

- `crates/htui-agent/tests/launch.rs` `CLAUDE_SETTINGS` (`:90-102`) is a hand-written §5.3 document, not the seed: leave it.
- `.snap` files mention `acceptEdits` only in recorded CLI streams (`cli_map__{sigint,thinking,turns}`); per project memory, still run a full insta pass in the gate.
- Seed-name lists (`cache.rs:2260,2377`, `pg_criteria.rs:1583`, `mem.rs:9946`, `fixtures.rs:2203`) are names only and are unaffected.
- **Unverified (tool access was cut before I could read them):** whether any `htui` or `htui-orch` test drives the fixture `claude-cli` row's own policy over an **edit**-kind prompt and expects a park. Candidates: `agent_worker.rs` `a_policy_rule_answers_a_permission_request_and_records_it` (`:11904`), `a_chat_s_htui_tool_call_is_answered_by_policy` (`:9670`), and `engine.rs` near `:17492`. The gate below is the check. A flip there is the intended D5 behaviour: update the expectation, never the seed.

**Validate**:
- `cargo test -p htui-core`;
- `cargo test -p htui-agent --all-features --test launch --test acp_conformance -- --test-threads=1`;
- `cargo test -p htui-store --all-features -- --test-threads=1` with `HTUI_TEST_DATABASE_URL` set (otherwise the store tests print SKIP and pass);
- `cargo test -p htui -p htui-orch --all-features --no-fail-fast -- --test-threads=1`.

---

## T5 — docs (after T2–T4; plugin wording follows T1)

| File | Where | Change |
|---|---|---|
| `docs/htui-mcp.md` | §Permission prompts, `:352-355` | Replace "(the seeded `claude-cli` runs in `acceptEdits`, so file edits proceed and shell commands ask)" with "(an isolated session reads no settings file, so that is the mode htui passes: the seeded `claude-cli` runs in `default`)". After the two bullets (`:357-361`), add "**Edits.**": the seeded row's one agent rule (`tool_kind: edit` → `allow_once`) lets edits proceed unasked, as `acceptEdits` did, but each is recorded with the rule's reason, and a persona's rules, evaluated first, can refuse it. Filesystem shell commands (`mkdir`, `mv`) that `acceptEdits` approved now ask. A row still on `acceptEdits` (edited by hand) keeps the old behaviour. Add F5: with no prompt tool (htui hosts no tools) the CLI refuses whatever `default` would ask, edits included |
| `docs/htui-mcp.md` | §Isolation "Off by default", `:393-398` | New bullet: your settings files themselves (user, project and local `settings.json`): their `permissions` rules and `defaultMode`, and with them `env`, `model`, `apiKeyHelper`, `enabledPlugins`, `enabledMcpjsonServers` and `additionalDirectories` |
| same | "Still on", `:400-410` | Delete the MOD-96 bullet (`:406-408`). Add "managed (policy) settings, which htui cannot turn off". Rewrite the plugin bullet (`:402`) per T1's finding |
| same | table, `:414-417` | CLI: append `--setting-sources ""`. ACP: append `settingSources: []`, then "and `session/set_mode` when the row pins a mode" |
| same | new paragraph after the table, "**The permission mode.**" | CLI (D3): an isolated row with no `permission_mode` gets `--permission-mode default`; a row's own mode passes as written (`acceptEdits` included); a tool-less session passes none; an opted-in row with none passes none. ACP (D4): `settings.acp.permission_mode` (seeded `default` on `claude`, absent on `agy`) is pinned after `session/new`, `session/resume` and `session/load`, because the adapter starts in the settings files' `defaultMode`, which `settingSources` does not reach. An agent that reports no modes, or none with that id, is refused before the session starts, naming the mode. No value sends nothing |
| same | "Letting your extensions back in", `:426-429` | Add `--setting-sources` to the flags that disappear. Add the second way back (Q1): an inline `--settings` in the row's `extra_args` that sets `"disableAllHooks": true` plus the keys needed (`env`, `apiKeyHelper`, …); D8 allows it. Note that `mods = y` also brings back the files' permission rules and `defaultMode` |
| same | "Your own `--settings`", `:436-451` | Add a `--setting-sources` paragraph: refused as two arguments or `=`-joined, whatever the value; quote T2's message verbatim in a code block; a `--setting-sources` in the launch `args` comes before htui's, so htui's wins |
| `docs/personas.md` | `:523-525` | "A call its permission mode allows by itself (an edit under `acceptEdits`, which a row may still carry) never reaches them. The seeded `claude-cli` runs in `default` and allows edits by an agent rule instead, and a persona's rules are evaluated before it, so a persona can refuse edits there." |
| `README.md` | `:464-467` | After "its permission mode decides alone": "(the seeded `default` refuses what it would ask, edits included)" |
| `README.md` | `:470-473` | "a `--settings` … or any `--setting-sources` …" |
| `README.md` | `:477-481` | Replace "and the permission rules in your settings files and the agent's permission mode still settle calls first" with "Your settings files are not read at all, so their permission rules and `defaultMode` no longer settle a call first: `htui` passes the permission mode itself (see the docs)" |

**End to end**: `chat_live_cli` and `chat_live` (`--ignored`) on the shipped argv, `_meta` and seed (`chat_live` exercises the D4 pin against the real adapter). Then the plan's full gate. Then `bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh`.

Close-out (not a task): `HANDOFF.md` live coordinates "next migration `0019`".

---

## File sets and independence

| Task | Files |
|---|---|
| T1 | `crates/htui-agent/tests/isolation_live.rs` |
| T2 | `crates/htui-agent/src/cli/mod.rs`, `crates/htui-agent/tests/cli_driver.rs` |
| T3 | `crates/htui-agent/src/launch.rs`, `crates/htui-agent/src/acp/mod.rs`, `crates/htui-agent/tests/acp_driver.rs`, `crates/htui-agent/tests/acp_conformance.rs`, `crates/htui-agent/tests/launch.rs` |
| T4 | `crates/htui-core/seeds/agent_claude_cli.json`, `crates/htui-core/seeds/agent_claude.json`, `crates/htui-core/src/model/agent.rs`, `crates/htui-store/migrations/0018_seed_permission_mode.sql` (new), `crates/htui-store/tests/migrations.rs`, `crates/htui-agent/tests/launch.rs` |
| T5 | `docs/htui-mcp.md`, `docs/personas.md`, `README.md` |

| Pair | Common files | Verdict |
|---|---|---|
| T2 ∩ T3 | none (verified: T2 touches neither `launch.rs` nor any `acp` file; T3 touches neither `cli/mod.rs` nor `cli_driver.rs`) | **Parallel**, but both build `htui-agent`: a half-written T3 breaks T2's `cargo test` on a shared tree. Use separate worktrees (project memory) or commit-gated turns |
| T1 ∩ T2 | none; T1's pinned CLI case needs T2's `argv` | T1's CLI case after T2 |
| T3 ∩ T4 | `tests/launch.rs`; T4's seed needs T3's field and T3's `acp_conformance` modes (F1) | **Serial**: T4 after T3 |
| T2 ∩ T4 | none | Parallel-capable |
| T5 | docs only | After T1–T4 |

## Build sequence

1. T2 (now), in parallel with T1 (main thread).
2. T3 after T1 confirms `settingSources: []` and `session/set_mode`.
3. T4 after T3.
4. T1's pinned CLI case after T2.
5. T5, then the end-to-end live runs and the full gate (the plan's Validation, plus a full insta run).

Each step: write the red test(s), see them fail, implement, see them green, then commit with explicit paths.
