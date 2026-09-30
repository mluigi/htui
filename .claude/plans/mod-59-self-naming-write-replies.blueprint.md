# Blueprint: MOD-59 - A write's reply names itself

**Source plan**: `.claude/plans/mod-59-self-naming-write-replies.plan.md` (CONFIRMED 2026-09-30). Produced by `code-architect` @ `5894118`.

## Architecture: MOD-59, a write's reply names itself (T1 Skills, T2 Templates, T3 Requirements)

Scope and decisions follow `.claude/plans/mod-59-self-naming-write-replies.plan.md` (D1 to D7). Four deviations are in section 6: DV-1 to DV-3 are applied in this blueprint, and DV-4 is offered but not applied. Every line number was checked on `hr/MOD-59` @ `5894118`, which has the same code as `08e0880`.

### Design Decisions
- **Three new `StoreReply` variants, one per writer.** Each one mirrors `AgentWritten` (`store_worker.rs:1122-1130`). Plain `Skills`, `Templates` and `Requirements` become read answers only (D1).
- **The outcome carries what the store returned** (D2). There is one exception: `SkillWrite::Attached` also carries the landed row's `updated_at` (DV-1).
- **`snapshot: Result<Box<Snapshot>, String>`** (D5). The `Err` string is `StoreError::to_string()`, which is the same rendering as `failed()` (`store_worker.rs:1519-1524`). `*Stale` replies are unchanged, and a stale write whose re-read fails still answers `Failed` (D3).
- **Landing gate is `busy == outcome.request_name()`** (D4). A reply with `Err` has no scope to check, so it is acted on only when its write is still in flight. A scope change resets `busy` (`library.rs:527-533`, `skills/templates.rs:331-337`, `requirements/mod.rs:1086-1088`).
- **One `landed_unread` sentence helper per view**, the way `in_flight` is duplicated per view (`library.rs:149`, `requirements/mod.rs:118`).
- **Worker-side D5 tests go through a pure mapping helper** (`written` / `saved`). `MemStore` cannot fail a read after an applied write within one `serve`: `MemFault` only covers `RefreshLease`, `ReleaseLease` and `ItemTransition` (`htui-core/src/store/mem.rs:112-119`).

### Files to Modify
| File | Changes | Task |
|---|---|---|
| `crates/htui/src/store_worker.rs` | imports `:50-54`; variant per task; re-doc `Templates`/`Skills`/`Requirements` (`:1073-1075`, `:1079-1081`, `:1131-1133`) and the write requests' docs (`:597-598`, `:612-617`, `:627-630`, `:640-643`, `:653-656`, `:729`, `:740`, `:755-756`, `~:773`) | T1, T2, T3 |
| `crates/htui/src/skills.rs` | `SkillWrite`, `answer`, `written`, serve arms `:245-328`, docs `:12-14`, `:217-234`, `:374-376`; tests | T1 |
| `crates/htui/src/ui/tabs/skills/library.rs` | `on_reply`, `land`, `landed_version`, `landed_unread`, doc `:6-9`, drop import `:26`; tests | T1 |
| `crates/htui/src/ui/tabs/skills/attach.rs` | `on_landed` takes the token (DV-1) `:278-304` | T1 |
| `crates/htui/src/templates.rs` | `serve` SaveTemplate arm `:142-170`, `saved`, doc `:8-10`; tests | T2 |
| `crates/htui/src/ui/tabs/skills/templates.rs` | `on_reply` `:349-394`, `land_save` `:831-889`, `landed_unread`; tests | T2 |
| `crates/htui/src/requirements.rs` | `RequirementWrite`, `answer`, `written`, serve arms `:391-526`, delete `is_diverged` `:729-732`, docs `:5-9`, `:30-32`, `:734-736`; tests | T3 |
| `crates/htui/src/ui/tabs/requirements/mod.rs` | new arm, `land` rewrite; remove `CHECKING_MINT`, `verifying`, `Sent` (DV-3); simplify `Failed`; tests | T3 |
| `crates/htui/tests/requirements_pg.rs` | `applied()` `:72-78` | T3 |

No files are created.

### Data Flow
1. The view sends a write and sets `busy = request.name()` (`library.rs:1186-1191`, `skills/templates.rs:727-739`, `requirements/mod.rs:733-738`).
2. The worker writes, turns the writer's return value into the outcome, re-reads, and answers `*Written { snapshot: Ok|Err, outcome }`. A stale write still answers `*Stale`, and a refusal still answers `Failed`.
3. `App::on_reply` runs the `(Origin, Discriminant<StoreRequest>)` freshness gate (`app/state.rs:164`, `:329-333`) and routes the reply to the tab.
4. The view lands the reply only when `busy == outcome.request_name()`. The `Ok` snapshot is taken if it is in scope. With `Err`, the held snapshot stays drawn, `unavailable` is set only when the view holds no snapshot, and the notice reports the failed re-read.

---

## 1. Exact type definitions

### 1.1 `StoreReply` variants (`crates/htui/src/store_worker.rs`)

`store_worker.rs` uses std `Result`, because the store alias is imported as `StoreResult` (`:31-33`). The enum derives `Debug, Clone` (`:929`). Every field needs a doc comment because of `#![warn(missing_docs)]` (`crates/htui/src/lib.rs:10`) and `-D warnings`.

T2, inserted after `TemplatesStale` (`:1076-1078`):
```rust
    /// The answer to a [`StoreRequest::SaveTemplate`] that applied (MOD-59 D1): the templates
    /// re-read after it and the row the store appended. Self-naming: the Templates view lands its
    /// save on this variant alone, and a plain [`StoreReply::Templates`] never closes its editor or
    /// moves its token.
    TemplateSaved {
        /// The templates as they are now; or, when only the re-read failed, its `StoreError`
        /// rendered through `Display`. The version was appended either way (D5).
        snapshot: Result<Box<TemplatesSnapshot>, String>,
        /// The template's project, as stored.
        project: ProjectId,
        /// The template's name, as stored.
        name: String,
        /// The version the save appended.
        version: i32,
    },
```

T1, inserted after `SkillsStale` (`:1082-1090`):
```rust
    /// The answer to every skill write that applied (MOD-59 D1): the snapshot re-read after it,
    /// and what the write did. Self-naming: the Skills view lands a write on this variant alone,
    /// and a plain [`StoreReply::Skills`] never closes its editor, form or question.
    SkillWritten {
        /// The snapshot as it is now; or, when only the re-read failed, its `StoreError` rendered
        /// through `Display`. The write landed either way (D5).
        snapshot: Result<Box<SkillsSnapshot>, String>,
        /// What the write did.
        outcome: SkillWrite,
    },
```

T3, inserted after `RequirementsStale` (`:1134-1136`):
```rust
    /// The answer to every tab write that applied (MOD-59 D1): the scope re-read after it, and
    /// what the write did. Self-naming: the Requirements tab lands a write on this variant alone,
    /// and a plain [`StoreReply::Requirements`] never closes its form.
    RequirementWritten {
        /// The requirements as they are now; or, when only the re-read failed, its `StoreError`
        /// rendered through `Display`. The write landed either way (D5).
        snapshot: Result<Box<RequirementsSnapshot>, String>,
        /// What the write did.
        outcome: RequirementWrite,
    },
```

Re-document the read variants. For example, `Skills`: "The Skills view's snapshot, freshly read: the answer to [`StoreRequest::Skills`] (MOD-9 D81). A read answer only: an applied write answers [`StoreReply::SkillWritten`] (MOD-59)." `Templates` and `Requirements` get the same shape.

Imports:
- `:54` becomes `use crate::skills::{self, SkillWrite, SkillsSnapshot, StaleWhat};`
- `:50-52` add `RequirementWrite`.

Size check: each new variant is about 72 to 112 bytes, comparable to `AgentWritten` and `StoreState`, so `clippy::large_enum_variant` is not at risk.

### 1.2 `SkillWrite` (`crates/htui/src/skills.rs`, inserted before `serve`'s doc at `:217`)

Needs `use chrono::{DateTime, Utc};` at the top; skills.rs has no chrono import outside its tests.
```rust
/// What one skill write did (MOD-59 D2), carried by [`StoreReply::SkillWritten`] beside the
/// snapshot re-read. Ids, names, keys and tokens only: a body stays in the snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkillWrite {
    /// [`StoreRequest::CreateSkill`]: the skill and its version 1 landed.
    Created {
        /// The id the worker minted.
        skill: SkillId,
        /// The name, as stored.
        name: String,
    },
    /// [`StoreRequest::SaveSkillVersion`]: a version was appended.
    Versioned {
        /// Which skill.
        skill: SkillId,
        /// The version the store appended.
        version: i32,
    },
    /// [`StoreRequest::EditSkill`]: the rename or re-describe applied.
    Edited {
        /// Which skill.
        skill: SkillId,
        /// The name, as stored.
        name: String,
    },
    /// [`StoreRequest::SetSkillBinding`] with an attach: the row at `key` is the one sent.
    Attached {
        /// The attachment's key.
        key: SkillBindingKey,
        /// The landed row's `updated_at`: the token a kept edit saves over (MOD-59 DV-1).
        updated_at: DateTime<Utc>,
    },
    /// [`StoreRequest::SetSkillBinding`] with a detach: no row at `key`.
    Detached {
        /// The attachment's key.
        key: SkillBindingKey,
    },
}

impl SkillWrite {
    /// The [`StoreRequest::name`] of the write this answers: what the view's `busy` holds while
    /// it is in flight, so only that write lands on it (MOD-59 D4).
    #[must_use]
    pub const fn request_name(&self) -> &'static str {
        match self {
            Self::Created { .. } => REQUEST_NAMES[1],
            Self::Edited { .. } => REQUEST_NAMES[2],
            Self::Versioned { .. } => REQUEST_NAMES[3],
            Self::Attached { .. } | Self::Detached { .. } => REQUEST_NAMES[4],
        }
    }
}
```

### 1.3 `RequirementWrite` (`crates/htui/src/requirements.rs`, inserted before `serve`'s doc at `:383`)

`RequirementAreaId` and `RequirementId` are already imported (`:38-43`).
```rust
/// What one tab write did (MOD-59 D2), carried by [`StoreReply::RequirementWritten`] beside the
/// scope re-read. Ids, codes, keys and versions only: a body stays in the snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequirementWrite {
    /// [`StoreRequest::CreateRequirementArea`]: the area landed.
    Area {
        /// The area's id.
        id: RequirementAreaId,
        /// Its code, as stored (trimmed).
        code: String,
    },
    /// [`StoreRequest::MintRequirement`]: the requirement landed at version 1.
    Minted {
        /// The id the worker minted.
        id: RequirementId,
        /// Its key, `R-<code>-<number>`.
        key: String,
    },
    /// [`StoreRequest::AmendRequirement`] at the head: the new head.
    Amended {
        /// The requirement.
        id: RequirementId,
        /// Its key.
        key: String,
        /// The version written.
        version: i32,
    },
    /// [`StoreRequest::WithdrawRequirement`] at the head.
    Withdrawn {
        /// The requirement.
        id: RequirementId,
        /// Its key.
        key: String,
    },
}

impl RequirementWrite {
    /// The [`StoreRequest::name`] of the write this answers: what the tab's `busy` holds while it
    /// is in flight (MOD-59 D4).
    #[must_use]
    pub const fn request_name(&self) -> &'static str {
        match self {
            Self::Area { .. } => REQUEST_NAMES[3],
            Self::Minted { .. } => REQUEST_NAMES[4],
            Self::Amended { .. } => REQUEST_NAMES[5],
            Self::Withdrawn { .. } => REQUEST_NAMES[6],
        }
    }
}
```

### 1.4 Request names

These were checked against `StoreRequest::name` (`store_worker.rs:817-923`) and the `REQUEST_NAMES` arrays (`skills.rs:204`, `requirements.rs:209-223`, `templates.rs:121`).

| Outcome | name | index |
|---|---|---|
| `Created` | `create_skill` | `skills::REQUEST_NAMES[1]` |
| `Edited` | `edit_skill` | `[2]` |
| `Versioned` | `save_skill_version` | `[3]` |
| `Attached` / `Detached` | `set_skill_binding` | `[4]` |
| `TemplateSaved` | `save_template` | `templates::REQUEST_NAMES[1]` (the view's `SAVE_NAME`, `skills/templates.rs:33`) |
| `Area` | `create_requirement_area` | `requirements::REQUEST_NAMES[3]` |
| `Minted` | `mint_requirement` | `[4]` |
| `Amended` | `amend_requirement` | `[5]` |
| `Withdrawn` | `withdraw_requirement` | `[6]` |

### 1.5 The D5 sentence (one copy each in `library.rs`, `skills/templates.rs`, `requirements/mod.rs`, next to `in_flight`)
```rust
/// MOD-59 D5: a write that applied although its re-read failed. It landed; the view still draws
/// what it held, and `r` reads again.
fn landed_unread(landed: &str, why: &str) -> String {
    format!("{landed} \u{2014} the re-read failed, r reloads: {why}")
}
```
It is always used as `Notice::Error(landed_unread(&landed, why))`. The `Ok` path keeps `Notice::Info(landed)`. `landed` is exactly today's sentence:
- `saved v3 · ~N tokens`, `created \`x\` v1 · ~N tokens`, `saved \`name\``, `attached to …`, `detached from …` (`library.rs:1265`, `:1277-1283`, `:1345-1371`)
- `saved v{n}` (`skills/templates.rs:876`, `:884-887`)
- `added area {code}`, `minted {key}`, `amended {key} to v{n}`, `withdrew {key}` (`requirements/mod.rs:751-788`)

With `NOTICE_LINES = 2` (`library.rs:51`, `skills/templates.rs:36`, `requirements/mod.rs:65`), a cut drops the error detail first.

---

## 2. T1: the Skills library's writes name themselves

### 2.1 Worker (`skills.rs`)

`Result` in this file is `htui_core::store::Result`, which takes one parameter (`:24`). So the outcome parameter is `core::result::Result<SkillWrite, StaleWhat>`, where `Err` means stale.
```rust
/// The re-read after a write (MOD-59 D1, D5): `SkillWritten` naming what `landed` wrote, or
/// `SkillsStale` naming the write that missed. A re-read that fails after an applied write still
/// answers `SkillWritten`; after a stale one it stays an error (D3). Its callers are past
/// `write_access`, so this box's paths are read here (MOD-9 D131).
async fn answer(backend: &Backend, writer: &Writer, scope: &Scope,
                landed: core::result::Result<SkillWrite, StaleWhat>) -> Result<StoreReply> {
    let fresh = reread(backend, writer, scope).await;
    match landed {
        Ok(outcome) => Ok(written(fresh, outcome)),
        Err(what) => Ok(StoreReply::SkillsStale { snapshot: Box::new(fresh?), what }),
    }
}
async fn reread(backend: &Backend, writer: &Writer, scope: &Scope) -> Result<SkillsSnapshot> {
    let box_paths = this_box_paths(backend).await?;
    snapshot(writer, scope, &box_paths).await
}
/// MOD-59 D5: the reply to a write that applied, whatever its re-read came to.
fn written(reread: Result<SkillsSnapshot>, outcome: SkillWrite) -> StoreReply {
    StoreReply::SkillWritten { snapshot: reread.map(Box::new).map_err(|err| err.to_string()), outcome }
}
```

Writer returns, checked against `htui-core/src/store/traits.rs`:

| Arm | Writer (traits.rs) | Mapping |
|---|---|---|
| CreateSkill `:245-266` | `create_skill -> Result<(Skill, SkillVersion)>` `:876` | `let (skill, _) = …?;` → `Ok(Created { skill: skill.id, name: skill.name })` |
| EditSkill `:267-283` | `update_skill -> Result<CasOutcome<Skill>>` `:884-889` | `Applied(row)` → `Ok(Edited { skill: row.id, name: row.name })`; Stale / NotFound → `Err(StaleWhat::Skill(*skill))` |
| SaveSkillVersion `:285-307` | `add_skill_version -> Result<CasOutcome<SkillVersion>>` `:899-904` | `Applied(row)` → `Ok(Versioned { skill: *skill, version: row.version })`; Stale / NotFound → `Err(StaleWhat::Version(*skill))` |
| SetSkillBinding `:308-328` | `set_skill_binding -> Result<CasOutcome<Option<SkillBinding>>>` `:915-920` | `Applied(Some(row))` → `Ok(Attached { key: *key, updated_at: row.updated_at })`; `Applied(None)` → `Ok(Detached { key: *key })`; Stale / NotFound → `Err(StaleWhat::Binding(*key))` |

`StoreRequest::Skills` (`:237-244`) and `ImportSkills` (`:327-340`) are unchanged.

Docs:
- Delete the "Known residue" paragraph (`:12-14`).
- Rewrite `serve`'s doc (`:218-224`): writes answer `SkillWritten` when they applied, `SkillsStale` when stale, and `Failed` when refused.
- Rewrite `answer`'s doc (`:374-376`).

### 2.2 View (`library.rs`)

**`on_reply` (`:551-601`).**
- In the `Skills` arm (`:553-561`), delete `self.land();` (`:559`). A read never lands.
- Add this arm:
```rust
StoreReply::SkillWritten { snapshot, outcome } => match snapshot {
    Ok(snapshot) => {
        if !in_scope(snapshot, ctx) { return; }
        self.snapshot = Some((**snapshot).clone());
        self.unavailable = None;
        self.land(outcome, None);
        self.clamp();
    }
    // No snapshot to check the scope by: only the write in flight owns this reply, and a scope
    // change has already forgotten that (`on_scope_change`).
    Err(why) if self.busy == Some(outcome.request_name()) => {
        if self.snapshot.is_none() { self.unavailable = Some(why.clone()); }
        self.land(outcome, Some(why));
    }
    Err(_) => {}
},
```
- The `SkillsStale` and `Failed` arms are unchanged (`:562-579`, `:589-598`).

**`land` (replaces `:1195-1286`).** The whole content predicate (`:1200-1250`) goes.
```rust
/// MOD-59 D4: the write in flight landed. `outcome` is the store's own answer, so nothing here
/// searches the snapshot for what was sent; a reply for any other write, and every plain `Skills`
/// read, leaves the draft, the token and `busy` alone. `unread` is the re-read's failure (D5).
fn land(&mut self, outcome: &SkillWrite, unread: Option<&str>) {
    if self.busy != Some(outcome.request_name()) { return; }
    self.busy = None;
    // `send` sets `busy` and `sent` together, so the write `busy` names has its `Sent`.
    let Some(sent) = self.sent.take() else { return; };
    let landed = match (outcome, sent) {
        (SkillWrite::Created { skill, name }, Sent::Create { body, .. }) =>
            self.landed_version(*skill, name, 1, &body),
        (SkillWrite::Versioned { skill, version }, Sent::Version { body, .. }) => {
            let name = snapshot_name(self.snapshot.as_ref(), *skill);
            self.landed_version(*skill, &name, *version, &body)
        }
        (SkillWrite::Edited { skill, name }, Sent::Rename { .. }) => {
            self.select(*skill);
            self.mode = Mode::Browse;
            format!("saved `{name}`")
        }
        (SkillWrite::Attached { key, updated_at }, Sent::Binding { change, target, .. }) => {
            let kept = self.attach.as_mut()
                .is_some_and(|pane| pane.on_landed(*key, &change, Some(*updated_at)));
            if kept { format!("attached to {target} \u{2014} later edits kept, Ctrl+S saves them") }
            else { format!("attached to {target}") }
        }
        (SkillWrite::Detached { key }, Sent::Binding { change, target, .. }) => {
            if let Some(pane) = &mut self.attach { pane.on_landed(*key, &change, None); }
            format!("detached from {target}")
        }
        // `busy` named this outcome's write, and `sent` is that write's.
        _ => return,
    };
    self.notice = Some(match unread {
        None => Notice::Info(landed),
        Some(why) => Notice::Error(landed_unread(&landed, why)),
    });
}
```

**`landed_version` (`:1330-1372`).**
- New signature: `fn landed_version(&mut self, skill: SkillId, name: &str, version: i32, body: &str) -> String`.
- Delete the `by_name` lookup and its early `return` (`:1336-1343`). A create whose re-read failed has no row in the held snapshot.
- Call `self.select(skill)` directly. `select` is already a no-op for an unknown id (`:1447-1457`).
- Return the notice text instead of setting `self.notice`. The three texts are unchanged (`:1351`, `:1356`, `:1366-1370`).
- The editor's "later edits kept" branch is unchanged. It compares `editor.area.text()` with the sent `body`, then sets `token = version`, `from = Some(version)` and `original = body`.

**`Sent` (`:363-402`) stays exactly as it is.** `Create.body` and `Version.body` feed the estimate and the "later edits kept" rule. `Binding.change` and `target` feed the notice and `on_landed`. The `SkillsStale` arm reads `Binding { change: Detach }` (`:569-575`). `Rename.token` and `patch` are now read only by the custom `Debug`, so there is no dead-code warning.

**Other edits:**
- Delete `use htui_core::model::skill_language;` (`:26`). Its only use was the predicate at `:1240`.
- Add `SkillWrite` to the import at `:42`.
- Rewrite the module doc (`:6-9`): "A write **lands on its own reply** (MOD-59): only a `SkillWritten` naming the write in flight closes the draft; a plain `Skills` read never does. A `SkillsStale` keeps the draft and moves the token to the row as it is now."

**`attach.rs` `on_landed` (`:278-304`, DV-1):**
```rust
pub(super) fn on_landed(&mut self, key: SkillBindingKey, change: &BindingChange,
                        token: Option<DateTime<Utc>>) -> bool {
    if let (AttachMode::Form(form) | AttachMode::Picker { form, .. }, BindingChange::Attach(sent), Some(token))
        = (&mut self.mode, change, token)
        && form.key == key
        && build(form).as_ref() != Ok(sent)
    {
        form.token = Some(token);
        return true;
    }
    self.mode = AttachMode::Browse;
    false
}
```
- Update the doc: the token is the write's own landed row, not the snapshot's.
- `DateTime` and `Utc` are already imported (`attach.rs:21`).
- The only caller is `LibraryView::land` (`library.rs:1274`).

**`store_worker.rs` request docs:** update to "answered with [`StoreReply::SkillWritten`] when it applied".
- CreateSkill `:615-617`
- EditSkill `:628-630`
- SaveSkillVersion `:641-643`
- SetSkillBinding `:654-656`

### 2.3 T1 tests

Shared library test helpers, added to `library.rs` `mod tests` (`:1769`):
- `use std::sync::Arc; use htui_core::clock::TestClock; use htui_core::store::WriteStore as _; use crate::skills::SkillWrite;`
- `fn raced(written: StoreReply, read: StoreReply) -> StoreReply` destructures `(SkillWritten { outcome, .. }, Skills(snapshot))` and returns `SkillWritten { snapshot: Ok(snapshot), outcome }`. It panics otherwise, so on the red commit it panics because the worker still answers `Skills`.

| # | Test (file) | Kind | Setup |
|---|---|---|---|
| W1 | `each_write_answers_skill_written_with_what_the_store_stored` (`skills.rs`, replaces `each_write_answers_skills_when_it_applies` `:706-780`) | red | `applied()` (`:433-439`) now matches `Ok(StoreReply::SkillWritten { snapshot: Ok(s), outcome })` and returns `(*s, outcome)`. Assert `Created { skill: entry.skill.id, name: "docs-style" }`, `Edited { skill: id, name: "docs-style" }`, `Versioned { skill: id, version: 2 }`, `Attached { key, updated_at: row.updated_at }`, `Detached { key }`. Keep all existing snapshot asserts. |
| W2 | `request_names_match_the_name_arms` (`skills.rs:940`) | extend | For create, edit, save and bind samples, `assert_eq!(outcome.request_name(), request.name())` over one `SkillWrite` per variant. |
| W3 | `a_reread_that_fails_after_an_applied_write_still_names_the_write` (`skills.rs`) | green commit | `written(Err(StoreError::Unreachable("gone".into())), SkillWrite::Versioned { skill: ids::SKILL_TESTS, version: 2 })` gives `SkillWritten { snapshot: Err("store unreachable: gone"), outcome: Versioned {..} }`. |
| – | `a_spent_token_answers_skills_stale_with_what_went_stale` (`:782`), `a_refused_write_answers_failed_with_the_store_sentence`, `the_snapshot_carries_…` (`read()` still matches `Skills`) | unchanged | These guard "stale is still `SkillsStale`", "a refusal is still `Failed`" and "`Skills` still answers `Skills`". |
| V-a | `a_rename_lands_although_another_session_renamed_it_before_the_reread` (`library.rs`) | red, HANDOFF (a) | `MemStore::demo()`, `vulkan()`. Read. Press `i`, `Tab`, type `" Mine."`, `Ctrl+S`, and take the `EditSkill`. Run `let written = serve(&backend, &edit)`. Take `ours = written`'s snapshot entry `updated_at`. Another session: `backend.writer().unwrap().update_skill(ids::SKILL_RUST_STYLE, ours, SkillPatch { name: None, description: Some("Theirs.".into()) })`. Then `let theirs = serve(Skills(scope))` and feed `raced(written, theirs)`. Assert `mode` is `Browse`, `busy` is `None`, and the notice is `Info("saved \`rust-style\`")`. On base, the description `"Theirs."` fails `library.rs:1222-1225`, so the form wedges. |
| V-a2 | `kept_attach_edits_save_over_the_writes_own_row_not_the_racing_one` (`library.rs`) | red, DV-1 | Follow `the_attach_form_keeps_edits_typed_during_its_save` (`:1968-2010`): press `a`, `j`, `Enter`, `Ctrl+S`, then `Space Space`, and serve to get `written`. Another session: `set_skill_binding(written_key, Some(ours.updated_at), Attach(always-with-position-1))`. Read, then feed `raced`. Assert it lands with "later edits kept". Press `Ctrl+S` again. The sent `SetSkillBinding.expected` must be `Some(ours.updated_at)`, the outcome's value, not the racing row's. Serve it: the answer is `SkillsStale`, so the conflict is surfaced rather than overwritten. |
| V-b | `a_rename_at_the_stores_frozen_instant_still_lands` (`library.rs`) | red, HANDOFF (b) | `let clock = TestClock::new(); let backend = Backend::memory(MemStore::demo().with_clock(Arc::new(clock.clone())));`, never advanced. Rename once (`i`, `Tab`, `"A"`, `Ctrl+S`), serve, feed. It lands because the fixture token is not `epoch()`. Rename again: `i` now opens with `token == epoch()`. Type `"B"`, `Ctrl+S`, serve. Precondition: assert the reply snapshot's `updated_at == token`. Then feed it and assert `Browse`, `busy` `None`, and `saved \`rust-style\``. On base, `updated_at != token` (`:1217`) is false, so the form wedges. |
| V-b2 | `an_attach_over_a_row_stamped_at_the_same_instant_still_lands` (`library.rs`) | red, optional | Same clock. Attach (`a`, `j`, `Enter`, `Ctrl+S`), serve, feed. Then `Enter` on the same row (token `Some(epoch)`), `Space`, `Ctrl+S`, serve, feed. Assert it lands. On base, `:1233` wedges. |
| V-c | `a_read_reply_does_not_close_the_attach_form_mid_save` (`:1884-1963`), `a_read_reply_does_not_close_the_editor_mid_save` (`:1823-1880`) | unchanged body | Rewrite the docs (`:1820-1822`, `:1882-1883`): "a plain `Skills` never lands; the write's own `SkillWritten` does". |
| V-c2 | `a_read_that_shows_the_saved_version_does_not_land_the_save` (`library.rs`) | red, D4 | Read. `e`, `x`, `Ctrl+S`. Then `let saved = serve(save)` and `let shows = serve(Skills(scope))`: v3 with our body is in the read. Feed `shows` first and assert `mode` is `Editing` and `busy` is `Some("save_skill_version")`. Then feed `saved` and assert it lands. On base, `shows` lands by content (`:1209-1211`). |
| V-d | `a_save_whose_reread_failed_lands_and_keeps_the_library_drawn` (`library.rs`) | red, D5 | Read, keeping `before = view.snapshot.clone()`. `e`, `x`, `Ctrl+S`. Feed `SkillWritten { snapshot: Err("store unreachable: gone".into()), outcome: Versioned { skill: ids::SKILL_RUST_STYLE, version: 3 } }`. Assert `Browse`, `busy` `None`, `sent` `None`, a notice `Error` that starts with `"saved v3 · ~"` and contains `"the re-read failed, r reloads: store unreachable: gone"`, `view.snapshot == before`, and `unavailable` `None`. |
| V-d2 | `a_create_whose_reread_failed_is_unavailable_only_when_nothing_was_held` (`library.rs`) | red, D5 | Build `LibraryView { busy: Some("create_skill"), sent: Some(Sent::Create { name: "docs-style".into(), body: "B.\n".into() }), mode: Mode::Editing(Editor::new(Target::New { name, description }, 0, None, "B.\n")), ..default }`. Feed `Err` with `Created { skill: SkillId::new(), name: "docs-style" }`. Assert `unavailable == Some("store unreachable: gone")`, `Browse`, and a notice that starts with `"created \`docs-style\` v1"`. |
| V-e | `a_skill_written_for_another_write_does_not_land` (`library.rs`) | guard | Rename in flight (`busy` is `edit_skill`). Feed `SkillWritten { Ok(untouched), Versioned { .. } }`: `busy` is unchanged and `mode` is `Info`. Then feed `SkillWritten { Err(..), Versioned { .. } }`: `unavailable` is `None` and the notice is still `Info("saving…")`. |
| – | `the_attach_form_keeps_edits_typed_during_its_save` (`:1968-2067`) | changed | `:2022-2028`: `let StoreReply::SkillWritten { outcome: SkillWrite::Attached { updated_at: token, .. }, .. } = &saved else { panic!(…) };`, then use `*token`. |

### 2.4 T1 validation
```bash
cargo test -p htui --all-features --lib -- skills
cargo test -p htui --all-features --test skills --test skills_pg -- --test-threads=1
```

---

## 3. T2: the Templates view lands on its save

### 3.1 Worker (`templates.rs`)

In the SaveTemplate arm (`:142-170`), delete the unconditional `let fresh = …?` (`~:165`) and the match (`:166-169`):
```rust
Ok(match outcome {
    CasOutcome::Applied(row) => saved(snapshot(backend, scope).await, &row),
    CasOutcome::Stale(_) => StoreReply::TemplatesStale(Box::new(snapshot(backend, scope).await?)),
})
```
```rust
/// MOD-59 D5: the reply to a save that applied, whatever its re-read came to. The project, name
/// and version are the stored row's (`append_prompt_template`, traits.rs:837-841).
fn saved(reread: Result<TemplatesSnapshot>, row: &PromptTemplate) -> StoreReply {
    StoreReply::TemplateSaved {
        snapshot: reread.map(Box::new).map_err(|err| err.to_string()),
        project: row.project_id, name: row.name.clone(), version: row.version,
    }
}
```
- Keep the comment at `:162-164`.
- Delete the residue paragraph in the module doc (`:8-10`).
- `store_worker.rs` SaveTemplate doc (`:597-598`): add "Answered with [`StoreReply::TemplateSaved`] when it applied, [`StoreReply::TemplatesStale`] when the token was spent."

### 3.2 View (`skills/templates.rs`)

**`on_reply` (`:349-394`).**
- In the `Templates` arm, delete `self.land_save();` (`:357`).
- Add this arm:
```rust
StoreReply::TemplateSaved { snapshot, project, name, version } => match snapshot {
    Ok(snapshot) => {
        if !in_scope(snapshot, ctx) { return; }
        self.snapshot = Some((**snapshot).clone());
        self.unavailable = None;
        self.land_save(*project, name, *version, None);
        self.clamp_cursor();
    }
    Err(why) if self.busy == Some(SAVE_NAME) => {
        if self.snapshot.is_none() { self.unavailable = Some(why.clone()); }
        self.land_save(*project, name, *version, Some(why));
    }
    Err(_) => {}
},
```

**`land_save` (replaces `:831-889`).** New signature: `fn land_save(&mut self, project: ProjectId, name: &str, version: i32, unread: Option<&str>)`.
1. `if self.busy != Some(SAVE_NAME) { return; }`, then `busy = None`.
2. Reset `shown`, `base`, `pane` and `scroll` as in `:864-867`.
3. Move the cursor to `Row::Template { project, name }` if `rows()` holds it. It may not, for a new name when the re-read failed.
4. If `Mode::Editing(editor)`:
   - `let sent = editor.sent.take().unwrap_or_default();`
   - If `editor.area.text() == sent`: `landed = format!("saved v{version}")` and `mode = Browse`.
   - Otherwise: `token = Some(version)`, `from = Some(version)`, `original = sent`, `confirm_item = false`, `esc_armed = false`, and `landed = "saved v{version} — later edits kept, Ctrl+S saves them as v{version+1}"`. The exact text is `:884-887`.
5. Otherwise `landed = format!("saved v{version}")`.
6. `notice = Info(landed)`, or `Error(landed_unread(&landed, why))`.

The content check (`:852-858`) and the `token.unwrap_or(0) + 1` arithmetic go. `Editor.sent` (`:220-222`) stays for the "later edits kept" rule. Rewrite its doc: "the body the save in flight carries: what tells a draft typed on since from the one that was saved."

### 3.3 T2 tests
| # | Test (file) | Kind | Setup |
|---|---|---|---|
| W1 | `a_save_at_the_head_answers_template_saved_with_the_stored_version` (`templates.rs`, replaces `:306-347`) | red | Match `Ok(StoreReply::TemplateSaved { snapshot: Ok(after), project, name, version })`. Assert `(project, name, version) == (PROJECT_HTUI, "implement", 2)` and keep the existing `after` asserts. |
| W2 | `a_save_at_a_stale_head_answers_templates_stale_and_writes_nothing` (`:349-393`) | changed | `:364`: `matches!(first, Ok(StoreReply::TemplateSaved { version: 2, .. }))`. |
| W3 | `a_new_name_saves_as_version_1` (`templates.rs`) | red | `save(&scope, PROJECT_HTUI, "implement-2", "Implement {{item}}\n", None)` (use a valid phase name) gives `TemplateSaved { version: 1, .. }`. |
| W4 | `a_reread_that_fails_after_a_save_still_answers_template_saved` | green commit | `saved(Err(StoreError::Unreachable("gone".into())), &row)`, where `row` is the demo `implement` v1. |
| V1 | `a_read_that_shows_the_saved_version_does_not_land_the_save` (`skills/templates.rs`) | red, D4 | Use the setup of `:1245-1302` (three `j`, `e`, `x`, `Ctrl+S`). Then `let saved = serve(save)` and `let shows = serve(Templates(scope))`. Feed `shows`: still `Editing`, `busy` `Some("save_template")`. Feed `saved`: `Browse` and `Info("saved v2")`. On base, `shows` lands (`:852-858`). |
| V2 | `a_save_lands_on_its_own_reply_whatever_body_the_reread_shows` | red, D4 guard | Serve the save. In the `Ok` snapshot, set the `implement` v2 row's body to `"Theirs.\n"`. Feed it: it lands with `saved v2`. |
| V3 | `a_save_whose_reread_failed_lands_and_keeps_the_tree_drawn` | red, D5 | Feed `TemplateSaved { snapshot: Err("store unreachable: gone"), project: PROJECT_VULKAN, name: "implement", version: 2 }`. Assert `Browse`, `busy` `None`, a notice `Error` that starts with `"saved v2 — the re-read failed"`, and the snapshot unchanged. |
| – | `a_read_reply_does_not_close_the_editor_mid_save` (`:1242-1327`) | doc only | Update the doc at `:1242-1244` and the comment at `:1317` ("the save's own answer, `TemplateSaved`"). The assertions pass unchanged. |
| – | `tests/templates.rs:458-499` `keys_typed_while_a_save_is_in_flight_are_kept` | must pass unedited | Pins "later edits kept" (`:485`). |

### 3.4 T2 validation
```bash
cargo test -p htui --all-features --lib -- templates
cargo test -p htui --all-features --test templates --test templates_pg -- --test-threads=1
```

---

## 4. T3: the Requirements tab lands on its write

### 4.1 Worker (`requirements.rs`)
- Read: `StoreRequest::Requirements(scope) => Ok(StoreReply::Requirements(Box::new(snapshot(backend, scope).await?)))` (`:391`).
- Area (`:429-439`): `let area = writer.create_requirement_area(…).await?; answer(backend, scope, Some(RequirementWrite::Area { id: area.id, code: area.code })).await`. Writer: traits.rs:1427.
- Mint (`:467-480`): `let row = writer.mint_requirement(…).await?;` then `Some(Minted { id: row.id, key: row.key })`. Writer: traits.rs:1437-1441.
- Amend (`:506-509`): `Updated(row)` → `Some(Amended { id: row.id, key: row.key, version: row.version })`, `Diverged { .. }` → `None`. `RequirementUpdate` is at `model/requirement.rs:281-291`, the writer at traits.rs:1452-1458.
- Withdraw (`:523-526`): `Updated(row)` → `Some(Withdrawn { id: row.id, key: row.key })`, `Diverged` → `None`. Writer: traits.rs:1466-1473.
```rust
/// The scope re-read after a tab write (MOD-59 D1, D5): `RequirementWritten` naming what `landed`
/// wrote, or `RequirementsStale` when an amend or withdraw missed its version (`None`). The worker
/// re-reads rather than handing the view the row the outcome carries: the view renders a tree, and
/// a row patched in locally would be a second source of truth. A re-read that fails after an
/// applied write still answers `RequirementWritten`; after a missed one it stays an error (D3).
async fn answer(backend: &Backend, scope: &Scope, landed: Option<RequirementWrite>) -> Result<StoreReply> {
    let fresh = snapshot(backend, scope).await;
    match landed {
        Some(outcome) => Ok(written(fresh, outcome)),
        None => Ok(StoreReply::RequirementsStale(Box::new(fresh?))),
    }
}
/// MOD-59 D5: the reply to a tab write that applied, whatever its re-read came to.
fn written(reread: Result<RequirementsSnapshot>, outcome: RequirementWrite) -> StoreReply {
    StoreReply::RequirementWritten { snapshot: reread.map(Box::new).map_err(|err| err.to_string()), outcome }
}
```
- Delete `is_diverged` (`:729-732`). Otherwise it is dead code, and `-D warnings` fails.
- Docs: `:5-9` (a tab write answers `RequirementWritten`), delete the residue paragraph `:30-32`, rewrite `:734-736`.
- `store_worker.rs` docs for the four writes (`:729`, `:740`, `:755-756`, `~:773`): add "answered with [`StoreReply::RequirementWritten`] when it applied".

### 4.2 View (`requirements/mod.rs`)

**Remove:**
- `CHECKING_MINT` (`:81-82`).
- The `sent` field (`:166-167`) and `verifying` (`:168-171`).
- `enum Sent` and its `Debug` impl (`:197-277`, DV-3).
- The `known` computation (`:620-625`).
- Every `Sent::…` construction (`:606`, `:635-640`, `:675-681`, `:726`).
- `self.sent = None` at `:794`, `:861`, `:1147`, `:1189`, `:1218`.
- `use` items that become unused: `Priority` and `RequirementAreaId` at `:32`, whose only uses were in `Sent` (`:212`, `:231`).

**Change:**
- `send` (`:733-738`) becomes `fn send(&mut self, request: StoreRequest, ctx: &Ctx<'_>)`.
- `save` builds only the request. Area: `self.send(StoreRequest::CreateRequirementArea { scope, project, code, title }, ctx)`.

**`land` (replaces `:740-799`):**
```rust
/// MOD-59 D4: the tab write in flight landed. `outcome` is the store's own answer, so nothing here
/// searches the snapshot: the form closes, the notice says what landed, and the cursor goes to the
/// row it landed on when the tree drawn holds it. `unread` is the re-read's failure (D5): the tree
/// is the one held before the write, which may not hold a new row yet.
fn land(&mut self, outcome: &RequirementWrite, unread: Option<&str>, ctx: &Ctx<'_>) {
    let (landed, row) = match outcome {
        RequirementWrite::Area { id, code } => (format!("added area {code}"), Row::Area(*id)),
        RequirementWrite::Minted { id, key } => (format!("minted {key}"), Row::Requirement(*id)),
        RequirementWrite::Amended { id, key, version } =>
            (format!("amended {key} to v{version}"), Row::Requirement(*id)),
        RequirementWrite::Withdrawn { id, key } => (format!("withdrew {key}"), Row::Requirement(*id)),
    };
    self.busy = None;
    self.mode = Mode::Browse;
    self.notice = Some(match unread {
        None => Notice::Info(landed),
        Some(why) => Notice::Error(landed_unread(&landed, why)),
    });
    let held = self.snapshot.as_ref().is_some_and(|snapshot| match row {
        Row::Area(id) => snapshot.area(id).is_some(),
        Row::Requirement(id) => snapshot.requirement(id).is_some(),
        Row::Project(id) => snapshot.project(id).is_some(),
    });
    if held { self.select_row(row, ctx); }
}
```

**`on_reply` (`:1131-1223`).**
- Extract the tail of the `Requirements` arm (`:1150-1158`, pending reveal plus `reselect`) into `fn after_read(&mut self, ctx: &Ctx<'_>)`.
- `Requirements` arm: `is_for` check, set the snapshot, `recovered()`, `after_read(ctx)`. `land` and `verifying` go (`:1139-1149`).
- New arm:
```rust
StoreReply::RequirementWritten { snapshot, outcome } => match snapshot {
    Ok(snapshot) => {
        if !snapshot.is_for(ctx.scope) { return; }
        self.snapshot = Some((**snapshot).clone());
        self.recovered();
        if self.busy == Some(outcome.request_name()) { self.land(outcome, None, ctx); }
        else { self.after_read(ctx); }
    }
    Err(why) if self.busy == Some(outcome.request_name()) => {
        if self.snapshot.is_none() { self.unavailable = Some(why.clone()); }
        self.land(outcome, Some(why), ctx);
    }
    Err(_) => {}
},
```
- `READ_NAME` `Failed` arm: delete the `verifying` branch (`:1186-1193`).
- Tab-write `Failed` arm (`:1205-1220`): keep the guard `is_tab_write(request) && self.busy == Some(*request)`. The body becomes `self.busy = None; self.notice = Some(Notice::Error(message.clone()));` with the comment "D5: a tab write's `Failed` is a refusal; nothing was written, so the form keeps its text."
- `stale()` (`:857-899`) keeps its logic minus `:861`.
- Rewrite the module doc (`:17-23`): "One write in flight (`busy`), and a write **lands on its own reply** (MOD-59): only a `RequirementWritten` naming the write in flight closes the form; a plain `Requirements` never does."

### 4.3 T3 tests
| # | Test (file) | Kind | Setup |
|---|---|---|---|
| W1 | `the_first_gated_write_claims_a_project_without_a_spec` (`requirements.rs:985-1028`) | changed, red | `:996` and `:1018` match `Ok(StoreReply::RequirementWritten { snapshot: Ok(after), outcome: RequirementWrite::Area { code, .. } })`. Assert `code == "API"` and `"UI"`. |
| W2 | `mint_answers_requirement_written_with_the_next_key` (renames `:1071-1103`) | red | Outcome `Minted { id: minted.id, key: "R-ENT-3" }`. |
| W3 | `amend_at_the_head_records_the_deciding_item` (`:1131-1164`, match `:1138`) | changed | Outcome `Amended { id: REQ_ENT_1, key: "R-ENT-1", version: 3 }`. |
| W4 | `withdraw_marks_the_requirement_withdrawn_and_cites_the_deciding_item` (`:1215-1242`, `:1222`) | changed | Outcome `Withdrawn { id: REQ_STO_1, key: "R-STO-1" }`. `:1239` (stale) is unchanged. |
| W5 | `the_deciding_item_is_found_outside_the_request_scope` (`:1617-1639`, `:1627`) | changed | `RequirementWritten { snapshot: Ok(after), .. }`. |
| W6 | `request_names_match_the_name_arms` (`:1669-1711`) | extend | One `RequirementWrite` per variant; `request_name() == REQUEST_NAMES[3..7]` in order. |
| W7 | `a_reread_that_fails_after_a_tab_write_still_answers_requirement_written` | green commit | Pure `written(Err(..), Minted { .. })`. |
| – | `amend_at_a_stale_version_answers_requirements_stale_and_writes_nothing` (`:1167`), `read()` (`:808-813`), every refusal test | unchanged | These guard "diverged is still stale" and "refused is still `Err`". |
| V1 | `an_amend_lands_on_its_own_reply_although_another_session_amended_again` (`requirements/mod.rs`) | red, HANDOFF (a) | Amend as `:1560-1568`. Build `theirs = with_ent_1(snapshot, 4, Active)` with body `"Theirs."`. Feed `RequirementWritten { Ok(theirs), Amended { id: REQ_ENT_1, key: "R-ENT-1", version: 3 } }`. Assert `busy` `None`, `Browse`, `Info("amended R-ENT-1 to v3")`, and a `RequirementDetail(REQ_ENT_1)` request. |
| V2 | `a_read_that_shows_the_mint_does_not_land_it` | red, D4 | Use `mint(&bench, &mut tab, "Twice is too many.")` (`:1749`). Build `landed` as `:1784-1801` did, keeping `row.id`. Feed `Requirements(landed)`: `busy` `Some(name)`, `mode` `Requirement`. Then feed `RequirementWritten { Ok(landed), Minted { id, key: "R-ENT-3" } }`: it lands with `Info("minted R-ENT-3")`. On base, the plain read lands by content. |
| V3 | `a_mint_whose_reread_failed_lands_and_reads_nothing_more` (replaces `:1761-1811`) | red, D5 | Mint. Feed `RequirementWritten { Err("store unreachable: gone"), Minted { id: RequirementId::new(), key: "R-ENT-3" } }`. Assert `busy` `None`, `Browse`, a notice `Error` that starts with `"minted R-ENT-3 — the re-read failed"`, `requests(&bench.emit.take())` empty (no `Requirements` re-read and no detail read, since the row is not held), and the snapshot unchanged. |
| V4 | `a_refused_mint_frees_the_form_without_a_read` (replaces `:1813-1838`) | red | Mint. Feed `Failed { request: name, message: "refused" }`. Assert `busy` `None`, `Error("refused")`, `mode` `Requirement`, and no request emitted. On base, `CHECKING_MINT` sends a `Requirements` request. |
| – | `another_sessions_amend_does_not_land_this_one` (`:1552-1602`) | doc only | Rewrite the doc at `:1552-1554` to D4 wording. The body passes unchanged. |
| – | `only_the_write_in_flight_is_freed_by_its_refusal` (`:1703-1746`) | unchanged | |
| P1 | `tests/requirements_pg.rs` `applied()` (`:72-78`) | changed | `StoreReply::RequirementWritten { snapshot: Ok(snapshot), .. } => *snapshot`. Leave `:240` alone (a plain read). |

### 4.4 T3 validation
```bash
cargo test -p htui --all-features --lib -- requirements
cargo test -p htui --all-features --test requirements --test requirements_pg -- --test-threads=1
```
`requirements_pg` runs against `HTUI_TEST_DATABASE_URL`.

---

## 5. Hazards
- **H-1: the Skills tab sends every reply to both views** (`ui/tabs/skills/mod.rs:125-128`). Each view must ignore the other's variant. Both already end in `_ => {}` (`library.rs:599`, `skills/templates.rs:392-393`), so no arm may be written as a catch-all over `SkillWritten`/`TemplateSaved`.
- **H-2: the worker loop's offline switch.** `go_offline` runs only on `try_serve`'s `Err` (`store_worker.rs:2036-2046`). Under D5, an `Unreachable` re-read after an applied write becomes `Ok(*Written { Err })`, so the Online→Offline swap waits for the refresher's health pass (`lost_the_server`, `:2315-2333`) or the next failing request. Behaviour stays correct; only the switch is delayed. See DV-4.
- **H-3: the status line.** `App::on_reply` puts only `Failed` on the status line (`app/update.rs:251-253`). A D5 re-read failure now shows only in the view's Error-styled notice. This is the same as `AgentWritten` today.
- **H-4: the staleness key is `(Origin, Discriminant<StoreRequest>)`** (`app/state.rs:164`, `:329-333`). The reply's variant does not matter to the gate: a newer write of the same kind drops an older reply, and a read never supersedes a write. No change is needed.
- **H-5: an `Err` snapshot cannot be scope-checked.** The views gate it on `busy` (see D4 for the reset lines). An `Err` reply for a write not in flight must set neither `unavailable` nor the notice (tested by V-e).
- **H-6: `-D warnings` gate** (`README.md:446`). Workspace lints: `clippy::all` warn, `missing_debug_implementations` warn, `unused_qualifications` warn (`Cargo.toml` `[workspace.lints]`), and `#![warn(missing_docs)]` (`lib.rs:10`). What would trip them:
  - `library.rs:26` `skill_language` becomes unused.
  - `requirements.rs:729-732` `is_diverged` becomes dead code.
  - `requirements/mod.rs:32` `Priority` and `RequirementAreaId` become unused.
  - `CHECKING_MINT` becomes dead.
  - Every new variant field needs `///`.
- **H-7: `Result` aliases.** `skills.rs:24`, `requirements.rs:44-47` and `templates.rs:17` import `htui_core::store::Result` (one type parameter). Write `Result<Box<_>, String>` only in `store_worker.rs`, and use `core::result::Result<SkillWrite, StaleWhat>` in `skills::answer`.
- **H-8: the create landing.** `landed_version`'s `by_name` early return (`library.rs:1336-1343`) must go, or a create whose re-read failed never notices anything.
- **H-9: the attach form's kept-edit token** comes from the snapshot (`attach.rs:285-304`). This is fixed by DV-1.
- **H-10: pending reveals** are decided only by a `Requirements` reply (`requirements/mod.rs:1150-1157`). A `RequirementWritten` that does not land must run the same tail (`after_read`), or an armed reveal waits for a later read.
- **H-11: every applied path must answer its variant**, or the form wedges exactly as before, now with no content fallback. The detach path is `Applied(None)` (`mem.rs:3176-3181`). W1 in T1 asserts all five outcomes, and W1 to W5 in T3 assert all four.
- **H-12: hand-built replies in existing tests** that change: `library.rs:2022`, `requirements/mod.rs:1770-1802` and `:1821-1830` (replaced), `tests/requirements_pg.rs:75`. No other `tests/*.rs` file matches `StoreReply::Skills`, `Templates` or `Requirements`; they drive `App` through `store_worker::serve`.
- **H-13: `TestClock` needs `htui-core/test-support`.** It is on in htui's test builds through the dev-dependency `htui-orch` with `test-support` (`crates/htui/Cargo.toml` `[dev-dependencies]`), whose feature forwards to `htui-core/test-support` (`crates/htui-orch/Cargo.toml` `[features]`). `--all-features` also covers it. Fallback: a local fixed `Clock`, as `tests/backlog.rs:552` does.
- **H-14: red commits must compile.** Precedent is `53e5a17` (MOD-23 T2: types added red, bodies later). Red = variant, outcome enum, `request_name`, and tests; the worker still answers the old variant and the views ignore the new one, so tests fail at runtime. The `written`/`saved` helpers and their tests land green, which avoids a `dead_code` warning on red.
- **H-15: a refused library write frees any skills write.** The library's `Failed` arm frees `busy` for any skills write name (`library.rs:592-598`), unlike the Requirements guard `busy == Some(*request)` (`requirements/mod.rs:1205-1206`). This predates MOD-59 and is out of scope; note it and leave it.
- **H-16: the plan's per-task `Validate` commands** (`cargo test -p htui --all-features skills`) filter by test name, and integration-test function names do not contain the binary name. They would silently skip most of `tests/skills.rs` and the others. Use the `--lib` plus `--test` pairs in sections 2.4, 3.4 and 4.4, then the full gate.

---

## 6. Deviations from the plan
- **DV-1 (applied): `SkillWrite::Attached` carries `updated_at`; D2 says `Attached { key }`.**
  - The kept-edits rule sets `form.token = snapshot.binding(key).updated_at` (`attach.rs:285-304`). In HANDOFF scenario (a), that row belongs to the other session, so the next `Ctrl+S` overwrites their row without the stale check. On the D5 path there is no fresh row at all.
  - `set_skill_binding` already returns the applied row (`traits.rs:915-920`; `mem.rs:3219-3229` stamps `updated_at: now`).
  - The field is the compare-and-set token, in the spirit of D2's "versions". Tested by V-a2.
- **DV-2 (applied): HANDOFF scenario (a) moves from a version save to a rename for T1, and T2 has no real-store race test.**
  - `skill_version` and `prompt_template` are append-only. Another session's later append leaves `version(token+1)` holding our body, and a save at the same token makes ours `Stale`. So the predicates at `library.rs:1209-1211` and `skills/templates.rs:852-855` cannot be defeated by a race.
  - A version-save race test would pass on `08e0880`, which fails acceptance ("fail on base").
  - The rename (`:1212-1226`), attach (`:1227-1241`), detach (`:1242-1246`) and create (`by_name`, `:1204-1208`) predicates can be defeated.
  - For Templates, the tests that fail on base are D4's "a read that shows the version lands it" (V1) and D5 (V3). V2 is a hand-built guard that the view does not consult content.
- **DV-3 (applied): `Sent` in `requirements/mod.rs` is removed whole; the plan removes only `Sent::Mint.known`.**
  - After D2/D4, `land` reads only the outcome, `stale()` reads the mode (`:862-899`), and the `Failed` arm no longer inspects `Sent::Mint`.
  - The form swallows keys and pastes while busy (`:558-561`, `:1123`), so no "later edits kept" body is needed.
  - `Sent` would be read only by its own `Debug`. No test references it (the test imports at `:1287` do not include it).
  - The library's `Sent` stays unchanged, since its bodies feed the estimate and "later edits kept".
- **DV-4 (offered, not applied): `snapshot: Result<Box<_>, StoreError>` instead of `String`.**
  - It would let the worker loop's `Ok` arm call `go_offline` on an `Unreachable` re-read (H-2).
  - `StoreError` is `Clone + Eq` (`htui-core/src/store/error.rs:10-51`).
  - The cost is a loop change (`store_worker.rs:2037`) with no cheap test, against D5's explicit "rendered through `Display`". The blueprint keeps D5 and records the refresher-delayed offline switch as H-2.

---

## 7. Build order and commit plan

The tasks run in series on one tree (D7). Each task is a red commit, then a green commit. Every commit ends with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. The style follows `53e5a17` / `553414a`.

**T1**
1. **Red:** `test(skills): a skill write's reply names itself, red (MOD-59 T1)`
   - Files: `store_worker.rs` (the `SkillWritten` variant, its doc and import), `skills.rs` (`SkillWrite` and `request_name`; W1, W2), `library.rs` (V-a, V-a2, V-b, V-b2, V-c2, V-d, V-d2, V-e; the change at `:2022`).
   - Body: which tests fail and why. The worker answers `Skills`, and the view ignores `SkillWritten`.
2. **Green:** `feat(skills): the library lands a write on its own reply, green (MOD-59 T1)`
   - Files: `skills.rs` (`answer`, `reread`, `written`, serve arms, docs; W3), `library.rs` (`on_reply`, `land`, `landed_version`, `landed_unread`, doc, import), `attach.rs` (`on_landed`), `store_worker.rs` (request and `Skills` docs).
   - Gate: section 2.4.

**T2**
3. **Red:** `test(templates): the save's reply names itself, red (MOD-59 T2)`
   - Files: `store_worker.rs` (`TemplateSaved`), `templates.rs` (W1, W2, W3), `skills/templates.rs` (V1, V2, V3).
4. **Green:** `feat(templates): the Templates view lands on TemplateSaved, green (MOD-59 T2)`
   - Files: `templates.rs` (serve arm, `saved`, doc; W4), `skills/templates.rs` (`on_reply`, `land_save`, `landed_unread`, docs), `store_worker.rs` docs.
   - Gate: section 3.4.

**T3**
5. **Red:** `test(requirements): a tab write's reply names itself, red (MOD-59 T3)`
   - Files: `store_worker.rs` (`RequirementWritten`, import), `requirements.rs` (`RequirementWrite`; W1 to W6), `requirements/mod.rs` (V1 to V4, the two old mint tests replaced), `tests/requirements_pg.rs` (`applied()`).
6. **Green:** `feat(requirements): the tab lands on RequirementWritten, no mint re-check, green (MOD-59 T3)`
   - Files: `requirements.rs` (serve arms, `answer`, `written`, delete `is_diverged`, docs; W7), `requirements/mod.rs` (remove `CHECKING_MINT`, `verifying` and `Sent`; `land`, `after_read`, `on_reply`, `Failed` arm, docs), `store_worker.rs` docs.
   - Gate: section 4.4.

**Final gate, on the real tree after T3** (per memory: `--test-threads=1`, `--all-features`):
```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-features --all-targets -- -D warnings
cargo test --workspace --all-features -- --test-threads=1
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```

Relevant paths:
- `/home/mluigi/projects/htui/.claude/plans/mod-59-self-naming-write-replies.plan.md`
- `/home/mluigi/projects/htui/crates/htui/src/store_worker.rs`
- `/home/mluigi/projects/htui/crates/htui/src/skills.rs`
- `/home/mluigi/projects/htui/crates/htui/src/templates.rs`
- `/home/mluigi/projects/htui/crates/htui/src/requirements.rs`
- `/home/mluigi/projects/htui/crates/htui/src/ui/tabs/skills/library.rs`
- `/home/mluigi/projects/htui/crates/htui/src/ui/tabs/skills/attach.rs`
- `/home/mluigi/projects/htui/crates/htui/src/ui/tabs/skills/templates.rs`
- `/home/mluigi/projects/htui/crates/htui/src/ui/tabs/requirements/mod.rs`
- `/home/mluigi/projects/htui/crates/htui/tests/requirements_pg.rs`
- `/home/mluigi/projects/htui/crates/htui/tests/templates.rs`
- `/home/mluigi/projects/htui/crates/htui-core/src/store/mem.rs`
- `/home/mluigi/projects/htui/crates/htui-core/src/clock.rs`
- `/home/mluigi/projects/htui/crates/htui/src/app/state.rs`
- `/home/mluigi/projects/htui/crates/htui/src/app/update.rs`
