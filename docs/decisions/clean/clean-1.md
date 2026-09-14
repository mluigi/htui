# CLEAN-1 - `cargo doc` cannot build `htui-agent` (shipped, 2026-09-14)

Fixed `rustdoc` private and ambiguous links in `htui-agent`.
- Demoted private intra-doc links to code backticks (e.g. `` `descend` ``) since they refer to private items that cannot be linked in public documentation under the `deny(warnings)` rules.
- Qualified ambiguous intra-doc links to specify the module or function (e.g. `[`mod@install`]`).
- The command `cargo doc -p htui-agent` now runs successfully without requiring `--document-private-items`.
