# ANA-17 - Per-model calibration of the prompt's section framing

> **Scope note:** Decide whether the framing `htui` puts around the sections of an assembled prompt
> should be calibrated per model. That covers the separator between the N blocks one placeholder
> expands to, and possibly the `<section>` wrapper itself. Opened from MOD-2 finding F-37
> (`docs/decisions/mod/mod-2.md:362-366`). `R-PRM-1`, `R-PRM-2`.
>
> **Status (2026-09-25): concluded.** Verdict: **no per-model calibration.** One frame serves every
> model. The `<section>` wrapper stays as ANA-5 §4.2 fixed it, and the blank line between blocks
> under one placeholder stops being provisional: it is the settled separator. No `MOD-N` item is
> spawned. §6 lists the conditions that would reopen the question.

---

## 1. Context and problem statement

ANA-5 §4.2 fixed the wrapper: every section renders as `<section name="..." ...>`, its content, and
`</section>` (`docs/ANA-5.md:453-460`, `crates/htui-core/src/prompt/render.rs:227`). ANA-5 did not fix
what goes **between** two sections that one placeholder stands for. `{{documents}}` renders one
section per resolved document and `{{candidates}}` one per judge candidate. MOD-2 needed some bytes
there, so it shipped a blank line (`crates/htui-core/src/prompt/mod.rs:543-545`,
`rendered.join("\n\n")`). The choice was made for readability and was never measured against any
model.

The maintainer closed F-37 on 2026-09-16 as bigger than a separator. The question is whether the
framing should be **calibrated per model at all**. The agent registry already has a row per agent,
and the token estimator already varies by model family (D108). Any verdict has to price what varying
the frame would cost. The frame bytes are digest input, and every golden snapshot under
`crates/htui-core/tests/snapshots/` records them.

## 2. Surface as read

What the tree does today, checked on `main` at `d966413`:

| Fact | Where |
|---|---|
| The wrapper is one uniform tag with the name and metadata as attributes, one LF each side of the content, and no blank line inside an empty section. | `render.rs:221-246` |
| Blocks under one placeholder are joined with `"\n\n"`. Blocks under different placeholders are separated by whatever the template body puts between the placeholders. Every seeded body puts its section placeholders on consecutive lines (`defaults.rs:31-37` and so on), so two adjacent rendered sections are separated by **one** LF, and by a blank line when an empty placeholder sat between them and step 5 collapsed the run. The whitespace at a section boundary is therefore already one or two LFs, depending on the data. Only the closing and opening tags are constant. | `mod.rs:543-545`, `prompt/defaults.rs`, golden snapshots `prompt_implement_attempt2` (lines 11-17) and `prompt_all_empty` (lines 8-10) |
| Canonicalisation collapses any run of 3+ LFs to 2 *after* substitution. A separator of one or two LFs survives unchanged, and anything longer is folded to two. | `digest.rs:36-63`, ANA-5 §4.7 step 5 |
| `prompt_digest` is `sha256` over the canonical text, so the separator and the wrapper are digest input. | `digest.rs:71`, ANA-5 §4.7 step 7 |
| Fan-out siblings get byte-identical prompts, because the assembler runs **once per group**, before any candidate is live. | ANA-5 invariant 3 (`docs/ANA-5.md:132`), §7 (`:1870`), `crates/htui-orch/src/engine.rs:3297-3302` |
| The judge's `judge_task` section carries the judged step's stored prompt text verbatim. | ANA-5 §4.2 (`docs/ANA-5.md:485`) |
| **The estimator is not per-agent in practice.** The function that varies by family is `TokenEstimator::for_model` (keyed on a model id, `R-AGT-5`); there is no `for_agent`. Only its own unit tests call it. Every production `PromptSpec` passes `TokenEstimator::DEFAULT`: the step prompt (`engine.rs:4945`), the judge prompt (`engine.rs:4323`) and the preview (`crates/htui/src/preview.rs:252`). | `prompt/estimate.rs:80-95`, `:291-316` |
| The seeded registry covers two families: `claude` (ACP) and `claude-cli` (CLI), both Anthropic with no default model, and `agy`, which is Antigravity over ACP with `gemini-3.7-flash-high`. | `crates/htui-core/seeds/agent_*.json` |
| The prompt is not sent to a bare model. It is the first user turn given to an agent harness (Claude Code over ACP or `claude -p`, Antigravity over ACP) that already has its own system prompt and tool framing. | ANA-4 transports, `R-AGT-4` |

The HANDOFF text for ANA-17 and MOD-36 both say `TokenEstimator::for_agent` "already varies by model
family". The function is `for_model`, and nothing in production calls it. This matters to MOD-36 and
is carried there (§7).

## 3. Evidence

### 3.1 Vendor guidance

- **Anthropic.** "XML tags help Claude parse complex prompts unambiguously, especially when your
  prompt mixes instructions, context, examples, and variable inputs." The best practices say "Use
  consistent, descriptive tag names across your prompts" and "Nest tags when content has a natural
  hierarchy (documents inside `<documents>`, each inside `<document index="n">`)". For multiple
  documents: "wrap each document in `<document>` tags with `<document_content>` and `<source>` (and
  other metadata) subtags for clarity." [A1]
- **OpenAI.** The GPT-4.1 prompting guide reports long-context tests of document formats for
  many-document inputs. XML (`<doc id='1' title='The Fox'>...</doc>`) performed well, as did a
  pipe-delimited `ID: 1 | TITLE: ... | CONTENT: ...` form, and JSON "performed particularly poorly".
  It also cautions that XML delimiters work less well when the documents themselves contain a lot
  of XML. [O1]
- **Google.** The Gemini prompting guidance says XML-style tags and Markdown headings both work as
  delimiters, and asks for **one** convention per prompt, applied consistently, rather than several
  overlapping ones. It also says to put the instructions after a long context. [G1]

*Access note.* This session's egress proxy blocked `developers.openai.com`, `cookbook.openai.com`
and `ai.google.dev`. [O1] and [G1] are therefore paraphrased from search-engine extracts of those
pages, not quoted from a direct fetch. Only [A1] was fetched and is quoted verbatim.

What the three agree on: XML-style tags with explicit boundaries are a recommended form for every
family in the registry. None of them says anything about the whitespace **between** two closed
elements. ANA-5's wrapper already has the attributes-as-metadata shape that [A1] and [O1] both
describe. The only caution that differs ([O1] on XML-heavy content) depends on the **content**, not
on the model: an XML-heavy excerpt is XML-heavy for every model.

### 3.2 Published measurements

- **Sclar et al., ICLR 2024** [S1]. Few-shot accuracy moved by up to 76 points under formatting
  changes (separators, casing, spacing) on LLaMA-2-13B, and the sensitivity persisted with model
  size and instruction tuning. It also found that "format performance only weakly correlates between
  models", so a format tuned for one model does not carry over to another. This is the strongest
  argument that per-model calibration *could* matter.
- **He et al., 2024** [H1]. Plain text, Markdown, JSON and YAML templates, compared on GPT-3.5 and
  GPT-4. GPT-3.5 varied by up to 40% on a code-translation task, and the best template differed
  between model series (top-template overlap often below 0.2). Larger models were markedly more
  stable.

Both studies change the **whole template**: field labels, separators between few-shot examples, and
serialisation format. They also measure short, structured tasks on models that are older or smaller
than the ones `htui` drives. Neither isolates the variable ANA-17 owns: one LF versus two between
closed XML-style elements, inside a long agentic prompt. Both also report that sensitivity shrinks
as models get larger. So the evidence says calibration is only worth anything when it is
**measured per model on the task that matters**. It does not say which separator any model prefers.

## 4. What varying the frame would cost

1. **Fan-out loses its invariant once groups mix models.** MOD-36 exists to put different models
   on the candidates of one group (`HANDOFF.md`, MOD-36). With a per-model frame, siblings on
   different families get different bytes. That breaks ANA-5 invariant 3 and `R-ORCH-7`'s "on the
   same prompt", and forces the assembler to run once per candidate instead of once per group
   (`engine.rs:3297-3302`). MOD-36 already names this conflict for the estimator. A per-model frame
   would add a second, unconditional source of it: the estimator only diverges when a budget
   actually binds, but a frame diverges on every prompt.
2. **The judge sees a model fingerprint.** `judge_task` carries a judged step's prompt text, and a
   per-model frame would leak which family that prompt was framed for. MOD-36 requires that "the
   judge must not learn which model wrote which candidate".
3. **The digest stops comparing across models.** `prompt_digest` answers "which prompt produced
   these commits" (ANA-5 §4.7). With a per-model frame, the same logical prompt has one digest per
   family, and a step can only be matched to its preview when both resolve to the same family. That
   is the kind of loss MOD-33 is removing for the hostname.
4. **Snapshot churn.** A separator change re-records every golden snapshot with a multi-block
   placeholder (`prompt_golden__prompt_judge_three_candidates.snap` and
   `prompt_golden__prompt_implement_attempt2.snap` today). A wrapper change re-records 27 of the 28
   snapshots, every one except `prompt_render__template_frame.snap`. A per-model frame multiplies
   the golden set by the number of families.
5. **No way to calibrate it.** `htui` has no evaluation harness. Its only quality signal is the
   fan-out judge, which picks one of N candidates on the whole artefact. A one-LF difference in
   framing is far below the noise of a three-way judge. And the harnesses the prompt goes into add
   their own framing, which `htui` cannot see or pin.

## 5. Options

| Option | Verdict | Reason |
|---|---|---|
| **A. One frame for every model.** Keep the `<section>` wrapper and settle the blank line as the separator. | **Adopted** | Every vendor in the registry endorses XML-style tagged blocks (§3.1). The prompt uses one boundary convention, the tag pair, which is what [G1] asks for. The whitespace between closed tags is not a convention; it already varies with the data (§2). Nothing in §4 is spent. |
| B. A per-family frame table keyed on the model id, like `TokenEstimator::for_model`: separator, and optionally tag name. | Rejected | It pays every cost in §4 for a benefit nobody has measured and `htui` cannot measure (§4.5). The estimator it would copy is itself not wired (§2). |
| C. A per-agent override on the `agent` row or in `app_setting`, set by hand. | Rejected | Same costs as B, and it hands the maintainer a knob with no feedback loop. Template bodies are already editable per phase (MOD-9), which is where framing a maintainer wants to change belongs. That change also applies to every sibling alike. |
| D. Build a calibration harness first, then decide. | Deferred, not rejected | It is the only honest way to get B's benefit. But it is a project in its own right (fixed task set, per-model runs, a scoring rule better than a three-way judge), and nothing in the backlog needs it. §6 names when it becomes worth opening. |
| E. Use one LF inside a placeholder, the same as between two adjacent placeholders. | Rejected | It looks more uniform, but it would not make boundaries uniform, because an absent section between two placeholders still leaves a blank line (§2). The closing tag already marks the boundary, so the change is cosmetic. It would still re-record the golden snapshots with a multi-block placeholder (`prompt_judge_three_candidates`, `prompt_implement_attempt2`) and change every such digest, with nothing measured in return. The blank line also marks N blocks as one list for a human reading the prompt in the preview. |

## 6. Verdict

**No per-model calibration of the section framing.** One frame serves every model:

- the `<section name="...">` ... `</section>` wrapper stays exactly as ANA-5 §4.2 fixed it;
- the separator between blocks under one placeholder is a **blank line** (`"\n\n"`), and it is now
  a settled part of the prompt contract rather than MOD-2's placeholder. Changing it is a contract
  change, with the snapshot re-record that implies.

**Reopen when any of these holds.** The owner then opens a new analysis rather than reviving this one.

1. A calibration harness (option D) exists and shows a per-family difference that survives its
   noise floor on `htui`'s own tasks.
2. A model family whose vendor guidance *disfavours* XML-style tags for long documents is added to
   the registry.
3. `htui` starts sending prompts to a bare model API instead of an agent harness, which would make
   its frame the only framing the model sees.

A content-driven variation, such as a different wrapper for excerpts that are mostly XML ([O1]'s
caution), does not need this analysis reopened. It varies by content and applies to every model
equally, so it is an ordinary change to the ANA-5 contract.

## 7. Phasing and downstream impact

- **No code change and no `MOD-N` item.** The blank line MOD-2 shipped is the verdict. The comment at
  `crates/htui-core/src/prompt/mod.rs:543-544` can cite ANA-17 the next time that file is touched
  for another reason. That is not worth a commit of its own.
- **MOD-36** gets a note. The frame is model-independent, so a mixed-family group's only divergence
  is the estimator. The note also corrects the function name: `TokenEstimator::for_model` exists
  and `for_agent` does not. Today every production call site passes `TokenEstimator::DEFAULT`
  (`engine.rs:4323`, `:4945`, `preview.rs:252`), so choosing the estimator per candidate or per
  group is new wiring for MOD-36, not an existing seam.
- **ANA-21** is unaffected. Its weights choose which models run a group and do not touch how the
  prompt is framed.

## 8. Sources

- [A1] Anthropic, *Prompting best practices*,
  https://platform.claude.com/docs/en/build-with-claude/prompt-engineering/claude-prompting-best-practices
  (fetched 2026-09-25).
- [O1] OpenAI, *GPT-4.1 Prompting Guide*,
  https://developers.openai.com/cookbook/examples/gpt4-1_prompting_guide (search extract, direct
  fetch blocked 2026-09-25).
- [G1] Google, *Prompt design strategies*, https://ai.google.dev/gemini-api/docs/prompting-strategies
  (search extract, direct fetch blocked 2026-09-25).
- [S1] M. Sclar, Y. Choi, Y. Tsvetkov, A. Suhr, *Quantifying Language Models' Sensitivity to
  Spurious Features in Prompt Design or: How I learned to start worrying about prompt formatting*,
  ICLR 2024, https://arxiv.org/abs/2310.11324.
- [H1] J. He et al., *Does Prompt Formatting Have Any Impact on LLM Performance?*, 2024,
  https://arxiv.org/abs/2411.10541.
