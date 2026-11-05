//! Top-level screens. Registration lives in [`register_all`](crate::app::register_all).

pub mod backlog;
pub mod chat;
pub mod logs;
pub mod registry;
pub mod requirements;
pub mod settings;
pub mod skills;

pub use backlog::BacklogTab;
pub use chat::ChatTab;
pub use logs::LogsTab;
pub use registry::{Tab, TabId, TabRegistry};
pub use requirements::RequirementsTab;
pub use settings::SettingsTab;
pub use skills::SkillsTab;
