//! Skills, their versions and their bindings (`docs/ANA-9.md` §5.6), plus the `R-SKL-2`
//! resolution the prompt's skills section renders (`docs/ANA-5.md` §4.2).
//!
//! Read-only in this milestone (plan D105): the three row types and the collapse land here so the
//! assembler and every backend share one definition of "which skill, at which version, in which
//! order"; `upsert_skill`, `add_skill_version` and `set_skill_binding` are MOD-9's and are
//! deliberately absent.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::model::ids::{PhaseId, ProjectId, SkillBindingId, SkillId, UserId};

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
    /// `skill_version.created_by`.
    pub created_by: UserId,
    /// `skill_version.created_at`.
    pub created_at: DateTime<Utc>,
}

/// A row of `skill_binding` (§5.6): a skill attached to a project, or to one phase of it.
///
/// `phase_id: None` is the project level. `UNIQUE NULLS NOT DISTINCT (skill_id, project_id,
/// phase_id)` is why a project and a phase binding of the same skill can coexist, and why
/// [`BoundSkill::collapse`] has to resolve them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillBinding {
    /// `skill_binding.id`.
    pub id: SkillBindingId,
    /// `skill_binding.skill_id`.
    pub skill_id: SkillId,
    /// `skill_binding.project_id`.
    pub project_id: ProjectId,
    /// `skill_binding.phase_id`; `None` is the project level.
    pub phase_id: Option<PhaseId>,
    /// `skill_binding.pinned_version`; `None` follows the latest version.
    pub pinned_version: Option<i32>,
    /// `skill_binding.position`: ascending render order, `skill.name` bytes breaking the tie.
    pub position: i32,
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
}

/// The `R-SKL-2` resolution of one binding: the skill, the version in force, the position it
/// renders at, and the body. Not a table.
///
/// This is what the prompt's skills section is built from, so it carries the joined `skill.name`
/// and `skill_version.body` rather than the ids to look them up by: the assembler is pure and
/// cannot reach a store.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoundSkill {
    /// `skill.id`, the key the project/phase override is resolved on.
    pub skill_id: SkillId,
    /// `skill.name`, rendered as the `<skill name="..">` attribute and the order's tie-break.
    pub name: String,
    /// `skill_version.version` in force, rendered as the `version="N"` attribute.
    pub version: i32,
    /// `skill_binding.position` of the binding that won.
    pub position: i32,
    /// `skill_version.body`, inlined verbatim.
    pub body: String,
}

impl BoundSkill {
    /// `docs/ANA-5.md` §4.2's collapse: a phase binding overrides the project binding of the same
    /// `skill_id`, the result is ordered by `(position, name bytes)`, and **each skill appears
    /// exactly once** even when both bindings exist.
    ///
    /// That last clause is the dedup OpenHands had to add after a reviewer asked "whether the
    /// microagent prompt would be included twice"
    /// (<https://github.com/OpenHands/OpenHands/pull/7516>).
    ///
    /// Pure, and the single definition of the order: every backend's reader and the assembler call
    /// this rather than each sorting its own way, because the skills section is protected from
    /// trimming and its bytes land in the prompt digest.
    #[must_use]
    pub fn collapse(project: Vec<Self>, phase: Vec<Self>) -> Vec<Self> {
        let mut resolved: Vec<Self> = Vec::with_capacity(project.len() + phase.len());
        for skill in phase.into_iter().chain(project) {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn at() -> DateTime<Utc> {
        DateTime::from_timestamp(1_788_393_600, 0).expect("a valid timestamp")
    }

    /// One resolved binding, as a backend hands it to [`BoundSkill::collapse`].
    fn bound(skill_id: SkillId, name: &str, version: i32, position: i32) -> BoundSkill {
        BoundSkill {
            skill_id,
            name: name.to_owned(),
            version,
            position,
            body: format!("# {name} v{version}"),
        }
    }

    fn binding(skill_id: SkillId, pinned_version: Option<i32>) -> SkillBinding {
        SkillBinding {
            id: SkillBindingId::new(),
            skill_id,
            project_id: ProjectId::new(),
            phase_id: None,
            pinned_version,
            position: 0,
            updated_at: at(),
        }
    }

    fn version(skill_id: SkillId, version: i32) -> SkillVersion {
        SkillVersion {
            skill_id,
            version,
            body: format!("v{version}"),
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
            bound(rust_style, "rust-style", 1, 0),
            bound(tests, "tests", 2, 1),
        ];
        let phase = vec![bound(rust_style, "rust-style", 3, 5)];

        let resolved = BoundSkill::collapse(project, phase);

        assert_eq!(
            resolved.iter().filter(|s| s.skill_id == rust_style).count(),
            1,
            "a skill bound at both levels is rendered exactly once"
        );
        assert_eq!(
            resolved,
            vec![
                bound(tests, "tests", 2, 1),
                bound(rust_style, "rust-style", 3, 5),
            ],
            "the phase binding's version and position are the ones in force"
        );
    }

    /// Ties on `skill_binding.position` break on `skill.name` **byte** order, not locale order:
    /// an ICU collation would read these as `apple`, `Ångström`, `Zebra`.
    #[test]
    fn equal_positions_break_on_name_bytes() {
        let project = vec![
            bound(SkillId::new(), "apple", 1, 0),
            bound(SkillId::new(), "Ångström", 1, 0),
            bound(SkillId::new(), "Zebra", 1, 0),
        ];

        let names: Vec<String> = BoundSkill::collapse(project, Vec::new())
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
}
