# ANA-15 - Bugsink integration via Sentry crate (concluded, 2026-09-14)

## Summary

The project requires error telemetry and evaluated whether Bugsink (a self-hostable Sentry alternative) can be integrated using the standard `sentry` Rust crate and its support crates (`sentry-anyhow`, etc.).

Full analysis: `docs/ANA-15.md`

## What was decided

**Verdict:** Yes, Bugsink is fully compatible with the standard `sentry` SDK, including support crates like `sentry-anyhow`. Because the support crates operate locally to translate Rust-specific errors into standard Sentry event JSON before transmission, Bugsink can ingest them without any custom client.

## Downstream items

- **MOD-29** - Bugsink integration via Sentry crate (spawned).

## Commits

- `docs/ANA-15.md`, this write-up, the `DECISIONS.md` index line and the `HANDOFF.md` close-out (ANA-15 line removed, MOD-29 opened, summary table and status line updated).
