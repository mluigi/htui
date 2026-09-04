//! Overlays: transient views drawn over the frame.

pub mod migration_prompt;
pub mod registry;
pub mod workspace_switcher;

pub use migration_prompt::MigrationPrompt;
pub use registry::{Overlay, OverlayId, OverlayRegistry, OverlayStack};
pub use workspace_switcher::WorkspaceSwitcher;
