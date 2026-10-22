//! The shared parts every tab draws with (MOD-115): the pane frame, list rows, key chips, status
//! glyphs, empty states, the modal frame, form rows, the filter band, display labels and text.
//! Styles come from [`Theme`](crate::ui::Theme) roles only, and key labels arrive already resolved
//! from [`Keys`](crate::keys::Keys), so nothing here names a colour or a chord.
//!
//! Task 0 of M1 declares the surface; each submodule's body is filled in by its own task. M3's
//! Task 0 adds `form`, `filter` and `label` (filled by M3 Tasks 1 and 2) and `text`.

pub mod chips;
pub mod empty;
pub mod filter;
pub mod form;
pub mod label;
pub mod list;
pub mod modal;
pub mod pane;
pub mod status;
pub mod text;
