# Blueprint for MOD-35: Add Qdrant connection settings

## Task 1: htui-store/secrets

**`crates/htui-store/src/secret.rs`**
Add the following functions and constant to manage the Qdrant DSN in the OS keyring:
```rust
/// Keyring user name of the Qdrant DSN entry.
pub const QDRANT_USER: &str = "qdrant-dsn";

/// Retrieves the stored Qdrant DSN, if any.
pub fn get_qdrant_dsn() -> Result<Option<String>>;

/// Stores the Qdrant DSN in the keyring.
pub fn set_qdrant_dsn(dsn: &str) -> Result<()>;

/// Removes the Qdrant DSN from the keyring.
pub fn clear_qdrant_dsn() -> Result<()>;
```
*Note: Update the mock keyring support (`FAKE` state/logic) to also accommodate tests for `QDRANT_USER`.*

**`crates/htui-store/src/qdrant_dsn.rs`** (New File)
Create a newtype for validating and handling the Qdrant settings safely:
```rust
use zeroize::Zeroizing;
use crate::StoreError;

#[derive(Clone)]
pub struct QdrantDsn(Zeroizing<String>);

impl QdrantDsn {
    /// Validates the text format: accepts a URL optionally followed by a space and an API key.
    pub fn parse(text: &str) -> Result<Self, StoreError>;
    
    /// Returns a redacted summary (e.g., `http://localhost:6334 [with API key]`).
    pub fn summary(&self) -> String;
    
    /// Returns a tuple of `(url, optional_api_key)` for builder usage.
    pub fn url_and_key(&self) -> (&str, Option<&str>);
    
    /// Provides access to the underlying string for the keyring store.
    pub(crate) fn as_str(&self) -> &str;
}

impl std::fmt::Debug for QdrantDsn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("QdrantDsn(<redacted>)")
    }
}
```

**`crates/htui-store/src/lib.rs`**
Expose the new module:
```rust
pub mod qdrant_dsn;
```

**`crates/htui-store/src/vector.rs`**
Update `QdrantStore::new(collection_name: &str)` to construct the store directly using the keyring credentials:
```rust
pub fn new(collection_name: &str) -> Result<Self, StoreError> {
    let text = crate::secret::get_qdrant_dsn()?
        .ok_or_else(|| StoreError::Backend("no Qdrant DSN stored".into()))?;
    let dsn = crate::qdrant_dsn::QdrantDsn::parse(&text)?;
    let (url, api_key) = dsn.url_and_key();

    let mut builder = qdrant_client::Qdrant::from_url(url);
    if let Some(key) = api_key {
        builder = builder.api_key(key);
    }
    
    let client = builder.build().map_err(|e| StoreError::Backend(e.to_string()))?;
    
    // Proceed to return Self ...
}
```

## Task 2: htui/worker

**`crates/htui/src/qdrant_dsn_info.rs`** (New File)
Define the state structures for Qdrant connection information:
```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QdrantState {
    Stored,
    NotStored,
    Unreadable,
}

#[derive(Debug, Clone)]
pub struct QdrantSnapshot {
    pub qdrant_state: QdrantState,
    pub qdrant_summary: Option<String>,
}
```

**`crates/htui/src/lib.rs`**
Expose the new module:
```rust
pub mod qdrant_dsn_info;
```

**`crates/htui/src/store_worker.rs`**
Update `StoreRequest` and `StoreReply` definitions and handle the requests:
```rust
// Inside enum StoreRequest:
    QdrantInfo,
    SetQdrantDsn(htui_store::qdrant_dsn::QdrantDsn),
    ClearQdrantDsn,

// Inside StoreRequest::name():
    Self::QdrantInfo => "qdrant_info",
    Self::SetQdrantDsn(_) => "set_qdrant_dsn",
    Self::ClearQdrantDsn => "clear_qdrant_dsn",

// Inside enum StoreReply:
    Qdrant(crate::qdrant_dsn_info::QdrantSnapshot),
```
Implement handlers in `store_worker.rs`'s event loop to invoke `secret::get_qdrant_dsn()`, `secret::set_qdrant_dsn()`, and `secret::clear_qdrant_dsn()` via blocking threads, returning `StoreReply::Qdrant(snapshot)` where `snapshot` encapsulates the resulting `QdrantState` and summary.

## Task 3: htui/ui

**`crates/htui/src/ui/tabs/settings/qdrant.rs`** (New File)
Implement the UI section representing Qdrant configuration:
```rust
use ratatui::Frame;
use ratatui::layout::Rect;
use crossterm::event::KeyEvent;

use crate::app::{Ctx, Handled};
use crate::store_worker::{StoreReply, StoreRequest};
use crate::ui::tabs::settings::{SectionId, SettingsSection};

pub struct QdrantSection {
    // fields for managing text input and UI state
}

impl SettingsSection for QdrantSection {
    fn id(&self) -> SectionId {
        SectionId("qdrant")
    }
    
    fn title(&self) -> &str {
        "Qdrant"
    }
    
    fn wants_requests(&self, _scope: &htui_core::model::Scope) -> Vec<StoreRequest> {
        vec![StoreRequest::QdrantInfo]
    }
    
    fn on_scope_change(&mut self, _scope: &htui_core::model::Scope) {}
    
    fn captures_input(&self) -> bool {
        // Return true if the user is currently editing the Qdrant connection field
        false
    }
    
    fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        // Handle 'e' to edit, 'c' to clear, and text input for the masked textfield
        Handled::Pass
    }
    
    fn on_reply(&mut self, reply: &StoreReply, _ctx: &mut Ctx<'_>) {
        // Update section state when receiving StoreReply::Qdrant
    }
    
    fn render(&self, frame: &mut Frame<'_>, area: Rect, _ctx: &Ctx<'_>) {
        // Render Qdrant status, summary, and masked textfield
    }
}
```

**`crates/htui/src/ui/tabs/settings/mod.rs`**
Register `QdrantSection`:
```rust
pub mod qdrant;
pub use qdrant::QdrantSection;
```
Ensure `QdrantSection` is added to the settings tab's sections wherever sections are registered in the application.
