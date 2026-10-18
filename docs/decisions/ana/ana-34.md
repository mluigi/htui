# ANA-34 - Coordinator at project and workspace level (concluded, 2026-10-18)

Full analysis: `docs/ANA-34.md` (surface as read, prior art, options, sources, what was not verified).
Origin: the 2026-10-15 requirements review, `docs/requirements-review-2026-10-15.md`. Addresses
`R-ORCH-14`, `R-TUI-9`, `R-ENT-8`, `R-ID-6`, `R-ORCH-7`, `R-ENT-15`; touches `R-BOX-4`, `R-STO-1` and
`R-SEC-2..4` at their seams.

## Verdict

The maintainer accepted all eleven answers of `docs/ANA-34.md` §8 ("all as recommended").

- **Hybrid, "propose versus dispose".** Deterministic code does every write: landing a run's work,
  recording the outcome, the close-out transaction, follow-up items and citation re-confirmations once
  a person accepts them. An optional agent part only proposes drafts. The agent never holds a push or
  forge credential, never merges, never resolves a conflict and never closes an item.
- **A role, not a process.** No coordinator box or resident process. `finish_run(Done)` writes one
  integration record per (run, repo); a new integrate arm of `sweep_once`, on the run's executing box,
  claims and performs it under a lease, idempotently, and parks on any refusal as reconcile does.
- **Agent turns on demand.** A person's message or a checklist opens a short turn seeded from the
  store (resumed where the driver can); "always take messages" (`R-TUI-6`) means a message always opens
  a turn. Turns are runs of a new kind `coordinator`; `run` gains a workspace owner.
- **Three modes per repo, project default, snapshotted at run start:** `local` (today's behaviour, the
  default), `push` (fast-forward only, never forced), `pull_request` (a run branch `htui/item/<key>`,
  `worktree` and `copy` isolation only).
- **Two levels of integration.** ANA-33's integrate step combines one run's parts and may resolve
  conflicts; the coordinator lands finished runs and never does, drafting a follow-up item instead.
- **Forge adapter**, four verbs (push, open or update a PR, auto-merge, read state): OneDev first, since
  `git.mluigi.it` runs OneDev, then GitHub; Forgejo/Gitea and GitLab later behind the same trait.
- **Credentials** in the executing box's OS keyring, used only by deterministic code through an htui
  credential helper (MOD-108's), never in argv, a file or any agent's environment; scrubbed; no
  fallback. Headless workers get the systemd-credential exception the DSN has.
- **Close-out** stays a person's confirmation, also in auto mode, over a checklist behind a new
  `CloseOut` waiting reason; resolution `done` requires every integration to have landed.
- **Off-box targets:** after ANA-42's collect (its diffstat confirmation stays the person's gate) the
  coordinator integrates per mode; ANA-42 §7.5's "merging stays the person's step" holds for `local`.
- **Workspace coordinator** follows and relays in version one and integrates nothing of its own;
  cross-project landing and "which work next" go to ANA-41.
- **Requirement edits owed** (maintainer-gated, `docs/REQUIREMENTS.md` not edited at close-out):
  `R-TUI-9` final text (`docs/ANA-34.md` §10.1), `R-ORCH-14`, `R-ID-6`, `R-ENT-8`, `R-ORCH-7`,
  `R-ENT-15`, `R-BOX-4`, `R-STO-1`, `R-ORCH-11`, `R-TUI-6`, `R-MCP-2`, and a `CONCEPTS.md` line.

## Spawned items

| Filed | From | Subject | Blocked on |
|---|---|---|---|
| MOD-133 | P1 | Integration records and modes (local, push, pull_request) | none |
| MOD-134 | P2 | Forge adapter and push/forge credentials | MOD-108, MOD-133 |
| MOD-135 | P3 | Close-out checklist | MOD-133 |
| MOD-136 | P4 | Coordinator turns and the workspace scope | MOD-135, MOD-115 |

Effects on open items: ANA-41 is unblocked; MOD-121's coordinator groups wait on MOD-136; ANA-33
carries the two-level split as a constraint; ANA-42 folds the narrowed merge sentence into its §8;
MOD-108's helper gains MOD-134 as a consumer; ANA-38's memory joins the coordinator turn's seed.

## Commits

`dd77fa67` (draft), and the conclusion commit that adds this file.

## Not verified

See `docs/ANA-34.md` §11: the bodies of `adopt`, `claim_scan` and the `GixIsolator::reconcile`
dispatcher; the OneDev version on `git.mluigi.it` and its exact pull-request API; a keyring-backed
credential helper on a headless Linux box; ANA-42 §8 still unanswered (Q9 depends on it); a few
prior-art details recalled, not fetched.
