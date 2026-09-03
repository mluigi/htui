---
name: handoff-add
description: "Create a HANDOFF.md item from a one-line description: correct prefix chosen (ANA/MOD/NEXT/VAL/TOOL/CLEAN), ID minted with the owned-ID method, checklist line under the right section, origin/blocked-on cross-links, summary table recount, validator green. Trigger: /handoff-add <description> [--dry-run]."
---

# /handoff-add

One command to mint a HANDOFF item correctly. The verdict gate below stays interactive.

## Usage

```
/handoff-add Port SlangCompiler donor seam to engine shader module
/handoff-add check whether format-v3 chunking helps archive mounts --dry-run
```

## References

- `../handoff-run/references/lifecycle.md` — P0 open-item procedure (ID mint, placement, recount, cross-links)
- `../handoff-run/scripts/next-item-id.sh` / `.ps1` — the owned-ID mint, run at step 3 (never mint by eye)
- `../handoff-run/scripts/validate-workflow-docs.sh` / `.ps1` — structural validator, run after writing
- Law: `.claude/rules/workflow-docs.md` (prefix table, section table, lifecycle rules). This skill is an operational
  checklist over the law; it does not restate it — on any conflict, the rule wins.

**On Windows run the `.ps1` of each script above, never the `.sh`** — the variants are behaviourally
identical and bash subprocesses stall or die on fork on this box (`0xC0000142`); through a git hook
that failure looks like a hang, not an error (`handoff-run/SKILL.md` References carries the
measurement).

## Flow

### 1. Locate

Read the **current repo's** root `HANDOFF.md` (the repo the session runs in). IDs are per-repo — never mint or resolve
against another repo's HANDOFF. Missing `HANDOFF.md` → stop and report; this skill never bootstraps the file.

### 2. Prefix verdict

Choose the prefix from the description using the law's prefix table. Heuristics:

| Signal in description | Prefix |
|---|---|
| "check whether", "evaluate", "survey", "feasibility", open question to conclude before code | `ANA` |
| "implement", "add", "port", "migrate", "extract", feature/behavior change | `MOD` |
| chapter/milestone-parity follow-up | `NEXT` |
| VUID / validation-layer finding | `VAL` |
| build script / tooling finding, not engine code | `TOOL` |
| cleanup / dead code / refactor, explicitly no behavior change | `CLEAN` |

Ambiguous (e.g. "check X then port it" mixes ANA and MOD) → pick the primary, name the runner-up in the verdict, lean on
the maintainer's call.

### 3. Mint the ID

Run the mint, never derive it by eye:

```
bash .claude/skills/handoff-run/scripts/next-item-id.sh --prefix <PREFIX>   # macOS/Linux
pwsh .claude/skills/handoff-run/scripts/next-item-id.ps1 -Prefix <PREFIX>   # Windows
```

Its `max open` / `max archived` / `next` feed the verdict line below verbatim. **Non-zero exit blocks the mint** — the
ID space is untrustworthy until the reported finding is fixed; report it and stop rather than minting over it.

The rule it implements (owned-ID method, `../handoff-run/references/lifecycle.md` P0): max over open checklist lines
(`^- \[ \] \*\*PREFIX-N`) in `HANDOFF.md` **plus** the archive — index lines (`^- \*\*\[PREFIX-N\]`) in
`DECISIONS.md`, cross-checked against the `docs/decisions/<prefix>/` listing — then +1. Never
grep raw `PREFIX-N` mentions: cross-repo references pollute the count. IDs are never reused.

### 4. Place

Pick the section by matching the target HANDOFF's **existing** headers against the law's section table — repos differ in
which sections they carry and some use local names (e.g. a single "Open items" section). Never invent a new section when
an existing one fits the prefix; if none fits, propose the law-table section name in the verdict and add it on accept.

### 5. Draft

Checklist line: `- [ ] **PREFIX-N - Title.** body`. Title from the description, body carries the rest. When the
description names an origin ("from ANA-N") or a blocker ("blocked on <repo> <ID>"), include the note and prepare the
**two-way** cross-links per law rule 5 — the other entry (this repo or the named repo's HANDOFF) gets the matching
pointer.

### 6. Verdict gate

Print, before any write:

```
Item:      <PREFIX-N> — <title>
Prefix:    <chosen> (runner-up: <alt> — <one-line reason>, if ambiguous)
ID mint:   max open <a> / archived <b> → <PREFIX-N>
Section:   <existing header matched | proposed new section from law table>
Line:      <the drafted checklist line>
Links:     <cross-link edits to make elsewhere, or none>
```

- `--dry-run` → stop here. Nothing writes.
- Otherwise **wait for the maintainer**: accept, or override any field (prefix, title, section, links). Never write on
  an unconfirmed verdict.

### 7. Write

1. Insert the checklist line under the chosen section.
2. Apply cross-link edits (both directions, including the other repo's `HANDOFF.md` when named).
3. Recount the summary table; add the prefix's row if the table lacks it.

### 8. Validate & report

Run `bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh` (macOS/Linux) or
`pwsh .claude/skills/handoff-run/scripts/validate-workflow-docs.ps1` (Windows) from the repo root. **Non-zero exit
blocks the done-report** — fix findings first. Then report: minted ID, section, cross-links applied, validator result. No commit or
push side effects — bookkeeping rides with the session's work under the deliberate-push policy.

## Hard rules

- Never reuse an ID; never mint against another repo's HANDOFF.
- No write on an unconfirmed verdict; no done-report over a red validator.
- This skill orchestrates; `workflow-docs.md` wins on any bookkeeping conflict.
