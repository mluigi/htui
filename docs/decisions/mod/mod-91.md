# MOD-91 - Keep the user's mods, settings hooks and skills out of htui-driven sessions (done, 2026-10-13)

**Requirements:** `R-ID-5`, `R-AGT-1`, `R-AGT-4`, `R-TUI-6`, `R-HIS-1`.
**Origin:** ANA-29 (`docs/ANA-29.md` §4.4, §5.5, §6.1 item 1).

**What was done.** Run in a sandbox on `hr/MOD-91`. The routing table gave the PRD path at threshold; the
maintainer routed it as a plan. Plan `.claude/plans/mod-91-session-isolation.plan.md` (D1-D8, probe results,
verified-claims table); blueprint `.claude/plans/mod-91-session-isolation.blueprint.md`. No migration.

- **Live probes first (T1, T2).** Run under a scratch `CLAUDE_CONFIG_DIR`, because the sandbox shares `~/.claude`
  with the host. The scratch dir held a copy of the credentials without the refresh token, a user-scope mod
  installed from a scratch directory marketplace, a skills-directory mod in a trusted checkout, user and
  project skills, and a `SessionStart` settings hook.
  - **Baseline:** with no flags, both mods, the hook and the skills all load into an htui-shaped `claude -p`
    session (`claude` 2.1.294). The same happens over `claude-agent-acp` 0.75.0, which hardcodes
    `settingSources: ["user","project","local"]`.
  - **Chosen levers:** with `--settings '{"disableAllHooks":true}' --disable-slash-commands`, auth still works,
    htui's `--mcp-config` server connects and `permission_prompt` is still asked. Neither mod loads, the hook
    does not fire, and no skill is listed.
  - **Rejected levers:** `--safe-mode` drops htui's MCP server (the CLI exits "permission_prompt … not found").
    `--setting-sources project,local` still loads the skills-directory mod. `enabledPlugins` removes plugins
    only by name.
  - **What stays loaded:** plugin agents and plugin MCP servers stay under the levers, and so do Claude Code's
    built-in mods.
  - **A later `--settings` replaces ours.** It does not merge (T1 case 1a).
  - **ACP:** the same levers travel in `_meta.claudeCode.options`, and the adapter's own SDK hooks keep firing
    under them. `082f9fa3`, `5f77eab6`.
- **CLI default (T3, D1-D4, D8).**
  - `AgentSettings.user_extensions` (serde default `false`) and `cli::ISOLATION_ARGS`. `argv` emits the levers
    on every session (doc rule 6⅞), after the narrowing and before `extra_args`, unless the row opts in.
  - `refuse_settings_override` (D8) refuses, before the spawn, an isolated row whose `extra_args` carry a
    `--settings` that does not itself set `"disableAllHooks": true`. Its message never quotes the value.
  - `launch::mark_session` sets `HTUI_SESSION=1` after `spec.env`.
  - `6331ab48`, `d99912a4`, `7b701177`.
- **ACP default (T4, D5).** `acp::isolation_meta()` / `session_meta()` are sent on `session/new`,
  `session/resume` and `session/load`, with no `claudeCode` key when opted in. `HTUI_SESSION=1` is set in
  `AcpDriver::launch_for`; logins and probes are not marked. `7ab1c51f`, `805b4343`, `42f8b469`.
- **Settings > Agents (T5, D6).** A ninth form field, `mods (y/n)` (label chosen by the maintainer so it fits the
  13-column label column). It defaults to `n`, and a create from an opted-in row still starts at `n`. It writes
  `settings.user_extensions` only when the value changes, and every other settings key stays byte-for-byte.
  `9af1d62e`, `6c315cec` (merged `62e91a7a`).
- **Docs and live proof (T6).**
  - `docs/htui-mcp.md` gained "Isolation from your Claude Code setup", and its stale `--mcp-config=<json>` line
    now says `<path>`. The README gained "Your Claude Code mods, hooks and skills".
  - `crates/htui-agent/tests/isolation_live.rs` (`#[ignore]`) drives the shipped levers against real `claude`
    and `claude-agent-acp`. Six cases pass: the CLI opt-in baseline, the CLI default, the D8 refusal, the ACP
    opt-in baseline, the ACP default, and the ACP default after `session/resume` in a second process.
  - `chat_live_cli` and `chat_live` pass on the shipped defaults.
  - `4e950b69`, `0eeb0600`, `4c241834`.

**Review.** `rust-reviewer` (R1): APPROVE WITH FIXES, with no CRITICAL or HIGH findings and 1 MEDIUM, 5 LOW and 4 NIT.
All of them were applied in `31bc10a9`, `57fe4b02`, `f7c7ec83`, `cacfb2b1`, `50752e61` and `732a259f`.
- **M1:** the docs overclaimed "nothing approves before htui". Settings-file permission rules and the
  permission mode still do. The docs now say what isolation stops, and the gap is filed as **MOD-96**.
- **L2:** a live ACP resume case was added and passes.

**Gate.**
- fmt passes, and clippy passes with and without `--all-features`.
- `cargo test --workspace --all-features --no-fail-fast -- --test-threads=1` is green except
  `htui-worker` `a_batch_overshoots_its_cap_by_at_most_one_attempt_pg`. That test fails identically on the base
  `bb33692a`, so it predates this item. It is being filed by the `hr/MOD-92` run as **MOD-95** and was not
  re-filed here.
- The host still owes the Postgres gate on the merged tree.

**Not verified.**
- `agy` was not run against the new `claudeCode` `_meta` key; it is not installed in the sandbox. ACP requires
  unknown `_meta` keys to be ignored.
- Native Windows runs are covered by the MOD-16 note.

**Process notes.**
- T5's new unit tests could only fail to compile before the implementation existed. The settings suite was
  seen failing.
- The R1 doc-link fix is two commits, because `--amend` is banned on a shared tree.
