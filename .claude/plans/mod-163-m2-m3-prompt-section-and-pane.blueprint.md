# Blueprint: MOD-163 M2 + M3, prompt section, memory pane and capture switch (T1–T5)

> Written by `code-architect` on 2026-11-08 from the confirmed plan
> `.claude/plans/mod-163-m2-m3-prompt-section-and-pane.plan.md` (D11–D23, Verified claims C1–C14 and S1–S9).
> Tree read at `239b0f66` on branch `hr/MOD-163`, repo root `/home/mluigi/projects/htui`.
> The plan is still the spec. §0 lists where the tree disagrees with the plan, or where the plan leaves a gap. Every row
> there is **adopted**, except P-1 and P-7, which carry a default and ask the maintainer to confirm. None of them reopens D11–D23.
> Line anchors are approximate (`~`) where a file is long. **Implement by symbol name**: the anchors exist so a
> reader can find the right place, not as an exact contract.

## Maintainer decisions (2026-11-08) — these OVERRIDE the defaults below

- **P-1 → the freeze moves to `graph::resolve`.** The memory read and `select` run in `graph::resolve`
  (`graph.rs:379`), only on `Resolution::Resolved`, and set `snapshot.memory` there. They are **not** in
  `resolve_or_refused`. `resolve_or_refused`'s literal keeps `memory: None`. A v1-snapshot resume
  (`resume_window` → `resolve_or_refused`) therefore never reads memory and cannot fail on a memory read.
  No signature changes. The read stays `Box::pin`ned. §T2.2 is read with this substitution, and the
  unit test calls `graph::resolve`. Add a test showing a v1 resume does not call `memory_entries`, or,
  if a spy is not practical, show that `resolve_or_refused`'s snapshot has `memory: None`.
- **P-7(b)/O-1 → an amend clash offers only "edit it".** When `MemorySubjectTaken` answers an
  **amend** (the draft's target is `Amend`), the clash mode offers `MemoryClashEdit` and `Back` only:
  - `MemoryClashReplace` is not handled there, and its hint is not shown;
  - use a separate stack `MEMORY_CLASH_AMEND` (view layer `[Act::MemoryClashEdit]`, `BACK`, `MODAL`), added to `VIEW_STACKS` and `DECLARED` ("in the Memory subject question (amend)"), so `DECLARED` goes to 91;
  - the question text for an amend clash is `"An entry with this subject exists: edit it?"` (`CLASH_QUESTION_AMEND`).
  - Replace (supersede) is offered only for a **new** entry's clash. Tests: `an_amend_clash_offers_only_edit` (`r` sends nothing; the hint has no `replace`) replaces the amend half of P-7(b).
  - P-7(a), excluding the amended entry's own id from the clash search, stands.

## 0. Plan deltas (Blockers / plan deltas)

No blocker: every decision can be built as written. The deltas below are evidence-backed precisions.

| # | Finding | Evidence | Resolution |
|---|---|---|---|
| P-1 | D12 says the read sits in `resolve_or_refused` so that "neither a refused resolution nor a **v1-resume** resolution pays for the read". That holds for a *refused* v1 resume only. `resume_window` calls `resolve_or_refused`, and its `Ok(Resolution::Resolved(live))` arm reaches the read. It then uses only `live.snapshot.phases`. The cost is one discarded read. A memory-read *error* also now fails such a resume. | `engine.rs:4301-4318` (the `Resolved` arm uses only phases); `graph.rs:597-626` | **Default, as written:** the read goes in `resolve_or_refused` after `check_fan_out_caps`, beside `store.repos`, behind `Box::pin`. **For the maintainer:** moving the freeze into `graph::resolve`, after `Resolution::Resolved`, delivers D12's stated goal exactly and changes no signature. T2 implements the default unless told otherwise. |
| P-2 | D14 says "`None` or a wholly trimmed section … writes no `sections[]` row". `trim.rs`'s module invariant says a section dropped by the trimmer keeps its row ("This was dropped" is the fact a reader needs), and `estimated_before` sums `tokens_before` over the rows. | `trim.rs:22-24` (module doc), `:670-677` (`drop_section`), `:1140-1141` | **Adopted reading.** The *text* is byte-identical in both cases. `None` (or a snapshot with no entries) writes no row. A section the trimmer removes keeps a `{tokens_after: 0, strategy: dropped, trimmed: true, dropped: Some(n)}` row. |
| P-3 | D11 says the field is `None` only "when the project has no live entry". `select` can therefore return `Some` with an empty `entries` list, when every active entry is over the cap. | D11, D12 | **Adopted:** `select` → `None` iff no active entry. `render::memory` → `None` when shown is 0, so there is no section, no row and no separator. The trim record still carries `memory: {shown: [], total: n}`, which shows that the cap removed everything. |
| P-4 | "Takes each entry whose rendered block still fits under `MEMORY_TOKEN_CAP`" does not say what is measured. | D12, D13 | **Adopted:** the cap bounds the **sum of the per-entry `<entry>` block estimates**. The wrapper, the preamble, the joining LFs, the left-out line and the separator sit outside the cap: a fixed overhead of about 80 tokens (the §T1.9 example measures it). |
| P-5 | The Judge and Handoff trim orders have no `Memory` step. A judge or handoff spec that carried memory would get an unprotected section the trimmer can never take. A judge could also be refused over text it never shows (the MOD-33 D269 precedent). | D16, D18; `trim.rs:357-375` | **Adopted (belt and braces for D18):** `assemble` renders, scrubs and audits `spec.memory` only when `spec.role == TemplateRole::Phase`. One helper, `memory_of`. Specs still set `None` for handoff and judge as D14 says. |
| P-6 | D21's request and reply shapes are a sketch. A reply needs the project (a pane reopened on another project must ignore a late reply). A write needs `all`, so its re-read matches the toggle. A stale reply needs the head, because the head of a closed entry is not in the live list. | D21; `requirements.rs` `RequirementWritten`, `RequirementsStale` | **Adopted:** see the exact shapes in §T4.2. Every name stays D21's. |
| P-7 | An amend clash: (a) the clash search must exclude the amended entry's own id, which matches its own folded subject otherwise; (b) D21's "Replace sends `SupersedeMemoryEntry`", taken literally, supersedes the **clashing** entry Y with the typed text and leaves the edited entry X as it was. | D21; M1 D4 | **(a) Adopted. (b) Default = the literal reading:** in an amend clash, Replace supersedes Y and the notice says X is unchanged. **Open point O-1 for the maintainer:** offer only "edit it" in an amend clash. |
| P-8 | "Suspect entries never reach a prompt" cannot be produced through `WriteStore` in M1: nothing sets `suspect` (M1 D4). | `model/memory.rs:31-45` | **Adopted:** T1's `select_takes_active_entries_only` covers suspect on a hand-built `MemoryEntry`. The orch case covers retired and superseded. |
| P-9 | "The handoff prompt has none" is not observable through orch `CASES`: the handoff text exists only on `CommandOutcome::Promoted { opening: OpeningPath::Handoff { text, .. } }`. | `engine.rs:17520-17575` (the precedent test), `promote.rs:470-473` | **Adopted:** an `engine.rs` unit test, `a_handoff_opening_carries_no_memory_section`, plus T1's promote and role-guard tests. CASES go **180 → 186** (six cases). |
| P-10 | Three of the plan's validate commands do not do what they intend. (1) `cargo test -p htui-core --all-features prompt` filters by test-function **name**, so most `tests/prompt_memory.rs` tests (`select_…`, `the_…`) never run. (2) `cargo test -p htui --all-features --lib keys memory` passes two positional filters, which cargo refuses. (3) A crate-wide `cargo insta test -p htui` on a shared tree sees the other lanes' pending snapshots. | cargo and libtest CLI; memory "Parallel fan-out hidden coupling" | **Adopted:** the per-task commands in §T1.13 / §T2.6 / §T3.5 / §T4.9 use `--test <bin>` and `-- <filter> <filter>`, and scope insta runs with `--test`. |
| P-11 | T2's file set is missing three snapshot files. The 9th `STAND_INS` note is rendered in them. | `git grep "preview: attempt 1" -- '*.snap'` → `backlog__detail_prompt.snap`, `prompt_preview__preview_ana_2.snap`, `prompt_preview__preview_feat_1.snap` | **Adopted:** T2 owns those three files. They do not intersect T3's or T4's sets. |
| P-12 | `tests/migrations.rs` holds `MOD33_COLUMN_COMMENTS` (`run_step.trim_record`, text v 4) in three chains (`:1059-1071`, `:1131-1143`, `:1153-1165`). Once 0026 restates the comment, the v4 row fails the verbatim check. Kept beside a new row, it also fails the "exactly the listed commented columns" check (a duplicate `(run_step, trim_record)`). | `tests/migrations.rs:456-481, 1059-1175` | **Adopted:** the `trim_record` row **moves** out of `MOD33_COLUMN_COMMENTS`, which keeps `prompt_digest`, into `MOD163_M2_COLUMN_COMMENTS`. That constant joins all three chains. The `ANA_COLUMN_COMMENTS` doc at `:227-230` is updated. |
| P-13 | `memory_capture` has a `PROJECT` rung, so `project_keys()` grows from four to **five**, and `app_keys()` / `as_rows()` grow from ten to **eleven**. | `prompt_settings.rs:109-118`; `settings.rs:95-100, 211-215`; `tests/prompt_settings.rs:161-187` | **Adopted:** the pins are restated in §T3.3, including `project.values[3]` → `[4]` for `token_budget`. |
| P-14 | Keys: (a) the pane captures input (`HierarchySection::captures_input` is `!Browse`), so its stacks carry **no** `SETTINGS` layer and need no `DELEGATES` entry; (b) `views.rs`'s `VIEWS` source list must include the new pane file, or the handled-act check fails; (c) the capturing-stack pin gains two names; (d) the count pins move: `Context::ALL` 51→52, `CATALOGUE` 217→222, `DECLARED` 85→90. | `views.rs:1120-1166, 1752-1786, 1810-1836, 2070-2161`; `catalogue.rs:2441, 2954`; `stack.rs:428` | **Adopted**, §T4.6. |
| P-15 | `SnapshotPersona` has `#[serde(deny_unknown_fields)]`. `GraphSnapshot`'s own doc requires a snapshot written by a later builder to decode, and MOD-164 and MOD-166 will add entry fields. | `run.rs:574-577`; `persona.rs:198-201` | **Adopted:** `SnapshotMemory` and `SnapshotMemoryEntry` do **not** deny unknown fields. |
| P-16 | `fixtures.rs:2781` (`step_impl_carries_the_golden_trim_record`): the demo record carries `memory: None`, so the live v5 record has no `memory` key to strip. | `fixtures.rs:2770-2789` | **Adopted:** comment-only edit ("assembler writes v 5"). |
| P-17 | The plan's T1 file set omits `model/mod.rs` (the `pub use memory::{…}` re-export). | `model/mod.rs:146-150` | **Adopted:** T1 edits it. T1 is serial, so no lane conflicts. |

---

## 1. Shared decisions for every task

- **Names.** `SnapshotMemory`, `SnapshotMemoryEntry` and `MemorySource` live in `htui_core::model::memory`, re-exported from `htui_core::model`. `select`, `MEMORY_TOKEN_CAP` and `MEMORY_PREAMBLE` live in `htui_core::prompt::memory`. `MEMORY_SEPARATOR` lives in `htui_core::prompt` beside `PERSONA_SEPARATOR`. `MemoryAudit` and `MemoryShown` live in `htui_core::prompt::trim`, re-exported from `htui_core::prompt`.
- **Order.** Order is `updated_at DESC, id ASC`, using `MemoryEntryId`'s `Ord` (UUID byte order). `select` re-sorts and never trusts the store's order.
- **Live in M2 means `Active`.** The engine and the preview read `Some(vec![MemoryState::Active])`, and `select` filters `state == Active` again defensively. `total` counts those entries.
- **Render order equals snapshot order equals trim tail.** Trimming drops `entries[k-1]`, the oldest kept. The kept set is always a prefix, so the trimmer needs only a count.
- **The separator belongs to the memory span.** Every per-section estimate of `SectionName::Memory` is `est(wrap(section) + MEMORY_SEPARATOR) × weight`. The frame estimate (`mod.rs:583-591`) does not change.
- **Scrubbing.** Subjects and bodies are masked once, in `scrubbed_inputs`, under the section name `memory`, and only for a Phase spec (P-5). Every render and re-render reads the masked spec.
- **No store, clock or map order** reaches `select` or the render. Both are pure.

---

## T1 — Section, snapshot type, select, render, trim, v5 (serial)

### T1.1 `crates/htui-core/src/model/memory.rs` (UPDATE)

Add these after `MemoryState` (~:53). Add `MemoryEntryId` to the `use` line; it is already imported.

```rust
str_enum!(
    /// Who wrote a memory entry, as a run snapshot records it (MOD-163 D15). M2 freezes `person`
    /// only (M1 D9: every entry is a person's); MOD-164 adds `run`.
    MemorySource {
        /// A person: the Settings pane, the CLI or the MCP note tool.
        Person => "person",
    }
);

impl MemorySource {
    /// The `trust` attribute the memory section renders (D15): `person-written` today. MOD-164's
    /// `run` source is `model-written`; MOD-166 adds `pinned`.
    #[must_use]
    pub const fn trust(self) -> &'static str {
        match self {
            Self::Person => "person-written",
        }
    }
}

/// One memory entry as a run froze it at `StartRun` (MOD-163 D11): a **copy of the text**, so a
/// retry or a resume renders the same bytes after the entry is amended or retired (`R-PRM-1`).
/// No `deny_unknown_fields` (blueprint P-15): a later builder's entry must still decode here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotMemoryEntry {
    /// `memory_entry.id`; recorded in `trim_record.memory.shown`, never rendered.
    pub id: MemoryEntryId,
    /// `memory_entry.version` at run start; recorded, never rendered.
    pub version: i32,
    /// The kind, rendered as `kind="…"`.
    pub kind: MemoryKind,
    /// The subject, rendered through `render::attr`.
    pub subject: String,
    /// The body, rendered verbatim (LF-normalised).
    pub body: String,
    /// Who wrote it; `trust` derives from it.
    pub source: MemorySource,
}

impl SnapshotMemoryEntry {
    /// Freezes `entry` (D11).
    #[must_use]
    pub fn freeze(entry: &MemoryEntry, source: MemorySource) -> Self;
}

/// `GraphSnapshot.memory` (MOD-163 D11): the project's memory as it stood at `StartRun`, already
/// selected under the cap. Outside `topology_v2`, so no `V` bump and no `SnapshotTampered`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotMemory {
    /// The estimator id the cap was measured with (`"chars-v2"`), so a later per-model estimator
    /// cannot drift silently.
    pub estimator: String,
    /// How many active entries the project had at run start (the left-out line's denominator).
    pub total: u32,
    /// The entries that fit the cap, in render order (`updated_at DESC, id ASC`).
    pub entries: Vec<SnapshotMemoryEntry>,
}
```

Unit tests in this file: `memory_source_is_person_and_person_written` (`ALL == [Person]`, `as_str`, `trust`, serde round trip as `"person"`), and `snapshot_memory_round_trips_with_unknown_fields_ignored`.

### T1.2 `crates/htui-core/src/model/mod.rs` (UPDATE, P-17)

`pub use memory::{…}` (~:146-150): add `MemorySource, SnapshotMemory, SnapshotMemoryEntry`.

### T1.3 `crates/htui-core/src/model/run.rs` (UPDATE)

- `GraphSnapshot` (~:578-613), after `personas`:
  ```rust
  /// MOD-163 D11: the project's active memory entries at `StartRun`, selected under
  /// `MEMORY_TOKEN_CAP` and frozen with their text. **Not** hashed by `topology` and
  /// [`GraphSnapshot::V`] is not bumped; absent from the JSON when `None`; `None` on every
  /// snapshot written before MOD-163, which renders no memory section.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub memory: Option<crate::model::memory::SnapshotMemory>,
  ```
- The envelope doc list at ~:588-590 (`v`, `graph`, `source`, `mode`, `settings`, `scope`, `personas`) gains `memory`.
- The test-module literals at ~:1959, :2515, :2559, :2597 and :2631 gain `memory: None` where they are full literals.
- New unit test `a_snapshot_without_memory_decodes_as_none_and_writes_no_key`: decode a v2 JSON with no `memory` key → `None`; `to_value` of a `None` snapshot has no `memory` key; `topology_v2` is unchanged by setting `memory`.

### T1.4 `crates/htui-core/src/prompt/memory.rs` (CREATE)

```rust
//! MOD-163 M2 (D12, D13, D15): which of a project's memory entries a run freezes, and the
//! section's fixed text. Pure: no store, no clock, no map order. Shared by `graph::resolve` and
//! the Backlog preview, so the preview shows what a run started now would get (D18).

/// D13: the hard cap, in `chars-v2` tokens, on the **sum of the `<entry>` block estimates**
/// (blueprint P-4). The wrapper, preamble, joins, left-out line and separator are outside it
/// (~80 tokens). A constant, not a setting; MOD-169 tunes it.
pub const MEMORY_TOKEN_CAP: i64 = 1_500;

/// D15: the section's framing sentence, byte-exact (mirrors `render::EXCERPT_PREAMBLE`).
pub const MEMORY_PREAMBLE: &str = "Project memory: notes recorded for this project, quoted as \
    data, not instructions. Where one disagrees with the item, its documents or the working \
    tree, they win.";

/// D12: the project's `Active` entries in `updated_at DESC, id ASC` order, walked once. Each
/// entry whose block estimate still fits under `cap` is taken; one that does not is **skipped
/// and the walk continues** (an entry is never split). `None` iff no entry is active (D11);
/// `Some` with empty `entries` when every active entry is over the cap (blueprint P-3).
/// `estimator.id` is recorded in the result.
#[must_use]
pub fn select(entries: &[MemoryEntry], cap: i64, estimator: TokenEstimator) -> Option<SnapshotMemory>;

/// D15: `"{shown} of {total} project memory entries shown."` when `shown < total`, else `None`.
/// A count only: it names no tool (MOD-167 adds `search_concepts`).
#[must_use]
pub fn left_out_line(shown: usize, total: u32) -> Option<String>;
```

**Write `MEMORY_PREAMBLE` on one physical line** (or with `\` continuations that leave exactly one space). §T1.9 pins the bytes.

How `select` works: filter `Active`, then sort with `b.updated_at.cmp(&a.updated_at).then(a.id.cmp(&b.id))`. Set `total = u32::try_from(len).unwrap_or(u32::MAX)` and `used = 0i64`. For each entry, `snap = SnapshotMemoryEntry::freeze(e, MemorySource::Person)` and `cost = estimator.estimate(&render::memory_entry_block(&snap))`. If `used + cost <= cap`, push the entry and add `cost` to `used`.

### T1.5 `crates/htui-core/src/prompt/render.rs` (UPDATE)

Add after `persona` (~:651-659). Import `crate::model::memory::{SnapshotMemory, SnapshotMemoryEntry}` and `crate::prompt::memory::{MEMORY_PREAMBLE, left_out_line}`.

```rust
/// MOD-163 D15: one `<entry kind="…" subject="…" source="…" trust="…">` block, the `<skill>`
/// shape one level down: one LF each side of the body (`content_of`), `</entry>` closing. The
/// subject goes through [`attr`]; `<`/`>` stay (rule 4), so a body holding `</entry>` or
/// `</section>` is inert. Ids and versions are never rendered.
#[must_use]
pub fn memory_entry_block(entry: &SnapshotMemoryEntry) -> String;

/// MOD-163 D15: the `memory` section over `memory.entries[..shown]` (clamped to the length):
/// the preamble, one LF, the blocks joined by one LF, then — when `shown < total` — one LF and
/// the left-out line. `None` when `shown` is 0 (no empty section, I-7). No attributes.
#[must_use]
pub fn memory(memory: &SnapshotMemory, shown: usize) -> Option<Rendered>;
```

Block format: `format!("<entry kind=\"{}\" subject=\"{}\" source=\"{}\" trust=\"{}\">\n{body}\n</entry>", kind.as_str(), attr(&subject), source.as_str(), source.trust())`. Use `body = content_of(&entry.body)`. An empty body gives `"{open}\n</entry>"`, which a body can never be (`MemoryBody` is non-empty), but the guard stays.

### T1.6 `crates/htui-core/src/prompt/template.rs` (UPDATE)

- `Placeholder` (~:72-128): add `Memory` **after `Persona`, before `Output`**, with a doc in the style of `Persona`'s: "MOD-163 D14: the memory section. **Internal** … a body that writes `{{memory}}` is refused as an unknown placeholder. `assemble` renders it after the persona and before the body."
- `token()` (~:165-190): add the arm `Self::Memory => "memory"`. This match is exhaustive.
- `ALL` (~:140-161): unchanged. Its doc names the three internal variants.
- `allowed_in` (~:202-231), Phase arm: add `| Self::Memory` to the excluded list. Judge and Handoff are allow-lists and already exclude it.
- `is_section` doc (~:233-236): name `Memory` beside `Persona` and `Output`.

### T1.7 `crates/htui-core/src/prompt/mod.rs` (UPDATE)

| Anchor | Change |
|---|---|
| module list ~:31-39 | `pub mod memory;` (no feature gate) |
| re-exports ~:42-50 | `pub use memory::{MEMORY_PREAMBLE, MEMORY_TOKEN_CAP, select as select_memory};` (the alias avoids clashing with `model::skill::select` in scope); `pub use trim::{MemoryAudit, MemoryShown, …}` |
| imports ~:57-63 | `use crate::model::memory::SnapshotMemory;` |
| `PromptSpec` ~:111-114 | after `persona`: `/// MOD-163 D14/D18: the run's frozen memory, rendered as the internal `memory` section after the persona and before the body. `None` for every judge, handoff, preview-without-entries and fixture spec, and for a run started before MOD-163; renders exactly as before (I-7). Rendered only for a Phase spec (blueprint P-5).` `pub memory: Option<SnapshotMemory>,` |
| after `PERSONA_SEPARATOR` ~:171 | `/// MOD-163 D14 (C8 option a): the blank line between the memory section and what follows it. Counted in the **memory section's own span**, not the frame, so dropping the section drops it and `estimated_after` describes only sent bytes.` `pub const MEMORY_SEPARATOR: &str = "\n\n";` |
| `SectionName` ~:278-316 | `Memory` after `Persona`: `/// MOD-163 D14: project memory. sections[1] without a persona, sections[2] with one (P-9).` |
| `render()` ~:318-341 | `Self::Memory => "memory".to_owned(),` |
| `is_protected` ~:346-358 | no change (`_ => false`); add Memory to the trimmable list in the test |
| new private fn | `fn memory_of(spec: &PromptSpec) -> Option<&SnapshotMemory> { spec.memory.as_ref().filter(\|_\| matches!(spec.role, TemplateRole::Phase)) }` |
| `assemble` ~:536-540 | after the persona loop: `for section in render_sections(Placeholder::Memory, spec, upstream, skills, candidates) { rendered.push((Placeholder::Memory, section, 1)); }` |
| `assemble` ~:600 | `let (sections, live, kept_excerpts, kept_memory) = trimmer.finish();` |
| `assemble` ~:641-650 | build `let memory = memory_of(spec).map(\|m\| MemoryAudit { shown: m.entries.iter().take(kept_memory).map(MemoryShown::of).collect(), total: m.total });` and pass it to `trim::record(…, memory, notes(spec))` |
| `substitute` ~:705-708 | after the persona block: `if let Some(memory) = blocks.get(&Placeholder::Memory) { text.push_str(&memory.join("\n\n")); text.push_str(MEMORY_SEPARATOR); }` with a comment: "`live` carries it only when the memory section survived the trim, so a memory-less prompt is byte-identical (I-7)" |
| `render_sections` ~:841 | `Placeholder::Memory => memory_of(spec).and_then(\|m\| render::memory(m, m.entries.len())).into_iter().collect(),` |
| `scrubbed_inputs` ~:976-980 | after the persona: `if matches!(spec.role, TemplateRole::Phase) && let Some(memory) = &mut spec.memory { let name = SectionName::Memory.render(); for entry in &mut memory.entries { mask(scrubber, &mut entry.subject, &name)?; mask(scrubber, &mut entry.body, &name)?; } }` |
| tests ~:1394-1450 | `section_names_are_one_vocabulary_with_two_uses` adds `assert_eq!(SectionName::Memory.render(), "memory")`; `the_protected_set_is_role_dependent_only_for_failure_reason` adds `SectionName::Memory` to the trimmable list |

### T1.8 `crates/htui-core/src/prompt/trim.rs` (UPDATE)

| Anchor | Change |
|---|---|
| `RECORD_VERSION` ~:56-59 | `5`; doc: "version 5 since MOD-163 D17 added `memory`; 4 since MOD-33 D270 …" |
| `TrimRecord` doc ~:196 | "version 5 (… and MOD-163 D17)" |
| `TrimRecord` fields, after `undigested` | `/// MOD-163 D17: which memory entries reached the prompt after trimming, and how many the project had at run start. Absent when the spec carried none, so a memory-less v5 record differs from v4 only in `v`.` `#[serde(skip_serializing_if = "Option::is_none")] pub memory: Option<MemoryAudit>,` |
| new types (beside `UndigestedSpan`) | see below |
| `TrimStep` ~:329-350 | `/// Phase rank 1b (MOD-163 D16): whole entries from the tail, then the section.` `Memory,` placed after `Excerpts` |
| `trim_order` ~:357-375 | Phase: `[Excerpts, Memory, Upstream, PreviousDiff, VerifyFailure, Documents, Item]`; Judge and Handoff unchanged |
| `Source` ~:382-400 | `/// How many of `spec.memory.entries` are still rendered (a prefix; the tail goes first).` `Memory(usize),` |
| new private fn | `fn span_tokens(est: TokenEstimator, rendered: &Rendered, weight: i64) -> i64 { let mut span = render::wrap(rendered); if rendered.name == SectionName::Memory { span.push_str(MEMORY_SEPARATOR); } est.estimate(&span) * weight }`. Doc: "MOD-163 D14: a section's span — its wrapper, plus the memory section's trailing separator." |
| `Trimmer::new` ~:464 | `let tokens = span_tokens(est, &rendered, weight);` |
| `re_estimate` ~:652-656 | `entry.record.tokens_after = span_tokens(self.est, &entry.rendered, entry.weight);` |
| `section_yardstick` ~:678-689 | `span_tokens(est, &candidate, weight)` (same arithmetic as `re_estimate`, M-3) |
| `source_of` ~:487-522 | `SectionName::Memory => Source::Memory(inputs.spec.memory.as_ref().map_or(0, \|m\| m.entries.len())),` (exhaustive match) |
| `run` ~:584-618 | `TrimStep::Memory => self.trim_memory(),` |
| `trim_diff` ~:818-834 | add `\| SectionName::Memory` to the enumerated `=> None` list |
| `finish` ~:621-642 | return `(Vec<Section>, Vec<(Placeholder, Rendered)>, Vec<usize>, usize)`. The 4th value is the live memory entry's `Source::Memory(k)` when the entry is alive, else `0`. If `clippy::type_complexity` fires, introduce `pub(crate) struct Finished { sections, live, kept_excerpts, kept_memory }` |
| `record` ~:1113-1155 | new 9th parameter `memory: Option<MemoryAudit>` (before `notes`); set `memory` in the literal; the `#[expect]` reason says "nine inputs" |
| test `the_three_orders_are_the_documented_ones` ~:1176 | the new Phase order |

New types:

```rust
/// One memory entry that reached the prompt (MOD-163 D17): what `trim_record.memory.shown[]`
/// holds, in render order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct MemoryShown {
    /// `memory_entry.id`.
    pub id: crate::model::MemoryEntryId,
    /// The version the run froze.
    pub version: i32,
}

impl MemoryShown {
    /// The record row of one frozen entry.
    #[must_use]
    pub const fn of(entry: &crate::model::SnapshotMemoryEntry) -> Self;
}

/// `trim_record.memory` (MOD-163 D17).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MemoryAudit {
    /// What reached the prompt after trimming, in render order; `[]` when the cap or the trim
    /// left nothing.
    pub shown: Vec<MemoryShown>,
    /// How many active entries the project had at run start.
    pub total: u32,
}
```

`trim_memory`, which follows the `trim_upstream` and `trim_excerpts` precedent with the counters set **before** the render (L-1):

```rust
/// MOD-163 D16: whole entries off the **tail** (the oldest), re-rendering the left-out line
/// after each; the last one takes the section and its separator with it.
fn trim_memory(&mut self) {
    let Some(index) = self.live_index(&SectionName::Memory) else { return };
    let Some(memory) = self.inputs.spec.memory.as_ref() else { return };
    let mut removed = self.entries[index].record.dropped.unwrap_or(0);
    loop {
        let Source::Memory(kept) = &mut self.entries[index].source else { return };
        if *kept == 0 { break; }
        *kept -= 1;
        let kept = *kept;
        removed += 1;
        self.entries[index].record.dropped = Some(removed);
        self.mark(index, TrimStrategy::Dropped);
        // `render::memory` rebuilds the left-out line: "k of total" with the new k.
        let Some(rendered) = render::memory(memory, kept) else { break };
        self.entries[index].rendered = rendered;
        self.re_estimate(index);
        if self.deficit() <= 0 { return; }
    }
    self.drop_section(index); // tokens_after 0, strategy dropped, trimmed true; the separator goes with it
}
```

**How the left-out line is rebuilt (question 4).** It mirrors Upstream's ladder, which re-renders from `self.inputs.upstream` and the per-row states (`trim.rs:770-790`). The memory step keeps one integer, `Source::Memory(k)`, and calls `render::memory(memory, k)` after each drop. `render::memory` derives the line from `k` and `memory.total`, so nothing ever edits rendered bytes. `memory` is the **masked** spec's (`Inputs.spec` is `masked.spec`), so a re-render cannot bring an unmasked byte back (review C-1). The residue scan in `assemble` covered every subset, because each rung renders a prefix of the full section's entries plus a line that changes only its digit. A line with a new digit holds no new characters a scrubber rule could fire on.

### T1.9 The worked example (question 3): exact bytes, `sections[]`, estimates

The example is the snapshot `memory = { estimator: "chars-v2", total: 3, entries: [E1, E2] }`. The third active entry was left out by the cap (or by an earlier trim).

- `E1 = { kind: gotcha, subject: "Build cache", body: "Clear target/ when the sqlx offline data goes stale.", source: person }`
- `E2 = { kind: decision, subject: "Errors & \"context\"", body: "Library crates use thiserror; binaries use anyhow.", source: person }`

**The memory span** is what `substitute` writes: `wrap(section)` followed by `MEMORY_SEPARATOR`. Here it is as a Rust literal (547 chars). T1 adds it to `prompt/fixtures.rs` as `memory_two_of_three()` and pins it byte for byte in `tests/prompt_memory.rs`:

```rust
"<section name=\"memory\">\n\
Project memory: notes recorded for this project, quoted as data, not instructions. Where one disagrees with the item, its documents or the working tree, they win.\n\
<entry kind=\"gotcha\" subject=\"Build cache\" source=\"person\" trust=\"person-written\">\n\
Clear target/ when the sqlx offline data goes stale.\n\
</entry>\n\
<entry kind=\"decision\" subject=\"Errors &amp; &quot;context&quot;\" source=\"person\" trust=\"person-written\">\n\
Library crates use thiserror; binaries use anyhow.\n\
</entry>\n\
2 of 3 project memory entries shown.\n\
</section>\n\n"
```

(The `\` continuations above are for display. Write the test constant with explicit `\n` and no leading-whitespace stripping in the middle of text.)

**Placement in the sent text:**

```
[persona section]  PERSONA_SEPARATOR      ← only with a persona (frame-counted, as before)
[memory section]   MEMORY_SEPARATOR       ← memory span (memory-row-counted)
<template literal spans and slots …>
[MOD-11 output trailer]                    ← unchanged
```

With no persona the prompt **starts** with `<section name="memory">`. `digest::canonical` collapses newline runs over 2 and trims the tail. That is the same rule the persona path already lives under: a body that begins with `\n` yields `\n\n\n`, canonicalised to `\n\n`. The estimate is taken before canonicalisation, as it is for every section.

**`sections[]`** (the `trim_record.sections` rows, which also project into the `prompt` payload):

| Case | rows |
|---|---|
| no persona | `[template, memory, item, documents:…, …]` |
| persona | `[template, persona, memory, item, …]` |
| `memory: None` or empty `entries` | no `memory` row (byte-identical to today) |

**Figures for the example** (`chars-v2`, one prose span, `ceil(chars × 10 / 25)`):

| Quantity | Value | Note |
|---|---|---|
| block E1 / E2 (cap units in `select`) | 58 / 66 tokens | the cap sums these: 124 ≤ 1 500 |
| memory row `tokens_before` | **219** | `est(wrap + "\n\n")`; 218 without the separator, so the separator costs 1 |
| after one trim drop (`"1 of 3 … shown."`) | `tokens_after` **153**, `dropped: 1`, `strategy: dropped`, `trimmed: true` | `re_estimate` re-measures the rebuilt line |
| after the last drop | `tokens_after` **0**, `dropped: 2`; text holds neither the section nor its `\n\n` | the row stays (P-2) |
| fixed overhead (wrapper + preamble + LF + separator, no entry) | 80 | outside the cap (P-4) |
| `template` row | unchanged by memory | the frame does not hold `MEMORY_SEPARATOR` |

**`estimated_before`** = template + Σ `tokens_before` and **`estimated_after`** = template + Σ `tokens_after`. Both are the existing sums (`trim.rs:1140-1141`). Because the separator lives in the memory row, a dropped memory section subtracts both its own bytes and its separator from `estimated_after`.

**`trim_record.memory`** for the untrimmed example: `{"shown":[{"id":"<E1>","version":v1},{"id":"<E2>","version":v2}],"total":3}`. After one drop it is `shown: [E1]`. After both drops it is `shown: []`.

### T1.10 Literals and fixtures (compile-green for T2–T4)

- **`PromptSpec` full literals → `memory: None`:**
  - `prompt/fixtures.rs` ×4 (`phase_implement_attempt2` ~:163, `phase_all_empty` ~:265, `judge_three_candidates` ~:371, `handoff_basic` ~:513);
  - `htui-orch/src/engine.rs` ~:6408 (`judge_prompts`, with the comment "MOD-163 D18: a judge carries no memory") and ~:7264 (`phase_spec`: T1 writes `memory: None`, T2 fills it);
  - `htui/src/preview.rs` ~:281 (T1 writes `memory: None`, T2 fills it).
  - `cargo check --workspace --all-targets --all-features` is the authority. `htui-agent/tests/excerpt.rs:1095, :1434` and the `..base` literals need nothing if they spread.
- **`GraphSnapshot` full literals → `memory: None`.** Plan C1 lists 11: `htui-agent/tests/relay.rs:97`, `htui-core/src/fixtures.rs:1455`, `model/run.rs` (test helpers), `store/conformance.rs:5330`, `store/mem.rs:13821`, `htui-mcp/tests/tools_backlog.rs:38`, `htui-orch/src/graph.rs:602`, `htui-store/tests/cache.rs:675, :984`, `pg_criteria.rs:1184`, `htui/tests/backlog.rs:1032`. Others found by search: `store/conformance.rs:5995, :6191`, `htui/tests/backlog.rs:3476`, `schedule.rs:234`, `status.rs:592`, `queue_overview.rs:306`, `tests/review_loop.rs:32`. Those either spread or build through a helper; let the compiler decide.
- **`promote.rs` `handoff_spec` (~:93-113):** add `// MOD-163 D18: a handoff carries no project memory.` and `memory: None,`. The test `the_handoff_spec_keeps_the_phase_s_item_and_budget` (~:462) gets `memory: None` in `expected`. New test `the_handoff_spec_drops_the_memory` (mirrors `the_handoff_spec_drops_the_persona` ~:488).
- **`prompt/fixtures.rs`:** new `#[cfg(feature = "test-support")]` fixtures `pub fn memory_entry(n: u128, kind, subject, body, updated_at) -> MemoryEntry` (fixed `MemoryEntryId::from_uuid(Uuid::from_u128(n))`, fixed timestamps, `ids::USER`) and `pub fn memory_two_of_three() -> SnapshotMemory` (the §T1.9 example).
- **`htui-core/src/fixtures.rs:2770-2789`:** comment only (P-16).
- **`graph.rs` doc ~:807-808:** the envelope list gains `memory`. T1 makes this edit because it touches the literal.

### T1.11 Migration `crates/htui-store/migrations/0026_trim_record_v5.sql` (CREATE)

```sql
-- 0026_trim_record_v5.sql - MOD-163 milestone 2 (plan D17).
-- Forward-only (R-STO-5).
--
-- run_step.trim_record is restated for record v 5, which gains memory {shown[], total} when a
-- phase step's run froze project memory, with shown[] empty when the cap or the trim left none.
-- Comment only: no table, column, constraint or index moves, and the SQLite mirror's schema is
-- untouched. schema_version still becomes 26, so each box rebuilds its mirror once on first
-- start (0008's R-56). MOD-147's sandbox also writes a 0026: whichever merges second renumbers
-- at collect, with the count pins.

COMMENT ON COLUMN run_step.trim_record IS
  'ANA-5 5.1 as amended by MOD-9 D42 and D118, MOD-33 and MOD-163: {v, template, budget, '
  'budget_source, reserve, target, estimator, estimated_before, estimated_after, sections[], '
  'skill_choices[], undigested[], excerpts, notes}, plus memory when a phase step''s run froze '
  'project memory, v 5. memory is {shown[], total}: shown[] is each memory entry that reached '
  'the prompt after trimming, as {id, version} in render order, empty when the cap or the trim '
  'left none, and total is how many active entries the project had when the run started. '
  'undigested[] names every span rendered into the prompt but excluded from prompt_digest: '
  'box.hostname, or empty. skill_choices[] is every candidate skill, ordered by position then '
  'name, each {skill, name, version, level, activation, active, reason} with reason always, '
  'matched, no_match, off, no_path, missing_version or not_placed (ANA-22 6 item 8); a matched '
  'choice adds path, <repo>:<path>, the first matching file in repo then path byte order. A v 4 '
  'record, written before 0026, has no memory and its prompt carried none; a v 3 record, '
  'written before 0010, has no undigested and digested the hostname; a v 2 record, written '
  'before 0008, has no matched or no_match; a v 1 record, written before 0007, has no '
  'skill_choices. Canonical; the prompt payload sections[] array is its abridged projection. '
  'Written at stage 3 by set_step_prompt, before the session starts.';
```

*Amended 2026-11-09 (review R1, R1-2):* the text said memory is present "when the step's prompt
carried project memory". The assembler writes `memory {shown: [], total: n}` whenever a phase
step's run froze project memory, even when the cap or the trim left no entry in the prompt, so the
comment now says so (above and in the Rust constant below).

A script asserted that the SQL fragments concatenate (with `''` → `'`) to the Rust constant's logical string, byte for byte.

**`crates/htui-store/tests/migrations.rs`:**

- New constant, placed after `MOD163_COLUMN_COMMENTS` (~:831):
  ```rust
  /// The one `COMMENT ON COLUMN` of `0026_trim_record_v5.sql` (MOD-163 plan D17), verbatim, for
  /// [`ANA_COLUMN_COMMENTS`]'s reason. It restates `run_step.trim_record`, so the v 4 text left
  /// [`MOD33_COLUMN_COMMENTS`] (blueprint P-12).
  const MOD163_M2_COLUMN_COMMENTS: &[(&str, &str, &str)] = &[(
      "run_step",
      "trim_record",
      "ANA-5 5.1 as amended by MOD-9 D42 and D118, MOD-33 and MOD-163: {v, template, budget, \
       budget_source, reserve, target, estimator, estimated_before, estimated_after, \
       sections[], skill_choices[], undigested[], excerpts, notes}, plus memory when a phase \
       step's run froze project memory, v 5. memory is {shown[], total}: shown[] is each memory \
       entry that reached the prompt after trimming, as {id, version} in render order, empty \
       when the cap or the trim left none, and total is how many active entries the project had \
       when the run started. undigested[] names every span rendered into the prompt but \
       excluded from prompt_digest: box.hostname, or empty. skill_choices[] is every candidate \
       skill, ordered by position then name, each {skill, name, version, level, activation, \
       active, reason} with reason always, matched, no_match, off, no_path, missing_version or \
       not_placed (ANA-22 6 item 8); a matched choice adds path, <repo>:<path>, the first \
       matching file in repo then path byte order. A v 4 record, written before 0026, has no \
       memory and its prompt carried none; a v 3 record, written before 0010, has no undigested \
       and digested the hostname; a v 2 record, written before 0008, has no matched or \
       no_match; a v 1 record, written before 0007, has no skill_choices. Canonical; the prompt \
       payload sections[] array is its abridged projection. Written at stage 3 by \
       set_step_prompt, before the session starts.",
  )];
  ```
- `MOD33_COLUMN_COMMENTS` (~:456-481): **remove** the `trim_record` row and keep `prompt_digest`. Its doc becomes "The `prompt_digest` text of 0010 …; its `trim_record` text was restated by 0026 and is pinned in `MOD163_M2_COLUMN_COMMENTS`".
- The `ANA_COLUMN_COMMENTS` doc (~:227-230): `run_step.trim_record` is now pinned in `MOD163_M2_COLUMN_COMMENTS`.
- Add `.chain(MOD163_M2_COLUMN_COMMENTS)` to the three chains (~:1071, ~:1143, ~:1165). Add "MOD-163 M2" to the assertion messages that list the sources (~:1086-1088, ~:1119-1121).
- Count pins:
  - `migrations_apply_on_a_clean_database` (~:115-130): append `26` to the vector, and add `…, MOD-138 M2's 0025_run_step_branch.sql and MOD-163 M2's 0026_trim_record_v5.sql, in ordinal order` to the text.
  - The lines ~:3907-3908, ~:4008-4009, ~:4017-4018, ~:4040-4041 and ~:4193-4194: `Pending(25)` / `MigrationsPending(25)` → `26`, and "twenty-five" → "twenty-six … through MOD-163's 0026_trim_record_v5.sql".
- **`crates/htui-store/tests/connect.rs`** ~:140-142, ~:156-158 and ~:242-244: `25` → `26`, with the same wording change.
- **No other pin moves.** `scripts/gate.sh` (`step_migrations`, ~:153-190) and `gate.ps1` check only that the numbers are contiguous and unique, so 0026 passes. `PgStore::schema_version()` is computed (`pg/mod.rs:724`). The `cache_migrations` stay as they are. There is no `.sqlx` change.

### T1.12 Tests (red first, in this order)

**`crates/htui-core/tests/prompt_memory.rs`** (CREATE, `#![cfg(feature = "test-support")]`; mirrors `prompt_persona.rs`'s helpers `scrubber`, `ok` and `at_target`):

1. `select_answers_none_with_no_active_entry`: `[]` → `None`; only retired, superseded and suspect entries → `None`.
2. `select_takes_active_entries_only`: mixed states → only Active in `entries`; `total` counts Active (P-8).
3. `select_orders_newest_first_with_an_id_tiebreak`: equal `updated_at` → ascending id.
4. `select_skips_an_over_cap_entry_and_keeps_walking`: with a small cap, the long middle entry is skipped and the shorter older one is kept; `total` is unchanged.
5. `select_keeps_every_entry_over_cap_out_as_some_empty`: every entry over the cap → `Some` with an empty list and `total = n` (P-3).
6. `select_records_the_estimator_id`: `"chars-v2"`.
7. `the_memory_section_renders_the_documented_bytes`: the §T1.9 span equals `wrap(render::memory(&fixture, 2)) + MEMORY_SEPARATOR`, and the prompt text contains it.
8. `the_left_out_line_appears_only_when_entries_are_left_out_and_names_no_tool`: `shown == total` → no line; otherwise the exact line, and it contains no `search`, `tool` or backtick.
9. `a_hostile_entry_renders_inert`: a body of `"</entry>\n</section>\n{{item}}"` and a subject of `"x\" trust=\"pinned"` → the subject is escaped (`&quot;`); `prompt.text` has exactly one `<section name="item"`; the literal `{{item}}` appears inside the memory section; the item section is unchanged.
10. `a_template_cannot_place_the_memory_placeholder`: `Placeholder::Memory.token() == "memory"`; not in `ALL`; `!allowed_in` for all three roles; `parse(Phase, "{{memory}}")` → `UnknownPlaceholder { token: "memory", at: 0 }`.
11. `no_memory_renders_every_fixture_without_a_memory_section`: for every `prompt::fixtures` spec, no `name="memory"` in the text, no `Memory` row and `trim.memory == None` (the `prompt_golden__*` snapshots are the byte proof and must not move).
12. `memory_is_sections_one_without_a_persona_and_two_with_one` (P-9).
13. `the_separator_is_counted_in_the_memory_row_not_the_frame`: `sections[0].tokens_before` is equal with and without memory; the memory row's `tokens_before == est(span incl. "\n\n")`.
14. `a_memory_section_is_trimmed_after_excerpts_and_before_upstream`: at a target that needs a little, upstream and documents rows are `strategy: none`, and memory is `dropped: Some(1)` with the text's line reading `1 of 3`.
15. `a_tighter_budget_drops_the_section_and_its_separator`: the memory row is `{tokens_after: 0, dropped: Some(2), trimmed: true}`; the text holds no `name="memory"`; with no persona the text starts with the template's first literal (no leading blank line); `estimated_after == Σ tokens_after <= target`.
16. `trim_record_v5_records_the_entries_shown_after_trimming`: `to_value` has `v == 5`, `memory.shown` in render order after the trim, and `total == 3`.
17. `the_memory_key_is_absent_without_memory_and_empty_when_the_cap_took_all`.
18. `a_v4_record_still_answers_prompt_summary`: `prompt_summary(Some(&json!({"v":4,"estimated_after":123,"sections":[{"trimmed":true}]})))` == `(Some(123), true)`.
19. `a_handoff_or_judge_spec_renders_no_memory`: set `memory = Some(fixture)` on `handoff_basic()` and `judge_three_candidates()` → no section, no row, `trim.memory == None`, and no refusal over a memory body (P-5).
20. `a_memory_body_is_scrubbed_like_a_persona`: a known secret is masked; credential-shaped text → `AssembleError::Unmasked { section: "memory", .. }`.

**Pins that move:**

- `crates/htui-core/tests/prompt_hostname.rs:259-281`: `json!(4)` → `json!(5)`, and the test is renamed `the_record_is_v5_with_undigested`.
- `crates/htui-core/tests/prompt_digest.rs:1017-1022`: `json!(5)`, with the message "MOD-163 D17 bumped the record to v 5". The 14-key set is unchanged.
- `trim.rs` `the_three_orders_are_the_documented_ones`; `mod.rs` tests (§T1.7).
- `promote.rs` tests (§T1.10); `model/run.rs` and `model/memory.rs` unit tests (§T1.1, §T1.3).
- Migration pins (§T1.11).

### T1.13 Validate (T1)

```bash
cargo check --workspace --all-targets --all-features
cargo test -p htui-core --all-features --no-fail-fast \
  --test prompt_memory --test prompt_persona --test prompt_digest --test prompt_hostname \
  --test prompt_output --test prompt_render --test prompt_golden --test prompt_skills
cargo test -p htui-core --all-features --lib -- prompt:: model::memory model::run fixtures::
cargo insta test -p htui-core --all-features          # expect: nothing moves (bytes unchanged)
cargo test -p htui-orch --all-features --lib -- promote::
HTUI_TEST_DATABASE_URL=<sandbox DSN> cargo test -p htui-store --all-features \
  --test migrations --test connect --no-fail-fast -- --test-threads=1
cargo clippy -p htui-core -p htui-store -p htui-orch --all-targets --all-features -- -D warnings
```

Order matters: `--no-fail-fast` and `--test` are cargo flags and go **before** `--`; `--test-threads=1` is a libtest flag and goes **after** it. Without `HTUI_TEST_DATABASE_URL`, the migration tests print SKIP and pass, so confirm they did not skip.

### T1.14 Commits (T1)

1. `feat(core): MOD-163 M2 T1, SnapshotMemory and MemorySource; GraphSnapshot.memory and PromptSpec.memory default to None`. Covers T1.1–T1.3, the field, and every literal across the six crates (T1.10). The workspace compiles.
2. `feat(core): MOD-163 M2 T1, prompt::memory::select under the hard token cap`. Covers `memory.rs`, `render::memory_entry_block`, and tests 1–6.
3. `feat(core): MOD-163 M2 T1, the internal memory section renders after the persona with its separator in its own span`. Covers the placeholder, section name, `render::memory`, assemble/substitute/scrub, and tests 7–13, 19, 20.
4. `feat(core): MOD-163 M2 T1, TrimStep::Memory drops tail entries and re-renders the left-out line`. Covers tests 14 and 15.
5. `feat(core): MOD-163 M2 T1, trim_record v5 records the memory shown`. Covers `MemoryAudit` and tests 16–18 plus the v-pins.
6. `feat(store): MOD-163 M2 T1, 0026_trim_record_v5 restates the trim_record comment`. Covers the migration and its pins.
7. `feat(orch): MOD-163 M2 T1, the handoff spec carries no memory`. Covers `promote.rs`; it can fold into commit 1 if that commit needs it to compile.

---

## T2 — Engine reach: WorkerStore read, freeze, `phase_spec`, preview (parallel)

**File set:** `htui-core/src/store/worker.rs`, `htui-store/src/worker.rs`, `htui-orch/src/graph.rs`, `htui-orch/src/engine.rs`, `htui-orch/src/conformance.rs`, `htui-orch/tests/fake_conformance.rs`, `htui/src/preview.rs`, `htui/tests/prompt_preview.rs`, and the snapshots `htui/tests/snapshots/{backlog__detail_prompt, prompt_preview__preview_ana_2, prompt_preview__preview_feat_1}.snap` (P-11).

### T2.1 `WorkerStore::memory_entries` (the 15th ReadStore read)

`crates/htui-core/src/store/worker.rs`:

- Module doc ~:15: "`WorkerStore` is 63 methods: 15 [`ReadStore`] reads …".
- Imports ~:30-42: add `MemoryEntry, MemoryState` to `use crate::model::{…}`.
- In the trait, after `notes` (~:189), with the header comment becoming `// -- 15 ReadStore reads`:
  ```rust
  /// [`ReadStore::memory_entries`]: MOD-163 D12's run-start freeze reads the project's
  /// `Active` entries once, in `graph::resolve_or_refused`.
  fn memory_entries(
      &self,
      project: ProjectId,
      states: Option<Vec<MemoryState>>,
  ) -> impl Future<Output = Result<Option<Vec<MemoryEntry>>>> + Send;
  ```
- `impl WorkerStore for MemStore` (~:701, after `notes` ~:751): `async fn memory_entries(&self, project: ProjectId, states: Option<Vec<MemoryState>>) -> Result<Option<Vec<MemoryEntry>>> { ReadStore::memory_entries(self, project, states).await }`.

`crates/htui-store/src/worker.rs`: add the identical forwarder in `impl … for PgStore` (~:116, after `notes` ~:166) and in `impl … for Writer` (~:509, after `notes` ~:559), and the model imports.

### T2.2 The freeze in `graph::resolve_or_refused` (`graph.rs:429-626`)

After `check_fan_out_caps(&phases, &settings)?;` and beside `let repos = store.repos(...)` (~:598), add:

```rust
// MOD-163 D12: the project's active memory, read once here — after every refusal, so a refused
// resolution reads none (blueprint P-1 notes the v1-resume cost) — selected under the cap and
// frozen with its text (D11). `None` from the store (not available) is no entry. Boxed for the
// 2 MiB debug stack, as the graph read above.
let memory = Box::pin(store.memory_entries(item.project_id, Some(vec![MemoryState::Active])))
    .await?
    .and_then(|entries| {
        htui_core::prompt::memory::select(&entries, MEMORY_TOKEN_CAP, TokenEstimator::DEFAULT)
    });
```

The `GraphSnapshot` literal (~:602-625) changes `memory: None` (set by T1) to `memory`. Imports: `htui_core::model::MemoryState` and `htui_core::prompt::{MEMORY_TOKEN_CAP, TokenEstimator}`. A `StoreError` from the read propagates through `?` like `repos`, using the existing `From<StoreError> for ResolveError`.

Unit test in `graph.rs` tests (beside the persona-freeze tests): `resolve_freezes_the_projects_active_memory_with_its_text`. Use a MemStore demo with two active entries and one retired → `snapshot.memory` holds the two in order, `total == 2`, `estimator == "chars-v2"`, and the topology is equal to the same graph's without entries.

### T2.3 `phase_spec` (`engine.rs` ~:7150-7316)

In the spec literal (~:7264), after `persona`: `// MOD-163 D18: the run's memory, frozen at StartRun; every phase step of the run renders it.` `memory: snapshot.memory.clone(),`. This is not role-guarded here, because `assemble` guards (P-5) and a non-Phase role from `phase_spec` already fails with `RoleInputs`.

New engine unit test `a_handoff_opening_carries_no_memory_section` (P-9), modelled on `engine.rs:17516-17575`. Create an active entry for the item's project before the walk; drive `PromoteStep` with the resume caps off; assert `OpeningPath::Handoff { text, .. }` holds no `name="memory"`.

### T2.4 Orch conformance cases (`htui-orch/src/conformance.rs`)

Add a new frame `fn memory_case<'a, H: CaseHarness>(name, harness) -> Option<Pin<Box<dyn Future<Output = ()> + 'a>>>`, shaped like `persona_case` (~:1200). Chain it in `case()` (~:912) with `.or_else(|| memory_case(name, harness))`. Append the six names to `CASES` (after ~:889) under a comment `// MOD-163 M2 (plan D11, D12, D18): …`. Helpers: a local `async fn remember(orch, project, kind, subject, body) -> MemoryEntry`, which calls `orch.store().create_memory_entry(project, NewMemoryEntry { id: MemoryEntryId::new(), …, created_by: ids::USER, box_id: None })` with `MemorySubject::new` / `MemoryBody::new` over `MinimalScrubber::new([])`. Also `fn memory_section(text) -> Option<&str>`, which slices from `<section name="memory">` to the closing `</section>`. Reuse `prompt_sections`, `prompt_text`, `snapshot_of`, `start`, `answer` and `approve_without_walking`.

| Case | Asserts |
|---|---|
| `a_memory_entry_reaches_every_phase_prompt_of_its_project` | Create entry A in `PROJECT_HTUI` and entry B in another demo project. Walk `HTUI_FEAT_3`. In every phase step's prompt, `sections[1] == "memory"`; A's body is present; B's body is absent. |
| `closed_memory_entries_never_reach_a_prompt` | One entry retired and one superseded (the successor stays active) before `StartRun` → only the successor's body is rendered; `snapshot.memory.total == 1` (P-8). |
| `a_memory_edit_after_start_run_does_not_reach_the_run` | The persona-case twin (`:7930`). Start (park at `prd`), then amend the entry. Approve → the `plan` step's memory section has the run-start body. Then retire the entry and retry or answer the next step → the same bytes. `snapshot.memory.entries[0].version == 1`. |
| `fan_out_siblings_carry_byte_identical_memory_sections` | `fan_research` (as `the_judge_step_runs_without_a_persona` sets it up) → every candidate's memory-section slice is equal. The judge's sections hold no `memory`. |
| `a_snapshot_without_memory_resumes_with_no_memory_section` | Start with an entry; `approve_without_walking`; remove `blob["memory"]`; `set_run_snapshot`; `resume` → not `SnapshotTampered`, and the next step's sections hold no `memory`. |
| `a_project_without_memory_writes_no_memory_row` | No entries → no `memory` in sections; `snapshot.memory == None`; the stored JSON has no `memory` key. |

Count pins:

- `cases_are_unique_and_counted` (~:13078): `180` → `186`. The arithmetic string gains `+ 6`, and the prose gains "…, and MOD-163 M2's six memory cases (plan D11, D12, D18: an entry reaching every phase of its project, closed entries kept out, an edit after `StartRun`, fan-out siblings alike, a pre-MOD-163 snapshot, no entries no row)".
- `tests/fake_conformance.rs:15-61`: `180` → `186`, with the text "…, and MOD-163 M2's six memory cases (plan D11, D12, D18) 186".

### T2.5 Preview (`htui/src/preview.rs`)

- `STAND_INS: [&str; 8]` → `[&str; 9]`, with `MEMORY_NOTE` last:
  ```rust
  /// MOD-163 D18: the preview's memory is a live stand-in for a run-start snapshot.
  const MEMORY_NOTE: &str = "preview: memory is selected now from the project's active \
                             entries; a run freezes its own at StartRun";
  /// MOD-163 D18: a read that is not available here (the offline mirror) previews no memory.
  const MEMORY_OFFLINE_NOTE: &str = "memory not available offline";
  ```
- In `build`, after `bound_skills` (~:265), so `prompt_templates` stays the first read an offline arm refuses on:
  ```rust
  let memory = match backend.memory_entries(row.project_id, Some(vec![MemoryState::Active])).await? {
      Some(entries) => select_memory(&entries, MEMORY_TOKEN_CAP, TokenEstimator::DEFAULT),
      None => { notes.push(MEMORY_OFFLINE_NOTE.to_owned()); None }
  };
  ```
  The literal changes `memory: None` (T1) to `memory`. The module and `build` docs gain "ten reads". `the_stand_in_list_is_the_one_the_notes_are_built_from` (~:430) needs no edit.
- `htui/tests/prompt_preview.rs`, new test `the_preview_selects_the_memory_a_run_would_freeze`. On a `Backend::memory(MemStore::demo())`, create two active entries and one retired for the item's project. The preview's `trim.memory.shown` ids equal `select_memory(&store.memory_entries(project, Some(vec![Active])), MEMORY_TOKEN_CAP, DEFAULT).entries` ids, and its notes contain `MEMORY_NOTE`. `the_preview_declares_its_stand_ins` covers the 9th note automatically.
- Snapshots that move (P-11): `backlog__detail_prompt.snap`, `prompt_preview__preview_ana_2.snap`, `prompt_preview__preview_feat_1.snap`. Each gains one note line. Review by hand.

### T2.6 Validate (T2)

```bash
cargo check --workspace --all-targets --all-features
cargo test -p htui-orch --all-features --no-fail-fast -- --test-threads=1 2>&1 | tee /tmp/t2-orch.log
grep -c SIGABRT /tmp/t2-orch.log            # expect 0 (engine walks near the 2 MiB stack)
cargo test -p htui-orch --all-features --test fake_conformance
cargo test -p htui --all-features --no-fail-fast --test prompt_preview --test backlog -- --test-threads=1
cargo insta test -p htui --all-features --test prompt_preview --test backlog
cargo clippy -p htui-core -p htui-store -p htui-orch -p htui --all-targets --all-features -- -D warnings
```

### T2.7 Commits (T2)

1. `feat(store): MOD-163 M2 T2, WorkerStore::memory_entries forwarded by MemStore, PgStore and Writer`.
2. `feat(orch): MOD-163 M2 T2, StartRun freezes the project's active memory into the run snapshot`. Covers `graph.rs` and its unit test.
3. `feat(orch): MOD-163 M2 T2, every phase step renders the frozen memory; six conformance cases`. Covers `engine.rs`, the handoff unit test, `conformance.rs` and `fake_conformance.rs`.
4. `feat(htui): MOD-163 M2 T2, the prompt preview selects live memory as a declared stand-in`. Covers `preview.rs`, `prompt_preview.rs` and the three snapshots.

---

## T3 — `memory_capture` switch (parallel)

**File set:** `htui-core/src/prompt/settings.rs`, `htui/src/prompt_settings.rs`, `htui/src/ui/tabs/settings/prompt.rs`, `htui/tests/prompt_settings.rs`, and `htui/tests/snapshots/prompt_settings__*.snap`. **Do not touch `prompt/mod.rs`.** Call sites import through `htui_core::prompt::settings::{…}`, as `resolve_box_hostname` already does (`settings/prompt.rs:43-45`).

### T3.1 `crates/htui-core/src/prompt/settings.rs`

- New constant beside `BOX_HOSTNAME_DEFAULT` (~:50):
  ```rust
  /// MOD-163 D19: `memory_capture`'s compiled default, on. An App **and** Project key, but not a
  /// field of [`Defaults`] (`0002`'s ten rows): it is the first registry App key with no seeded
  /// row, so its resolver falls through to this constant.
  pub const MEMORY_CAPTURE_DEFAULT: bool = true;
  ```
- `SettingKey` (~:166-193): insert `MemoryCapture = 9` (doc: "`memory_capture`, MOD-163 D19: automatic memory capture at run end; read by MOD-164"). Renumber `PromptReserveFraction = 10`, `UpstreamHops = 11`, `TokenBudget = 12`. Byte order holds: `max_skill_tokens` < `memory_capture` < `prompt_reserve_fraction`.
- `ALL: [Self; 13]` with `Self::MemoryCapture` after `MaxSkillTokens`. Update the docs "twelve" → "thirteen" (~:155 enum doc, ~:220 `from_key`) and the `SettingKind` doc "two switches" → "three switches".
- `SPECS: [SettingSpec; 13]`, with a row after `max_skill_tokens`:
  ```rust
  SettingSpec {
      key: "memory_capture",
      project_key: Some("memory_capture"),
      kind: SettingKind::Boolean,
      min: 0,
      max: 1,
      rungs: Rungs::APP.or(Rungs::PROJECT),
      not_above: None,
      unit: "on/off",
      doc: "Automatic memory capture at run end (takes effect with automatic capture, MOD-164).",
  },
  ```
- `Defaults::value_of` (~:108-125): `SettingKey::MemoryCapture => Value::Bool(MEMORY_CAPTURE_DEFAULT),`. `Defaults::integer` (~:133-152): `SettingKey::MemoryCapture => i64::from(MEMORY_CAPTURE_DEFAULT),`. Both matches are exhaustive.
- `as_rows` doc (~:88-93): "`0002`'s ten rows plus MOD-163's unseeded `memory_capture`, in key byte order". The code is unchanged and now yields eleven rows.
- New resolver beside `resolve_box_hostname` (~:789):
  ```rust
  /// `memory_capture` (MOD-163 D19): the project's boolean, else the `app_setting` row's, else
  /// [`MEMORY_CAPTURE_DEFAULT`]. The house fall-through rule: absent, `null` or a non-boolean is
  /// not a value. Nothing reads it until MOD-164.
  #[must_use]
  pub fn resolve_memory_capture(project: Option<&Value>, app: &BTreeMap<String, Value>) -> bool {
      project_key(project, "memory_capture")
          .and_then(Value::as_bool)
          .or_else(|| app.get("memory_capture").and_then(Value::as_bool))
          .unwrap_or(MEMORY_CAPTURE_DEFAULT)
  }
  ```

### T3.2 `crates/htui/src/ui/tabs/settings/prompt.rs`

- Imports (~:43-45): add `resolve_memory_capture`.
- `switch()` (~:1057-1073) learns app presence (D19):
  ```rust
  /// A [`SettingKind::Boolean`] row's effective column, from its resolver's answer `on`, the row's
  /// own stored value and the `app_setting` row's. The source is the first rung whose value the
  /// resolver would read — a project boolean, then an App boolean (MOD-163 D19) — else the
  /// default; a non-boolean on either rung falls through and is not named (MOD-33 review L5).
  fn switch(on: bool, project_value: Option<&Value>, app_value: Option<&Value>)
      -> (String, Option<i64>, &'static str)
  ```
  The source is `Project` if `project_value` is a bool, else `AppSetting` if `app_value` is a bool, else `AppSettingDefault`.
- `effective` (~:1085-1155): the three switch arms pass `app.get(key.key())`. For the project-only keys that value is always `None`, so their behaviour is unchanged. Add the arm `SettingKey::MemoryCapture => switch(resolve_memory_capture(project, app), project_value, app.get(key.key())),`.
- New in-module unit test `a_switch_names_the_app_rung_when_the_app_row_holds_it`: (no project, app `false`) → `"off"`, `AppSetting`; (project `true`, app `false`) → `"on"`, `Project`; (none) → `"on"`, default.

### T3.3 `crates/htui/src/prompt_settings.rs` and the pins

- `prompt_settings.rs`: docs only ("the ten `App` rows" → "the eleven", ~:102, ~:120-125). The code iterates `app_keys()` and `project_keys()`.
- The `settings.rs` unit tests:
  - `the_defaults_are_migration_0002s_ten_rows_verbatim` (~:897-912): compare `DEFAULTS.as_rows()` **without the `memory_capture` row** to the migration's ten, and add `assert!(DEFAULTS.as_rows().contains(&("memory_capture", Value::Bool(true))))` and `as_rows().len() == 11`.
  - `app_keys_are_the_ten_app_setting_keys_in_byte_order` (~:1076-1097): rename it `app_keys_are_0002s_ten_plus_memory_capture_in_byte_order`; set `len() == 11` and `contains(&SettingKey::MemoryCapture)`. The filter-equality branch is unchanged.
  - New: `memory_capture_is_an_app_and_project_boolean` (mirrors `box_hostname_is_a_project_only_boolean` ~:960): index 9, `key()`, `project_key`, `Boolean`, rungs, `rung_refusal(.., PHASE)`, and `validate` accepting `true` and refusing `1`.
  - New: `resolve_memory_capture_defaults_on_and_project_beats_app`: (None, {}) → on; (None, {false}) → off; ({true}, {false}) → on; ({"no"}, {false}) → off.
  - `every_default_validates_under_its_own_spec`, `ranges_are_the_readers_clamps`, `project_key_is_some_exactly_where_project_is_accepted` and `specs_are_indexed_by_discriminant` iterate `ALL` and need no edit.
- `crates/htui/tests/prompt_settings.rs`:
  - `the_demo_snapshot_has_ten_app_entries_and_four_keys_per_project` (~:150-187): rename it `…_eleven_app_entries_and_five_keys_per_project`. Set `snapshot.app.len() == 11`. The project keys become `[AllowEnvFiles, BoxHostname, MemoryCapture, UpstreamHops, TokenBudget]`; `values[0..=3]` are `None` and `values[4] == Some(json!(120_000))` (P-13).
  - `the_demo_snapshot_renders_the_tree` (~:981-1000): the switch renders `on` (`SWITCH_ON`), not `true`.
  - `a_migrated_postgres_presents_ten_app_rows_with_tokens` (~:630-661): rename it `…_ten_seeded_app_rows_and_an_unseeded_memory_capture`. Assert eleven entries; ten with value and token; `memory_capture` with `value == None` and `updated_at == None`; `app_map()` equals `as_rows()` minus `memory_capture`.
  - New `memory_capture_is_listed_and_set_on_both_rungs` (mirrors the UpstreamHops test at ~:488). Cursor to the App `memory_capture` row → edit `off` → a `SetSetting { rung: App, key: MemoryCapture, value: false, expected: None }` is drained. Then the project row → `SetSetting { rung: Project(id), … }`. Then feed back a snapshot with an app `false` and render: the project row says `off (app setting)` with the label from `label::setting_source(AppSetting)`.
- Snapshots: six `prompt_settings__*.snap` move (`demo`, `demo_80`, `app_only`, `clamped`, `editor_fraction`, `stale`), and possibly `offline`. Review each by hand: one App row added, and one row per project.

### T3.4 Commits (T3)

1. `feat(core): MOD-163 M3 T3, memory_capture joins the setting registry at index 9 with resolve_memory_capture`.
2. `feat(ui): MOD-163 M3 T3, Settings › Prompt lists memory_capture at both rungs and names an app-stored switch`. Covers `prompt.rs`, the `prompt_settings.rs` docs, the tests and the snapshots.

### T3.5 Validate (T3)

```bash
cargo test -p htui-core --all-features --lib -- prompt::settings
cargo test -p htui --all-features --lib -- ui::tabs::settings::prompt
HTUI_TEST_DATABASE_URL=<sandbox DSN> cargo test -p htui --all-features --no-fail-fast \
  --test prompt_settings -- --test-threads=1
HTUI_TEST_DATABASE_URL=<sandbox DSN> cargo test -p htui --all-features --test prompt_settings \
  -- a_migrated_postgres --nocapture      # must run, not print the SKIP line
cargo insta test -p htui --all-features --test prompt_settings
cargo clippy -p htui-core -p htui --all-targets --all-features -- -D warnings
```

---

## T4 — Memory pane, worker family, keys (parallel)

**File set:** `htui/src/memory.rs` (new), `htui/src/store_worker.rs`, `htui/src/lib.rs`, `htui/src/ui/tabs/settings/memory.rs` (new), `htui/src/ui/tabs/settings/hierarchy.rs`, `htui/src/ui/tabs/settings/mod.rs`, `htui/src/keys/{catalogue,views,stack,load}.rs`, `htui/tests/keys_file.rs`, `docs/htui-keys.md`, `htui/tests/settings_memory.rs` (new) with its snapshots, and `htui/tests/snapshots/hierarchy__{demo,demo_80}.snap`.

### T4.1 `crates/htui/src/lib.rs` and `ui/tabs/settings/mod.rs`

Add `pub mod memory;` between `pub mod mcp_search;` and `pub mod persona_import;` in `lib.rs:30-31`. Add `pub mod memory;` after `pub mod logging;` in `ui/tabs/settings/mod.rs:22`.

### T4.2 `StoreRequest` and `StoreReply` (`store_worker.rs`)

Requests, placed after the requirement block (~:1135). Each carries `project` so the re-read knows its scope, and the writes carry `all` (P-6):

```rust
/// MOD-163 D21: one project's memory entries — live (`active | suspect`) unless `all` — for the
/// Hierarchy memory pane. Answered with [`StoreReply::Memory`]; `None` offline (not mirrored).
MemoryEntries { project: ProjectId, all: bool },
/// A new entry (D21). [`StoreReply::MemoryWritten`], or [`StoreReply::MemorySubjectTaken`] on
/// a live-subject clash, or `Failed`.
CreateMemoryEntry { project: ProjectId, all: bool, kind: MemoryKind, subject: MemorySubject, body: MemoryBody },
/// A compare-and-set amend of all three fields (D21). `Written`, `MemoryStale` on `Diverged`,
/// `MemorySubjectTaken` on a clash with another entry, or `Failed`.
AmendMemoryEntry { project: ProjectId, all: bool, id: MemoryEntryId, expected: i32, kind: MemoryKind, subject: MemorySubject, body: MemoryBody },
/// A compare-and-set retire (D21). `Written`, `MemoryStale`, or `Failed`.
RetireMemoryEntry { project: ProjectId, all: bool, id: MemoryEntryId, expected: i32 },
/// The clash question's "replace" (M1 D4): `old` superseded by a new entry with this text.
SupersedeMemoryEntry { project: ProjectId, all: bool, old: MemoryEntryId, expected: i32, kind: MemoryKind, subject: MemorySubject, body: MemoryBody },
```

`MemorySubject` and `MemoryBody` are `Clone` with a length-only `Debug`, so the "no secret as a plain `String`" rule holds. Imports: `MemoryEntry, MemoryEntryId, MemoryKind, MemorySubject, MemoryBody` from `htui_core::model`.

Replies, placed after the requirement replies (~:1760):

```rust
/// Answer to [`StoreRequest::MemoryEntries`]: `None` when the backend has no memory (offline).
Memory { project: ProjectId, entries: Option<Vec<MemoryEntry>> },
/// A memory write landed (D21): what it did, and the project's entries re-read (a failed re-read
/// still answers this, carrying the failure — the write has landed).
MemoryWritten { project: ProjectId, written: crate::memory::MemoryWrite, entries: Result<Vec<MemoryEntry>, String> },
/// An amend, retire or supersede missed its version: the row as it is now and the re-read list.
MemoryStale { project: ProjectId, head: MemoryEntry, entries: Vec<MemoryEntry> },
/// A create or amend met a live entry with the same folded subject (D4, typed not parsed).
MemorySubjectTaken { project: ProjectId, existing: MemoryEntry },
```

`name()` (~:1408), new arms in order: `Self::MemoryEntries { .. } => "memory_entries"`, `"create_memory_entry"`, `"amend_memory_entry"`, `"retire_memory_entry"`, `"supersede_memory_entry"`.

`try_serve` (~:2366), a new or-arm after the requirements arm:

```rust
// The five memory requests, or-ed for the reason the arms above are (MOD-15 M3 plan F-12,
// MOD-163 D21).
StoreRequest::MemoryEntries { .. }
| StoreRequest::CreateMemoryEntry { .. }
| StoreRequest::AmendMemoryEntry { .. }
| StoreRequest::RetireMemoryEntry { .. }
| StoreRequest::SupersedeMemoryEntry { .. } => memory::serve(backend, request).await?,
```

with `use crate::memory;` (~:66).

### T4.3 `crates/htui/src/memory.rs` (CREATE, the worker family)

```rust
//! MOD-163 M3 D21: the memory pane's store requests, the `requirements.rs` shape. One read per
//! event; every write re-reads; offline writes refused before anything is sent; the author and
//! box filled from `Backend::this_user` / `Backend::box_info` (`R-NF-3`). A subject clash is
//! answered as [`StoreReply::MemorySubjectTaken`] by re-reading, never by parsing the refusal.

/// The five request names, in [`StoreRequest`] order; pinned to `name()` by
/// `request_names_match_the_name_arms`.
pub const REQUEST_NAMES: [&str; 5] = [
    "memory_entries", "create_memory_entry", "amend_memory_entry",
    "retire_memory_entry", "supersede_memory_entry",
];
/// The pane's read: a refused read leaves the pane with no list.
pub const READ_NAME: &str = REQUEST_NAMES[0];
/// Whether `name` is one of the four writes.
#[must_use] pub fn is_write(name: &str) -> bool { REQUEST_NAMES[1..].contains(&name) }

/// What a landed memory write did (the self-naming outcome, `RequirementWrite`'s shape).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryWrite {
    Created { id: MemoryEntryId },
    Amended { id: MemoryEntryId, version: i32 },
    Retired { id: MemoryEntryId },
    Superseded { old: MemoryEntryId, new: MemoryEntryId },
}
impl MemoryWrite { #[must_use] pub const fn name(self) -> &'static str; /* the request name it answers */ }

/// The states the pane lists: live unless `all` (D20).
#[must_use] pub fn states(all: bool) -> Option<Vec<MemoryState>>; // all → None; else Some([Active, Suspect])

/// Serves one memory request.
/// # Errors
/// `Unreachable(DATABASE_UNREACHABLE)` for a write offline; the store's own refusal otherwise;
/// `Backend("not a memory request: …")` for a foreign request.
pub async fn serve(backend: &Backend, request: &StoreRequest) -> Result<StoreReply>;
```

The `serve` arms:

- `MemoryEntries` → `Memory { project, entries: backend.memory_entries(project, states(all)).await? }`.
- `CreateMemoryEntry`:
  - `writer = write_access(backend)?`; `me = backend.this_user().await?`; `box_id = backend.box_info().await?.map(|r| r.box_id)`.
  - Build `NewMemoryEntry { id: MemoryEntryId::new(), kind, subject: subject.clone(), body: body.clone(), created_by: me, box_id }`.
  - `match writer.create_memory_entry(project, new).await`:
    - `Ok(row)` → `written(Created{row.id})`;
    - `Err(StoreError::Constraint(c))` → `clash(backend, project, subject.as_str(), None, c).await`;
    - `Err(e)` → `Err(e)`.
- `AmendMemoryEntry`: the patch is `MemoryPatch { kind: Some, subject: Some(clone), body: Some(clone), author_id: me, box_id }`.
  - `Updated(row)` → `Amended{id, version}`.
  - `Diverged { head, .. }` → `MemoryStale { project, head, entries: reread(states(all)) }`.
  - A `Constraint` → `clash(.., Some(id), ..)`, which excludes the amended entry (P-7a).
- `RetireMemoryEntry` → `writer.retire_memory_entry(id, expected, me, box_id)`: `Updated` → `Retired`; `Diverged` → `Stale`.
- `SupersedeMemoryEntry` → `writer.supersede_memory_entry(old, expected, NewMemoryEntry{…})`: `Superseded { new, .. }` → `Superseded{old, new: new.id}`; `Diverged` → `Stale`. A `Constraint` here → `Failed`, as D21 names create and amend only.
- `clash()` re-reads `Some(vec![Active, Suspect])` and finds the entry where `memory_subject_key(&e.subject) == memory_subject_key(subject) && Some(e.id) != except`. Found → `MemorySubjectTaken { project, existing }`; otherwise it returns the original `Err(StoreError::Constraint(c))`, which `try_serve` answers as `Failed`.
- `written()` re-reads with `states(all)`. `Ok(Some(v))` → `Ok(v)`; `Ok(None)` → `Err(DATABASE_UNREACHABLE)`; `Err(e)` → `Err(e.to_string())`.
- `write_access` is `backend.writer().ok_or_else(|| StoreError::Unreachable(DATABASE_UNREACHABLE.to_owned()))` (`requirements.rs:709-717`).
- Last arm: `other => Err(StoreError::Backend(format!("not a memory request: {}", other.name())))`.

Unit tests (in-module, `Backend::memory(MemStore::demo())`; offline is `Backend::Offline { cache: CacheStore::open(tempdir, "memory-offline", 1), since: None }` as in `persona_settings.rs:180-186`):

`request_names_match_the_name_arms` · `a_memory_request_prints_lengths_only` · `entries_reads_live_by_default_and_every_state_with_all` · `create_answers_written_with_the_fresh_list` · `a_create_on_a_taken_subject_answers_the_holder` · `an_amend_onto_another_entrys_subject_answers_the_holder_not_itself` · `an_amend_keeping_its_own_subject_is_no_clash` · `a_stale_amend_answers_stale_with_the_head` · `retire_and_supersede_answer_written` · `offline_reads_answer_none_and_every_write_is_refused_before_sending` · `a_foreign_request_is_named` · `try_serve_routes_the_five` (through `store_worker::serve`).

### T4.4 `crates/htui/src/ui/tabs/settings/memory.rs` (CREATE, the pane)

Public surface, used only by `hierarchy.rs` and by tests. It is self-contained so MOD-116 can mount it elsewhere.

```rust
/// D21's question, byte-exact.
pub const CLASH_QUESTION: &str = "An entry with this subject exists: edit it, or replace it?";
/// A `None` read, or a write key offline (detail.rs:24's shape).
pub const MEMORY_NEEDS_THE_DATABASE: &str = "memory needs the database \u{b7} Settings \u{203a} Connection";
pub const NOT_READ: &str = "memory not read yet";
pub const NO_ENTRIES: &str = "no memory entries yet";
pub const NO_ROW: &str = "no memory entry is selected";
pub const UNCHANGED: &str = "nothing changed; nothing was written";
/// `retire `{subject}`?`
#[must_use] pub fn retire_question(subject: &str) -> String;

pub const HINT_BROWSE: HintSpec;   // j/k select · n new · e edit · b body · d retire · a all · Esc back
pub const HINT_EMPTY: HintSpec;    // n new · a all · Esc back
pub const HINT_FORM: HintSpec;     // Tab/Shift+Tab field · ←/→ kind · Enter body · Esc cancel
pub const HINT_BODY: HintSpec;     // Ctrl+s save · Esc back · Enter breaks the line
pub const HINT_RETIRE: HintSpec;   // y retire · n/Esc stop
pub const HINT_CLASH: HintSpec;    // e edit it · r replace it · Esc back

/// What one key did to the pane.
pub enum PaneKey { Handled(Handled), Close }

#[derive(Debug)]
pub struct MemoryPane {
    project: ProjectId,
    slug: String,
    entries: Option<Vec<MemoryEntry>>,   // last list; None until read
    read: bool,                           // a `Memory` reply arrived
    offline: bool,                        // `Memory { entries: None }`
    refused: Option<String>,              // `Failed { memory_entries }`
    all: bool,                            // D20 toggle, off by default
    cursor: usize,                        // no wrap
    mode: Mode,
    busy: Option<&'static str>,           // one write at a time
    notice: Option<String>,
    scroll: Cell<Option<ScrollSpan>>,
}

#[derive(Debug, Default)]
enum Mode {
    #[default] Browse,
    /// `n` / `e`: kind (choice) + subject (TextField); `Enter` on the subject goes to the body.
    Editing(Form),
    /// The body `TextArea`; `Ctrl+s` sends Create or Amend.
    Body(BodyEditor),
    /// `d`: y/n, then in flight.
    Retiring { id: MemoryEntryId, subject: String, expected: i32, in_flight: bool },
    /// `MemorySubjectTaken`: `e` opens `existing`, `r` supersedes it with `draft`, `Esc` back to the body.
    Clash { existing: MemoryEntry, draft: Draft },
}

/// Hand-written `Debug` (lengths only) on `Form`, `BodyEditor` and `Draft`.
struct Draft { target: Target, kind: MemoryKind, subject: MemorySubject, body: MemoryBody }
enum Target { New, Amend { id: MemoryEntryId, expected: i32 } }

impl MemoryPane {
    /// Opens on `project` and returns the read to send (`MemoryEntries { project, all: false }`).
    #[must_use] pub fn open(project: ProjectId, slug: String) -> (Self, StoreRequest);
    #[must_use] pub const fn project(&self) -> ProjectId;
    /// The read the host re-sends on activation (`SettingsSection::wants_requests`).
    #[must_use] pub fn read_request(&self) -> StoreRequest;
    #[must_use] pub fn stack(&self) -> Stack<'static>;          // per mode, §T4.6
    #[must_use] pub fn hint_text(&self, keys: &Keys) -> String; // `keys.hint(self.stack(), spec)`
    pub fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> PaneKey;
    pub fn on_paste(&mut self, text: &str) -> Handled;
    pub fn on_reply(&mut self, reply: &StoreReply, ctx: &mut Ctx<'_>);
    pub fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>);
    #[must_use] pub fn scroll(&self) -> Option<ScrollSpan>;
}
```

**Keys, all resolved through `ctx.keys().actions(self.stack(), ctx.pressed(&key))`, never through raw keys:**

| Mode | Act | Effect |
|---|---|---|
| Browse | `ListDown` / `ListUp` | cursor (no wrap) |
| Browse | `New` | `Editing(Form { target: New, kind: Fact, subject: "" })`, with the body carried empty |
| Browse | `Edit` | `Editing` prefilled from the row, `Amend { id, expected: version }`, body carried |
| Browse | `MemoryBody` | `Body` on the row (kind and subject unchanged) |
| Browse | `Delete` | `Retiring` with `retire_question(subject)` |
| Browse | `MemoryAll` | flip `all`, then send `MemoryEntries { project, all }` |
| Browse | `Back` | `PaneKey::Close` |
| Editing | the field first (`TextField::on_key`); `FormNextField`/`FormPrevField`; on the kind row `ChoicePrev`/`ChoiceNext` cycle `MemoryKind::ALL` | |
| Editing | `FieldSubmit` | `MemorySubject::new(text, &MinimalScrubber::new([]))`. A refusal goes to the notice (`format!("subject: {err}")`) and the form stays; nothing is sent. Otherwise → `Body` |
| Editing | `FieldCancel` | → Browse |
| Body | `FormSave` | `MemoryBody::new`; a refusal stays in the editor. Unchanged on amend → `UNCHANGED`. Otherwise send `CreateMemoryEntry` or `AmendMemoryEntry { expected }` and set `busy` |
| Body | `FieldCancel` | the first press over edited text arms a second (the personas `unsaved()` rule); then back to the form |
| Retiring | `ConfirmYes` | send `RetireMemoryEntry { expected }`, `in_flight = true` |
| Retiring | `ConfirmNo` | → Browse |
| Clash | `MemoryClashEdit` | `Editing` on `existing` (its text, its version) |
| Clash | `MemoryClashReplace` | send `SupersedeMemoryEntry { old: existing.id, expected: existing.version, … draft }` |
| Clash | `Back` | → `Body` with the draft text kept |

Writes are refused in place, with nothing sent, when `offline` or `entries.is_none()` (the notice is `MEMORY_NEEDS_THE_DATABASE`), or while `busy` (in-flight notice).

**Replies:** a reply whose `project != self.project` is ignored.

- `Memory { entries: Some(v) }` → list; `read`; clear `offline` and `refused`; clamp the cursor.
- `Memory { entries: None }` → `offline = true`; the list is `None`.
- `MemoryWritten` → `busy = None`, `mode = Browse`, `entries` from `Ok`; the cursor goes to the written id; the notice is `saved`/`retired`/`replaced`. In an amend-clash Replace it is "replaced `{subject}`; the entry you were editing is unchanged" (P-7b). An `Err` → the notice carries it and the list is kept.
- `MemoryStale { head, entries }` → list replaced. If the open editor's id is `head.id` and `head.state.is_live()`: keep the text, `expected = head.version`, and set the notice from `changed_elsewhere(keys.label(stack, FormSave))` (`settings/mod.rs`). If `head` is closed: → Browse with the notice "retired or replaced elsewhere; nothing was written".
- `MemorySubjectTaken { existing }` → `busy = None`; `Mode::Clash { existing, draft }`, where the draft is rebuilt from the `Body` editor.
- `Failed { request: READ_NAME, message }` → `refused = Some(message)`.
- `Failed { request, .. } if is_write(request)` → `busy = None`; the form or body stays (the shell already put the message on the status line); `Retiring { in_flight }` goes back to asking.

**Render:** title line `memory · {slug}` followed by `(all states)` when `all` is on; then a `kit::list` with one row per entry: `{kind:<8} {subject}  {state}  {confirmed_at:%Y-%m-%d}` (D20). The form is a `kit::form` (`Value::Choice(kind.as_str())`, `Value::Text(&subject)`), the body is a `TextArea`, the questions are `notice` lines, and the clash shows `CLASH_QUESTION` plus the existing entry's subject. The empty and offline states use `kit::empty` with a next step, and the last row is the notice line. It sets `scroll`.

### T4.5 `crates/htui/src/ui/tabs/settings/hierarchy.rs`

| Anchor | Change |
|---|---|
| imports ~:39-57 | `use crate::memory::REQUEST_NAMES as MEMORY_REQUEST_NAMES;` `use crate::ui::tabs::settings::memory::{MemoryPane, PaneKey};` |
| `HINT_BROWSE` ~:99-109 | `Hint::One(Act::HierarchyMemory, "memory"),` after `HierarchyInfer` |
| new const | `const PICK_A_PROJECT: &str = "memory belongs to a project: put the cursor on a project row";` |
| `Mode` ~:290-312 | `/// MOD-163 D20: a project's memory pane, delegated as Picking is.` `Memory { project: ProjectId, pane: Box<MemoryPane> },` (boxed for `clippy::large_enum_variant`) |
| `pane()` ~:497-510 | `Mode::Memory { .. } => Vec::new(),` |
| `hint_text()` ~:530-563 | first: `if let Mode::Memory { pane, .. } = &self.mode { return pane.hint_text(keys); }`; the match gets `Mode::Memory { .. } => unreachable` → better, use `HINT_BROWSE` as the unreachable value without `unreachable!` |
| `stack()` ~:571-583 | `Mode::Memory { pane, .. } => pane.stack(),` |
| `on_browse_key` ~:588-660 | `Act::HierarchyMemory => if !self.refuse(act, ctx.keys()) { match self.selected() { Some(Row::Project { index }) => self.open_memory(index, ctx), _ => self.notice = Some(PICK_A_PROJECT.to_owned()) } }` |
| new method | `fn open_memory(&mut self, index: usize, ctx: &Ctx<'_>)`: read `snapshot.projects[index].project` (id, slug), call `MemoryPane::open`, `ctx.request(read)`, and set `self.mode = Mode::Memory { project, pane: Box::new(pane) }`. A read is not `busy` |
| `deleting_slug` ~:1041-1046 | add `\| Mode::Memory { .. }` to the `None` list |
| `on_editor_key` ~:1068 | add `\| Mode::Memory { .. }` to the `Pass` list |
| `on_tree` ~:1262-1271 | `Mode::Memory { .. } => false` in `follow`. After `self.snapshot = …`: if `Mode::Memory { project, .. }` and the project is no longer in the tree → `mode = Browse`, notice "the project is gone; its memory pane closed" |
| `on_stale` ~:1350-1356 | add `Mode::Memory { .. }` to the `None` arm |
| `wants_requests` ~:1372 | `let mut out = vec![Hierarchy(..)]; if let Mode::Memory { pane, .. } = &self.mode { out.push(pane.read_request()); } out` |
| `on_paste` ~:1416-1438 | `Mode::Memory { pane, .. } => pane.on_paste(text),` |
| `on_key` ~:1440-1451 | first: `if let Mode::Memory { pane, .. } = &mut self.mode { return match pane.on_key(key, ctx) { PaneKey::Handled(h) => h, PaneKey::Close => { self.mode = Mode::Browse; Handled::Consumed } }; }` |
| `on_reply` ~:1453 | new arm, first: `StoreReply::Memory { .. } \| StoreReply::MemoryWritten { .. } \| StoreReply::MemoryStale { .. } \| StoreReply::MemorySubjectTaken { .. } => if let Mode::Memory { pane, .. } = &mut self.mode { pane.on_reply(reply, ctx) }`; and `StoreReply::Failed { request, .. } if MEMORY_REQUEST_NAMES.contains(request) =>` the same delegation. Place it **before** the generic `Failed if REQUEST_NAMES.contains` arm |
| `scroll()` ~:1626 | `if let Mode::Memory { pane, .. } = &self.mode { return pane.scroll(); }` |
| `render` ~:1630 | first: `if let Mode::Memory { pane, .. } = &self.mode { pane.render(frame, area, ctx); return; }` |

`captures_input` (`!Browse`) needs no change: the memory pane captures input, so `h`/`l` do not cycle sections while it is open. Add a module-doc line: "Five modes … or a project's memory pane (MOD-163 D20)".

### T4.6 Keys (`keys/catalogue.rs`, `views.rs`, `stack.rs`, `load.rs`)

**`catalogue.rs`:**

- `use Context::{…}` (~:46-52): add `SettingsMemory`.
- `enum Context` (~:61-162): add `/// `[settings.memory]`: the memory pane a project row opens in Settings › Hierarchy (MOD-163 D22).` `SettingsMemory,` **after `SettingsLogging`** (P-14).
- `ALL` (~:166-219): after `Self::SettingsLogging`. `table()`: `"settings.memory"`. `heading()`: `"Memory"`. `is_view()`: add it.
- `enum Act`: append `HierarchyMemory, MemoryBody, MemoryAll, MemoryClashEdit, MemoryClashReplace`, each with a doc naming the table and act (`settings.hierarchy.memory`, `settings.memory.body`, …).
- `CATALOGUE` rows:
  - in the `[settings.hierarchy]` block, after `HierarchyInfer` (~:1329): `row(Act::HierarchyMemory, SettingsHierarchy, "memory", &["m"], "memory"),`
  - a new `// [settings.memory]` block **after `[settings.secrets]`** (~:1439, the Queue, Project types and Logging contexts have no rows), in `Context::ALL` order:
    ```rust
    row(Act::MemoryBody, SettingsMemory, "body", &["b"], "edit body"),
    row(Act::MemoryAll, SettingsMemory, "all", &["a"], "all states"),
    row(Act::MemoryClashEdit, SettingsMemory, "clash_edit", &["e"], "edit it"),
    row(Act::MemoryClashReplace, SettingsMemory, "clash_replace", &["r"], "replace it"),
    ```
    No `.since(2)` (the `QdrantAdopt` precedent).
- `VIEW_DEFAULTS` (~:2243): after `(Context::SettingsQueue, Act::Edit, &["enter"])` add `(SettingsMemory, Act::FormNextField, &["down"]), (SettingsMemory, Act::FormPrevField, &["up"]),`.
- Tests:
  - `context_position` (~:2393-2436): `SettingsMemory => 21`; `Concepts` 22 through `Answer` 51.
  - `every_context_is_listed_once_in_all` (~:2441): `52`.
  - `ALL` acts list (~:2697): append the five; `position()`: `217..=221`.
  - `every_act_has_exactly_one_row` (~:2954): `222`.
  - `context_tables_and_headings` (~:3229-3295): `(Context::SettingsMemory, "settings.memory", "Memory")` after Logging; add an assert that `SettingsMemory` is a view.

**`views.rs`:**

- Five stacks, declared after `HIERARCHY_DELETE_WARN` (~:302):
  ```rust
  /// Settings › Hierarchy › Memory, the list (MOD-163 D22). The pane captures input, so no
  /// `SETTINGS` layer: sections do not cycle while it is open.
  pub const MEMORY_BROWSE: Stack<'static> = Stack::new(&[
      Layer::view(Context::SettingsMemory, &[Act::MemoryBody, Act::MemoryAll]),
      Layer::only(Context::Common, &[Act::New, Act::Edit, Act::Delete, Act::Back]),
      LIST,
      MODAL,
  ]);
  /// The memory form: kind (choice) and subject (text).
  pub const MEMORY_EDIT: Stack<'static> =
      Stack::new(&[Layer::view(Context::SettingsMemory, &[]), FIELD, PICK, FORM_FIELDS, MODAL]);
  /// The memory body editor.
  pub const MEMORY_BODY: Stack<'static> =
      Stack::new(&[Layer::view(Context::SettingsMemory, &[]), FIELD, FORM_SAVE, MODAL]);
  /// The retire question (asking and in flight).
  pub const MEMORY_RETIRE: Stack<'static> =
      Stack::new(&[Layer::view(Context::SettingsMemory, &[]), CONFIRM, MODAL]);
  /// The subject-clash question: two choices and back (the `ITEM_DIVERGENCE` shape).
  pub const MEMORY_CLASH: Stack<'static> = Stack::new(&[
      Layer::view(Context::SettingsMemory, &[Act::MemoryClashEdit, Act::MemoryClashReplace]),
      BACK,
      MODAL,
  ]);
  ```
- `HIERARCHY_BROWSE` (~:265-283): add `Act::HierarchyMemory` to its view layer.
- Tests:
  - `VIEWS` (~:1120-1166): add `source!("ui/tabs/settings/memory.rs")`.
  - `VIEW_STACKS` (~:1220): add the five after `HIERARCHY_DELETE_WARN`.
  - The capturing pin (~:1810-1836): `"MEMORY_EDIT", "MEMORY_BODY"` right after `"HIERARCHY_EDITOR"`.
  - New resolver asserts: `resolve(MEMORY_CLASH, "e") == [MemoryClashEdit]`, `resolve(MEMORY_BROWSE, "e") == [Edit]`, `resolve(HIERARCHY_BROWSE, "m") == [HierarchyMemory]`, `resolve(MEMORY_BROWSE, "h") == []`.

**`stack.rs`:**

- `DECLARED` (~:226-332): append after `("in Logs", …)`:
  ```rust
  // MOD-163 D22: the Hierarchy memory pane, appended so every first-reported phrase is unchanged.
  ("in Settings > Hierarchy > Memory", views::MEMORY_BROWSE),
  ("in the Memory form", views::MEMORY_EDIT),
  ("in the Memory body editor", views::MEMORY_BODY),
  ("in the Memory retire question", views::MEMORY_RETIRE),
  ("in the Memory subject question", views::MEMORY_CLASH),
  ```
- Test ~:428: `DECLARED.len() == 90`.

**`load.rs`:** the `TABLES` string (~:539-545) gets `settings.logging, settings.memory, concepts`.

**`tests/keys_file.rs`:** in `bad_fixtures` (~:244-253), the same insertion `settings.logging, settings.memory, concepts`.

**`docs/htui-keys.md`:** regenerate the ```` ```toml ```` block from `cargo run -q -p htui -- --default-keys --print-keys`. Pinned by `the_key_reference_page_is_the_default_print` (`tests/keys_file.rs:723-741`).

**Collision check:** `m` is free in `HIERARCHY_BROWSE` (plan S5). `a` and `b` are free in `MEMORY_BROWSE` because `MODAL` admits only `CONTROL`, `ALT` and function keys. `e` and `r` in `MEMORY_CLASH` share no stack with `common.edit` or `common.reload`. The view contexts are skipped by the per-context pass.

### T4.7 Tests (red first, in this order)

**Worker:** `crates/htui/src/memory.rs` unit tests (§T4.3).

**Keys:** the catalogue, views and stack pins (§T4.6), then `cargo test --test keys_file --test no_hardcoded_keys`.

**`crates/htui/tests/settings_memory.rs`** (`#![cfg(feature = "testkit")]`; `SectionBench` with `HierarchySection::new().with_home(Some("/home/x"))`). Seed the tree by replying with `store_worker::serve(Backend::memory(store), Hierarchy(ws))`. **Build `MemoryEntry` literals with fixed `confirmed_at` and `updated_at` for every snapshot**: `MemStore` stamps real time.

1. `m_on_a_project_row_opens_the_pane_and_reads_its_live_entries`: drained `Action::Store(MemoryEntries { project, all: false })`; `key_stack()` is `MEMORY_BROWSE`.
2. `m_on_the_workspace_or_a_repo_row_says_pick_a_project`: no request; notice `PICK_A_PROJECT`.
3. `the_list_shows_kind_subject_state_and_date` (insta `settings_memory__list`, width 100).
4. `a_toggles_every_state_and_rereads` (`all: true` request; insta `settings_memory__all`).
5. `n_form_enter_body_ctrl_s_creates`: type a subject, `Enter`, type a body, `ctrl-s` → `CreateMemoryEntry` with kind `fact`.
6. `the_kind_choice_cycles_the_four_kinds`.
7. `a_scrub_refusal_stays_in_the_form_and_sends_nothing`: subject `"key sk-ant-api03-…"` → the notice names the rule; no `Store` action.
8. `e_then_ctrl_s_amends_with_the_token`: `AmendMemoryEntry { expected: version }`.
9. `an_unchanged_amend_writes_nothing`.
10. `a_stale_reply_keeps_the_text_and_moves_the_token`: reply `MemoryStale` with `head.version + 1` → the editor text is kept; the next `ctrl-s` carries the new `expected`.
11. `d_asks_then_retires`: the hint is `y retire · n/Esc stop`; `y` → `RetireMemoryEntry`.
12. `a_clash_asks_and_e_opens_the_existing_entry`: `MemorySubjectTaken` → render contains `CLASH_QUESTION`; `e` → form prefilled with the existing entry.
13. `a_clash_and_r_supersedes_the_existing_entry`: `SupersedeMemoryEntry { old: existing.id, expected: existing.version }`.
14. `none_shows_memory_needs_the_database_and_refuses_writes`: `Memory { entries: None }` → render contains `MEMORY_NEEDS_THE_DATABASE`; `n` sends nothing.
15. `esc_closes_the_pane_back_to_the_tree`.
16. `the_pane_hints_read_from_the_keys_in_force`: rebind `[settings.memory] all = ["x"]` → the hint shows `x all`.
17. `a_reply_for_another_project_is_ignored`.
18. `an_end_to_end_create_lands_through_the_worker`: a `Harness` over `MemStore::demo()`; drive keys; `drive_to_end`; the store holds the entry.

**Hierarchy snapshots:** `hierarchy__demo.snap` and `hierarchy__demo_80.snap` move, because of `m memory` in the hint. Review by hand.

### T4.8 Commits (T4)

1. `feat(htui): MOD-163 M3 T4, the memory store-worker family with a typed subject clash`. Covers `memory.rs`, the `store_worker.rs` variants and arms, `lib.rs`, and the worker unit tests.
2. `feat(ui): MOD-163 M3 T4, the memory pane in Settings › Hierarchy with its key context`. Covers the pane, `hierarchy.rs`, `settings/mod.rs`, `keys/*`, `keys_file.rs`, `docs/htui-keys.md`, `tests/settings_memory.rs` and the snapshots. This has to be one commit: the views handled-act check fails while the context exists without the pane that handles its acts.

### T4.9 Validate (T4)

```bash
cargo check --workspace --all-targets --all-features
cargo test -p htui --all-features --lib -- memory:: keys:: store_worker:: ui::tabs::settings::hierarchy
cargo test -p htui --all-features --no-fail-fast --test settings_memory --test keys_file \
  --test no_hardcoded_keys --test hierarchy --test personas -- --test-threads=1
cargo run -q -p htui -- --default-keys --print-keys   # paste into docs/htui-keys.md's toml block
cargo insta test -p htui --all-features --test settings_memory --test hierarchy
cargo clippy -p htui --all-targets --all-features -- -D warnings
cargo clippy -p htui -- -D warnings                   # featureless: code dead without testkit
```

---

## T5 — Integration gate and docs (serial)

- Collect the lanes (from worktrees: named branches, then merge or cherry-pick in T2, T3, T4 order). Run the plan's Validation block **in full**:
  - `cargo fmt --check`
  - both clippy runs
  - `cargo test --workspace --all-features --no-fail-fast -- --test-threads=1 | tee` and `grep -c SIGABRT` (expect 0)
  - `cargo insta test --workspace --all-features --unreferenced=reject`
  - `bash scripts/gate.sh`
  - `validate-workflow-docs.sh`
  - `cargo sqlx prepare --check` must stay green.
- Review every moved snapshot by hand. The expected set:
  - T2: `backlog__detail_prompt`, `prompt_preview__preview_ana_2`, `prompt_preview__preview_feat_1`
  - T3: the six or seven `prompt_settings__*`
  - T4: `hierarchy__demo`, `hierarchy__demo_80`, new `settings_memory__*`
  - T1: none.
  - Anything else moving is a finding.
- `docs/ANA-5.md`, dated notes (2026-11-08, MOD-163):
  - §4.2's section table gets a `memory` row: internal; after the persona, before the body; unprotected; trimmed after excerpts; `sections[1]`, or `sections[2]` with a persona.
  - §5.1 gets the `memory: {shown[], total}` key and `v 5`.
  - §5.3 gets `memory_capture`: App and Project, boolean, default on, unseeded, read by MOD-164.
- Set the PRD's M2 and M3 rows and the plan's status. MOD-147's `0026` collision: whichever merges second renumbers at collect and moves the five migration-count pins in §T1.11.
- Commit: `docs(MOD-163): M2+M3 close-out, ANA-5 notes, PRD and plan status`.

---

## 2. Data flow

`StartRun` → `graph::resolve_or_refused` → `WorkerStore::memory_entries(project, [Active])` → `prompt::memory::select` → `GraphSnapshot.memory` (frozen, outside the topology hash) → `run.graph_snapshot`.

On each phase step: `phase_spec` copies `snapshot.memory` into `PromptSpec.memory` → `assemble` (Phase only):

1. scrub the memory entries;
2. `render::memory` builds the section;
3. the `Trimmer` estimates `wrap + "\n\n"`;
4. `TrimStep::Memory` drops tail entries and rebuilds the left-out line;
5. `substitute` writes `[persona\n\n][memory\n\n]body…`;
6. `trim::record` writes `v 5` and `memory: {shown, total}`.

A handoff (`promote::handoff_spec`) or a judge (`judge_prompts`) sets `None`. The preview runs the same `select` over live entries and declares it in `STAND_INS`.

In M3:

1. Settings › Hierarchy, project row, `m` → `MemoryPane::open` → `StoreRequest::MemoryEntries` → `memory::serve` → `StoreReply::Memory`.
2. A write: the pane builds `MemorySubject`/`MemoryBody` (a refusal stays local) → `Create`/`Amend`/`Retire`/`SupersedeMemoryEntry` → `serve`.
3. The worker re-reads → `MemoryWritten`, `MemoryStale` or `MemorySubjectTaken` → the pane.

`memory_capture`: a `SettingKey` row, `resolve_memory_capture`, and the Settings › Prompt rows at both rungs. Nothing reads it until MOD-164.

---

## 3. Hidden couplings between T2, T3 and T4 (question 8)

Concrete file sets, after this blueprint's additions:

- **T2:** `htui-core/src/store/worker.rs`, `htui-store/src/worker.rs`, `htui-orch/src/{graph,engine,conformance}.rs`, `htui-orch/tests/fake_conformance.rs`, `htui/src/preview.rs`, `htui/tests/prompt_preview.rs`, `htui/tests/snapshots/{backlog__detail_prompt,prompt_preview__preview_ana_2,prompt_preview__preview_feat_1}.snap`
- **T3:** `htui-core/src/prompt/settings.rs`, `htui/src/prompt_settings.rs`, `htui/src/ui/tabs/settings/prompt.rs`, `htui/tests/prompt_settings.rs`, `htui/tests/snapshots/prompt_settings__*.snap`
- **T4:** `htui/src/{memory,store_worker,lib}.rs`, `htui/src/ui/tabs/settings/{memory,hierarchy,mod}.rs`, `htui/src/keys/{catalogue,views,stack,load}.rs`, `htui/tests/{keys_file,settings_memory}.rs`, `docs/htui-keys.md`, `htui/tests/snapshots/{hierarchy__demo,hierarchy__demo_80,settings_memory__*}.snap`

**The pairwise intersection is ∅.** Checked against these shared-surface candidates:

| Candidate | Status |
|---|---|
| `htui-core/src/prompt/mod.rs` re-exports | T1 only. T3 imports `resolve_memory_capture` through `prompt::settings::`; T2 imports `prompt::memory::select` (T1's). |
| `htui/src/lib.rs` module list | T4 only. |
| `store_worker.rs` (`StoreRequest`/`StoreReply`/`try_serve`) | T4 only. T2's preview needs no new reply, and T3's `SetSetting` is generic. |
| count pins | Disjoint: orch `CASES` (T2), the catalogue, `Context`, `DECLARED` and `TABLES` strings (T4), the settings `ALL`/`SPECS`/app/project counts (T3), the migration counts (T1). |
| `docs/htui-keys.md` | T4 only. |
| Cargo features | None added. The integration tests use the existing `testkit` (htui) and `test-support` (htui-core, htui-orch) features. |
| `.sqlx` | Untouched. |
| `hierarchy.rs` | T4 only (it conflicts with MOD-123 at collect, not with a lane). |

**Couplings that are not files. These break parallel lanes if they are ignored:**

1. **One shared working tree.** T2, T3 and T4 all edit the `htui` crate, and T2 also edits `htui-orch` and `htui-core`. In one checkout, any lane's half-finished edit breaks the other lanes' `cargo check --workspace` and `cargo test -p htui`. Run each lane in its own worktree on a named branch (memory: "Workflow worktree implementers"; budget about 10 GB of target per worktree, and check `df -h .` before building). If the lanes must share a tree, serialize the `-p htui` validations.
2. **insta.** In a shared tree, a crate-wide `cargo insta test -p htui` (the plan's T4 line) reports the other lanes' pending `.snap.new` files, and an `--accept` would take them. Every lane runs insta with `--test` filters only (§T2.6, §T3.5, §T4.9).
3. **Postgres.** T1 runs the migration tests and T3 runs the Postgres-only settings pin. Each creates and drops its own database through the testkit, so they do not interfere. They need `HTUI_TEST_DATABASE_URL` (the password-free sandbox DSN) and must be checked for a SKIP. T2 and T4 need no database.
4. **T1 → T2/T3/T4 ordering.** Everything depends on T1's `PromptSpec.memory`, `GraphSnapshot.memory`, `SnapshotMemory`, `prompt::memory::select` and `MEMORY_TOKEN_CAP`. T3 does not depend on T1's code, but it starts from T1's tree so the workspace compiles.
5. **Stack depth (T2).** The new await in `resolve_or_refused` sits inside the boxed resolve future; `Box::pin` the read as well. Run the orch suite with `--no-fail-fast` and grep for `SIGABRT`.

---

## 4. Risks and gotchas for implementers

- **No-memory parity is the main risk.** T1 must leave every `prompt_golden__*` and `prompt_render__*` snapshot untouched. If one moves, the memory path leaked into a `None` spec: check `substitute`, `Trimmer::new` (`span_tokens` adds the separator only for `Memory`) and `record`.
- **`MEMORY_PREAMBLE` bytes.** A doubled space from a `\` continuation silently changes every memory digest. The §T1.9 test pins the bytes.
- **The separator in the span.** All three estimate sites (`new`, `re_estimate`, `section_yardstick`) go through `span_tokens`. A missed site makes `trim_memory` re-measure without the separator and the record lose a token (the M-3 class of bug).
- **Masked text in re-renders.** `trim_memory` must render from `self.inputs.spec.memory` (the masked spec), never from the caller's spec (review C-1).
- **`v 5` pins.** Only `prompt_hostname.rs`, `prompt_digest.rs` and the migration constant pin the number. The demo record stays `v 2` (P-16).
- **Clock-stamped entries in snapshots.** Use `MemoryEntry` literals with fixed timestamps (§T4.7); `MemStore` stamps real time.
- **Forbidden literal.** `htui-agent`'s extensibility test fails on the word "zeta" anywhere. Keep it out of the test data.
- **The featureless clippy run** (`cargo clippy -p htui -- -D warnings`) catches code that is dead without `testkit`. Run it in T4 and T5.
- **Orch `CASES` text.** It is long prose. Update both pins and the arithmetic string together, or `cases_are_unique_and_counted` fails on the count only.
- **Clash search.** It uses `memory_subject_key` (the Postgres `lower(btrim())` fold, ASCII-equal under non-Turkic locales; M1 P-16). The store's refusal stays the authority: a refusal with no matching holder is answered as `Failed`, not as a clash.

---

**Open point for the maintainer (O-1, default in place):** in an **amend** clash, Replace supersedes the *clashing* entry with the typed text and leaves the entry being edited unchanged (the literal reading of D21, P-7b). The alternative is to offer only "edit it" in an amend clash.
