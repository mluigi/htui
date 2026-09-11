//! The demo fixture loader (feature `demo`), blueprint C.5.
//!
//! One transaction, explicit ids and explicit `created_at` / `updated_at` on every row, so the
//! fixture MOD-1's conformance suite asserts against is identical on Postgres and in `MemStore`:
//! the `BEFORE UPDATE` trigger of the migration does not touch an `INSERT`.
//!
//! No `ON CONFLICT DO NOTHING` anywhere: `load_demo` runs against a database created seconds
//! earlier, so a conflict is a bug that must surface. The single exception is `agent`, which
//! `seed_if_empty_as` has filled since MOD-2: the fixture deletes those two rows by name before
//! inserting its own, because it brings a whole world rather than rows to merge (see the comment
//! at the `agent` loop).

use htui_core::fixtures::DemoData;
use htui_core::model::{
    AgentId, BoxId, CommandQueue, DocumentId, Gate, GateOutcome, Isolation, ItemId, ItemKindId,
    NoteId, PhaseId, ProjectId, RunId, SkillBindingId, SkillId, StepGraphId, StepId, UserId,
    WorkspaceId,
};
use htui_core::store::Result;

use crate::error::map_sqlx;
use crate::pg::PgStore;

impl PgStore {
    /// Loads a [`DemoData`] fixture with its own ids and timestamps, in foreign-key order.
    ///
    /// `DemoData` has no `repo`, `repo_box_path`, `workspace_box_path`, `capability_tag`,
    /// `phase_agent`, `agent_box`, `run_step_commit`, `command_run` or `app_setting` field, so
    /// those tables are left alone. `repo` stays on that list by plan D110, which dropped the
    /// fixture rows this milestone would otherwise have added.
    ///
    /// `skill`, `skill_version`, `skill_binding` and `box_tool` came **off** it in milestone 9
    /// (blueprint E-7): the fixture carries those rows for `MemStore`, and a loader that skipped
    /// them would run `PgStore::bound_skills` and `PgStore::box_profile` against empty tables
    /// while `MemStore` answered from the fixture — two backends the conformance suite could not
    /// compare.
    ///
    /// Takes `&mut self` because the fixture also names the box and the author the store points at
    /// (`this_box`, `this_user`), exactly as `MemStore::from_demo` does; the blueprint's `&self`
    /// cannot express that.
    ///
    /// # Errors
    ///
    /// Whatever the driver reports, through [`map_sqlx`]. The transaction
    /// is rolled back on the first failure.
    #[allow(clippy::too_many_lines)] // nineteen tables, one INSERT each; splitting hides the order.
    pub async fn load_demo(&mut self, data: &DemoData) -> Result<()> {
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;

        for row in &data.users {
            sqlx::query!(
                "INSERT INTO app_user (id, name, email, created_at, updated_at) \
                 VALUES ($1, $2, $3, $4, $5)",
                row.id.as_uuid(),
                row.name,
                row.email.as_deref(),
                row.created_at,
                row.updated_at,
            )
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        }

        for row in &data.boxes {
            sqlx::query!(
                "INSERT INTO box (id, user_id, hostname, os_family, os_version, arch, cpu, ram_mb, \
                 gpu_present, gpu_vendor, htui_version, probed_tags, declared_tags, quirks, \
                 settings, registered_at, last_seen_at, last_probed_at, updated_at) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, \
                 $17, $18, $19)",
                row.id.as_uuid(),
                row.user_id.as_uuid(),
                row.hostname,
                row.os_family.as_str(),
                row.os_version,
                row.arch,
                row.cpu,
                row.ram_mb,
                row.gpu_present,
                row.gpu_vendor.as_deref(),
                row.htui_version,
                &row.probed_tags[..],
                &row.declared_tags[..],
                row.quirks,
                &row.settings,
                row.registered_at,
                row.last_seen_at,
                row.last_probed_at,
                row.updated_at,
            )
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        }

        for row in &data.workspaces {
            sqlx::query!(
                "INSERT INTO workspace (id, slug, name, description, created_by, created_at, \
                 updated_at) VALUES ($1, $2, $3, $4, $5, $6, $7)",
                row.id.as_uuid(),
                row.slug,
                row.name,
                row.description,
                row.created_by.as_uuid(),
                row.created_at,
                row.updated_at,
            )
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        }

        for row in &data.projects {
            sqlx::query!(
                "INSERT INTO project (id, slug, name, description, secret_provider, secret_scope, \
                 settings, created_by, created_at, updated_at) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
                row.id.as_uuid(),
                row.slug,
                row.name,
                row.description,
                row.secret_provider.as_deref(),
                row.secret_scope.as_deref(),
                &row.settings,
                row.created_by.as_uuid(),
                row.created_at,
                row.updated_at,
            )
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        }

        for row in &data.workspace_projects {
            sqlx::query!(
                "INSERT INTO workspace_project (workspace_id, project_id, position) \
                 VALUES ($1, $2, $3)",
                WorkspaceId::as_uuid(row.workspace_id),
                ProjectId::as_uuid(row.project_id),
                row.position,
            )
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        }

        // The one exception to this module's "a conflict is a bug" rule, and it is about *names*,
        // not rows. Since MOD-2, `seed_if_empty_as` seeds `claude` and `agy` from
        // `docs/ANA-4.md` §5.3, so a freshly migrated database is no longer empty of agents, and
        // the fixture carries those same two names under its own deterministic ids. The fixture is
        // a whole world - it brings its own `app_user` and `box` for the same reason - so it owns
        // the registry too: the seeded rows go, and the fixture's take their place. Nothing
        // references them yet (the fixture's own `run_step` rows are inserted further down), and
        // `agent_box` would cascade if anything did.
        let fixture_agents: Vec<String> = data.agents.iter().map(|row| row.name.clone()).collect();
        sqlx::query!(
            "DELETE FROM agent WHERE name = ANY($1)",
            &fixture_agents[..]
        )
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;

        for row in &data.agents {
            sqlx::query!(
                "INSERT INTO agent (id, name, transport, launch, models, default_model, billing, \
                 enabled, settings, created_at, updated_at) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
                AgentId::as_uuid(row.id),
                row.name,
                row.transport.as_str(),
                &row.launch,
                &row.models[..],
                row.default_model.as_deref(),
                row.billing.as_str(),
                row.enabled,
                &row.settings,
                row.created_at,
                row.updated_at,
            )
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        }

        for row in &data.graphs {
            sqlx::query!(
                "INSERT INTO step_graph (id, project_id, name, description, created_at, \
                 updated_at) VALUES ($1, $2, $3, $4, $5, $6)",
                StepGraphId::as_uuid(row.id),
                ProjectId::as_uuid(row.project_id),
                row.name,
                row.description,
                row.created_at,
                row.updated_at,
            )
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        }

        for row in &data.phases {
            sqlx::query!(
                "INSERT INTO step_graph_phase (id, graph_id, position, name, fan_out, gate, \
                 gate_hard, retry_limit, input_kinds, output_kind, isolation, command_queue, \
                 verify_command, template_name, template_version, token_budget, updated_at) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, \
                 $17)",
                row.id.as_uuid(),
                StepGraphId::as_uuid(row.graph_id),
                row.position,
                row.name,
                row.fan_out,
                Gate::as_str(row.gate),
                row.gate_hard,
                row.retry_limit,
                &row.input_kinds[..],
                row.output_kind,
                row.isolation.map(Isolation::as_str),
                CommandQueue::as_str(row.command_queue),
                row.verify_command.as_deref(),
                row.template_name,
                row.template_version,
                row.token_budget,
                row.updated_at,
            )
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        }

        for row in &data.templates {
            sqlx::query!(
                "INSERT INTO prompt_template (id, project_id, name, version, body, created_by, \
                 created_at, updated_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
                row.id.as_uuid(),
                ProjectId::as_uuid(row.project_id),
                row.name,
                row.version,
                row.body,
                UserId::as_uuid(row.created_by),
                row.created_at,
                row.updated_at,
            )
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        }

        // `box_tool` and the three `skill` tables, in foreign-key order: `skill` before its
        // versions and its bindings, and the bindings after `step_graph_phase` (inserted above)
        // because a phase-level binding references one. `box_tool` needs only the `box` rows from
        // the top of this transaction.
        for row in &data.box_tools {
            sqlx::query!(
                "INSERT INTO box_tool (box_id, name, version, path, probed_at) \
                 VALUES ($1, $2, $3, $4, $5)",
                BoxId::as_uuid(row.box_id),
                row.name,
                row.version,
                row.path,
                row.probed_at,
            )
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        }

        for row in &data.skills {
            sqlx::query!(
                "INSERT INTO skill (id, name, description, created_by, created_at, updated_at) \
                 VALUES ($1, $2, $3, $4, $5, $6)",
                SkillId::as_uuid(row.id),
                row.name,
                row.description,
                UserId::as_uuid(row.created_by),
                row.created_at,
                row.updated_at,
            )
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        }

        for row in &data.skill_versions {
            sqlx::query!(
                "INSERT INTO skill_version (skill_id, version, body, created_by, created_at) \
                 VALUES ($1, $2, $3, $4, $5)",
                SkillId::as_uuid(row.skill_id),
                row.version,
                row.body,
                UserId::as_uuid(row.created_by),
                row.created_at,
            )
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        }

        for row in &data.skill_bindings {
            sqlx::query!(
                "INSERT INTO skill_binding (id, skill_id, project_id, phase_id, pinned_version, \
                 position, updated_at) VALUES ($1, $2, $3, $4, $5, $6, $7)",
                SkillBindingId::as_uuid(row.id),
                SkillId::as_uuid(row.skill_id),
                ProjectId::as_uuid(row.project_id),
                row.phase_id.map(PhaseId::as_uuid),
                row.pinned_version,
                row.position,
                row.updated_at,
            )
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        }

        for row in &data.kinds {
            sqlx::query!(
                "INSERT INTO item_kind (id, project_id, prefix, name, description, \
                 default_graph_id, position, updated_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
                ItemKindId::as_uuid(row.id),
                ProjectId::as_uuid(row.project_id),
                row.prefix,
                row.name,
                row.description,
                StepGraphId::as_uuid(row.default_graph_id),
                row.position,
                row.updated_at,
            )
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        }

        for ((project_id, prefix), last_value) in &data.item_key_counter {
            sqlx::query!(
                "INSERT INTO item_key_counter (project_id, prefix, last_value) \
                 VALUES ($1, $2, $3)",
                ProjectId::as_uuid(*project_id),
                prefix,
                last_value,
            )
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        }

        // `key` is GENERATED ALWAYS (§5.5) and is never inserted.
        for row in &data.items {
            sqlx::query!(
                "INSERT INTO item (id, project_id, kind_id, key_prefix, key_number, title, body, \
                 status, priority, required_tags, touched_paths, step_graph_id, version, \
                 created_by, created_at, updated_at, closed_at) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, \
                 $17)",
                ItemId::as_uuid(row.id),
                ProjectId::as_uuid(row.project_id),
                ItemKindId::as_uuid(row.kind_id),
                row.key_prefix,
                row.key_number,
                row.title,
                row.body,
                row.status.as_str(),
                row.priority,
                &row.required_tags[..],
                &row.touched_paths[..],
                row.step_graph_id.map(StepGraphId::as_uuid),
                row.version,
                UserId::as_uuid(row.created_by),
                row.created_at,
                row.updated_at,
                row.closed_at,
            )
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        }

        for row in &data.revisions {
            sqlx::query!(
                "INSERT INTO item_revision (item_id, version, title, body, required_tags, \
                 author_id, box_id, reason, created_at) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
                ItemId::as_uuid(row.item_id),
                row.version,
                row.title,
                row.body,
                &row.required_tags[..],
                UserId::as_uuid(row.author_id),
                row.box_id.map(BoxId::as_uuid),
                row.reason,
                row.created_at,
            )
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        }

        for row in &data.runs {
            sqlx::query!(
                "INSERT INTO run (id, project_id, item_id, kind, mode, status, target_box_id, \
                 executing_box_id, graph_snapshot, started_by, queued_at, started_at, \
                 finished_at, failure, updated_at) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)",
                RunId::as_uuid(row.id),
                ProjectId::as_uuid(row.project_id),
                row.item_id.map(ItemId::as_uuid),
                row.kind.as_str(),
                row.mode.as_str(),
                row.status.as_str(),
                BoxId::as_uuid(row.target_box_id),
                row.executing_box_id.map(BoxId::as_uuid),
                row.graph_snapshot.as_ref(),
                UserId::as_uuid(row.started_by),
                row.queued_at,
                row.started_at,
                row.finished_at,
                row.failure.as_deref(),
                row.updated_at,
            )
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        }

        for row in &data.steps {
            sqlx::query!(
                "INSERT INTO run_step (id, run_id, position, attempt, fanout_index, phase_name, \
                 agent_id, model, status, gate_outcome, gate_note, selected, exit_code, \
                 prompt_digest, trim_record, usage, isolation_path, started_at, finished_at, \
                 updated_at) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, \
                 $17, $18, $19, $20)",
                StepId::as_uuid(row.id),
                RunId::as_uuid(row.run_id),
                row.position,
                row.attempt,
                row.fanout_index,
                row.phase_name,
                row.agent_id.map(AgentId::as_uuid),
                row.model.as_deref(),
                row.status.as_str(),
                row.gate_outcome.map(GateOutcome::as_str),
                row.gate_note.as_deref(),
                row.selected,
                row.exit_code,
                row.prompt_digest.as_deref(),
                row.trim_record.as_ref(),
                row.usage.as_ref(),
                row.isolation_path.as_deref(),
                row.started_at,
                row.finished_at,
                row.updated_at,
            )
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        }

        // The three forward references to `run_step` are live constraints by now, so `item_link`,
        // `item_note` and `document` follow the steps rather than the items (the blueprint's C.5
        // table puts `document` before `run`, which the fixture's `produced_by_step_id` refutes).
        for row in &data.links {
            sqlx::query!(
                "INSERT INTO item_link (from_item_id, to_item_id, kind, proposed_by_step_id, \
                 created_at, updated_at, deleted_at) VALUES ($1, $2, $3, $4, $5, $6, $7)",
                ItemId::as_uuid(row.from_item_id),
                ItemId::as_uuid(row.to_item_id),
                row.kind.as_str(),
                row.proposed_by_step_id.map(StepId::as_uuid),
                row.created_at,
                row.updated_at,
                row.deleted_at,
            )
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        }

        for row in &data.notes {
            sqlx::query!(
                "INSERT INTO item_note (id, item_id, body, created_by, box_id, via_step_id, \
                 created_at) VALUES ($1, $2, $3, $4, $5, $6, $7)",
                NoteId::as_uuid(row.id),
                ItemId::as_uuid(row.item_id),
                row.body,
                UserId::as_uuid(row.created_by),
                row.box_id.map(BoxId::as_uuid),
                row.via_step_id.map(StepId::as_uuid),
                row.created_at,
            )
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        }

        for row in &data.documents {
            sqlx::query!(
                "INSERT INTO document (id, item_id, kind, version, title, body, \
                 produced_by_step_id, created_by, created_at) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
                DocumentId::as_uuid(row.id),
                ItemId::as_uuid(row.item_id),
                row.kind,
                row.version,
                row.title,
                row.body,
                row.produced_by_step_id.map(StepId::as_uuid),
                UserId::as_uuid(row.created_by),
                row.created_at,
            )
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        }

        for row in &data.events {
            sqlx::query!(
                "INSERT INTO session_event (run_step_id, seq, turn, kind, role, tool_call_id, \
                 payload, raw, at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
                StepId::as_uuid(row.run_step_id),
                row.seq,
                row.turn,
                row.kind.as_str(),
                row.role.as_str(),
                row.tool_call_id.as_deref(),
                &row.payload,
                row.raw.as_ref(),
                row.at,
            )
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        }

        tx.commit().await.map_err(map_sqlx)?;

        // The fixture names the box the top bar points at and the author the conformance cases
        // mint under, exactly as `MemStore::from_demo` does.
        if let Some(this_box) = data.this_box {
            self.this_box = this_box;
        }
        if let Some(user) = data.users.first() {
            self.this_user = user.id;
        }
        Ok(())
    }
}
