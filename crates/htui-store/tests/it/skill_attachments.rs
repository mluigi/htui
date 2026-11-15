//! MOD-9 milestone 2 (plan D38, D41; ANA-22 §6 items 2-4, §7.1): skill attachments at three
//! levels, on a real server.
//!
//! `0007_skill_attachments.sql` lets `skill_binding.project_id` be NULL (a global attachment) and
//! adds the activation. What `MemStore` cannot show is that Postgres agrees with it: the same
//! attachments planted in both answer the same candidates, a global row survives the project
//! delete that cascades the project's own, and the new checks refuse the rows ANA-22 rules out,
//! each under its own constraint name. `0041_toolbox_binding.sql` (MOD-127 M1) renames the table
//! to `toolbox_binding` and adds the workspace and item-kind levels; the cases here plant and read
//! all five.
//!
//! Every insert is `sqlx::query`, unchecked, so the file adds nothing to `.sqlx` (the
//! `prompt_template_cas.rs` rule). With `HTUI_TEST_DATABASE_URL` unset each case prints
//! `common::SKIP` and passes (plan D13).
#![cfg(feature = "demo")]

use htui_store::testkit as common;

use chrono::Utc;
use htui_core::fixtures::{DemoData, demo_data, ids};
use htui_core::model::skill::{StepFiles, select};
use htui_core::model::{
    Activation, Binding, BindingLevel, ChoiceReason, ItemKindId, PhaseId, ProjectId, ResolveScope,
    Skill, SkillBindingId, SkillId, SkillVersion, WorkspaceId,
};
use htui_core::store::{MemStore, WriteStore as _};
use htui_store::MIGRATOR;
use sqlx::postgres::PgPool;

/// One attachment to plant: the skill's name, its two keys, the activation, globs, position.
type Attachment = (
    &'static str,
    Option<ProjectId>,
    Option<PhaseId>,
    &'static str,
    &'static [&'static str],
    i32,
);

/// Plants skill `name` with one version, body `"{name} rules."`, created by the fixture user.
async fn plant_skill(pool: &PgPool, name: &str) -> SkillId {
    let id = SkillId::new();
    sqlx::query("INSERT INTO skill (id, name, created_by) VALUES ($1, $2, $3)")
        .bind(id.as_uuid())
        .bind(name)
        .bind(ids::USER.as_uuid())
        .execute(pool)
        .await
        .expect("insert a skill");
    sqlx::query(
        "INSERT INTO skill_version (skill_id, version, body, created_by) VALUES ($1, 1, $2, $3)",
    )
    .bind(id.as_uuid())
    .bind(format!("{name} rules."))
    .bind(ids::USER.as_uuid())
    .execute(pool)
    .await
    .expect("insert its v1");
    id
}

/// Inserts one `toolbox_binding` row of kind `skill`, unpinned, at any of the five levels, and
/// hands back the driver's answer so a refusal can be inspected.
#[allow(clippy::too_many_arguments)]
async fn plant_row(
    pool: &PgPool,
    skill: SkillId,
    workspace: Option<WorkspaceId>,
    project: Option<ProjectId>,
    item_kind: Option<ItemKindId>,
    phase: Option<PhaseId>,
    activation: &str,
    globs: &[&str],
    position: i32,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO toolbox_binding \
         (kind, ref_id, workspace_id, project_id, item_kind_id, phase_id, activation, globs, \
          position) \
         VALUES ('skill', $1, $2, $3, $4, $5, $6, $7, $8)",
    )
    .bind(skill.as_uuid())
    .bind(workspace.map(WorkspaceId::as_uuid))
    .bind(project.map(ProjectId::as_uuid))
    .bind(item_kind.map(ItemKindId::as_uuid))
    .bind(phase.map(PhaseId::as_uuid))
    .bind(activation)
    .bind(
        globs
            .iter()
            .map(|glob| (*glob).to_owned())
            .collect::<Vec<String>>(),
    )
    .bind(position)
    .execute(pool)
    .await
    .map(|_| ())
}

/// Inserts one project-or-global `toolbox_binding` row: [`plant_row`] with no workspace and no
/// item kind.
async fn plant_binding(
    pool: &PgPool,
    skill: SkillId,
    project: Option<ProjectId>,
    phase: Option<PhaseId>,
    activation: &str,
    globs: &[&str],
    position: i32,
) -> Result<(), sqlx::Error> {
    plant_row(
        pool, skill, None, project, None, phase, activation, globs, position,
    )
    .await
}

/// The constraint a refused insert names, for the assertion message and the comparison.
fn constraint(result: Result<(), sqlx::Error>) -> String {
    let err = result.expect_err("the insert must be refused");
    err.as_database_error()
        .and_then(|db| db.constraint())
        .unwrap_or_else(|| panic!("a constraint violation, got {err}"))
        .to_owned()
}

/// The demo fixture's skill id by name.
fn demo_skill(data: &DemoData, name: &str) -> SkillId {
    data.skills
        .iter()
        .find(|skill| skill.name == name)
        .unwrap_or_else(|| panic!("the fixture has skill `{name}`"))
        .id
}

/// `MemStore::from_demo` over `demo_data()` edited as `plant` edits Postgres: `house` added (with
/// the id Postgres minted) and `attachments` bound.
fn mem_with(house: Option<SkillId>, attachments: &[Attachment]) -> MemStore {
    let mut data = demo_data();
    let now = Utc::now();
    if let Some(house) = house {
        data.skills.push(Skill {
            id: house,
            name: "house".to_owned(),
            description: String::new(),
            created_by: ids::USER,
            created_at: now,
            updated_at: now,
        });
        data.skill_versions.push(SkillVersion {
            skill_id: house,
            version: 1,
            body: "house rules.".to_owned(),
            source: serde_json::json!({}),
            created_by: ids::USER,
            created_at: now,
        });
    }
    for (name, project_id, phase_id, activation, globs, position) in attachments {
        let skill_id = if *name == "house" {
            house.expect("house is planted")
        } else {
            demo_skill(&data, name)
        };
        data.skill_bindings.push(Binding {
            id: SkillBindingId::new(),
            skill_id,
            workspace_id: None,
            project_id: *project_id,
            item_kind_id: None,
            phase_id: *phase_id,
            pinned_version: None,
            position: *position,
            activation: activation.parse().expect("a valid activation"),
            globs: globs.iter().map(|glob| (*glob).to_owned()).collect(),
            languages: Vec::new(),
            updated_at: now,
        });
    }
    MemStore::from_demo(data)
}

/// Plants `attachments` in Postgres, `house` first when any attachment names it.
async fn plant(pool: &PgPool, attachments: &[Attachment]) -> Option<SkillId> {
    let data = demo_data();
    let house = if attachments.iter().any(|(name, ..)| *name == "house") {
        Some(plant_skill(pool, "house").await)
    } else {
        None
    };
    for (name, project, phase, activation, globs, position) in attachments {
        let skill = if *name == "house" {
            house.expect("house is planted")
        } else {
            demo_skill(&data, name)
        };
        plant_binding(pool, skill, *project, *phase, activation, globs, *position)
            .await
            .unwrap_or_else(|err| panic!("plant {name}: {err}"));
    }
    house
}

/// ANA-22 §6 items 2-3 on both stores: the same attachments answer the same candidates, bodies,
/// levels and activations included, for a phase step, a project step, and another project.
#[tokio::test(flavor = "multi_thread")]
async fn pg_and_mem_agree_on_global_project_and_phase() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    let attachments: [Attachment; 4] = [
        ("house", None, None, "always", &[], 5),
        ("tests", None, None, "off", &[], 3),
        ("rust-style", None, None, "always", &[], 9),
        (
            "tests",
            Some(ids::PROJECT_HTUI),
            Some(ids::PHASE_HTUI_IMPLEMENT),
            "off",
            &[],
            0,
        ),
    ];
    let house = plant(&db.pool, &attachments).await;
    let mem = mem_with(house, &attachments);

    for (project, phase) in [
        (ids::PROJECT_HTUI, None),
        (ids::PROJECT_HTUI, Some(ids::PHASE_HTUI_IMPLEMENT)),
        (ids::PROJECT_AGY, None),
    ] {
        assert_eq!(
            db.store
                .bound_skills(ResolveScope::project(project, phase))
                .await
                .expect("PgStore::bound_skills"),
            mem.bound_skills(ResolveScope::project(project, phase))
                .await
                .expect("MemStore::bound_skills"),
            "both stores resolve ({project:?}, {phase:?}) with model::skill::resolve"
        );
    }

    let implement = db
        .store
        .bound_skills(ResolveScope::project(
            ids::PROJECT_HTUI,
            Some(ids::PHASE_HTUI_IMPLEMENT),
        ))
        .await
        .expect("PgStore::bound_skills");
    assert_eq!(
        implement
            .iter()
            .map(|s| (
                s.name.as_str(),
                s.level,
                s.activation,
                s.version,
                s.position
            ))
            .collect::<Vec<_>>(),
        vec![
            ("tests", BindingLevel::Phase, Activation::Off, Some(1), 0),
            (
                "rust-style",
                BindingLevel::Phase,
                Activation::Always,
                Some(1),
                2
            ),
            (
                "house",
                BindingLevel::Global,
                Activation::Always,
                Some(1),
                5
            ),
        ],
        "the phase's off beats the project's and the global tests; the phase pin holds \
         rust-style at v1; house reaches the project from the global level"
    );

    let agy = db
        .store
        .bound_skills(ResolveScope::project(ids::PROJECT_AGY, None))
        .await
        .expect("PgStore::bound_skills");
    assert_eq!(
        agy.iter()
            .map(|s| (s.name.as_str(), s.level, s.activation, s.position))
            .collect::<Vec<_>>(),
        vec![
            ("tests", BindingLevel::Global, Activation::Off, 3),
            ("house", BindingLevel::Global, Activation::Always, 5),
            ("rust-style", BindingLevel::Global, Activation::Always, 9),
        ],
        "a project with no attachment of its own sees the three global ones, off included"
    );

    db.drop_db().await;
}

/// ANA-22 §6 item 4: an `off` phase attachment is the winner, so it hides the project's `tests`
/// from the phase step, and `select` records why.
#[tokio::test(flavor = "multi_thread")]
async fn an_off_phase_attachment_hides_a_project_skill() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    plant(
        &db.pool,
        &[(
            "tests",
            Some(ids::PROJECT_HTUI),
            Some(ids::PHASE_HTUI_IMPLEMENT),
            "off",
            &[],
            0,
        )],
    )
    .await;

    let candidates = db
        .store
        .bound_skills(ResolveScope::project(
            ids::PROJECT_HTUI,
            Some(ids::PHASE_HTUI_IMPLEMENT),
        ))
        .await
        .expect("PgStore::bound_skills");
    let tests = candidates
        .iter()
        .find(|s| s.name == "tests")
        .expect("tests is still a candidate");
    assert_eq!(
        (tests.level, tests.activation),
        (BindingLevel::Phase, Activation::Off),
        "the phase row wins over the project row"
    );

    let (active, choices) = select(candidates, true, &StepFiles::default());
    assert_eq!(
        active.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
        vec!["rust-style"],
        "only rust-style renders"
    );
    assert_eq!(
        choices
            .iter()
            .map(|c| (c.name.as_str(), c.active, c.reason))
            .collect::<Vec<_>>(),
        vec![
            ("tests", false, ChoiceReason::Off),
            ("rust-style", true, ChoiceReason::Always),
        ],
        "tests is recorded off, not dropped"
    );

    db.drop_db().await;
}

/// ANA-22 §7.1: `project_id`'s `ON DELETE CASCADE` never fires for a NULL key, so the delete
/// takes the project's three attachments and leaves the global one to every other project.
#[tokio::test(flavor = "multi_thread")]
async fn a_global_row_survives_project_delete() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    let house = plant(&db.pool, &[("house", None, None, "always", &[], 5)])
        .await
        .expect("house is planted");

    let reach = db
        .store
        .delete_project(ids::PROJECT_HTUI)
        .await
        .expect("the delete lands");
    assert_eq!(
        reach.skill_bindings, 3,
        "the reach counts the project's own attachments, not the global one"
    );
    let global: i64 =
        sqlx::query_scalar("SELECT count(*) FROM toolbox_binding WHERE project_id IS NULL")
            .fetch_one(&db.pool)
            .await
            .expect("count the global rows");
    assert_eq!(global, 1, "the global row survives");
    assert_eq!(
        db.store
            .bound_skills(ResolveScope::project(ids::PROJECT_AGY, None))
            .await
            .expect("PgStore::bound_skills")
            .iter()
            .map(|s| s.skill_id)
            .collect::<Vec<_>>(),
        vec![house],
        "and still reaches the remaining project"
    );

    db.drop_db().await;
}

/// ANA-22 §7.1's checks, each refusing under its own name (probed on Postgres 16, 2026-09-26),
/// and the column defaults the demo's pre-`0007` rows read back.
#[tokio::test(flavor = "multi_thread")]
async fn the_checks_refuse_a_phase_row_without_a_project_and_glob_without_globs() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    let skill = plant_skill(&db.pool, "house").await;

    assert_eq!(
        constraint(
            plant_binding(
                &db.pool,
                skill,
                None,
                Some(ids::PHASE_HTUI_IMPLEMENT),
                "always",
                &[],
                0
            )
            .await
        ),
        "toolbox_binding_phase_needs_project",
        "a phase attachment needs its project"
    );
    assert_eq!(
        constraint(
            plant_binding(
                &db.pool,
                skill,
                Some(ids::PROJECT_HTUI),
                None,
                "glob",
                &[],
                0
            )
            .await
        ),
        "toolbox_binding_glob_needs_globs",
        "a glob attachment needs at least one glob"
    );
    assert_eq!(
        constraint(plant_binding(&db.pool, skill, None, None, "sometimes", &[], 0).await),
        "toolbox_binding_activation_check",
        "activation is always, glob or off"
    );
    plant_binding(&db.pool, skill, None, None, "always", &[], 0)
        .await
        .expect("one global attachment of a skill is accepted");
    assert_eq!(
        constraint(plant_binding(&db.pool, skill, None, None, "off", &[], 1).await),
        "toolbox_binding_key",
        "UNIQUE NULLS NOT DISTINCT allows one global attachment per skill"
    );
    plant_binding(
        &db.pool,
        skill,
        Some(ids::PROJECT_HTUI),
        None,
        "glob",
        &["**/*.rs"],
        0,
    )
    .await
    .expect("a glob attachment with a glob is accepted");

    let demo: Vec<(String, Vec<String>, Vec<String>)> = sqlx::query_as(
        "SELECT activation, globs, languages FROM toolbox_binding WHERE id = ANY($1) ORDER BY id",
    )
    .bind(
        demo_data()
            .skill_bindings
            .iter()
            .map(|row| row.id.as_uuid())
            .collect::<Vec<_>>(),
    )
    .fetch_all(&db.pool)
    .await
    .expect("read the demo attachments");
    assert_eq!(
        demo,
        vec![("always".to_owned(), Vec::new(), Vec::new()); 3],
        "the demo loader writes none of the new columns, so its three rows read the defaults"
    );

    db.drop_db().await;
}

/// One attachment at any level to plant: the skill's name, its four keys, the activation, the
/// position.
type Scoped = (
    &'static str,
    Option<WorkspaceId>,
    Option<ProjectId>,
    Option<ItemKindId>,
    Option<PhaseId>,
    &'static str,
    i32,
);

/// Plants `rows` in Postgres and builds the `MemStore` with the same ones, `house` (the skill both
/// need that the demo lacks) planted first.
async fn plant_scoped(db: &common::TestDb, rows: &[Scoped]) -> MemStore {
    let data = demo_data();
    let house = plant_skill(&db.pool, "house").await;
    let mut mem = demo_data();
    let now = Utc::now();
    mem.skills.push(Skill {
        id: house,
        name: "house".to_owned(),
        description: String::new(),
        created_by: ids::USER,
        created_at: now,
        updated_at: now,
    });
    mem.skill_versions.push(SkillVersion {
        skill_id: house,
        version: 1,
        body: "house rules.".to_owned(),
        source: serde_json::json!({}),
        created_by: ids::USER,
        created_at: now,
    });
    for (name, workspace, project, item_kind, phase, activation, position) in rows {
        let skill = if *name == "house" {
            house
        } else {
            demo_skill(&data, name)
        };
        plant_row(
            &db.pool,
            skill,
            *workspace,
            *project,
            *item_kind,
            *phase,
            activation,
            &[],
            *position,
        )
        .await
        .unwrap_or_else(|err| panic!("plant {name}: {err}"));
        mem.skill_bindings.push(Binding {
            id: SkillBindingId::new(),
            skill_id: skill,
            workspace_id: *workspace,
            project_id: *project,
            item_kind_id: *item_kind,
            phase_id: *phase,
            pinned_version: None,
            position: *position,
            activation: activation.parse().expect("a valid activation"),
            globs: Vec::new(),
            languages: Vec::new(),
            updated_at: now,
        });
    }
    MemStore::from_demo(mem)
}

/// What `bound_skills` answers for `scope` on Postgres and on `mem`, which must be one answer.
async fn both(
    db: &common::TestDb,
    mem: &MemStore,
    scope: ResolveScope,
) -> Vec<htui_core::model::BoundSkill> {
    let pg = db
        .store
        .bound_skills(scope)
        .await
        .expect("PgStore::bound_skills");
    assert_eq!(
        pg,
        mem.bound_skills(scope)
            .await
            .expect("MemStore::bound_skills"),
        "both stores resolve {scope:?} with model::skill::resolve"
    );
    pg
}

/// `(name, level, activation)` by name, the part of an answer these cases pin.
fn by_name(
    answer: &[htui_core::model::BoundSkill],
) -> std::collections::BTreeMap<&str, (BindingLevel, Activation)> {
    answer
        .iter()
        .map(|s| (s.name.as_str(), (s.level, s.activation)))
        .collect()
}

/// MOD-127 D5: every level at once. Five levels of attachments are planted in both stores, and
/// every scope of a matrix (workspace named or not, two projects, four item kinds or none, a phase
/// or none) answers the same candidates; a few scopes are also pinned by level.
#[tokio::test(flavor = "multi_thread")]
async fn pg_and_mem_agree_on_all_five_levels() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    let platform = Some(ids::WORKSPACE_PLATFORM);
    let htui = Some(ids::PROJECT_HTUI);
    let rows: [Scoped; 6] = [
        ("house", None, None, None, None, "always", 5),
        ("tests", platform, None, None, None, "always", 4),
        ("rust-style", platform, None, None, None, "off", 2),
        (
            "tests",
            None,
            htui,
            Some(ids::KIND_HTUI_FEAT),
            None,
            "off",
            6,
        ),
        (
            "house",
            None,
            htui,
            Some(ids::KIND_HTUI_FIX),
            None,
            "always",
            1,
        ),
        (
            "rust-style",
            None,
            Some(ids::PROJECT_AGY),
            Some(ids::KIND_AGY_FEAT),
            None,
            "always",
            3,
        ),
    ];
    let mem = plant_scoped(&db, &rows).await;

    for workspace in [
        None,
        Some(ids::WORKSPACE_PLATFORM),
        Some(ids::WORKSPACE_GRAPHICS),
    ] {
        for project in [ids::PROJECT_HTUI, ids::PROJECT_AGY] {
            for item_kind in [
                None,
                Some(ids::KIND_HTUI_FEAT),
                Some(ids::KIND_HTUI_FIX),
                Some(ids::KIND_AGY_FEAT),
            ] {
                for phase in [None, Some(ids::PHASE_HTUI_IMPLEMENT)] {
                    both(
                        &db,
                        &mem,
                        ResolveScope {
                            workspace,
                            project,
                            item_kind,
                            phase,
                        },
                    )
                    .await;
                }
            }
        }
    }

    let feat_implement = both(
        &db,
        &mem,
        ResolveScope {
            workspace: platform,
            project: ids::PROJECT_HTUI,
            item_kind: Some(ids::KIND_HTUI_FEAT),
            phase: Some(ids::PHASE_HTUI_IMPLEMENT),
        },
    )
    .await;
    assert_eq!(
        by_name(&feat_implement),
        [
            ("house", (BindingLevel::Global, Activation::Always)),
            ("rust-style", (BindingLevel::Phase, Activation::Always)),
            ("tests", (BindingLevel::ItemKind, Activation::Off)),
        ]
        .into_iter()
        .collect(),
        "the phase beats the workspace's off, the item kind beats the project's and the workspace's"
    );
    let fix = both(
        &db,
        &mem,
        ResolveScope {
            workspace: None,
            project: ids::PROJECT_HTUI,
            item_kind: Some(ids::KIND_HTUI_FIX),
            phase: None,
        },
    )
    .await;
    assert_eq!(
        by_name(&fix),
        [
            ("house", (BindingLevel::ItemKind, Activation::Always)),
            ("rust-style", (BindingLevel::Project, Activation::Always)),
            ("tests", (BindingLevel::Project, Activation::Always)),
        ]
        .into_iter()
        .collect(),
        "with no workspace named the workspace rows do not apply; the item kind beats global"
    );
    let agy = both(
        &db,
        &mem,
        ResolveScope {
            workspace: platform,
            project: ids::PROJECT_AGY,
            item_kind: None,
            phase: None,
        },
    )
    .await;
    assert_eq!(
        by_name(&agy),
        [
            ("house", (BindingLevel::Global, Activation::Always)),
            ("rust-style", (BindingLevel::Workspace, Activation::Off)),
            ("tests", (BindingLevel::Workspace, Activation::Always)),
        ]
        .into_iter()
        .collect(),
        "a project with no row of its own sees the workspace's and the global ones"
    );

    db.drop_db().await;
}

/// MOD-127 D5: a workspace row applies only when the scope names that workspace; a `None` in the
/// scope matches no workspace row.
#[tokio::test(flavor = "multi_thread")]
async fn a_workspace_row_applies_only_when_the_scope_names_it() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    let mem = plant_scoped(
        &db,
        &[(
            "house",
            Some(ids::WORKSPACE_PLATFORM),
            None,
            None,
            None,
            "always",
            0,
        )],
    )
    .await;

    for (workspace, applies) in [
        (None, false),
        (Some(ids::WORKSPACE_GRAPHICS), false),
        (Some(ids::WORKSPACE_PLATFORM), true),
    ] {
        let answer = both(
            &db,
            &mem,
            ResolveScope {
                workspace,
                project: ids::PROJECT_AGY,
                item_kind: None,
                phase: None,
            },
        )
        .await;
        let levels: Vec<BindingLevel> = answer
            .iter()
            .filter(|s| s.name == "house")
            .map(|s| s.level)
            .collect();
        assert_eq!(
            levels,
            if applies {
                vec![BindingLevel::Workspace]
            } else {
                Vec::new()
            },
            "workspace {workspace:?}: the row applies only to the workspace it names"
        );
    }

    db.drop_db().await;
}

/// MOD-127 D5: an item-kind row applies only to that kind of that project: not to another kind,
/// not to no kind, not to the same kind id under another project.
#[tokio::test(flavor = "multi_thread")]
async fn an_item_kind_row_applies_only_to_that_kind() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    let mem = plant_scoped(
        &db,
        &[(
            "house",
            None,
            Some(ids::PROJECT_HTUI),
            Some(ids::KIND_HTUI_FEAT),
            None,
            "always",
            0,
        )],
    )
    .await;

    for (project, item_kind, applies) in [
        (ids::PROJECT_HTUI, None, false),
        (ids::PROJECT_HTUI, Some(ids::KIND_HTUI_FIX), false),
        (ids::PROJECT_HTUI, Some(ids::KIND_HTUI_FEAT), true),
        (ids::PROJECT_AGY, Some(ids::KIND_HTUI_FEAT), false),
    ] {
        let answer = both(
            &db,
            &mem,
            ResolveScope {
                workspace: None,
                project,
                item_kind,
                phase: None,
            },
        )
        .await;
        let levels: Vec<BindingLevel> = answer
            .iter()
            .filter(|s| s.name == "house")
            .map(|s| s.level)
            .collect();
        assert_eq!(
            levels,
            if applies {
                vec![BindingLevel::ItemKind]
            } else {
                Vec::new()
            },
            "({project:?}, {item_kind:?}): the row applies only to its own kind"
        );
    }

    db.drop_db().await;
}

/// MOD-127 D3: rows written through the pre-migration shape (`skill_binding`, `skill_id`, three
/// keys) at v40 survive `0041` with their ids and answer the same candidates afterwards, as global
/// and project levels, with no workspace or item kind.
#[tokio::test(flavor = "multi_thread")]
async fn old_rows_resolve_as_before_the_migration() {
    let Some(db) = common::bare_db().await else {
        return;
    };
    MIGRATOR
        .run_to(40, &db.pool)
        .await
        .expect("apply 0001 through 0040");
    let pool = &db.pool;
    let user = uuid::Uuid::now_v7();
    let project = uuid::Uuid::now_v7();
    let (house, style) = (uuid::Uuid::now_v7(), uuid::Uuid::now_v7());
    sqlx::query("INSERT INTO app_user (id, name) VALUES ($1, 'old-rows')")
        .bind(user)
        .execute(pool)
        .await
        .expect("insert app_user");
    sqlx::query("INSERT INTO project (id, slug, name, created_by) VALUES ($1, 'old', 'old', $2)")
        .bind(project)
        .bind(user)
        .execute(pool)
        .await
        .expect("insert project");
    for (id, name) in [(house, "house"), (style, "style")] {
        sqlx::query("INSERT INTO skill (id, name, created_by) VALUES ($1, $2, $3)")
            .bind(id)
            .bind(name)
            .bind(user)
            .execute(pool)
            .await
            .expect("insert skill");
        sqlx::query(
            "INSERT INTO skill_version (skill_id, version, body, created_by) \
             VALUES ($1, 1, $2, $3)",
        )
        .bind(id)
        .bind(format!("{name} rules."))
        .bind(user)
        .execute(pool)
        .await
        .expect("insert skill_version");
    }
    let (global_row, project_row) = (uuid::Uuid::now_v7(), uuid::Uuid::now_v7());
    sqlx::query(
        "INSERT INTO skill_binding (id, skill_id, project_id, phase_id, position, activation) \
         VALUES ($1, $2, NULL, NULL, 5, 'always')",
    )
    .bind(global_row)
    .bind(house)
    .execute(pool)
    .await
    .expect("insert a global row, pre-migration shape");
    sqlx::query(
        "INSERT INTO skill_binding \
         (id, skill_id, project_id, phase_id, position, activation, globs) \
         VALUES ($1, $2, $3, NULL, 2, 'glob', ARRAY['**/*.rs'])",
    )
    .bind(project_row)
    .bind(style)
    .bind(project)
    .execute(pool)
    .await
    .expect("insert a project row, pre-migration shape");

    MIGRATOR.run_to(41, pool).await.expect("apply 0041");

    let answer = db
        .store
        .bound_skills(ResolveScope::project(ProjectId::from_uuid(project), None))
        .await
        .expect("PgStore::bound_skills");
    assert_eq!(
        answer
            .iter()
            .map(|s| (
                s.name.as_str(),
                s.level,
                s.activation,
                s.version,
                s.position
            ))
            .collect::<Vec<_>>(),
        vec![
            ("style", BindingLevel::Project, Activation::Glob, Some(1), 2),
            (
                "house",
                BindingLevel::Global,
                Activation::Always,
                Some(1),
                5
            ),
        ],
        "the two old rows resolve as they did before the rename"
    );
    let kept: Vec<(uuid::Uuid, String, uuid::Uuid)> =
        sqlx::query_as("SELECT id, kind, ref_id FROM toolbox_binding ORDER BY position DESC")
            .fetch_all(pool)
            .await
            .expect("read the renamed rows");
    assert_eq!(
        kept,
        vec![
            (global_row, "skill".to_owned(), house),
            (project_row, "skill".to_owned(), style),
        ],
        "the migration keeps the row ids and gives them kind skill"
    );

    db.drop_db().await;
}
