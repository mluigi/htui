//! MOD-138 M2 D3 (ANA-33 §7.3): the closed `if` set, parsed from the opaque
//! [`EdgePredicate`](crate::model::EdgePredicate) value. The value keeps its type, so no column,
//! serde or digest moves; this module only reads it.
//!
//! A predicate is a JSON object with exactly one key:
//!
//! - `{"all_changed_paths_match": ["<glob>", ..]}` and `{"any_changed_path_matches": [..]}`: globs
//!   in `repo:glob` form ([`SkillGlob`]'s syntax); a bare glob reads the **primary** repo (F12),
//!   unlike an attachment glob, which reads any;
//! - `{"verify": "passed" | "failed"}`;
//! - `{"item_kind": "<key_prefix>"}` and `{"item_tag": "<tag>"}` (`item.required_tags`, V13);
//! - `{"all": [..]}`, `{"any": [..]}` and `{"not": <predicate>}`.

use serde_json::Value;

use crate::model::ItemKind;
use crate::model::run::VerifyOutcome;
use crate::model::skill_glob::{self, GlobError, SkillGlob};

/// How deep a predicate may nest; the outermost object is depth 1.
pub const MAX_DEPTH: usize = 32;

/// One `repo:glob` (§4.7): `repo: None` is a bare glob, read against the **primary** repo (F12).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathGlob {
    /// The repo qualifier; `None` for a bare glob.
    pub repo: Option<String>,
    /// The glob proper, already accepted by [`SkillGlob::parse`].
    pub glob: String,
}

/// What `{"verify": ..}` asks of the step's `verify_outcome`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifyWant {
    /// `"passed"`: `verify_outcome = pass`.
    Passed,
    /// `"failed"`: `verify_outcome = fail`.
    Failed,
}

/// One predicate of the closed `if` set (D3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IfPredicate {
    /// `{"all_changed_paths_match": [..]}`: every changed path matches some glob.
    AllChangedPathsMatch(Vec<PathGlob>),
    /// `{"any_changed_path_matches": [..]}`: some changed path matches some glob.
    AnyChangedPathMatches(Vec<PathGlob>),
    /// `{"verify": "passed" | "failed"}`.
    Verify(VerifyWant),
    /// `{"item_kind": "<key_prefix>"}`.
    ItemKind(String),
    /// `{"item_tag": "<tag>"}`.
    ItemTag(String),
    /// `{"all": [..]}`: every member holds.
    All(Vec<IfPredicate>),
    /// `{"any": [..]}`: some member holds.
    Any(Vec<IfPredicate>),
    /// `{"not": <predicate>}`.
    Not(Box<IfPredicate>),
}

/// What a decision reads (Q-A). Paths are `(repo name, repo-relative path)`, in the run's scope.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BranchFacts {
    /// The winner's changed paths, `(repo name, path)`.
    pub paths: Vec<(String, String)>,
    /// F12: the primary repo's name when it is in scope; a bare glob matches only there.
    pub primary: Option<String>,
    /// The path list was cut short, so an `all_…` predicate cannot be proved.
    pub truncated: bool,
    /// The winner's `verify_outcome`.
    pub verify: Option<VerifyOutcome>,
    /// `item.key_prefix`.
    pub item_prefix: String,
    /// `item.required_tags` (V13).
    pub item_tags: Vec<String>,
}

/// Why a JSON value is not a predicate of the closed set.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PredicateError {
    /// Not an object, or an object with no key or more than one.
    #[error("a predicate is a JSON object with exactly one key")]
    NotOneKey,
    /// A key outside the closed set.
    #[error(
        "`{0}` is not in the closed set (all_changed_paths_match, any_changed_path_matches, \
         verify, item_kind, item_tag, all, any, not)"
    )]
    Unknown(String),
    /// A known key whose value has the wrong shape.
    #[error("`{key}` takes {want}")]
    BadValue {
        /// The predicate key.
        key: &'static str,
        /// What it takes, in words.
        want: &'static str,
    },
    /// A glob [`SkillGlob::parse`] refuses.
    #[error(transparent)]
    Glob(#[from] GlobError),
    /// Nesting past [`MAX_DEPTH`].
    #[error("a predicate nests deeper than {}", MAX_DEPTH)]
    TooDeep,
}

const GLOBS: &str = "a non-empty array of globs";
const STRING: &str = "a non-empty string";
const PREDICATES: &str = "a non-empty array of predicates";
const VERIFY: &str = "\"passed\" or \"failed\"";
/// R1 (S3): the `item_kind.prefix` CHECK ([`invalid_prefix`](crate::store::invalid_prefix)'s
/// rule, [`ItemKind::prefix_is_valid`]), so a predicate can only name a prefix a kind can hold.
const PREFIX_RULE: &str = "an item-kind prefix matching `^[A-Z][A-Z0-9]{1,15}$`";
/// R1 (S3): a tag a human typed is matched by bytes, so blank or padded text is refused.
const TAG_RULE: &str = "a non-blank tag with no leading or trailing whitespace";

impl IfPredicate {
    /// Parses one predicate. Every glob list must be non-empty, as must `all` and `any`;
    /// `item_kind` and `item_tag` take non-empty strings, `item_kind` one matching the item-kind
    /// prefix rule `^[A-Z][A-Z0-9]{1,15}$` ([`ItemKind::prefix_is_valid`]) and `item_tag` one that
    /// is non-blank and equal to its trim (R1, S3); each glob is trimmed and read by [`SkillGlob::parse`].
    ///
    /// # Errors
    /// [`PredicateError`]: the first refusal, depth first, in array order.
    pub fn parse(value: &Value) -> Result<Self, PredicateError> {
        Self::parse_at(value, 1)
    }

    fn parse_at(value: &Value, depth: usize) -> Result<Self, PredicateError> {
        if depth > MAX_DEPTH {
            return Err(PredicateError::TooDeep);
        }
        let Some((key, value)) = value
            .as_object()
            .filter(|object| object.len() == 1)
            .and_then(|object| object.iter().next())
        else {
            return Err(PredicateError::NotOneKey);
        };
        let text = |key: &'static str| {
            value
                .as_str()
                .filter(|text| !text.is_empty())
                .map(str::to_owned)
                .ok_or(PredicateError::BadValue { key, want: STRING })
        };
        let members = |key: &'static str| {
            value
                .as_array()
                .filter(|members| !members.is_empty())
                .ok_or(PredicateError::BadValue {
                    key,
                    want: PREDICATES,
                })?
                .iter()
                .map(|member| Self::parse_at(member, depth + 1))
                .collect::<Result<Vec<_>, _>>()
        };
        Ok(match key.as_str() {
            "all_changed_paths_match" => {
                Self::AllChangedPathsMatch(globs("all_changed_paths_match", value)?)
            }
            "any_changed_path_matches" => {
                Self::AnyChangedPathMatches(globs("any_changed_path_matches", value)?)
            }
            "verify" => Self::Verify(match value.as_str() {
                Some("passed") => VerifyWant::Passed,
                Some("failed") => VerifyWant::Failed,
                _ => {
                    return Err(PredicateError::BadValue {
                        key: "verify",
                        want: VERIFY,
                    });
                }
            }),
            "item_kind" => {
                let prefix = text("item_kind")?;
                if !ItemKind::prefix_is_valid(&prefix) {
                    return Err(PredicateError::BadValue {
                        key: "item_kind",
                        want: PREFIX_RULE,
                    });
                }
                Self::ItemKind(prefix)
            }
            "item_tag" => {
                let tag = text("item_tag")?;
                if tag.trim().is_empty() || tag.trim() != tag {
                    return Err(PredicateError::BadValue {
                        key: "item_tag",
                        want: TAG_RULE,
                    });
                }
                Self::ItemTag(tag)
            }
            "all" => Self::All(members("all")?),
            "any" => Self::Any(members("any")?),
            "not" => Self::Not(Box::new(Self::parse_at(value, depth + 1)?)),
            other => return Err(PredicateError::Unknown(other.to_owned())),
        })
    }

    /// D3 over `facts`:
    /// - `all_changed_paths_match`: `paths` is not empty, `truncated` is false, and every path is
    ///   matched by some glob in that path's repo (no vacuous truth);
    /// - `any_changed_path_matches`: some listed path is matched;
    /// - a glob matches in its own repo, a bare one in `primary` (in none without one), on raw
    ///   bytes with no case folding;
    /// - `verify`: `passed` is `Some(Pass)`, `failed` is `Some(Fail)`; `unavailable` or none
    ///   matches neither;
    /// - `item_kind` compares bytes with `item_prefix`; `item_tag` is membership in `item_tags`.
    #[must_use]
    pub fn evaluate(&self, facts: &BranchFacts) -> bool {
        match self {
            Self::AllChangedPathsMatch(globs) => {
                let compiled = Compiled::new(globs, facts);
                !facts.truncated
                    && !facts.paths.is_empty()
                    && facts
                        .paths
                        .iter()
                        .all(|(repo, path)| compiled.matches(repo, path))
            }
            Self::AnyChangedPathMatches(globs) => {
                let compiled = Compiled::new(globs, facts);
                facts
                    .paths
                    .iter()
                    .any(|(repo, path)| compiled.matches(repo, path))
            }
            Self::Verify(want) => {
                let wanted = match want {
                    VerifyWant::Passed => VerifyOutcome::Pass,
                    VerifyWant::Failed => VerifyOutcome::Fail,
                };
                facts.verify == Some(wanted)
            }
            Self::ItemKind(prefix) => facts.item_prefix.as_bytes() == prefix.as_bytes(),
            Self::ItemTag(tag) => facts.item_tags.contains(tag),
            Self::All(members) => members.iter().all(|member| member.evaluate(facts)),
            Self::Any(members) => members.iter().any(|member| member.evaluate(facts)),
            Self::Not(inner) => !inner.evaluate(facts),
        }
    }

    /// Whether any leaf is a path predicate, so whether `changed_paths` must be read (F13).
    #[must_use]
    pub fn needs_paths(&self) -> bool {
        match self {
            Self::AllChangedPathsMatch(_) | Self::AnyChangedPathMatches(_) => true,
            Self::Verify(_) | Self::ItemKind(_) | Self::ItemTag(_) => false,
            Self::All(members) | Self::Any(members) => members.iter().any(Self::needs_paths),
            Self::Not(inner) => inner.needs_paths(),
        }
    }
}

/// A path predicate's glob list: a non-empty array of strings, each trimmed and parsed.
fn globs(key: &'static str, value: &Value) -> Result<Vec<PathGlob>, PredicateError> {
    let bad = PredicateError::BadValue { key, want: GLOBS };
    let entries = value
        .as_array()
        .filter(|entries| !entries.is_empty())
        .ok_or_else(|| bad.clone())?;
    entries
        .iter()
        .map(|entry| {
            let text = entry
                .as_str()
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .ok_or_else(|| bad.clone())?;
            let SkillGlob { repo, glob } = SkillGlob::parse(text)?;
            Ok(PathGlob { repo, glob })
        })
        .collect()
}

/// One evaluation's globs, each compiled once, with the repo it reads.
struct Compiled<'a> {
    globs: Vec<(Option<&'a str>, globset::GlobMatcher)>,
}

impl<'a> Compiled<'a> {
    fn new(globs: &'a [PathGlob], facts: &'a BranchFacts) -> Self {
        Self {
            globs: globs
                .iter()
                // `parse` accepted every glob, so a refusal here cannot happen; one is skipped,
                // never matched.
                .filter_map(|glob| {
                    let compiled = skill_glob::matcher(&glob.glob).ok()?;
                    Some((glob.repo.as_deref().or(facts.primary.as_deref()), compiled))
                })
                .collect(),
        }
    }

    fn matches(&self, repo: &str, path: &str) -> bool {
        let candidate = globset::Candidate::from_bytes(path.as_bytes());
        self.globs.iter().any(|(reads, compiled)| {
            *reads == Some(repo) && compiled.is_match_candidate(&candidate)
        })
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{
        BranchFacts, IfPredicate, MAX_DEPTH, PREFIX_RULE, PathGlob, PredicateError, TAG_RULE,
        VerifyWant,
    };
    use crate::model::VerifyOutcome;
    use crate::model::skill_glob::GlobError;

    fn parse(value: serde_json::Value) -> IfPredicate {
        IfPredicate::parse(&value).unwrap_or_else(|err| panic!("{value} parses: {err}"))
    }

    fn refused(value: serde_json::Value) -> PredicateError {
        IfPredicate::parse(&value).expect_err("refused")
    }

    fn path(repo: &str, path: &str) -> (String, String) {
        (repo.to_owned(), path.to_owned())
    }

    /// Facts with `htui` primary and the given changed paths.
    fn facts(paths: &[(&str, &str)]) -> BranchFacts {
        BranchFacts {
            paths: paths.iter().map(|&(repo, p)| path(repo, p)).collect(),
            primary: Some("htui".to_owned()),
            truncated: false,
            verify: None,
            item_prefix: "HTUI".to_owned(),
            item_tags: Vec::new(),
        }
    }

    #[test]
    fn parse_accepts_each_closed_form() {
        assert_eq!(
            parse(json!({"all_changed_paths_match": [" docs/** ", "web:src/*.ts"]})),
            IfPredicate::AllChangedPathsMatch(vec![
                PathGlob {
                    repo: None,
                    glob: "docs/**".to_owned()
                },
                PathGlob {
                    repo: Some("web".to_owned()),
                    glob: "src/*.ts".to_owned()
                },
            ])
        );
        assert_eq!(
            parse(json!({"any_changed_path_matches": ["**/*.rs"]})),
            IfPredicate::AnyChangedPathMatches(vec![PathGlob {
                repo: None,
                glob: "**/*.rs".to_owned()
            }])
        );
        assert_eq!(
            parse(json!({"verify": "passed"})),
            IfPredicate::Verify(VerifyWant::Passed)
        );
        assert_eq!(
            parse(json!({"verify": "failed"})),
            IfPredicate::Verify(VerifyWant::Failed)
        );
        assert_eq!(
            parse(json!({"item_kind": "HTUI"})),
            IfPredicate::ItemKind("HTUI".to_owned())
        );
        assert_eq!(
            parse(json!({"item_tag": "gpu"})),
            IfPredicate::ItemTag("gpu".to_owned())
        );
        assert_eq!(
            parse(json!({"item_kind": "A1234567890BCDEF"})),
            IfPredicate::ItemKind("A1234567890BCDEF".to_owned()),
            "sixteen characters, the prefix rule's longest"
        );
        assert_eq!(
            parse(json!({"item_tag": "needs gpu"})),
            IfPredicate::ItemTag("needs gpu".to_owned()),
            "inner whitespace is kept"
        );
        assert_eq!(
            parse(json!({"all": [{"verify": "passed"}, {"not": {"item_tag": "gpu"}}]})),
            IfPredicate::All(vec![
                IfPredicate::Verify(VerifyWant::Passed),
                IfPredicate::Not(Box::new(IfPredicate::ItemTag("gpu".to_owned()))),
            ])
        );
        assert_eq!(
            parse(json!({"any": [{"item_kind": "BUG"}]})),
            IfPredicate::Any(vec![IfPredicate::ItemKind("BUG".to_owned())])
        );
    }

    #[test]
    fn parse_refuses_unknown_keys_and_shapes() {
        assert_eq!(
            refused(json!({"verdict": "approve"})),
            PredicateError::Unknown("verdict".to_owned())
        );
        assert_eq!(refused(json!({})), PredicateError::NotOneKey);
        assert_eq!(refused(json!(["verify"])), PredicateError::NotOneKey);
        assert_eq!(refused(json!("verify")), PredicateError::NotOneKey);
        assert_eq!(
            refused(json!({"verify": "passed", "item_kind": "A"})),
            PredicateError::NotOneKey
        );
        for (value, key, want) in [
            (
                json!({"verify": "pass"}),
                "verify",
                "\"passed\" or \"failed\"",
            ),
            (
                json!({"verify": true}),
                "verify",
                "\"passed\" or \"failed\"",
            ),
            (json!({"item_kind": ""}), "item_kind", "a non-empty string"),
            (json!({"item_tag": 3}), "item_tag", "a non-empty string"),
            // R1 (S3): `item_kind` is an item-kind prefix, `^[A-Z][A-Z0-9]{1,15}$` (the
            // `item_kind.prefix` CHECK, `invalid_prefix`); `item_tag` is non-blank and trimmed.
            (json!({"item_kind": "A"}), "item_kind", PREFIX_RULE),
            (json!({"item_kind": "htui"}), "item_kind", PREFIX_RULE),
            (json!({"item_kind": " HTUI"}), "item_kind", PREFIX_RULE),
            (json!({"item_kind": "1HTUI"}), "item_kind", PREFIX_RULE),
            (json!({"item_kind": "HTUI-X"}), "item_kind", PREFIX_RULE),
            (
                json!({"item_kind": "ABCDEFGHIJKLMNOPQ"}),
                "item_kind",
                PREFIX_RULE,
            ),
            (json!({"item_tag": "  "}), "item_tag", TAG_RULE),
            (json!({"item_tag": " gpu"}), "item_tag", TAG_RULE),
            (json!({"item_tag": "gpu\n"}), "item_tag", TAG_RULE),
            (
                json!({"all_changed_paths_match": []}),
                "all_changed_paths_match",
                "a non-empty array of globs",
            ),
            (
                json!({"any_changed_path_matches": ["a", 1]}),
                "any_changed_path_matches",
                "a non-empty array of globs",
            ),
            (
                json!({"any_changed_path_matches": ["  "]}),
                "any_changed_path_matches",
                "a non-empty array of globs",
            ),
            (
                json!({"any_changed_path_matches": "a"}),
                "any_changed_path_matches",
                "a non-empty array of globs",
            ),
            (json!({"all": []}), "all", "a non-empty array of predicates"),
            (json!({"any": {}}), "any", "a non-empty array of predicates"),
        ] {
            assert_eq!(
                refused(value.clone()),
                PredicateError::BadValue { key, want },
                "{value}"
            );
        }
        assert!(matches!(
            refused(json!({"any_changed_path_matches": ["src/[a"]})),
            PredicateError::Glob(GlobError::Invalid { .. })
        ));
        assert_eq!(
            refused(json!({"all": [{"verify": "passed"}, {"verdict": 1}, {"nope": 2}]})),
            PredicateError::Unknown("verdict".to_owned()),
            "the first refusal in array order"
        );
        assert_eq!(
            refused(json!({"not": {"x": 1}})),
            PredicateError::Unknown("x".to_owned())
        );
        assert_eq!(
            PredicateError::Unknown("verdict".to_owned()).to_string(),
            "`verdict` is not in the closed set (all_changed_paths_match, \
             any_changed_path_matches, verify, item_kind, item_tag, all, any, not)"
        );
        assert_eq!(
            PredicateError::BadValue {
                key: "verify",
                want: "\"passed\" or \"failed\""
            }
            .to_string(),
            "`verify` takes \"passed\" or \"failed\""
        );
    }

    /// F12: a bare glob reads the primary repo only; a qualified one its own repo.
    #[test]
    fn a_bare_glob_means_the_primary_repo() {
        let bare = parse(json!({"any_changed_path_matches": ["src/**"]}));
        assert!(bare.evaluate(&facts(&[("htui", "src/a.rs")])));
        assert!(
            !bare.evaluate(&facts(&[("web", "src/a.rs")])),
            "a bare glob does not read another repo"
        );
        let no_primary = BranchFacts {
            primary: None,
            ..facts(&[("htui", "src/a.rs")])
        };
        assert!(
            !bare.evaluate(&no_primary),
            "with no primary in scope a bare glob matches nowhere"
        );
        let qualified = parse(json!({"any_changed_path_matches": ["web:src/**"]}));
        assert!(qualified.evaluate(&facts(&[("web", "src/a.rs")])));
        assert!(!qualified.evaluate(&facts(&[("htui", "src/a.rs")])));
        assert!(
            qualified.evaluate(&BranchFacts {
                primary: None,
                ..facts(&[("web", "src/x")])
            }),
            "a qualified glob needs no primary"
        );
        let cased = parse(json!({"any_changed_path_matches": ["SRC/**"]}));
        assert!(
            !cased.evaluate(&facts(&[("htui", "src/a.rs")])),
            "no case folding"
        );
    }

    #[test]
    fn no_changed_paths_makes_both_path_predicates_false() {
        let all = parse(json!({"all_changed_paths_match": ["**"]}));
        let any = parse(json!({"any_changed_path_matches": ["**"]}));
        assert!(!all.evaluate(&facts(&[])), "no vacuous truth");
        assert!(!any.evaluate(&facts(&[])));
        assert!(all.evaluate(&facts(&[("htui", "a")])));
        assert!(any.evaluate(&facts(&[("htui", "a")])));
    }

    #[test]
    fn a_truncated_list_fails_all_and_keeps_any() {
        let all = parse(json!({"all_changed_paths_match": ["docs/**"]}));
        let any = parse(json!({"any_changed_path_matches": ["docs/**"]}));
        let listed = facts(&[("htui", "docs/a.md")]);
        let truncated = BranchFacts {
            truncated: true,
            ..listed.clone()
        };
        assert!(all.evaluate(&listed));
        assert!(!all.evaluate(&truncated), "`all` cannot be proved");
        assert!(any.evaluate(&truncated), "a listed match still counts");
        let mixed = facts(&[("htui", "docs/a.md"), ("htui", "src/a.rs")]);
        assert!(!all.evaluate(&mixed));
        assert!(any.evaluate(&mixed));
    }

    #[test]
    fn verify_unavailable_or_none_matches_neither() {
        let passed = parse(json!({"verify": "passed"}));
        let failed = parse(json!({"verify": "failed"}));
        let with = |verify| BranchFacts {
            verify,
            ..facts(&[])
        };
        assert!(passed.evaluate(&with(Some(VerifyOutcome::Pass))));
        assert!(!failed.evaluate(&with(Some(VerifyOutcome::Pass))));
        assert!(failed.evaluate(&with(Some(VerifyOutcome::Fail))));
        assert!(!passed.evaluate(&with(Some(VerifyOutcome::Fail))));
        for verify in [Some(VerifyOutcome::Unavailable), None] {
            assert!(!passed.evaluate(&with(verify)), "{verify:?}");
            assert!(!failed.evaluate(&with(verify)), "{verify:?}");
        }
    }

    #[test]
    fn item_kind_is_the_key_prefix_and_item_tag_is_membership() {
        let tagged = BranchFacts {
            item_tags: vec!["gpu".to_owned(), "linux".to_owned()],
            ..facts(&[])
        };
        assert!(parse(json!({"item_kind": "HTUI"})).evaluate(&tagged));
        assert!(!parse(json!({"item_kind": "HTUIS"})).evaluate(&tagged));
        assert!(!parse(json!({"item_kind": "HTU"})).evaluate(&tagged));
        assert!(parse(json!({"item_tag": "linux"})).evaluate(&tagged));
        assert!(!parse(json!({"item_tag": "mac"})).evaluate(&tagged));
        assert!(!parse(json!({"item_tag": "gpu"})).evaluate(&facts(&[])));
    }

    #[test]
    fn combinators_all_any_not() {
        let f = BranchFacts {
            verify: Some(VerifyOutcome::Pass),
            ..facts(&[])
        };
        let yes = json!({"verify": "passed"});
        let no = json!({"verify": "failed"});
        assert!(parse(json!({"all": [yes, yes]})).evaluate(&f));
        assert!(!parse(json!({"all": [yes, no]})).evaluate(&f));
        assert!(parse(json!({"any": [no, yes]})).evaluate(&f));
        assert!(!parse(json!({"any": [no, no]})).evaluate(&f));
        assert!(parse(json!({"not": no})).evaluate(&f));
        assert!(!parse(json!({"not": yes})).evaluate(&f));
        assert!(parse(json!({"not": {"not": yes}})).evaluate(&f));
    }

    #[test]
    fn needs_paths_finds_a_nested_path_predicate() {
        assert!(!parse(json!({"verify": "passed"})).needs_paths());
        assert!(!parse(json!({"all": [{"item_kind": "BUG"}, {"item_tag": "b"}]})).needs_paths());
        assert!(parse(json!({"all_changed_paths_match": ["a"]})).needs_paths());
        assert!(
            parse(json!({
                "any": [{"verify": "passed"}, {"not": {"any_changed_path_matches": ["a"]}}]
            }))
            .needs_paths()
        );
    }

    #[test]
    fn nesting_past_the_cap_is_refused() {
        let nest = |nots: usize| {
            let mut value = json!({"verify": "passed"});
            for _ in 0..nots {
                value = json!({ "not": value });
            }
            value
        };
        assert!(
            IfPredicate::parse(&nest(MAX_DEPTH - 1)).is_ok(),
            "{MAX_DEPTH} levels, the leaf included, parse"
        );
        assert_eq!(
            IfPredicate::parse(&nest(MAX_DEPTH)),
            Err(PredicateError::TooDeep)
        );
        assert_eq!(
            PredicateError::TooDeep.to_string(),
            "a predicate nests deeper than 32"
        );
    }
}
