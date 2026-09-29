# MOD-62 - A verify command keeps the process environment; no allowlist (decided, 2026-09-29)

**Decision: no change to the verifier's environment.** A verify command keeps htui's own process
environment unchanged (MOD-4 plan D30). PR #20, which swapped it for a fixed allowlist, was closed
unmerged. The maintainer confirmed on 2026-09-29 after reviewing the reasoning below.
**Requirements:** `R-SEC-2`, `R-ORCH-11`.
**Origin:** review of PR #20 (CodeRabbit's verifier environment allowlist) during the 2026-09-29
merge run. MOD-62 was opened as the replacement for #20; it is retired here without code.

## Why nothing changes

ANA-2 `:493` asks that a verify command see "the agent's environment minus the secrets". On main
that is already what happens:

- `crates/htui-orch/src/verify.rs:321` hands the child the process environment and says why: the
  walk's `SessionSpec.env` is empty, so the agent's environment *is* this process's.
- The agent inherits the same environment. `crates/htui-agent/src/launch.rs:472` and `:1128` add
  `SessionSpec.env` on top, and neither calls `env_clear`. `htui-orch`'s `drive_once` builds every
  graph `SessionSpec` with an empty `env` (`engine.rs:5349`).
- The verifier's output is scrubbed before it is stored, like every digested byte.

So an allowlist on the verifier alone hides nothing the agent it checks cannot already read. The
verifier runs in the very tree the agent just had write access to.

It also breaks real verifiers. #20's list (`PATH`, `HOME`, temp dirs, `CARGO_HOME`, `RUSTUP_HOME`, a
few Windows roots) had no passthrough, so it dropped `HTUI_TEST_DATABASE_URL` (the Postgres suites
then print "skipped" and a `cargo test` verify passes without running them), proxy and CA
variables, locale, `RUSTUP_TOOLCHAIN`, `CARGO_TARGET_DIR`, and on Windows `APPDATA`, `PATHEXT` and
`ComSpec`.

## What carries forward

The one real boundary arrives with **MOD-10**: resolved project secrets will go into the agent's
`SessionSpec.env`, and that map must never be handed to the verifier. It holds by construction today
because the verifier never sees a `SessionSpec`. MOD-10's HANDOFF entry now carries that note,
including that a test pinning it belongs to MOD-10.

If a verifier ever needs its environment narrowed, for example on a shared box, the shape to revisit
is a named passthrough (`verify_env` per phase or as an `app_setting`), not a fixed allowlist.
