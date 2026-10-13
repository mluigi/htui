# Plan: MOD-96 — settings-file permission rules and the permission mode in htui-driven sessions

**Status: DONE 2026-10-13. CONFIRMED 2026-10-13 (all defaults); Q2 reversed after review R1 (see the amendment); write-up `docs/decisions/mod/mod-96.md`.**

**Source**: `HANDOFF.md` MOD-96 (from MOD-91 review M1, `docs/decisions/mod/mod-91.md`).
**Routing**: plan path (C3 only; accepted by the maintainer 2026-10-13). **Re-routing note:** the probe
widened the breadth to about 15 files (C4), which by the table would make it PRD. Recommendation:
stay on plan, because the probe has already answered the open questions a PRD would have asked.
**Requirements**: `R-TUI-6`, `R-HIS-1`, `R-AGT-4` (the relay and its record are the approval path);
`R-AGT-5` (nothing branches on an agent's name).
**Complexity**: Medium. One live ACP probe gates T3. There is a migration only if Q3 = (b).
**Staffing**: every agent inherits the session model. The reviewer is `rust-reviewer`
(`.claude/workflow-config.json`).


**Amended 2026-10-13 after review R1 (finding 1, HIGH): Q2 reversed by the maintainer (option 1).** `acceptEdits`
allows an edit only inside the working directories, and only after the CLI's safety checks (2.1.296
bundle: `let H=Kg(path,…); if(mode==="acceptEdits"&&H) return allow`; the safety checks return `ask`
first). The planned `{"tool_kind":"edit"} → allow_once` rule had no path condition, so out-of-tree
and safety-checked edits (`~/.bashrc`, `~/.claude/settings.json`, `.git/hooks/*`) would have been
allowed unasked. The `claude-cli` seed therefore **keeps `acceptEdits` and `rules: []`**:
- D5's CLI half and Q2 are withdrawn;
- `0018` pins only the `claude` ACP row;
- F5 no longer applies.

D1–D4, and the ACP half of D5 and D6, stand.

## Summary

MOD-91's levers stop the user's mods, hooks and skills, but three things still settle a tool call
before `mcp__htui__permission_prompt` (CLI) or `session/request_permission` (ACP) is asked:

- `permissions.allow` rules in the user, project and local `settings.json`;
- the CLI row's `acceptEdits`;
- **(new, found while planning)** the ACP session's starting mode. `claude-agent-acp` seeds it from
  `permissions.defaultMode` in the settings files, and this box's user `settings.json` says `auto`.

A pre-plan live probe (§Probe results) shows that `--setting-sources ""` removes every
settings-file rule and `defaultMode`, while auth, htui's MCP server, `--permission-prompt-tool` and
htui's own `--settings` keep working. It also shows that a CLI session with no explicit mode can
start in `auto`.

The plan:

- neutralise the settings files in isolated sessions, on both transports;
- always state the permission mode explicitly;
- let htui pin the ACP mode;
- replace the seed's `acceptEdits` with an htui policy rule that allows edits **and records them**.

## Probe results (run 2026-10-13, before the plan; `claude` 2.1.296, Haiku)

Setup:

- scratch `CLAUDE_CONFIG_DIR` (credentials without the refresh token; the shared `~/.claude` was
  untouched);
- a trusted scratch checkout whose `.claude/settings.json` is `{"permissions":{"allow":["Bash"]}}`,
  with a scratch user `settings.json` that allows `Write`;
- a stub stdio MCP server `htui` whose `permission_prompt` logs the call and answers deny;
- every run carries MOD-91's `ISOLATION_ARGS` and `--permission-prompt-tool mcp__htui__permission_prompt`.

The Bash probe command was `python3 -c "open('py_out.txt','w').write('x')"`: `echo` is read-only to
the CLI and is never asked about (cases A2, A3, B3 proved nothing).

| Case | Extra args | Asked htui? | Effect |
|---|---|---|---|
| A4 | `--permission-mode default` | **no** | file written, so the project `allow` settles the call. **The gap is real** |
| B4 | A4 + `--setting-sources ""` | yes, denied | nothing written. Auth OK, `htui` MCP `connected` |
| D4 | `--settings '{"disableAllHooks":true,"permissions":{"ask":["Bash"]}}'` | yes, denied | `ask` beats a file `allow` (the alternative lever, rejected in D1) |
| A5 | `--setting-sources ""` + `--permission-mode acceptEdits`, Write | **no** | the mode settles edits by itself |
| B5 | `--setting-sources ""` + `--permission-mode default`, Write | yes, denied | the user-scope `allow: ["Write"]` is gone too |
| E1 | `--setting-sources ""` … `--setting-sources user,project,local` | **no** | **the last `--setting-sources` wins** (D2) |
| E2 | no mode; user `defaultMode: acceptEdits` | no | init `permissionMode: acceptEdits` |
| F3 | `--setting-sources ""`, no mode, parent-session env removed | **no** | init `permissionMode: **auto**` (D3) |

From the bundle (`cli.js` inside the 2.1.296 binary):

- `--setting-sources ""` parses to `[]`, and `flagSettings` and `policySettings` are always added
  back. `--restricted` sets the same empty list.
- `allowManagedPermissionRulesOnly` is read from `policySettings` only, so htui cannot set it.

From the adapter (`claude-agent-acp` 0.75.0, `dist/acp-agent.js`):

- `settingSources: ["user","project","local"]` is followed by `...userProvidedOptions` (`:5933-5935`),
  so `_meta.claudeCode.options.settingSources` overrides it.
- `permissionMode` is set **after** the spread (`:5952`), from
  `resolvePermissionMode(settingsManager.getSettings().permissions?.defaultMode)` (`:5827`).
  `SettingsManager` reads the user, project, local and managed files itself (`settings.js:90`) and
  filters only escalation from repo-committed sources. So `_meta` cannot set the mode, but
  `session/set_mode` can (`setSessionMode`, `:4738`).

### T1 — ACP probe results (run 2026-10-13 after CONFIRM; `claude-agent-acp` 0.75.0 on Node 22.23.3, `claude` 2.1.296)

Each case uses its own scratch config dir and a trusted checkout with project `allow: ["Bash"]`. The
client advertises no `fs`/terminal capabilities and rejects every `session/request_permission`.
"MOD-91 `_meta`" is today's `isolation_meta()`; "+ `settingSources`" adds `"settingSources": []` to
`options`.

| Case | User `settings.json` | `_meta` | `set_mode` | `currentModeId` at `session/new` | Asked? | Result |
|---|---|---|---|---|---|---|
| c1 | `{}` | MOD-91 | — | `default` | **no** | Bash ran (positive baseline) |
| c2 | `{}` | + `settingSources` | — | `default` | yes | denied, no file. **D4's `_meta` lever holds** |
| c3a | `defaultMode: acceptEdits` | + `settingSources` | — | **`acceptEdits`** | no | Write ran: `_meta` does not reach the adapter's own mode read |
| c3b | `defaultMode: acceptEdits` | + `settingSources` | `default` | `acceptEdits` → `default` | yes | denied. **D4's `set_mode` pin holds** |
| c3c | `defaultMode: auto` | + `settingSources` | — | **`auto`** | no | Write ran. This box's user settings say `auto` |

- The adapter's own SDK hooks still fire under `settingSources: []`: c3a's tool call reached
  `tool_call_update` `completed`, which the adapter's `PostToolUse` hook emits.
- Every mode list was `default, acceptEdits, plan, auto, bypassPermissions`.
- D4 stands unamended. The live cases are re-pinned in `isolation_live.rs` on the production code in T5.

## Questions for the maintainer (defaults assumed below)

- **Q1 — Neutralise, or only show? Default: neutralise** (D1). An isolated session (`mods = n`) no
  longer reads any settings file.
  - **Show-only** instead would mean htui reads the three files and displays the rules that would
    pre-approve. That is new TUI surface (C2), it leaves the bypass open, and it would re-route to PRD.
  - **Costs of neutralising.** The isolated session also loses everything else in those files:
    `env`, `model`, `apiKeyHelper`, `enabledPlugins`, `enabledMcpjsonServers` (approvals for the
    project's `.mcp.json`), `additionalDirectories` and `defaultMode`.
  - **The two ways back** are `mods = y` (everything, rules included), or an inline `--settings`
    in the row's `extra_args` that sets `"disableAllHooks": true` plus the keys needed, which D8
    already allows. A key the operator puts in htui's row is htui configuration, and Settings shows it.
- **Q2 — `acceptEdits`. Default: retire it from the seed** (D5). The seeded `claude-cli` row gets
  `permission_mode: "default"` and one agent rule,
  `{"match":{"tool_kind":"edit"},"answer":"allow_once","reason":"…"}`.
  - Edits still proceed unasked in a chat, as today, but now through htui's policy: recorded, and
    refusable by a persona's rule. A persona's rules are evaluated before the agent's, so this also
    closes the `docs/personas.md:524-525` limitation.
  - Cost: `acceptEdits` also auto-approved filesystem shell commands (`mkdir`, `mv`, …). Those now
    ask.
- **Q3 — Existing installs. Default: (b) a narrow migration `0018`.** Seeds are a name-keyed
  insert-once (`seed_if_empty_as`, `pg/mod.rs:504-528`), and Settings › Agents has no field for the
  mode (`agent_settings.rs:29-39`). So without a migration an existing box keeps `acceptEdits`, and
  its `claude` row keeps whatever mode the user's `defaultMode` picks (`auto` on this box).
  - **(b)** rewrites a seeded row only while its values still equal the old seed:
    - `claude-cli` with `cli.permission_mode = 'acceptEdits'` and `permission.rules = []`;
    - `claude` (`transport = 'acp'`) with no `acp.permission_mode`.

    It bumps `updated_at` for the cache mirror (T4 checks how the mirror resyncs).
  - **(a)** is docs only: the operator edits the row by hand.
- **Q4 — Routing.** Stay on plan (see Routing above), or re-route to PRD.

## Decisions

- **D1 — CLI lever.** `ISOLATION_ARGS` gains the pair `--setting-sources`, `""` (spaced, as probed
  in B4). It comes after `--disable-slash-commands`, on every isolated session, in the same 6⅞
  position.
  - Rejected: `permissions.ask` in htui's `--settings` (D4 works for `Bash`). It would need a
    complete list of gated tools that drifts with every CLI version. It cannot reliably cover the
    user's MCP tools, and it leaves `defaultMode` and the rest of the files in force.
  - With `enabledPlugins` gone, plugin agents and plugin MCP servers from **settings-enabled**
    plugins should also drop. T1 records it, and the docs' "Still on" list follows T1, not this plan.
- **D2 — D8 extended.** `refuse_settings_override` also refuses `--setting-sources` (exactly that
  flag, spaced or `=`-joined, any value) in an isolated row's `extra_args`, because the last one
  wins (E1). The message names `mods (y/n)` as the way back. The doc comment that says
  `--setting-sources` "is not this check's" is rewritten. The function name stays: the message
  wording and its callers are unchanged apart from the new arm.
- **D3 — The CLI mode is always explicit.** An isolated, tool-carrying session whose
  `cli.permission_mode` is empty gets `--permission-mode default` (F3: otherwise `auto`). A
  non-empty value is the operator's and passes unchanged, `acceptEdits` included (Q2 changes the
  seed, not the code). Tool-less sessions still drop the mode (MOD-55 M1). Opted-in rows are
  unchanged.
- **D4 — ACP.**
  - `isolation_meta()` gains `"settingSources": []` in `options` (T1 verifies it end to end through
    the SDK).
  - A new `AcpSettings.permission_mode: Option<String>` (serde default `None`, skipped when `None`)
    is the mode htui pins. After `session/new`, `session/resume` or `session/load` succeeds:
    - if it is `Some(id)` and `session.modes()` reports a different `current_mode_id`, send
      `session/set_mode {id}`;
    - if the agent reports no modes, or none with that id, **refuse** with `DriverError::Transport`
      naming the mode. Fail closed: the approval path would otherwise be unknown.
  - `None` sends nothing, which is today's behaviour, and that is what `agy` gets. This applies
    whether or not the row opts back in: it is the row's own setting, not an isolation lever.
- **D5 — Seeds.**
  - `agent_claude_cli.json`: `cli.permission_mode` becomes `"default"`, and `permission.rules`
    gains the edit rule (Q2).
  - `agent_claude.json`: `acp.permission_mode: "default"`.
  - `seed_rows_match_ana4_5_3` (`model/agent.rs:299`) is re-pinned.
- **D6 — Migration `0018_seed_permission_mode`** (Q3 = b): forward-only and idempotent, touching
  only rows that still equal the old seed values. It is Postgres only: the agent mirror is the cache's.
  T4 verifies how a migrated `updated_at` reaches the mirror; if the mirror does not resync, T4
  stops and reports.
- **D7 — Out of scope.** Unchanged from MOD-91 D7: the repo's `CLAUDE.md`, and the user's own MCP
  servers in `~/.claude.json` (not a settings file; no `--strict-mcp-config`). Also out of scope: a
  form field for the mode (a separate item, if wanted).

## Patterns to mirror

| Category | Source | Pattern |
|---|---|---|
| Isolation lever | `crates/htui-agent/src/cli/mod.rs:255-259` `ISOLATION_ARGS`, doc rule 6⅞ (`:143-147`) | One const, with its doc naming the probe case; numbered argv doc |
| Refusal | `cli/mod.rs:315-357` `refuse_settings_override` | Exact-flag match, spaced or joined; `DriverError::Transport`; never quotes the value |
| ACP `_meta` | `crates/htui-agent/src/acp/mod.rs:1096-1128` `isolation_meta` / `session_meta` | One literal, pinned by one test |
| ACP session step | `acp/mod.rs:1194-1243` (new / restore), `model_values` (`:1994`) reading `session.config_options()` | Read `session.modes()` the same way; `cx.send_request(..).block_task()` as `initialize` does |
| Settings field | `crates/htui-agent/src/launch.rs:308-330` `AcpSettings` | `#[serde(default)]`, `skip_serializing_if = "Option::is_none"` as `model_config_id` |
| argv tests | `crates/htui-agent/tests/cli_driver.rs` exact-list tests, `refuse_settings_override` cases (`:1034-1090`) | Exact lists; a refusal asserts the message never contains the value |
| ACP tests | `crates/htui-agent/tests/acp_driver.rs` `isolation_meta_is_the_probed_options` (`:2150`), `a_resume_and_a_load_carry_the_same_meta` (`:2198`) | Scripted agent captures requests; the JSON is asserted |
| Live proof | `crates/htui-agent/tests/isolation_live.rs` (MOD-91) | `#[ignore]` cases, scratch config dir, results printed |
| Migration | `crates/htui-store/migrations/0009_agent_box_user_off.sql` | Header comment with the item, decision and "Forward-only (R-STO-5)" |

## Files to change

| File | Action | Task | Why |
|---|---|---|---|
| `crates/htui-agent/tests/isolation_live.rs` | UPDATE | T1 | Live cases: the CLI setting-sources lever (A4/B4), the ACP `settingSources` and mode pin |
| `crates/htui-agent/src/cli/mod.rs` | UPDATE | T2 | D1, D2, D3: `ISOLATION_ARGS`, `refuse_settings_override`, `argv` mode, doc rules |
| `crates/htui-agent/tests/cli_driver.rs` | UPDATE | T2 | Exact argv lists, new refusal cases, the explicit mode |
| `crates/htui-agent/src/launch.rs` | UPDATE | T3 | `AcpSettings.permission_mode` |
| `crates/htui-agent/src/acp/mod.rs` | UPDATE | T3 | `isolation_meta` + `settingSources`; the mode pin after new/resume/load |
| `crates/htui-agent/tests/acp_driver.rs` | UPDATE | T3 | `_meta` literal; `set_mode` sent, skipped and refused |
| `crates/htui-core/seeds/agent_claude_cli.json`, `agent_claude.json` | UPDATE | T4 | D5 |
| `crates/htui-core/src/model/agent.rs` | UPDATE | T4 | Seed test re-pinned |
| `crates/htui-store/migrations/0018_seed_permission_mode.sql` | CREATE | T4 | D6 (Q3 = b) |
| `crates/htui-store/tests/*` (migration test) | UPDATE/CREATE | T4 | A pre-seeded old row is rewritten; an edited row is not |
| `docs/htui-mcp.md`, `docs/personas.md`, `README.md` | UPDATE | T5 | Isolation: off/still-on lists, the ways back, the mode, the CLI permission-prompt paragraph |
| `HANDOFF.md` live coordinates | UPDATE | close-out | Next migration `0019` (if D6 lands) |

## Tasks

### T1 — ACP live probe, plus pinning the CLI result in `isolation_live.rs` (main thread; gate for T3)

- Install Node ≥ 22 and `claude-agent-acp@0.75.0` under `/tmp` (the sandbox has neither). Use the
  scratch `CLAUDE_CONFIG_DIR`. The access token lives for minutes: re-copy it before each batch.
- ACP cases:
  1. no `_meta` + project `allow: ["Bash"]` → Bash runs unasked. The baseline must be positive.
  2. `_meta.claudeCode.options` with `settingSources: []` (plus MOD-91's keys), with the scratch
     user `defaultMode` unset → `session/request_permission` arrives for Bash;
  3. scratch user `defaultMode: "acceptEdits"`: the `session/new` response `modes.currentModeId` is
     `acceptEdits`; after `session/set_mode {"default"}`, a Write asks;
  4. the adapter's own hooks still fire under (2), as in MOD-91 T2.
- The CLI cases A4/B4 become `isolation_live` cases on the production `argv`, once T2 lands.
- Amend D4 if (2) or (3) falsifies it; the maintainer re-confirms only if the lever set changes.
- **Validate**:
  `cargo test -p htui-agent --features test-support --test isolation_live -- --ignored --nocapture --test-threads=1`.

### T2 — CLI (can start now: the probe already holds)

- **TDD** in `cli_driver.rs`:
  - update the exact argv lists (two new elements on isolated sessions);
  - add `isolated_session_with_no_mode_says_default`, `opted_in_session_with_no_mode_passes_none`,
    and `tool_less_session_still_drops_the_mode`;
  - add refusal cases for `--setting-sources` (spaced and joined), plus "an opted-in row is never
    checked".
- Then implement D1 to D3, and update the 6⅞ doc rule and `ISOLATION_ARGS`' doc (probe cases B4,
  E1, F3).
- **Validate**: `cargo test -p htui-agent --all-features --test cli_driver -- --test-threads=1`.

### T3 — ACP (after T1)

- **TDD** in `acp_driver.rs`:
  - the `_meta` literal carries `settingSources: []`;
  - with `permission_mode: Some("default")` and the scripted agent reporting `currentModeId:
    "acceptEdits"`, a `session/set_mode` goes out after `session/new`, and also after resume and
    load;
  - no request when they already match, and none for `None`;
  - an agent offering no such mode refuses the start with the mode named.
- Then implement D4 in `launch.rs` and `acp/mod.rs`.
- **Validate**: `cargo test -p htui-agent --all-features --test acp_driver -- --test-threads=1`.

### T4 — Seeds and migration (after T3, for the key name)

- **TDD**: re-pin `seed_rows_match_ana4_5_3`; then add a Postgres migration test:
  - an old-seed `claude-cli` row and an old-seed `claude` row are rewritten;
  - a row whose mode or rules were edited, and a row named otherwise, are untouched;
  - running it twice is a no-op.
- Find out how the cache's agent mirror picks up a server-side `updated_at` change. If it does not,
  stop and report before writing the migration.
- **Validate**: `cargo test -p htui-core`, and
  `cargo test -p htui-store --all-features -- --test-threads=1` against `HTUI_TEST_DATABASE_URL`.

### T5 — Docs and end-to-end (after T2 to T4)

- **`docs/htui-mcp.md` §Isolation**:
  - "Off by default" gains the settings files themselves, with the keys that go with them;
  - "Still on" loses the MOD-96 bullet, and the plugin bullets follow T1;
  - add a "permission mode" paragraph (D3, D4) and the two ways back (Q1).
- **`docs/htui-mcp.md` §Permission prompts**: replace the `acceptEdits` sentence (Q2).
- **`docs/personas.md:524-525`**: the limitation now holds only for a row that still runs
  `acceptEdits`.
- **README**: one line under the isolation paragraph.
- **End-to-end**: `chat_live_cli` and `chat_live` (`--ignored`) on the shipped argv and `_meta`.
- **Full gate**: see Validation.

### Independence (file-set intersection)

| Pair | Files in common | Verdict |
|---|---|---|
| T2 ∩ T3 | none (`cli/mod.rs`, `cli_driver.rs` vs `launch.rs`, `acp/mod.rs`, `acp_driver.rs`) | **Parallel** (T3 waits only on T1) |
| T1 ∩ T2 | `isolation_live.rs` is T1's only; it uses T2's `argv` for the pinned CLI case | T1's CLI case after T2 |
| T3 ∩ T4 | none written in common; T4's seed carries T3's key name | **Serial**: T4 after T3 |
| T2 ∩ T4 | none | Parallel-capable |
| T5 | docs only | After all |

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| A user who routes through `env` or `apiKeyHelper` in `settings.json` (a proxy base URL, OpenRouter) loses it in isolated sessions | Medium | Docs name both ways back; the row's launch `env` already carries process env; `mods = y` restores everything |
| The SDK maps `settingSources: []` differently from the CLI flag | Low–medium | T1 case 2 tests it end to end; the fallback is `extraArgs: {"setting-sources": ""}` |
| `session/set_mode` races the adapter's own mode changes (plan exit) | Low | Pinned once, at start; later changes arrive as `current_mode_update` and go through the relay |
| The access token expires mid-probe (minutes, not hours, today) | High | Re-copy per batch; the host session refreshes the shared credentials |
| The migration does not reach the cache mirror | Unknown | T4 checks before writing it; stop and report |
| The CLI's default mode drifts again (`auto` today) | Medium | D3 makes htui's mode explicit, so the CLI default no longer matters for isolated rows |

## Validation

```bash
cargo fmt --check
cargo clippy --workspace --all-features -- -D warnings
cargo clippy --workspace -- -D warnings
cargo test --workspace --all-features --no-fail-fast -- --test-threads=1 2>&1 | tee /tmp/mod96-gate.log
grep -E "SIGABRT|overflowed its stack" /tmp/mod96-gate.log && exit 1 || true
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```

## Verified claims

| Claim | Verdict | Evidence |
|---|---|---|
| A project `permissions.allow` settles a call under MOD-91's levers | TRUE | probe A4 |
| `--setting-sources ""` sends that call to the prompt tool; auth and htui's MCP keep working | TRUE | probe B4 (init `mcp_servers` `htui: connected`, `result: success`) |
| `--setting-sources ""` parses to "no file sources"; flag and policy settings stay | TRUE | bundle: `if(e==="")return[]`; `t.add("flagSettings"),t.add("policySettings")` |
| `acceptEdits` settles an edit even with no settings files | TRUE | probe A5 |
| The last `--setting-sources` on the argv wins | TRUE | probe E1 |
| No mode and no settings → the CLI can start in `auto` | TRUE | probe F3 (parent-session env removed) |
| `allowManagedPermissionRulesOnly` cannot be set by htui | TRUE | bundle: `fe("policySettings")?.allowManagedPermissionRulesOnly` |
| `_meta.claudeCode.options` overrides the adapter's `settingSources` | TRUE (code) / T1 (live) | `acp-agent.js:5933-5935` spread order |
| The ACP starting mode comes from the settings files and `_meta` cannot set it | TRUE | `acp-agent.js:5827-5828, 5952`; `settings.js:88-97` |
| The adapter supports `session/set_mode`; htui's crates expose it | TRUE | `acp-agent.js:4738`; `agent-client-protocol-schema` 1.7.0 `SetSessionModeRequest` (`v1/agent.rs:1969`); `ActiveSession::modes()` (`agent-client-protocol` 2.1.0 `session.rs:1004`) |
| htui never sends `set_mode` today | TRUE | no `SetSessionMode`/`modes()` use in `crates/htui-agent/src` |
| Seeds never update an existing row | TRUE | `pg/mod.rs:504-528` `ON CONFLICT (name) DO NOTHING` |
| Settings › Agents has no mode field | TRUE | `agent_settings.rs:29-39` (nine labels) |
| CLI `Edit`/`Write`/`MultiEdit`/`NotebookEdit` map to kind `edit` for the policy | TRUE | `cli/claude.rs:508-517` |
| The next migration is `0018` | TRUE | `migrations/` ends at `0017_follow_up.sql`; HANDOFF live coordinates |
| `.snap` files mentioning `acceptEdits` are recorded streams, not seed renders | TRUE | `cli_map__cli_map__{sigint,thinking,turns}.snap` only |
| T2 and T3 touch disjoint files | TRUE | Files-to-change table |
