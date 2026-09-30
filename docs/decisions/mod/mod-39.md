# MOD-39 - Requirements tab and item traceability (done, 2026-09-29)

**Requirements:** `R-TUI-1`, `R-TUI-9`, `R-ENT-8`, `R-ENT-14`, `R-ENT-15`; `R-TUI-4` for CLEAN-6,
folded in.
**Origin:** ANA-11 §6 phase 2 (`docs/ANA-11.md`). MOD-38 built the seam, `item.resolution` and
`Resolution::default_for` (`docs/decisions/mod/mod-38.md`).
**Artifacts:** PRD [`.claude/prds/mod-39-requirements-tab.prd.md`](../../../.claude/prds/mod-39-requirements-tab.prd.md)
(routed PRD, C2 and C4 fired; decisions D1-D4 all (a), maintainer 2026-09-29), plan
[`.claude/plans/mod-39-requirements-tab.plan.md`](../../../.claude/plans/mod-39-requirements-tab.plan.md)
(P1-P13, fact-checked, confirmed 2026-09-29) and blueprint
[`.claude/plans/mod-39-requirements-tab.blueprint.md`](../../../.claude/plans/mod-39-requirements-tab.blueprint.md)
(F-1..F-16).
**Commits:** `c25bb49` (PRD, plan), `a5d7956` (blueprint), `6f8cb47` + `a1efffc` (T1 worker
module and its review), `634abfd` + `daae26c` (T2 picker, CLEAN-6 and review), `d94a6f5` +
`dcf858c` (T3 tab and review), `71446db` + `fbf7938` (T4 sub-tab and review), `67c6edd` +
`1b60d27` (T5 snapshot sweep and README), `eb2841a` (main merged in), `1ebcc73` (rust-reviewer
findings), plus the close-out commit.

## What shipped

- **Worker module `crates/htui/src/requirements.rs`** (plan P1-P9). Ten `StoreRequest` variants
  (three reads: `Requirements`, `RequirementDetail`, `ItemRequirements`; four tab writes; three
  citation writes) and four `StoreReply` variants, served through one or-ed `try_serve` arm in the
  `templates.rs` shape. The worker fills the author and box. No store, schema or trait change.
- **Maintainer gate (D1).** The project's `requirement_spec.owner_id` is the maintainer. Area
  create, mint, amend and withdraw re-check it in the worker; on a project with no spec, the first
  gated write claims it for `this_user` (`set_requirement_spec(project, None, …)`), and a lost race
  (`Stale`) defers to the winner. Every input check the worker can make runs before the claim, so a
  refused write claims nothing (F-9). Cite, uncite and re-confirm are not gated.
- **Deciding item.** Amend and withdraw take a typed item key, matched exactly (trimmed,
  upper-cased) in the requirement's own project only, before any write (P6).
- **Requirements tab (D2)**, registered before Settings: `1 Backlog 2 Skills 3 Requirements
  4 Settings 5 Chat`. A project → area → requirement tree, withdrawn rows dimmed with `✕`, a
  client-side filter (no read per keystroke, F-13), and a detail pane with body, rationale,
  version, coverage (citing item, kind, stamp, suspect, status, resolution) and the revision trail
  with each deciding item's key. Offline, revisions read "revisions need the database". Keys `a`
  new area, `n` new requirement, `e` amend, `W` withdraw (deciding key, then the requirement key
  typed back). Forms save on `Ctrl+S` (the `TextArea` takes `Enter`, F-1). Write keys are dimmed and
  refused offline or on a project this user does not maintain.
- **Reqs sub-tab (D3, D4)** in item detail, after Prompt; "Documents" became "Docs" so the strip
  stays 40 of 43 columns. Citations with kind, stamp and a `! suspect` marker; `r` re-confirms, `c`
  cites an active requirement of the item's project as `addresses` or `reserves` (ones already
  cited that way are not offered), `u` uncites after `y`. `amends`/`withdraws` citations record a
  decision and can be neither cited by hand nor uncited (P9).
- **Close-out resolution picker (T2).** `Resolution::default_for(Open)` is now `Withdrawn`, so
  `close_out_enabled` lets an `open` item through unchanged and `engine.rs` needed no edit (P13).
  The Warn stage cycles `Resolution::ALL` filtered by `closes_from(preview.status)` with `←/→` or
  `h/l`, starting on `default_for`, and `CloseOut` carries the pick. Postgres tests land both the
  default (`withdrawn`) and a picked `rejected` on an open item.
- **CLEAN-6.** The Runs pane module doc now reads "approve (`AnswerGate`) / reject with a typed
  note" (`crates/htui/src/ui/tabs/backlog/detail/runs.rs`).
- **README.md** covers the tab, the sub-tab and the picker.
- **Stale `engine.rs` docs.** Once MOD-40 had landed, the `Engine::close_out` and
  `close_out_preview` doc comments were brought in line with the picker (the maintainer asked for
  them here rather than as a separate item).

## Review

Each task had an adversarial review and a fix pass before merge (T3's major: the suspect marker
was clipped at 80 columns; it now follows the stamp). The configured `rust-reviewer` then reviewed
the whole change: no blocking or major findings. Fixed in `1ebcc73`: a stale answer now re-reads
the detail on screen; a refused mint re-reads before offering a retry (the worker answers `Failed`
when only its re-read failed, and a duplicate requirement could never be deleted); a tab-write
refusal frees only the write in flight; an amend over a head withdrawn elsewhere is refused locally;
`this_user` errors other than `NotFound` now fail the read instead of reading as "read-only".

## Carried

- **CLEAN-7**: the stale offline-buffer comments were fixed here at the maintainer's request
  (`ffd4ffc`); the code and UI text the removed buffer left behind are CLEAN-7
  (`docs/decisions/clean/clean-7.md`).
- **MOD-60**: the tab's `tree::pad`/`clip` and the Reqs pane's `cut` count `char`s, and on a narrow
  pane `pad` cuts the ` · read-only` marker before the project name.
- **MOD-59**: the Requirements tab lands writes by content like the Skills tab; a reply that names
  its write would retire both that and the mint re-read.
- **Known gaps.** A refusal only the store makes after the claim (a box or user row deleted between
  read and write) still leaves the claimed spec. The offline worker test uses an empty mirror, so
  cached areas and requirements offline are proved by MOD-38's cache tests, not here. `tree::groups`
  runs a few times per frame; fine at demo scale.

## Pins

`StoreRequest` 85 and `StoreReply` 47 variants; 107 files in `crates/htui/tests/snapshots` (43
existing ones re-accepted for the strip changes and the close-out hint only). No migration: the
next is still `0008`.
