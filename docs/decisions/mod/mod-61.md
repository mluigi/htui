# MOD-61 - The run engine's scrubber masks nothing; folded into MOD-10 (decided, 2026-09-29)

**Decision: no code change; the work moves into MOD-10.** The maintainer chose on 2026-09-29 to
retire MOD-61 and carry its one rule on MOD-10, after reviewing the findings below.
**Requirements:** `R-SEC-3`.
**Origin:** the MOD-32 review gate (`docs/decisions/mod/mod-32.md`), opened 2026-09-28. MOD-32
made `TrimRecord::to_value` scrub the whole record; `scrub` masks the resolved secrets, then fails
closed on residue. Only the residue half was live on the run path.

## Why there is nothing to build yet

MOD-61 asked for the run engine's scrubber to be built "from the run's resolved secrets the way the
chat path does". On main at `34b9888` there are no resolved secrets on any path:

- The run engine's scrubber is `MinimalScrubber::new(std::iter::empty::<String>())`
  (`crates/htui/src/run_worker.rs:1253`), and `mask` returns its input unchanged on an empty list
  (`crates/htui-core/src/scrub.rs:119`).
- The chat path builds its scrubber from `spec.env` (`crates/htui/src/agent_worker.rs:3176`), but
  every production `SessionSpec` it is built from has an empty `env` (`agent_worker.rs:932`,
  `:1812`), with a comment naming MOD-10 as the item that fills it.
- The run engine's own sessions are built the same way: `htui-orch`'s `drive_once` sets
  `env: BTreeMap::new()` (`crates/htui-orch/src/engine.rs:5349`), again naming MOD-10.

So the masking half is inert everywhere, not only on the run path. A seam added now would carry an
always-empty list. The rule that matters is that the scrubber masks exactly what the session env
carries, and that can only be enforced where the env is filled, which is MOD-10's work in
`drive_once`.

## The verifier's scrubber

MOD-61 also named `run_worker.rs:935`, the `ShellVerifier`'s scrubber. That one stays pattern-only.
Handing it the resolved map would put the secrets inside `htui-orch/src/verify.rs`, which MOD-10's
verifier boundary (from MOD-62, `docs/decisions/mod/mod-62.md`) forbids. The verify child keeps
htui's own process environment, which never receives the resolved map, so its output has nothing of
that map to echo. Its credential-prefix rules still apply.

## What carries forward

MOD-10's HANDOFF entry now carries a **Run engine scrubber** note: build the run engine's scrubber
from the same map that fills the step's `SessionSpec.env`, keep the verifier's scrubber
pattern-only, and add a test pinning that a record string equal to a resolved secret is stored as
`[REDACTED]`.
