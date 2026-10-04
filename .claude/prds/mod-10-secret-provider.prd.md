# MOD-10 — Secret provider (Infisical) and fail-closed scrubbing

> Routed as **PRD** by `/handoff-run MOD-10` (C2 and C4 fired, low confidence at the threshold;
> accepted by the maintainer 2026-10-03, sandbox run `hr/MOD-10`). Ultracode recommended for the
> implement phase (C4) and the review phase (wide, security-sensitive change set). `docs/ANA-7.md`
> is the design; it predates MOD-2's `Scrubber` seam and MOD-41's worker topology, and this PRD
> records where the tree has moved past it.
> Requirements: `R-SEC-1..4`, `R-TUI-8`, `R-ID-7`, `R-STO-1`, `R-NF-2`.

## Problem

The maintainer's projects keep their credentials in a `.env` file inside the repository. An agent
can read anything in its worktree, so today the only way to give a run a credential is to put it
where the agent — and from there the LLM context and the stored transcript — can see it. `htui`
has no way to hand a run a secret without that exposure: every agent session starts with an empty
secret environment, and the scrubber that guards every persisted byte has no secret values to mask,
so a transcript string equal to a secret would be stored verbatim. Left unsolved, every project
that needs a credential to build, test or run either keeps a readable `.env` or cannot be driven by
`htui` at all.

## Evidence

Paths are relative to the repo root, read on `hr/MOD-10` (base `187ca50e`).

- **Observed by the maintainer (2026-10-03):** "projects where I have a `.env` that the agent should
  not read at all, so we should move to Infisical". The maintainer already runs a **self-hosted
  Infisical** instance.
- **No session ever receives a secret.** `SessionSpec.env` (`crates/htui-agent/src/driver.rs`,
  documented as "Resolved secrets only (`R-SEC-2`)") is built empty by the graph engine's
  `drive_once` (`crates/htui-orch/src/engine.rs`, MOD-4 plan D176) and by the chat path
  (`crates/htui/src/agent_worker.rs`, whose comment names this item).
- **Exact-match masking is inert everywhere.** The run engine's scrubber and the verifier's are
  built with an empty secret list (`crates/htui-worker/src/runtime.rs`), and the chat path's is
  built from the empty `spec.env`. Only the fixed prefix rules (`crates/htui-core/src/scrub.rs`)
  are live (MOD-61, folded into this item).
- **Agent processes inherit `htui`'s whole process environment** (no `env_clear` on any agent
  spawn), and so does the verifier's `sh -c` (`crates/htui-orch/src/verify.rs`, plan D30, whose
  comment says this is acceptable only while `SessionSpec.env` is empty).
- **Per-project configuration already has a home but no writer.** `project.secret_provider` and
  `project.secret_scope` exist since migration `0001`; no store method writes them and `NewProject`
  / `ProjectPatch` exclude them on purpose.
- **Two tests are owed to this item:** MOD-62 (the verify child never sees a resolved secret) and
  MOD-61 (a record string equal to a resolved secret is stored as `[REDACTED]`).
- **Downstream:** MOD-48 (secret distribution, phase 2) is blocked on this item.
- **Infisical API facts** (survey 2026-10-03, against current docs): Universal Auth login is
  `POST /api/v1/auth/universal-auth/login`; listing is `GET /api/v4/secrets`; three failed logins
  lock the identity for five minutes; Infisical Cloud rate-limits secret reads (self-hosted does
  not). The official Rust crate (`infisical` 0.0.3) targets the deprecated v3 endpoint and drops
  the error detail R-SEC-4's "clear error" needs — a plan-level input, not a PRD decision.

## Users

- **Primary: the maintainer** (`R-USR-1`), running agents against their own projects whose code
  needs real credentials (API keys, database URLs, tokens) to build, test or run, and who wants to
  delete the project's `.env` from the tree.
- **Downstream items:** MOD-48 (worker-side secret distribution), MOD-44 (container env), MOD-55
  (scrubbing a body before it leaves).
- **Not for:** agent credentials (`R-AGT-9` unchanged — `htui` never holds them); team or
  multi-user secret policy (`R-USR-3`, phase 2); providers other than Infisical in this item.

## Hypothesis

We believe **resolving a project's secrets from Infisical at run start, straight into the agent
process environment only, with every transcript masked against those exact values** will **let a
project drop its `.env` without losing credential-dependent runs** for **the maintainer running
agents on their own projects**. We'll know we're right when **a project with its `.env` removed
completes a run whose tests need a secret, and no resolved value appears in any stored row.**

## Success Metrics

| Metric | Target | How measured |
|---|---|---|
| Credential-dependent run without `.env` | A real project, `.env` removed, configured against the maintainer's self-hosted Infisical, completes a graph run whose tests need a secret | One manual run recorded in the write-up |
| No resolved value persists | A record, event or note string equal to a resolved secret is stored as `[REDACTED]`; no resolved value appears in any persisted row | Test (owed by MOD-61) on the run path and the chat path |
| Verifier isolation | The verify child never sees a resolved secret, in its environment or otherwise | Test (owed by MOD-62) |
| `htui`'s credentials never reach a session | The DSN, Qdrant key and Infisical machine identity are absent from every `SessionSpec.env` and from the agent's environment | Test |
| Fail-closed persistence | A known key format surviving exact-match masking blocks the row and fails the run with a **typed** failure the UI shows | Tests per call-site family |
| Clear refusal (`R-SEC-4`) | Provider unreachable, bad credentials, missing project/environment, hidden values, or no identity on this box → the run is refused before any agent starts, with a reason naming the cause; never a plaintext fallback | Tests per refusal cause against a fake provider |
| Lockout-safe | A rejected login is never retried automatically | Test |
| Projects without a provider unaffected | A project with no secret provider configured runs exactly as today | Existing suites stay green |

## Scope

**MVP**

- **Scrubber hardening (`R-SEC-3`, `R-ID-7`).** Exact-match masks from every resolved value, plus a
  pattern rule set for known key formats broader than today's fixed prefixes, behind the existing
  `Scrubber` trait. The run-path scrubber is built from the **same** resolved map that fills the
  step's `SessionSpec.env`, so the two cannot drift; the chat path does the same. The verifier's
  scrubber stays pattern-only. A fail-closed scrub fails the run with a typed failure.
- **Provider (`R-SEC-1`).** A `SecretProvider` contract — list keys for a project scope, resolve
  values, bootstrap a machine identity, report health — with Infisical (self-hosted base URL
  configurable; cloud URLs work too) as the first implementation, using Universal Auth. The access
  token is held in memory only and reused until it nears expiry.
- **Machine identity storage (`R-STO-1`).** Client id and client secret live in the OS keyring of
  the box that executes the run, beside the DSN and Qdrant slots. Never in a file, `argv`, the
  process environment, a log or the store.
- **Run-start injection (`R-SEC-2`, `R-SEC-4`).** Scope is **per project, whole run**: one
  Infisical project + environment + path per `htui` project, and every step of a run (and chat
  bound to that project) gets the same map. Resolution happens in the process that executes the
  run (TUI or local worker), at run start, before any agent launches. Any failure refuses the run
  with a clear reason. Resolved values exist only in memory and in the agent's environment block.
- **Settings (`R-TUI-8`).** A secret-provider section: base URL, masked machine-identity entry
  (stored / not stored / unreadable, clear with confirm, never echoed), last health check. Per
  project: provider and scope (project, environment, path) editable and persisted to the existing
  columns.

**Out of scope**

- **Worker-side identity on headless boxes without a keyring** (systemd-creds provisioning via
  MOD-45's path) — such a worker refuses secret-needing runs with a clear error; MOD-48 or a
  follow-up owns delivery.
- **Server-side resolution and distribution** (MOD-48, ANA-16 §8 item 9; `R-SEC-2` unchanged).
- **Per-phase or per-step secret scoping** — least privilege per phase is a possible follow-up.
- **Other providers** (Vault, 1Password, env-file) — the contract leaves room, none ship here.
- **Writing, rotating or importing secrets** from `htui`, including a `.env` → Infisical importer.
- **Detecting or blocking a `.env` in a worktree** — natural follow-up, see Open Questions.
- **Container exec environment** (MOD-44) and remote dispatch (MOD-43).

## Constraints (fixed before planning)

- **Maintainer decisions, 2026-10-03:** (1) the machine identity comes from the executing box's
  keyring; a box without one refuses with a clear error; (2) a fail-closed scrub is a **typed run
  failure**, and `R-SEC-3`'s "marks the step failed" is amended to the run (no migration for a
  per-step reason); (3) scope is per project, whole run; (4) the maintainer's deployment is
  self-hosted, so the base URL is configuration, not a constant.
- Resolved secrets travel **only** through `SessionSpec.env` — never into `htui`'s own process
  environment, which every agent and the verifier inherit.
- No external daemon or CLI (`R-NF-2`): the provider is an in-process client.
- No plaintext fallback (`R-SEC-4`), and no secret value in any error message, log line or
  `Debug` output.
- The `Scrubber` trait stays the seam its call sites already use (MOD-2, MOD-32).

## Delivery Milestones

<!-- Status: pending | in-progress | complete -->

| # | Milestone | Outcome | Status | Plan |
|---|---|---|---|---|
| 1 | Scrubber hardening | Exact-match masking from a resolved map and a broader pattern rule set guard every persisted byte; a fail-closed scrub fails the run with a typed, visible reason | complete | `.claude/plans/mod-10-m1-scrubber-hardening.plan.md` |
| 2 | Infisical provider | A box with a machine identity in its keyring can authenticate to the configured Infisical, list and resolve a project scope's secrets, and report health; every failure is a typed, non-leaking error | in-progress | `.claude/plans/mod-10-m2-infisical-provider.plan.md` |
| 3 | Run-start injection | Graph runs and chat on a configured project receive its secrets in the agent environment only, with the scrubber built from the same map; failures refuse the run; the verifier never sees them | pending | — |
| 4 | Settings section | The maintainer enters the machine identity and base URL, sees provider health, and sets each project's scope, without leaving the TUI | pending | — |

## Carried into M3 from M1's review (2026-10-04)

- **Seam scan across persisted rows** (review M2): a key cut at the 16 KiB flush boundary persists
  in two rows; carry the scrubbed tail of the previous text row and check tail+head, sized for the
  longest resolved secret, not only the pattern rules.
- **Escaped token starts** (review L1): treat `\n`/`\uXXXX`-style escapes and `%XX` as token starts;
  re-run the host audit before shipping, since it widens what fails closed.
- **`(?-u:\b)` anchor** (review N1): equivalent; adopt only with a benchmark.
- **`from_resolved` normalisation**: a provider value with a trailing newline must mask its trimmed
  occurrence too.

## Open Questions

- [x] **`.env` in the worktree.** Should `htui` detect a `.env` (or similar) in a run's tree and warn
      or refuse, now that a provider exists? **Decided 2026-10-03:** follow-up item, filed at
      close-out; not in this MVP.
- [x] **Pattern rule set breadth.** A broader rule set catches more leaks but, fail-closed, can halt
      runs on false positives. Which source (curated subset of gitleaks' MIT rules vs extended
      prefix list) and how is a false positive handled? TBD — needs validation via a scrub pass
      over existing stored transcripts in the plan.
      **Decided 2026-10-03:** as proposed — the M1 plan measures candidate sets before choosing.
- [x] **Imported and referenced secrets.** Infisical returns imports separately; which copy wins on
      a key clash, and do hidden values (identity lacks read permission) refuse the run? Proposed:
      refuse on hidden values; precedence settled in the plan.
      **Decided 2026-10-03:** as proposed.
- [x] **Short values.** Exact-match masking of very short values (ANA-7 proposed length ≥ 6)
      over-masks transcripts; below the floor, mask anyway, skip, or refuse? TBD in the plan.
      **Decided 2026-10-03:** deferred to the plan.
- [x] **Resolve-once vs per-step.** Is one resolution per run enough for long runs, or does a
      retried / resumed step (MOD-37 resume) re-resolve? TBD in the plan.
      **Decided 2026-10-03:** deferred to the plan.

## Risks

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| A secret reaches a session through `htui`'s inherited environment | Medium | High | Constraint: values only via `SessionSpec.env`; test that `htui`'s own credentials never appear in the agent's env |
| Verifier sees a resolved secret | Medium | High | MOD-62 pinned test; verifier scrubber stays pattern-only |
| False positives from the pattern rule set halt runs | Medium | Medium | Open question above; measure against stored transcripts before choosing the set |
| Identity lockout from retried bad logins | Low | Medium | Never retry a rejected login; test |
| Infisical API drift (v3 → v4) on older self-hosted servers | Medium | Low | Thin client with version detection or a clear error naming the endpoint; decided in the plan |
| Secret value leaks via an error, log or `Debug` | Low | High | Constraint plus tests on every error path |
| Headless workers silently lack secrets | Low | Medium | Explicit refusal naming the missing identity; MOD-48 follow-up |

---
*Status: DRAFT — requirements only. Implementation planning pending via /plan.*
