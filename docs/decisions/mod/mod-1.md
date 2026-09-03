# MOD-1 - TUI scaffold (done, 2026-09-04)

## Summary

First code in the repo: a Cargo workspace with `htui-core` (ANA-9 domain types, the §6.1 store
seam, an in-memory store with the ANA-9 write semantics, a conformance suite and demo fixtures)
and `htui` (the ratatui/crossterm/tokio shell with a Backlog tab, Skills and Settings stubs and
a workspace switcher). Skeleton only, read-only views, every extension point a registry.
Requirements addressed: `R-TUI-1..3` (skeleton), `R-NF-1`, `R-NF-3`.

Artifacts: PRD `.claude/prds/mod-1-tui-scaffold.prd.md`, plan
`.claude/plans/mod-1-tui-scaffold.plan.md` (verified-claims table, design decisions D1-D11),
blueprint `.claude/plans/mod-1-tui-scaffold.blueprint.md` (signatures, fixture spec, errata).

## What was built

1. **Workspace** (`Cargo.toml`, `rust-toolchain.toml` 1.98, edition 2024, `rustfmt.toml`,
   `clippy.toml`). `unsafe_code = "forbid"`, clippy `all` warned and denied at the gate, rustdoc
   intra-doc lints denied by name. Two crates under `crates/`.
2. **`htui-core`.** Every ANA-9 §5 table the TUI reads as a Rust type with the §5 column names;
   UUID newtypes per entity (v7 minted client-side); CHECK lists as enums with `as_str`/`FromStr`.
   `ReadStore` / `WriteStore` / `UpdateOutcome` copied verbatim from §6.1 as native `async fn`
   traits (`#[allow(async_fn_in_trait)]`, plan D2). `Backend` is a concrete enum
   (`Memory(MemStore)` now; MOD-6 adds `Online` / `Offline`) delegating `ReadStore`, plus inherent
   hierarchy reads (`workspaces`, `box_info`, `active_runs`, `projects`) the shell needs.
   `MemStore` enforces §4.1 (counter per `(project, prefix)`, `key_prefix` copied at mint and never
   rewritten, no delete, kind must exist and belong to the item's project, non-nil author) and
   §4.2 (compare-and-set on `version` over exactly the spec columns, `Diverged { head, ancestor }`,
   status CAS never bumps `version`, `closed_at` follows terminal status both ways). Locks are
   `std::sync::RwLock` behind closures, so no guard can live across an `.await`.
   Conformance suite (feature `test-support`): 15 named cases MOD-6 runs unchanged against
   `PgStore`; mutation-checked (a store that rewrites the key on re-kind, bumps `version` on
   status, or keeps one counter per project fails it). Demo fixtures (feature `demo`): two
   workspaces, three projects, seeded kinds/graphs/templates per `R-ENT-6`, items across every
   status, links, notes, documents, one finished and one queued run with a §4.3 event stream;
   deterministic UUIDs and timestamps so snapshots never carry a wall clock.
3. **`htui`.** Library + thin binary (`--demo`, tracing to a file only under `HTUI_LOG`).
   Terminal guard and panic hook restore the terminal on every exit path. Elm-style `App` /
   `Action` / `update` / render. Store worker task owns the `Backend`; the UI sends
   `StoreRequest` envelopes and receives addressed `StoreReply` envelopes with a
   `(origin, request kind)` staleness rule, so no view type holds a store or channel handle
   (`R-NF-3` by construction). Registries for tabs, detail sub-tabs, overlays and key bindings;
   the event loop has three `select!` arms (terminal, replies, tick) and no feature match.
   Backlog tab: items of the workspace grouped by project, fold on `Enter`, `j/k/g/G`, five
   read-only detail sub-tabs (Body, Runs, Graph at one hop, Documents, Notes) each with a
   one-line empty state. Workspace switcher overlay on `w`; the startup overlay opens only when
   the first workspace list is empty. Skills and Settings are stubs. `testkit::Harness` drives
   snapshot tests (`insta`, 100x30) without touching the shell files.
4. **README** with build, run, keys, platform notes (Windows Terminal target, conhost
   best-effort, Linux/macOS not built in this item).

## Decisions worth keeping

- ANA-9 has priority over the item text and the plan; where verifiers found the blueprint wrong,
  the code follows ANA-9 and the blueprint carries an errata section.
- The TUI scope is always a workspace (maintainer, 2026-09-03). `R-ENT-2` stays as written at the
  store level; the shell never offers the no-workspace fallback.
- Skeleton scope: filters, item editing and divergence view went to MOD-13, Graph traversal to
  MOD-14, workspace/project/repo/kind management to MOD-15; run and close actions to MOD-4,
  queue to MOD-12; Settings tab sections were assigned to MOD-2/7/10/12/15 (all opened or
  amended in `HANDOFF.md` on 2026-09-03).
- Demo fixtures are opt-in (`--demo`); an empty `MemStore` is the default.
- Native `async fn` in traits, not `async_trait`: `Backend` is concrete so spawned futures are
  `Send` by inference; the rustc lint is allowed on the two traits with a pointer to plan D2.

## Process

Routed PRD (C2 + C4) with ultracode for implement and review. Implementation ran as three
workflows on Opus: waves A+B serial (T1 core, T2 store, T3 shell, each implement then adversarial
verify), wave C parallel in git worktrees (T4 Backlog tab, T5 switcher) cherry-picked onto main
by T6, then the `rust-reviewer` gate (request-changes, 7 findings plus 2 carried) whose findings
were each adversarially verified (all real), applied, and re-reviewed to **approve**.

## Watch items for later modules

- MOD-6: unbounded request/reply channels are fine in-process; add backpressure once `PgStore`
  latency is real. `link_graph` visited set is a `Vec`; `items()` re-sorts the scope per call.
  `require_author` rejects nil only; the Postgres FK covers unknown users.
- MOD-13: `BacklogTab::on_reply` drops fold state for a project that has zero items, visible once
  filters can empty a group. `ItemPatch::default()` still yields a nil author; give it a
  constructor when the first edit path lands.
- Conformance: `run_all` is hand-listed against `CASES` (length asserted); make it iteration
  driven when MOD-6 reports per case.

## Validation

`cargo fmt --all -- --check`, `cargo clippy --workspace --all-features --all-targets -- -D
warnings`, `cargo test --workspace --all-features` (69 tests), `cargo test -p htui` (no
features), `cargo doc --workspace --no-deps` (0 warnings), `cargo run -p htui -- --demo` on
Windows Terminal. Linux and macOS not built (only the Windows target installed on this box).

## Commits

- `fdf1182` feat(mod-1): workspace, htui-core model and store seam, htui shell
- `dbb166a` feat(mod-1): backlog tab with read-only detail sub-tabs
- `73f21ec` feat(mod-1): workspace switcher overlay
- `2774907` feat(mod-1): register backlog tab and workspace switcher, add README
- `f83ac97` fix(mod-1): open the switcher only on an empty first workspace list, gate backlog tests on testkit
- `dc3d2c9` fix(mod-1): apply rust-reviewer findings F1-F7 and carried C1
