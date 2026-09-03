# Selection — `/handoff-run next`

How `next` resolves to exactly one item ID. Only used when the argument is the literal `next`; an
explicit `<ITEM-ID>` skips this file entirely and goes straight to `routing.md`.

Selection answers *which item*; routing answers *how to run it*. They are separate steps and the
maintainer confirms both (selection block, then routing verdict — see `SKILL.md` steps 1.5 and 2).

A maintainer is present, so an unresolved tie is a **question**, never a coin-flip. `next` never picks
by file order when a real choice exists — step 4 is where that choice is put to the maintainer.

**Who runs this file.** Steps 1–3 and the §5 block draft run in a Sonnet 5 subagent (`SKILL.md`
step 1.5) — bounded rule application, cheaper tier, no authority. Step 4's ask and every gate after
it belong to the session model on the main thread: the subagent flags the ambiguity (`Ask: yes`),
the main thread poses it. A subagent that resolves a §4 ambiguity itself has violated this file.

## 1. Candidate set

Every open checklist line in the **current repo's** root `HANDOFF.md` — `^- \[ \] \*\*PREFIX-N`,
the same pattern `next-item-id.sh`/`.ps1` uses. Never another repo's HANDOFF.

Empty candidate set → report "no open items" and stop.

## 2. Eligibility filter

Drop a candidate when either holds:

| # | Filter | Drops when |
|---|---|---|
| E1 | **Blocked** | Item body carries a "blocked on `<repo> <PREFIX-N>`" cross-link and that item is still open in the named repo's `HANDOFF.md` |
| E2 | **Owned elsewhere** | The work this item still needs done lives in another repo — the local line is now a pointer or an anchor, and nothing about it can be executed in this repo |

E1 needs the other repo's `HANDOFF.md` read. If that file is unreadable (submodule not checked out,
path stale), do **not** guess: keep the candidate and mark it `blocked?` — an unverifiable blocker
is an ambiguity for step 4, not a silent pass or a silent drop.

### E2 in practice — the anchor case

E2 is the one judgment call in this file, and the common shape is a **multi-phase coordination
anchor**: earlier phases landed in *this* repo, so the item looks in-flight and would win R1, but
the body's own `**Remaining:**` note hands the rest to another repo.

Worked example — workspace `MOD-4` (toolchain migration):

> Phases 1–3 landed here (registry ports, logging mingw-first, `din-settings` reflection flip).
> Body reads `**Remaining:** engine **MOD-10** (dedicated session)`. Engine `MOD-10` is open in
> `engine/HANDOFF.md`. → **E2 fires.** Every remaining action is an engine-repo action; selecting it
> here would route and plan work this repo cannot perform.

The test is **not** "does the item mention another repo" — cross-repo items mention other repos
constantly. It is: **is there a next action producible in this repo?** Landed phases do not create
one. Concretely:

| Body says | Verdict |
|---|---|
| `**Remaining:** <other-repo> <ID>` and nothing else outstanding locally | **E2 — drop.** Pure anchor |
| Remaining work names files/ports/docs in *this* repo, other repos only as dependents | **Keep.** Coordination that produces local changes is real work |
| Remaining work is split — some local, some in another repo | **Keep**, and say so in the selection block; the run scopes to the local part |
| Local remainder is only "wait for `<other-repo> <ID>`, then bump" | **E2 — drop.** Waiting is not a next action; the bump becomes one when that item closes |

An E2 drop is never silent and never final: it is reported with its pointer (below), and the item
stays open here because the anchor is exactly what carries the sequencing.

### Zero eligible

Every candidate dropped → print each open item, the filter that dropped it, **and where the work
actually lives** — the owning repo and its item ID, plus the command that reaches it
(`cd <repo> && /handoff-run next`; IDs are per-repo, so this repo's run can never route them).
A zero-eligible report without that pointer is incomplete: it says "nothing here" when the useful
answer is "not here — there".

Then stop. Do not fall back to running a blocked or elsewhere-owned item.

## 3. Ranking

Surviving candidates sort on these keys in order. First key that separates wins.

| Key | Rule | Why |
|---|---|---|
| R1 | **In flight** before not-started | An item with a `**Phase N landed**` note or a plan/PRD artifact named for the item already has direction and sunk work; finishing beats opening a front. Landed phases are *not* by themselves a local next action — E2 already ran and drops the anchor case |
| R2 | **Unblocks others** before standalone | An item another open item in this repo declares itself blocked on is on the critical path; among several, more dependents first |
| R3 | **Section order** per `workflow-docs.md` | Next features → Analyses → Deferred backlog → Runtime validation findings → Tooling findings. "Deferred backlog" is deferred by name — it loses to everything above it |
| R4 | **File order**, then lowest ID | Last resort only, and it never decides alone — see step 4 |

## 4. Ambiguity → ask

If two or more candidates are still tied after **R1–R3** — i.e. only R4 would separate them — that
is an ambiguity. Ask the maintainer with the tied candidates as options; do not pick by file order.

Also ask when:

- a candidate is marked `blocked?` by an unverifiable E1 link and it would otherwise be the winner;
- the item text is too thin to place it (no section, or a body that names no work);
- the top candidate is in flight (R1) but its existing artifact looks stale — a plan or PRD older
  than the last edit to the item — since resuming and restarting are materially different runs.

Question format: one line per candidate, `<PREFIX-N> — <title>` plus the one fact that made it a
contender (`in flight, Phase 1 landed` / `unblocks MOD-9` / `Tooling findings, top of section`).

## 5. Selection block

Print before the routing verdict, always, including when only one candidate survived:

```
Selected:  <PREFIX-N> — <title>
From:      <n> open, <m> eligible
Because:   R1 in flight (Phase 1 landed 2026-07-25)   # the key that decided it
Runners-up: <PREFIX-N> (<why it lost>), <PREFIX-N> (...)   | none
Ineligible: <PREFIX-N> (blocked on vfs MOD-2), ...        | none
```

Zero eligible → same block, `Selected: none`, no routing verdict (nothing to route), plus the
`Work lives in:` pointer required above:

```
Selected:      none
From:          1 open, 0 eligible
Ineligible:    MOD-4 (E2 — remaining work is engine MOD-10, open at engine/HANDOFF.md)
Work lives in: engine — MOD-10 and 5 others; reach it with `cd engine && /handoff-run next`
```

Then continue to step 2 of `SKILL.md` — routing runs on the selected item unchanged, and its
verdict block is printed and confirmed as usual. The maintainer may reject the selection and name a
different ID; that override is taken without argument, exactly like a routing override.

`--dry-run` with `next` prints the selection block **and** the routing verdict, then stops.
