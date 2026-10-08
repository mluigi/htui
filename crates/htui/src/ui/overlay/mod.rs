//! Overlays: transient views drawn over the frame.

pub mod concepts_search;
pub mod migration_prompt;
pub mod queue;
pub mod registry;
pub mod waiting_list;
pub mod workspace_switcher;

pub use concepts_search::ConceptsSearch;
pub use migration_prompt::MigrationPrompt;
pub use queue::QueueOverlay;
pub use registry::{Overlay, OverlayId, OverlayRegistry, OverlayStack};
pub use waiting_list::WaitingList;
pub use workspace_switcher::WorkspaceSwitcher;
