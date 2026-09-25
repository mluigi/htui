//! The Templates view of the Skills tab (MOD-9 milestone 1; plan D6, D11–D14; blueprint D19, D20,
//! D27): the scope's prompt templates as a tree, any version's body, a line diff between two
//! versions or a version and the compiled default, and an editor that saves through `parse`.

use htui_core::model::ProjectId;
use ratatui::Frame;
use ratatui::layout::Rect;

use crate::app::{Ctx, Handled};
use crate::editor::ExternalEditOutcome;
use crate::store_worker::StoreReply;
use crate::templates::TemplatesSnapshot;
use crate::ui::{TextArea, TextField};
use crossterm::event::KeyEvent;

/// The Templates view. Holds no store handle and no `UserId` (`R-NF-3`).
#[derive(Debug, Default)]
pub(super) struct TemplatesView {
    /// The last read, or `None` before the first reply.
    snapshot: Option<TemplatesSnapshot>,
    /// A refused read's message.
    unavailable: Option<String>,
    /// The highlighted row, an index into `rows()`.
    cursor: usize,
    /// The version shown for the selected name; `None` is the head.
    shown: Option<i32>,
    /// What `d` diffs against, once `b` or `D` chose it.
    base: Option<DiffBase>,
    /// Which pane the Browse layout shows.
    pane: Pane,
    /// Browsing, naming a new template, or editing one.
    mode: Mode,
    /// The write in flight, by `StoreRequest::name`.
    busy: Option<&'static str>,
    /// The last outcome, one line above the hint.
    notice: Option<Notice>,
    /// What `E`/`Ctrl+E` asked `$EDITOR` for, until `on_external_edit`.
    external: Option<Pending>,
}

/// What `d` diffs the shown version against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DiffBase {
    /// One stored version.
    Version(i32),
    /// The compiled default (`prompt::body_of`).
    Default,
}

/// The right-hand pane in Browse.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum Pane {
    /// The shown version's body.
    #[default]
    Body,
    /// The diff against the base.
    Diff,
}

/// What the keys are doing.
#[derive(Debug, Default)]
enum Mode {
    /// Moving through the tree.
    #[default]
    Browse,
    /// Typing a new template's name.
    Naming {
        /// The project the new name goes into.
        project: ProjectId,
        /// The name.
        field: TextField,
    },
    /// The in-app editor.
    Editing(Editor),
}

/// An open editor.
#[derive(Debug)]
struct Editor {
    /// The template's project.
    project: ProjectId,
    /// The template's name.
    name: String,
    /// The head version when the editor opened: the compare-and-set token.
    token: Option<i32>,
    /// The draft.
    area: TextArea,
}

/// An `$EDITOR` handoff in flight.
#[derive(Debug)]
struct Pending {
    /// The editor the outcome opens or returns to.
    editor: Editor,
    /// Whether the handoff came from the in-app editor (`Ctrl+E`).
    resume: bool,
}

/// One line of report.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Notice {
    /// Dim.
    Info(String),
    /// `theme.error`.
    Error(String),
}

impl TemplatesView {
    /// Whether an editor or the name prompt is taking every key.
    pub(super) fn captures_input(&self) -> bool {
        !matches!(self.mode, Mode::Browse)
    }

    /// The scope changed: everything read or open belongs to the workspace that was left.
    pub(super) fn on_scope_change(&mut self) {
        let notice = self.notice.take();
        *self = Self {
            notice,
            ..Self::default()
        };
    }

    /// A key the tab did not take for the view switch.
    pub(super) fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        let _ = (key, ctx);
        todo!("MOD-9 T5: the Templates view's keys")
    }

    /// A reply addressed to the Skills tab.
    pub(super) fn on_reply(&mut self, reply: &StoreReply, ctx: &mut Ctx<'_>) {
        let _ = (reply, ctx);
        todo!("MOD-9 T5: the Templates view's replies")
    }

    /// The `$EDITOR` handoff came back.
    pub(super) fn on_external_edit(&mut self, outcome: ExternalEditOutcome, ctx: &mut Ctx<'_>) {
        let _ = (outcome, ctx);
        todo!("MOD-9 T5: the $EDITOR outcome")
    }

    /// Draws the view below the switch line.
    pub(super) fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        let _ = (frame, area, ctx);
        todo!("MOD-9 T5: the Templates view's frame")
    }
}

#[cfg(test)]
mod tests {
    use htui_core::fixtures::ids;
    use htui_core::model::Scope;
    use htui_core::store::MemStore;
    use htui_store::Backend;

    use super::*;
    use crate::app::{Action, Emit, TopBarState};
    use crate::keymap::Keymap;
    use crate::store_worker::{Origin, StoreRequest};
    use crate::templates;
    use crate::ui::Theme;
    use crate::ui::tabs::SkillsTab;
    use crossterm::event::{KeyCode, KeyModifiers};

    /// The Harness's startup scope: the Graphics workspace and its one project.
    fn vulkan() -> Scope {
        Scope {
            workspace_id: ids::WORKSPACE_GRAPHICS,
            project_ids: vec![ids::PROJECT_VULKAN],
        }
    }

    /// One served reply, as the worker would send it.
    async fn serve(backend: &Backend, request: StoreRequest) -> StoreReply {
        templates::serve(backend, &request)
            .await
            .unwrap_or_else(|err| panic!("the template request failed: {err}"))
    }

    /// D27 (F-J): a `Templates` read served while a save is in flight — `Tab` away and back, `2`,
    /// or `r` — must not be taken for the save's answer. `settle` serves in queue order, save
    /// first, so only a direct drive can put a read's reply ahead of the save's.
    #[tokio::test]
    async fn a_read_reply_does_not_close_the_editor_mid_save() {
        let backend = Backend::memory(MemStore::demo());
        let scope = vulkan();
        let (top_bar, keymap, theme, emit) = (
            TopBarState::default(),
            Keymap::default_global(),
            Theme::default(),
            Emit::default(),
        );
        let mut ctx = Ctx::new(
            &scope,
            &[],
            &top_bar,
            &keymap,
            &theme,
            Origin::Tab(SkillsTab::ID),
            &emit,
        );
        let mut view = TemplatesView::default();
        let untouched = serve(&backend, StoreRequest::Templates(scope.clone())).await;
        view.on_reply(&untouched, &mut ctx);

        // The tree is `[vulkan, fix, handoff, implement, …]`: three `j`s reach `implement`.
        for _ in 0..3 {
            view.on_key(
                KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE),
                &mut ctx,
            );
        }
        view.on_key(
            KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE),
            &mut ctx,
        );
        view.on_key(
            KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE),
            &mut ctx,
        );
        view.on_key(
            KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL),
            &mut ctx,
        );
        let sent: Vec<StoreRequest> = emit
            .take()
            .into_iter()
            .filter_map(|action| match action {
                Action::Store(request) => Some(request),
                _ => None,
            })
            .collect();
        let [save] = sent.as_slice() else {
            panic!("exactly the save was sent: {sent:?}");
        };
        let StoreRequest::SaveTemplate { name, expected, .. } = save else {
            panic!("not a save: {save:?}");
        };
        assert_eq!((name.as_str(), *expected), ("implement", Some(1)));
        assert_eq!(view.busy, Some("save_template"));

        // A read at the token's own head: the save has not landed in it.
        view.on_reply(&untouched, &mut ctx);
        assert!(
            matches!(&view.mode, Mode::Editing(editor) if editor.name == "implement"),
            "a read whose head is the token is not the save's answer: {:?}",
            view.mode
        );
        assert_eq!(
            view.busy,
            Some("save_template"),
            "the save is still in flight"
        );

        // The save's own answer: the head is token + 1.
        let saved = serve(&backend, save.clone()).await;
        view.on_reply(&saved, &mut ctx);
        assert!(
            matches!(view.mode, Mode::Browse),
            "the head moved past the token, so the save landed: {:?}",
            view.mode
        );
        assert_eq!(view.busy, None);
        assert_eq!(view.notice, Some(Notice::Info("saved v2".to_owned())));
    }
}
