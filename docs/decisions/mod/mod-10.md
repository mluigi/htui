# MOD-10 - Secret provider (done, 2026-10-07)

**Requirements:** `R-SEC-1..4`, `R-TUI-8`, `R-ID-7`, `R-STO-1`, `R-NF-2`.
**Origin:** ANA-7 (`docs/ANA-7.md`, `docs/decisions/ana/ana-7.md`). Folded in: MOD-61 (the run engine's inert
scrubber, `docs/decisions/mod/mod-61.md`) and MOD-62's verifier-isolation test (`docs/decisions/mod/mod-62.md`).
**Artifacts:**
- PRD [`.claude/prds/mod-10-secret-provider.prd.md`](../../../.claude/prds/mod-10-secret-provider.prd.md):
  four milestones, maintainer decisions of 2026-10-03, the carry-overs between milestones;
- plans, each with its verified-claims table and decisions:
  `.claude/plans/mod-10-m1-scrubber-hardening.plan.md`, `.claude/plans/mod-10-m2-infisical-provider.plan.md`,
  `.claude/plans/mod-10-m3-run-start-injection.plan.md`, `.claude/plans/mod-10-m4-settings-section.plan.md`;
- blueprints `.claude/plans/mod-10-m1..m4-*.blueprint.md`: the per-milestone amendments (M4: A-1..A-11);
- operator page `docs/htui-secrets.md`: setup, scope, refusals, masking, logins and lockout safety, Settings,
  residuals.

Decision numbers are local to each milestone's plan (the MOD-31 convention).

Routed as **PRD** (C2 and C4 fired, low confidence at the threshold; accepted 2026-10-03). Run in a TOOL-7 sandbox
(`hr/MOD-10`); every milestone was planned, fact-checked, blueprinted, implemented in lanes and reviewed
(rust-reviewer, one verifier per finding).

## The problem

The maintainer's projects kept credentials in a `.env` inside the repository, readable by any agent working there.
htui had no way to give a run a secret without that exposure: every `SessionSpec.env` was built empty (MOD-4 plan
D176, the chat path), and the scrubbers on the run and verify paths were built with an empty secret list, so a
transcript string equal to a secret would have been stored verbatim (MOD-61).

## Maintainer decisions (PRD gate, 2026-10-03)

1. The machine identity comes from the **executing box's keyring**; a box without one refuses with a clear error.
2. A fail-closed scrub is a **typed run failure**; `R-SEC-3`'s "marks the step failed" amended to the run.
3. Scope is **per project, whole run**: one Infisical project + environment + path per htui project.
4. The deployment is **self-hosted Infisical**, so the base URL is configuration.
5. Open questions: a `.env` in the worktree is a follow-up filed at close-out (**MOD-89**); the pattern rule set
   measured before choosing (M1); hidden values refuse; short values and resolve-once deferred to the plans.

## What was built

### Milestone 1 - Scrubber hardening (`857be690`..`7b2bfd0e`, 2026-10-04)

- Whole-token pattern rules (15, `regex` in `htui-core`): an `openai_api_key` gate with a prose filter, Stripe live
  keys only.
- `MinimalScrubber::from_resolved` with a 6-character floor for exact-match masks.
- One typed `RunFailure::ScrubRefused` sentence on the plain-step, candidate, judge and chat paths.
- The opt-in `raw` re-checked per JSON pointer at the flush. `R-SEC-3` amended.
- The host audit (T0) was skipped; it is the `scripts/scrub-audit.sql` step still owed below.

### Milestone 2 - Infisical provider (`b2a4cfb8`..`5d93d748`, 2026-10-06)

- The `htui_core::secret` seam: `SecretProvider`, `SecretScope`, `ResolvedSecrets`, `MachineIdentity`, and a typed
  `SecretError` with 15 variants that never carry a value.
- Three keyring slots in `htui-store::secret`: `infisical-url`, `infisical-client-id`, `infisical-client-secret`.
- The new `htui-secrets` crate: Universal Auth, `GET /api/v4/secrets` with both imports spellings,
  folder-over-imports merge, fail-closed validation, https except loopback, no redirects, body caps, and a
  cancel-safe single-flight login with a permanent 401 latch and a 30 s cool-down after an unanswered login.
  The official `infisical` crate was not used (it targets the deprecated v3 endpoint and drops error detail).
- Operator page `docs/htui-secrets.md`.

### Milestone 3 - Run-start injection (`f8d26e35`..`a4f9ab7e`, 2026-10-06)

- One per-walk `htui_orch::secrets::RunSecrets` is both the walk's scrubber and its env source, so the two cannot
  drift (the MOD-61 requirement). It resolves lazily, once per walk, at the first live step, fan-out group or judge
  (and before an accept's verify).
- The agent gets exactly the resolved map as `SessionSpec.env`; a provider-less project never touches the keyring.
- Any failure is `secrets_refused: <cause>` and fails the run before a driver starts, transient causes too, with no
  retry. Chats resolve in their own task and leave no run row on refusal. `HTUI_`-prefixed keys (any case) are
  refused.
- One `KeyringInfisical` per process (TUI and `htui worker`) shares the 401 latch, with a 120 s keyring timeout.
- Scrubber carry-overs from M1's review: escaped token starts, newline-free and multi-line forms, zeroize on drop,
  and a hold-back seam at the 16 KiB flush. The verify output and agent error text are re-masked with the walk's
  scrubber; the verifier itself stays pattern-only.
- The tests owed by MOD-61 (a record string equal to a resolved secret is stored as `[REDACTED]`) and MOD-62 (the
  verify child never sees a resolved secret) landed. `Engine::dispatch` arms are boxed for debug-build stack
  headroom.
- Review: 1 high graded medium and 9 low, all applied in R1.

### Milestone 4 - Settings section (2026-10-07)

Plan `516605d5`, `7e17fdad` (confirmed 2026-10-06, OQ-1..4 as recommended, 18 claims fact-checked, 0 falsified);
blueprint `7c492de4` (amendments A-1..A-11).

- **Settings > Secrets** (appended last): Provider, URL, Identity and Health rows, then one row per project of the
  current workspace. `e`/`c`/`t` edit, clear (with a confirm) and check; `r` re-reads. The identity is a two-field
  form (client ID plain, client secret masked, both zeroized) and is never read back; the row shows
  `stored` / `not stored` / half stored / unreadable.
- **URL** normalised on the UI task (`normalise_base_url`) before any request; a refused URL sends nothing.
- **Provider health check** on the process's one `KeyringInfisical`, so a refused check latches exactly as a walk
  does. **Scope check** shows a key count only, never names or values (OQ-2).
- **Store writer for the secret columns** (T1): `ProjectPatch.secret: Option<Option<SecretScope>>` writes
  `secret_provider` and `secret_scope` together under CAS on both stores; `SecretScope` gains serde through its
  column form, so deserialising validates. One conformance case (`CASES` 148 → 149); one `.sqlx` entry; no
  migration (the columns exist since `0001`).
- **Keyring rows and redaction** (T2a): `crate::secrets_settings` with `Redacted` and `IdentityEntry`, whose `Debug`
  prints no value. Folded in (OQ-3): `SetQdrantApiKey` carries `Redacted`, closing a pre-existing `Debug` leak.
- **Project scope write** (T2b) with its own self-naming reply (`SecretScopeWritten`, A-1), so the Hierarchy section
  never mistakes it for its own write; the Hierarchy section adopts the tree passively.
- **Docs** (T4): `docs/htui-secrets.md` Settings section, latch and demo rules.

Blueprint amendments that changed behaviour:
- **A-4:** re-entering the **same** identity forces a provider rebuild via a keyring-write generation in
  `crates/htui/src/secrets.rs`; without it a latched provider survived until restart. The generation is per process
  (see MOD-90).
- **A-5:** no secret source under `--demo`: a demo walk or chat on a scoped project is refused, and the section
  sends no keyring request in a demo session.
- **A-6:** the Qdrant key field now stores what was typed. Before, a masked `TextField` always sent `""`, which the
  worker treated as clear.

## Review (M4)

rust-reviewer: 0 critical/high, 1 medium, 9 low, 7 NITs. One verifier per finding confirmed 9 (M-1 graded low) and
refuted L-7. R1 applied all 9:
- `eaf558ec`: M-1 a self-naming keyring-write reply (`SecretsWritten`), L-3 a self-naming `SecretsTree` read,
  L-4 a deleted project closes the scope form;
- `a03de95d`: L-1 a provider-check generation (the latch line shows only for a check built after the last write),
  L-2 a process-wide keyring I/O lock;
- `71c73e19`: L-5 an empty Qdrant key submit leaves the key unchanged, L-8 no join unwraps;
- `478ca1db`: L-6 the demo doc scope, L-9 fixed URL refusal sentences;
- `4194d0cb`, `cc040c2e`: blueprint and doc wording.

L-7 and the NITs are carried in **CLEAN-8**.

## Commits

- M1: `857be690`..`7b2bfd0e`; M2: `b2a4cfb8`..`5d93d748`; M3: `f8d26e35`..`a4f9ab7e`;
- M4 plan, blueprint: `516605d5`, `7e17fdad`, `7c492de4`;
- M4 T1 `ProjectPatch.secret`, `SecretScope` serde, `.sqlx`: `bdffbeeb`, `680b5538`;
- M4 T2a keyring rows, provider check, `Redacted`/`IdentityEntry`, Qdrant `Debug` fix: `9e008387`, `8e42667c`;
- M4 T2b project scope write and scope check: `090cab9c`, `46e66886`;
- M4 T3 Settings > Secrets section: `7188c47d`, `294d3a95`, `65caa6bb`;
- M4 T4 `docs/htui-secrets.md`: `bd756fb9`;
- M4 R1: `eaf558ec`, `a03de95d`, `71c73e19`, `478ca1db`, `4194d0cb`, `cc040c2e`.

## Verification (sandbox, 2026-10-07)

- `cargo fmt`; `cargo clippy --workspace` with and without `--all-targets --all-features`, `-D warnings`: clean.
- `SQLX_OFFLINE=true cargo check`: clean.
- Full workspace tests with `--test-threads=1` and insta: 10366 passed, 0 failed, 0 SIGABRT.
- `validate-workflow-docs.sh`: green.

## Host steps still owed before merge

1. `scripts/scrub-audit.sql` against the host's stored transcripts (M3 OQ-D; the M1 audit T0 was skipped), to see
   what the widened rules would fail closed on.
2. `crates/htui-secrets/tests/infisical_live.rs` once against the self-hosted Infisical (`HTUI_TEST_INFISICAL_*`,
   `docs/htui-secrets.md`, M2 carry-over). The URL and identity can now also be checked from Settings > Secrets.
3. The Postgres gate on the merged tree (the sandbox cannot reach the host database).

The PRD's success metric "a real project with its `.env` removed completes a credential-dependent run" needs the
live Infisical and is not recorded yet.

## Carried

- **MOD-89** - warn or refuse when a run's worktree holds a `.env` file (PRD open question, decided 2026-10-03).
- **MOD-90** - a separate `htui worker` keeps its 401 latch after the same identity is re-entered in the TUI (A-4's
  generation is per process); documented in `docs/htui-secrets.md`.
- **CLEAN-8** - review residuals: L-7 (`read_body` growing an unwiped `Vec<u8>`), the 7 NITs, and a `--demo` guard
  on Settings > Qdrant's keyring arms.
- **MOD-55 review L1** (`docs/decisions/mod/mod-55.md`), not filed: help turns still scrub with an empty `env`
  (pattern rules only), so `holds_mask`'s `[REDACTED]` count stays sound for now; once a help turn masks a project's
  resolved values, the runtime should report how many masks it applied. The note sits in HANDOFF under MOD-16.
- Out of scope per the PRD, unchanged: worker-side identity on headless boxes without a keyring and server-side
  resolution (MOD-48), per-step scoping, other providers, writing or importing secrets, container env (MOD-44).
- Documented in `docs/htui-secrets.md`: copies htui does not wipe (`SessionSpec.env` and the driver's copies, the
  agent process's environment block), and non-loopback requests going through the system proxy (untested, M2).
