# MOD-26 - Declarative Agent Personas (done, 2026-10-03)

**Requirements:** `R-AGT-4`, `R-AGT-8` (quota fallback: a persona never sets the model), `R-ID-3`
(the row is the truth), `R-ID-5`, `R-ID-6` (no LLM in the import), `R-PRM-1`, `R-PRM-3`, `R-HIS-1`
(a started run keeps its personas), `R-NF-3` (reads, writes and file reads on the store worker),
`R-STO-5` (forward-only migrations), `R-TUI-*` (the Settings section).
**Origin:** ANA-13 (`docs/ANA-13.md` §3.1, §4 item 1), amended by ANA-16 (`docs/ANA-16.md` §6.2,
§8: registry rows, not a per-box `~/.config/htui/agents.d/`) and ANA-27 (`docs/ANA-27.md` §5.1 T9:
narrow-only, fixed precedence, body inlined). Consumer: MOD-27 (swarm `handoff` to a named
persona).
**Artifacts:**
- PRD [`.claude/prds/mod-26-agent-personas.prd.md`](../../../.claude/prds/mod-26-agent-personas.prd.md): two milestones, gate decisions Q-storage, Q-model, Q-binding, Q-authoring, Q-seeds, Q-enforcement;
- milestone 1 plan `.claude/plans/mod-26-agent-personas.plan.md` (T0-T5: D1-D13, I-1-I-7, OQ-1-OQ-6) and its blueprint `.claude/plans/mod-26-agent-personas.blueprint.md`;
- milestone 2 plan `.claude/plans/mod-26-m2-persona-authoring.plan.md` (T6-T10: D14-D23, I-8-I-12, OQ-7-OQ-12) and its blueprint `.claude/plans/mod-26-m2-persona-authoring.blueprint.md` (findings F-1-F-18, decisions B-1-B-15, deviations D-1-D-11);
- operator guide `docs/personas.md`.

Decision numbers are local to MOD-26 (the MOD-31 convention). Milestone 2 continues milestone 1's
numbering (D14…, I-8…, OQ-7…, T6…), so commit subjects never collide.

Routed as **PRD** (C2, C3 and C4 fired). Ultracode was accepted for the implement phase. Both
milestones ran in the TOOL-7 sandbox `hr/MOD-26`.

**Decisions (maintainer):**
- 2026-10-02, PRD gate: personas are **Postgres registry rows** (Q-storage), carry **no model**
  (Q-model), are bound on the **step-graph phase** (Q-binding, revised at `/plan` from the phase
  candidate: `phase_agent` has no writer, HANDOFF R-6, and rung 2/3 runs have no `phase_agent`
  row), are authored in a **Settings tab plus a `.md` import** (Q-authoring), are seeded as
  `reviewer` and `architect` (Q-seeds), and are enforced on **both layers**, transport flags where
  supported and the relay for all (Q-enforcement);
- 2026-10-02, milestone 1 plan confirmed, OQ-1 to OQ-6 as recommended;
- 2026-10-02, milestone 1 review: R1 fixes applied, N4 and N5 deferred to milestone 2;
- 2026-10-02, milestone 2 plan confirmed, OQ-7 to OQ-12 as recommended;
- 2026-10-03, milestone 2 review: M-1, L-1-L-4 and N-2 fixed; N-1 (long functions) not done; N-2
  (`~/` expansion) applied to the persona import only.

**Commits:**
- milestone 1 (2026-10-02), `dc5ba263`..`d1871f5e`, merged to `main` in `6df86cf8`:
  - T0 parser, save rules, seeds and freeze: `dc5ba263`, `c309c181`, `832447b2`, `0d912369`;
  - T1 store (persona rows, writers, phase binding, `0012`): `fcb95350`, `55372a38`, `bd6a3ca4`,
    `8acc4fc4`, `b3ef112f`, merged `84713618`;
  - T2 `narrow`, CLI tool names, ACP fs refusal: `69314fa7`, `1f889d12`, merged `736fea63`;
  - T3 persona prompt frame: `89973089`, `b98704bc`, merged `8d24af29`;
  - T4 engine (freeze at `StartRun`, stage-3 lookup, narrowing): `d03a6975`, `463e676f`,
    `eaa68b9d`;
  - T5 docs and counts: `a27fd8bf`;
  - review R1: `41d964cd` (L1), `cd4ea1d1` (L3), `2a34ad01`, `d1871f5e` (N1), `ecc33a3c` (N2),
    `1f53aeb9` (N3), `dc8500ed` (N7), `c1730fe4` (N6), `0604a595` (N8), `e52b59a5` (I-5, I-6
    named), `8ffcf463` (L2, L4 documented), `359f2bfa`, `ba03872d` (PRD review record).
- milestone 2 (2026-10-02..03), `ac8d0c3a`..`3b7b6297`:
  - plan: `ac8d0c3a` (draft), `257729f2` (fact-checked), `cb69a011` (confirmed); blueprint
    `5ffd271f`;
  - T6 core: `cd3de2ff` (N4), `5be8867f` (closed rule kinds), `6cd50a13` (rule lines), `cd1b899b`
    (import mode), `8884a86d`, `ebddc56a`, `6898e469` (D-10);
  - T7 store: `cad373ae` (`delete_persona`), `7c7e78c2` (`0013`, N5), `f4f9e9bc`, `410e671f`
    (D-11);
  - T8 Kinds `persona` field: `622892a1`, `1f4c18a2`, merged `16555513`;
  - T9 Settings › Personas and the import: `7ce21e4d`, `01caf67e`, `faf7eafd`, `a0d103bd`,
    `55d21ae5`, `c9022af9`, `0e1bc14f`, `757b448c`;
  - review R1: `2aedd54c` (L-4), `9fca4b56` (M-1), `d5348fa2` (L-1), `a3620ce3` (N-2), `10df35e0`
    (L-2), `0f175aa2` (L-3), `3b7b6297` (ADV-1, the R1 verifier's finding);
  - T10 docs, pins and this close-out: the commit that adds this file.

---

## What was built

### Milestone 1 - registry rows applied by the engine

- **Rows.** A global `persona` table (`0012_persona.sql`, like skills: no project, unique name)
  holds a name, a description, a prompt body, `tools {allow, deny, deny_kinds, command_run}` and
  `permission {default, rules}`. There is no model key. A nullable `step_graph_phase.persona_id`
  (`ON DELETE RESTRICT`) binds one persona to a phase. It applies to whichever candidate wins the
  phase, on any rung.
- **Save rules (D3).** Both stores refuse, with one shared sentence each, a bad name, a NUL, a
  blank body, a malformed tool name, an `mcp__` entry in `allow`, a `deny_kinds` entry outside the
  seven narrowable kinds, an empty rule match and a NUL in a rule. Every shape refuses unknown keys
  (I-2). A persona cannot be saved if it would widen anything.
- **File format and seeds (D7, D8).** The `.claude/agents/*.md` shape: a flat frontmatter of seven
  keys (`name`, `description`, `tools`, `disallowed-tools`, `deny-kinds`, `command-run`,
  `permission-default`), then the body. `model` is refused with its own sentence (I-5). The
  `reviewer` and `architect` seeds are compiled in, deny `edit`/`delete`/`move`, keep `execute`,
  and are topped up by name on every connect (`ON CONFLICT (name) DO NOTHING`).
- **Freeze (D9, I-3).** At `StartRun` every persona the graph names is frozen into the run
  snapshot by name (`phases[].persona`, `personas[] {name, digest, body, tools, permission}`).
  Edits never change a started run. The persona name is part of the topology digest. A phase whose
  snapshot lacks its persona fails before a token (I-4).
- **Prompt (D13, OQ-3).** The body renders first, as a protected, scrubbed
  `<section name="persona">` before the template (`sections[1]`). A template cannot place it.
  Persona-less prompts are byte-identical (I-7).
- **Narrowing (D10, D11, OQ-1).** `htui_agent::persona::narrow` only removes. Precedence is "agent
  row, then the persona narrows; the model is the phase candidate's". This replaces ANA-27's
  wording, which had a model rung. On `claude-cli`, `allow` becomes `--tools=` and `deny` plus the
  inverted kinds become `--disallowedTools=`; htui never emits `--allowedTools`, which
  auto-approves. On ACP, the relay rejects denied kinds and htui's `fs/*` handlers refuse denied
  reads and writes. The residual gaps (the shell, ACP calls made without asking, operator
  `extra_args`) are documented in `docs/personas.md`, not hidden.
- **Scope (OQ-2, OQ-5, OQ-6).** Engine steps only: the fan-out judge, promoted steps and chat run
  without the persona. There is no per-step "layers in force" record: the layers are a fixed
  function of the transport.

### Milestone 2 - authoring in the TUI

- **Settings › Personas (D21, D22).** The eighth Settings section, registered last. It lists one
  line per persona and binds `j k n e b r d I`, with no reload key (B-12).
  - `n`/`e` open seven fields in the frontmatter spellings; lists split with the file grammar's
    `list_of` (B-3).
  - `b` and `r` open embedded `TextArea` editors: `Ctrl+S` saves, and `Esc` warns once over unsaved
    text.
  - Every save is one compare-and-set `UpdatePersona` carrying only the changed fields. `Stale`
    rebases the untouched ones, and `Gone` closes the editor.
  - Every refusal the form shows is the store's own sentence (I-8). The worker sends a
    `Constraint` back bare (B-11, D-9).
  - Personas are not mirrored, so offline the section shows the unreachable sentence and offers
    only navigation.
  - Five requests (`Personas`, `CreatePersona`, `UpdatePersona`, `DeletePersona`,
    `ImportPersonas`) are served by `htui/src/persona_settings.rs`. Three replies come back
    (`Personas`, `PersonaWritten` with a self-naming `PersonaWrite`, `PersonaImports`).
- **Delete (D14, I-9, N5).** `WriteStore::delete_persona` refuses a bound persona with
  `persona_is_bound`'s sentence, naming up to five `<project>/<graph>/<phase>` holders sorted on
  the triple (B-4), then "and *n* more". Override graphs count.
  - On Postgres the delete locks the row `FOR UPDATE` and runs a guarded `DELETE … NOT EXISTS`.
    A bind that commits first gets the refusal, and a bind that waits gets `references_no_row`.
  - Migration `0013_persona_phase_index.sql` indexes `step_graph_phase.persona_id` (index only; no
    cache migration, no column comment).
- **Closed rule kinds (D18).** A rule's `tool_kind` must be one of the ten ACP kinds. The check is a
  second pass in `permission_refusal`, so every milestone 1 sentence keeps its precedence (B-1,
  D-1). Both stores and the rules editor refuse a misspelt kind.
- **Rule lines (D17, OQ-10).** `parse_rules`/`format_rules` are pure and round-trip, one rule per
  line: `<answer> key=value… [# reason]`, with quoted values and five escapes. Other control
  characters are written raw inside quotes (B-2, D-7).
- **Import (D19, D20, OQ-7-OQ-9).** `I` takes a file, or a directory whose depth-0 `*.md` files are
  each one persona; the store worker reads them (I-11). `parse_import` drops `mcp__` entries from
  `tools` and names them, and it refuses a `tools` that held only `mcp__` entries (that would widen
  to every built-in tool, R-13). The walk itself:
  - a fence-less file is skipped;
  - a known name is refused, never overwritten (OQ-8), and a name met twice in one import is refused
    the second time;
  - the caps are 256 KiB per file (held during the read), 64 files and 256 examined entries per
    directory;
  - a per-file report opens whenever anything was refused, skipped or dropped. No path is stored
    (I-10).
- **Kinds `persona` field (D23, OQ-11).** The phase edit form gains a seventh field, `persona`,
  typed by name. Blank clears it. A known name binds. An unknown name is refused before sending,
  listing the known names. An untouched field sends nothing (B-8: compared against the text it
  opened with). A bound phase's line shows `· persona <name>` (I-12). `CatalogueSnapshot` carries
  the persona list.
- **N4 (D16).** `GraphSnapshot::persona_for` returns a typed `PersonaNotInSnapshot`. Its `Display`
  is milestone 1's sentence byte for byte, and the persisted reasons are unchanged (B-15 pins the
  literal).

## Open questions as answered

- **OQ-1** tool vocabulary: `deny_kinds` everywhere plus agent-native names on `claude-cli` only.
- **OQ-2** judges run without the phase's persona.
- **OQ-3** the persona block is an htui frame before the template, protected.
- **OQ-4** seeds deny `edit`/`delete`/`move`, keep `execute`, carry no `allow`.
- **OQ-5** engine steps only (no chat, no promoted step).
- **OQ-6** no per-step enforcement record; the matrix is documented.
- **OQ-7** the import drops `mcp__` `tools` entries and names them, and refuses an all-`mcp__`
  `tools`; the save rule and the seed path are unchanged.
- **OQ-8** an import refuses a known name.
- **OQ-9** a file or a directory's depth-0 `*.md`; fence-less files skipped; the sweep is new logic,
  not the skill walk (`skill_import.rs` untouched).
- **OQ-10** the rules editor uses a one-line, round-tripping rule syntax.
- **OQ-11** the Kinds `persona` field is typed by name, on the edit form only.
- **OQ-12** no `$EDITOR`: the embedded `TextArea` only; `$EDITOR` comes with or after MOD-13
  milestone 4.

## Deviations from the plans

Milestone 1's are recorded in its plan's "Verified claims" and its blueprint. Milestone 2's are
from its blueprint (§0b):

- **D-1** the closed-kind check is a second pass (B-1), so no milestone 1 sentence changes.
- **D-2** `persona_import::import` returns `Result<Vec<PersonaOutcome>>`: offline and the one
  `personas()` read are errors.
- **D-3** every `PersonaWrite` variant names its row (`Created{id,name}`, `Updated{id,name}`,
  `Deleted{id}`, `Stale{id}`, `Gone{id}`), so a reply closes only its own editor.
- **D-4** the broken shape in Kinds was persona + budget (one `SetPhaseBudget`, persona dropped),
  not persona-only; both shapes are tested.
- **D-5** eight `kinds__*` snapshots existed, not seven; seven are unchanged.
- **D-6** only migration-count prose moved; the `TABLES` messages stay.
- **D-7** five escapes; other control characters raw inside quotes.
- **D-8** the T7 gate also built `htui-agent`'s tests (two `WriteStore` spies).
- **D-9** a store `Constraint` reaches the form as the bare sentence (B-11).
- **D-10** the closed-kind sentence uses `escape_debug`, so a backtick in a kind stays raw.
- **D-11** the Pg guard and the holders read are two READ COMMITTED snapshots. An unbind landing
  between them could empty the holders read, so the guard re-runs. Review L-3 then capped it at
  three passes, ending in `Backend("delete_persona: holders kept changing")` with nothing deleted.
  The SQL texts are byte-identical, so `.sqlx` did not move.

## Review

- **Milestone 1** (`rust-reviewer`, 2026-10-02): approve with fixes, 0 critical/high/medium.
  - Fixed in R1: L1 (`PhasePatch.persona` keeps its clear across serde), L3 (D5's clash-first
    order pinned on both phase writers), N1-N3, N6-N8.
  - Documented in `docs/personas.md`: L2 (renaming a persona, seed re-creation) and L4 (an
    `extra_args` `--disallowedTools` overrides the persona's).
  - Deferred to milestone 2, both closed there: N4 (typed refusal) and N5 (the index).
- **Milestone 2** (`rust-reviewer`, 2026-10-03): approve with fixes, 0 critical/high. Fixed in R1:
  - **M-1** an import that lands while an editor is open keeps its report until the editor closes;
  - **L-1** the byte cap holds during the read (a growing file or a FIFO), and a sweep examines at
    most 256 entries;
  - **L-2** a store lost mid-import stops the batch and keeps the report;
  - **L-3** `delete_persona`'s guard loop is capped at three passes;
  - **L-4** an untouched body holding a lone `\r` reads as unchanged;
  - **N-2** the persona import expands a leading `~/` (persona only, by maintainer decision);
  - **ADV-1**, found by the R1 verifier: L-2's fix had wrapped a lost store into an `Ok`, so the
    backend stayed online over a dead pool. The re-read error now stays typed, and the worker's
    `lost_the_store` check drops the backend offline (`3b7b6297`).
  - **N-1** (long functions) was not done, by maintainer decision.

## Moved counts

Milestone 1's column is its T5 count on the sandbox branch. Other items merged into `main` between
the milestones, so the `main` column is the base milestone 2 started from (`aeba8f63`). The last
column is a fresh count at close-out (2026-10-03, on `3b7b6297`), each read at its pin site.

| Pin | Milestone 1 base | After milestone 1 | `main` at `aeba8f63` | After milestone 2 |
|---|---|---|---|---|
| Store conformance `CASES` (`htui-core/tests/mem_store.rs`, `htui-store/tests/pg_conformance.rs`) | 119 | 124 | 130 | **133** |
| `READ_CASES` | 14 | 14 | 14 | **14** |
| `htui-orch` `CASES` | 86 | 91 | 92 | **92** |
| Migrations | 11 | 12 (`0012_persona`) | 12 | **13** (`0013_persona_phase_index`; next `0014`, cache next `0005`) |
| Postgres `TABLES` | 41 | 42 | 42 | **42** |
| Commented columns | 35 | 44 | 44 | **44** |
| `.sqlx` files | 307 | 315 | 318 | **321** |
| `crates/htui/tests/snapshots` | 128 | 128 | 134 | **142** (+1 `kinds__phase_persona`, +7 `personas__*`) |
| `StoreRequest` / `StoreReply` | - | - | 99 / 58 | **104 / 61** |
| `persona_settings::REQUEST_NAMES` | - | - | - | **5** |
| Settings sections / strip columns | 7 / 61 | 7 / 61 | 7 / 61 | **8 / 71** of 100 |
| `persona_writers_refuse_every_widening_shape` shapes | - | 17 | 17 | **18** |

Unchanged: `GraphSnapshot::V`, `RECORD_VERSION` 4, `Placeholder::ALL` 20, `MIRRORED_TABLES`,
workspace members, and every snapshot but `kinds__editor_phase` (I-12).

## Carried and not done

- **`$EDITOR` for the body and rules editors** (OQ-12): with or after MOD-13 milestone 4. MOD-13's
  HANDOFF entry notes that Settings › Personas would take the same hand-off.
- **`~/` in the skill import:** `skill_import.rs` still takes its path as typed. The persona import
  expands it (review N-2, persona only by maintainer decision).
- **PRD out of scope:** per-candidate binding (`phase_agent.persona_id`, after R-6's writer),
  personas on chat, promoted and judge steps, export to a file, swarm `handoff` (MOD-27), and
  change push to other boxes (MOD-48).
- **A superuser DSN for one test:** `pg_criteria.rs::a_persona_delete_gives_up_after_three_passes`
  (L-3) sets `session_replication_role`, which needs a superuser. That holds in the sandbox.
- **The PRD metric "persona applied end to end in one real run"** is a manual check on the host
  after collect: bind `reviewer` in Settings › Kinds, run one step, and read the step's prompt
  `sections[]` and its argv or relay record.
- **Review N-1:** the long functions the milestone 2 review named stay as they are.
