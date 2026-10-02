# Blueprint: MOD-60 — display width in every hand-laid-out row

> Produced by `code-architect` 2026-10-02 from the CONFIRMED plan
> `.claude/plans/mod-60-display-width.plan.md`, which stays the spec (D1–D12, OQ-1 = yes). Base
> `hr/MOD-60` at `dd47231e` (= `aeba8f63` for every source file; the plan commit touched only the
> plan). **Line numbers below are HEAD's**; they drift as soon as a lane edits, so a lane finds a
> site by the quoted code, not by the number. §0 lists the places this blueprint pins something
> the plan left open or reads it differently (B1–B9). Each one is adopted unless the main thread
> voids it.

## 0. Deviations and clarifications (B1–B10)

| # | What | Why | Plan text it touches |
|---|---|---|---|
| B1 | `clip_spans` puts `…` in the style of **the span the cut falls in**: the first span not kept whole. When the cut lands exactly on a span boundary, that is the first *dropped* span, and `…` is a span of its own in that style. | This is `runs::fit_line`'s behaviour (`runs.rs:1211-1218`). It marks the span that lost text. Read literally, "last kept span" differs only at a boundary, and no test pins either. | D1 "`…` in the last kept span's style" |
| B2 | D8 value cells use `cells::fit(row, vw)`, not `cells::pad`. | Identical whenever the wrapped row is ≤ `vw` cells, which `wrapped` guarantees except for one case. When `vw == 1`, a 2-cell cluster sits alone on its row, `pad` overruns, and D8's own invariant ("exactly `LABEL + STATE + 3·vw + 2`") breaks. `fit` holds it. | D8 "pad with `cells::pad`" |
| B3 | `workspace_switcher.rs` box width: `lines.iter().map(Line::width)` → the sum of `cell_width(span.content)` per line. | D9's reasoning applies verbatim: `Line::width` skips the `ｶﾞ` rule, and a name the column pads by cells would then be measured a cell short and clipped by the border. Same file as T4's switcher site. | D10/T4 (scope addition, one line, own lane) |
| B4 | `tree.rs:221-224` `"  no areas"` goes through `cells::clip(.., width)`. | It bypasses `pad`, so T3's required test ("never wider than the pane at every width 1…45") fails below 10 cells without it. | T3 (required by T3's own test) |
| B5 | `runs.rs:1135`/`:1140` (`{:>USAGE_WIDTH$}`, `{:>DURATION_WIDTH$}`) become `cells::pad_left`, though D6 would let these ASCII sites stay. | No runtime-text right-alignment exists anywhere in `ui/`. Without these two, `pad_left` has no non-test caller, and once T5 lifts B6's `allow` it is `dead_code`, which fails `-D warnings`. The alternative is dropping `pad_left` from D1. | D1 (`pad_left`), D6 |
| B6 | T0 marks `clip`, `pad`, `pad_left`, `fit`, `clip_spans` with `#[allow(dead_code, reason = "MOD-60 T0 lands before its callers (T1-T4); T5 deletes this attribute")]`. `wrap` needs none: T0's `wrapped` calls it. T5 deletes all five. | `mod cells;` is private (`ui/mod.rs:4`), so an unused `pub(crate) fn` is `dead_code`. Clippy runs `--all-targets -D warnings`, and the non-test lib build does not see test uses. `#[expect]` is wrong here: a lane that starts calling one op makes its expectation unfulfilled, which is a warning. | Plan silent |
| B7 | `settings::wrapped`'s hard-break follows `cells::wrap`'s rule: after a word is broken by grapheme, the next word joins the broken word's last row if it fits. `wrapped` is implemented as `cells::wrap` over the whitespace-collapsed text. | One break rule in the codebase. Today's `notice_lines` leaves the last piece alone on its row. V10's probe shows no snapshot holds an overlong word, so neither rule moves one. | D5 (detail unspecified) |
| B8 | `cells::wrap` flattens control clusters in **both** push paths. `wrap_row`/`wrap_line` flattened only in the grapheme path: a non-first word that fits after a space was pushed raw. Splitting stays on U+0020 only, so a tab is not a new break opportunity. | D3: every `clip`/`fit`/`wrap` output draws a control as one blank cell. Widths are unchanged, since a control cluster is 1 cell either way. | D4 "`wrap_row` promoted" |
| B9 | `item_form::notice_lines` is **deleted**, not kept as a wrapper. Its two callers (`item_form::render`, `divergence::row_lines`) call `settings::wrapped` directly. Its doc reasoning (why pre-wrap instead of `Paragraph::wrap`) moves to the `item_form::render` call site. | "becomes a call to `wrapped`" (D5) read as the call sites becoming calls. A one-line forwarding fn is the kind of local copy D1 removes. | D5, D8 wording |
| B10 | **Added during T0 (adversarial verify, round 1; commit `5e54774f`).** `cell_width` is the **sum of per-grapheme widths** (`graphemes(s).map(cluster_width).sum()`, where `cluster_width` is the old body applied to one cluster: `UnicodeWidthStr::width` + one per halfwidth sound mark), not `UnicodeWidthStr::width` over the whole string. | `unicode-width` 0.2.2 applies ligature/ZWJ rules across cluster boundaries (Arabic lam-alef U+0644 U+0627 measures 1, Tifinagh/Lisu/Khmer sequences likewise), while ratatui's `Buffer::set_stringn` advances one grapheme at a time (`ratatui-core-0.1.2/src/buffer/buffer.rs:350-353`), so lam-alef draws 2. With the whole-string measure, `clip(salam, 3)` drew 4 cells and `wrapped` regressed against its old char count. Per-cluster summing equals the renderer by construction. Single-cluster results are unchanged, so MOD-54's `TextField`/`TextArea` (which measure per cluster) do not move; no snapshot moved (gate: 1835 passed). Pinned by `a_string_is_as_wide_as_ratatui_draws_it` and `no_op_draws_wider_than_its_width`. | MOD-54 D2 (`cell_width` definition); plan D1 |

Plan claims re-checked and holding: V5 (`mod cells;` private), V6 (5 outside importers of `wrapped`:
`templates.rs:33`, `library.rs:45`, `requirements/mod.rs:51`, `requirements/detail.rs:15`,
`item_form.rs:41`), V7, V8, V11 (`list::clip` importers: `item_form.rs:40`, `divergence.rs:34`,
`filter.rs:20`, `tree.rs:18`), V12 (see §6). The D11 sweep over `crates/htui/src/ui` at HEAD has
**157 hits** (§7).

---

## 1. T0 — `ui/cells.rs` and `settings::wrapped`

### 1.1 Module shape

- Imports added: `use std::borrow::Cow;` and `use ratatui::text::Span;`. The existing
  `UnicodeSegmentation as _` / `UnicodeWidthStr` imports stay.
- The module doc gains a third paragraph listing the operations: *"Every clip, pad, fit and wrap
  in `ui/` goes through the operations below (MOD-60 D1), so a row is never measured one way and
  drawn another."*
- `pub(crate) const ELLIPSIS: char = '\u{2026}';` with the doc comment *"The one cut mark (MOD-60
  D2): East Asian Ambiguous, so 1 cell under `width()` (see the module doc)."* Lanes may assert
  against it in tests. `runs::CUT` stays (see H4).
- Every new fn is `#[must_use]` (house pattern; `must_use_candidate` itself is pedantic and off),
  `pub(crate)`, and has a doc comment citing MOD-60 D*n*. Each takes `&str` or a slice and returns
  owned values.
- A private helper, `fn flatten(text: &str) -> Cow<'_, str>`, does the D3 rule. When no `char` of
  `text` is `char::is_control`, it returns `Borrowed`. Otherwise it rebuilds the string from
  `graphemes(text)`, replacing every cluster that contains a control char with `" "`.
  - `char::is_control` is the Unicode `Cc` set: C0, DEL and C1. This matches the four existing
    flatteners.
  - It is **per cluster**, so `"\r\n"` becomes one space: `cell_width("\r\n") == 1`. The old
    per-`char` flatteners in `runs`/`reqs` made two.
  - UAX #29 always isolates controls (GB4/GB5), so a control cluster is pure control, never
    `"a\u{1}"`.
  - Why it matters: `cell_width("\u{1}") == 1`, but ratatui skips control graphemes when it draws.
    Flattening is what makes the measured width and the drawn width agree.

### 1.2 Signatures and doc intent

```rust
/// `text` in at most `width` cells, cut at a grapheme boundary with [`ELLIPSIS`] as its last cell
/// when anything was cut (MOD-60 D1, D2). A control character draws as one blank cell (D3) —
/// also when nothing is cut. Width 0 is `""`: a lone `…` would be a cell over. A wide cluster that
/// would straddle the cut is dropped, so a cut result may be one cell short; never one long.
#[must_use]
pub(crate) fn clip(text: &str, width: usize) -> String

/// `text` followed by spaces up to `width` cells (MOD-60 D1). Never clips, never rewrites:
/// a `text` already `width` or wider comes back unchanged.
#[must_use]
pub(crate) fn pad(text: &str, width: usize) -> String

/// Spaces then `text`, right-aligned in `width` cells — `{:>N}` measured in cells. Never clips.
#[must_use]
pub(crate) fn pad_left(text: &str, width: usize) -> String

/// Exactly `width` cells: [`clip`] then [`pad`], so a straddling wide cluster's lost cell is
/// padded back and the next column stays put (MOD-60 D1, R-3).
#[must_use]
pub(crate) fn fit(text: &str, width: usize) -> String

/// `line` in rows of at most `width` cells (MOD-60 D4, from `divergence::wrap_row`): broken at a
/// space where one fits, inside a word by grapheme only when the word alone is wider; a cluster
/// wider than the whole width sits alone on its row (the one overflow). An empty line is one
/// empty row; leading spaces are kept; a space that does not fit is the break. Width 0 reads as 1.
/// Control clusters draw as one blank cell (D3); only U+0020 splits words.
#[must_use]
pub(crate) fn wrap(line: &str, width: usize) -> Vec<String>

/// `spans` cut from the end to at most `width` cells, every kept span keeping its style; when
/// anything was cut, [`ELLIPSIS`] ends the line in the style of the span the cut fell in (B1).
/// Controls flattened per span (D3). No padding. Width 0 is no spans.
#[must_use]
pub(crate) fn clip_spans(spans: &[Span<'_>], width: usize) -> Vec<Span<'static>>
```

### 1.3 Algorithms (pin these exactly)

- **`clip`**
  1. `let flat = flatten(text);`
  2. If `cell_width(&flat) <= width`, return `flat.into_owned()`. **Decision: the fast path
     flattens.** Its output is what gets drawn, and an unflattened control there is the defect D3
     fixes. `execution_graph::clip`, `runs::fit` and `reqs::cut` already behave this way.
  3. If `width == 0`, return `""`.
  4. Otherwise, with `room = width - 1`, walk `graphemes(&flat)`, summing `cell_width(g)`, and
     stop at the **first** `g` with `used + w > room`. Do not skip it to look for a narrower
     cluster behind it.
  5. Push `ELLIPSIS`.
- **`pad`**: `text.to_owned()` plus `width.saturating_sub(cell_width(text))` spaces.
- **`pad_left`**: the same number of spaces, placed before `text`.
- **`fit`**: `pad(&clip(text, width), width)`. Invariant: `cell_width(&fit(t, w)) == w` for every
  `t` and every `w`.
- **`wrap`**: `divergence::wrap_row` (`divergence.rs:571-612`) with one change (B8): each word is
  flattened before both push paths.

  ```text
  width := max(width, 1); rows := [""]
  for (at, raw) in line.split(' ').enumerate():
      word := flatten(raw)
      used := cell_width(last row)
      if at > 0:
          if used + 1 + cell_width(word) <= width: last row += " " + word; continue
          if word.is_empty(): continue                  // a space that does not fit is the break
          if used > 0: rows.push("")
      for g in graphemes(word):
          if last row non-empty and cell_width(last row) + cell_width(g) > width: rows.push("")
          last row += g
  return rows
  ```

  Re-measuring the row with `cell_width` (rather than keeping a running sum) is deliberate. It is
  what `wrap_row` does, and it measures what is drawn.
- **`clip_spans`**
  1. `flat: Vec<(Cow<str>, Style)>` = each span's content flattened, with its style.
  2. If `Σ cell_width <= width`, return every span (empty spans kept), each as
     `Span::styled(content.into_owned(), style)`.
  3. If `width == 0`, return `vec![]`.
  4. Otherwise, with `room = width - 1`, take each span whole while `cell_width(span) <= room`,
     subtracting it from `room`.
  5. The first span that does not fit whole is the **cut span**. Keep its graphemes while they fit
     `room`, stopping at the first that does not. Push `Span::styled(head + "…", cut_span.style)`,
     which may be just `"…"` (B1). Then stop.

### 1.4 Behaviour tables (T0's red tests — `cells.rs` `mod tests`)

Test fixtures, written as `\u{…}` escapes in house style:

| Constant | Value | Cells |
|---|---|---|
| `CJK` | `"\u{6f22}\u{5b57}\u{6587}"` (漢字文) | 6 |
| `FAMILY` | `"\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}"` | 2 (one cluster) |
| `COMBINING` | `"e\u{301}"` | 1 (one cluster) |
| `SOUND` | `"\u{ff76}\u{ff9e}"` (ｶﾞ) | 2 (one cluster) |

**`clip`** (`clip_cuts_by_cells_and_marks_the_cut`):

| text | w | out | cells |
|---|---|---|---|
| `"abcdef"` | 6 / 5 / 1 / 0 | `"abcdef"` / `"abcd…"` / `"…"` / `""` | 6/5/1/0 |
| `CJK` | 6 | `CJK` | 6 |
| `CJK` | 5 | `"\u{6f22}\u{5b57}…"` | 5 |
| `CJK` | 4 | `"\u{6f22}…"` (字 would straddle) | 3 |
| `CJK` | 3 / 2 / 1 / 0 | `"\u{6f22}…"` / `"…"` / `"…"` / `""` | 3/1/1/0 |
| `FAMILY`×2 | 4 / 3 / 2 | unchanged / `FAMILY+"…"` / `"…"` | 4/3/1 |
| `COMBINING`×3 | 3 / 2 / 1 | unchanged / `"e\u{301}…"` / `"…"` | 3/2/1 |
| `SOUND`×2 | 4 / 3 / 2 | unchanged / `SOUND+"…"` / `"…"` | 4/3/1 |
| `"a\u{1}b"` | 3 | `"a b"` — **flattened on the fast path** | 3 |
| `"a\r\nb"` | 3 | `"a b"` (one cluster, one space) | 3 |
| `"ab\ncd"` | 3 | `"ab…"` | 3 |
| `"\t"` | 1 | `" "` | 1 |

Also add a property test, `clip_is_never_wider_than_its_width`. For every input above and every
`w` in `0..=8`, assert `cell_width(&out) <= w` (message `"{out:?} against {w}"`), and assert
`out == flatten(input)` whenever `cell_width(input) <= w`.

**`pad` / `pad_left`** (`pad_fills_to_the_width_and_never_clips`, `pad_left_right_aligns_by_cells`):

| call | out |
|---|---|
| `pad("ab", 4)` | `"ab  "` |
| `pad("\u{6f22}", 4)` | `"\u{6f22}  "` |
| `pad(FAMILY, 3)` | `FAMILY + " "` |
| `pad(SOUND, 3)` | `SOUND + " "` |
| `pad(COMBINING, 2)` | `"e\u{301} "` |
| `pad("abcdef", 4)` | `"abcdef"` (never clips) |
| `pad("", 0)` | `""` |
| `pad("a\u{1}", 3)` | `"a\u{1} "` (no rewrite; measured 2) |
| `pad_left("ab", 4)` | `"  ab"` |
| `pad_left("\u{6f22}", 3)` | `" \u{6f22}"` |
| `pad_left("abc", 2)` | `"abc"` |

**`fit`** (`fit_is_exactly_its_width`, which also ports `runs::fit_pads_cuts_and_flattens`
verbatim):

| call | out |
|---|---|
| `fit("ab", 4)` | `"ab  "` |
| `fit("abcd", 4)` | `"abcd"` |
| `fit("abcde", 4)` | `"abc\u{2026}"` |
| `fit("a\nb", 3)` | `"a b"` |
| `fit("\u{2014}", 2)` | `"\u{2014} "` |
| `fit("abc", 0)` | `""` |
| `fit(CJK, 4)` | `"\u{6f22}\u{2026} "` |
| `fit(CJK, 5)` | `"\u{6f22}\u{5b57}\u{2026}"` |
| `fit(FAMILY×2, 2)` | `"\u{2026} "` |
| `fit(SOUND×2, 2)` | `"\u{2026} "` |

Plus a property: `cell_width(&fit(t, w)) == w` for every table input and every `w` in `0..=8`.

**`wrap`**:

- `wrap_breaks_at_spaces_and_inside_only_a_wider_word` ports `runs.rs:2404-2420` verbatim:
  - `("", 5)` → `[""]`
  - `("ab cd ef", 5)` → `["ab cd", "ef"]`
  - `("  ab", 5)` → `["  ab"]`
  - `("abcdefgh ij", 3)` → `["abc", "def", "gh", "ij"]`
  - `("abc ", 3)` → `["abc"]`
  - `("word "×40, 7)`: every row ≤ 7 cells.
- `no_wrapped_row_is_wider_than_its_width` ports `divergence.rs:1058-1083` verbatim (inputs,
  widths `[1, 2, 3, 7, 20, 48]`, the `width.max(2)` allowance, and the "nothing dropped"
  assertion).
- `wrap_keeps_a_cluster_whole`:
  - `("\u{6f22}"×5, 4)` → `["\u{6f22}\u{6f22}", "\u{6f22}\u{6f22}", "\u{6f22}"]`
  - `("\u{6f22}"×3, 1)` → three single-char rows (the allowed overflow)
  - `(FAMILY×3, 4)` → `[FAMILY×2, FAMILY]`
  - `(FAMILY, 1)` → `[FAMILY]`
  - `(COMBINING×3, 2)` → `[COMBINING×2, COMBINING]`
  - `(SOUND×3, 4)` → `[SOUND×2, SOUND]`
  - `("ab", 0)` → `["a", "b"]` (width 0 reads as 1)
- `wrap_draws_a_control_char_as_a_blank_cell`:
  - `("a\u{1}b c", 80)` → `["a b c"]`
  - `("x \u{1}y", 80)` → `["x  y"]` (B8: the fits-after-a-space path)
  - `("a\tb", 80)` → `["a b"]` (a tab is not a break)

**`clip_spans`** (`clip_spans_cuts_from_the_end_and_keeps_styles`,
`clip_spans_puts_the_ellipsis_in_the_style_of_the_span_it_cuts`,
`clip_spans_drops_a_wide_cluster_that_would_straddle_the_cut`), where `A`/`B` are two distinct
`Style`s:

| spans | w | out |
|---|---|---|
| `[("ab",A),("cd",B)]` | ≥4 | unchanged |
| same | 3 | `[("ab",A),("…",B)]` (boundary cut, B1) |
| same | 2 | `[("a…",A)]` |
| same | 1 | `[("…",A)]` |
| same | 0 | `[]` |
| `[("\u{6f22}",A),("\u{5b57}\u{6587}",B)]` | 5 | `[("\u{6f22}",A),("\u{5b57}…",B)]` — 5 cells |
| same | 4 | `[("\u{6f22}",A),("…",B)]` — 3 cells (字 straddles) |
| `[("a\nb",A)]` | 3 | `[("a b",A)]` (flattened, fits) |
| `[("",A),("ab",B)]` | 2 | `[("",A),("ab",B)]` (empty span kept) |

### 1.5 `settings::wrapped` (`settings/mod.rs:80-101`)

```rust
/// `text` in rows of at most `width` cells, its whitespace collapsed: words joined by one space,
/// a word wider than `width` broken by grapheme (MOD-60 D5, OQ-1) and the next word joining its
/// last piece when it fits (B7); a control character draws as one blank cell (D3). Text with no
/// words is no rows. Width 0 reads as 1.
pub(crate) fn wrapped(text: &str, width: usize) -> Vec<String> {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.is_empty() {
        return Vec::new();
    }
    cells::wrap(&words.join(" "), width)
}
```

- The signature is unchanged, so the 5 outside importers and the 13 settings call sites keep
  their `use`. Import with `use crate::ui::cells;`, and call `cells::wrap` qualified: `wrapped`
  is the name in scope here, so there is no `unused_qualifications` hazard.
- The old doc ("byte-identical … no frame moves") is replaced. MOD-60 is the move.
- Control handling, justified against D3:
  - `split_whitespace` already removes the whitespace controls (`\t \n \r \x0b \x0c`, and U+0085
    NEL).
  - The remaining controls (`\u{1}`, DEL, other C1) stay inside their word and are flattened to
    one blank cell by `wrap`.
  - The effect is D3's rule, with no word split at a non-whitespace control.
- Equivalence: for every input whose words are all ≤ `width` cells and ASCII, the output is
  byte-identical to today's. Today's join test is `cw(line) + 1 + cw(word) > width ⇒ break`,
  which is `wrap`'s `used + 1 + cw(word) <= width` negated.

Red tests go in a new `#[cfg(test)] mod tests` in `settings/mod.rs`, which has none today:

| test | asserts |
|---|---|
| `ascii_text_wraps_as_it_did` | `("ab cd ef", 5)` → `["ab cd", "ef"]`; `("  a \t b\n c ", 80)` → `["a b c"]`; `("", 9)` and `("   ", 9)` → `[]` |
| `a_wide_sentence_wraps_by_cells` | `("\u{6f22}"×10, 7)` → `["\u{6f22}"×3, "\u{6f22}"×3, "\u{6f22}"×3, "\u{6f22}"]`, every row `cell_width ≤ 7` |
| `a_word_wider_than_the_width_breaks_inside` | `("see abcdefghijkl now", 5)` → `["see", "abcde", "fghij", "kl", "now"]`; `("abcdefg hi", 5)` → `["abcde", "fg hi"]` (B7) |
| `a_cluster_is_never_split_across_rows` | `(FAMILY×4, 3)` → four `FAMILY` rows; `("x\u{1}y", 80)` → `["x y"]` |

### 1.6 T0 validation

- `cargo test -p htui --all-features --lib ui::cells ui::tabs::settings -- --test-threads=1`
- **Then the full `cargo test -p htui --all-features --no-fail-fast -- --test-threads=1`.**
  `wrapped` is shared by 18 call sites in 9 files, so T0 owns any test it moves (V10 says none).
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `git status --short crates/htui/tests/snapshots` is empty.

---

## 2. Conventions for every lane

- **Import:** `use crate::ui::cells::{self, cell_width};`, dropping `cell_width` where unused, and
  call the ops **qualified**: `cells::clip`, `cells::fit`, `cells::pad`, `cells::pad_left`,
  `cells::wrap`, `cells::clip_spans`.
  - Never import the op names themselves.
  - `list.rs` keeps a `pub fn clip` until T5, so importing `cells::clip` there is E0255.
  - Importing `cells::clip` and also writing `cells::clip` fires `unused_qualifications`, which
    is a workspace warn.
  - In `divergence.rs` a local `let cells = …` (in `row_lines`) does not shadow the module: paths
    resolve in the type namespace. Renaming the local to `values` is still kinder to readers.
- **Measures:** every `x.chars().count()` used as a **width** becomes `cell_width(&x)`, ASCII or
  not. It costs nothing and keeps the allowlist short. Only `format!` width specifiers on
  compile-time ASCII stay, per D6.
- **Casts:** `u16::try_from(cell_width(x)).unwrap_or(u16::MAX)` (house form). Never `as u16`.
  `cast_possible_truncation` is pedantic and off, but the house does not cast.
- **Deleting a helper:** search the file for its name in doc comments too. `[`wrap_line`]`,
  `[`fit`]`, `[`cut`]` and `[`notice_lines`]` links would dangle. The rustdoc lints
  `broken_intra_doc_links`/`private_intra_doc_links` are `deny` under `cargo doc`. Rewrite the
  doc text as plain code spans (`` `cells::wrap` ``), not links.
- **Red tests:** in-file unit tests (D12). CJK literals are `\u{…}`. Assertions use the form
  `assert!(cell_width(&x) <= w, "{x:?} against {w}")`. Test names are full prose sentences in
  snake case, like the existing suite.

---

## 3. T1 — settings sections

Files: `settings/{agents,boxes,connection,hierarchy,kinds,prompt,qdrant}.rs`. Helpers deleted:
`agents::fit_label`, `boxes::clip`.

| File:line (HEAD) | Before | After |
|---|---|---|
| agents.rs:2031-2035 | `label_width` = max `field.label.chars().count()` | `cell_width(field.label)` |
| agents.rs:2042-2044 | `" ".repeat(label_width - field.label.chars().count())` + `format!("{}{padding}: ", field.label)` | `format!("{}: ", cells::pad(field.label, label_width))` |
| agents.rs:2130-2135 | `longest` = max `field.tool.chars().count()` | `cell_width(&field.tool)` |
| agents.rs:2143 | `fit_label(&field.tool, column)` | `cells::fit(&field.tool, column)`. Same at column 0 (`""`), same for ASCII. |
| agents.rs:2158-2169 | `fn fit_label` | **deleted** (its doc's "MOD-66 B8" rationale moves to `PathsForm::lines`' doc, reworded "in cells") |
| agents.rs:2232-2233 | `notice.chars().count() + separator.len() + label.chars().count() + owed.chars().count()` | `cell_width(&notice) + cell_width(separator) + cell_width(label) + cell_width(&owed)` |
| boxes.rs:1085-1109 | `list_label` doc "in `room` chars"; `room.saturating_sub(tail.chars().count())`; two `clip(..)` | doc "cells"; `cell_width(&tail)`; `cells::clip(&row.hostname, host_room)` and `cells::clip(&label, room)` |
| boxes.rs:1111-1121 | `fn clip` | **deleted**. Width 0 already returned `""`, so no behaviour change there. |
| connection.rs:568 | `label.chars().count()` (`"DSN: "`) | `cell_width(label)` |
| connection.rs:625 | `keys.chars().count() + text.chars().count() + 3 > room` | `cell_width(&keys) + cell_width(&text) + 3 > room` (the `" · "` separator is 3 cells) |
| connection.rs:894 | `row.label().chars().count()` | `cell_width(row.label())` |
| hierarchy.rs:433 | hint check (`notice`) | as connection.rs:625 |
| hierarchy.rs:1485-1497 | `Editor::lines` label width + padding | as agents.rs:2031-2044 |
| hierarchy.rs:1683 | `prompt.chars().count()` (`Type \`{slug}\` to confirm: `, slug runtime) | `cell_width(&prompt)` |
| kinds.rs:1231 | hint check | as connection.rs:625 |
| kinds.rs:1588-1600 | `Editor::lines` | as agents.rs:2031-2044 |
| prompt.rs:701 | `label.chars().count()` (`"{key}: "`) | `cell_width(&label)` |
| prompt.rs:733 | hint check | as connection.rs:625 |
| prompt.rs:1058 | `KEY_WIDTH` = max `key.key().chars().count()` | `cell_width(key.key())` |
| qdrant.rs:467, :489 | `area.width.saturating_sub(l.chars().count() as u16)` | `area.width.saturating_sub(u16::try_from(cell_width(l)).unwrap_or(u16::MAX))` |
| qdrant.rs:519 | hint check | as connection.rs:625 |

**Stay per D6** (T5 allowlist):

| File:line | Hit | Reason |
|---|---|---|
| boxes.rs:1125 | `{name:<LABEL_WIDTH$}` | detail-row labels are string literals at every caller (`detail` :933-1053) |
| boxes.rs:1220 | `spec.digest.chars().take(DIGEST_SHOWN)` | hex digest prefix |
| connection.rs:262 | `.field("len", &text.chars().count())` | `Debug` length (redaction) |
| connection.rs:553 | `{label:<label_width$}` | `Row::label()` is `&'static str` ASCII; the measure is converted |
| prompt.rs:1070 | `{:<width$}` | `SettingKey::key()` is the compiled-in registry, ASCII |
| qdrant.rs:459 | `{l:<5}` | `"URL"` / `"Key"` literals |

**Red tests first.** `agents.rs`, `boxes.rs` and `hierarchy.rs` have no test module; add
`#[cfg(test)] mod tests { use super::*; … }`.

| File | Test | Asserts |
|---|---|---|
| connection.rs (existing mod) | `a_wide_notice_takes_the_hint_line_alone` | `ConnectionSection { notice: Some(Notice::Error("\u{6f22}".repeat(k))), ..ConnectionSection::new() }` in Browse. Let `keys = hint_text()` and pick `width = cell_width(&keys) + k + 3` (fits by chars, not by cells). `hint(width, &Theme::default())` is a single span whose content is the notice. Red today: two spans. This pins the rule shared by all five hint checks. |
| hierarchy.rs (new mod) | `a_wide_slug_leaves_the_confirm_field_its_room` | `delete_pane(DeleteTarget::Project(id), &"\u{6f22}".repeat(8), &DeleteStage::Typed { .. }, 40, &theme)`: the last line's summed `cell_width` over spans is ≤ 40. Red today: the field gets 16 cells too many. |
| agents.rs (new mod) | `a_wide_tool_name_is_fitted_to_the_label_column` | `PathsForm { fields: [tool "\u{6f22}"×20, tool "git"], .. }.lines(60, &theme)`: every line's first span has `cell_width == column + 2` (`": "`), and the wide one ends `"\u{2026}: "`. |
| boxes.rs (new mod) | `a_wide_hostname_keeps_the_suffix_and_the_marker` | `list_label` with hostname `"\u{6f22}"×20`, this box, room 24: `cell_width(&out) <= 24` and `out.ends_with(" (this box)")`. |

If building `BoxRecord`/`DeleteStage::Typed` in a unit test needs more than a page of fixture,
use the closest existing constructor (`BoxesSnapshot`/`TextField::new()`). Do not skip the test.

---

## 4. T2 — backlog

Files: `backlog/{list,filter,item_form,divergence}.rs` and
`backlog/detail/{requirements,runs,prompt}.rs`.

Helpers deleted: `list::pad`, `reqs::cut` (with `reqs::CUT`), `runs::fit`, `runs::fit_line`,
`runs::wrap_line`, `divergence::wrap_row`, `item_form::notice_lines` (B9). **`list::clip` stays
(`pub`, body unchanged) for T5.**

### 4.1 Sites

| File:line | Before | After |
|---|---|---|
| list.rs:170 | `item.key.chars().count()` | `cell_width(&item.key)` |
| list.rs:177 | `item.status.as_str().len()` | `cell_width(item.status.as_str())` (ASCII; consistency) |
| list.rs:207, :209 | `pad(&item.key, key_width)`, `pad(item.status.as_str(), status_width)` | `cells::pad(..)` |
| list.rs:212 | `clip(&item.title, title_width)` | `cells::clip(..)`. `title_width ≥ 1`, so no width-0 change. |
| list.rs:230 | `span.content.chars().count()` | `cell_width(&span.content)` |
| list.rs:240-248 | `fn pad` | **deleted** |
| filter.rs:20 | `use …list::clip;` | `use crate::ui::cells::{self, cell_width};` |
| filter.rs:343 | `head.chars().count()` | `cell_width(&head)` |
| filter.rs:349, :386, :387, :430 | `clip(..)` | `cells::clip(..)` |
| filter.rs:886-888 (test) | `hint.chars().count()` ×2 | `cell_width(hint)` (D12) |
| item_form.rs:40 | `use …list::clip;` | `use crate::ui::cells::{self, cell_width};` (keep `:41 use …settings::wrapped;`) |
| item_form.rs:849 | `head.chars().count()` | `cell_width(&head)` |
| item_form.rs:850, :864, :933, :972 | `clip(..)` | `cells::clip(..)` |
| item_form.rs:899-903 | `notice_lines(sentence, usize::from(inner.width))` | `wrapped(sentence, usize::from(inner.width))`, with the moved B9 comment |
| item_form.rs:977-991 | `pub(super) fn notice_lines` | **deleted** (B9) |
| item_form.rs:1432-1434 (test) | `hint.chars().count()` ×2 | `cell_width(hint)` |
| divergence.rs:30 | `use super::item_form::{ctrl_s, notice_lines};` | `use super::item_form::ctrl_s;` + `use crate::ui::tabs::settings::wrapped;` |
| divergence.rs:33 | `use crate::ui::cells::{cell_width, graphemes};` | `use crate::ui::cells;`. `graphemes` and `cell_width` go unused in non-test code once `wrap_row` is gone, so the test mod imports `crate::ui::cells::cell_width` itself. |
| divergence.rs:34 | `use …backlog::{filter, list::clip};` | `use crate::ui::tabs::backlog::filter;` |
| divergence.rs:444-451 | headings `format!("{:<LABEL_WIDTH$}{:<STATE_WIDTH$}{:<vw$} {:<vw$} {:<vw$}", …, clip(..,vw)…)` | `format!("{}{}{} {} {}", cells::fit("field", LABEL_WIDTH), cells::fit("state", STATE_WIDTH), cells::fit(&format!("ancestor v{}", ..), vw), cells::fit(&format!("theirs v{}", ..), vw), cells::fit("mine", vw))` (D8) |
| divergence.rs:481 | `clip(&headings, width)` | `cells::clip(..)` |
| divergence.rs:500 | section label `clip(&text, at.width)` | `cells::clip(..)` |
| divergence.rs:533 | hint `clip(&view.hint(), width)` | `cells::clip(..)` |
| divergence.rs:548 | `wrap_row(line, usize::from(width))` | `cells::wrap(..)` |
| divergence.rs:564-612 | `fn wrap_row` + doc | **deleted**. `diff_rows`' doc now names `cells::wrap`. |
| divergence.rs:617 | `notice_lines(value, vw)` | `wrapped(value, vw)` |
| divergence.rs:632-633 | `format!("{label:<LABEL_WIDTH$}")`, `format!("{state:<STATE_WIDTH$}")` | `cells::fit(label, LABEL_WIDTH)`, `cells::fit(state, STATE_WIDTH)` (D8) |
| divergence.rs:640 | `format!("{text:<vw$}")` | `cells::fit(text, vw)` (B2) |
| divergence.rs:1058-1083 (test) | `wrap_row(&line, width)` | `cells::wrap(&line, width)`. Keep the test: it is the view's MOD-13 regression, and T0 has the module copy. |
| divergence.rs:1127, :1134 (test) | `chars().count()` | `cell_width(..)` |
| detail/requirements.rs:74-75 | `/// Marks a cut in [`cut`].` + `const CUT` | **deleted**. It would be `dead_code`. |
| detail/requirements.rs:375, :386, :387, :390, :399, :419 | `cut(..)` | `cells::clip(..)`. Same width-0 rule, same flattening (now per cluster). |
| detail/requirements.rs:391 | `moves.chars().count()` | `cell_width(&moves)` |
| detail/requirements.rs:576-597 | `fn cut` + doc | **deleted** |
| detail/runs.rs:684-686 | `artifact_rows` doc "each wrapped by [`wrap_line`]" | "by `cells::wrap`" |
| detail/runs.rs:694, :699 | `wrap_line(..)` | `cells::wrap(..)` |
| detail/runs.rs:704-738 | `fn wrap_line` | **deleted** |
| detail/runs.rs:957-977 | `fn fit` + doc | **deleted** |
| detail/runs.rs:991, 1059, 1064, 1125, 1128, 1132, 1146, 1148, 1162, 1232, 1583, 1604 | `fit(..)` | `cells::fit(..)` |
| detail/runs.rs:1135, :1140 | `format!("{:>USAGE_WIDTH$}", usage_cell(..))`, `format!("{:>DURATION_WIDTH$}", duration_cell(step))` | `cells::pad_left(&usage_cell(..), USAGE_WIDTH)`, `cells::pad_left(&duration_cell(step), DURATION_WIDTH)` (B5) |
| detail/runs.rs:1184-1223 | `fn fit_line` + doc | **deleted** |
| detail/runs.rs:1235-1238 | `fit_line(PermissionStrip::line(..), PANE)` | `let mut spans = cells::clip_spans(&PermissionStrip::line(..).spans, PANE); let used: usize = spans.iter().map(\|s\| cell_width(&s.content)).sum(); spans.push(Span::raw(blank(PANE.saturating_sub(used)))); Line::from(spans)` |
| detail/runs.rs:1665 | `prompt.chars().count()` | `cell_width(&prompt)` |
| detail/runs.rs:2105 (test) | `fit(&step.phase_name, PHASE_WIDTH)` | `cells::fit(..)` |
| detail/runs.rs:1923, 2327, 2384, 4370, 4476 (tests) | `chars().count()` | `cell_width(..)` (D12; 4370 is a fixture sanity check, converted to shorten the allowlist) |
| detail/runs.rs:2390-2401 | `fit_pads_cuts_and_flattens` | **deleted** (ported to `cells` by T0) |
| detail/runs.rs:2403-2420 | `wrap_line_breaks_at_spaces_and_inside_only_a_wider_word` | **deleted** (ported by T0) |
| detail/prompt.rs:274-280 | `format!("{:<24}{:>8}{:>8}  {}", section.name.render(), …)` | `format!("{}{:>8}{:>8}  {}", cells::pad(&section.name.render(), 24), …)`. `SectionName::Documents(String)` carries a runtime `document.kind`. `pad`, not `fit`: std never clipped this column. |

`runs::CUT` (`runs.rs:154`) **stays**. `execution_graph.rs:278` uses `super::CUT` and
`runs.rs:2008` asserts against it.

### 4.2 Stay per D6 or not layout (T5 allowlist)

| File:line | Hit | Reason |
|---|---|---|
| list.rs:204, :206, :211 | `" ".repeat(INDENT\|GAP)` | constant ASCII spaces |
| list.rs:231 | `" ".repeat(width.saturating_sub(used))` | row fill after a `cell_width` measure |
| list.rs:252, :255 | `list::clip` | gone at T5 |
| filter.rs:341 | `{label:<LABEL$}` | `"status"`/`"project"`/`"tags"`/`"ready"` |
| item_form.rs:846 | `{label:<LABEL$}` | field-label literals |
| item_form.rs:1567, :1579, :1586 | `.repeat(..)` | test fixtures |
| divergence.rs:1038, :1039, :1061, :1062 | `.repeat(..)` | test fixtures |
| detail/requirements.rs:1048 | `.chars().take(10)` | test reads an ASCII row prefix |
| detail/runs.rs:981 | `" ".repeat(width)` in `blank` | spaces |
| detail/runs.rs:1852 | `line[..byte].chars().count()` in test `column` | column in a buffer row: one symbol per cell |
| detail/runs.rs:1948, :1984 | `chars().take(INDENT)`, `" ".repeat(INDENT)` | tests over buffer rows |
| detail/runs.rs:2004, :2189, :4347 | `.repeat(..)` | test fixtures |
| detail/prompt.rs:271 | `{:<24}{:>8}{:>8}` header | ASCII literals |
| detail/prompt.rs:≈276 (new row) | `{:>8}` ×2 | integers |
| detail/prompt.rs:304 | `{label:<LABEL$}` | `labelled`'s labels are literals at all 9 callers |

### 4.3 Red tests

| File | Test | Asserts |
|---|---|---|
| list.rs | `a_wide_key_and_title_keep_every_row_the_pane_width` | `lines(&ListView { items, projects, folded: &[], selected: None, filter: None }, &theme, 50)` with one item's `key = "\u{6f22}\u{5b57}-1"` and `title = "\u{6587}"×40` (items from `MemStore::demo()`): every line's summed span `cell_width == 50`, and the status column starts at the same cell offset on every item row. Red today: the CJK row overruns. |
| filter.rs | `a_wide_project_name_is_clipped_to_the_row` | `FilterForm::line(FilterRow::Project, 30, &theme)` with project name `"\u{6f22}"×30`: summed `cell_width <= 30`. |
| item_form.rs | `a_wide_graph_name_is_clipped_to_the_row` | `row_line(Field::Graph, 30, &theme)` with graph name `"\u{6f22}"×30`: `≤ 30` cells. |
| item_form.rs | `a_wide_notice_wraps_by_cells_and_keeps_its_end` | `form.settle(Some(format!("{}\u{7d42}", "\u{6f22}"×60)))`, then `drawn(&form)`: the text holds `'\u{7d42}'` (today's char-chunking cuts it off the edge). Rows come from the buffer, so assert containment, not width. |
| divergence.rs | `every_table_line_is_the_table_width_with_wide_values` | View with theirs title `"\u{6f22}"×40` and mine `FAMILY`×5. For `vw` in `[1, 2, 3, 7, 25]`, for every row of `view.rows()` and every line of `row_lines(row, vw, &theme)`, the summed span `cell_width == LABEL_WIDTH + STATE_WIDTH + 3 * vw + 2`. Red today: char padding overruns. |
| detail/requirements.rs | `a_wide_requirement_body_is_cut_by_cells` | `list_lines(30, &theme)` with a cited requirement whose body is `"\u{6f22}"×40`: every second line's `cell_width <= 30` and ends `'\u{2026}'`. |
| detail/runs.rs | `a_wide_phase_name_keeps_the_usage_column` | `step_lines` with `phase_name = "\u{5b9f}\u{88c5}"×6`: the first line is exactly `PANE` cells, and the usage text begins at cell 31 (prefix `cell_width`). |
| detail/runs.rs | `a_wide_permission_summary_is_forty_three_cells` | `permission_lines` with summary `"\u{6f22}"×40` and an option label in CJK: both lines are exactly `PANE` cells. |
| detail/runs.rs | `a_wide_document_wraps_by_cells` | `artifact_rows` over a `Document` whose body is `"\u{6f22}"×50`, width 20: every row `cell_width <= 20` and the last char survives. |
| detail/prompt.rs | `a_wide_document_kind_keeps_the_numbers_in_their_columns` | `section_lines(&[Section { name: SectionName::Documents("\u{6587}\u{66f8}".into()), .. }])`: the `before` figure ends at cell 32 on the CJK row as on the header. |

### 4.4 D8 arithmetic (pinned)

Let `W = inner.width` and `vw = W.saturating_sub(LABEL_WIDTH + STATE_WIDTH + 2) / 3 = (W − 22) / 3`.

- **Table line** = `fit(label, 10) + fit(state, 10) + fit(a_i, vw) + " " + fit(t_i, vw) + " " + fit(m_i, vw)`, which is exactly `22 + 3·vw` cells. That is ≤ `W`, leaving 0–2 cells of right slack, `(W − 22) mod 3`.
- **Column starts** (from the inner left edge): ancestor at 20, gap at `20 + vw`, theirs at `21 + vw`, gap at `21 + 2vw`, mine at `22 + 2vw`. The existing `cells_stay_in_their_columns` adds 1 for the border.
- **Values:** `wrapped(value, vw)`, which is `wrap(…, vw.max(1))`, gives rows of ≤ `vw` cells except a lone wider cluster. `fit(row, vw)` then makes each exactly `vw` (B2). Height is the max row count over the three values, and at least 1.
- **Headings** use the same composition, then `cells::clip(&headings, W)`. That clip is a no-op unless `W < 22`.
- **At `vw == 0`** (`W < 22`) every value cell is `""`, the line is 20 + 2 cells, and ratatui clips it at the border. That is today's behaviour too. No panic: `tiny_areas_draw_without_a_panic`.

---

## 5. T3 — requirements tab

Files: `requirements/{tree,detail,forms}.rs`. Helpers deleted: `tree::padded`, `detail::padded`,
`forms::padded`. `tree::pad` is rewritten (D7).

### 5.1 D7 header algorithm (`tree.rs:197-212`)

```text
head   := format!("{} ", marker(folded))       // 2 cells (▸/▾ are 1 cell under width())
count  := format!(" ({})", group.count())
TAIL   := " \u{b7} read-only"                  // 12 cells; only when !group.entry.maintainer
name   := &group.project.name
fixed  := cell_width(head) + cell_width(count)
tail_w := if read_only { cell_width(TAIL) } else { 0 }
NAME_MIN := 2                                   // graph.rs fit_label's `slug_room >= 2`
fits(room) := cell_width(name) <= room || room >= NAME_MIN

1. if let Some(room) = width.checked_sub(fixed + tail_w) && fits(room):
       spans = [Span::styled(head + cells::clip(name, room) + count, theme.title)]
               ++ (read_only ? [Span::styled(TAIL, theme.dim)] : [])
2. else if read_only && let Some(room) = width.checked_sub(fixed) && fits(room):
       spans = [Span::styled(head + cells::clip(name, room) + count, theme.title)]   // tail dropped
3. else:
       spans = the full header spans (title span with the whole name, tail if read_only);
       `pad` below clip_spans them. The tail can never survive here: the title span alone is
       wider than `width - 1`, so the cut falls inside it and the marker is what remains.
```

- Span structure, text and styles are byte-identical to today whenever the header fits. That is
  what keeps `requirements__requirements_read_only.snap` and `requirements__requirements_tree.snap`
  in place: `▾ htui (3) · read-only` fits 45.
- `let`-chains (`if let … && …`) are stable on edition 2024 / MSRV 1.98.
- Room arithmetic, worked: open project, count 3, read-only, width 24. `fixed + tail_w` is
  2 + 4 + 12 = 18, so room is 6. `clip("a very long project name", 6)` gives `"a ver…"`, and the
  header is `"▾ a ver… (3) · read-only"`, 24 cells.
- Fallback thresholds:
  - Branch 1 needs `width ≥ fixed + tail_w + min(NAME_MIN, cell_width(name))`.
  - Branch 2 needs `width ≥ fixed + min(NAME_MIN, cell_width(name))`.
  - Anything narrower takes branch 3.

### 5.2 `tree::pad` (`tree.rs:296-316`)

```rust
/// The line cut to `width` cells (MOD-60 D7: `cells::clip_spans`, so a body row cuts its body),
/// padded out to it so the selected style covers the whole row.
fn pad(line: Line<'static>, width: usize, selected: bool, theme: &Theme) -> Line<'static> {
    let mut spans = cells::clip_spans(&line.spans, width);
    let used: usize = spans.iter().map(|span| cell_width(&span.content)).sum();
    spans.push(Span::raw(" ".repeat(width.saturating_sub(used))));
    …selected styling unchanged…
}
```

The trailing raw span is now pushed in the cut case too, where it was not before. No test counts
spans. `withdrawn_rows_render_dim` filters whitespace spans, and the padding span is whitespace.

### 5.3 Sites

| File:line | Before | After |
|---|---|---|
| tree.rs:18 | `use …list::clip;` | `use crate::ui::cells::{self, cell_width};` |
| tree.rs:186 | `row.key.chars().count()` | `cell_width(&row.key)` |
| tree.rs:197-212 | project header | §5.1 |
| tree.rs:221-224 | `Line::styled(format!("{}no areas", " ".repeat(AREA_INDENT)), theme.dim)` | `Line::styled(cells::clip(&format!(..), width), theme.dim)` (B4) |
| tree.rs:272, :275 | `padded(..)` | `cells::pad(..)` |
| tree.rs:277 | `clip(first_line(..), body_width)` | `cells::clip(..)` (`body_width ≥ 1`) |
| tree.rs:285-294 | `fn padded` | **deleted** |
| tree.rs:296-316 | `fn pad` | §5.2 |
| detail.rs:56-63 | `fn padded` | **deleted** |
| detail.rs:114 | `row.item.key.chars().count()` | `cell_width(&row.item.key)` |
| detail.rs:119 | `row.item.status.as_str().len()` | `cell_width(..)` |
| detail.rs:131, 133, 136, 146, 149 | `padded(..)` | `cells::pad(..)`; `padded("", SUSPECT.len() + 2)` becomes `cells::pad("", cell_width(SUSPECT) + 2)` |
| detail.rs:160, 162, 164 | `padded(..)` in `revision` | `cells::pad(..)`. `by` carries a runtime item key. |
| forms.rs:411, :545, :582 | `padded(..)` | `cells::pad(..)` |
| forms.rs:589-597 | `fn padded` | **deleted** |

`detail.rs::paragraph` (`:42-54`) needs no edit; it inherits T0's `wrapped`.

**Stay** (T5 allowlist): tree.rs:232, :267, :269, :274, :276 are `" ".repeat(AREA_INDENT|ROW_INDENT|GAP)`
(constant spaces). The new `" ".repeat(width.saturating_sub(used))` in `pad` is a row fill after a
`cell_width` measure.

### 5.4 Red tests (in tree.rs's existing mod; `platform()` fixture)

Each test sets `snapshot.projects[_].maintainer = false` and/or renames
`projects[_].name` for project htui, then calls `lines(&view, width, &theme)`.

| Test | Asserts |
|---|---|
| `a_narrow_pane_keeps_read_only_and_elides_the_project_name` | name `"a very long project name"`, read-only, width 24: the header text is exactly `"\u{25be} a ver\u{2026} (3) \u{b7} read-only"`. Red today: the tail is cut and the name kept. |
| `a_wide_project_name_is_clipped_by_cells` | name `"\u{6f22}\u{5b57}"×6`, maintainer, width 20: header `cell_width == 20`, contains `" (3)"` and `'\u{2026}'`, starts with `"\u{25be} "`. |
| `no_tree_line_is_wider_than_the_pane_at_any_width` | CJK name, read-only, one requirement body `"\u{6587}"×30`, one area with no requirement. For `width in 1..=45`, every line's summed `cell_width <= width` (`"{text:?} at {width}"`). |
| `a_wide_key_keeps_the_priority_column` (detail.rs or tree.rs) | `coverage` with an item key `"\u{6f22}\u{5b57}-1"` and one ASCII key: the kind column starts at the same cell offset on both lines. |

---

## 6. T4 — skills, overlays, chat

Files: `skills/{attach,library,templates}.rs`, `overlay/workspace_switcher.rs`, `chat/mod.rs`,
`path_picker.rs`. Helpers deleted: `attach::cut`, `chat::columns`, `chat::clip`.

| File:line | Before | After |
|---|---|---|
| attach.rs:43-44 | doc "in chars … cut to `LABEL_WIDTH - 1` and `…`" | "in cells … fitted with `cells::fit`" |
| attach.rs:723, :732 | `cut(&label(..), LABEL_WIDTH)` + `format!("{star} {label:<LABEL_WIDTH$} {summary}")` | `let label = cells::fit(&label(..), LABEL_WIDTH); format!("{star} {label} {summary}")`. Extract this as `fn row_text(star: char, label: &str, summary: &str) -> String` so it is testable. |
| attach.rs:933-941 | `fn cut` | **deleted** |
| library.rs:56-60 | docs `` `  {name:<22} v{head:<3}` is 29 chars `` / "in chars" | prose in cells, **without** format-spec syntax (otherwise it stays a sweep hit), e.g. "two spaces, the name fitted to [`NAME_WIDTH`] cells, ` v` and the head: 29 cells" |
| library.rs:1449-1454, :1460 | char cut + `format!("  {name:<NAME_WIDTH$} v{head:<3}")` | `fn browse_row(name: &str, head: i32) -> String { format!("  {} v{head:<3}", cells::fit(name, NAME_WIDTH)) }` |
| library.rs:1500 | `prompt.chars().count()` | `cell_width(&prompt)` |
| library.rs:1514 | `label.chars().count()` | `cell_width(label)` |
| templates.rs:43-48 | docs | as library.rs:56-60 |
| templates.rs:944-951 | char cut + `format!("  {name:<NAME_WIDTH$} v{head:<3} {role}")` | `fn template_row(name: &str, head: i32, role: &str) -> String { format!("  {} v{head:<3} {role}", cells::fit(name, NAME_WIDTH)) }` |
| templates.rs:1003 | `prompt.chars().count()` | `cell_width(&prompt)` (the slug is runtime) |
| workspace_switcher.rs:98 | `w.name.chars().count()` | `cell_width(&w.name)` |
| workspace_switcher.rs:131-132 | `" ".repeat(column.saturating_sub(workspace.name.chars().count()))` + `format!("{marker}{}{padding}{GAP}", workspace.name)` | `format!("{marker}{}{GAP}", cells::pad(&workspace.name, column))` |
| workspace_switcher.rs:190 | `lines.iter().map(Line::width)` | `lines.iter().map(\|line\| line.spans.iter().map(\|span\| cell_width(&span.content)).sum::<usize>())` (B3) |
| chat/mod.rs:113-139 | `fn columns`, `fn clip` | **deleted** |
| chat/mod.rs:≈362-368 | `columns(..)` ×4, `clip(..)` ×2 | `cell_width(..)`, `cells::clip(..)`. Output is identical for the existing test (worked: width 40 gives room 31; `" · handoff · scripted · sonnet"` clipped to 6 is `" · ha…"` both ways). |
| chat/mod.rs:1210 (test) | `columns(&text(40))` | `cell_width(&text(40))` (D9) |
| path_picker.rs:292 | `u16::try_from(GOTO.len())` | `u16::try_from(cell_width(GOTO))` (already imported at :19) |

**Stay** (T5 allowlist):

| File:line | Hit | Reason |
|---|---|---|
| attach.rs:928, templates.rs:1146 | `project.to_string().chars().take(8)` | UUID prefix |
| attach.rs:1016, :1027, :1038 | `{name:<FORM_LABEL$}` / `{:<FORM_LABEL$}` | `&'static str` form labels |
| library.rs:1609 | `{label:<INFO_LABEL$}` | `"name"`/`"description"` |
| library.rs `browse_row`, templates.rs `template_row` | `v{head:<3}` | integer |

**Red tests** (`chat` has a test mod; add mods where missing):

| File | Test | Asserts |
|---|---|---|
| chat/mod.rs | `a_halfwidth_phase_keeps_the_promoted_header_within_its_width` | `promoted.phase = "\u{ff76}\u{ff9e}"×10` (20 cells; `Span::width` says 10). `cell_width(&text(40)) <= 40` and the text ends with `" · live-1"`. Red today: `columns` undercounts by 10. |
| chat/mod.rs | `a_family_emoji_is_never_split_in_the_header` | `promoted.agent = FAMILY×10`, width 30: the text contains no lone `'\u{200d}'` at the cut, i.e. it holds `FAMILY` whole or not at all. Red today: `chat::clip` steps per `char`. |
| attach.rs | `a_wide_label_keeps_the_summary_in_its_column` | `row_text('*', l, "S")` for `l` in [ASCII×40, `"\u{6f22}"×20`, `"\u{6f22}"×5`, `FAMILY`×3]: `cell_width` of everything before `"S"` is `2 + LABEL_WIDTH + 1`. |
| library.rs | `a_wide_skill_name_keeps_the_version_column` | `browse_row(n, 7)` for ASCII/CJK names: `cell_width` up to `" v7"` is `2 + NAME_WIDTH`. |
| templates.rs | `a_wide_template_name_keeps_the_head_and_role_columns` | as library, `template_row(n, 3, "plan")`. |
| workspace_switcher.rs | `a_wide_workspace_name_keeps_the_counts_in_one_column` | `WorkspaceSwitcher { workspaces: [Platform, "\u{5e73}\u{53f0}"], loaded: true, .. }.lines(&theme)`: the first span's `cell_width` is equal on both rows. |
| path_picker.rs | — | none (ASCII constant; consistency only) |

---

## 7. Behaviour changes that could move a test or snapshot (checked against the actual tests)

| # | Change | Where | Tests / snapshots looked at | Verdict |
|---|---|---|---|---|
| C1 | `wrapped` measures cells + hard-breaks (OQ-1), with B7's join | T0; all 18 callers | item_form `a_notice_word_wider_than_the_pane_breaks_inside`: rows go from `["touched path", 43, 39, "is bad"]` to `["touched path", 43, 38+"\` is", "bad"]`; trimmed-and-concatenated text still holds `"a/"×40`. Also `the_hedge_after_a_long_failure_shows_whole`, `the_hedge_shows_whole_on_an_80_by_24_terminal`, `a_failure_too_long_for_the_pane_keeps_the_hedge`, divergence `a_long_title_wraps_inside_its_column` (no word > `vw` = 25). `tests/{settings,kinds,box_settings,prompt_settings,qdrant}`. | no move (V10's probe). A long URL/DSN in connection.rs:550 / qdrant.rs:447 value rows now wraps instead of being clipped, which is OQ-1's stated cost; no snapshot holds one. |
| C2 | width 0: `"…"` → `""` | `list::clip` callers (filter :349/:386-387/:430, item_form :850/:864/:933/:972, divergence headings at `vw == 0`); `attach::cut` (`LABEL_WIDTH` const: never 0) | divergence `tiny_areas_draw_without_a_panic` (asserts no panic only) | no move |
| C3 | D3 flattening where none was | list titles, filter values, item_form picker labels, divergence, tree, attach, library, templates, chat header, boxes hostnames, agents tool names | no fixture or snapshot holds a `Cc` char in these fields. V9 scanned W/F/Mn/Me/Cf; ratatui drops a `Cc` cluster when drawing, so none can appear in a `.snap`. | no move. T5's snapshot check is the backstop. |
| C4 | per-cluster flattening (`"\r\n"` → 1 space, not 2) | `runs::fit`/`fit_line`, `reqs::cut` sites | runs `a_reason_line_is_forty_three_columns_whatever_the_note` (`\n`, `\t` only) | no move |
| C5 | B8: `wrap` flattens in the fits-after-a-space path | divergence diffs, runs artifact | divergence `no_wrapped_row_is_wider_than_its_column` (its "kept" filter already drops controls); `backlog__runs_artifact.snap` (ASCII) | no move |
| C6 | `fit_line` → `clip_spans` + fill: an extra (possibly empty) raw span in the cut case | runs `permission_lines` | no `spans.len()` assertion anywhere in `ui/` tests; `a_step_with_a_pending_request_takes_two_more_lines_each_forty_three_wide` measures widths | no move |
| C7 | D7 header | tree.rs | `requirements__requirements_read_only.snap` (`▾ htui (3) · read-only` fits 45); `withdrawn_rows_render_dim`; `tests/requirements.rs::a_non_maintainer_is_answered_on_the_status_line` (`contains("read-only")`) | no move |
| C8 | D8 `fit` cells | divergence | `backlog__item_divergence.snap`, `cells_stay_in_their_columns` users | no move (ASCII rows ≤ `vw`) |
| C9 | B4 `"no areas"` clipped | tree.rs | `requirements__*` snapshots (`  no areas` fits) | no move |
| C10 | **compile breaks unless handled**: tests calling deleted helpers | runs `fit_pads_cuts_and_flattens`, `wrap_line_breaks_at_spaces_and_inside_only_a_wider_word`, `every_step_row_fits_forty_three_columns` (:2105 `fit`); divergence `no_wrapped_row_is_wider_than_its_column` (`wrap_row`); chat `a_narrow_promoted_header_keeps_the_whole_session_ref` (`columns`) | handled in §4.1 / §6 | — |

R-1's rule stands: a `.snap.new` stops the lane; never `cargo insta accept`.

---

## 8. Hazards

- **H1 — file-set disjointness, re-checked against the site list.**
  - Every site above sits in its lane's file. No lane needs an edit in another lane's file.
  - `requirements/mod.rs`, the 13 settings callers, `library.rs`/`templates.rs` notices and
    `requirements/detail.rs::paragraph` inherit T0's `wrapped` without an edit.
  - Files with sweep hits that no lane touches, all allowlist-only: `detail/graph.rs`,
    `runs/execution_graph.rs`, `overlay/concepts_search.rs`, `text_area.rs`, `text_field.rs`.
  - Cross-lane **symbols** (not files):
    - `list::clip`: T2's file, used by T3's tree.rs.
    - `runs::CUT`: used by `execution_graph.rs`, which is in no lane, so it stays.
    - `settings::wrapped`: T0 owns it.
- **H2 — `list::clip` lifetime.**
  - T2 leaves `pub fn clip` (`list.rs:250-258`) untouched and stops calling it.
  - T3 switches tree.rs's import.
  - It is reachable as public API (`lib.rs:38 pub mod ui` → `tabs` → `backlog` → `pub mod list`),
    so after both land it does **not** warn as dead. T5 must delete it deliberately (D11/T5), and
    there are no `tests/*.rs` users.
- **H3 — `dead_code` on T0's ops.** B6. Lanes must **not** remove the `allow`s. T5 removes all
  five and re-runs clippy. Every op then needs a non-test caller:
  - `clip`: many sites.
  - `pad`: list, tree.
  - `pad_left`: runs (B5).
  - `fit`: runs, attach.
  - `wrap`: `wrapped`, divergence, runs.
  - `clip_spans`: tree, runs.
- **H4 — leftover imports and constants after deletions.** Each of these is a warning under
  `-D warnings`:
  - divergence `graphemes` and `cell_width` (non-test) go unused.
  - reqs `CUT` goes unused: delete it.
  - item_form `notice_lines` callers.
  - runs `Style` may still be used elsewhere; check before removing it.
  - chat still uses `Span`.
- **H5 — clippy.** `clippy::all` at warn is promoted by `-D warnings`. `clippy::pedantic` is
  deliberately **off** (`Cargo.toml` `[workspace.lints.clippy]`), so no
  `cast_possible_truncation` or `must_use_candidate` is enforced. The house adds `#[must_use]`
  anyway and uses `try_from`.
  - Lints that do bite new code: `uninlined_format_args` (write `"{x:?}"`), `needless_range_loop`,
    `manual_let_else` (pedantic, off).
  - Workspace rust lints: `unused_qualifications` (§2), `missing_debug_implementations` (no new
    types are planned), `unsafe_code = forbid`.
- **H6 — buffer rows in tests.** ratatui resets a wide glyph's continuation cell to `" "`, so a
  buffer row read symbol by symbol has one `char` per cell. Column checks by char index (as
  `cells_stay_in_their_columns` does) stay valid with CJK. Its *text* has an extra space after
  each wide glyph, so assert containment of single chars, not of CJK runs (D12's `buffer_text`
  note).
- **H7 — T0 validation scope.** Run the whole htui suite in T0 (§1.6), not only `ui::cells`.
  `wrapped` reaches 9 files.
- **H8 — `--test-threads=1` and `--all-features`.** These are the house rules: without `testkit`,
  `tests/*.rs` run 0 tests and still report ok.

---

## 9. T5 — sweep and cleanup

1. Delete `list::clip` (`list.rs:250-258`) and the five `#[allow(dead_code, …)]` in `cells.rs`
   (B6). Run `cargo clippy … -D warnings`: no `dead_code`.
2. Run the sweep from the repo root:

   ```bash
   rg -n 'chars\(\)\.count\(\)|chars\(\)\.take\(|\.repeat\(|:<\w+\$\}|:>\w+\$\}|:<\d+\}|:>\d+\}' crates/htui/src/ui
   ```

   This is exactly D11's patterns. At HEAD it returns 157 hits. After T1–T4 it should return
   about 95, every one on the allowlist below.
3. Write the allowlist into the plan's "Allowlist" section as `| file:line | hit | reason |`, with
   line numbers **re-derived at T5**. The skeleton below is HEAD-numbered and groups lane
   remainders by reason:

| Reason | Entries (HEAD lines) |
|---|---|
| Test fixtures (`x.repeat(n)` building input) | concepts_search.rs:980, 1008, 1010, 1011 · graph.rs:1613, 1615 · runs.rs:2004, 2189, 4347 · divergence.rs:1038, 1039, 1061, 1062 · item_form.rs:1567, 1579, 1586 · text_area.rs:618, 1271, 1279, 1282, 1441, 1461, 1525, 1531, 1579 · text_field.rs:746, 789, 796, 885, 887 |
| Test helpers reading buffer/ASCII rows (one char per cell) | runs.rs:1852, 1948, 1984 · execution_graph.rs:939 · detail/requirements.rs:1048 · text_area.rs:1130, 1252 |
| Not layout: `TextArea::len` char-count API (D10) | text_area.rs:124 (doc), 131 |
| Not layout: `Debug` length (redaction) | connection.rs:262 |
| Not layout: UUID / digest prefixes | attach.rs:928 · templates.rs:1146 · boxes.rs:1220 |
| Spaces: constant indents/gaps (`" ".repeat(CONST)`; a space is one cell) | list.rs:204, 206, 211 · tree.rs:223 (inside the B4 clip), 232, 267, 269, 274, 276 · graph.rs:578, 888 · runs.rs:981 (`blank`) |
| Spaces: row fill after a `cell_width` measure | list.rs:231 · tree.rs `pad` (new line) |
| D6: `format!` width on compile-time ASCII labels | filter.rs:341 · item_form.rs:846 · detail/prompt.rs:271, 304 · boxes.rs:1125 · connection.rs:553 · settings/prompt.rs:1070 · qdrant.rs:459 · attach.rs:1016, 1027, 1038 · library.rs:1609 · graph.rs:568 (link kind enum) |
| D6: `format!` width on integers | detail/prompt.rs:≈276 (`{:>8}` ×2 on the new row) · library.rs `browse_row` (`v{head:<3}`) · templates.rs `template_row` (`v{head:<3}`) |

4. Run the full Validation block from the plan.
   - `git status --short crates/htui/tests/snapshots` must be empty: no OQ-1 move is expected
     (V10).
   - `git diff --stat crates/htui/tests/snapshots` must be empty.
5. Close-out: record the plan's "Correction to the item text" (diff/top_bar/transcript untouched)
   in the write-up.
