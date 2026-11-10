# MOD-97 - Strip inherited vendor API keys from subscription-billed agent spawns (done, 2026-11-10)
**Requirements:** `R-AGT-5`, `R-AGT-7`, `R-AGT-9`, `R-SEC-2`.
**Origin:** ANA-30 §2.3 and §8 call 7 (`docs/ANA-30.md`).

**What was done.** Run in the sandbox on `hr/MOD-97`. Routed as plan (C3 fired, C2 borderline);
the maintainer accepted the plan with decisions D1–D6 unchanged. Plan:
`.claude/plans/mod-97-subscription-env-strip.plan.md` (`f3ba2e3c`, verified-claims table inside).
Blueprint: `.claude/plans/mod-97-subscription-env-strip.blueprint.md` (`20e57976`).

**The defect.** `spawn_supervised` built every agent child with `.envs(&launch.env)` and no
`env_clear`, and Claude Code in `-p` mode always uses `ANTHROPIC_API_KEY` when present. An `htui`
started where the key was exported (`hr attach --api-key` or `--shell`), or a project secret of that
name (`SessionSpec.env`), moved every `claude`/`claude-cli` run onto API billing while the row still
said `subscription`, so the quota latch watched the wrong thing and no per-token cap applied.

## Decisions

- **D1: remove the variables, do not refuse the spawn.** The row's billing says what the operator
  wants; the key is an ambient variable nobody chose for the row. A removal of a variable that was
  actually set logs one `warn!` with the agent and the variable name, never the value. A run billed
  to a key is a `per_token` row (ANA-30 §2.3 route (c)).
- **D2: the list is row data, `agent.settings.api_billing_env`.** It is top-level in `AgentSettings`
  (both transports, like `user_extensions`) and applies only when `agent.billing = subscription`: a
  `per_token` copy of the row keeps the variables without editing the list. No vendor name is in Rust
  source; `tests/extensibility.rs::the_spawn_path_names_no_billing_variable` sweeps `src/` for the
  prefix. Not `discovery.credential.env`, which means the opposite ("this variable counts as logged
  in").
- **D3: seeded on `claude` and `claude-cli`** as `["ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN"]`.
  `agy` is not seeded: its `credential.env` lists `GEMINI_API_KEY` as a login tier, so stripping it
  would make the probe say ready and the session fail. Whether agy on that key is per-token billing is
  unverified.
- **D4: migration `0028_seed_api_billing_env`** adds the list to an existing `claude` (acp) or
  `claude-cli` (cli) row whose settings object lacks the key, mirroring `0018_seed_permission_mode`.
  An explicit value is left alone (`null` and `[]` both mean none); a second run matches nothing; a
  fresh database gets it from the seeds. No cache migration (the mirror replaces `agent` whole).
- **D5: mechanics.**
  - `ResolvedLaunch.env_remove` (`serde(default, skip_serializing_if = "Vec::is_empty")`, so a probe
    snapshot round-trips byte for byte; `Debug` prints it only when non-empty).
  - Each driver computes `strip_env` in `from_row_with_probe` (`launch::billing_env_to_strip`, an
    exhaustive `Billing` match).
  - `launch_for` runs `extend(spec.env)`, then `launch::strip_billing_env` (removes the names from the
    resolved env and sets `env_remove`), then `mark_session`.
  - `spawn_supervised` calls `env_remove` for each name after `.envs`, beside MOD-158's `HTUI_OWN_ENV`
    loop, so the inherited copy is gone too.
  - Probes and the login flow are not stripped: they run no billed turns.
- **D6: `--bare` pin.** Claude Code 2.1.322 has no `--no-bare`; `--bare` sets `CLAUDE_CODE_SIMPLE=1`
  and makes auth strictly key or `apiKeyHelper`. Pinned by
  `cli_driver.rs::the_seeded_cli_row_never_asks_for_bare` (seeded argv has no `--bare`, its env no
  `CLAUDE_CODE_SIMPLE`) and by the ignored live
  `cli_live.rs::case_11_a_subscription_row_ignores_an_inherited_api_key`, which re-runs itself with a
  bogus key and token in the parent environment and asserts the turn authenticates with
  `system/init.apiKeySource == "none"`. When the CLI makes `--bare` the `-p` default, case 11 fails.

## Review round R1 (rust-reviewer: no Critical or High)

- **M1:** a `null` or bare-string `api_billing_env` failed the whole `AgentSettings` parse, and the
  drivers' `unwrap_or_default()` then stripped nothing, silently. The field now reads `null` as none
  and a string as one name (`names_or_none`), and an unreadable settings document logs one `warn!`
  (agent, serde error class and position, never the document) before the defaults
  (`launch::settings_of`). The fallback itself stays: it is what every other setting does.
- **L1:** the session marker survives a list naming it in any case (`eq_ignore_ascii_case`), since
  Windows removes variables case-blind.
- **L2:** on Windows the strip also drops case variants from the resolved env (spawn-time removal is
  the authority on every platform).
- **L3:** `ResolvedLaunch::to_acp_config` does not apply `env_remove`; documented as never to be used
  to spawn a session (its one caller is a test).
- **L5:** an ACP "no list strips nothing" case, and a composed test through the factory where a real
  child sees neither listed variable on a subscription row and both on a `per_token` row.
- **L6:** case 11 matches `test result: ok. 1 passed` and says it builds argv by hand.
- **Not code, recorded here.** A row with `user_extensions = true` loads the user's Claude settings,
  whose `env` block or `apiKeyHelper` can set the key inside the CLI after the strip (L4). A
  subscription row created from scratch in Settings starts with `{}` and strips nothing until
  `api_billing_env` is set; the form does not edit that key (L7, also in `docs/hr-sandbox.md`).

## Commits

`09a75240` (T1 red), `3df38d39` (T1), `336068c8` (T2 red), `dfdcb0b3` (T2 seeds and 0028),
`0817daea` (T3 `--bare` pin), `7b6ee03a` (R1 red), `438a1101` (R1).

## Owed on the host

- `cargo test -p htui-agent --test cli_live -- --ignored case_11_a_subscription_row_ignores_an_inherited_api_key`
  (spends tokens; needs the subscription login).
- The Windows build of `remove_case_variants` (`#[cfg(windows)]`, compiled here only as a body).
- The Postgres gate on the merged tree. `0028` renumbers at collect if another branch takes it first.
