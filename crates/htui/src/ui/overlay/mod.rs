//! Overlays: transient views drawn over the frame.

pub mod registry;
pub mod workspace_switcher;

pub use registry::{Overlay, OverlayId, OverlayRegistry, OverlayStack};
