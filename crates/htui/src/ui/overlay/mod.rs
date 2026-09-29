//! Overlays: transient views drawn over the frame.

pub mod concepts_search;
pub mod migration_prompt;
pub mod registry;
pub mod workspace_switcher;

pub use concepts_search::ConceptsSearch;
pub use migration_prompt::MigrationPrompt;
pub use registry::{Overlay, OverlayId, OverlayRegistry, OverlayStack};
pub use workspace_switcher::WorkspaceSwitcher;
