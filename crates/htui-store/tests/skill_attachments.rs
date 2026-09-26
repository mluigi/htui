//! MOD-9 milestone 2 (plan D38, D41; ANA-22 §6 items 2-4, §7.1): skill attachments at three
//! levels, on a real server.
//!
//! `0007_skill_attachments.sql` lets `skill_binding.project_id` be NULL (a global attachment) and
//! adds the activation. What `MemStore` cannot show is that Postgres agrees with it: the same
//! attachments planted in both answer the same candidates, a global row survives the project
//! delete that cascades the project's own, and the new checks refuse the rows ANA-22 rules out,
//! each under its own constraint name.
//!
//! Every insert is `sqlx::query`, unchecked, so the file adds nothing to `.sqlx` (the
//! `prompt_template_cas.rs` rule). With `HTUI_TEST_DATABASE_URL` unset each case prints
//! `common::SKIP` and passes (plan D13).
#![cfg(feature = "demo")]

use htui_store::testkit as common;

use chrono::Utc;
use htui_core::fixtures::{DemoData, demo_data, ids};
use htui_core::model::skill::select;
use htui_core::model::{
    Activation, ChoiceReason, PhaseId, ProjectId, Skill, SkillBinding, SkillBindingId, SkillId,
    SkillLevel, SkillVersion,
};
use htui_core::store::{MemStore, WriteStore as _};
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

/// Inserts one `skill_binding` row, unpinned, and hands back the driver's answer so a refusal can
/// be inspected.
async fn plant_binding(
    pool: &PgPool,
    skill: SkillId,
    project: Option<ProjectId>,
    phase: Option<PhaseId>,
    activation: &str,
    globs: &[&str],
    position: i32,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO skill_binding (skill_id, project_id, phase_id, activation, globs, position) \
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(skill.as_uuid())
    .bind(project.map(ProjectId::as_uuid))
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
        data.skill_bindings.push(SkillBinding {
            id: SkillBindingId::new(),
            skill_id,
            project_id: *project_id,
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
                .bound_skills(project, phase)
                .await
                .expect("PgStore::bound_skills"),
            mem.bound_skills(project, phase)
                .await
                .expect("MemStore::bound_skills"),
            "both stores resolve ({project:?}, {phase:?}) with model::skill::resolve"
        );
    }

    let implement = db
        .store
        .bound_skills(ids::PROJECT_HTUI, Some(ids::PHASE_HTUI_IMPLEMENT))
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
            ("tests", SkillLevel::Phase, Activation::Off, Some(1), 0),
            (
                "rust-style",
                SkillLevel::Phase,
                Activation::Always,
                Some(1),
                2
            ),
            ("house", SkillLevel::Global, Activation::Always, Some(1), 5),
        ],
        "the phase's off beats the project's and the global tests; the phase pin holds \
         rust-style at v1; house reaches the project from the global level"
    );

    let agy = db
        .store
        .bound_skills(ids::PROJECT_AGY, None)
        .await
        .expect("PgStore::bound_skills");
    assert_eq!(
        agy.iter()
            .map(|s| (s.name.as_str(), s.level, s.activation, s.position))
            .collect::<Vec<_>>(),
        vec![
            ("tests", SkillLevel::Global, Activation::Off, 3),
            ("house", SkillLevel::Global, Activation::Always, 5),
            ("rust-style", SkillLevel::Global, Activation::Always, 9),
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
        .bound_skills(ids::PROJECT_HTUI, Some(ids::PHASE_HTUI_IMPLEMENT))
        .await
        .expect("PgStore::bound_skills");
    let tests = candidates
        .iter()
        .find(|s| s.name == "tests")
        .expect("tests is still a candidate");
    assert_eq!(
        (tests.level, tests.activation),
        (SkillLevel::Phase, Activation::Off),
        "the phase row wins over the project row"
    );

    let (active, choices) = select(candidates, true);
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
        sqlx::query_scalar("SELECT count(*) FROM skill_binding WHERE project_id IS NULL")
            .fetch_one(&db.pool)
            .await
            .expect("count the global rows");
    assert_eq!(global, 1, "the global row survives");
    assert_eq!(
        db.store
            .bound_skills(ids::PROJECT_AGY, None)
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
        "skill_binding_phase_needs_project",
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
        "skill_binding_glob_needs_globs",
        "a glob attachment needs at least one glob"
    );
    assert_eq!(
        constraint(plant_binding(&db.pool, skill, None, None, "sometimes", &[], 0).await),
        "skill_binding_activation_check",
        "activation is always, glob or off"
    );
    plant_binding(&db.pool, skill, None, None, "always", &[], 0)
        .await
        .expect("one global attachment of a skill is accepted");
    assert_eq!(
        constraint(plant_binding(&db.pool, skill, None, None, "off", &[], 1).await),
        "skill_binding_skill_id_project_id_phase_id_key",
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
        "SELECT activation, globs, languages FROM skill_binding WHERE id = ANY($1) ORDER BY id",
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
