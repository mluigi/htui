//! Skills, their versions and their bindings (`docs/ANA-9.md` §5.6), plus the `R-SKL-2`
//! resolution the prompt's skills section renders (`docs/ANA-5.md` §4.2).
//!
//! The writers are MOD-9 milestone 3's (`WriteStore::create_skill` and three others); a `glob`
//! attachment fires from PRD milestone 5 (D86). Since MOD-9 milestone 2 (ANA-22 §6-§7) a skill
//! attaches globally, to a project or to one phase; [`resolve`] picks the most specific
//! attachment per skill, and [`select`] decides, per step, which winners render and records why.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::model::ids::{PhaseId, ProjectId, SkillBindingId, SkillId, UserId};

str_enum!(
    /// `skill_binding.activation` (ANA-22 §6 item 4): whether the winning attachment puts its
    /// skill into a step's prompt.
    Activation {
        /// Always rendered. The column default, so every binding written before `0007` is
        /// unchanged.
        Always => "always",
        /// Rendered when the step's file set matches `globs` (PRD milestone 5, D86). Until the
        /// matcher fires, a `glob` winner is inactive and records `no_path` (plan D40, OQ-12).
        Glob => "glob",
        /// Attached more broadly but not here: a narrower `off` hides a broader attachment.
        Off => "off",
    }
);

/// Which level an attachment sits at (ANA-22 §6 item 2). **Declaration order is specificity**:
/// `Global < Project < Phase`, so the attachment that wins is the `max` (`R-SKL-2` as amended).
///
/// Not a column — it is derived from the two nullable keys by [`SkillBinding::level`] — so it is a
/// plain enum rather than a `str_enum!`, and its serde spelling is its [`as_str`](Self::as_str).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillLevel {
    /// `project_id IS NULL`: every project.
    Global,
    /// `project_id` set, `phase_id IS NULL`.
    Project,
    /// Both set: one phase of one project.
    Phase,
}

impl SkillLevel {
    /// `global`, `project` or `phase`: the record's and the Prompt sub-tab's spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::Project => "project",
            Self::Phase => "phase",
        }
    }
}

/// A row of `skill` (§5.6): one entry of the global skill library. Scoping is by binding, so the
/// row itself belongs to no project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Skill {
    /// `skill.id`.
    pub id: SkillId,
    /// `skill.name`, unique across the library and the tie-break of the render order.
    pub name: String,
    /// `skill.description`; not rendered into a prompt, it is the picker's one-liner.
    pub description: String,
    /// `skill.created_by`.
    pub created_by: UserId,
    /// `skill.created_at`.
    pub created_at: DateTime<Utc>,
    /// `skill.updated_at`.
    pub updated_at: DateTime<Utc>,
}

/// A row of `skill_version` (§5.6): one immutable body of a skill. The primary key is
/// `(skill_id, version)`, so a version number is unique within its skill and nowhere else.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillVersion {
    /// `skill_version.skill_id`.
    pub skill_id: SkillId,
    /// `skill_version.version`, `>= 1`.
    pub version: i32,
    /// `skill_version.body`: the text inlined into the prompt's skills section verbatim.
    pub body: String,
    /// `skill_version.source` (ANA-22 §6 item 11): import provenance and the raw frontmatter.
    /// Never read by the prompt builder; `{}` for a version that was not imported.
    pub source: serde_json::Value,
    /// `skill_version.created_by`.
    pub created_by: UserId,
    /// `skill_version.created_at`.
    pub created_at: DateTime<Utc>,
}

/// A row of `skill_binding` (§5.6 as amended by ANA-22 §7.1): one attachment of a skill,
/// globally, to a project, or to one phase of it. `UNIQUE NULLS NOT DISTINCT (skill_id,
/// project_id, phase_id)` allows one attachment per skill per level, which is why [`resolve`] has
/// to pick one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillBinding {
    /// `skill_binding.id`.
    pub id: SkillBindingId,
    /// `skill_binding.skill_id`.
    pub skill_id: SkillId,
    /// `skill_binding.project_id`; `None` is a global attachment (every project).
    pub project_id: Option<ProjectId>,
    /// `skill_binding.phase_id`; `None` is the project (or global) level. `Some` requires
    /// `project_id` (`skill_binding_phase_needs_project`).
    pub phase_id: Option<PhaseId>,
    /// `skill_binding.pinned_version`; `None` follows the latest version.
    pub pinned_version: Option<i32>,
    /// `skill_binding.position`: ascending render order, `skill.name` bytes breaking the tie.
    pub position: i32,
    /// `skill_binding.activation`.
    pub activation: Activation,
    /// `skill_binding.globs`: the effective globs, non-empty when `activation` is `Glob`
    /// (`skill_binding_glob_needs_globs`). Nothing matches them before PRD milestone 5 (D86).
    pub globs: Vec<String>,
    /// `skill_binding.languages`: as authored, display only; the matcher reads `globs`.
    pub languages: Vec<String>,
    /// `skill_binding.updated_at`.
    pub updated_at: DateTime<Utc>,
}

impl SkillBinding {
    /// The `skill_version` this binding puts in force (`docs/ANA-5.md` §4.2): `pinned_version`
    /// when set, otherwise the highest `version` of the skill.
    ///
    /// `versions` may hold any skill's rows — rows of another skill are ignored — so a backend can
    /// pass one query's worth of `skill_version` and resolve every binding against it. `None` when
    /// the skill has no version row at all, and when `pinned_version` names one that does not
    /// exist: a pin that cannot be honoured renders nothing rather than quietly falling back to a
    /// body the binding did not ask for.
    #[must_use]
    pub fn version_in_force<'a>(&self, versions: &'a [SkillVersion]) -> Option<&'a SkillVersion> {
        let mut mine = versions.iter().filter(|v| v.skill_id == self.skill_id);
        match self.pinned_version {
            Some(pinned) => mine.find(|v| v.version == pinned),
            None => mine.max_by_key(|v| v.version),
        }
    }

    /// The level this attachment sits at, from its two nullable keys. A `(None, Some(_))` row is
    /// refused by `skill_binding_phase_needs_project`; were one ever read, it is `Global`, the
    /// same answer `PgStore`'s `WHERE b.project_id IS NULL` gives it.
    #[must_use]
    pub fn level(&self) -> SkillLevel {
        SkillBindingKey::of(self).level()
    }
}

/// D71: a skill name is 1-64 bytes of `[a-z0-9-]` with no leading, trailing or doubled hyphen
/// (ANA-22 §6 item 12, the Agent Skills rule). `true` when `name` may be stored.
///
/// Checked by the writers and the Skills view, not by a constraint, so a hand-written row still
/// loads; [`invalid_skill_name`](crate::store::invalid_skill_name) phrases the refusal.
#[must_use]
pub fn validate_name(name: &str) -> bool {
    (1..=64).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !name.starts_with('-')
        && !name.ends_with('-')
        && !name.contains("--")
}

/// Arguments of `WriteStore::create_skill` (D75, D77): the `skill` row **and** its version 1,
/// written together so no skill exists without a body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewSkill {
    /// `skill.id`, minted client-side as a UUIDv7.
    pub id: SkillId,
    /// `skill.name`; must pass [`validate_name`].
    pub name: String,
    /// `skill.description`, the picker's one-liner; may be empty.
    pub description: String,
    /// Version 1's `skill_version.body`; refused when blank (D77, OQ-20).
    pub body: String,
    /// Version 1's `skill_version.source`: `{}` from the Skills view; milestone 4's import fills it.
    pub source: serde_json::Value,
    /// `skill.created_by` and version 1's `created_by`.
    pub created_by: UserId,
}

/// Edit passed to `WriteStore::update_skill` (D76); `None` leaves the column.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillPatch {
    /// `skill.name`; must pass [`validate_name`] and be free (OQ-17: a rename is allowed).
    pub name: Option<String>,
    /// `skill.description`.
    pub description: Option<String>,
}

/// Arguments of `WriteStore::add_skill_version` (D75, D77); the version number is the store's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewSkillVersion {
    /// `skill_version.body`; refused when blank.
    pub body: String,
    /// `skill_version.source`, `{}` from the Skills view.
    pub source: serde_json::Value,
    /// `skill_version.created_by`.
    pub created_by: UserId,
}

/// The natural key of one attachment (D78): `UNIQUE NULLS NOT DISTINCT (skill_id, project_id,
/// phase_id)`. `project: None` is global; `phase: Some` needs `project: Some`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SkillBindingKey {
    /// `skill_binding.skill_id`.
    pub skill: SkillId,
    /// `skill_binding.project_id`.
    pub project: Option<ProjectId>,
    /// `skill_binding.phase_id`.
    pub phase: Option<PhaseId>,
}

impl SkillBindingKey {
    /// The key a stored row sits under.
    #[must_use]
    pub fn of(binding: &SkillBinding) -> Self {
        Self {
            skill: binding.skill_id,
            project: binding.project_id,
            phase: binding.phase_id,
        }
    }

    /// The level this key names, by [`SkillBinding::level`]'s rule.
    #[must_use]
    pub fn level(self) -> SkillLevel {
        match (self.project, self.phase) {
            (None, _) => SkillLevel::Global,
            (Some(_), None) => SkillLevel::Project,
            (Some(_), Some(_)) => SkillLevel::Phase,
        }
    }
}

/// What an attachment says (D75, D78): the editable columns, `globs` **as typed** — the writer
/// stores `canonical_globs(globs, languages)` and the normalised languages.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attachment {
    /// `skill_binding.pinned_version`; `None` follows the latest.
    pub pinned_version: Option<i32>,
    /// `skill_binding.position`, `>= 0`.
    pub position: i32,
    /// `skill_binding.activation`.
    pub activation: Activation,
    /// Typed globs: `<glob>` or `<repo>:<glob>` (D74).
    pub globs: Vec<String>,
    /// Typed language names (D73).
    pub languages: Vec<String>,
}

impl Attachment {
    /// A stored row's attachment, as D80's clone copies it: the stored `globs` passed as typed with
    /// the stored `languages`, which `canonical_globs` leaves unchanged (it is idempotent).
    #[must_use]
    pub fn of(binding: &SkillBinding) -> Self {
        Self {
            pinned_version: binding.pinned_version,
            position: binding.position,
            activation: binding.activation,
            globs: binding.globs.clone(),
            languages: binding.languages.clone(),
        }
    }
}

/// What `WriteStore::set_skill_binding` does to the row at its key (D75).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BindingChange {
    /// Insert (`expected: None`) or replace (`expected: Some(updated_at)`) the row.
    Attach(Attachment),
    /// Delete the row.
    Detach,
}

/// One step's candidate skill: the winning attachment of one skill, resolved to a version and a
/// body. Not a table.
///
/// This is what the prompt's skills section is built from, so it carries the joined `skill.name`
/// and `skill_version.body` rather than the ids to look them up by: the assembler is pure and
/// cannot reach a store.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoundSkill {
    /// `skill.id`, the key the most-specific-wins collapse is resolved on.
    pub skill_id: SkillId,
    /// `skill.name`, rendered as the `<skill name="..">` attribute and the order's tie-break.
    pub name: String,
    /// `skill_version.version` in force, rendered as `version="N"`. `None` when the winning
    /// attachment's pin names no version, or the skill has none: the skill then renders nothing
    /// and records `missing_version`, with no fallback to a broader attachment (plan D39, OQ-9).
    pub version: Option<i32>,
    /// `skill_binding.position` of the attachment that won.
    pub position: i32,
    /// `skill_version.body`, inlined verbatim; empty when `version` is `None`.
    pub body: String,
    /// The level of the attachment that won.
    pub level: SkillLevel,
    /// The winning attachment's activation, which [`select`] reads.
    pub activation: Activation,
    /// The winning attachment's globs, for PRD milestone 5's matcher (D86). Recorded nowhere yet.
    pub globs: Vec<String>,
}

impl BoundSkill {
    /// `R-SKL-2` as amended (ANA-22 §6 item 3): per `skill_id`, the candidate at the **most
    /// specific** level wins (phase over project over global) with its own pin, position and
    /// activation; the result is ordered by `(position, name bytes)`; and **each skill appears
    /// exactly once**.
    ///
    /// That last clause is the dedup OpenHands had to add after a reviewer asked "whether the
    /// microagent prompt would be included twice"
    /// (<https://github.com/OpenHands/OpenHands/pull/7516>).
    ///
    /// Pure, and the single definition of the order: every backend's reader and the assembler call
    /// this rather than each sorting its own way, because the skills section is protected from
    /// trimming and its bytes land in the prompt digest.
    ///
    /// Two candidates of one skill at one level cannot come from a store — `UNIQUE NULLS NOT
    /// DISTINCT (skill_id, project_id, phase_id)` and one project and at most one phase per read —
    /// but the assembler re-collapses a caller's list, so the tie is defined: the first in input
    /// order wins.
    #[must_use]
    pub fn collapse(mut skills: Vec<Self>) -> Vec<Self> {
        // Most specific first. `sort_by_key` is stable, so equal levels keep their input order.
        skills.sort_by_key(|skill| core::cmp::Reverse(skill.level));
        let mut resolved: Vec<Self> = Vec::with_capacity(skills.len());
        for skill in skills {
            if !resolved.iter().any(|kept| kept.skill_id == skill.skill_id) {
                resolved.push(skill);
            }
        }
        // `str`'s own `Ord` is byte order, and `as_bytes` says so where a reader would otherwise
        // have to remember it: Postgres would order these by collation.
        resolved.sort_by(|a, b| {
            a.position
                .cmp(&b.position)
                .then_with(|| a.name.as_bytes().cmp(b.name.as_bytes()))
        });
        resolved
    }
}

/// One step's candidates from its attachment rows (plan D39): each row, paired with its joined
/// `skill.name`, becomes a [`BoundSkill`] at its own level with the version its own pin puts in
/// force, and [`BoundSkill::collapse`] keeps the most specific per skill.
///
/// `rows` are every attachment that applies to the step — the global ones, the project's, and the
/// phase's — and `versions` may hold any skill's rows, as [`SkillBinding::version_in_force`]
/// allows. **The winner is picked by level before its version is looked at**: a winning pin that
/// names no version yields `version: None` rather than the broader attachment's body (OQ-9). Both
/// stores call this, so "which attachment wins" has one definition.
#[must_use]
pub fn resolve(rows: Vec<(SkillBinding, String)>, versions: &[SkillVersion]) -> Vec<BoundSkill> {
    let candidates = rows
        .into_iter()
        .map(|(binding, name)| {
            let in_force = binding.version_in_force(versions);
            BoundSkill {
                skill_id: binding.skill_id,
                name,
                version: in_force.map(|version| version.version),
                position: binding.position,
                body: in_force
                    .map(|version| version.body.clone())
                    .unwrap_or_default(),
                level: binding.level(),
                activation: binding.activation,
                globs: binding.globs,
            }
        })
        .collect();
    BoundSkill::collapse(candidates)
}

/// Why a candidate did or did not render (plan D40, ANA-22 §6 item 8). Serialised snake_case into
/// `trim_record.skill_choices[].reason`. PRD milestone 5 (D86) adds `matched` (with the path) and
/// `no_match`; no variant here is renamed then.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChoiceReason {
    /// `activation = always`: rendered. The only active reason.
    Always,
    /// `activation = off` on the winning attachment.
    Off,
    /// `activation = glob` and no repo root resolves for the step (every step, until PRD milestone
    /// 5, D86).
    NoPath,
    /// The winning attachment's pin names no version, or the skill has none.
    MissingVersion,
    /// The template body places no `{{skills}}`, so nothing could render.
    NotPlaced,
}

impl ChoiceReason {
    /// The serde spelling: `always`, `off`, `no_path`, `missing_version`, `not_placed`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Always => "always",
            Self::Off => "off",
            Self::NoPath => "no_path",
            Self::MissingVersion => "missing_version",
            Self::NotPlaced => "not_placed",
        }
    }
}

/// One candidate's line in `trim_record.skill_choices` (plan D42): what was attached, at which
/// level, and whether it rendered. `name` is the masked name — the assembler scrubs every
/// candidate before it selects (plan D43).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillChoice {
    /// `skill.id`.
    pub skill: SkillId,
    /// `skill.name`, masked.
    pub name: String,
    /// The version in force; `null` in the record when none is.
    pub version: Option<i32>,
    /// The winning attachment's level.
    pub level: SkillLevel,
    /// The winning attachment's activation.
    pub activation: Activation,
    /// Whether the skill rendered.
    pub active: bool,
    /// Why.
    pub reason: ChoiceReason,
}

/// Plan D40: decides every candidate, in order, for one step. Pure.
///
/// The rules, first match wins: a body that does not place `{{skills}}` (`placed == false`) makes
/// every candidate `not_placed`; `version: None` is `missing_version`; `Off` is `off`; `Glob` is
/// `no_path` (no step resolves a root before PRD milestone 5, D86; OQ-12); `Always` is `always`
/// and the only active outcome. Returns the active candidates in input order — which is collapse
/// order, the render order — and one [`SkillChoice`] per candidate in the same order.
#[must_use]
pub fn select(candidates: Vec<BoundSkill>, placed: bool) -> (Vec<BoundSkill>, Vec<SkillChoice>) {
    let mut active = Vec::with_capacity(candidates.len());
    let mut choices = Vec::with_capacity(candidates.len());
    for skill in candidates {
        let reason = if !placed {
            ChoiceReason::NotPlaced
        } else if skill.version.is_none() {
            ChoiceReason::MissingVersion
        } else {
            match skill.activation {
                Activation::Off => ChoiceReason::Off,
                Activation::Glob => ChoiceReason::NoPath,
                Activation::Always => ChoiceReason::Always,
            }
        };
        let is_active = reason == ChoiceReason::Always;
        choices.push(SkillChoice {
            skill: skill.skill_id,
            name: skill.name.clone(),
            version: skill.version,
            level: skill.level,
            activation: skill.activation,
            active: is_active,
            reason,
        });
        if is_active {
            active.push(skill);
        }
    }
    (active, choices)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at() -> DateTime<Utc> {
        DateTime::from_timestamp(1_788_393_600, 0).expect("a valid timestamp")
    }

    /// One resolved candidate, as a backend hands it to [`BoundSkill::collapse`]: `Always`, with a
    /// version and no globs.
    fn bound(
        skill_id: SkillId,
        name: &str,
        version: i32,
        position: i32,
        level: SkillLevel,
    ) -> BoundSkill {
        BoundSkill {
            skill_id,
            name: name.to_owned(),
            version: Some(version),
            position,
            body: format!("# {name} v{version}"),
            level,
            activation: Activation::Always,
            globs: Vec::new(),
        }
    }

    fn binding(skill_id: SkillId, pinned_version: Option<i32>) -> SkillBinding {
        SkillBinding {
            id: SkillBindingId::new(),
            skill_id,
            project_id: Some(ProjectId::new()),
            phase_id: None,
            pinned_version,
            position: 0,
            activation: Activation::Always,
            globs: Vec::new(),
            languages: Vec::new(),
            updated_at: at(),
        }
    }

    /// One attachment row as a store hands it to [`resolve`]: the binding and its joined name.
    fn attach(
        skill_id: SkillId,
        name: &str,
        keys: (Option<ProjectId>, Option<PhaseId>),
        pinned_version: Option<i32>,
        position: i32,
        activation: Activation,
    ) -> (SkillBinding, String) {
        let (project_id, phase_id) = keys;
        let globs = if activation == Activation::Glob {
            vec!["**/*.rs".to_owned()]
        } else {
            Vec::new()
        };
        (
            SkillBinding {
                id: SkillBindingId::new(),
                skill_id,
                project_id,
                phase_id,
                pinned_version,
                position,
                activation,
                globs,
                languages: Vec::new(),
                updated_at: at(),
            },
            name.to_owned(),
        )
    }

    fn version(skill_id: SkillId, version: i32) -> SkillVersion {
        SkillVersion {
            skill_id,
            version,
            body: format!("v{version}"),
            source: serde_json::json!({}),
            created_by: UserId::new(),
            created_at: at(),
        }
    }

    /// §4.2's dedup: a skill bound at both project and phase level appears **once**, at the phase
    /// binding's version and position. This is the bug OpenHands had to fix after a reviewer asked
    /// whether a microagent prompt would be included twice
    /// (<https://github.com/OpenHands/OpenHands/pull/7516>).
    #[test]
    fn phase_overrides_project_once() {
        let rust_style = SkillId::new();
        let tests = SkillId::new();
        let project = vec![
            bound(rust_style, "rust-style", 1, 0, SkillLevel::Project),
            bound(tests, "tests", 2, 1, SkillLevel::Project),
        ];
        let phase = vec![bound(rust_style, "rust-style", 3, 5, SkillLevel::Phase)];

        let resolved = BoundSkill::collapse([project, phase].concat());

        assert_eq!(
            resolved.iter().filter(|s| s.skill_id == rust_style).count(),
            1,
            "a skill bound at both levels is rendered exactly once"
        );
        assert_eq!(
            resolved,
            vec![
                bound(tests, "tests", 2, 1, SkillLevel::Project),
                bound(rust_style, "rust-style", 3, 5, SkillLevel::Phase),
            ],
            "the phase binding's version and position are the ones in force"
        );
    }

    /// Ties on `skill_binding.position` break on `skill.name` **byte** order, not locale order:
    /// an ICU collation would read these as `apple`, `Ångström`, `Zebra`.
    #[test]
    fn equal_positions_break_on_name_bytes() {
        let project = vec![
            bound(SkillId::new(), "apple", 1, 0, SkillLevel::Project),
            bound(SkillId::new(), "Ångström", 1, 0, SkillLevel::Project),
            bound(SkillId::new(), "Zebra", 1, 0, SkillLevel::Project),
        ];

        let names: Vec<String> = BoundSkill::collapse(project)
            .into_iter()
            .map(|s| s.name)
            .collect();

        assert_eq!(names, vec!["Zebra", "apple", "Ångström"]);
    }

    /// `pinned_version` when set, otherwise the highest `skill_version.version` (§4.2). A pin that
    /// names no row resolves to nothing rather than silently falling back to the latest, which
    /// would render a body the binding did not ask for.
    #[test]
    fn pinned_version_wins_over_latest() {
        let skill_id = SkillId::new();
        let other = SkillId::new();
        let versions = vec![
            version(skill_id, 1),
            version(skill_id, 3),
            version(skill_id, 2),
            version(other, 9),
        ];

        assert_eq!(
            binding(skill_id, Some(2))
                .version_in_force(&versions)
                .map(|v| v.version),
            Some(2),
            "the pin wins"
        );
        assert_eq!(
            binding(skill_id, None)
                .version_in_force(&versions)
                .map(|v| v.version),
            Some(3),
            "no pin follows the latest"
        );
        assert_eq!(
            binding(skill_id, Some(7)).version_in_force(&versions),
            None,
            "a pin naming no row resolves to nothing"
        );
        assert_eq!(
            binding(SkillId::new(), None).version_in_force(&versions),
            None,
            "another skill's rows are not this skill's versions"
        );
    }

    /// ANA-22 §6 item 3: phase over project over global, each level with its own pin and
    /// position. Dropping the most specific row hands the skill to the next level down.
    #[test]
    fn phase_beats_project_beats_global() {
        let skill = SkillId::new();
        let project = ProjectId::new();
        let phase = PhaseId::new();
        let versions: Vec<SkillVersion> = (1..=3).map(|n| version(skill, n)).collect();
        let global_row = attach(skill, "house", (None, None), Some(1), 7, Activation::Always);
        let project_row = attach(
            skill,
            "house",
            (Some(project), None),
            Some(2),
            5,
            Activation::Always,
        );
        let phase_row = attach(
            skill,
            "house",
            (Some(project), Some(phase)),
            Some(3),
            2,
            Activation::Always,
        );
        let summary = |rows: Vec<(SkillBinding, String)>| {
            resolve(rows, &versions)
                .into_iter()
                .map(|s| (s.level, s.version, s.position, s.body))
                .collect::<Vec<_>>()
        };

        assert_eq!(
            summary(vec![
                global_row.clone(),
                project_row.clone(),
                phase_row.clone()
            ]),
            vec![(SkillLevel::Phase, Some(3), 2, "v3".to_owned())],
            "the phase attachment wins, with its own pin and position"
        );
        assert_eq!(
            summary(vec![phase_row, global_row.clone(), project_row.clone()]),
            vec![(SkillLevel::Phase, Some(3), 2, "v3".to_owned())],
            "input order does not pick the winner; the level does"
        );
        assert_eq!(
            summary(vec![global_row.clone(), project_row]),
            vec![(SkillLevel::Project, Some(2), 5, "v2".to_owned())],
            "without a phase row the project attachment wins"
        );
        assert_eq!(
            summary(vec![global_row]),
            vec![(SkillLevel::Global, Some(1), 7, "v1".to_owned())],
            "without either, the global attachment stands"
        );
    }

    /// ANA-22 §6 item 4: a narrower `off` wins like any other attachment, so the skill is out of
    /// the step even though a broader level would have rendered it.
    #[test]
    fn off_at_a_narrower_level_wins_and_keeps_the_skill_out_of_broader_levels() {
        let skill = SkillId::new();
        let project = ProjectId::new();
        let versions = vec![version(skill, 1)];
        let candidates = resolve(
            vec![
                attach(skill, "house", (None, None), None, 0, Activation::Always),
                attach(
                    skill,
                    "house",
                    (Some(project), None),
                    None,
                    0,
                    Activation::Off,
                ),
            ],
            &versions,
        );
        assert_eq!(candidates.len(), 1, "one candidate per skill");
        assert_eq!(
            (candidates[0].level, candidates[0].activation),
            (SkillLevel::Project, Activation::Off),
            "the project's off wins over the global always"
        );

        let (active, choices) = select(candidates, true);
        assert!(active.is_empty(), "an off winner renders nothing");
        assert_eq!(
            choices
                .iter()
                .map(|c| (c.active, c.reason, c.level))
                .collect::<Vec<_>>(),
            vec![(false, ChoiceReason::Off, SkillLevel::Project)],
            "and is recorded off, at the level that switched it off"
        );
    }

    /// OQ-9 (plan D39): the winner is chosen by level first; a winning pin that names no version
    /// yields `version: None`, never the broader attachment's body.
    #[test]
    fn a_missing_pin_on_the_winner_does_not_fall_back() {
        let skill = SkillId::new();
        let project = ProjectId::new();
        let phase = PhaseId::new();
        let versions = vec![version(skill, 1)];
        let resolved = resolve(
            vec![
                attach(
                    skill,
                    "house",
                    (Some(project), None),
                    Some(1),
                    0,
                    Activation::Always,
                ),
                attach(
                    skill,
                    "house",
                    (Some(project), Some(phase)),
                    Some(7),
                    4,
                    Activation::Always,
                ),
            ],
            &versions,
        );
        assert_eq!(
            resolved
                .iter()
                .map(|s| (s.level, s.version, s.body.as_str(), s.position))
                .collect::<Vec<_>>(),
            vec![(SkillLevel::Phase, None, "", 4)],
            "the phase attachment wins with no version, not the project's v1"
        );
    }

    /// Global rows order like any other: `(position, name bytes)`, a negative position first.
    #[test]
    fn global_rows_order_by_position_then_name_bytes() {
        let ids: Vec<SkillId> = (0..4).map(|_| SkillId::new()).collect();
        let versions: Vec<SkillVersion> = ids.iter().map(|id| version(*id, 1)).collect();
        let rows = vec![
            attach(ids[0], "apple", (None, None), None, 0, Activation::Always),
            attach(
                ids[1],
                "Ångström",
                (None, None),
                None,
                0,
                Activation::Always,
            ),
            attach(ids[2], "Zebra", (None, None), None, 0, Activation::Always),
            attach(ids[3], "last", (None, None), None, -1, Activation::Always),
        ];
        let names: Vec<String> = resolve(rows, &versions)
            .into_iter()
            .map(|s| s.name)
            .collect();
        assert_eq!(names, vec!["last", "Zebra", "apple", "Ångström"]);
    }

    /// `level` reads the two nullable keys; the row the check constraint refuses reads `Global`,
    /// as `PgStore`'s `WHERE b.project_id IS NULL` would.
    #[test]
    fn level_of_a_binding_follows_its_nullable_keys() {
        let level = |project_id: Option<ProjectId>, phase_id: Option<PhaseId>| {
            SkillBinding {
                project_id,
                phase_id,
                ..binding(SkillId::new(), None)
            }
            .level()
        };
        let project = Some(ProjectId::new());
        let phase = Some(PhaseId::new());
        assert_eq!(level(None, None), SkillLevel::Global);
        assert_eq!(level(project, None), SkillLevel::Project);
        assert_eq!(level(project, phase), SkillLevel::Phase);
        assert_eq!(
            level(None, phase),
            SkillLevel::Global,
            "a phase row with no project is refused by skill_binding_phase_needs_project; were \
             one read, it is global"
        );
    }

    /// Plan D40: `not_placed` first, then `missing_version`, then the activation.
    #[test]
    fn select_applies_its_rules_in_order() {
        let with = |name: &str, activation: Activation, version: Option<i32>| BoundSkill {
            activation,
            version,
            ..bound(SkillId::new(), name, 1, 0, SkillLevel::Project)
        };
        let candidates = vec![
            with("a", Activation::Always, Some(1)),
            with("b", Activation::Off, Some(1)),
            with("c", Activation::Glob, Some(1)),
            with("d", Activation::Always, None),
        ];

        let (active, choices) = select(candidates.clone(), true);
        assert_eq!(
            choices.iter().map(|c| c.reason).collect::<Vec<_>>(),
            vec![
                ChoiceReason::Always,
                ChoiceReason::Off,
                ChoiceReason::NoPath,
                ChoiceReason::MissingVersion,
            ]
        );
        assert_eq!(
            choices.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
            vec!["a", "b", "c", "d"],
            "one choice per candidate, in input order"
        );
        assert_eq!(
            choices.iter().map(|c| c.active).collect::<Vec<_>>(),
            vec![true, false, false, false]
        );
        assert_eq!(active, vec![candidates[0].clone()], "only `always` renders");

        let (active, choices) = select(candidates, false);
        assert!(active.is_empty(), "nothing renders where nothing is placed");
        assert!(
            choices
                .iter()
                .all(|c| c.reason == ChoiceReason::NotPlaced && !c.active),
            "every candidate is not_placed: {choices:?}"
        );
    }

    /// Plan D55: the record's keys, and the snake_case spellings of the two plain enums.
    #[test]
    fn choices_serialize_their_documented_keys() {
        let choice = SkillChoice {
            skill: SkillId::new(),
            name: "house".to_owned(),
            version: None,
            level: SkillLevel::Phase,
            activation: Activation::Glob,
            active: false,
            reason: ChoiceReason::MissingVersion,
        };
        let value = serde_json::to_value(&choice).expect("a choice serialises");
        let keys: std::collections::BTreeSet<&str> = value
            .as_object()
            .expect("an object")
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            keys,
            [
                "skill",
                "name",
                "version",
                "level",
                "activation",
                "active",
                "reason"
            ]
            .into_iter()
            .collect(),
        );
        assert_eq!(value["reason"], serde_json::json!("missing_version"));
        assert_eq!(value["level"], serde_json::json!("phase"));
        assert_eq!(value["activation"], serde_json::json!("glob"));
        assert_eq!(value["version"], serde_json::Value::Null);
        for reason in [
            ChoiceReason::Always,
            ChoiceReason::Off,
            ChoiceReason::NoPath,
            ChoiceReason::MissingVersion,
            ChoiceReason::NotPlaced,
        ] {
            assert_eq!(
                serde_json::to_value(reason).expect("a reason serialises"),
                serde_json::json!(reason.as_str()),
                "as_str is the serde spelling"
            );
        }
        for level in [SkillLevel::Global, SkillLevel::Project, SkillLevel::Phase] {
            assert_eq!(
                serde_json::to_value(level).expect("a level serialises"),
                serde_json::json!(level.as_str()),
                "as_str is the serde spelling"
            );
        }
        assert!(
            SkillLevel::Global < SkillLevel::Project && SkillLevel::Project < SkillLevel::Phase
        );
    }

    /// D71: the Agent Skills rule, byte for byte; the demo names pass.
    #[test]
    fn skill_names_follow_the_agent_skills_rule() {
        for name in ["rust-style", "tests", "a", "a1-b2", &"a".repeat(64)] {
            assert!(validate_name(name), "`{name}` may be stored");
        }
        for name in [
            "",
            &"a".repeat(65),
            "Rust",
            "-a",
            "a-",
            "a--b",
            "a_b",
            "a b",
            "ä",
        ] {
            assert!(!validate_name(name), "`{name}` is refused");
        }
    }

    /// D80's copy reads a stored row back as a key and an attachment; nothing is lost.
    #[test]
    fn a_key_and_an_attachment_copy_a_row() {
        let project = ProjectId::new();
        let phase = PhaseId::new();
        let row = SkillBinding {
            project_id: Some(project),
            phase_id: Some(phase),
            pinned_version: Some(2),
            position: 3,
            activation: Activation::Glob,
            globs: vec!["htui:src/**".to_owned(), "**/*.rs".to_owned()],
            languages: vec!["rust".to_owned()],
            ..binding(SkillId::new(), None)
        };

        let key = SkillBindingKey::of(&row);
        assert_eq!(
            key,
            SkillBindingKey {
                skill: row.skill_id,
                project: Some(project),
                phase: Some(phase),
            }
        );
        assert_eq!(key.level(), SkillLevel::Phase);
        assert_eq!(key.level(), row.level(), "one rule for both");
        assert_eq!(
            SkillBindingKey { phase: None, ..key }.level(),
            SkillLevel::Project
        );
        assert_eq!(
            SkillBindingKey {
                project: None,
                phase: None,
                ..key
            }
            .level(),
            SkillLevel::Global
        );

        assert_eq!(
            Attachment::of(&row),
            Attachment {
                pinned_version: Some(2),
                position: 3,
                activation: Activation::Glob,
                globs: row.globs.clone(),
                languages: row.languages.clone(),
            }
        );
    }
}
