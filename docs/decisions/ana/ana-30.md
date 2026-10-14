# ANA-30 - Utility models: auxiliary cheap-model calls (session titles etc.) (concluded, 2026-10-14)

Opened to survey every place htui could hand background or drafting work to a small, cheap model
(session and item titles, summaries, history digests, commit messages, judge or triage calls), record
per use the call site, ANA-21 tier, API route, token cost and trigger, budget the set against a $100
Claude API credit, and decide whether a `utility_model` setting is worth building. Addresses `R-ID-6`,
`R-AGT-7`; touched at their seams `R-ID-7`, `R-SEC-2`/`-3`, `R-AGT-5`, `R-AGT-8`, `R-AGT-9`, `R-ORCH-7`,
`R-ORCH-11`, `R-HIS-1`, `R-PRM-1`, `R-ENT-9`/`-10`/`-14`, `R-ID-4`, `R-NF-2`. Analysis: `docs/ANA-30.md`.

**Surface (`docs/ANA-30.md` §2).** htui reaches a model only through an `AgentDriver` session tied to a
`run_step`; no code calls a model HTTP API. The seeded `claude`/`claude-cli` rows bill the subscription,
so as seeded nothing in htui spends the API credit; only `scripts/hr attach --api-key` does today. The one
existing one-shot utility pattern is MOD-55's EditHelp (tool-less, deny-all, scrubbed, recorded as a
`chat` run).

**Survey (§4).** Twelve allowed or marked uses (U1-U12), six ruled out by an existing requirement or
analysis (X1-X6: a model-written handoff step summary, excerpt ranking, commit messages (htui's only one is
a parsed reconcile marker), duplicate/link proposals, automatic triage, queue order or permission answers, and
a model-assisted legacy import), and one overflow row (O1). At Haiku 5.5
every allowed use together is about $4-16 a month (§5); the credit is not the constraint, plumbing and
requirement fit are.

**Verdict (§7).** No `utility_model` setting now and no MOD for one: the two uses worth having (item-title
drafting, chat titles) need surfaces that do not exist (`HelpTarget` has no item variant; there is no list
of past chats), and a direct Messages API client runs against `R-NF-2` (ANA-5) and ANA-18's
third-transport cost for about $9 a month saved. The credit is spent through a disabled-by-default
`per_token` registry row (`--bare`, pinned model, `apiKeyHelper` key route), used as phase overflow, an
optional judge and EditHelp's agent: data, not code. The setting (option B, `app_setting` `utility =
{agent_id, model}`) is built inside the first MOD that adds a second model-drafting surface.

**Found on the way (§2.3).** `spawn_supervised` passes no `env_clear` (`launch.rs:1184-1199`), so an
exported `ANTHROPIC_API_KEY` (an `hr attach --api-key`/`--shell` session, or a project secret of that
name) moves every subscription-billed `claude`/`claude-cli` run onto API billing while the row still says
`subscription`.

**Fact-check.** Five parallel readers (tree surfaces, tree inference routes, requirements and prior ANAs,
web pricing and Claude Code behaviour, prior art across eleven tools) and a gap-filler fed one author; two
read-only verifiers (tree citations, web figures and arithmetic) raised seven corrections, all re-checked
and applied before the verdict went to the maintainer. They corrected the cheap-judge cost (two calls per
point, $0.017, not one 120k prompt), the Haiku 4.5 multiplier (7.7x per call after the tokenizer change,
not 10x), the Settings wiring size of option B (`SettingKind` has no object kind), Zed's fast-model
fallbacks, one Gemini cache price and one line citation.

**Decisions (maintainer, 2026-10-14).** Every §8 recommendation accepted:
1. A display-only label a model derives and a draft a person accepts are outside `R-ID-6`, as long as no
   status transition, link, import, cache refresh or prompt reads the unaccepted text; recorded as a
   one-line clarification under `R-ID-6` in `docs/REQUIREMENTS.md`.
2. Which credit the $100 is (monthly Max credit or purchased) is checked in the Console before spending.
3. No `utility_model` setting now.
4. An `apiKeyHelper` command on a registry row is compatible with `R-AGT-9`; verified live before a row
   relies on it.
5. Upstream summaries come from an opt-in `summary` phase on a cheap row (MOD-98).
6. One dev-database query for stored ACP `session_info_update` titles precedes any chat-title design.
7. The env-strip item is filed now (MOD-97, filed as MOD rather than the proposed CLEAN because it changes
   behaviour; also carries the `--bare`-by-default risk to D92).
8. ANA-21's `haiku` row is re-keyed on `claude-haiku-5-5` and left unscored; dated amendment under
   `docs/ANA-21.md` §5.7, note on MOD-36.
9. No cheap judge yet; revisit with MOD-36's mixed-model data.

**Spawned.** MOD-97 (strip inherited vendor API keys from subscription-billed spawns), MOD-98 (opt-in
`summary` phase). Also applied: `--api-key` documented in `docs/hr-sandbox.md`. Owed on the host: calls 2
and 6.

Commits: analysis and close-out in the commit that added this file.
