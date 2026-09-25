# ANA-17 - Per-model calibration of the prompt's section framing (concluded, 2026-09-25)

Decide whether the framing around an assembled prompt's sections, meaning the separator between the
N blocks of `{{documents}}` or `{{candidates}}` and possibly the `<section>` wrapper itself, should
be calibrated per model. Opened from MOD-2 finding F-37. `R-PRM-1`, `R-PRM-2`.

**Verdict: no per-model calibration.** One frame serves every model. The `<section name="...">`
wrapper stays as ANA-5 §4.2 fixed it. The blank line MOD-2 shipped between blocks under one
placeholder (`crates/htui-core/src/prompt/mod.rs:543-545`) is now the settled separator, not a
placeholder, so changing it is a prompt-contract change.

Why:
- Anthropic, OpenAI (GPT-4.1 guide) and Google (Gemini) all endorse XML-style tagged blocks for
  multi-document input. None of them says anything about whitespace between closed elements. The
  one caution that differs, OpenAI's on XML-heavy documents, depends on the content, not the model.
- The published sensitivity results (Sclar et al. 2024; He et al. 2024) vary whole templates on
  short tasks with older or smaller models. They show that format preferences do not transfer
  between models, so calibration only has value when measured. `htui` has nothing to measure it
  with: the fan-out judge is far too coarse, and the agent harnesses add framing `htui` cannot see.
- A per-model frame would break ANA-5 invariant 3 (byte-identical siblings) as soon as MOD-36 mixes
  families in one group, leak model identity to the judge through `judge_task`, split
  `prompt_digest` per family, and multiply the golden snapshot set.

Rejected: a per-family frame table keyed on the model id, a hand-set per-agent override, and one LF
inside a placeholder. Whitespace at section boundaries already varies between one and two LFs with
the data, because seeded bodies put placeholders on consecutive lines and absent sections collapse,
so one LF would buy nothing. Deferred, not rejected: a calibration harness, which is the only route
to measured per-model framing. The reopen conditions are in `docs/ANA-17.md` §6.

Finding carried to **MOD-36**: the estimator function is `TokenEstimator::for_model`, not
`for_agent`, and every production `PromptSpec` passes `TokenEstimator::DEFAULT` (`engine.rs:4323`,
`:4945`, `preview.rs:252`). Per-candidate or per-group estimator choice is therefore new wiring.

No `MOD` item spawned. No code change.

Commits: the ANA-17 close-out commit on `claude/project-thread-6didvh`.

See `docs/ANA-17.md` for the full analysis.
