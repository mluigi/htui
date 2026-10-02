//! MOD-13 milestone 3 D2: a field-level three-way merge over the seven `version`-covered columns
//! of an item (ANA-9 §4.2).
//!
//! A field is [`FieldState::Same`], [`FieldState::Theirs`] or [`FieldState::Mine`] when at most
//! one side moved it (or both moved it to the same value), and those keep both sides' changes; a
//! pick ([`Side`]) decides the [`FieldState::Conflict`] fields only. That is why "take mine whole"
//! is wrong: the form's spec still holds the ancestor's value in every field the user did not
//! touch, so sending it whole would revert the head's changes (R-ENT-10, run the other way).
//!
//! Body and paths compare whole: there is no per-hunk merge (the PRD). There is no I/O here.

use crate::model::ItemSpec;

/// The seven `version`-covered columns, in the item form's `Tab` order minus its project picker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpecField {
    /// `item.kind_id`.
    Kind,
    /// `item.title`.
    Title,
    /// `item.priority`.
    Priority,
    /// `item.required_tags`.
    Tags,
    /// `item.step_graph_id`.
    Graph,
    /// `item.touched_paths`.
    Paths,
    /// `item.body`.
    Body,
}

impl SpecField {
    /// Every field, in that order: the order the view lists its rows in.
    pub const ALL: [Self; 7] = [
        Self::Kind,
        Self::Title,
        Self::Priority,
        Self::Tags,
        Self::Graph,
        Self::Paths,
        Self::Body,
    ];

    /// The form's row label: `kind`, `title`, `priority`, `tags`, `graph`, `paths`, `body`.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Kind => "kind",
            Self::Title => "title",
            Self::Priority => "priority",
            Self::Tags => "tags",
            Self::Graph => "graph",
            Self::Paths => "paths",
            Self::Body => "body",
        }
    }
}

/// How one field moved between the ancestor and each side (D2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldState {
    /// No side changed it, or both changed it to the same value.
    Same,
    /// Only the head changed it.
    Theirs,
    /// Only the form changed it.
    Mine,
    /// Both changed it, to different values.
    Conflict,
}

impl FieldState {
    /// `same`, `theirs`, `mine`, `conflict`: the view's state column.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Same => "same",
            Self::Theirs => "theirs",
            Self::Mine => "mine",
            Self::Conflict => "conflict",
        }
    }
}

/// Which side wins the conflicts (`t` / `m`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    /// The head's value wins (`t`).
    Theirs,
    /// The form's value wins (`m`).
    Mine,
}

/// The three specs a divergence compares. `Debug` is derived: `ItemSpec`'s own prints the body
/// and the paths as lengths (milestone 2 E6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpecMerge {
    ancestor: ItemSpec,
    theirs: ItemSpec,
    mine: ItemSpec,
}

/// D2: `ancestor` is the item the form opened on, `theirs` the head, `mine` the form's spec.
#[must_use]
pub fn merge(ancestor: &ItemSpec, theirs: &ItemSpec, mine: &ItemSpec) -> SpecMerge {
    SpecMerge {
        ancestor: ancestor.clone(),
        theirs: theirs.clone(),
        mine: mine.clone(),
    }
}

impl SpecMerge {
    /// The item the form opened on.
    #[must_use]
    pub const fn ancestor(&self) -> &ItemSpec {
        &self.ancestor
    }

    /// The head.
    #[must_use]
    pub const fn theirs(&self) -> &ItemSpec {
        &self.theirs
    }

    /// The form's spec.
    #[must_use]
    pub const fn mine(&self) -> &ItemSpec {
        &self.mine
    }

    /// One field's state, compared as stored values.
    #[must_use]
    pub fn state(&self, field: SpecField) -> FieldState {
        let (a, t, m) = (&self.ancestor, &self.theirs, &self.mine);
        match field {
            SpecField::Kind => classify(&a.kind_id, &t.kind_id, &m.kind_id),
            SpecField::Title => classify(&a.title, &t.title, &m.title),
            SpecField::Priority => classify(&a.priority, &t.priority, &m.priority),
            SpecField::Tags => classify(&a.required_tags, &t.required_tags, &m.required_tags),
            SpecField::Graph => classify(&a.step_graph_id, &t.step_graph_id, &m.step_graph_id),
            SpecField::Paths => classify(&a.touched_paths, &t.touched_paths, &m.touched_paths),
            SpecField::Body => classify(&a.body, &t.body, &m.body),
        }
    }

    /// The fields that are not `Same`, in [`SpecField::ALL`] order.
    #[must_use]
    pub fn changed(&self) -> Vec<SpecField> {
        SpecField::ALL
            .into_iter()
            .filter(|field| self.state(*field) != FieldState::Same)
            .collect()
    }

    /// Whether any field is `Conflict` (without one, `m` and `t` give the same spec).
    #[must_use]
    pub fn has_conflict(&self) -> bool {
        SpecField::ALL
            .into_iter()
            .any(|field| self.state(field) == FieldState::Conflict)
    }

    /// D2: `theirs` for `Same`/`Theirs`, `mine` for `Mine`, `side` for `Conflict`.
    #[must_use]
    pub fn resolve(&self, side: Side) -> ItemSpec {
        ItemSpec {
            kind_id: self.pick(SpecField::Kind, side, |spec| spec.kind_id),
            title: self.pick(SpecField::Title, side, |spec| spec.title.clone()),
            body: self.pick(SpecField::Body, side, |spec| spec.body.clone()),
            priority: self.pick(SpecField::Priority, side, |spec| spec.priority),
            required_tags: self.pick(SpecField::Tags, side, |spec| spec.required_tags.clone()),
            touched_paths: self.pick(SpecField::Paths, side, |spec| spec.touched_paths.clone()),
            step_graph_id: self.pick(SpecField::Graph, side, |spec| spec.step_graph_id),
        }
    }

    /// One field's resolved value: `theirs` for `Same`/`Theirs`, `mine` for `Mine`, `side`'s
    /// for `Conflict`.
    fn pick<T>(&self, field: SpecField, side: Side, get: impl Fn(&ItemSpec) -> T) -> T {
        match (self.state(field), side) {
            (FieldState::Same | FieldState::Theirs, _) | (FieldState::Conflict, Side::Theirs) => {
                get(&self.theirs)
            }
            (FieldState::Mine, _) | (FieldState::Conflict, Side::Mine) => get(&self.mine),
        }
    }
}

/// One field's state from its three values (D2).
fn classify<T: PartialEq>(ancestor: &T, theirs: &T, mine: &T) -> FieldState {
    match (theirs != ancestor, mine != ancestor) {
        (false, false) => FieldState::Same,
        (true, false) => FieldState::Theirs,
        (false, true) => FieldState::Mine,
        (true, true) if theirs == mine => FieldState::Same,
        (true, true) => FieldState::Conflict,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ItemKindId, StepGraphId};

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    /// A hand-built spec (A15: not the demo).
    fn base() -> ItemSpec {
        ItemSpec {
            kind_id: ItemKindId::new(),
            title: "Fix it".to_owned(),
            body: "one\ntwo".to_owned(),
            priority: 3,
            required_tags: strings(&["rust"]),
            touched_paths: strings(&["src/**"]),
            step_graph_id: Some(StepGraphId::new()),
        }
    }

    /// Sets `field` on `spec` to a value distinct per `variant` (0 and 1 differ from each other
    /// and from [`base`]'s).
    fn moved(spec: &ItemSpec, field: SpecField, variant: usize) -> ItemSpec {
        let mut next = spec.clone();
        match field {
            SpecField::Kind => next.kind_id = ItemKindId::new(),
            SpecField::Title => next.title = format!("Title {variant}"),
            SpecField::Priority => next.priority = 10 + i16::try_from(variant).unwrap(),
            SpecField::Tags => next.required_tags = vec![format!("tag{variant}")],
            SpecField::Graph => {
                // Variant 0 clears the graph to the kind's default: `Some(g1)` -> `None`.
                next.step_graph_id = (variant != 0).then(StepGraphId::new);
            }
            SpecField::Paths => next.touched_paths = vec![format!("p{variant}/**")],
            SpecField::Body => next.body = format!("body {variant}"),
        }
        next
    }

    #[test]
    fn each_field_moves_through_every_state() {
        let ancestor = base();
        for field in SpecField::ALL {
            let same = merge(&ancestor, &ancestor, &ancestor);
            assert_eq!(same.state(field), FieldState::Same, "{field:?}");

            let theirs = moved(&ancestor, field, 0);
            let only_theirs = merge(&ancestor, &theirs, &ancestor);
            assert_eq!(only_theirs.state(field), FieldState::Theirs, "{field:?}");
            assert_eq!(only_theirs.changed(), vec![field]);

            let mine = moved(&ancestor, field, 1);
            let only_mine = merge(&ancestor, &ancestor, &mine);
            assert_eq!(only_mine.state(field), FieldState::Mine, "{field:?}");
            assert_eq!(only_mine.changed(), vec![field]);

            let both = merge(&ancestor, &theirs, &mine);
            assert_eq!(both.state(field), FieldState::Conflict, "{field:?}");
            assert!(both.has_conflict(), "{field:?}");
            for other in SpecField::ALL.into_iter().filter(|other| *other != field) {
                assert_eq!(both.state(other), FieldState::Same, "{field:?}/{other:?}");
            }
        }
    }

    #[test]
    fn both_sides_to_the_same_value_is_same() {
        let ancestor = base();
        let theirs = ItemSpec {
            title: "Agreed".to_owned(),
            ..ancestor.clone()
        };
        let mine = theirs.clone();
        let merged = merge(&ancestor, &theirs, &mine);
        assert_eq!(merged.state(SpecField::Title), FieldState::Same);
        assert!(merged.changed().is_empty());
        assert!(!merged.has_conflict());
        assert_eq!(merged.resolve(Side::Theirs), theirs);
        assert_eq!(merged.resolve(Side::Mine), theirs);
    }

    #[test]
    fn resolve_takes_the_chosen_side_for_conflicts_only() {
        let ancestor = base();
        let theirs = ItemSpec {
            title: "Theirs".to_owned(),
            ..ancestor.clone()
        };
        let mine = ItemSpec {
            title: "Mine".to_owned(),
            body: "my body".to_owned(),
            ..ancestor.clone()
        };
        let merged = merge(&ancestor, &theirs, &mine);
        assert_eq!(merged.state(SpecField::Title), FieldState::Conflict);
        assert_eq!(merged.state(SpecField::Body), FieldState::Mine);

        let took_theirs = merged.resolve(Side::Theirs);
        assert_eq!(took_theirs.title, "Theirs");
        assert_eq!(took_theirs.body, "my body");

        let took_mine = merged.resolve(Side::Mine);
        assert_eq!(took_mine.title, "Mine");
        assert_eq!(took_mine.body, "my body");
    }

    /// The D2 regression: "take mine whole" would revert the head's priority.
    #[test]
    fn resolve_mine_keeps_a_priority_only_theirs_changed() {
        let ancestor = base();
        let theirs = ItemSpec {
            priority: 9,
            ..ancestor.clone()
        };
        let mine = ItemSpec {
            title: "Mine".to_owned(),
            ..ancestor.clone()
        };
        let resolved = merge(&ancestor, &theirs, &mine).resolve(Side::Mine);
        assert_eq!(resolved.priority, 9, "theirs' priority survives");
        assert_eq!(resolved.title, "Mine", "my title lands");
        assert_eq!(
            resolved,
            ItemSpec {
                title: "Mine".to_owned(),
                ..theirs
            }
        );
    }

    #[test]
    fn resolve_theirs_keeps_my_changes_that_do_not_conflict() {
        let ancestor = base();
        let theirs = ItemSpec {
            title: "Theirs".to_owned(),
            ..ancestor.clone()
        };
        let mine = ItemSpec {
            title: "Mine".to_owned(),
            required_tags: strings(&["go", "rust"]),
            ..ancestor.clone()
        };
        let resolved = merge(&ancestor, &theirs, &mine).resolve(Side::Theirs);
        assert_eq!(resolved.title, "Theirs");
        assert_eq!(resolved.required_tags, strings(&["go", "rust"]));
    }

    #[test]
    fn without_a_conflict_both_sides_resolve_alike() {
        let ancestor = base();
        let theirs = ItemSpec {
            priority: 9,
            step_graph_id: None,
            ..ancestor.clone()
        };
        let mine = ItemSpec {
            title: "Mine".to_owned(),
            touched_paths: strings(&["docs/**"]),
            ..ancestor.clone()
        };
        let merged = merge(&ancestor, &theirs, &mine);
        assert!(!merged.has_conflict());
        assert_eq!(merged.resolve(Side::Mine), merged.resolve(Side::Theirs));
        assert_eq!(
            merged.resolve(Side::Mine),
            ItemSpec {
                title: "Mine".to_owned(),
                touched_paths: strings(&["docs/**"]),
                ..theirs
            }
        );
    }

    #[test]
    fn changed_lists_fields_in_form_order() {
        let ancestor = base();
        let theirs = ItemSpec {
            body: "their body".to_owned(),
            ..ancestor.clone()
        };
        let mine = ItemSpec {
            kind_id: ItemKindId::new(),
            required_tags: strings(&["go"]),
            ..ancestor.clone()
        };
        assert_eq!(
            merge(&ancestor, &theirs, &mine).changed(),
            vec![SpecField::Kind, SpecField::Tags, SpecField::Body]
        );
    }

    #[test]
    fn labels_are_the_form_row_labels() {
        assert_eq!(
            SpecField::ALL.map(SpecField::label),
            [
                "kind", "title", "priority", "tags", "graph", "paths", "body"
            ]
        );
        assert_eq!(
            [
                FieldState::Same,
                FieldState::Theirs,
                FieldState::Mine,
                FieldState::Conflict,
            ]
            .map(FieldState::label),
            ["same", "theirs", "mine", "conflict"]
        );
    }

    #[test]
    fn accessors_return_the_three_specs() {
        let ancestor = base();
        let theirs = moved(&ancestor, SpecField::Title, 0);
        let mine = moved(&ancestor, SpecField::Title, 1);
        let merged = merge(&ancestor, &theirs, &mine);
        assert_eq!(merged.ancestor(), &ancestor);
        assert_eq!(merged.theirs(), &theirs);
        assert_eq!(merged.mine(), &mine);
    }
}
