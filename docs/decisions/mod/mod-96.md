# MOD-96 - Settings-file permission rules and the permission mode in htui-driven sessions (done, 2026-10-13)

**Requirements:** `R-TUI-6`, `R-HIS-1`, `R-AGT-4`, `R-AGT-5`.
**Origin:** MOD-91 review M1 (`docs/decisions/mod/mod-91.md`).

**What was done.** Run in a sandbox on `hr/MOD-96`. Routed as a plan (C3 only). The maintainer took every
default at CONFIRM (Q1 neutralise, Q2 retire `acceptEdits`, Q3 a narrow migration, Q4 stay on plan) and later
reversed Q2 after review R1. Plan: `.claude/plans/mod-96-settings-permission-rules.plan.md` (probe tables,
decisions D1-D7, the R1 amendment, verified claims). Blueprint: `.claude/plans/mod-96-settings-permission-rules.blueprint.md`.

- **Probes before the plan** (`claude` 2.1.296 on Haiku, scratch `CLAUDE_CONFIG_DIR`, a stub `permission_prompt` that
  answers deny):
  - Under MOD-91's levers, a trusted checkout's `.claude/settings.json` `allow: ["Bash"]` settled a Bash call
    without asking htui (A4).
  - `--setting-sources ""` sent that call to htui. Auth, htui's MCP server and htui's own `--settings` kept
    working (B4).
  - The last `--setting-sources` on the argv wins (E1).
  - With no settings file and no `--permission-mode`, the CLI started in `auto` (F3).
  - `acceptEdits` settles in-tree edits by itself (A5).
- **ACP probe, T1** (`claude-agent-acp` 0.75.0):
  - `_meta.claudeCode.options.settingSources: []` sent a project-allowed Bash call to `session/request_permission`.
  - The adapter still seeds its starting mode from the settings files' `defaultMode` itself; this box's user
    settings say `auto`. `_meta` cannot reach that read.
  - `session/set_mode {"default"}` after `session/new` made a Write ask.
- **CLI (T2):** `4bd98740`, red tests `c3cca23e`.
  - `cli::ISOLATION_ARGS` gains `--setting-sources ""` (D1).
  - `refuse_settings_override` refuses any `--setting-sources` in an isolated row's `extra_args`, spaced or
    `=`-joined, whatever its value, and never quotes the value (D2).
  - An isolated, tool-carrying row with an empty `permission_mode` gets `--permission-mode default`
    (`ISOLATED_PERMISSION_MODE`, D3). A row's own mode, `acceptEdits` included, passes unchanged.
- **ACP (T3):** `74b0cb02`, red tests `fe201cbb`, merged `1228eaf5`.
  - `isolation_meta()` gains `"settingSources": []`.
  - New `AcpSettings.permission_mode: Option<String>`, pinned by `pin_mode` with `session/set_mode` after
    `session/new`, `session/resume` and `session/load`, before the banner (D4).
  - `mode_to_set` fails closed when the agent reports no modes, or none with that id. The refusal is owed, so
    the child is killed and reaped before `start` answers.
  - `acp_conformance.rs`'s scripted agent now reports modes. Without that, the seeded pin would have refused
    every conformance case (blueprint F1).
- **Live pins:**
  - `isolation_live.rs` gained `cli_settings_file_allow_is_asked_under_isolation` (`d90f3f88`), with an
    opted-in positive baseline, and `acp_pinned_mode_asks_under_isolation` on the production ACP driver
    (`97fa9032`). The ACP case proves the pin by the reported mode, not just by the ask.
  - MOD-91's `cli_isolated_by_default`, `acp_isolated_by_default` and `acp_isolated_on_resume` were re-pinned:
    with no settings file read, no plugin is enabled, so the debug file no longer names the probe mods at all.
- **Seeds and migration:**
  - The `claude` (ACP) seed pins `acp.permission_mode: "default"`.
  - `0018_seed_permission_mode` sets the same pin on an existing `claude` row only while its
    `acp.permission_mode` is absent. An explicit JSON `null`, a set value and other names are left alone, and
    a second run matches nothing. `updated_at` moves through the trigger. The cache mirror replaces its
    agent table on every refresh, so no cache migration is needed.
  - Commits `08725ade` (red `df98a5bd`), reworked in R1a `4bce4c0c` (red `1c21d74c`, merged `e5d5121a`). Seven
    migration-count pins moved to 18 (`migrations.rs`, `connect.rs`).
- **Docs:**
  - `docs/htui-mcp.md` §Isolation now lists the settings files themselves as off, and with them their
    permission rules, `defaultMode`, `env`, `apiKeyHelper`, the plugins they enable and the project's
    `.mcp.json` servers (the last probed live, M1/M2).
  - New sections: the permission mode on both transports, an ACP row with no pinned mode, the second way
    back (an inline `--settings`, minus any `permissions` keys), a `--setting-sources` refusal, and upgrading
    every box together.
  - Whether the claude.ai connectors and `~/.claude.json` servers stay is recorded as not settled.
  - README follows. Commits `14876a39`, `5ff7c51f`, `c690e9f5`.

**Review.**
- **R1 (`rust-reviewer`): REQUEST CHANGES**, 1 HIGH, 3 MEDIUM, 4 LOW, 3 NIT.
  - **The HIGH:** the planned seed rule `{"tool_kind":"edit"} → allow_once` had no path condition, while
    `acceptEdits` allows an edit only inside the working directories and after the CLI's safety checks
    (confirmed in the 2.1.296 bundle). So out-of-tree and safety-checked edits (`~/.bashrc`,
    `~/.claude/settings.json`, `.git/hooks/*`) would have been allowed unasked.
  - **The maintainer reversed Q2 (option 1):** the `claude-cli` seed keeps `acceptEdits` and `rules: []`, and
    is byte-identical to base. `0018` touches only the `claude` row. Plan finding F5, that a `claude-cli`
    session with no prompt tool refuses edits, no longer applies. Plan amendment `7c8790ad`.
  - **The rest:**
    - a warning when an isolated ACP session pins no mode (`8960083d`);
    - a refused-`set_mode` test on all three routes, and the guard grids widened to `session/load`
      (`7d489731`, `813479ff`);
    - the live ACP case (`97fa9032`);
    - doc wording (R1a);
    - a `null`-mode migration case (R1a).
  - **Moot:** #5 (the no-prompt-tool banner) under option 1.
- **R2: APPROVE.** All R1 findings resolved. Applied in `78556ada`: warn only when the agent reports modes, so
  `agy` (no modes, cannot be pinned) is not warned on every start; plus two line wraps. Not applied:
  - an in-tree `Write` probe under the pin, optional, because the reported mode already proves it;
  - a live ACP check of the plugin and `.mcp.json` findings, recorded as unverified in the docs;
  - the warning that a database which applied the pre-R1 `0018` would refuse the reworked file on checksum.
    That version only ever ran on scratch test databases in this sandbox, never the host's.

**Gate.**
- On the merged tree before `78556ada`: fmt passes; clippy passes with and without `--all-features`;
  `cargo test --workspace --all-features --no-fail-fast -- --test-threads=1` gives 6151 passed, 0 failed,
  38 ignored, no `SIGABRT` or stack overflow.
- After `78556ada`: `htui-agent` 705 passed, clippy clean.
- Live `#[ignore]` cases run: the four `isolation_live` CLI cases and the four ACP cases.
- The host still owes the Postgres gate on the merged tree.

**Not done or not verified.**
- Featureless `cargo test -p htui-core` does not compile: `model/run.rs:1303` imports `crate::fixtures`, which
  exists only under `demo`. It predates this item (`a39893a0`, MOD-55).
- `agy` was not run against the new `settingSources` key; ACP requires unknown `_meta` keys to be ignored.
- No native Windows run (the MOD-16 note covers it).
- Isolation also drops the user's settings-file `deny` rules. Such a call now goes to htui's policy (ask by
  default). The docs cover it under "their `permissions` rules".
