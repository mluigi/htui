# MOD-33 - The box hostname leaves the digest and gains a settings switch (done, 2026-09-30)

From MOD-2 finding L-5 (maintainer-decided 2026-09-16). `R-PRM-1`, `R-PRM-3`, `R-TUI-8`.
Plan `.claude/plans/mod-33.plan.md` (decisions D263–D277, fact-checked: 40 claims), blueprint
`.claude/plans/mod-33.blueprint.md`. Routed as **plan** by `/handoff-run MOD-33`, run in a
`scripts/hr` sandbox.

## What was built

**The hostname is rendered but not digested.** Until now one string was sent, digested and stored:
the box section's first line `hostname: <box>` was a digest input, so two prompts that differed only
in which box assembled them had different `prompt_digest`s. `assemble()` now substitutes **twice
from one set of trimmed sections** (D263):

- `digest_text` is the canonical text with the box section in its **digest form**,
  `hostname: [hostname]` (`render::HOSTNAME_STAND_IN`, D264). Every other stage of the pipeline —
  mask/residue scan, estimate, trim, `trim_record` — sees this form (D265), so no trim decision and
  no record figure depends on the hostname. `prompt_digest = sha256(digest_text)`.
- `text` — what the model is sent and what the payload stores — is the same text with the box
  section re-rendered in its **sent form** (the real, masked hostname) and swapped in **by
  `SectionName::Box`, never by searching text**. The box is protected and never re-rendered by the
  trimmer, so the swap is safe; the sent box passes through the same `scrub_text` mask-and-scan as
  every other section; the hostname is forced onto one line (CRLF first, then lone CR/LF, each to a
  space, D266), so `canonical()` treats both strings identically and they differ only in that
  value. `render::box_profile` takes `HostnameLine { Omitted, StandIn, Shown }`.

**The recorder hashes the text ANA-5 supplies for digesting** (D271). New
`Recorder::record_prompt_digesting(text, digest_text, sections, at)`: the payload `{text, sections}`
is scrubbed as before; `digest_text` is scrubbed with the same scrubber (a residue refuses through
the same `refuse` path, reported at `/digest_text`) and hashed; that hash is the payload `digest` and
`run_step.prompt_digest`. `record_prompt` delegates with the same string twice and, on equal strings,
reuses the scrubbed payload text, so the chat path is byte-identical. `Engine::open_recorder` is the
one production call switched. Without this the recorder would have overwritten the pre-flight digest
with `sha256(sent text)` and ANA-5 §12 criterion 11 would have failed on every box-bearing step.

**`trim_record` v 4** gains `undigested`, always present: `["box.hostname"]` when a box section was
rendered with its hostname, `[]` otherwise (`UndigestedSpan`, a closed vocabulary; D270). It goes
through MOD-32's whole-record scrub unchanged. Rule: `box` in `sections[]` and `undigested` empty
means the switch was off. Nothing deserialises a record, so no compat shim.

**The switch** is `project.settings.box_hostname`, a JSON boolean, default **on**, Project rung only
(D267): new `SettingKind::Boolean`, `SettingKey::BoxHostname` (discriminant 0 — it sorts first; the
other ten shift by one), `settings::resolve_box_hostname` (absent, `null` or non-boolean ⇒ `true`),
`SettingKey::app_keys()` for the ten App keys. `PromptSpec.box_hostname` is resolved by the caller —
`phase_spec`, `judge_prompts`, `preview::build` — so `assemble` stays pure (D268). It appears as the
first row of every project group in **Settings > Prompt** (`e` opens the one-field editor; `on`,
`off`, `true`, `false` in any case; empty clears; the source reads `(default)` when unset and
`project` only for a stored boolean; D276). No store, trait, SQL or `.sqlx` change: both stores are
generic over `project_key`.

**Off** omits the `hostname:` line entirely; the hostname is then neither masked nor scanned, and
`text == digest_text` (D269). Review finding L1 extended the same rule to the switch-on case: the
hostname is masked (and can refuse a prompt) only when the body actually places `{{box}}`, so a
judge, a handoff or a custom phase body without `{{box}}` is never refused over a value it does not
carry — this predated MOD-33.

**Migration `0010_prompt_digest_undigested`** (comment only, the `0008` precedent; D273) restates
`run_step.prompt_digest` (over the text as sent with each undigested span's stand-in) and
`run_step.trim_record` (v 4, `undigested[]`). Pinned by `MOD33_COLUMN_COMMENTS` in
`tests/migrations.rs`; 35 commented columns, unchanged.

## Decisions taken at CONFIRM

- **Q-1, scope: hostname only.** After MOD-33 a step digest and a preview digest match across two
  boxes only when the boxes also agree on `os`, `cpu`, `ram`, `gpu`, `htui` and `tools`; ANA-5 §4.7
  treats those as legitimate variation.
- **D272 kept:** the judge replays the first survivor's stored payload `text` as its task, so the
  judge prompt still digests that candidate's hostname. Rewriting it would show the judge a task the
  candidate never saw; judge prompts are never previewed.
- **D273/T3 included:** the comment-only migration.

## Where the HANDOFF item was off

- "An identical `PromptSpec` assembled on two boxes produces different bytes" — `assemble` is pure,
  so an identical spec is identical bytes; the item means two specs differing only in
  `box_profile.hostname`.
- "MOD-4's criterion 11 (comparing a real step's digest against the preview's)" — no such criterion
  exists. MOD-4's "criterion 11" is ANA-2 §12 criterion 11 (worktree isolation). The criterion this
  item bears on is **ANA-5 §12 criterion 11** (`prompt_digest` equals the payload `digest`), which
  D271 keeps true.
- ANA-5 §4.7 rule 8 does not contain "no machine identifier"; the hostname is discussed in "What
  varies legitimately" (`docs/ANA-5.md:1441-1445`), which the amendment below rewrites.

## The ANA-5 amendment (maintainer-approved in the item; `docs/ANA-5.md` itself is not edited)

Per the milestone-5 precedent (`docs/decisions/mod/mod-2.md:374-377`: an ANA edit is maintainer-only),
the amendment is recorded here:

- **§4.2, the box projection** (`docs/ANA-5.md:510-537`): `hostname` is rendered when
  `project.settings.box_hostname` is true (the default) and omitted entirely when it is false. When
  rendered it is a **rendered-but-not-digested span**: the model sees the value; the digest sees the
  fixed stand-in `[hostname]` (§4.7). The closed field list otherwise stands.
- **§4.7, what is digested** (`:1367-1373`): the adopted option becomes "the assembled prompt text
  alone, canonicalised, as sent, **with each undigested span's value replaced by its stand-in**".
  The rejected "canonicalise for hashing but send the original bytes" (`:1379`) keeps its verdict:
  that option digested a string never sent; here the digest text differs from the sent text in one
  declared span that `trim_record.undigested` records.
- **§4.7, the pipeline** (`:1386-1398`): step 7 is `sha256` over the UTF-8 bytes of the **digest
  text** — step 6's result with every undigested span's value replaced by its stand-in; step 8 hands
  the sent `String` to `AgentDriver::start`, and the sent `String` with its digest text to the
  recorder, which hashes the digest text. The estimate (§4.4) is over the digest form, so the sent
  text can exceed the estimate by the hostname's length minus 10 characters.
- **§4.7, "What varies legitimately"** (`:1441-1445`): the box profile section carries `os`, `cpu`,
  `ram` and the tool list, so the same logical prompt on two different boxes digests differently.
  The hostname is rendered but not digested, so it is the one machine identifier that never moves a
  digest; a project may also omit it from the prompt.
- **§5.1** (`:1483`): `trim_record` v 4 adds `undigested`. **§5.3** (`:1577`): `box_hostname` is a
  `project.settings` key, not an `app_setting` key. **§9**'s column comments are restated by `0010`.
- **ANA-16 §5.3 / §8** (`docs/ANA-16.md:366-368`, `:782`): a container child box's hostname no
  longer affects the digest; the switch covers child boxes by construction (D275) — no child-box
  code exists yet, and whichever box row MOD-44 makes a step execute on is rendered, stood-in and
  gated like any other.

**Re-verifying a stored digest offline** now needs the rule: in each box section, replace the
`hostname:` value with `[hostname]` (only when `trim_record.v >= 4` and `undigested` lists
`box.hostname`). Phase-prompt digests recorded before this change do not compare with later ones.

## Review gate

`rust-reviewer`: **approve with fixes**, no CRITICAL/HIGH. Applied (maintainer: all but L4):
M1 (a known secret in the hostname, the sent-form scan, and a secret split by a line break are
pinned — each single mutation of the two masking layers is caught), M2 (six doc comments that said
the recorder hashes "the same text", including a doc-only edit to the `set_step_prompt` contract in
`store/traits.rs`), L1 (above), L2 (criterion 11 compared with a stage-3 re-assembly, since
`MemStore` keeps no history of the overwritten column), L3, L5, L6 (doc drift), L7
(`debug_assert!` on the protected box section).

**Carried: L4** — `assemble` clones every live section to swap the box one; `substitute` could take
a box override instead. Once per assembly; not worth the churn now.

**Also carried:** D277 — the MCP `box_profile` tool (`R-MCP-2`, MOD-11) returns `BoxProfile`; MOD-11
must decide whether it honours the project switch.

## Commits

Plan `bddaf96`, `c9ffce1`, `12d2c01`; blueprint `640bd6e`.
T0 recorder `d2b5cf6`, `8eacfbc` (merge `55426d1`); T1 settings `e033470`, `fb2effa`;
T3 migration `24c9c0f`, `220abde` (merge `74e0c68`); T2 assembler `01a94a3`, `0314155`;
review fixes `2cad145` (M1), `e821450` (L1), `41937db` (L7), `89cc257` (M2), `80ce323` (L3),
`18b5a7c` (L2), `c3b031a` (L5), `16efd62` (L6).

Gates on the final tree: `cargo fmt --check`, `cargo clippy --workspace --all-features --all-targets
-D warnings`, `cargo test --workspace --all-features -- --test-threads=1` against the sandbox
Postgres: 2830 passed, 0 failed, 26 ignored.
