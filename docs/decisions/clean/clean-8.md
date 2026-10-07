# CLEAN-8 - MOD-10 M4 review residuals (done, 2026-10-07)

**Requirements:** `R-SEC-3`, `R-TUI-8`.
**Origin:** MOD-10 (`docs/decisions/mod/mod-10.md`, M4 review: L-7, the 7 rust-reviewer NITs, the `--demo`
Qdrant note).

**What was done.** Run with MOD-90 in the same sandbox run, on `hr/MOD-90` (`docs/decisions/mod/mod-90.md`). Plan
`.claude/plans/mod-90-clean-8-worker-latch-residuals.plan.md` (D5, D6) and blueprint `.claude/plans/mod-90-clean-8-worker-latch-residuals.blueprint.md` (A-1..A-5).
Lanes B, C and D ran in parallel worktrees with disjoint file sets and merged without conflicts. No migration, no
`.sqlx` change, no changed snapshot.

- **#1 L-7, the response body is wiped.** `read_body` (`htui-secrets/src/infisical.rs`) returns
  `Zeroizing<Vec<u8>>`, pre-sized from `Content-Length` (capped). A pure `append` (`#[must_use]`, R1 L-5) never lets
  the buffer reallocate in place: past capacity it moves into a fresh zeroizing buffer, and the old one drops wiped.
  The cap and the 401 rule are unchanged. reqwest's own chunks and the TLS buffers are listed among the copies htui
  cannot wipe (R1 L-1). `df9d8de0`.
- **#2, `ProjectPatch.secret` keeps `Some(None)` on the wire.** It uses the existing `present_option` idiom
  (`default`, `deserialize_with`, `skip_serializing_if`). D5: `RepoPatch.remote_url` and `ItemPatch.step_graph_id`
  have the same latent shape, are never serialized in production, and are not fixed. `4c1fc92a`.
- **#3, `SecretScope::to_column` borrows** through a `ScopeColumnRef<'_>`, and the column text is byte-identical.
  `SecretScope`'s serde `into` still clones. `4c1fc92a`.
- **#4, the identity is stored trimmed.** `IdentityEntry::to_identity` trims both halves, matching the form.
  `aa0bbad5`.
- **#5, `SecretsSection` rows without allocating:** `row_count` + `row_at`. `1f9095b8`.
- **#6, one tree re-read:** `fresh_tree` is the only "snapshot or `NotFound` workspace" in `hierarchy.rs`, used by
  `reread`, `cas` and both `infer` sites. `d543ba00`.
- **#7, typed half-stored identity.** Adds `htui_store::secret::IdentityRead` and `read_machine_identity`, plus
  `half_identity_sentence`. `get_machine_identity` wraps them with the same sentence byte for byte (pinned by a
  literal). `identity_state` no longer sniffs the message prefix. `StoreError` is untouched. `ac6c070f`.
- **#8, long functions split:** `on_failed` (all four `Failed` arms, A-1), `on_tree_gone`, `on_scope_stale`,
  `project_lines`, `guide`, and one `style_of` (R1 nit). These are pure moves. `64caf423`, `2c285783`.
- **#9, `--demo` guard on Settings > Qdrant (D6).**
  - `qdrant_settings_info::serve`: on `Backend::Memory` the read is `QdrantState::NotApplicable` (`n/a in a demo
    session`), and writes are refused with `a demo session has no keyring to change`.
  - The section refuses `e`/`c` and sends nothing.
  - A-2: the four worker-loop arms were deleted, so `--demo`, the TUI and the harness share `try_serve`'s path.
  - The tests check both `store_worker::serve` and a spawned loop, and hold `mock_keyring_broken` so a regression
    cannot touch the real keyring (R1 L-2, also applied to the two Secrets demo precedents).
  - `6f949799`, `de48ce32`.

**Side effect (A-3).** A non-demo offline harness that opens Settings now gets a real `Qdrant` answer instead of
`Failed "handled in worker loop"`, so it reads the Qdrant keyring (read-only). No test or snapshot changed.

**Not done (D6).** Concepts search and indexing still read the Qdrant keyring in a demo session
(`concepts_worker.rs`, `concepts.rs`); they were not checked.

**Review.** rust-reviewer: approve with fixes. Its CLEAN-8 findings (L-1, L-2, L-5 and the NITs) were applied in
`2c285783` and `c53b5ff6`.
