# CLEAN-3 - `cargo doc --workspace --no-deps` has never been green (done, 2026-09-19)

Fixed broken intra-doc links across the workspace (`htui-store`, `htui-core`, `htui-agent`).

The doc gate was confirmed to document with `--all-features`, as workspace docs should cover internal API surfaces including `testkit`. 

Intra-doc links to private items and items hidden behind feature gates were replaced with standard code backticks. 
Additionally, dead links left behind by CLEAN-2 (which removed the offline buffered-write path `Writer::Buffered` and `BufferedWriter`) were removed from `backend.rs` and `writer.rs`.

Commit: 06442b7
