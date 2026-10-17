//! The shared parts every tab draws with (MOD-115): the pane frame, list rows, key chips, status
//! glyphs, empty states and the modal frame. Styles come from [`Theme`](crate::ui::Theme) roles
//! only, and key labels arrive already resolved from [`Keys`](crate::keys::Keys), so nothing here
//! names a colour or a chord.
//!
//! Task 0 of M1 declares the surface; each submodule's body is filled in by its own task.

pub mod chips;
pub mod empty;
pub mod list;
pub mod modal;
pub mod pane;
pub mod status;
