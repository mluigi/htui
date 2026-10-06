# MOD-55 — Ask an agent for help while editing a template or skill

> Routed as **PRD** by `/handoff-run MOD-55` (criteria C2 and C3 fired, C4 undecided from the item
> text, so exactly at threshold). Ultracode not recommended; the maintainer accepted. Spawned by
> MOD-9's PRD gate (2026-09-25, `.claude/prds/mod-9-skill-library-templates.prd.md` D2,
> `docs/decisions/mod/mod-9.md`). The contract is `R-SKL-3` and `R-PRM-4`, bounded by `R-ID-5`,
> `R-ID-7`, `R-SEC-3` and `R-HIS-1`.

## Problem

The maintainer writes and tunes htui's prompt templates and skill bodies in the editors MOD-9
shipped: the in-app `TextArea` and the `$EDITOR` handoff in the Skills tab. When a template body
fails `parse` (an unknown placeholder, a missing `{{item}}`), or does not make the agent behave as
wanted, the fix has to be worked out by hand against the placeholder contract. A new skill starts
from a blank page. Today's workaround is to copy the body and placeholder table into a separate chat,
paste the reply back and re-run the gate. That means leaving the editor, rebuilding the context by
hand, comparing versions without a diff, and sending whatever secrets the body holds to an outside
tool with no scrubbing.

## Evidence

- Assumption — needs validation by prototype: use the action on the next several template and skill
  edits and compare against the copy-into-chat workflow.
- The need was recognised at MOD-9's PRD gate (2026-09-25), which split it off as this item rather
  than leave it out entirely. No reason for the split is recorded beyond scope.

## Users

- **Primary**: the htui maintainer, editing a template or skill body in the Skills tab. The need arises
  when the body fails the gate, misbehaves in a run, or is being written from nothing.
- **Not for**: agents. No agent edits or saves a template or skill (`R-ID-5`: htui owns every prompt
  and skill text; `R-ID-6`: no LLM agent in a bookkeeping path). Not for bulk or unattended rewriting.

## Hypothesis

We believe an **in-editor "ask an agent" action that returns a diffed proposed edit, saved only
through the existing gate,** will **remove the copy-into-chat workaround** for **the maintainer
editing templates and skills**.
We'll know we're right when, **over the next 10 template or skill edits that use the action, most
proposals are accepted into the buffer (as-is or tweaked) without leaving the editor, no proposal is
saved past a failing gate, and no body leaves the machine with a secret unscrubbed**.

## Success Metrics

| Metric | Target | How measured |
|---|---|---|
| Proposals accepted into the buffer | > 50% of the first 10 uses | Recorded help runs (D2), accept vs discard |
| Proposals saved past a failing gate | 0 | Save path: the gate is the only way to persist (tests) |
| Bodies sent with a known secret value or a key-pattern hit | 0 | Scrub-before-send refusal (D3), covered by tests |
| Edits that still needed the copy-into-chat workaround | TBD — needs validation by the maintainer's self-report after the 10 uses | Maintainer note in the close-out write-up |

## Scope

**MVP** — One action in both Skills-tab editors (Templates and Library), registered as a named action
in MOD-67's catalogue. The maintainer types a request. The body being edited, the role's placeholder
table (templates only; skills have none) and the request are scrubbed, then sent to a registered agent
in a single turn. The reply is shown as a diff against the current body, and the maintainer accepts
it into the editor buffer or discards it. Nothing is saved by the action: persisting stays the normal
save (`Ctrl+S`) with its gate, which is `template::parse` for templates and the existing
blank/NUL/unchanged check for skills.

**Out of scope**
- Triggering the action from inside `$EDITOR` or MOD-57's embedded editor pane: the action works on
  the buffer once the external editor has returned.
- Multi-turn conversation, or refining a proposal with follow-ups: one request, one proposal.
- The agent saving, versioning or binding anything itself (`R-ID-5`).
- A placeholder parse for skills: skills have no placeholder contract today, and adding one is a
  skill-model change, not editor help.
- A new non-streaming, one-shot CLI agent API: the existing driver is used (D1).
- Applying part of a proposal (accepting individual hunks): the diff is accepted or discarded whole,
  and the maintainer can still edit the buffer afterwards.

## Decisions taken at the PRD gate

| # | Question (from the item text) | Decision |
|---|---|---|
| D1 | Which agent and model answer: the chat driver or a one-shot CLI call | **A registry agent, one turn.** Any agent from Settings > Agents, through the existing driver and its transport (ACP or CLI), using that agent's default model, chosen when the request is made. The session is started, sent one prompt and closed after the final reply. No new transport and no single-call API. |
| D2 | Whether the exchange is recorded | **Yes, as a chat-style run.** The help exchange is a run and run step like a chat, so its prompt and reply are stored scrubbed (`R-HIS-1`). It appears in history, and accept/discard is countable for the success metrics. |
| D3 | How secrets in a body are scrubbed before they leave | **Scrub before send, fail closed** (`R-ID-7`, `R-SEC-3`). Known secret values are masked. Any key-pattern hit refuses the send with a typed error that names the rule and location, never the text. Known-value masking gets better as MOD-10 resolves secrets; pattern refusal works today. Unlike today's chat path, which scrubs only before persisting, this send is scrubbed **before** it leaves. |
| D4 | Templates, skills, or both in the MVP | **Both, one action.** Templates send their placeholder table and are gated by `template::parse`; skills send only the body and keep their existing save check. |

## Constraints (fixed before planning)

- The action proposes and never persists. Every byte reaches the store through the editor's existing
  save path and gate (`R-SKL-3`, `R-PRM-4`).
- The send is refused, never degraded, when the scrub refuses (`R-ID-7` fails closed).
- The agent receives no htui credentials (`R-SEC-2`).
- The new key is a named action in MOD-67's catalogue, never a hard-coded chord.

## Delivery Milestones
<!-- Business outcomes, not engineering tasks. /plan turns each into a plan. -->
<!-- Status: pending | in-progress | complete -->

| # | Milestone | Outcome | Status | Plan |
|---|---|---|---|---|
| 1 | Agent help in the Skills editors | From the Templates or Library editor, the maintainer asks a registered agent for an edit, sees the scrubbed, recorded reply as a diff, and accepts it into the buffer or discards it; saving still goes through the gate | pending | — |

## Open Questions

Resolved by the maintainer after the PRD gate (2026-10-06); `/plan` adopts these:

- [x] Which project a help run belongs to: **the active project**; the action is refused when none is
  selected. `/plan` checks this against the run schema.
- [x] What instruction frames the request: **a fixed instruction in code for the MVP**, not a new
  versioned template role (`R-ID-5` holds, as the instruction is htui's own text).
- [x] How a reply becomes a proposed body: **the agent is asked for the whole body in one fenced
  block**; a reply without one is shown as text with nothing to accept.
- [x] Behaviour while waiting: **the request can be cancelled, and the buffer is locked until the
  reply arrives**, so the diff is always against what was sent.
- [x] Database unreachable: **help is unavailable**, as chat is.

## Risks

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| A masked secret comes back as the mask marker, so accepting the proposal silently replaces a real value with `[REDACTED]` | Medium | High | The diff shows the change; flag proposals that contain the mask marker before accept |
| Known-value masking is incomplete until MOD-10 fills in resolved secrets (the run engine's secret list is empty today) | High | Medium | Pattern refusal runs regardless (D3); the PRD does not claim more than the scrubber delivers |
| A proposal passes the gate but quietly changes behaviour (a dropped section, a reworded instruction) | Medium | Medium | Whole-body diff before accept; saving bumps a version that can be diffed and rolled back (`R-SKL-3`) |
| The help run is mistaken for a work run in history or counts | Low | Low | Mark it as an editor-help run; plan decides how |
| Skills get little safety from their gate (blank/NUL/unchanged only) | Medium | Low | Out of scope to strengthen here; the diff and the maintainer's accept are the check |

---
*Status: DRAFT — requirements only. Implementation planning pending via /plan.*
