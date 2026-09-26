# MOD-7 - Box registry + capabilities (done, 2026-09-26)

**Requirements:** `R-BOX-1..4`, `R-ORCH-10`, `R-AGT-6`, the box-profile clause of `R-TUI-8`; by
construction `R-NF-3`, `R-ID-4`, `R-SEC-3`, `R-PRM-1`, `R-PRM-3`.
**Design authority:** no ANA precedes this item. The contract is the requirements above, with the
capability refusal designed in `docs/ANA-2.md` §4.10 (criterion 14) and the box profile projection
and excerpt fallback root in `docs/ANA-5.md` §4.2 and §4.5. The PRD gate decisions D0-D7 settle
what those leave open.
**Artifacts:** PRD [`.claude/prds/mod-7-box-registry.prd.md`](../../../.claude/prds/mod-7-box-registry.prd.md)
(routed PRD 2026-09-25, gate decisions D0-D7). Plans and `code-architect` blueprints, one pair per
milestone:

| # | Plan | Blueprint |
|---|---|---|
| 1 | [`.claude/plans/mod-7-box-identity-probe.plan.md`](../../../.claude/plans/mod-7-box-identity-probe.plan.md) | [`.claude/plans/mod-7-box-identity-probe.blueprint.md`](../../../.claude/plans/mod-7-box-identity-probe.blueprint.md) |
| 2 | [`.claude/plans/mod-7-box-settings-section.plan.md`](../../../.claude/plans/mod-7-box-settings-section.plan.md) | [`.claude/plans/mod-7-box-settings-section.blueprint.md`](../../../.claude/plans/mod-7-box-settings-section.blueprint.md) |
| 3 | [`.claude/plans/mod-7-capability-refusal.plan.md`](../../../.claude/plans/mod-7-capability-refusal.plan.md) | [`.claude/plans/mod-7-capability-refusal.blueprint.md`](../../../.claude/plans/mod-7-capability-refusal.blueprint.md) |
| 4 | [`.claude/plans/mod-7-paths-excerpts.plan.md`](../../../.claude/plans/mod-7-paths-excerpts.plan.md) | [`.claude/plans/mod-7-paths-excerpts.blueprint.md`](../../../.claude/plans/mod-7-paths-excerpts.blueprint.md) |

Decision and risk numbering runs across the four milestones: milestone 1 D1-D38, R-1-R-19, OQ-1-OQ-12;
milestone 2 D39-D74, R-20-R-32, OQ-13-OQ-19; milestone 3 D75-D103, R-33-R-43, OQ-20-OQ-25;
milestone 4 D104-D144, R-44-R-58, OQ-26-OQ-32.
**Commits:** four milestones, `27c4318`..`c46a11e`. Each milestone below names its own range.
**Spawned:** MOD-49 (interactive path picker, PRD gate), MOD-51 (probe spec editor, milestone 2
OQ-18), MOD-52 (`ctrl-c` does not quit, milestone 2 fact-check), MOD-53 and MOD-54 (milestone 2
review deferrals), MOD-58 (milestone 3 review test gaps), and CLEAN-5 (a pre-existing MOD-4 test
flake seen at milestone 4's gate). MOD-57 was raised by MOD-9 at the merge of milestone 2.

## What shipped

Every box fact `htui` reads is now true, and every capability mismatch is a named refusal. A box is
keyed on the random id in its `box.toml`, checked by a keyed hash of the OS machine identity, so a
rename keeps the box and a copied `box.toml` mints a new one. A probe fills hardware, the `R-BOX-2`
tool list and derived tags at registration, on an `htui` version or probe-spec change, and on
demand, and then probes the agents and names installable missing ones. Settings > **Boxes** lists
every box of the user and edits declared tags and multi-line quirks as a compare-and-set a reconnect
cannot stale. An item whose `required_tags` the box lacks is refused at enqueue (no `run` row, item
`blocked`, the note names exactly the missing tags) and at claim (inside the admission transaction,
`run.failure = "missing tags: a, b"`), which is ANA-2 criterion 14's capability half with its own
conformance case. Repo paths are inferred under this box's workspace root, remote first and name
second, and phase prompts and the Backlog preview carry excerpts read from those roots.

**PRD gate decisions (2026-09-25), all honoured.** D0 MOD-40 is not a dependency. D1 the box id is
the key and a machine fingerprint checks it; this absorbed ANA-16 C4's re-key from MOD-40, which
keeps the heartbeat half. D2 probed tags come from a presence map that is data. D3 quirks get a
multi-line widget. D4 the section lists every box; the probe runs on this box only. D5 paths are
inferred per repo with a manual fallback, and the picker is MOD-49. D6 excerpts are wired here.
D7 registration offers install and never performs it.

**Migrations:** one, `0005_box_identity` (milestone 1). Milestones 2, 3 and 4 needed none, so the
next migration is `0008` after MOD-38's `0006` and MOD-9's `0007`.

---

## The four milestones

### 1 - A box knows itself (`27c4318`..`670c108`, 2026-09-25)

- **Identity.** Migration `0005_box_identity` drops `UNIQUE (user_id, hostname)` and adds
  `machine_fingerprint`, `edit_version` (milestone 2's CAS token) and `probe_spec_digest`.
  `register_box` keys on the `box.toml` id and inserts first. A stored keyed-HMAC fingerprint that
  disagrees mints a new box (`Registration::Copied`, never adoption). A box with no readable machine
  identity registers by id alone. A failed `box.toml` write-back stays online under the minted id
  for the session. The raw machine identity never reaches Postgres, a log or a file.
- **Probe.** `htui_agent::box_probe` fills hardware (`sysinfo`; GPU detection per OS), `box_tool`
  and `probed_tags` from a seed `spec.json` (39 tools, 14 tag rules) overlaid by name from
  `app_setting.box_probe_spec`. It runs after an `Online` swap when the box was never probed, on an
  `htui` version change or a spec digest change, then probes the agents and names installable
  `missing` ones on the status line; it never installs (D7). `StoreRequest::ProbeBox` exists, bound
  to no key until milestone 2. `WriteStore` gained `record_box_probe` and `boxes`.
- **Left open:** R-18, macOS `xcode-select` stubs (`/usr/bin/git` and others) open an install dialog
  when probed on a Mac without the Command Line Tools. Deferred until someone verifies macOS; the
  guard is sketched in the milestone 1 blueprint §11. The Windows runtime facts moved to MOD-16.

### 2 - The maintainer sees and edits it (`63f5673`..`00c2d48`, 2026-09-26)

- `BoxRow` carries `edit_version`. `WriteStore::edit_box` is a compare-and-set on it over every
  store (`NotFound`, then `Stale`, then `Constraint`; another user's box is `NotFound`), and no
  reconnect, probe or `register_box` bumps it. Declared tags go through `canonical_declared_tags`
  (`[a-z0-9_-]{1,64}`, sorted, deduplicated, an invalid tag refused).
- `crate::box_settings` serves `StoreRequest::{Boxes, EditBox}` on the store worker and shows the
  effective probe spec read-only.
- Settings > **Boxes**, the seventh section: `t` tags, `e` quirks in the new multi-line
  `htui::ui::TextArea` (`ctrl-s` saves), `p` probes this box only and refuses locally while a probe
  is in flight (a deviation from blueprint D63, `16079aa`), `r` reloads.
- The probe spec editor was deferred to MOD-51 at the CONFIRM gate. The manual live check was
  skipped by the maintainer; the Postgres `box_probe_pg` cases cover the reconnect, the registration
  probe and the concurrent edit instead.
- Review: three low findings; two applied (`e2c6072`, `00c2d48`), two deferrals opened as MOD-53
  and MOD-54.

### 3 - A mismatch is refused by name (`a9d2908`..`d73d120`, 2026-09-26)

- `R-ORCH-10` at enqueue and at claim, no migration. The engine reaches box tags through a seventh
  `GraphSource` method, `missing_tags(item, box)`, over the store's existing read.
- `Engine::enqueue` refuses an `open` or `failed` item whose `required_tags` the box lacks before it
  resolves the graph, like MOD-4's rung-4 refusal: no `run` row, the item `blocked` (a `failed` item
  stays `failed`) with the note `missing tags: a, b`, `EngineError::MissingTags`, and `Unblock`
  reopens it. That is ANA-2 criterion 14's capability half, pinned by its own conformance case.
- At claim, `claim_run` answers the new `Claim::MissingTags` (after `NotClaimable`, before
  `SlotFull` and `Overlaps`) inside the admission transaction. It fails the run with
  `run.failure = "missing tags: a, b"` and blocks the item on MemStore and PgStore alike; `Claim`
  lost `Copy`. The engine maps it to `EngineError::MissingTags`, never `ClaimRefused`, so the worker
  does not re-queue it. The note is written after the transaction; `run.failure` is the lasting
  record.
- **Known gaps:** a claim refused on the worker's retry path (`reclaim`) reaches the note but not the
  status line; PgStore reads `item.required_tags` unlocked by design, because a lock taken after the
  box's would deadlock with `create_run`.
- Review: one medium, six low; five applied (`49d809c`..`d73d120`), two rejected, two test gaps
  deferred as MOD-58.

### 4 - Paths and excerpts are real (`e4ca5b3`..`c46a11e`, 2026-09-26)

Plan `e4ca5b3` (fact-check: 56 claims, 46 verified, 10 amended, 0 falsified; OQ-26..OQ-32 all
answered with their defaults on 2026-09-26), blueprint `eac2ed6` (deviations P-1..P-12, decisions
D126-D144). Five tasks in two waves of worktrees, merged in order: T0 `7810c4c`, T1 `2626737`, T2
`c312cbf`, T3 `97f29fd`, T4 `81e74f0`, then `e7f110e` (docs) and the review fixes `8798792`,
`3ace657`, `d7171ef`, `c46a11e`. 35 commits (30 without merges), 35 files changed. No migration.

- **T0, the insert-if-absent writer (D104, D105, D110).** `WriteStore::infer_repo_box_path(&RepoBoxPath)
  -> Result<bool>` inserts only where no row exists for `(repo_id, box_id)`: `INSERT … ON CONFLICT
  (repo_id, box_id) DO NOTHING` on PgStore, the same rule on MemStore, forwarded by `Writer`,
  `UsageSpy` and `SpyStore`. There is no manual-row marker and no migration (OQ-26). A manual
  `SetRepoPath` upsert always wins, and inference never replaces a row, so a stale inferred row is
  fixed by hand like a stale manual one. The row's absence is the compare-and-set token, which no
  reconnect changes. One store conformance case, `infer_repo_box_path_inserts_only_where_absent`.
- **T1, discovery and matching (D111-D113, D123, D130, D131).** A new `htui_orch::infer` module.
  `normalise_remote` makes one key of `https://github.com/o/r.git`, `git@github.com:o/r`,
  `ssh://git@github.com:22/o/r/` and `https://user:tok@GitHub.com/o/r`; it strips userinfo, so a
  `Checkout` carries normalised keys only and no raw remote URL is logged, rendered or reported.
  `find_checkouts` walks a canonical root depth-first in byte order, never following links, at most
  `MAX_DEPTH` 3 below the root and `MAX_DIRS` 5000 directories; a truncated scan infers nothing.
  `choose` takes the remote rung first, then the name rung; a name match whose remotes all
  normalise elsewhere is rejected (OQ-31), and several candidates on either rung are a failure.
- **T2, the shared excerpt pass (D106-D109, D118, D119, D122, D125-D129, D132, D133, D141).**
  `htui_agent::excerpt::excerpts_for` holds the whole pass, so the engine and the preview cannot
  drift (MOD-2 D103's rule). The root per scope repo is the step's `run_step_tree` row, else this
  box's `repo_box_path` row, else `no_path` (ANA-5 §4.5 step 1). The budget is the residual measured
  by assembling the spec once without excerpts (`htui_core::prompt::excerpt_residual`, OQ-29). A
  file the scrubber refuses is dropped with a note instead of refusing the whole prompt (OQ-30,
  which decides ANA-5 open 8) through `htui_core::prompt::drop_unmaskable_excerpts`; since review
  fix M-1, `withhold_unmaskable_notes` also withholds any pass note that names a masked or refused
  string. The blocking read runs under `spawn_blocking` only when a root is readable. The engine
  calls it through `Engine::with_excerpts` from `assemble_prompt` only, so `phase_spec` still builds
  `no_excerpts` and the promote/handoff path reads no files (D125). A fan-out group assembles before
  any candidate tree exists, so it reads `repo_box_path` and says so in a note (OQ-32). Tier 2
  (`changed_paths`) stays empty (D122).
- **T3, the preview (D120, D139).** The Backlog preview runs the same pass over this box's
  `repo_box_path` roots for the project's repos, so the Prompt sub-tab shows what a run would read.
  `empty_excerpts` and its unit test are gone; MOD-2 D110 (no repo read in the preview) is reversed.
- **T4, inference served and shown (D114-D117, D124, D134-D138, D142-D144).**
  `StoreRequest::InferRepoPaths(WorkspaceId)` is served by `crate::hierarchy` on the store worker:
  this box's root, canonicalised; one walk under `spawn_blocking`; `choose` per repo without a row,
  sequentially, with the held set growing with each write; each match canonicalised (F-102) and
  inserted-if-absent; a reply `RepoPathsInferred` carrying the re-read tree and a per-repo report
  with names, canonical paths and outcomes, never a URL or an id. Settings > Hierarchy sends it on
  `i`, and once automatically after an applied editor write of a workspace root, a new repo or an
  edited repo when this box has a root (D136; `p` is excluded). The manual fallback is the existing
  `b` key (OQ-28). The report becomes the section's notice (`inferred N of M · K already set · no
  checkout: … · ambiguous: … — b on a repo sets it by hand`, names capped at three plus `+N more`).
- **Every production phase-prompt digest changes.** This is the PRD's expected risk: digests are
  recorded per step and no test pins a value. Handoff digests do not change (D125).
- **Pins moved:** store `CASES` 76 → 77, `.sqlx` 267 → 268, `StoreRequest`/`StoreReply` 68/39 →
  69/40, `hierarchy::REQUEST_NAMES` 12 → 13, `crates/htui/tests/snapshots` 87 → 88 (the new
  `hierarchy__inferred`). `htui-orch` `CASES` stays 72, `GraphSource` stays at seven methods,
  migrations are unchanged.
- **Verifier rounds** fixed a masked path that could appear in a T2 drop note (`1b487f9`) and, in
  T4, added the D134 held-set and D136 follow-up tests and carried only the follow-up's cause in
  front of an inference report (`42450d0`, `22279fb`).
- **Review** (`rust-reviewer`, Opus 5.5): approve with warnings; one medium, four low.
  - M-1, applied (`8798792`): an excerpt pass note could name a string the scrubber masked or
    refused and reach `trim_record.notes` unscrubbed; `withhold_unmaskable_notes` withholds it.
  - L-1, applied (`d7171ef`): the held set is canonicalised, so a checkout that a legacy linked
    row points at is held.
  - L-2, applied (`3ace657`): a cancelled excerpt pass is told apart from a panicked one in its note.
  - L-4, applied (`c46a11e`): `infer_one` is pinned to keep a row that lands before its insert
    (`AlreadySet`).
  - L-3, rejected: one store read instead of one per scope repo needs `repo_paths(box)`, which is
    not on the `ReadStore`/`WriteStore` traits; adding it to every implementor is out of proportion
    to the saving.

---

## Deviations from the PRD and ANA-5

- **The judge prompt carries no excerpts (D109).** The PRD's D6 says "phase and judge prompts";
  ANA-5 §4.1's judge placeholder set cannot place `{{excerpts}}`, and the default `verdict` body
  does not either, so reading files for a judge would be I/O for an audit row only. This is a
  deliberate narrowing of PRD D6.
- **A fan-out group reads `repo_box_path`, not a candidate tree (D108, OQ-32).** ANA-5 §4.5 assumes
  every candidate tree exists at stage 3; `drive_group` assembles before any candidate is prepared.
  The group's prompt says so in a note.
- **The manual path text box is Settings > Hierarchy's `b` (D115, OQ-28),** not a new box in the
  Boxes section as MOD-49's text first placed it.
- **No manual-row marker (D110, OQ-26).** Inference only inserts where no row exists.
- **ANA-5 open 8 is decided here** (a scrubber-tripping excerpt is dropped with a note, D119) rather
  than in ANA-5.
- **Stale ANA-5 text, not edited here (ANA edits are maintainer-only):** ANA-5 §4.5 and MOD-2's
  record say `repo_box_path` has no reader and no writer. `run_worker::repo_map` reads it, MOD-15's
  `SetRepoPath` writes it by hand and milestone 4's inference writes it automatically.
- **The PRD's constraint "next migration is `0005`"** was true for milestone 1 only.

---

## Carried

| What | Owner | Detail |
|---|---|---|
| R-18, macOS `xcode-select` stubs | none (deferred) | A probe on a Mac without the Command Line Tools opens an install dialog; guard sketched in the milestone 1 blueprint §11. Revisit when someone verifies macOS. |
| Windows runtime facts of the probe | **MOD-16** | `MachineGuid` fingerprint, `sysinfo`, the `powershell.exe` CIM GPU query, the WSL `bash.exe` launcher and the Store `python3` stub. |
| Probe spec editor | **MOD-51** | Milestone 2 ships only the read-only view. |
| Terminal replies of panicked runtime tasks; wide characters in the text widgets | **MOD-53**, **MOD-54** | Milestone 2 review deferrals. |
| Two claim-time test gaps | **MOD-58** | Milestone 3 review. |
| Interactive path picker | **MOD-49** | Unblocked by milestone 4; the typed fallback is Hierarchy's `b`. |
| A manual row landing mid-pass is not added to `held` | accepted | Best effort: the manual row itself is never overwritten (the conflict clause answers `AlreadySet`), but its path is not held, so another repo's match in the same pass could still take it. `b` corrects it. |
| A caller's own notes in `excerpts_for` skip `withhold_unmaskable_notes` | accepted | They are engine-authored and carry no filesystem text. |
| The judge prompt carries no excerpts | accepted | D109, above. |
| Tier 2 excerpts (`changed_paths`, the previous attempt's diff) | not opened | `DiffBlock` carries no repo-qualified path list; the PRD does not ask for it (D122). |
| Claim refused on `reclaim` does not reach the status line; PgStore reads `required_tags` unlocked | accepted | Milestone 3's known gaps. |
| `a_merge_dropped_mid_hook_still_lands_and_leaves_no_merge_head` flakes under load | **CLEAN-5** | Pre-existing (MOD-4, `62777f4`), seen at milestone 4's gate; not caused by MOD-7. |

---

## Validation

Milestone 4's workspace gate on the merged tree, with `USERNAME=htui-ci` and
`HTUI_TEST_DATABASE_URL` set: `cargo test --workspace --all-features -- --test-threads=1` 2262
passed, 0 failed; `cargo clippy --workspace --all-features --all-targets -- -D warnings` clean;
`cargo sqlx prepare --check -- --all-targets --all-features` from `crates/htui-store` against a
scratch database migrated through `0007` passes; `cargo doc --workspace --no-deps --keep-going`
shows exactly the six baseline errors. After the review fixes: `htui-core` 378, `htui-agent` 425,
`htui-orch` lib 441, `htui` 778 tests, all 0 failed. Milestones 1-3 recorded their gates in their
plans; each was green at `--test-threads=1` when it merged.

## Live coordinates

Kept in `HANDOFF.md`'s "Live coordinates" line: store `CASES` 77, `READ_CASES` 14, `htui-orch`
`CASES` 72, `StoreRequest`/`StoreReply` 69/40, 268 `.sqlx` files, 88 `crates/htui/tests/snapshots`,
next migration `0008`, seven Settings sections.
