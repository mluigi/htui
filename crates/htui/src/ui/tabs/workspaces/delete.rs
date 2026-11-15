//! Deleting a workspace on the Workspaces tab (MOD-116 M3 D10, `R-TUI-14`): Settings ›
//! Hierarchy's flow, drawn as a kit `Ask` over the tab.
//!
//! `d` counts first ([`StoreRequest::DeleteReach`]), so the numbers on screen are the store's;
//! the warning names them ([`workspace_delete_warning`], the words Hierarchy draws) and says the
//! workspace's folder stays on disk (nothing on disk is removed). `y` (or `Enter`, the
//! `[workspaces]` view default) asks for the slug, typed; `Enter` with the right slug sends
//! [`StoreRequest::DeleteWorkspace`], a wrong one clears the field and says so. The store's
//! refusal (a live or a stopped run) goes back to the warning with its sentence; `y` asks for the
//! slug again, `Esc` keeps the workspace.
//!
//! Keys: the count, the warning and a refusal answer through
//! [`views::WORKSPACES_CONFIRM`]; the typed slug and the write in flight through
//! [`views::CAPTURE`], whose field sees every key first, so `n` and `q` are letters of a slug.

use crossterm::event::KeyEvent;
use htui_core::model::WorkspaceId;
use htui_core::store::{DeleteReach, DeleteTarget};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::Line;

use crate::app::{Ctx, Handled};
use crate::hierarchy::workspace_delete_warning;
use crate::keys::{Act, Hint, HintSpec, KeyChord, Keys, Stack, views};
use crate::store_worker::{StoreReply, StoreRequest};
use crate::ui::kit::modal::{Ask, modal_rest};
use crate::ui::{FieldOutcome, TextField};

/// [`StoreRequest::DeleteReach`]'s name.
pub(super) const DELETE_REACH: &str = "delete_reach";

/// [`StoreRequest::DeleteWorkspace`]'s name.
pub(super) const DELETE_WORKSPACE: &str = "delete_workspace";

/// The box's title, and its told boxes'.
pub(super) const DELETE_TITLE: &str = "Delete workspace";

/// The warning's second line (PRD D13), Hierarchy's words.
const NOT_UNDONE: &str = "Nothing here can be undone.";

/// A typed slug that is not the workspace's.
const WRONG_SLUG: &str = "that is not the slug; nothing was deleted";

/// The widest the box's content grows.
const WIDTH: u16 = 72;

/// The count's keys.
const COUNTING_HINT: HintSpec = &[Hint::All(Act::ConfirmNo, "keep")];

/// The warning's keys.
const WARN_HINT: HintSpec = &[
    Hint::One(Act::ConfirmYes, "delete"),
    Hint::All(Act::ConfirmNo, "keep"),
];

/// A refusal's keys: `y` asks for the slug again.
const REFUSED_HINT: HintSpec = &[
    Hint::One(Act::ConfirmYes, "retry"),
    Hint::All(Act::ConfirmNo, "keep"),
];

/// The typed slug's keys, the field's own.
const TYPED_HINT: HintSpec = &[
    Hint::One(Act::FieldSubmit, "delete"),
    Hint::One(Act::FieldCancel, "keep"),
];

/// The write in flight.
const IN_FLIGHT_HINT: HintSpec = &[Hint::Text("deleting\u{2026}")];

/// How far a delete has got.
#[derive(Debug)]
pub(super) enum Stage {
    /// `delete_reach` is out.
    Counting,
    /// The counts are on screen; `y` is the first confirmation.
    Warn(Box<DeleteReach>),
    /// The slug is being typed: the second confirmation, the one that deletes.
    Typed {
        /// What the warning listed.
        reach: Box<DeleteReach>,
        /// What has been typed so far.
        field: TextField,
        /// Whether the last slug submitted was wrong.
        wrong: bool,
    },
    /// `delete_workspace` is out.
    InFlight(Box<DeleteReach>),
    /// The store refused the delete: the warning again, with its sentence.
    Refused {
        /// What the warning listed; `None` when the refusal came before any count.
        reach: Option<Box<DeleteReach>>,
        /// The store's sentence.
        message: String,
    },
}

/// What one key or reply did to a delete.
#[derive(Debug, Clone)]
pub(super) enum DeleteOutcome {
    /// Nothing for the tab to do.
    Stay,
    /// Send this.
    Send(StoreRequest),
    /// The workspace is gone.
    Deleted {
        /// It.
        workspace: WorkspaceId,
    },
    /// Closed with nothing deleted.
    Closed,
    /// Closed, and this is told in the tab's box (the workspace was already gone, or the count
    /// was refused); the list is read again.
    Told(String),
    /// A chord the modal layer lets through (`ctrl-c`, `F1`): the shell's.
    Pass,
}

/// One workspace being deleted, behind two confirmations.
#[derive(Debug)]
pub(super) struct Deleting {
    /// What the requests name.
    workspace: WorkspaceId,
    /// The slug the second confirmation asks to be typed.
    slug: String,
    /// Its folder on this box as the tab shows it, for the line that says it stays.
    folder: Option<String>,
    /// How far it has got.
    stage: Stage,
}

impl Deleting {
    /// `d` on `workspace` (`slug`, its folder `folder` as shown): the count goes out first.
    pub(super) fn begin(
        workspace: WorkspaceId,
        slug: String,
        folder: Option<String>,
    ) -> (Self, StoreRequest) {
        let deleting = Self {
            workspace,
            slug,
            folder,
            stage: Stage::Counting,
        };
        (
            deleting,
            StoreRequest::DeleteReach(DeleteTarget::Workspace(workspace)),
        )
    }

    /// The stage, for tests.
    #[cfg(test)]
    const fn stage(&self) -> &Stage {
        &self.stage
    }

    /// The keys in force: the confirm stack until the slug is asked for, then the field's.
    pub(super) const fn stack(&self) -> Stack<'static> {
        match self.stage {
            Stage::Counting | Stage::Warn(_) | Stage::Refused { .. } => views::WORKSPACES_CONFIRM,
            Stage::Typed { .. } | Stage::InFlight(_) => views::CAPTURE,
        }
    }

    /// One key (`key` as delivered, `pressed` as the user pressed it).
    pub(super) fn on_key(
        &mut self,
        key: KeyEvent,
        pressed: KeyChord,
        keys: &Keys,
    ) -> DeleteOutcome {
        let stack = self.stack();
        let answer = keys.actions(stack, pressed).first().copied();
        let rest = || match modal_rest(stack, pressed) {
            Handled::Pass => DeleteOutcome::Pass,
            Handled::Consumed => DeleteOutcome::Stay,
        };
        match &mut self.stage {
            Stage::Counting => match answer {
                Some(Act::ConfirmNo) => DeleteOutcome::Closed,
                Some(Act::ConfirmYes) => DeleteOutcome::Stay,
                _ => rest(),
            },
            Stage::Warn(reach) => match answer {
                Some(Act::ConfirmYes) => {
                    self.stage = Stage::Typed {
                        reach: reach.clone(),
                        field: TextField::new(),
                        wrong: false,
                    };
                    DeleteOutcome::Stay
                }
                Some(Act::ConfirmNo) => DeleteOutcome::Closed,
                _ => rest(),
            },
            Stage::Refused { reach, .. } => match answer {
                Some(Act::ConfirmYes) => match reach.take() {
                    Some(reach) => {
                        self.stage = Stage::Typed {
                            reach,
                            field: TextField::new(),
                            wrong: false,
                        };
                        DeleteOutcome::Stay
                    }
                    None => {
                        self.stage = Stage::Counting;
                        DeleteOutcome::Send(StoreRequest::DeleteReach(DeleteTarget::Workspace(
                            self.workspace,
                        )))
                    }
                },
                Some(Act::ConfirmNo) => DeleteOutcome::Closed,
                _ => rest(),
            },
            Stage::Typed {
                reach,
                field,
                wrong,
            } => match field.on_key(key) {
                FieldOutcome::Submit if field.text() == Some(self.slug.as_str()) => {
                    self.stage = Stage::InFlight(reach.clone());
                    DeleteOutcome::Send(StoreRequest::DeleteWorkspace(self.workspace))
                }
                FieldOutcome::Submit => {
                    field.clear();
                    *wrong = true;
                    DeleteOutcome::Stay
                }
                FieldOutcome::Cancel => DeleteOutcome::Closed,
                // Typed: `n` is a letter of a slug here, not an answer.
                FieldOutcome::Consumed => DeleteOutcome::Stay,
                FieldOutcome::Pass => rest(),
            },
            // Nothing to answer: the rows are already going.
            Stage::InFlight(_) => rest(),
        }
    }

    /// A reply to the tab: the count lands, the delete lands, or either is refused. Any other
    /// reply is not this delete's.
    pub(super) fn on_reply(&mut self, reply: &StoreReply) -> DeleteOutcome {
        match reply {
            StoreReply::DeleteReach(Some(reach)) if matches!(self.stage, Stage::Counting) => {
                self.stage = Stage::Warn(reach.clone());
                DeleteOutcome::Stay
            }
            // Nothing to count: someone else got there first.
            StoreReply::DeleteReach(None) if matches!(self.stage, Stage::Counting) => {
                DeleteOutcome::Told(format!("`{}` is already gone", self.slug))
            }
            StoreReply::Deleted {
                target: DeleteTarget::Workspace(id),
                ..
            } if *id == self.workspace => DeleteOutcome::Deleted { workspace: *id },
            StoreReply::Failed { request, message } if *request == DELETE_REACH => {
                DeleteOutcome::Told(format!(
                    "`{}` cannot be counted for deleting: {message}",
                    self.slug
                ))
            }
            StoreReply::Failed { request, message } if *request == DELETE_WORKSPACE => {
                let reach = match &mut self.stage {
                    Stage::Warn(reach) | Stage::Typed { reach, .. } | Stage::InFlight(reach) => {
                        Some(reach.clone())
                    }
                    Stage::Refused { reach, .. } => reach.take(),
                    Stage::Counting => None,
                };
                self.stage = Stage::Refused {
                    reach,
                    message: message.clone(),
                };
                DeleteOutcome::Stay
            }
            _ => DeleteOutcome::Stay,
        }
    }

    /// The box over `area`: the count, the warning (and the typed slug), the write in flight, or
    /// the refusal; the stage's keys on its border.
    pub(super) fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        let theme = ctx.theme;
        let spec = match &self.stage {
            Stage::Counting => COUNTING_HINT,
            Stage::Warn(_) => WARN_HINT,
            Stage::Refused { .. } => REFUSED_HINT,
            Stage::Typed { .. } => TYPED_HINT,
            Stage::InFlight(_) => IN_FLIGHT_HINT,
        };
        let hint = ctx.keys().hint(self.stack(), spec);
        let prompt = format!("Type `{}` to confirm: ", self.slug);
        let mut ask = Ask::new(DELETE_TITLE).width(WIDTH).hint(&hint);
        let reach = match &self.stage {
            Stage::Counting => {
                ask = ask.line(Line::styled("counting rows\u{2026}", theme.base));
                None
            }
            Stage::InFlight(_) => {
                ask = ask.line(Line::styled(
                    format!("deleting `{}`\u{2026}", self.slug),
                    theme.base,
                ));
                None
            }
            Stage::Warn(reach) | Stage::Typed { reach, .. } => Some(reach.as_ref()),
            Stage::Refused { reach, .. } => reach.as_deref(),
        };
        if let Some(reach) = reach {
            for line in self.warning(reach) {
                ask = ask.line(Line::styled(line, theme.error));
            }
            if let Some(folder) = &self.folder {
                ask = ask.line(Line::styled(
                    format!("Its folder `{folder}` stays on disk."),
                    theme.dim,
                ));
            }
        }
        match &self.stage {
            Stage::Typed { field, wrong, .. } => {
                ask = ask.input(&prompt, field);
                if *wrong {
                    ask = ask.error(WRONG_SLUG);
                }
            }
            Stage::Refused { message, .. } => ask = ask.error(message.as_str()),
            Stage::Counting | Stage::Warn(_) | Stage::InFlight(_) => {}
        }
        let _ = ask.render(frame, area, theme);
    }

    /// The warning's two sentences: what the delete takes, and that it cannot be undone.
    fn warning(&self, reach: &DeleteReach) -> [String; 2] {
        [
            workspace_delete_warning(&self.slug, reach),
            NOT_UNDONE.to_owned(),
        ]
    }
}

#[cfg(test)]
mod tests {
    use htui_core::fixtures::ids;
    use htui_core::store::{DeleteReach, DeleteTarget};

    use super::{DELETE_WORKSPACE, DeleteOutcome, Deleting, Stage};
    use crate::keys::{KeyChord, Keys};
    use crate::store_worker::{StoreReply, StoreRequest};

    /// `spec` (a `KeyChord::parse` spec) on `deleting`.
    fn key(deleting: &mut Deleting, spec: &str) -> DeleteOutcome {
        let parsed = KeyChord::parse(spec).unwrap_or_else(|| panic!("`{spec}` is a chord"));
        deleting.on_key(parsed.to_event(), parsed, Keys::compiled())
    }

    /// One key per character of `text`, each taken by the field.
    fn type_text(deleting: &mut Deleting, text: &str) {
        for c in text.chars() {
            assert!(stays(&key(deleting, &c.to_string())), "{c}");
        }
    }

    fn stays(outcome: &DeleteOutcome) -> bool {
        matches!(outcome, DeleteOutcome::Stay)
    }

    fn closes(outcome: &DeleteOutcome) -> bool {
        matches!(outcome, DeleteOutcome::Closed)
    }

    /// A delete of Graphics with its count landed: the warning.
    fn warned() -> Deleting {
        let (mut deleting, request) =
            Deleting::begin(ids::WORKSPACE_GRAPHICS, "graphics".to_owned(), None);
        assert!(matches!(
            request,
            StoreRequest::DeleteReach(DeleteTarget::Workspace(id)) if id == ids::WORKSPACE_GRAPHICS
        ));
        let reach = DeleteReach {
            workspace_links: 1,
            ..DeleteReach::default()
        };
        let _ = deleting.on_reply(&StoreReply::DeleteReach(Some(Box::new(reach))));
        assert!(matches!(deleting.stage(), Stage::Warn(_)));
        deleting
    }

    fn typed_text(deleting: &Deleting) -> Option<(String, bool)> {
        match deleting.stage() {
            Stage::Typed { field, wrong, .. } => {
                Some((field.text().unwrap_or_default().to_owned(), *wrong))
            }
            _ => None,
        }
    }

    #[test]
    fn the_stages_answer_their_keys() {
        // Counting: `y` does nothing, `Esc` closes.
        let (mut deleting, _) =
            Deleting::begin(ids::WORKSPACE_GRAPHICS, "graphics".to_owned(), None);
        assert!(stays(&key(&mut deleting, "y")));
        assert!(matches!(deleting.stage(), Stage::Counting));
        assert!(stays(&key(&mut deleting, "q")), "swallowed");
        assert!(closes(&key(&mut deleting, "esc")));

        // Warn: `y` and `Enter` ask for the slug; `n` closes.
        let mut deleting = warned();
        assert!(stays(&key(&mut deleting, "y")));
        assert_eq!(typed_text(&deleting), Some((String::new(), false)));
        let mut deleting = warned();
        assert!(stays(&key(&mut deleting, "enter")));
        assert!(typed_text(&deleting).is_some());
        let mut deleting = warned();
        assert!(closes(&key(&mut deleting, "n")));
        assert!(matches!(key(&mut warned(), "ctrl-c"), DeleteOutcome::Pass));

        // Typed: `n` is a letter; a wrong slug clears the field and says so.
        let mut deleting = warned();
        let _ = key(&mut deleting, "y");
        type_text(&mut deleting, "nope");
        assert!(stays(&key(&mut deleting, "enter")));
        assert_eq!(typed_text(&deleting), Some((String::new(), true)));
        // The right slug sends the delete.
        type_text(&mut deleting, "graphics");
        assert!(matches!(
            key(&mut deleting, "enter"),
            DeleteOutcome::Send(StoreRequest::DeleteWorkspace(id)) if id == ids::WORKSPACE_GRAPHICS
        ));
        assert!(matches!(deleting.stage(), Stage::InFlight(_)));
        assert!(stays(&key(&mut deleting, "esc")), "in flight");
        assert!(matches!(
            deleting.on_reply(&StoreReply::Deleted {
                target: DeleteTarget::Workspace(ids::WORKSPACE_GRAPHICS),
                reach: Box::default(),
                mirror: crate::hierarchy::MirrorAfterDelete::NoMirror,
            }),
            DeleteOutcome::Deleted { workspace } if workspace == ids::WORKSPACE_GRAPHICS
        ));

        // `Esc` in the field keeps the workspace.
        let mut deleting = warned();
        let _ = key(&mut deleting, "y");
        assert!(closes(&key(&mut deleting, "esc")));
    }

    #[test]
    fn a_refused_delete_goes_back_to_the_warning() {
        let mut deleting = warned();
        let _ = key(&mut deleting, "y");
        type_text(&mut deleting, "graphics");
        let _ = key(&mut deleting, "enter");
        let refused = StoreReply::Failed {
            request: DELETE_WORKSPACE,
            message: "workspace `graphics` has 1 stopped run".to_owned(),
        };
        assert!(stays(&deleting.on_reply(&refused)));
        assert!(
            matches!(deleting.stage(), Stage::Refused { reach: Some(_), message }
                if message.contains("stopped run")),
            "{:?}",
            deleting.stage()
        );
        // `y` asks for the slug again; `Esc` from a refusal closes.
        assert!(stays(&key(&mut deleting, "y")));
        assert_eq!(typed_text(&deleting), Some((String::new(), false)));
        let mut deleting = warned();
        let _ = deleting.on_reply(&refused);
        assert!(closes(&key(&mut deleting, "esc")));

        // A refused count closes, told.
        let (mut deleting, _) =
            Deleting::begin(ids::WORKSPACE_GRAPHICS, "graphics".to_owned(), None);
        let outcome = deleting.on_reply(&StoreReply::Failed {
            request: super::DELETE_REACH,
            message: "the database is unreachable".to_owned(),
        });
        assert!(
            matches!(&outcome, DeleteOutcome::Told(text) if text.contains("unreachable")),
            "{outcome:?}"
        );
        assert!(matches!(
            deleting.on_reply(&StoreReply::DeleteReach(None)),
            DeleteOutcome::Told(_)
        ));
    }
}
