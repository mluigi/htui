# ANA-6 - Architectural necessity and alternatives to OneDev as issue tracker (done, 2026-09-03)

## Summary

Resolved by the requirements session that produced `docs/REQUIREMENTS.md`; no separate analysis
file was written. The question was whether an external OneDev instance is a necessary part of the
`htui` topology.

## What was decided

1. **Postgres is the sole source of truth** for items, story, notes, documents, runs and transcripts
   (`R-ID-3`). The dual-canonical split from ANA-1, where the issue tracker owned discussion, is
   dropped.
2. **Human discussion lives in Postgres** as item notes (`R-ENT-11`); transcripts are stored in full
   in Postgres after scrubbing (`R-HIS-1`). No attachment upload path to a tracker.
3. **Issue trackers are optional downstream mirrors** behind an `IssueSync` trait (`R-LATER-2`),
   later tier. OneDev, GitHub or others plug in as sinks; none is required to run `htui`.
4. **No extra infrastructure** beyond Postgres and the agents (`R-NF-2`).

## Downstream items

- MOD-5 (issue tracker sync) moves to the deferred backlog as a later-tier mirror.
- ANA-9 (data model v2) carries no tracker tables; a mirror adds its own mapping table when built.
