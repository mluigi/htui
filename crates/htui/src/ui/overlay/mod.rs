//! Overlays: transient views drawn over the frame.

pub mod abort_confirm;
pub mod concepts_search;
pub mod leave;
pub mod migration_prompt;
pub mod queue;
pub mod quit_confirm;
pub mod registry;
pub mod waiting_list;
pub mod workspace_switcher;

pub use abort_confirm::AbortConfirm;
pub use concepts_search::ConceptsSearch;
pub use leave::LeaveDialog;
pub use migration_prompt::MigrationPrompt;
pub use queue::QueueOverlay;
pub use quit_confirm::QuitConfirm;
pub use registry::{Overlay, OverlayId, OverlayRegistry, OverlayStack};
pub use waiting_list::WaitingList;
pub use workspace_switcher::WorkspaceSwitcher;
