//! The Backlog's writes (MOD-115 M4 T1): the filter form's apply, and the item form's opening,
//! saves and their answers (applied, diverged, failed).

use htui_core::model::ItemId;

use crate::app::{Action, Ctx, Handled, RevealTarget};
use crate::item_writes::{self, ItemDivergence, ItemFormContext, ItemWrite};
use crate::keys::{Act, views};
use crate::store_worker::StoreRequest;
use crate::ui::kit::modal::modal_rest;
use crate::ui::tabs::backlog::filter::{BacklogFilter, FormOutcome};
use crate::ui::tabs::backlog::item_form::{
    Busy, ItemForm, ItemFormOutcome, mint_may_have_landed, mint_may_have_landed_in_the_old_scope,
};
use crate::ui::tabs::backlog::list::Selection;
use crate::ui::tabs::backlog::{BacklogTab, Opening};
use crate::ui::tabs::registry::Tab;
use crossterm::event::KeyEvent;

impl BacklogTab {
    /// A key while the filter form is open (MOD-13 D1).
    ///
    /// Every plain key the form does not use is consumed; CONTROL, ALT and function keys pass
    /// (`BACKLOG_FILTER`'s modal layer, MOD-67 M5), so `ctrl-c` quits, `Ctrl+f` searches and `F1`
    /// opens help.
    pub(super) fn on_form_key(&mut self, key: KeyEvent, ctx: &Ctx<'_>) -> Handled {
        let Some(form) = self.form.as_mut() else {
            return Handled::Pass;
        };
        // MOD-115 M2 RV1 M: the form resolves its acts from the chord pressed (`Ctx::pressed`).
        let pressed = ctx.pressed(&key);
        match form.on_pressed(key, pressed, ctx.keys()) {
            FormOutcome::Pass => {
                return modal_rest(views::BACKLOG_FILTER, pressed);
            }
            FormOutcome::Stay => {}
            FormOutcome::Cancel => self.form = None,
            FormOutcome::Refused(sentence) => ctx.emit(Action::Error(sentence)),
            FormOutcome::Apply(filter) => {
                self.form = None;
                self.apply(filter, ctx);
            }
        }
        Handled::Consumed
    }

    /// Sets the filter and re-reads: one `Items`, the request kind the unfiltered list uses, so
    /// the shell's staleness index supersedes an earlier read (MOD-13 D2). An unchanged filter
    /// still reads.
    pub(super) fn apply(&mut self, filter: BacklogFilter, ctx: &Ctx<'_>) {
        self.filter = filter;
        // A filtered reply must not decide a reveal sent before it (D251): the item may be
        // hidden. An unfiltered one may, and is the read a reveal miss waits for (review N5).
        if !self.filter.is_empty() {
            self.pending_reveal = None;
        }
        ctx.request(self.filter.to_request(ctx.scope));
    }

    /// MOD-13 milestone 2 D7: `N` reads the form's catalogue for the selected item's project, a
    /// selected header's project, else the first scope project. The selected item's kind is the
    /// hint (§9 decision 4).
    pub(super) fn open_new(&mut self, ctx: &Ctx<'_>) {
        let target = match self.selected {
            Some(Selection::Item(_)) => self
                .item()
                .map(|item| (item.project_id, Some(item.kind_id))),
            Some(Selection::Project(project)) => Some((project, None)),
            None => None,
        }
        .or_else(|| ctx.projects.first().map(|first| (first.project_id, None)));
        let Some((project, kind)) = target else {
            return;
        };
        self.opening = Some(Opening {
            project,
            item: None,
            kind,
        });
        ctx.request(StoreRequest::ItemForm {
            project,
            item: None,
        });
    }

    /// D7: `e` reads the selected item fresh with its project's catalogue; with no item
    /// selected nothing is sent.
    pub(super) fn open_edit(&mut self, ctx: &Ctx<'_>) {
        let Some((project, id)) = self.item().map(|item| (item.project_id, item.id)) else {
            return;
        };
        self.opening = Some(Opening {
            project,
            item: Some(id),
            kind: None,
        });
        ctx.request(StoreRequest::ItemForm {
            project,
            item: Some(id),
        });
    }

    /// A key while the item form is open (D8, A7).
    ///
    /// Every plain key is consumed, used by the form or not: a passed `q`, digit, `w`, `?` or
    /// `Tab` would act globally (blueprint §8). The form already applied its stack's pass rule
    /// (`ITEM_FORM`/`ITEM_DIVERGENCE`, MOD-67 M5): a `Pass` is a CONTROL, ALT or function key it
    /// did not use.
    pub(super) fn on_item_form_key(&mut self, key: KeyEvent, ctx: &Ctx<'_>) -> Handled {
        let Some(form) = self.item_form.as_mut() else {
            return Handled::Pass;
        };
        // MOD-115 M2 RV1 M: the form resolves its acts from the chord pressed (`Ctx::pressed`).
        match form.on_pressed(key, ctx.pressed(&key), ctx.keys()) {
            ItemFormOutcome::Pass => return Handled::Pass,
            ItemFormOutcome::Stay => {}
            ItemFormOutcome::Cancel => self.item_form = None,
            ItemFormOutcome::Reload(project) => ctx.request(StoreRequest::ItemForm {
                project,
                item: None,
            }),
            ItemFormOutcome::Save(request) => ctx.request(request),
            ItemFormOutcome::External(edit) => ctx.emit(Action::EditExternally(edit)),
        }
        Handled::Consumed
    }

    /// An `ItemForm` reply: a reloading form re-targets (E7); otherwise it opens a form only when
    /// it answers the read `N`/`e` sent (E8) and nothing else captures meanwhile.
    ///
    /// Review M1: an edit opens only while its item is still the selected one. The cursor may
    /// have moved while the read was out, and a form on the old item would sit under the new
    /// selection's header.
    pub(super) fn on_item_form(&mut self, context: &ItemFormContext, ctx: &Ctx<'_>) {
        if let Some(form) = self.item_form.as_mut() {
            if form.busy() == Some(Busy::Reloading(context.project)) && context.item.is_none() {
                form.retarget(context.clone());
            }
            return;
        }
        let Some(opening) = self.opening.take() else {
            return;
        };
        let answers = opening.project == context.project
            && opening.item == context.item.as_ref().map(|item| item.id);
        let moved = opening.item.is_some() && opening.item != self.selected_item();
        if !answers || moved || self.form.is_some() || self.detail.captures_input() {
            return;
        }
        self.item_form = match opening.item {
            None => Some(ItemForm::open_new(
                context.clone(),
                ctx.projects,
                opening.kind,
            )),
            Some(_) => ItemForm::open_edit(context.clone()),
        };
    }

    /// An applied write lands only on the write in flight (MOD-59's self-naming rule).
    ///
    /// An edit closes the form and re-reads the item and the list under the active filter (D9,
    /// A5). The item's reads go out only while it is still selected (review M1): a list that
    /// moved the cursor off it would otherwise put its fresh Body under another item's header,
    /// and the list re-read alone carries the edit then. A mint closes it and reveals the new
    /// item, whose miss branch clears the filter (A5).
    pub(super) fn on_item_written(&mut self, item: ItemId, outcome: &ItemWrite, ctx: &mut Ctx<'_>) {
        let Some(form) = self.item_form.as_ref() else {
            return;
        };
        match outcome {
            ItemWrite::Edited { .. }
                if form.busy() == Some(Busy::Editing) && form.item_id() == Some(item) =>
            {
                self.item_form = None;
                if self.selected == Some(Selection::Item(item)) {
                    self.read_item(item, ctx);
                }
                ctx.request(self.filter.to_request(ctx.scope));
            }
            ItemWrite::Minted { key } if form.busy() == Some(Busy::Minting) => {
                self.item_form = None;
                Tab::reveal(
                    self,
                    &RevealTarget::Item {
                        id: item,
                        key: key.clone(),
                    },
                    ctx,
                );
            }
            ItemWrite::Edited { .. } | ItemWrite::Minted { .. } => {}
        }
    }

    /// Milestone 3 D1, D3: a stale edit opens the three-way view, only on the form whose save it
    /// answers: busy editing this item, and the reply's ancestor at this form's token. A late reply
    /// from an earlier token (a resolution has since moved it) is dropped, as one for another item
    /// is.
    pub(super) fn on_item_diverged(&mut self, divergence: &ItemDivergence) {
        if let Some(form) = self.item_form.as_mut()
            && form.busy() == Some(Busy::Editing)
            && form.item_id() == Some(divergence.head.id)
            && form.token() == Some(divergence.ancestor.version)
        {
            form.open_divergence(divergence);
        }
    }

    /// An item request's `Failed`. `App::on_reply` already reports every `Failed` on the status
    /// line (A8), so the tab adds no `Action::Error` of its own but in one case below.
    ///
    /// A refused form read opens nothing (D2). A mint that may have followed a COMMIT is hedged
    /// and the whole list re-read, the filter cleared so it cannot hide the item (D11, §10.1,
    /// §10.2); a refusal given before the insert is said plainly with nothing re-read.
    ///
    /// Review L4, the one case: such a mint `Failed` that finds no minting form (a scope change
    /// dropped it, and a form opened since is not the mint's) has nowhere to show its hedge, so
    /// the tab emits a status-line hedge as an `Action::Error`. It is drained after `App`'s plain
    /// report, so the hedge is what the status line keeps. Nothing is re-read: the item would be
    /// the old scope's, so the hedge promises no re-read and no Ctrl+S, and names that scope. A
    /// refused mint stays silent there, `App`'s report says it all.
    pub(super) fn on_item_failed(&mut self, request: &str, message: &str, ctx: &Ctx<'_>) {
        let busy = self.item_form.as_ref().and_then(ItemForm::busy);
        match (request, busy) {
            (item_writes::FORM_NAME, Some(Busy::Reloading(_))) => {
                if let Some(form) = self.item_form.as_mut() {
                    form.settle(Some(message.to_owned()));
                }
            }
            (item_writes::FORM_NAME, _) => self.opening = None,
            (item_writes::MINT_NAME, Some(Busy::Minting)) => {
                let refused = item_writes::mint_refused(message);
                if let Some(form) = self.item_form.as_mut() {
                    form.settle(Some(if refused {
                        message.to_owned()
                    } else {
                        mint_may_have_landed(
                            message,
                            ctx.keys().label(views::ITEM_FORM, Act::FormSave).as_deref(),
                        )
                    }));
                }
                if !refused {
                    self.apply(BacklogFilter::default(), ctx);
                }
            }
            (item_writes::MINT_NAME, _) if !item_writes::mint_refused(message) => {
                ctx.emit(Action::Error(mint_may_have_landed_in_the_old_scope(
                    message,
                )));
            }
            (item_writes::EDIT_NAME, Some(Busy::Editing)) => {
                if let Some(form) = self.item_form.as_mut() {
                    form.settle(Some(message.to_owned()));
                }
            }
            _ => {}
        }
    }
}
