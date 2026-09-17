//! The kinds section of the Settings tab: every project of the scope with its item kinds, the
//! graphs those kinds run and the phases of each graph, editable in place (MOD-15 milestone 4,
//! D4/D10/D11/D15/D16; `R-ENT-6`, `R-ORCH-1`, `R-TUI-8`).
//!
//! It holds **no store handle, no `UserId` and no `BoxId`** (`R-NF-3`): it names one read
//! ([`StoreRequest::Catalogue`]), is handed the catalogue that comes back, and every write leaves
//! through `ctx.request` for [`crate::catalogue::serve`] to carry out. What is on screen is always
//! the last snapshot the worker assembled — no row is ever patched in locally, so there is exactly
//! one source of truth (D3).
//!
//! Four modes, and the mode is what [`captures_input`](SettingsSection::captures_input) is derived
//! from rather than a flag of its own: in `Browse` the tab still cycles on `h`/`l` and the global
//! table still owns `q`; while an editor, the prefix warning or a delete confirmation is open those
//! letters are text and are swallowed, with `CONTROL` chords excepted so `ctrl-c` still quits.

use ratatui::Frame;
use ratatui::layout::Rect;

use htui_core::model::Scope;

use crate::app::{Ctx, Handled};
use crate::catalogue::CatalogueSnapshot;
use crate::store_worker::{StoreReply, StoreRequest};
use crate::ui::tabs::settings::{SectionId, SettingsSection, message};
use crossterm::event::KeyEvent;

/// What the rows pane says before any catalogue has arrived.
const NO_WORKSPACE: &str = "no workspace: nothing to list";

/// What the rows pane says when the read itself was refused.
const UNAVAILABLE: &str = "catalogue unavailable";

/// The catalogue of the scope, with the keys that edit it.
#[derive(Debug, Default)]
pub struct KindsSection {
    /// The last catalogue the worker assembled, or `None` before the first reply.
    snapshot: Option<CatalogueSnapshot>,
    /// `Some(message)` after `Failed { request: "catalogue" }`: the read itself was refused.
    unavailable: Option<String>,
}

impl KindsSection {
    /// Identity of the kinds section.
    pub const ID: SectionId = SectionId("kinds");

    /// A section with nothing read yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl SettingsSection for KindsSection {
    fn id(&self) -> SectionId {
        Self::ID
    }

    fn title(&self) -> &str {
        "Kinds"
    }

    fn wants_requests(&self, scope: &Scope) -> Vec<StoreRequest> {
        vec![StoreRequest::Catalogue(scope.clone())]
    }

    fn on_scope_change(&mut self, _scope: &Scope) {
        self.snapshot = None;
    }

    fn on_key(&mut self, _key: KeyEvent, _ctx: &mut Ctx<'_>) -> Handled {
        Handled::Pass
    }

    fn on_reply(&mut self, reply: &StoreReply, _ctx: &mut Ctx<'_>) {
        match reply {
            StoreReply::Catalogue(snapshot) => {
                self.unavailable = None;
                self.snapshot = Some((**snapshot).clone());
            }
            StoreReply::Failed { request, message } if *request == "catalogue" => {
                self.unavailable = Some(message.clone());
            }
            _ => {}
        }
    }

    fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        let text = match &self.unavailable {
            Some(why) => format!("{UNAVAILABLE}: {why}"),
            None => NO_WORKSPACE.to_owned(),
        };
        message(frame, area, &text, ctx.theme);
    }
}
