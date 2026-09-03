//! Top-level screens. Registration lives in [`register_all`](crate::app::register_all).

pub mod backlog;
pub mod registry;
pub mod settings;
pub mod skills;

pub use backlog::BacklogTab;
pub use registry::{Tab, TabId, TabRegistry};
pub use settings::SettingsTab;
pub use skills::SkillsTab;
