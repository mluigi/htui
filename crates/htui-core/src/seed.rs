//! The catalogue every project is born with (MOD-15 milestone 2, plan D1–D3).
//!
//! `R-ENT-6` fixes five item kinds, each with a default step graph; ANA-5 §5.4 fixes ten prompt
//! templates. One table for both of this crate's row sources: `store::mem` and `htui-store`'s
//! `PgStore` build a project's thirty-five rows from it with fresh UUIDv7 ids, and the demo
//! fixture builds the same rows with its deterministic `demo_uuid` ids. There is no second copy
//! anywhere, which is what makes PRD risk "seed and fixture drift" structurally impossible.
//!
//! The table is ANA-2 §4.1 as amended (`docs/ANA-2.md:307-319`): `implement` takes `plan` *and*
//! `review` (PRD D5 extends that to `refactor` and `tooling`), `bug`'s `fix` takes `reproduce`
//! and `review`, and `gate_hard` is set on exactly three phases — `feature`'s `prd` and `plan`,
//! `analysis`'s `verdict` (PRD D3, which overrides HANDOFF's "prd, plan and verdict" wording).
//! Every other phase column is ANA-2's frozen default, written by [`phase_row`].
//!
//! The reserved template names `judge` and `handoff` ([`TemplateRole::of_name`]) are never phase
//! names here; the unit tests pin that, and nothing re-checks it at runtime (plan D6).
//!
//! [`TemplateRole::of_name`]: crate::prompt::TemplateRole::of_name

use chrono::{DateTime, Utc};

use crate::model::{
    CommandQueue, Gate, ItemKind, ItemKindId, PhaseId, ProjectId, PromptTemplate, PromptTemplateId,
    StepGraph, StepGraphId, StepGraphPhase, UserId,
};

/// One phase of a default graph: the three columns the table varies, in position order within
/// [`KindSeed::phases`]. Everything else on the row is [`phase_row`]'s frozen default.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhaseSeed {
    /// `step_graph_phase.name`; also its `output_kind` and `template_name`.
    pub name: &'static str,
    /// `step_graph_phase.input_kinds`: names of phases of the **same** graph whose documents this
    /// phase reads. Empty at position 0 on every graph.
    pub input_kinds: &'static [&'static str],
    /// `step_graph_phase.gate_hard` (PRD D3).
    pub gate_hard: bool,
}

/// One `R-ENT-6` kind with its default graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KindSeed {
    /// `item_kind.prefix`, e.g. `FEAT`; passes [`ItemKind::prefix_is_valid`].
    pub prefix: &'static str,
    /// `item_kind.name`, which is also `step_graph.name`.
    pub name: &'static str,
    /// `item_kind.description`.
    pub description: &'static str,
    /// The default graph's phases, position order.
    pub phases: &'static [PhaseSeed],
}

/// The five kinds, in `item_kind.position` order (plan D3).
pub const KINDS: [KindSeed; 5] = [
    KindSeed {
        prefix: "ANA",
        name: "analysis",
        description: "A question answered in writing",
        phases: &[
            PhaseSeed {
                name: "research",
                input_kinds: &[],
                gate_hard: false,
            },
            PhaseSeed {
                name: "verdict",
                input_kinds: &["research"],
                gate_hard: true,
            },
        ],
    },
    KindSeed {
        prefix: "FEAT",
        name: "feature",
        description: "New behaviour",
        phases: &[
            PhaseSeed {
                name: "prd",
                input_kinds: &[],
                gate_hard: true,
            },
            PhaseSeed {
                name: "plan",
                input_kinds: &["prd"],
                gate_hard: true,
            },
            PhaseSeed {
                name: "implement",
                input_kinds: &["plan", "review"],
                gate_hard: false,
            },
            PhaseSeed {
                name: "review",
                input_kinds: &["implement"],
                gate_hard: false,
            },
        ],
    },
    KindSeed {
        prefix: "FIX",
        name: "bug",
        description: "Behaviour that is wrong",
        phases: &[
            PhaseSeed {
                name: "reproduce",
                input_kinds: &[],
                gate_hard: false,
            },
            PhaseSeed {
                name: "fix",
                input_kinds: &["reproduce", "review"],
                gate_hard: false,
            },
            PhaseSeed {
                name: "review",
                input_kinds: &["fix"],
                gate_hard: false,
            },
        ],
    },
    KindSeed {
        prefix: "CLEAN",
        name: "refactor",
        description: "Behaviour kept, shape improved",
        phases: &[
            PhaseSeed {
                name: "plan",
                input_kinds: &[],
                gate_hard: false,
            },
            PhaseSeed {
                name: "implement",
                input_kinds: &["plan", "review"],
                gate_hard: false,
            },
            PhaseSeed {
                name: "review",
                input_kinds: &["implement"],
                gate_hard: false,
            },
        ],
    },
    KindSeed {
        prefix: "TOOL",
        name: "tooling",
        description: "The workshop rather than the product",
        phases: &[
            PhaseSeed {
                name: "plan",
                input_kinds: &[],
                gate_hard: false,
            },
            PhaseSeed {
                name: "implement",
                input_kinds: &["plan", "review"],
                gate_hard: false,
            },
            PhaseSeed {
                name: "review",
                input_kinds: &["implement"],
                gate_hard: false,
            },
        ],
    },
];

/// Phases across all of [`KINDS`]: fifteen. The fixture's per-project phase id stride.
pub const PHASES_PER_PROJECT: usize = {
    let mut total = 0;
    let mut i = 0;
    while i < KINDS.len() {
        total += KINDS[i].phases.len();
        i += 1;
    }
    total
};

/// `step_graph.description` of a kind's default graph.
#[must_use]
pub fn graph_description(kind_name: &str) -> String {
    format!("Default graph for {kind_name} items")
}

/// A kind's default `step_graph` row. `now` fills both timestamp columns; a Postgres seeder
/// leaves those to the database and discards it (plan D10).
#[must_use]
pub fn graph_row(
    id: StepGraphId,
    project_id: ProjectId,
    kind: &KindSeed,
    now: DateTime<Utc>,
) -> StepGraph {
    StepGraph {
        id,
        project_id,
        name: kind.name.to_owned(),
        description: graph_description(kind.name),
        created_at: now,
        updated_at: now,
    }
}

/// A `step_graph_phase` row at `position` of `graph_id`: the seed's three columns plus ANA-2
/// §4.1's frozen defaults — `fan_out 1`, `gate Always`, `retry_limit 1`, `isolation None`,
/// `command_queue FanOutOnly`, `verify_command None`, `output_kind` and `template_name` equal to
/// the name, `template_version None` (follow latest), `token_budget None` (project default).
#[must_use]
pub fn phase_row(
    id: PhaseId,
    graph_id: StepGraphId,
    position: i32,
    phase: &PhaseSeed,
    now: DateTime<Utc>,
) -> StepGraphPhase {
    StepGraphPhase {
        id,
        graph_id,
        position,
        name: phase.name.to_owned(),
        fan_out: 1,
        gate: Gate::Always,
        gate_hard: phase.gate_hard,
        retry_limit: 1,
        input_kinds: phase
            .input_kinds
            .iter()
            .map(|kind| (*kind).to_owned())
            .collect(),
        output_kind: phase.name.to_owned(),
        isolation: None,
        command_queue: CommandQueue::FanOutOnly,
        verify_command: None,
        template_name: phase.name.to_owned(),
        template_version: None,
        token_budget: None,
        updated_at: now,
    }
}

/// An `item_kind` row whose default graph is `graph_id` (the row [`graph_row`] built for the
/// same `kind`) and whose `position` is the kind's index in [`KINDS`].
#[must_use]
pub fn kind_row(
    id: ItemKindId,
    project_id: ProjectId,
    graph_id: StepGraphId,
    position: i32,
    kind: &KindSeed,
    now: DateTime<Utc>,
) -> ItemKind {
    ItemKind {
        id,
        project_id,
        prefix: kind.prefix.to_owned(),
        name: kind.name.to_owned(),
        description: kind.description.to_owned(),
        default_graph_id: graph_id,
        position,
        updated_at: now,
    }
}

/// A version-1 `prompt_template` row. Callers iterate [`crate::prompt::DEFAULT_TEMPLATES`] and
/// pass its `(name, _, body)`; `created_by` is the project's creator on the stores and the
/// fixture user in `fixtures.rs`.
#[must_use]
pub fn template_row(
    id: PromptTemplateId,
    project_id: ProjectId,
    name: &str,
    body: &str,
    created_by: UserId,
    now: DateTime<Utc>,
) -> PromptTemplate {
    PromptTemplate {
        id,
        project_id,
        name: name.to_owned(),
        version: 1,
        body: body.to_owned(),
        created_by,
        created_at: now,
        updated_at: now,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::{KINDS, PHASES_PER_PROJECT, graph_row, kind_row, phase_row, template_row};
    use crate::model::{
        CommandQueue, Gate, ItemKind, ItemKindId, PhaseId, ProjectId, PromptTemplateId,
        StepGraphId, UserId,
    };
    use crate::prompt::{DEFAULT_TEMPLATES, TemplateRole, body_of};
    use chrono::Utc;

    /// `R-ENT-6`'s five kinds and ANA-2 §4.1's fifteen phases, which are what the seeders count
    /// out and what `PHASES_PER_PROJECT` has to agree with — the fixture mints phase ids from it.
    #[test]
    fn five_kinds_and_fifteen_phases() {
        assert_eq!(KINDS.len(), 5, "R-ENT-6 fixes five kinds");
        assert_eq!(PHASES_PER_PROJECT, 15);
        assert_eq!(
            KINDS
                .iter()
                .map(|kind| kind.phases.len())
                .collect::<Vec<_>>(),
            [2, 4, 3, 3, 3],
            "analysis 2, feature 4, bug/refactor/tooling 3 each"
        );
    }

    /// Both columns are unique per project in the schema (`uq_item_kind_prefix`,
    /// `uq_item_kind_name`), so a duplicate here would refuse the seed on Postgres and go
    /// unnoticed on `MemStore`, whose maps are keyed by id.
    #[test]
    fn kind_names_and_prefixes_are_unique() {
        let names: HashSet<&str> = KINDS.iter().map(|kind| kind.name).collect();
        let prefixes: HashSet<&str> = KINDS.iter().map(|kind| kind.prefix).collect();
        assert_eq!(names.len(), KINDS.len(), "five distinct names");
        assert_eq!(prefixes.len(), KINDS.len(), "five distinct prefixes");
    }

    /// The `item_kind.prefix` CHECK, applied to the table rather than to a request: the seed is
    /// the one writer that never goes through `create_item_kind`'s validation (plan D5).
    #[test]
    fn every_prefix_passes_the_check() {
        for kind in &KINDS {
            assert!(
                ItemKind::prefix_is_valid(kind.prefix),
                "`{}` is not a legal item_kind.prefix",
                kind.prefix
            );
        }
    }

    /// D6.1: `judge` and `handoff` are template roles, never phase names. Nothing refuses a
    /// seed phase called `judge` at runtime — the editor's refusal is milestone 4's — so this
    /// constant-table test is the whole enforcement.
    #[test]
    fn no_seed_phase_is_a_reserved_template_role() {
        for kind in &KINDS {
            for phase in kind.phases {
                assert_eq!(
                    TemplateRole::of_name(phase.name),
                    TemplateRole::Phase,
                    "`{}`/`{}` is a reserved template name",
                    kind.name,
                    phase.name
                );
            }
        }
    }

    /// D6.3: a seeded phase always has the template the seed also writes, so a fresh project can
    /// render every step of every default graph without a hand-written body.
    #[test]
    fn every_phase_name_has_a_shipped_body() {
        for kind in &KINDS {
            for phase in kind.phases {
                assert!(
                    body_of(phase.name).is_some(),
                    "`{}`/`{}` has no DEFAULT_TEMPLATES body",
                    kind.name,
                    phase.name
                );
            }
        }
        assert_eq!(DEFAULT_TEMPLATES.len(), 10, "ANA-5 §5.4's ten");
        for reserved in ["judge", "handoff"] {
            assert!(
                DEFAULT_TEMPLATES.iter().any(|(name, ..)| *name == reserved),
                "the seed writes the reserved `{reserved}` template too"
            );
        }
    }

    /// Phase positions are dense by construction — [`phase_row`] takes the enumeration index —
    /// so what is left to check is that no graph names a phase twice, which would make
    /// `input_kinds` ambiguous and `template_name` collide.
    #[test]
    fn names_are_unique_and_positions_dense_per_graph() {
        let graph = StepGraphId::new();
        for kind in &KINDS {
            let names: HashSet<&str> = kind.phases.iter().map(|phase| phase.name).collect();
            assert_eq!(
                names.len(),
                kind.phases.len(),
                "`{}` names a phase twice",
                kind.name
            );
            for (position, phase) in kind.phases.iter().enumerate() {
                let row = phase_row(PhaseId::new(), graph, position as i32, phase, Utc::now());
                assert_eq!(row.position, position as i32);
            }
        }
    }

    /// The first phase of a graph has nothing upstream to read: it is the step that turns an
    /// item's own body into the graph's first document.
    #[test]
    fn position_zero_takes_no_inputs() {
        for kind in &KINDS {
            assert!(
                kind.phases[0].input_kinds.is_empty(),
                "`{}`'s first phase reads something",
                kind.name
            );
        }
    }

    /// PRD D3 names three and says "none elsewhere", explicitly against HANDOFF's wording, which
    /// would have flagged `plan` on `refactor` and `tooling` too.
    #[test]
    fn gate_hard_is_exactly_the_three_prd_flags() {
        let flagged: Vec<(&str, &str)> = KINDS
            .iter()
            .flat_map(|kind| {
                kind.phases
                    .iter()
                    .filter(|phase| phase.gate_hard)
                    .map(move |phase| (kind.name, phase.name))
            })
            .collect();
        assert_eq!(
            flagged,
            [
                ("analysis", "verdict"),
                ("feature", "prd"),
                ("feature", "plan"),
            ],
            "PRD D3's three hard gates and no fourth"
        );
    }

    /// `input_kinds` holds phase names of the *same* graph (ANA-2 §4.1), so a typo or a name
    /// borrowed from another kind's graph would resolve to no document at all.
    #[test]
    fn every_input_kind_names_a_phase_of_the_same_graph() {
        for kind in &KINDS {
            let names: HashSet<&str> = kind.phases.iter().map(|phase| phase.name).collect();
            for phase in kind.phases {
                for input in phase.input_kinds {
                    assert!(
                        names.contains(input),
                        "`{}`/`{}` reads `{input}`, which is no phase of that graph",
                        kind.name,
                        phase.name
                    );
                }
            }
        }
    }

    /// The four constructors, column by column: what the seeders write and never re-derive.
    #[test]
    fn rows_carry_the_frozen_defaults() {
        let now = Utc::now();
        let project = ProjectId::new();
        let graph_id = StepGraphId::new();
        let feature = &KINDS[1];
        let implement = &feature.phases[2];

        let phase = phase_row(PhaseId::new(), graph_id, 2, implement, now);
        assert_eq!(
            (
                phase.fan_out,
                phase.gate,
                phase.retry_limit,
                phase.isolation,
                phase.command_queue,
                phase.verify_command.as_deref(),
                phase.template_version,
                phase.token_budget,
            ),
            (
                1,
                Gate::Always,
                1,
                None,
                CommandQueue::FanOutOnly,
                None,
                None,
                None
            ),
            "ANA-2 §4.1's frozen defaults"
        );
        assert_eq!(phase.output_kind, "implement");
        assert_eq!(phase.template_name, "implement");
        assert_eq!(phase.input_kinds, ["plan", "review"], "in table order");
        assert_eq!(phase.graph_id, graph_id);
        assert_eq!(phase.position, 2);
        assert_eq!(phase.updated_at, now);

        let graph = graph_row(StepGraphId::new(), project, feature, now);
        assert_eq!(graph.name, "feature");
        assert_eq!(graph.description, "Default graph for feature items");
        assert_eq!((graph.created_at, graph.updated_at), (now, now));
        assert_eq!(graph.project_id, project);

        let kind = kind_row(ItemKindId::new(), project, graph_id, 1, feature, now);
        assert_eq!(
            (kind.prefix.as_str(), kind.name.as_str()),
            ("FEAT", "feature")
        );
        assert_eq!(kind.description, "New behaviour");
        assert_eq!(kind.default_graph_id, graph_id);
        assert_eq!(kind.position, 1);
        assert_eq!(kind.updated_at, now);

        let author = UserId::new();
        let (name, _, body) = DEFAULT_TEMPLATES[0];
        let template = template_row(PromptTemplateId::new(), project, name, body, author, now);
        assert_eq!(template.name, "prd");
        assert_eq!(template.version, 1, "the seed writes version 1 only");
        assert_eq!(template.body, body);
        assert_eq!(template.created_by, author, "the caller's, not a constant");
        assert_eq!((template.created_at, template.updated_at), (now, now));
    }
}
