# Plan: MOD-33 — the box hostname leaves the digest and gains a settings switch

**Status: CONFIRMED by the maintainer 2026-09-30** (fact-checked: 40 claims, three parallel
checkers). At CONFIRM: Q-1 answered *hostname only*; D272 kept (the judge digests its replayed
task); D273/T3 included. The pass falsified no design decision. It corrected: a missed third digest
snapshot (`prompt_preview__preview_ana_2.snap`); T3's pin list (asserts, not only messages, plus
four `Pending(9)` sites and three chain sites); D266's collapse (per-character, so CRLF needs its
own rule); the sent-form box must be *masked*, not only scanned; T1's list of test adaptations
(positional indices shift, two more tests, four snapshots named); phantom `PromptSpec` literal
sites dropped. All corrections are folded in below and itemised in "Verified claims".

**Source**: `HANDOFF.md:273-294` (MOD-33, from MOD-2 finding L-5, `docs/decisions/mod/mod-2.md:367-377`;
maintainer-decided 2026-09-16). Requirements `R-PRM-1`, `R-PRM-3`, `R-TUI-8`. Amends ANA-5 §4.2 and
§4.7 (maintainer-approved in the item; recorded at close-out, never in `docs/ANA-5.md`, D274).
Relates to ANA-16 §5.3 / §8 (child boxes, MOD-44).

**Complexity: medium.** One new concept in the assembler (a span that is rendered but not digested),
one new registry key with a boolean kind, one new recorder entry point, one `trim_record` field
(record v 4), and one optional comment-only migration (D273). No new store method, no `WriteStore`
trait change, no new SQL statement, no `.sqlx` entry, no new dependency, no new error variant, no
conformance case.

**Routing**: routed as **plan** by `/handoff-run MOD-33` (1 of C1–C4 fired: C2, the new "rendered but
not digested" span concept; C3/C4 borderline). Ultracode not needed.

**Numbering**: decision IDs continue the global sequence of `.claude/plans/*`, whose highest is
**D262** (`mod-23-agent-registry-editing.blueprint.md:1158`, `mod-64-concepts-search.blueprint.md`).
This plan's decisions are **D263…D277**. Risks **R-1…R-8**, questions **Q-1**, tasks **T0…T3**, all
local to this plan and cited as "MOD-33 R-n" outside it.

**Gortex note**: the index serves the primary checkout at `f5de3d4`, which is this branch's head
(`hr/MOD-33` has no commit of its own before this plan). Every `file:line` below is at `f5de3d4`.

---

## Summary

Today one string is sent, digested and stored. `assemble()` renders every section
(`prompt/mod.rs:467-477`), trims, substitutes (`:507`), canonicalises and hashes the result
(`:509-511`), and `AssembledPrompt`'s own doc says so: "`text` is canonical and is what is sent, what
is digested and what is persisted" (`mod.rs:324-326`). The box section's first line is
`hostname: {profile.hostname}` (`render.rs:376`), so the hostname is a digest input. The recorder then
re-hashes the scrubbed payload text itself (`record.rs:664`) and writes that as the payload's
`digest` and `run_step.prompt_digest` (`record.rs:665-667`, `sync_step` `:1162-1173`), overwriting the
pre-flight value stage 3 wrote with `set_step_prompt(step.id, &prompt.digest, …)`
(`engine.rs:3184`, `:3768`, `:4597`). ANA-5 §12 criterion 11 (`docs/ANA-5.md:2295-2297`) — the
pre-flight `prompt_digest` equals the payload's `digest` — holds only because the two hashes are over
the same bytes (`digest.rs:66-71` calls it "an identity rather than a second opinion").

MOD-33 splits that string in two, and only in one place:

- **The sent text** is unchanged: it carries `hostname: <the box's hostname>` exactly as today, when
  the project's switch is on.
- **The digest text** is the same canonical text with the hostname *value* replaced by a fixed
  stand-in, `[hostname]`. `prompt_digest` is `sha256(digest text)`.
- The two are produced by **substituting twice from one set of trimmed sections** — once with the
  box section in its digest form (the form every other step of the pipeline sees: scrub scan,
  estimate, trim, record) and once with the box section re-rendered in its sent form — and
  canonicalising each. Because the hostname is single-lined, the two canonical strings differ in
  exactly that span (D263, D266).
- The recorder is handed both strings and hashes the digest text, so criterion 11 keeps holding and
  the recomputation stays an identity — over the text ANA-5 supplies *for digesting* (D271).
- `trim_record` (v 4) gains `undigested: ["box.hostname"]` when the prompt carried the span, `[]`
  otherwise (D270).
- A new per-project boolean, `project.settings.box_hostname` (default **on**), decides whether the
  line is rendered at all; off omits it entirely and stops masking/scanning a value the model no
  longer sees (D267, D269). It is a new registry key, `SettingKey::BoxHostname`, so it appears in
  `Settings > Prompt` on each project's group with no new section (D276).

---

## What the tree says today (the facts the design rests on)

1. **Only the phase role can render a box section.** `Placeholder::allowed_in` (`template.rs:188-215`)
   admits `{{box}}` for `Phase` and not for `Judge` or `Handoff`. The eight seeded phase bodies place
   it (`defaults.rs:34`, `:51`, `:69`, `:89`, `:114`, `:131`, `:145`, `:164`); the judge and handoff
   bodies do not.
2. **The box section is protected and never re-rendered by the trimmer** (`SectionName::is_protected`,
   `mod.rs:301-307`; comment at `mod.rs:767-768`). So its digest form survives the ladder untouched,
   and the sent form can be rebuilt after it.
3. **`canonical()` only rewrites line endings, a leading BOM, runs of ≥3 LFs, and end-of-string
   whitespace** (`digest.rs:36-62`). A substitution inside one line that introduces no CR/LF, sits
   neither at the string's start nor at its end, therefore commutes with it.
4. **The residue scan runs over the rendered sections before the trim** (`mod.rs:484-490`) and the
   estimator measures each wrapped section (`Trimmer::new`, `trim.rs:417-446`,
   `est.estimate(&render::wrap(&rendered))`). The estimate is a function of **characters**
   (`estimate.rs:114-148`, `line.chars().count()`).
5. **`scrubbed_inputs` masks `profile.hostname` unconditionally** (`mod.rs:767-780`, the value at
   `:772`).
6. **Five production `PromptSpec` builders exist**: `engine.rs` `phase_spec` (`:5128`, `box_profile`
   at `:5144`) and `judge_prompts` (`:4421`, `:4439`); `preview.rs` `build` (`:279`, `:292`);
   `promote.rs` `handoff_spec` (`:88-106`, spreads `..phase`); `htui-agent/src/excerpt.rs` `step_pass`
   (`:1142-1144`, spreads `..spec.clone()`). Every settings-derived field is resolved by the caller
   from the raw `project.settings` blob (`settings::resolve_budget(…, Some(&project.settings), …)`,
   `engine.rs` `phase_spec`; `preview.rs` `build`).
7. **The only production `record_prompt` caller that records an assembled prompt is
   `Engine::open_recorder`** (`engine.rs:5376-5406`, the call at `:5402`). The other production
   caller, `agent_worker.rs` `run_chat` (`:3454`), records a chat's user text. A promoted step's
   handoff is a `follow_up`, never a second prompt (`a_handoff_is_a_follow_up_at_the_next_turn_not_a_second_prompt`,
   `htui-agent/tests/recorder.rs:3804`), and `OpeningPath::Handoff.digest` is read by no production
   consumer (`agent_worker.rs:932` matches `{ text, .. }`).
8. **`TrimRecord` derives `Serialize` only** (`trim.rs:184`), is written through `TrimRecord::to_value`
   (`trim.rs:273-277`, MOD-32's whole-record scrub) and read back by nobody as a type: the Runs
   projection reads `estimated_after` and `sections[].trimmed` only (`pg/read.rs:342`, the
   `prompt_summary` twin). `RECORD_VERSION` is 3 (`trim.rs:59`).
9. **The prompt settings registry is integer/fraction-only and App-first.** `SettingKind` has two
   variants (`settings.rs:277-282`); `SettingKey` is ten variants in key byte order with the
   discriminant indexing `SPECS` (`settings.rs:150-212`, `:320-431`); `Defaults::as_rows` iterates
   `SettingKey::ALL` and is pinned to migration `0002`'s ten rows
   (`the_defaults_are_migration_0002s_ten_rows_verbatim`, `settings.rs:807-823`);
   `prompt_settings::snapshot` reads `setting(SettingRung::App, key)` for **every** key in `ALL`
   (`crates/htui/src/prompt_settings.rs:141-149`). Both stores' `set_setting` / `clear_setting` /
   `stored_setting` are generic over `SettingSpec::project_key` on the `Project` rung
   (`pg/write.rs:3142-3276`, the SQL `settings || jsonb_build_object($2::text, $3::jsonb)` takes the key
   as a bind; `mem.rs:3251`, `:3332`, `:3436`).
10. **No child box exists in code.** No `parent_box`, no box kind; MOD-44 is open (`HANDOFF.md:608`).
11. **The golden texts carry `hostname: dev-win-01` and `hostname: ci-linux-01`**
    (`prompt/fixtures.rs:44`, `:279`; `prompt_golden__prompt_implement_attempt2.snap`,
    `prompt_golden__prompt_all_empty.snap`, `prompt_render__section_box.snap`,
    `prompt_render__section_box_minimal.snap`). The golden tests snapshot `prompt.text`, never the
    digest (`prompt_golden.rs:58`, `:116`, `:124`, `:132`). **Three** `crates/htui` snapshots render
    an assembled digest: `prompt_preview__preview_feat_1.snap:11`, `prompt_preview__preview_ana_2.snap:11`
    (`d68f3503…`, carries a `box` section) and `backlog__detail_prompt.snap:11`
    (`b9fb821f…`, from `ui/tabs/backlog/detail/prompt.rs:151`).

---

## Design decisions (settled here, not in code review)

| # | Decision | Why |
|---|---|---|
| D263 | **Two substitutions, one set of sections.** The box section's `Rendered` that the pipeline carries (scan, estimate, trim, record, `digest_text`) is its **digest form**: `hostname: [hostname]`. After the trim, `assemble` re-renders the box section in its **sent form** (the real, already-masked hostname), substitutes a second time with that section swapped in, and canonicalises both. `digest_text = canonical(substitute(live))`, `text = canonical(substitute(live with the sent box))`, `digest = sha256_hex(&digest_text)`. When no hostname is rendered, `text` is `digest_text` (one substitution). | Rejected alternatives: **(a) splice by byte offset** — `canonical()` collapses LF runs and so moves every offset after the first collapsed run; **(b) search-and-replace the hostname in the sent text** — ambiguous whenever the hostname also appears in an item body, a document or an excerpt (a hostname of `a` would match everywhere), which would make the digest depend on unrelated content; **(c) drop the line from the digest form** — on and off would then digest identically, so the digest could no longer say whether the model was shown a machine identifier at all; **(d) a span field on `Rendered`** — touches every `Rendered { … }` constructor in `render.rs` and `trim.rs` for one field that one section uses. The concept is named where it is recorded instead (D270): `UndigestedSpan`. The re-render is safe because the box is protected and never re-rendered by the trimmer (fact 2), and the swap is by `SectionName::Box`, never by searching text. |
| D264 | **The stand-in is the fixed constant `render::HOSTNAME_STAND_IN = "[hostname]"`.** It is a digest input forever, is never sent, and is never persisted (the digest text is hashed and dropped; D271). | A fixed, self-describing token. It is 10 characters, the same as the fixture hostname `dev-win-01`, so the golden fixtures' token figures do not move (D265). |
| D265 | **The estimator and the trimmer see the digest form**, so every trim decision and every `trim_record` number is independent of the hostname. The sent text can differ from what was estimated by the hostname's length minus 10 characters, at the prose rate. | Otherwise a longer hostname costs tokens, can push a rung at the margin, and moves the digest through *another* section — the promise "the hostname is not a digest input" would be false exactly at the margin. The error is bounded by one short line and is far inside the reserve (`prompt_reserve_fraction` 0.10). This deliberately relaxes H-1's "the estimate and the substituted bytes are the same strings" (`mod.rs:549-553`, `render.rs:255-258`) for this one span; the doc comments say so. *Fact-check nuance*: the estimator rounds per code/prose span, not per line, and the error is multiplied by the slot weight when `{{box}}` is placed twice. Only `dev-win-01` is 10 characters: `ci-linux-01` (11) and test hostnames such as `HTUI-TEST`, `SECOND-BOX`, `DESKTOP-HTUI` are not, so any test pinning *their* token figures or trim decisions may move by a token — implementers review each such diff rather than assume none. |
| D266 | **The sent-form hostname is single-lined at render**: each `\r\n` pair becomes one space, then any remaining lone `\r` or `\n` becomes a space. (Fact-check: `render::attr`'s collapse is per character — `render.rs:148` — so it would turn `\r\n` into two spaces; it is *not* reused.) | It is what makes fact 3 apply: with no CR/LF in the span, `canonical(sent)` and `canonical(digest)` differ in exactly that span. `gethostname` never yields a newline (`identity.rs:330`), so this changes no real prompt; it closes the case rather than trusting it. |
| D267 | **The switch is `project.settings.box_hostname`, a JSON boolean, default `true`, Project rung only.** It is a new registry key `SettingKey::BoxHostname` (`key = project_key = "box_hostname"`, `kind = SettingKind::Boolean` (new), `rungs = Rungs::PROJECT`, `min 0`, `max 1`, `unit "on/off"`), read by a new `settings::resolve_box_hostname(project: Option<&Value>) -> bool` that follows the house fall-through rule (absent, `null` or non-boolean ⇒ the compiled default `true`). `ProjectSettings` (`model/kind.rs:257-280`) is **not** extended: the prompt resolvers read the raw blob, as `resolve_budget` and `resolve_hops` do. | **Per-project, not `app_setting`**: the item says per-project, and the maintainer's case is a property of a project (a graphics engine built across machines), not of an installation. An App rung would also need a seed row (a data migration), would break the ten-row `0002` pin, and would need a compiled offline default — cost for a global default nobody asked for. Going through the registry is what puts it in `Settings > Prompt` with the existing compare-and-set write path, and both stores already accept any `project_key` (fact 9), so **no store, trait, SQL or `.sqlx` change**. |
| D268 | **`PromptSpec` gains `box_hostname: bool`**, resolved by the caller — `phase_spec` and `judge_prompts` in `engine.rs`, `build` in `preview.rs` — like `command_queue` (`mod.rs:111-112`). `assemble` stays pure (ANA-5 invariant 2). `render::box_profile` takes a `HostnameLine { Omitted, StandIn, Shown }` argument. | The switch is policy about a prompt, not a fact about a box: it does not belong on `BoxProfile`, which is also the MCP `box_profile` tool's payload (`box_.rs:267-268`). The two spreading builders (`promote.rs`, `step_pass`) inherit the field. |
| D269 | **Off: no `hostname:` line, and the hostname is neither masked nor scanned.** `scrubbed_inputs` masks `profile.hostname` only when `spec.box_hostname` is true; `undigested` is `[]`. | A value the model never sees must not refuse a prompt: today a credential-shaped hostname fails the step at `mod.rs:772`/`:779`; with the switch off that would be a refusal over bytes that are not sent. With the switch on, masking and the residue scan are exactly today's: the sent-form box section goes through the **same `scrub_text` mask-and-scan pass** as every rendered section (`mod.rs:484-490`), not a scan alone — otherwise a known secret masked in the digest form could survive in the sent form and the two strings would differ beyond the value (D263). |
| D270 | **`trim_record` v 4 gains `undigested`, always present: `["box.hostname"]` when a box section was rendered with its hostname, else `[]`.** Typed as `Vec<UndigestedSpan>` with one variant, `BoxHostname`, serialised `"box.hostname"` — a closed vocabulary like `TrimStrategy`. Listed once however many times the body places `{{box}}`. `RECORD_VERSION` becomes 4. | "The `trim_record` must say the prompt carried one" (item). A list rather than a boolean so the concept is not box-shaped. **Compat**: nothing deserialises a record (fact 8), so an old row lacking the key needs no `#[serde(default)]`; a reader branches on `v`. **v 4 rule**: `box` in `sections[]` and `undigested` empty ⇒ the switch was off. It goes through MOD-32's whole-record scrub unchanged — the pass enumerates no field (`trim.rs:222-228`), and the value is a fixed spelling. |
| D271 | **`AssembledPrompt` gains `digest_text: String`, and the recorder hashes it.** New `Recorder::record_prompt_digesting(text, digest_text, sections, at)`: the payload `{text, sections}` is scrubbed exactly as today; `digest_text` is scrubbed with the same scrubber (residue refuses through the same `refuse` path, `record.rs:1547`) and hashed; that hash is the payload's `digest` and `run_step.prompt_digest`. `record_prompt(text, sections, at)` becomes `record_prompt_digesting(text, text, sections, at)` and reuses the scrubbed payload text when the two are the same string, so the chat path is byte-identical. `open_recorder` is the one production call switched. The payload schema is unchanged. | ANA-4 fixes the division of labour: "The driver computes the digest; **ANA-5 owns what is digested**" (`docs/ANA-4.md:1257-1260`, quoted at `docs/ANA-5.md:1360-1364`). ANA-5 now supplies two strings — what is sent and what is digested — and the recorder still computes. Without this, the recorder would overwrite the pre-flight digest with `sha256(sent text)` and criterion 11 would fail on every box-bearing phase step. Passing the assembler's digest in instead was rejected: a scrub that masked anything would then leave a stored digest describing neither stored string. |
| D272 | **The judge's replayed task keeps the hostname, and so the judge prompt digests it.** `judge_prompts` replays the first survivor's stored payload `text` as `{{task}}` (`engine.rs:4330-4346`) — the sent form. It is not rewritten. | Rewriting a quoted prompt would show the judge a task the candidate never saw. The judge prompt is §4.7 rule 8's one deliberate exception already ("assembled once and has no sibling to match", `docs/ANA-5.md:1429-1435`), it is never previewed (`preview::offered` drops both reserved names), and it runs on the executing box beside its candidates. **The one decision most worth the maintainer's override at CONFIRM.** |
| D273 | **A comment-only migration `0010_prompt_digest_undigested.sql` restates `run_step.prompt_digest` and `run_step.trim_record`.** Precedent `0008_trim_record_v3.sql`, which exists only to restate the record's comment for a version bump. **Separable (T3):** if the maintainer prefers no migration, T3 is dropped and the two stale comments become a carried finding. | `0002`'s comment says the digest is "over the canonical assembled prompt TEXT as sent" (`0002_agent_probe.sql:49-52`) and `0008`'s enumerates the v 3 keys (`0008_trim_record_v3.sql:9-17`); both become false, and both are pinned verbatim by `the_ana_column_comments_are_present_and_verbatim` (`tests/migrations.rs:184-228`, `:438`). A forward-only comment is cheap; a column contract that lies is not. Cost stated: schema target 10, one mirror rebuild per box (`0008`'s R-56), and the migration count pins in `tests/connect.rs:142`, `:158`, `:244` and `tests/migrations.rs:93`, `:1176`. |
| D274 | **`docs/ANA-5.md` is not edited.** The amendment is written up in `docs/decisions/mod/mod-33.md` at close-out, quoting the §4.2 and §4.7 text it amends (draft below). | The milestone-5 precedent, restated for this very item at `docs/decisions/mod/mod-2.md:374-377`: "the amendment is recorded here and in `HANDOFF.md` rather than in `docs/ANA-5.md`, which only the maintainer edits." |
| D275 | **Child boxes are covered by construction.** There is no child-box code (fact 10). Every prompt takes its box section from one `BoxProfile` (`EngineParts.box_profile`, `engine.rs:417-418`; the preview's `box_profile(info.box_id)`, `preview.rs:240-257`); whichever box row MOD-44 makes a step execute on, its hostname is rendered, stood-in and gated by the same project switch. | MOD-44 inherits it if it builds a child box's `BoxProfile` with `BoxProfile::project` like any other row. ANA-16's worry that a container's churning hostname would move the digest (`docs/ANA-16.md:366-368`) no longer applies after this item; its hostname still reaches the text. |
| D276 | **Settings surface**: the key appears on every project group of `Settings > Prompt`, never on the `app` group (which stays the ten `App` keys). `e` opens the existing one-field editor prefilled `true`/`false`; `on`, `off`, `true`, `false` (ASCII case-insensitive) parse to a JSON boolean; empty clears (the default, on, answers). The effective column reads `on`/`off` from `resolve_box_hostname`; the pane prints `on or off · rungs project` instead of a numeric range. `SettingKey::app_keys()` (the keys whose rungs contain `APP`) replaces `SettingKey::ALL` in `Defaults::as_rows` and in `prompt_settings::snapshot`'s App loop. `BoxHostname` sorts first in key byte order, so it takes discriminant 0 and the ten shift by one; `project_keys()` keeps `ALL` order, so `box_hostname` becomes the **first** row of every project group and every positional index into `ProjectEntry.values` shifts by one. The source label for an unset project-only key reads `(default)`, not `(app_setting_default)` — no `app_setting` row can exist for it (`present_source`, `prompt.rs:920-926`). The UI module doc's "an eleventh key … appears here without this file being touched" (`prompt.rs:13-18`) is restated (a Boolean kind needed this file). | Reuses the section's single compare-and-set write path (D3/D4 of MOD-15 M5) instead of a new toggle key; `validate` stays the one place that judges shape. `SettingKey` derives no `Serialize` (`settings.rs:150`) and is carried in-process only, so renumbering persists nothing. |
| D277 | **The MCP `box_profile` tool is out of scope.** `BoxProfile` is also what that `R-MCP-2` tool will return (`box_.rs:267-268`); MOD-11 has not built it. | Recorded for MOD-11: it must decide whether the tool honours the project switch. Nothing here changes `BoxProfile`. |

---

## Files to change

| File | Action | Task | Why |
|---|---|---|---|
| `crates/htui-agent/src/record.rs` | edit | T0 | `record_prompt_digesting`; `record_prompt` delegates (D271) |
| `crates/htui-agent/tests/recorder.rs` | edit | T0 | new cases (D271) |
| `crates/htui-core/src/prompt/settings.rs` | edit | T1 | `SettingKind::Boolean`, `SettingKey::BoxHostname`, `SPECS[11]`, `app_keys()`, `validate`, `resolve_box_hostname`, tests (D267, D276) |
| `crates/htui/src/prompt_settings.rs` | edit | T1 | App loop over `app_keys()`; docs "ten" (D276) |
| `crates/htui/src/ui/tabs/settings/prompt.rs` | edit | T1 | `effective` arm, `submit` Boolean parse, pane line (D276) |
| `crates/htui/tests/prompt_settings.rs` | edit | T1 | counts over `app_keys()`; new cases (D276) |
| `crates/htui/tests/snapshots/prompt_settings__{demo,clamped,editor_fraction,stale}.snap` | regenerate | T1 | a `box_hostname` row first in each project group (`app_only`, `offline` unchanged) |
| `crates/htui-core/src/prompt/render.rs` | edit | T2 | `HostnameLine`, `HOSTNAME_STAND_IN`, `box_profile` signature, single-lining, unit tests (D263, D264, D266) |
| `crates/htui-core/src/prompt/mod.rs` | edit | T2 | `PromptSpec.box_hostname`, `AssembledPrompt.digest_text`, conditional mask, two substitutions, docs (D263, D265, D268, D269, D271) |
| `crates/htui-core/src/prompt/digest.rs` | docs only | T2 | module doc and `sha256_hex` doc: "as sent" and "identity" restated (D271) |
| `crates/htui-core/src/prompt/trim.rs` | edit | T2 | `UndigestedSpan`, `TrimRecord.undigested`, `RECORD_VERSION = 4`, `record()` argument (D270) |
| `crates/htui-core/src/prompt/fixtures.rs` | edit | T2 | `box_hostname: true` in the full literals |
| `crates/htui-core/src/fixtures.rs` | edit | T2 | `step_impl_carries_the_golden_trim_record` (`:2512-2536`) also strips `undigested` from the live record, as it already patches `v` |
| `crates/htui-core/tests/prompt_hostname.rs` | **new** | T2 | the MOD-33 cases |
| `crates/htui-core/tests/prompt_render.rs` | edit | T2 | `box_profile` call sites (`:40`, `:207`, `:232`) + two new snapshots |
| `crates/htui-core/tests/snapshots/prompt_render__section_box_stand_in.snap`, `…_section_box_no_hostname.snap` | **new** | T2 | D263, D269 |
| `crates/htui-core/tests/prompt_digest.rs` | edit | T2 | key set gains `undigested`, `v` is 4 (`:950-1010`) |
| `crates/htui-orch/src/engine.rs` | edit | T2 | resolve `box_hostname` in `phase_spec` and `judge_prompts`; `open_recorder` calls `record_prompt_digesting`; one new test; doc "closed enum of ten" (`:6119`) |
| `crates/htui-orch/src/conformance.rs` | docs only | T2 | "closed enum of ten" (`:1849-1850`) |
| `crates/htui/src/preview.rs` | edit | T2 | resolve `box_hostname` in `build` |
| `crates/htui/tests/prompt_preview.rs` | edit | T2 | new cases |
| `crates/htui/tests/snapshots/prompt_preview__preview_feat_1.snap`, `prompt_preview__preview_ana_2.snap`, `backlog__detail_prompt.snap` | regenerate | T2 | the digest shown changes (fact 11) |
| `crates/htui-store/migrations/0010_prompt_digest_undigested.sql` | **new** | T3 | D273 |
| `crates/htui-store/tests/migrations.rs` | edit | T3 | a `MOD33_COLUMN_COMMENTS` pin chained at all three sites (`:444-447`, `:497-501`, `:511-515`); `ANA_COLUMN_COMMENTS`' two restated rows move out of it; count asserts `:88`, `:1174`; `Pending(9)`/`MigrationsPending(9)` at `:901-902`, `:1000`, `:1005`, `:1024`; doc counts `:176-183` |
| `crates/htui-store/tests/connect.rs` | edit | T3 | count asserts and messages `:140`, `:155-156`, `:242` |

**Not touched, on purpose:** `crates/htui-core/src/model/box_.rs` (D268, D277),
`crates/htui-core/src/model/kind.rs` (D267), every store (`store/traits.rs`, `store/mem.rs`,
`htui-store/src/**`) (D267), `crates/htui-core/src/scrub.rs`, `crates/htui-store/.sqlx/` (no new
query), `cache_migrations/`, `crates/htui/src/agent_worker.rs` (chat keeps `record_prompt`),
`docs/ANA-5.md`, `docs/ANA-16.md` (D274). `HANDOFF.md`, `DECISIONS.md` and
`docs/decisions/mod/mod-33.md` are the main thread's, at close-out.

---

## Tasks

**Independence.** T0, T1 and T3 have pairwise-disjoint file lists (table above) and each leaves the
workspace building and green on its own: T0 adds a method nobody calls yet, T1 adds a key nothing
reads yet, T3 changes only comments and their pins. **T2 depends on T0** (it calls
`record_prompt_digesting`) **and on T1** (it calls `settings::resolve_box_hostname`), and it must land
every `PromptSpec` literal in the same commit that adds the field, or the workspace does not build.
Hidden couplings checked: no two tasks share a snapshot file (`prompt_settings__*` is T1's,
`prompt_preview__*`/`backlog__detail_prompt` and `crates/htui-core/tests/snapshots/*` are T2's); no
task touches `.sqlx`; T3's comment text is fixed in this plan, so it does not wait for T2's code.

| Task | Files (complete list) | Parallel |
|---|---|---|
| T0 | `htui-agent/src/record.rs`, `htui-agent/tests/recorder.rs` | with T1, T3 |
| T1 | `htui-core/src/prompt/settings.rs`, `htui/src/prompt_settings.rs`, `htui/src/ui/tabs/settings/prompt.rs`, `htui/tests/prompt_settings.rs`, `htui/tests/snapshots/prompt_settings__*.snap` | with T0, T3 |
| T2 | everything marked T2 in the table above | after T0 and T1 |
| T3 | `htui-store/migrations/0010_prompt_digest_undigested.sql`, `htui-store/tests/migrations.rs`, `htui-store/tests/connect.rs` | with T0, T1, T2; droppable (D273) |

If the tasks fan out to worktrees, each worktree costs a `target/` of ~10 GB; check `df -h /` first
and run serially (T0, T1, T3, T2) when space is short.

**Conditions binding on every implementer:**
1. **No store, trait, `.sqlx`, `BoxProfile` or `ProjectSettings` change** (D267, D268). If one seems
   needed, stop and report.
2. **No `Deserialize` on `TrimRecord`**, no `#[serde(default)]` compat shim (D270).
3. **Never find the hostname by searching text.** The sent form is a re-render swapped in by
   `SectionName::Box` (D263).
4. **No edit to `docs/ANA-*.md`, `HANDOFF.md`, `DECISIONS.md`** (D274); bookkeeping is the main thread's.
5. **Test first**: each task's new cases are written and seen failing (or failing to compile) before
   the change that makes them pass.
6. **Commit incrementally**, staging only your own paths. No `git stash`, no `git add -A`, no
   `--amend`, no push.

### T0 — the recorder hashes the text ANA-5 supplies for digesting (D271)

Test first (`crates/htui-agent/tests/recorder.rs`, beside `seq_is_gapless_turns_count_and_digest_reaches_the_step` `:1181`):
1. `a_split_prompt_stores_the_sent_text_and_digests_the_digest_text`: `record_prompt_digesting("A x Z",
   "A [s] Z", …)` → the seq-0 payload `text` is `"A x Z"`, its `digest` is `sha256("A [s] Z")`,
   `RecorderSummary.prompt_digest` equals it, and the spy's first `set_step_usage` carries it.
2. `record_prompt_is_the_split_with_one_string`: `record_prompt(t, …)` and
   `record_prompt_digesting(t, t, …)` on two fresh recorders produce identical rows and digests.
3. `a_secret_in_the_digest_text_is_masked_before_the_hash`: with `MinimalScrubber::new([SECRET])`, a
   digest text containing `SECRET` hashes to `sha256` of its masked form.
4. `residue_in_the_digest_text_refuses_the_prompt`: a credential-shaped digest text (prefix rule)
   refuses through the same `refuse` path a credential-shaped prompt payload takes today: no prompt
   row, the refusal row, no digest. (Fact-check: `scrub_residue_refuses_write`,
   `conformance.rs:3164`, refuses a *tool_result* and lets its prompt land — it is not the model.
   Scrubbing `digest_text` as a bare string reports `Unmasked.path = ""`; the implementation passes
   `"/digest_text"` (or equivalent) to `residue_row` so the refusal message names the field. The
   case is unreachable in production — the stand-in is fixed — and exists to pin the path.)

Then `record.rs`: add `record_prompt_digesting`; make `record_prompt` delegate; update the doc at
`:634` and the module doc's digest sentences. The existing `seq_gapless_and_turns` and
`usage_deltas_sum_to_step_usage` expectations (`sha256(PROMPT)`, conformance `:1421`, `:2114`) must
pass unchanged.

### T1 — the `box_hostname` key and its Settings row (D267, D276)

Test first:
- `settings.rs` in-file: `box_hostname_is_a_project_only_boolean` (spec fields; `rung_refusal(BoxHostname,
  APP)` is `Some`); `validate` accepts `true`/`false` on `PROJECT` and refuses `1`, `"on"`, `null` with
  ``"`box_hostname` must be a JSON boolean, got …"``; `resolve_box_hostname` is `true` for `None`,
  `{}`, `{"box_hostname": null}`, `{"box_hostname": 1}`, and follows the value for `true`/`false`;
  `app_keys()` is the ten App keys in byte order. Adapt `every_default_validates_under_its_own_spec`
  (validate each key on a rung its spec accepts), `specs_are_indexed_by_discriminant` (eleven keys,
  still strictly ascending), and keep `the_defaults_are_migration_0002s_ten_rows_verbatim` and
  `as_rows_is_key_byte_order` unchanged in substance (they now read `app_keys()` through `as_rows`).
- `crates/htui/tests/prompt_settings.rs`: `the_demo_snapshot_has_ten_app_entries_and_two_keys_per_project`
  becomes ten App entries and **three** keys per project; row-index helpers (`app_row` `:651-660`,
  `project_row` `:662-667`) and `no_key_is_listed_on_a_rung_its_spec_refuses` (`:891-905`) count
  `app_keys()` for the App group; new `e_then_off_then_enter_sends_set_setting_false_on_the_project_rung`
  (the `SetSetting` carries `Value::Bool(false)` and the project row's token) and
  `set_setting_box_hostname_on_a_project_stores_a_boolean_and_keeps_foreign_keys` (MemStore through
  `Backend::memory`, the `set_setting_on_project_stores_under_the_project_key_and_keeps_foreign_keys`
  shape at `:335`); an unparsable entry (`maybe`) is refused locally with
  ``"`box_hostname` is on or off, or empty to clear"`` and sends nothing.
  **Also adapt** (fact-check; all in the same file): `the_demo_snapshot_has_ten_app_entries…`
  (`:174-178`) — the `vec![UpstreamHops, TokenBudget]` assertion and `values[0]`/`values[1]` indices
  shift (`BoxHostname` is now first); `set_setting_on_project_stores_under_the_project_key_and_keeps_foreign_keys`
  (`:378-379`, `values[0]` is now `BoxHostname`); `the_demo_snapshot_renders_the_tree` (`:801-818`,
  loops `ALL` expecting `DEFAULTS.value_of(key)` — loop `app_keys()` for the App group and assert
  `on` for `box_hostname`); `no_workspace_lists_the_app_group_alone` (`:845`, `ALL.len()` →
  `app_keys().len()`).

Then implement per D267/D276. Regenerate the `prompt_settings__*` snapshots that show a project group
(review each diff: exactly one new `box_hostname  unset | on (app_setting_default)  on/off` row per
project, nothing else).

### T2 — the assembler splits sent from digested, and the callers read the switch (D263–D266, D268–D272, D275)

Test first — new `crates/htui-core/tests/prompt_hostname.rs` over `prompt::fixtures`:
1. `the_digest_does_not_move_with_the_hostname`: `phase_implement_attempt2()` assembled with hostname
   `dev-win-01` and with `a-much-longer-build-host.example.internal` → equal `digest`, equal
   `digest_text`, equal `trim.to_value(..)`, different `text`; each `text` contains its own
   `hostname: …` line.
2. `the_digest_text_is_the_sent_text_with_the_stand_in`: `digest_text == text.replacen("hostname: dev-win-01", "hostname: [hostname]", 1)`
   for the fixture (safe here because the fixture places `{{box}}` once and nothing else spells it),
   `canonical(&digest_text) == digest_text`, `sha256_hex(&digest_text) == digest`.
3. `the_switch_off_omits_the_line_and_records_nothing_undigested`: `box_hostname = false` → no
   `hostname:` anywhere in `text`; `text == digest_text`; `trim.undigested` is `[]`; `box` is still in
   `sections`; the digest differs from the switch-on digest.
4. `switch_on_records_box_hostname_once`: a body placing `{{box}}` twice renders the hostname in both
   sections of `text`, the stand-in in both of `digest_text`, and `undigested == ["box.hostname"]`.
5. `an_off_switch_never_refuses_over_the_hostname`: a hostname the prefix rules refuse (e.g.
   `sk-ant-…`-shaped) → switch on: `AssembleError::Unmasked { section: "box", .. }`; switch off:
   assembles.
6. `a_newline_in_the_hostname_is_one_line_in_the_text`: `"a\r\nb"` renders `hostname: a b` (one
   space, D266), `"a\rb\nc"` renders `hostname: a b c`, and the digest equals the `dev-win-01` case's.
7. `an_item_body_that_spells_the_box_is_left_alone`: an item body containing
   `<section name="box">\nhostname: [hostname]` and the literal hostname → the item section's bytes
   are identical in `text` and `digest_text` (the swap is by section, not by search).
8. `the_record_is_v4_with_undigested`: `v == 4`, `undigested == ["box.hostname"]` in `to_value`.
9. Criterion 5 still holds: the fan-out siblings case (`prompt_digest.rs:74-75`) is untouched and green.

`render.rs` in-file: `box_profile(&full, Shown)` is today's content byte for byte (the existing test
`:1165-1212`, updated to the new signature); `StandIn` differs only in the first line;
`Omitted` has no hostname line and starts with `os:`. New snapshots in `prompt_render.rs`
(`section_box_stand_in`, `section_box_no_hostname`); `section_box` and `section_box_minimal` must not
change.

Then the implementation, in this order:
1. `render.rs`: `HOSTNAME_STAND_IN`, `HostnameLine`, `box_profile(profile, line)` (D264, D266).
2. `trim.rs`: `UndigestedSpan`, the field, `RECORD_VERSION = 4` with its doc history line, `record()`
   takes the list (D270).
3. `mod.rs`: `PromptSpec.box_hostname` (doc: "`project.settings.box_hostname`, resolved by the caller
   (MOD-33 D267)"); `AssembledPrompt.digest_text` and the rewritten struct doc (`:322-337`);
   `scrubbed_inputs` masks the hostname only when on (`:767-780`); `render_sections` passes `StandIn`
   or `Omitted` (`:631`); `assemble` renders the sent box and passes it through the same `scrub_text`
   mask-and-scan as the residue pass, right after it (`:484-490`; D269), substitutes twice after the trim (`:506-511`), and records `undigested` (D263, D269).
   Rewrite the H-1 comments at `:549-553` and `render.rs:255-258` to name the one span the estimate
   does not measure (D265).
4. `digest.rs`: docs only — the module doc's "as sent" (`:4-8`) and `sha256_hex`'s identity paragraph
   (`:66-71`) now say: the digest is over the digest text, which is the sent text with each undigested
   span's stand-in, and the recorder's recomputation is an identity over that string (D271).
5. `prompt/fixtures.rs`, `crates/htui-core/src/fixtures.rs` (`:2512-2536`, strip `undigested` from the
   live record next to the `v` patch), `prompt_digest.rs` (`:990-1010`: fourteen keys, `v` 4),
   every `PromptSpec` full literal the compiler names. Fact-checked set: `prompt/fixtures.rs:161`,
   `:260`, `:363`, `:502`, `engine.rs:4421`, `:5128`, `preview.rs:279`. Every other site spreads a
   base (`mod.rs:541`, `:1365`, `promote.rs:105`, `:429`, `excerpt.rs:1142`) or mutates a fixture
   (`htui-agent/tests/excerpt.rs:1095`, `:1434`, `prompt_digest.rs`, `prompt_skills.rs`), and needs no
   edit.
6. `engine.rs`: `box_hostname: settings::resolve_box_hostname(Some(&project.settings))` in
   `phase_spec` (`:5128-5171`) and `judge_prompts` (`:4421-4454`); `open_recorder` (`:5402`) calls
   `record_prompt_digesting(&prompt.text, &prompt.digest_text, prompt.payload_sections_value(), self.now())`.
   New engine test `a_step_digest_is_the_payload_digest_and_not_the_sent_text_hash`: after a phase
   step's stage 3, `run_step.prompt_digest` equals the seq-0 payload `digest` (criterion 11) and
   differs from `sha256(payload.text)`; a second walk with a `box_profile` whose hostname differs
   records the same `prompt_digest`.
7. `preview.rs`: the same resolver in `build` (`:279-304`). New `prompt_preview.rs` cases: two
   memory backends whose registered box hostnames differ preview the same `digest` and different
   `text`; a project with `{"box_hostname": false}` previews no `hostname:` line.
8. Regenerate `prompt_preview__preview_feat_1.snap`, `prompt_preview__preview_ana_2.snap` and
   `backlog__detail_prompt.snap`; review that
   the **only** change is the digest line (the preview's hostname `DESKTOP-HTUI` is 12 characters, so
   a token figure may move by at most one, D265 — say so in the commit if it does).

### T3 — the column comments say what the columns now hold (D273)

Test first: add `MOD33_COLUMN_COMMENTS` in `tests/migrations.rs` with the two texts below, chain it
into `the_ana_column_comments_are_present_and_verbatim` at **all three** chain sites (`:444-447`,
the commented-column list `:497-501`, the expected set `:511-515` — chaining only the first fails
the "exactly thirty-five commented columns" assert), and remove the `run_step.prompt_digest` and
`run_step.trim_record` rows from `ANA_COLUMN_COMMENTS` (`:208-228`) with a doc line naming `0010` as
their restatement (the `0008` doc sentence at `:181-182` is the model; the doc counts at `:176-183`,
"twenty-five" / "the next five", become 23 / 3; the total stays 35). Update every count pin (ten
migrations, through MOD-33's `0010_prompt_digest_undigested.sql`): asserts and messages at
`tests/connect.rs:140`, `:155-156`, `:242`; `tests/migrations.rs:88` (the applied-version `vec![…]` gains `10`), `:1174`;
`Pending(9)` / `HeadlessError::MigrationsPending(9)` at `tests/migrations.rs:901-902`, `:1000`,
`:1005`, `:1024`. Nothing in `src/`, `build.rs`, `.sqlx` or the cache mirror pins the count
(`schema_version()` derives from the migrator, `pg/mod.rs:626`).

Then the migration, forward-only, comment only, header in `0008`'s form:

```sql
COMMENT ON COLUMN run_step.prompt_digest IS
  'ANA-5 4.7 as amended by MOD-33: sha256, lowercase hex, over the canonical assembled prompt '
  'TEXT as sent with each undigested span replaced by its fixed stand-in - today only the box '
  'section''s hostname value, as [hostname] - LF normalised, BOM stripped, one trailing LF, '
  'scrubbed before hashing. trim_record.undigested lists the spans. Not over the payload and not '
  'over sections[]. An audit field, never a replay key (ANA-2 4.9).';

COMMENT ON COLUMN run_step.trim_record IS
  'ANA-5 5.1 as amended by MOD-9 D42 and D118 and MOD-33: {v, template, budget, budget_source, '
  'reserve, target, estimator, estimated_before, estimated_after, sections[], skill_choices[], '
  'undigested[], excerpts, notes}, v 4. undigested[] names every span rendered into the prompt '
  'but excluded from prompt_digest: box.hostname, or empty. skill_choices[] is every candidate '
  'skill, ordered by position then name, each {skill, name, version, level, activation, active, '
  'reason} with reason always, matched, no_match, off, no_path, missing_version or not_placed '
  '(ANA-22 6 item 8); a matched choice adds path, <repo>:<path>, the first matching file in repo '
  'then path byte order. A v 3 record, written before 0010, has no undigested and digested the '
  'hostname; a v 2 record, written before 0008, has no matched or no_match; a v 1 record, written '
  'before 0007, has no skill_choices. Canonical; the prompt payload sections[] array is its '
  'abridged projection. Written at stage 3 by set_step_prompt, before the session starts.';
```

(The fact-check pass should re-verify `0010` is still the next free number immediately before T3
runs; see R-4.)

---

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| **R-1** — Cross-box digest equality still fails whenever two boxes differ in `os`, `cpu`, `ram`, `gpu`, `htui` or `tools`, all of which remain digest inputs | Certain for heterogeneous boxes | Stated, not fixed: the maintainer's decision names the hostname only, and ANA-5 §4.7 "What varies legitimately" (`docs/ANA-5.md:1441-1445`) calls the rest legitimate variation. **Q-1** asks whether that is the intended scope. |
| **R-2** — Re-verifying a stored digest offline now needs a rule (in each box section, replace the `hostname:` value with `[hostname]`), and an item body that spells an inert `<section name="box">` could mislead a naive re-verifier | Low | The stored digest itself is correct: the recorder hashes the string the assembler built, never a search result (D263, D271). The rule and its caveat go into the write-up; `trim_record.undigested` tells a reader when it applies. |
| **R-3** — The judge prompt digests the hostname through its replayed task (D272) | Certain when the switch is on | Recorded as D272, flagged for CONFIRM. |
| **R-4** — Another in-flight item takes migration `0010` | Medium (HANDOFF names `0010` as next, `HANDOFF.md:50-51`) | T3 is separable and runs last-checked; renumber at merge if needed. |
| **R-5** — The estimate is off by the hostname's length minus 10 characters | Certain, bounded | D265: one short line against a 10 % reserve; the fixtures' `dev-win-01` is exactly 10 characters. |
| **R-6** — A reviewer reads `digest_text` as "canonicalise for hashing but send the original bytes", which §4.7 rejected (`docs/ANA-5.md:1379`) | Medium | The write-up answers it directly: the rejected option digested a string *never sent*; here the digest text differs from the sent text in one declared, recorded span, and the recorder hashes what ANA-5 supplies. |
| **R-7** — Settings snapshots churn and the Boolean kind leaks into integer-only code paths (`value_line`, the D13 clamp line) | Medium | D276 sets `Effective.number = None` for the key (the fraction key's precedent), so the clamp line never fires; T1's snapshot review is "one row per project, nothing else". |
| **R-8** — A Postgres test failure is read as real when the dev Postgres is recovering | High on this box | `df -h /` first, then re-run the case alone (project memory). |

## Validation

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-features --all-targets -- -D warnings
cargo test -p htui-core --all-features -- --test-threads=1
USERNAME=htui-ci HTUI_TEST_DATABASE_URL=postgres://postgres@localhost:5439/postgres \
  cargo test --workspace --all-features -- --test-threads=1
find crates -name '*.snap.new' | wc -l                       # 0: no pending snapshot
ls crates/htui-store/.sqlx | wc -l                           # 289, unchanged
git diff --stat f5de3d4 -- crates/htui-core/src/store crates/htui-store/src \
  crates/htui-core/src/model crates/htui-store/.sqlx crates/htui-store/cache_migrations   # empty
```

`--test-threads=1` is not optional (the keyring fake is process-wide). Before believing a Postgres
failure, `df -h /`, then re-run the case alone.

## Acceptance

- [ ] With the switch on, `text` carries `hostname: <box>` exactly as before; `digest` is over
      `digest_text`, which differs from `text` only in that value (`[hostname]`); two boxes differing
      only in hostname produce the same `digest` and the same `trim_record` (T2 cases 1, 2).
- [ ] With the switch off, no `hostname:` line is rendered, the hostname is not masked or scanned,
      and `undigested` is `[]` (T2 cases 3, 5).
- [ ] `trim_record` is v 4 with `undigested` always present (T2 case 8); it goes through MOD-32's
      scrub unchanged.
- [ ] ANA-5 §12 criterion 11 holds: `run_step.prompt_digest` equals the seq-0 payload `digest` for a
      box-bearing phase step (T2 step 6's engine test); the chat path's digests are unchanged (T0 case 2).
- [ ] `project.settings.box_hostname` is editable per project in `Settings > Prompt`, default on,
      through the existing compare-and-set path; the `app` group still lists exactly ten keys (T1).
- [ ] No store, trait, `.sqlx`, `BoxProfile`, `ProjectSettings` or `docs/ANA-*.md` change.
- [ ] Either T3 landed with its pins green, or the maintainer declined it and the two stale comments
      are carried in the write-up.
- [ ] `validate-workflow-docs.sh` exits 0 at close-out.

---

## The ANA-5 amendment (draft for `docs/decisions/mod/mod-33.md`, close-out)

Quote, then amend (D274):

- **§4.2, the box projection** (`docs/ANA-5.md:510-537`): the example's first line `hostname:
  dev-win-01` becomes conditional. *Amended:* "`hostname` is rendered when
  `project.settings.box_hostname` is true (the default) and omitted entirely when it is false. When
  rendered it is a **rendered-but-not-digested span**: the model sees the value; the digest sees the
  fixed stand-in `[hostname]` (§4.7)." The closed field list otherwise stands (Open for the
  maintainer 5, `:537`).
- **§4.7, options for what is digested** (`:1367-1373`): the adopted row "The assembled prompt text
  alone, canonicalised, as sent" gains "…with each undigested span's value replaced by its stand-in".
  The rejected row "Canonicalise for hashing but send the original bytes" (`:1379`) keeps its verdict
  and gains the distinction R-6 states.
- **§4.7, the pipeline** (`:1386-1398`): step 7 becomes "`sha256` over the UTF-8 bytes of the
  *digest text*: the result of step 6 with every undigested span's value replaced by its stand-in";
  step 8 becomes "hand the sent `String` to `AgentDriver::start`, and the sent `String` with its
  digest text to the recorder, which hashes the digest text". The estimate (§4.4) is over the digest
  form (D265).
- **§4.7, rule 8 and "What varies legitimately"** (`:1429-1445`): the HANDOFF item says rule 8's
  "no machine identifier" becomes a statement about the digest; the rule's text actually reads "no
  absolute path, no run id and no step id … in the digested text", and the hostname is discussed in
  the following paragraph. *Amended paragraph:* "The box profile section carries `os`, `cpu`, `ram`
  and the tool list, so the same logical prompt on two different boxes digests differently. The
  hostname is rendered but not digested (MOD-33), so it is the one machine identifier that never
  moves a digest; a project may also omit it from the prompt."
- **§5.1** (`:1483`): `trim_record` v 4 adds `undigested`. **§5.3** (`:1577`): `box_hostname` is a
  `project.settings` key, not an `app_setting` key. **§9**'s column comments: restated by `0010` (or
  carried, D273).
- **ANA-16 §5.3 / §8** (`docs/ANA-16.md:366-368`, `:782`): a child box's hostname no longer affects
  the digest; the switch covers it by construction (D275).

---

## Where the HANDOFF or the tree disagree

- **`HANDOFF.md:276-277` — "an identical `PromptSpec` assembled on two boxes produces different
  bytes".** Not literally: `assemble` is pure (ANA-5 invariant 2, `mod.rs:422-433`) and
  `box_profile` is part of the spec, so an identical spec is identical bytes on any box. What the item
  means, and what this plan fixes, is two specs that differ **only in `box_profile.hostname`**. Other
  box fields still differ between heterogeneous boxes (R-1, Q-1).
- **`HANDOFF.md:278-279` — "MOD-4's criterion 11 (comparing a real step's digest against the
  preview's)".** No such criterion was found: `docs/decisions/mod/mod-4.md` and
  `.claude/prds/mod-4-orchestrator-manual-mode.prd.md` state none about a preview digest. The
  criterion 11 that bears on this item is **ANA-5 §12 criterion 11** (`docs/ANA-5.md:2295-2297`:
  `prompt_digest` equals the payload's `digest`), which MOD-33 would *break* without D271. The
  preview-versus-step comparison is still what the fix enables (the preview runs on the TUI's box,
  the step on the executing box). Fact-check sharpening: MOD-4 *does* cite a "criterion 11" — it
  is **ANA-2 §12 criterion 11** (two-repo `worktree` rows, `docs/ANA-2.md:2167`; `mod-4.md:154-155`),
  unrelated to digests. MOD-4's only digest criterion is "`prompt_digest` is unchanged" on promotion
  (`mod-4.md:53`). The cross-box digest worry the item echoes is the Windows-LF note,
  `HANDOFF.md:463-466`.
- **`HANDOFF.md:289-290` — rule 8's "no machine identifier".** Rule 8 does not contain those words
  (`docs/ANA-5.md:1429-1435`); the hostname is discussed in "What varies legitimately" (`:1441-1445`).
  The write-up amends that paragraph (above).
- **`HANDOFF.md:287` — "`app_setting`/`project.settings`".** Resolved to `project.settings` only
  (D267).
- **`docs/ANA-16.md:371-373` — "`box` is `UNIQUE (user_id, hostname)`".** Dropped by
  `0005_box_identity` (MOD-7; `tests/migrations.rs:1713`). Informational; not this item's.
- **`docs/ANA-5.md:1596-1600` — "`ProjectSettings` … gains `upstream_hops`".** It did not; the
  resolvers read the raw blob. D267 follows the code, not the sentence.

## Questions for the maintainer

- **Q-1 (scope).** After MOD-33 a step's digest and a preview's digest match across two boxes only
  when the boxes also agree on `os`, `cpu`, `ram`, `gpu`, `htui` and `tools` (R-1). The item decides
  the hostname alone, and this plan implements exactly that. Is that the intended scope, or should
  the cross-box comparison the item motivates be a follow-up (e.g. a separate `spec_digest`, ANA-5
  Open for the maintainer 10)? Recommendation: hostname only, as decided; the rest is legitimate
  variation per §4.7.

Override points at CONFIRM (decided here, not open): **D272** (the judge digests the hostname through
its replayed task) and **D273** (the comment-only migration).

---

## Claims to verify

Every claim is a statement about the tree at `f5de3d4`.

1. `render::box_profile` is at `render.rs:374` and its first pushed line is `hostname: {}` at `:376`.
2. `assemble` canonicalises and hashes at `mod.rs:509-511`, substitutes at `:507`, and runs the residue scan at `:484-490`.
3. `AssembledPrompt`'s doc says `text` "is what is sent, what is digested and what is persisted" (`mod.rs:324-326`); `assemble` is its only constructor.
4. `scrubbed_inputs` masks `profile.hostname` unconditionally (`mod.rs:767-780`, value at `:772`).
5. `Recorder::record_prompt` hashes the scrubbed payload text at `record.rs:664` and sets `prompt_digest` and `digest_pending` at `:666-667`; `sync_step` (`:1162-1173`) writes it through `set_step_usage`.
6. The only production `record_prompt` callers are `Engine::open_recorder` (`engine.rs:5402`) and `agent_worker.rs` `run_chat` (`:3454`); only the former records an assembled prompt.
7. The only production `set_step_prompt` call sites are `engine.rs:3184`, `:3768`, `:4597`.
8. `{{box}}` is allowed in the `Phase` role only (`template.rs:188-215`); the judge and handoff default bodies do not place it; the eight phase bodies do.
9. `SectionName::Box` is protected (`mod.rs:301-307`) and the trimmer never re-renders it.
10. `canonical()` rewrites only CR/CRLF, a leading BOM, LF runs ≥3, and end-of-string whitespace (`digest.rs:36-62`).
11. `TokenEstimator::estimate` counts characters per line (`estimate.rs:114-148`), and `Trimmer::new` estimates `render::wrap(&rendered)` per section (`trim.rs:417-446`).
12. `TrimRecord` derives `Serialize` and not `Deserialize` (`trim.rs:184`); `RECORD_VERSION` is 3 (`trim.rs:59`); `record()` is at `trim.rs:1080-1111`.
13. No production code deserialises `run_step.trim_record` into a type; the Runs projection reads only `estimated_after` and `sections[].trimmed` (`pg/read.rs` `runs`, `model::prompt_summary`).
14. `to_value_is_byte_stable_and_carries_the_documented_keys` pins thirteen keys and `v == 3` (`prompt_digest.rs:950-1010`).
15. `step_impl_carries_the_golden_trim_record` (`crates/htui-core/src/fixtures.rs:2512-2536`) compares the demo literal to the live record with only `v` patched.
16. `SettingKind` has exactly two variants; `SettingKey` has ten, declared in key byte order, discriminant-indexed into `SPECS` (`settings.rs:150-212`, `:277-282`, `:320-431`).
17. `SettingKey` derives no `Serialize`/`Deserialize` and is never persisted by discriminant.
18. `Defaults::as_rows` iterates `SettingKey::ALL` and is pinned to `0002`'s ten rows (`settings.rs:88-93`, `:807-823`).
19. `prompt_settings::snapshot` reads `setting(SettingRung::App, key)` for every key of `SettingKey::ALL` (`crates/htui/src/prompt_settings.rs:141-149`), and `setting` on a rung the spec refuses returns an error (so a project-only key there would fail the read).
20. `PgStore::set_setting`'s Project branch binds the key name (`jsonb_build_object($2::text, $3::jsonb)`, `pg/write.rs:3142-3276`); `MemStore` and `PgStore` `stored_setting` / `clear_setting` are generic over `project_key` — so a new Project-rung key needs no SQL and no `.sqlx` entry.
21. The only exhaustive matches over `SettingKey` are `Defaults::value_of`, `Defaults::integer` (`settings.rs`) and `effective` (`ui/tabs/settings/prompt.rs:938`); the only exhaustive match over `SettingKind` outside `settings.rs` is `submit` (`prompt.rs:494-550`).
22. `ProjectSettings` has no `upstream_hops` field and the prompt resolvers read the raw `project.settings` blob (`model/kind.rs:257-280`; `settings.rs:627-715`).
23. `PromptSpec` has no `Default`; its full struct literals (no `..base`) are at `prompt/fixtures.rs:161`, `:260`, `:363`, `:502`; `engine.rs:4421`, `:5128`; `preview.rs:279`; `htui-agent/tests/excerpt.rs:1095`, `:1434`; and possibly `promote.rs:416`, `prompt_digest.rs:384`, `:575`, `:607`, `prompt_skills.rs:29`, `:45` — the compiler is authoritative.
24. `promote::handoff_spec` spreads `..phase` (`promote.rs:88-106`) and `step_pass` spreads `..spec.clone()` (`htui-agent/src/excerpt.rs:1142-1144`).
25. `judge_prompts` replays the first survivor's seq-0 payload `text` as the task (`engine.rs:4330-4346`).
26. `preview::offered` excludes the `judge` and `handoff` names from the picker.
27. `OpeningPath::Handoff`'s `digest` has no production reader (`agent_worker.rs:932` binds `{ text, .. }`).
28. No child-box code exists (no `parent_box`, no box kind); MOD-44 is open (`HANDOFF.md:608`).
29. The golden tests snapshot `prompt.text` only (`prompt_golden.rs:58`, `:116`, `:124`, `:132`); the hostnames in the core fixtures are `dev-win-01` (10 chars) and `ci-linux-01` (11 chars) (`prompt/fixtures.rs:44`, `:279`).
30. `prompt_preview__preview_feat_1.snap:11` and `backlog__detail_prompt.snap:11` display a computed assembled digest (`b9fb821f…`), not a hand-written constant.
31. `0002_agent_probe.sql:49-52` states the digest is over the text "as sent"; `0008_trim_record_v3.sql` exists only to restate a comment; both comments are pinned verbatim by `tests/migrations.rs` (`:184-228`, `:438`).
32. The migration-count pins are `tests/connect.rs:142`, `:158`, `:244` and `tests/migrations.rs:93`, `:1176`, and nothing else in the workspace pins the count or the schema target at 9.
33. `HANDOFF.md:47-51` names `0010` as the next migration; `.sqlx` holds 289 files.
34. ANA-5 §12 criterion 11 is at `docs/ANA-5.md:2295-2297` and requires `prompt_digest` to equal the payload's `digest`.
35. ANA-5 §4.7 rule 8 (`docs/ANA-5.md:1429-1435`) does not contain the words "machine identifier"; the hostname is discussed at `:1441-1445`.
36. `docs/decisions/mod/mod-2.md:374-377` records that the MOD-33 amendment goes in the write-up and HANDOFF, not in `docs/ANA-5.md`.
37. No MOD-4 decision doc or PRD states a criterion comparing a preview digest with a step digest.
38. The highest decision ID in `.claude/plans/*` is D262.
39. `gethostname` is the only source of `box.hostname` and never yields a newline (`htui-store/src/identity.rs:330`).
40. `crates/htui/src/ui/tabs/backlog/detail/prompt.rs:151` renders `assembled.digest`.

## Verified claims

Three parallel checkers, 2026-09-30, against HEAD (`f5de3d4` + this plan). Line drift of ≤ 2 lines
is TRUE with the corrected line in the evidence. Every FALSE / PARTLY row is folded into the plan
text above.

| # | Claim | Verdict | Evidence |
|---|---|---|---|
| 1 | `box_profile` at `render.rs:374`, hostname line `:376` | TRUE | `render.rs:374` fn, `:376` `lines.push(format!("hostname: {}", …))` |
| 2 | `assemble` substitute `:507`, canonical/hash `:509-511`, scan `:484-490` | TRUE | `mod.rs:507`, `:510`, `:511`; scan loop `:484-490` |
| 3 | `AssembledPrompt` doc; `assemble` sole constructor | TRUE | doc `mod.rs:322-326`; only literal `mod.rs:521` |
| 4 | hostname masked unconditionally | TRUE | `mod.rs:770-780`, `&mut profile.hostname` `:772`, `mask` `:779` |
| 5 | recorder hashes scrubbed payload text; `sync_step` writes it | TRUE | `record.rs:655-667`; `sync_step` `:1162-1173` |
| 6 | two production `record_prompt` callers | TRUE (drift) | `engine.rs:5403`, `agent_worker.rs:3455`; others `cfg(feature = "test-support")` |
| 7 | three production `set_step_prompt` sites | TRUE | `engine.rs:3184`, `:3768`, `:4597` |
| 8 | `{{box}}` Phase-only; eight phase bodies | TRUE | `template.rs:188-215`; `defaults.rs:34…164`; judge `:177`, handoff `:200` lack it |
| 9 | box protected, never re-rendered | TRUE | `mod.rs:301-307`; `trim.rs:477-481` `Source::Fixed`; no `TrimStep` names Box |
| 10 | `canonical()`'s rewrites | TRUE (range) | `digest.rs:36-64` |
| 11 | estimator chars-based; `Trimmer::new` estimates `wrap` | TRUE (nuance → D265) | `estimate.rs:114-160` rounds per code/prose span; `trim.rs:427` |
| 12 | `TrimRecord` Serialize-only; v 3; `record()` | TRUE | `trim.rs:184`, `:59`, `:1080-1111` |
| 13 | no typed reader of `trim_record` | TRUE (citation) | PG `pg/read.rs:364-388`, **plus the SQLite twin `cache/read.rs:559-565`**; `run.rs:761` |
| 14 | thirteen-key pin, `v == 3` | TRUE | `prompt_digest.rs:991-1010` |
| 15 | demo trim-record test patches only `v` | TRUE | `fixtures.rs:2512-2536`, `live["v"] = 2` `:2531` |
| 16 | registry shape | TRUE | `settings.rs:150-172`, `ALL` `:176`, `SPECS[self as usize]` `:192`, `SPECS` `:320-431`, `SettingKind` `:277-282` |
| 17 | `SettingKey` not persisted | TRUE | derives `Debug, Clone, Copy, PartialEq, Eq, Hash` only (`:150`) |
| 18 | `as_rows` over `ALL`, `0002` pin | TRUE | `settings.rs:88-93`, `:807-823`; other caller `htui/tests/prompt_settings.rs:536` stays green |
| 19 | snapshot reads App for every key; refusal errors | TRUE | `prompt_settings.rs:141-149`; `mem.rs:6094-6099`, `pg/write.rs:3368-3373` |
| 20 | stores generic over `project_key`; no SQL change | TRUE | `pg/write.rs:3211` binds name; `:3289`; `pg/read.rs:2542-2590`; `mem.rs:3251`, `:3332`, `:3436`; no CHECK on `project.settings` |
| 21 | exhaustive matches over `SettingKey` / `SettingKind` | TRUE | `settings.rs:101`, `:124`, `:499`/`:511`; `prompt.rs:938`, `:534-547` |
| 22 | `ProjectSettings` lacks `upstream_hops`; raw-blob resolvers | TRUE | `kind.rs:257-280`; `settings.rs:627`, `:637`, `:690` |
| 23 | `PromptSpec` literal sites | **PARTLY** | real literals: `prompt/fixtures.rs:161,260,363,502`, `engine.rs:4421,5128`, `preview.rs:279`. `htui-agent/tests/excerpt.rs:1095,1434`, `promote.rs:416`, `prompt_digest.rs`, `prompt_skills.rs` are **not** literals (dropped from the Files table); `mod.rs:541`, `:1365` spread |
| 24 | `handoff_spec` and `step_pass` spread | TRUE | `promote.rs:105`; `excerpt.rs:1142-1145` |
| 25 | judge replays payload text | TRUE (drift) | `engine.rs:4327-4341`, into `judge_inputs` ~`:4452` |
| 26 | preview picker drops reserved names | TRUE | `preview.rs:56`, `:327-336`, `:348-349` |
| 27 | handoff digest unread | TRUE | built `engine.rs:1329-1332`; `agent_worker.rs:932`, `run_worker.rs:2236` ignore it; only reader test-support conformance |
| 28 | no child-box code | TRUE | 0 hits for `parent_box`/`child_box`/`BoxKind`; `HANDOFF.md:608` |
| 29 | goldens snapshot text only; fixture hostnames | TRUE | `prompt_golden.rs:58,116,124,132`; `fixtures.rs:44` (10), `:279` (11) |
| 30 | `crates/htui` snapshots showing a computed digest | **PARTLY** | **three**, not two: `+ prompt_preview__preview_ana_2.snap:11` (folded into fact 11, Files, T2 step 8) |
| 31 | `0002`/`0008` comments and their pins | TRUE | `0002_agent_probe.sql:49-52`; `0008` 18 lines; `migrations.rs:185-349`, `:438` |
| 32 | migration-count pins | **FALSE** | lines cited were messages; asserts at `connect.rs:140,155-156,242`, `migrations.rs:88,1174`; **missed** `migrations.rs:901-902,1000,1005,1024` and the three chain sites (folded into T3) |
| 33 | next migration `0010`; 289 `.sqlx` | TRUE | `HANDOFF.md:47-51`; 289 files; no `0010_*` on any host branch |
| 34 | ANA-5 §12 criterion 11 | TRUE | `docs/ANA-5.md:2295-2297` |
| 35 | rule 8 wording | TRUE | `ANA-5.md:1429-1435`; phrase only in `HANDOFF.md:286,290` |
| 36 | `mod-2.md` amendment placement | TRUE | `docs/decisions/mod/mod-2.md:367-377` |
| 37 | no MOD-4 preview-digest criterion | TRUE | MOD-4's "criterion 11" is ANA-2's (worktree); sharpened in "disagree" |
| 38 | highest D is D262 | TRUE | `mod-23…blueprint.md:1158`, `mod-64…blueprint.md:1326`, `docs/decisions/mod/mod-23.md:10`; D263+ unused on every host branch (D249–D262 are already double-used by MOD-23/MOD-64 — not this plan's problem) |
| 39 | `gethostname` only source, never a newline | **PARTLY** | production source `identity.rs:329-336`; demo/test seeds too; nothing in code forbids a newline (Linux `sethostname` does not validate) — which is why D266 stays |
| 40 | backlog detail renders `assembled.digest` | TRUE | `ui/tabs/backlog/detail/prompt.rs:151` |

**Design checks** (not numbered claims):

| Check | Verdict | Evidence |
|---|---|---|
| D263: two substitutions differ only in the hostname value | TRUE | `parsed.used` dedups (`template.rs:260-261`, `:306`), so one `Rendered` feeds every `{{box}}` slot; span is preceded by `hostname: ` and followed by `\n` + `os:` (`render.rs:377`), never at string start/end, LF-run counter 0 at both edges. Caveat → D269: sent box must be *masked*, not only scanned |
| D265: no existing invariant breaks when the estimate sees the digest form | TRUE | `SectionEntry` is `{name, tokens, trimmed}` (`trim.rs:151-159`); no test re-estimates `prompt.text`; H-1 test (`prompt_digest.rs:713-735`) is about the frame scrub |
| D266: reuse `render::attr`'s collapse | **FALSE** | per-character (`render.rs:148`) → `\r\n` becomes two spaces; D266 now defines its own CRLF-first rule |
| D271: `record_prompt_digesting` fits the recorder | TRUE | one digest into `prompt_digest` + `digest_pending` (`record.rs:646-680`, `:1013`); `sync_step` unchanged; judge has no box so its two strings are equal |
| D267/D276: no store/trait/SQL/`.sqlx` change | TRUE | every store path is rung check → `validate` → `project_key`; Phase-branch integer parse unreachable for a `PROJECT`-only key; conformance suites use specific keys |
| T0 ∩ T1 ∩ T2 ∩ T3 file sets | disjoint (T0, T1, T3 pairwise); T2 after T0+T1 | T1's extra test adaptations and all T3 pins lie inside their own files; T2 gained `htui-orch/src/conformance.rs` (docs), touched by no other task |
| T0 case 4 precedent | **FALSE** | `scrub_residue_refuses_write` refuses a tool_result; case rewritten |
