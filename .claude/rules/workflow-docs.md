---
paths:
  - "HANDOFF.md"
  - "**/HANDOFF.md"
  - "DECISIONS.md"
  - "**/DECISIONS.md"
  - "**/docs/ANA-*.md"
  - "docs/decisions/**/*.md"
  - "**/docs/decisions/**/*.md"
---

# Workflow Docs Rule

Governs `HANDOFF.md`, `DECISIONS.md`, `docs/decisions/**`, and `docs/ANA-*.md` in this repo. Read
this before touching any of them.

## Files

- **`HANDOFF.md`** (repo root) - outstanding work only. Open items live here as checklist entries
  grouped by section (Next features / Analyses / Deferred backlog / Runtime validation findings /
  Tooling findings). Ends with a summary table counting open items per section.
- **`DECISIONS.md`** (repo root) - archive **index**, not the archive itself. One line per resolved
  item, newest first, carrying ID, title, status, date and a link to the write-up. This is still the
  file a session opens to find history; it is now a table of contents rather than the contents.
  Format below.
- **`docs/decisions/<prefix>/<prefix>-N.md`** - the write-up itself, one file per resolved item.
  Lowercase prefix directory, lowercase filename: `MOD-7` lives at `docs/decisions/mod/mod-7.md`,
  `ANA-9` at `docs/decisions/ana/ana-9.md`. Holds what was decided/built, why, commit hashes.
  `HANDOFF.md` keeps only a one-line recap pointer.
- **`docs/ANA-N.md`** - one file per analysis, the actual research/design doc (survey of prior art,
  options considered, verdict, phasing). `HANDOFF.md`/`DECISIONS.md` only ever summarize an ANA;
  the detail stays in the `docs/ANA-N.md` file itself and isn't duplicated.
- **`docs/REQUIREMENTS.md`** (htui only) - product requirements with stable `R-<AREA>-<N>` IDs.
  Sits above every ANA and MOD: an ANA decides how a requirement is met, a MOD cites the IDs it
  satisfies. Edited only by explicit maintainer decision, never at item close-out.

## Item ID prefixes

| Prefix    | Meaning                                                                                | Lives in                                       |
|-----------|----------------------------------------------------------------------------------------|------------------------------------------------|
| `ANA-N`   | Analysis / design doc - research before code, no implementation until concluded        | `docs/ANA-N.md`, tracked in HANDOFF "Analyses" |
| `MOD-N`   | Modification / feature work - implementation task, often spawned by a concluded ANA    | HANDOFF "Next features" / "Deferred backlog"   |
| `NEXT-N`  | Chapter/milestone-parity follow-up item                                                | HANDOFF "Next features"                        |
| `VAL-N`   | Runtime validation finding (VUID / validation-layer error caught via `run-target.ps1`) | HANDOFF "Runtime validation findings"          |
| `TOOL-N`  | Tooling/build-script finding (not engine code)                                         | HANDOFF "Tooling findings"                     |
| `CLEAN-N` | Cleanup / dead-code / refactor item, no behavior change                                | HANDOFF "Deferred backlog"                     |

IDs are never reused. Next ID = max **owned** ID for that prefix + 1, taken over two sources and no
others:

1. open checklist lines in `HANDOFF.md` - `^- \[ \] \*\*PREFIX-N`;
2. the archive - index lines in `DECISIONS.md` (see format below), cross-checked against the
   `docs/decisions/<prefix>/` directory listing.

**Never** mint by grepping raw `PREFIX-N` mentions. Cross-repo references (`engine MOD-11`,
`vfs MOD-1`) are mentions of *other repos'* items and inflate the count, which mints a duplicate -
and reuse is the one thing this rule never permits.

The mint is **purely file-derived and offline**. It reads the working tree and nothing else: no
network source, no index service, no generated answer. A mint that cannot enumerate its inputs
deterministically is not a mint.

An item can spawn another: an `ANA-N` verdict commonly spawns a `MOD-N` implementation task (e.g.
ANA-11 to MOD-16) - note the origin in the `MOD-N` entry ("from ANA-11").

**No repo- or library-specific prefixes.** Every repo uses the *same* six prefixes from the table
above - `ANA`/`MOD`/`NEXT`/`VAL`/`TOOL`/`CLEAN`. Don't mint a new prefix named after the library or
feature being worked on (e.g. no `SET-N` for settings work, no `VFS-N` for vfs work) - a library
extraction or rewrite is just a `MOD-N` like any other implementation task; what it's about goes in
the title, not the prefix.

**ID scope is per-file, not global.** Each repo's `HANDOFF.md`/`DECISIONS.md` numbers its own items
starting at 1, independently. `MOD-1` in one repo's `HANDOFF.md` and `MOD-1` in another's are
unrelated items. Always disambiguate with the repo name when referencing an ID outside its own
HANDOFF.md (e.g. "VulkanTutorials MOD-16", "workspace MOD-1", "vfs MOD-1").

## Index line format

`DECISIONS.md` is a list of these, newest first, one per resolved item:

```
- **[MOD-7](docs/decisions/mod/mod-7.md)** - Lightweight DECISIONS.md + per-item decision files (done, 2026-07-29)
```

Five fields: **ID** (linked to its write-up), **title**, **status**, **date**. The link target is
always `docs/decisions/<lowercase prefix>/<lowercase prefix>-<N>.md` and must agree with the ID -
`MOD-7` never links `mod-6.md`.

Parsed by:

```
^- \*\*\[(?<id>(?:ANA|MOD|NEXT|VAL|TOOL|CLEAN)-\d+)\]\((?<path>docs/decisions/[a-z]+/[a-z]+-\d+\.md)\)\*\*\s*[-–—]\s*(?<title>.+?)\s*\((?<status>[^,()]+),\s*(?<date>\d{4}-\d{2}-\d{2})\)\s*$
```

The separator alternation accepts a hyphen, en dash or em dash: parts of this tree carry mojibake
from an earlier encoding pass, and an item must not fall out of the ID space because its dash got
mangled. A line that does not parse is an **error**, never a skip - silently dropping a line drops
its ID, and a dropped ID gets minted twice.

The write-up file opens with the same heading the section used to carry, promoted to `#` because it
is now a file title: `# PREFIX-N - Title (status, YYYY-MM-DD)`.

## Lifecycle

1. New work starts as a checklist line (`- [ ] **PREFIX-N - Title.** ...`) under the right
   HANDOFF.md section.
2. Non-trivial features/analyses get a saved plan under `.claude/plans/` and, for analyses, the
   full doc under `docs/ANA-N.md`.
3. On completion: delete the checklist line from HANDOFF.md, write the detailed writeup + commit
   hash to `docs/decisions/<prefix>/<prefix>-N.md`, prepend its index line to the top of
   DECISIONS.md (reverse-chronological, newest first), and update HANDOFF's summary table + top
   status line. The status line is a recap, not an archive - cap it at the most recent handful of
   completions (roughly 2-3); when a new one lands, drop the oldest mention rather than appending.
   The index and the write-up files stay uncapped - together they are the full permanent record.
4. Multi-phase work (e.g. MOD-16) stays as a single open HANDOFF.md checklist line, appending a
   `**Phase N landed (commit)**` note per phase, until every phase is done - then it archives as
   **one** write-up file and **one** index line covering all phases.
5. Cross-repo dependents: this repo references another HANDOFF.md by relative path (e.g.
   `../HANDOFF.md`, `../vfs/HANDOFF.md`) when an item is blocked on work that moved to the owning
   repo - keep that pointer current rather than duplicating the other repo's item. Cross-link both
   ways: the dependent entry notes "blocked on `<repo> <PREFIX-N>` (`<path>/HANDOFF.md`)", the
   owning entry notes "originates from `<repo> <PREFIX-N>`".
6. Live coordinates: a completed item may still hold values an open item depends on (registry
   baselines, port REFs, pinned hashes). Keep those in the `HANDOFF.md` recap line rather than
   burying them in the archive - sessions read `HANDOFF.md` first.

## Session start

Read `HANDOFF.md` top-of-file status line + "Open items" first, pick the next item. For historical
context on a specific item, read `DECISIONS.md` (the index) to find it, then open only that item's
`docs/decisions/<prefix>/<prefix>-N.md` - or `docs/ANA-N.md` for an analysis. Reading one item's
history costs the index plus one file; it never costs the whole archive.
