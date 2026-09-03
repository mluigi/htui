# Routing — /handoff-run

How a HANDOFF item is routed to one of three paths. Item semantics:
`.claude/rules/workflow-docs.md`. The decision table below is the routing law itself —
self-contained, no external artifact needed.

## Paths

| Path | Chain | Produces |
|---|---|---|
| **ANA** | research → `docs/ANA-N.md` (survey of prior art, options, verdict, phasing) | Analysis doc only. **Never implementation** — a concluding verdict may spawn a `MOD-N`, which is a new item routed on its own. |
| **PRD** | `plan-prd` → `.claude/prds/<name>.prd.md` → `plan` → `.claude/plans/<name>.plan.md` → CONFIRM → implementation | Full artifact chain for big/underspecified work |
| **plan** | `plan` → `.claude/plans/<name>.plan.md` → CONFIRM → implementation | Direct plan for small/well-specified work |

## Decision table

1. `ANA-N` → **ANA path** — *when the item is actually a research/design question*. If the
   item text is implementation work wearing an ANA prefix (verbs like "migrate", "port",
   "implement"; no open question to conclude), that is a **prefix mismatch**: flag it in
   the verdict, recommend re-minting as `MOD-N` per `lifecycle.md` P0 (noting "from
   ANA-N", old line dropped — IDs never reused), and route the re-minted item by the
   table below. Maintainer confirms the re-mint like any other verdict. (Example: an item
   `ANA-N - Migrate <file> to <module>` is implementation wearing an ANA prefix — re-mint
   as MOD.)
2. Any other prefix (`MOD`/`NEXT`/`VAL`/`TOOL`/`CLEAN`): evaluate four criteria **from the
   item text alone** (no code scan needed to route):

   | # | Criterion | Fires when |
   |---|---|---|
   | C1 | Cross-repo reach | Item touches more than one repo, or a repo + the vcpkg registry |
   | C2 | New public API / architecture surface | New library, new public headers, new module seam, engine-structure decision |
   | C3 | Unresolved design questions in the item text | Explicit open questions, "TBD", "check whether", options listed but not chosen |
   | C4 | Estimated breadth > ~10 files | From the item's own description of scope |

   **≥2 fire → PRD path. Else → plan path.**
3. `CLEAN-N` / `TOOL-N` / `VAL-N` default to plan path; the ≥2 rule can still promote them
   (rare by construction — these prefixes describe bounded work).

## Verdict output format

Always shown, **before any execution**:

```
Item:      <PREFIX-N> — <title>
Path:      ANA | PRD | plan
Criteria:  C1 ✗  C2 ✓ (new public mount API)  C3 ✓ (backend choice open)  C4 ✗   → 2 fired
Ultracode: recommended (<phase> — <which trigger>[; <phase> — <trigger>…]) | not needed
Reasoning: <one or two sentences>
```

**Ultracode line** — recommend dynamic multi-agent orchestration (one deterministic
Workflow-tool script instead of hand-driven agent calls) for any chain phase whose work
decomposes into independent agent tasks. Name each qualifying phase in the line with its
trigger. Phase triggers:

| Phase | Recommend when |
|---|---|
| **research** (ANA path) | The question spans multiple repos, backends, or source families — multi-modal sweep (per-repo/per-backend readers + web survey in parallel). The ANA doc itself is still written by one agent from the sweep's returns. |
| **implement** (PRD/plan paths) | **C4 fired** (breadth > ~10 files), **≥3 criteria fired**, or the item text asks for an exhaustive sweep/audit/migration. |
| **review** | The change set is wide enough that reviewer findings warrant parallel adversarial verification (one verifier per finding). Never replaces the reviewer gate — the configured reviewer still runs; the workflow only verifies its findings. |

No phase qualifies → `not needed`. A phase qualifies only when a scripted workflow is
actually cheaper and safer than driving the same agents by hand — a handful of sequential
tasks is not a workflow, and orchestration is never permission to skip tests or gates.
The maintainer's route-accept covers the recommendation; they can strike it ("no
ultracode"), force it ("with ultracode"), or scope it ("ultracode for implement only") on
any item regardless of triggers.

- `--dry-run`: stop here. Print verdict, nothing executes.
- Otherwise: **wait for the maintainer.** Accept = proceed. Override = maintainer replies
  with the path to use (e.g. "route as plan") — the override is taken without argument and
  noted in the eventual plan/PRD artifact ("routed as X by maintainer override").
- Low-confidence verdicts (criteria count exactly at threshold, or item text too thin to
  judge C2/C4) must say so explicitly and lean on the maintainer's call.

## Guardrails

- Routing never skips downstream gates: `plan` CONFIRM, PRD open questions, deliberate
  push policy all remain (maintainer present at every gate, per PRD).
- A misroute discovered mid-path is not sunk cost: stop, re-route, keep any artifact that
  is still valid (a PRD produced for what turns out to be a plan-path item is just extra
  context, not waste).
