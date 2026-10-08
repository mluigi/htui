//! Everything that draws. No module below this one holds a store handle or a channel: views
//! receive a [`crate::app::Ctx`] and answer with actions (`R-NF-3`, plan D4).

mod cells;
pub mod diff;
pub mod editor_pane;
pub mod layout;
mod markdown;
pub mod overlay;
pub mod path_picker;
pub mod tabs;
pub mod text_area;
pub mod text_field;
pub mod theme;
pub mod top_bar;

pub use path_picker::{PathPicker, PickerOutcome};
pub use text_area::TextArea;
pub use text_field::{FieldOutcome, TextField};
pub use theme::Theme;
