//! MOD-11 D12, B-3: the concept index `search_concepts` queries, adapted from the binary's
//! `ConceptIndex` to `htui_mcp::search::ConceptSearch`.
//!
//! T6 ships the seam with no index behind it: [`production`] answers `None`, so the tool is not
//! advertised (I-7). T7 fills this module.

use std::sync::Arc;

/// The production concept search for `htui_mcp::McpHost::with_search`, or `None` when the build
/// has none to offer (T7 fills this).
#[must_use]
pub fn production() -> Option<Arc<dyn htui_mcp::search::ConceptSearch>> {
    None
}
