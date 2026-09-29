# MOD-63 - Settings › Qdrant `r` does nothing (done, 2026-09-29)

**Requirements:** `R-TUI-8`. Found during the README rewrite (2026-09-29).
**Design authority:** no ANA precedes this item. The fix shape is the item text's, and it mirrors
the other Settings sections' `r` and the Connection section's `send`.
**Artifacts:** none. Routed plan 2026-09-29 (0 of 4 routing criteria fired, no ultracode). The plan
was the in-thread proposal the maintainer accepted, with no plan file, because the change is one
section and one test file.
**Commits:** `8615899` (the fix and its tests), plus the close-out commit, which carries the
review's findings.

## What shipped

`QdrantSection::on_key` answers `r` in browse mode with `StoreRequest::QdrantInfo` (the section's
`wants_requests`) and consumes the key. Before, it returned `Handled::Pass`, and no tab or global
binding took `r`. It is never refused, like the Connection section's `r`. That matters most in the
unavailable state, whose only hint is `r reload`: a failed read now recovers without leaving the
section, and the next snapshot clears `unavailable`.

**Folded in, with the maintainer's ok:** the section never set `busy`. As a result, `on_snapshot`'s
"stored" and "the Qdrant settings are gone from the keyring" notices, the `… in flight` hint, and
the `Failed` arm that routes a write's refusal to the section were all dead. The failure still
reached the status line through the app's global path (`app/update.rs`). The three writes now go
through a `send` helper that sets `busy = Some(request.name())` and clears the notice, which is the
Connection section's shape.

## Tests

In `crates/htui/tests/settings.rs`:

- `qdrant_r_re_reads_and_recovers_the_unavailable_state`: `r` sends exactly one `QdrantInfo` from
  browse mode and from the unavailable state, and the next snapshot brings the `e edit` hint back.
- `a_qdrant_write_says_what_it_did_or_why_it_failed`: the clear shows its in-flight hint, then the
  cleared notice. A refused clear shows the refusal on the section and leaves no in-flight hint
  behind.

Both tests fail on the parent commit and pass on `8615899`. The whole `htui` suite
(`--features testkit`) and `cargo clippy -p htui --all-targets -D warnings` are green.

## The review's contribution

The reviewer (run as `rust-reviewer`) found nothing blocking. Three findings were applied in the
close-out commit:

- `blocked()` now refuses `e` and `c` by name (`` `clear_qdrant_settings` is still in flight ``),
  the way the Connection section does. Before, it refused silently.
- `j`/`k` no longer wait for a write, because moving the cursor is not a write.
- `on_snapshot`'s attribution is documented.

A third test, `a_qdrant_write_in_flight_refuses_e_and_c_but_not_r`, pins that behaviour.

CodeRabbit then flagged the attribution itself (medium, security). A `Qdrant` reply names no
request, so an `r` sent just before a write could land first and be taken as the write's answer. The
section would then say the settings were stored or **cleared** before the write had run, and it
kept saying so if the write then failed. The Connection section makes the same trade. Tagging the
reply with its request would mean changing `store_worker.rs`, which MOD-40 owns, so the fix stays
in the section instead: `r` sets `read_out`, and a snapshot that lands while it is set answers the
read and leaves `busy` alone. This is sound because the worker answers in order and the app's
staleness gate drops a superseded read. A write sent before the `r` answers first, so its notice
waits for the read's snapshot and is true when shown. A fourth test,
`a_qdrant_reload_just_before_a_write_is_not_taken_as_its_answer`, pins it and fails without the fix.

One nit stays as is: the tests use a small `requests_of` helper rather than matching
`bench.drained()` inline.
