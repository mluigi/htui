//! MOD-13 D4: the one validator of an item's spec columns, the `version`-covered ones of §4.2.
//!
//! The Backlog form calls it for early feedback, and the store worker calls it again as the
//! authority, against a fresh read of the project's kinds, step graphs and repos. Both get the same
//! sentence for the same refusal, because there is only this one place that writes it.
//!
//! The text front (`parse_*`, `*_text`) turns what the form holds into typed values and back. The
//! checks (`check_*`) take typed values and answer the **canonical** ones: a title trimmed, tags
//! sorted and deduplicated, paths trimmed and deduplicated in order. An edit checks only the fields
//! it changes (A4), so an item holding a legacy value can still have its title fixed.
//!
//! `touched_paths` follows `overlap::resolve`'s rule (htui-orch), through
//! [`split_qualifier`]: a qualified entry must name one of the project's repos, a bare one needs a
//! primary repo, which `resolve` would otherwise drop silently.

use crate::model::skill_glob::{self, GlobError};
use crate::model::{
    BoxId, Item, ItemId, ItemKind, ItemKindId, ItemPatch, NewItem, ProjectId, Repo, StepGraph,
    StepGraphId, UserId, canonical_declared_tags, declared_tags_from_text,
};
use crate::prompt::excerpt::split_qualifier;

/// `item_revision.reason` of a form edit (D5).
pub const EDITED: &str = "edited";
/// D5: an edit whose every field equals the item's. Said by the form, refused by the worker.
pub const NOTHING_TO_SAVE: &str = "nothing to save: no field differs from the item";

/// What a spec is checked against: one project's kinds, step graphs and repos.
#[derive(Debug, Clone, Copy)]
pub struct SpecContext<'a> {
    /// The project's item kinds.
    pub kinds: &'a [ItemKind],
    /// The project's graphs. `is_override` rows may be present (the worker passes
    /// `step_graphs()` whole); the check never accepts one as a change (A3).
    pub graphs: &'a [StepGraph],
    /// The project's repos, with their names and `is_primary`.
    pub repos: &'a [Repo],
}

/// Every `version`-covered spec column of a new item, typed and canonical (D4). The wire payload
/// of `StoreRequest::MintItem`.
#[derive(Clone, PartialEq, Eq)]
pub struct ItemSpec {
    /// `item.kind_id`.
    pub kind_id: ItemKindId,
    /// `item.title`.
    pub title: String,
    /// `item.body`.
    pub body: String,
    /// `item.priority`.
    pub priority: i16,
    /// `item.required_tags`.
    pub required_tags: Vec<String>,
    /// `item.touched_paths`.
    pub touched_paths: Vec<String>,
    /// `None` = the kind's default graph.
    pub step_graph_id: Option<StepGraphId>,
}

/// The columns an edit changes; `None` leaves a column alone (D5, A4). The wire payload of
/// `StoreRequest::EditItem`. `step_graph_id` is `ItemPatch`'s double option: `Some(None)` clears.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct SpecChanges {
    /// New `item.kind_id`.
    pub kind_id: Option<ItemKindId>,
    /// New `item.title`.
    pub title: Option<String>,
    /// New `item.body`.
    pub body: Option<String>,
    /// New `item.priority`.
    pub priority: Option<i16>,
    /// New `item.required_tags`.
    pub required_tags: Option<Vec<String>>,
    /// New `item.touched_paths`.
    pub touched_paths: Option<Vec<String>>,
    /// New `item.step_graph_id`; the inner `None` is the kind's default graph.
    pub step_graph_id: Option<Option<StepGraphId>>,
}

/// Lengths, not text, for `body` and `touched_paths` (E6): the rule of
/// `StoreRequest`'s doc (`store_worker.rs:96-104`), since this struct rides in one.
impl core::fmt::Debug for ItemSpec {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ItemSpec")
            .field("kind_id", &self.kind_id)
            .field("title", &self.title)
            .field("body_len", &self.body.len())
            .field("priority", &self.priority)
            .field("required_tags", &self.required_tags)
            .field("touched_paths", &self.touched_paths.len())
            .field("step_graph_id", &self.step_graph_id)
            .finish()
    }
}

/// Lengths, not text, for `body` and `touched_paths` (E6): the rule of
/// `StoreRequest`'s doc (`store_worker.rs:96-104`), since this struct rides in one.
impl core::fmt::Debug for SpecChanges {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SpecChanges")
            .field("kind_id", &self.kind_id)
            .field("title", &self.title)
            .field("body", &self.body.as_ref().map(String::len))
            .field("priority", &self.priority)
            .field("required_tags", &self.required_tags)
            .field("touched_paths", &self.touched_paths.as_ref().map(Vec::len))
            .field("step_graph_id", &self.step_graph_id)
            .finish()
    }
}

/// Why a spec is refused. Every sentence names the field or the entry (D4).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SpecError {
    /// The title is empty once trimmed.
    #[error("an item needs a title")]
    BlankTitle,
    /// The priority text, trimmed, is not an `i16`.
    #[error("priority `{0}` is not a whole number from -32768 to 32767")]
    Priority(String),
    /// `canonical_declared_tags`' sentence, which names the tag (`box_.rs:298-303`).
    #[error("{0}")]
    Tags(String),
    /// The kind is not one of the context's kinds.
    #[error("that kind is not one of this project's item kinds")]
    Kind(ItemKindId),
    /// The graph is not one of the context's non-override graphs.
    #[error("that step graph is not one of this project's graphs")]
    Graph(StepGraphId),
    /// The title or the body holds U+0000, which Postgres `text` cannot store (`22021`). The
    /// column is named as `has_nul` names it, so both stores refuse it before any write.
    #[error("{0} must not contain a NUL character")]
    Nul(&'static str),
    /// One `touched_paths` entry is refused.
    #[error("touched path `{entry}` {why}")]
    Path {
        /// The entry, trimmed.
        entry: String,
        /// Why, as the rest of the sentence.
        why: String,
    },
}

/// The title as stored: trimmed.
///
/// # Errors
/// [`SpecError::BlankTitle`] when nothing is left, then [`SpecError::Nul`] for a NUL.
pub fn parse_title(text: &str) -> Result<String, SpecError> {
    match text.trim() {
        "" => Err(SpecError::BlankTitle),
        title if title.contains('\0') => Err(SpecError::Nul("item.title")),
        title => Ok(title.to_owned()),
    }
}

/// The priority field's text as an `i16`, surrounding whitespace ignored.
///
/// # Errors
/// [`SpecError::Priority`] naming the trimmed text.
pub fn parse_priority(text: &str) -> Result<i16, SpecError> {
    let text = text.trim();
    text.parse()
        .map_err(|_| SpecError::Priority(text.to_owned()))
}

/// The tags field's comma list, through `declared_tags_from_text` (canonical).
///
/// # Errors
/// [`SpecError::Tags`] with the sentence naming the first bad tag.
pub fn parse_tags(text: &str) -> Result<Vec<String>, SpecError> {
    declared_tags_from_text(text).map_err(SpecError::Tags)
}

/// The paths field: one entry per line, each trimmed, blanks dropped, duplicates dropped keeping
/// the first. Unchecked: [`check_paths`] is the check.
#[must_use]
pub fn parse_paths(text: &str) -> Vec<String> {
    canonical_paths(text.lines())
}

/// The tags field's text for `tags`: [`parse_tags`]' inverse on a canonical list.
#[must_use]
pub fn tags_text(tags: &[String]) -> String {
    tags.join(", ")
}

/// The paths field's text for `paths`: [`parse_paths`]' inverse on a canonical list.
#[must_use]
pub fn paths_text(paths: &[String]) -> String {
    paths.join("\n")
}

/// `touched_paths` checked against the project's repos (D4, A2). The list is canonicalised first
/// (trimmed, blanks dropped, duplicates dropped keeping order), so the function is idempotent.
///
/// # Errors
/// [`SpecError::Path`] for the first entry refused, naming it and the rule that fired.
pub fn check_paths(paths: &[String], repos: &[Repo]) -> Result<Vec<String>, SpecError> {
    let paths = canonical_paths(paths.iter().map(String::as_str));
    if let Some((entry, why)) = paths
        .iter()
        .find_map(|entry| path_refusal(entry, repos).map(|why| (entry, why)))
    {
        return Err(SpecError::Path {
            entry: entry.clone(),
            why,
        });
    }
    Ok(paths)
}

/// A new item's spec, checked and canonical. The order is title, body, kind, tags, graph, paths.
///
/// # Errors
/// The first [`SpecError`] in that order.
pub fn check_spec(spec: &ItemSpec, ctx: &SpecContext<'_>) -> Result<ItemSpec, SpecError> {
    let title = parse_title(&spec.title)?;
    let body = check_body(&spec.body)?;
    let kind_id = check_kind(spec.kind_id, ctx)?;
    let required_tags = check_tags(&spec.required_tags)?;
    let step_graph_id = check_graph(spec.step_graph_id, ctx)?;
    let touched_paths = check_paths(&spec.touched_paths, ctx.repos)?;
    Ok(ItemSpec {
        kind_id,
        title,
        body,
        priority: spec.priority,
        required_tags,
        touched_paths,
        step_graph_id,
    })
}

/// An edit's changes, checked and canonical: only the `Some` fields are looked at (A4), in
/// [`check_spec`]'s order.
///
/// # Errors
/// The first [`SpecError`] in that order.
pub fn check_changes(
    changes: &SpecChanges,
    ctx: &SpecContext<'_>,
) -> Result<SpecChanges, SpecError> {
    let title = changes.title.as_deref().map(parse_title).transpose()?;
    let body = changes.body.as_deref().map(check_body).transpose()?;
    let kind_id = changes.kind_id.map(|id| check_kind(id, ctx)).transpose()?;
    let required_tags = changes
        .required_tags
        .as_deref()
        .map(check_tags)
        .transpose()?;
    let step_graph_id = changes
        .step_graph_id
        .map(|id| check_graph(id, ctx))
        .transpose()?;
    let touched_paths = changes
        .touched_paths
        .as_deref()
        .map(|paths| check_paths(paths, ctx.repos))
        .transpose()?;
    Ok(SpecChanges {
        kind_id,
        title,
        body,
        priority: changes.priority,
        required_tags,
        touched_paths,
        step_graph_id,
    })
}

/// The body as given, unless it holds a NUL.
fn check_body(body: &str) -> Result<String, SpecError> {
    if body.contains('\0') {
        Err(SpecError::Nul("item.body"))
    } else {
        Ok(body.to_owned())
    }
}

/// The kind is one of the context's.
fn check_kind(id: ItemKindId, ctx: &SpecContext<'_>) -> Result<ItemKindId, SpecError> {
    if ctx.kinds.iter().any(|kind| kind.id == id) {
        Ok(id)
    } else {
        Err(SpecError::Kind(id))
    }
}

/// `canonical_declared_tags`, the stores' own canonical form.
fn check_tags(tags: &[String]) -> Result<Vec<String>, SpecError> {
    canonical_declared_tags(tags).map_err(SpecError::Tags)
}

/// `None` (the kind's default) or one of the context's graphs that is not an override (A3). An
/// item's unchanged override graph is never in a [`SpecChanges`], so it never reaches here.
fn check_graph(
    id: Option<StepGraphId>,
    ctx: &SpecContext<'_>,
) -> Result<Option<StepGraphId>, SpecError> {
    match id {
        Some(id) if !ctx.graphs.iter().any(|g| g.id == id && !g.is_override) => {
            Err(SpecError::Graph(id))
        }
        id => Ok(id),
    }
}

/// Entries trimmed, blanks dropped, duplicates dropped keeping the first.
fn canonical_paths<'a>(entries: impl Iterator<Item = &'a str>) -> Vec<String> {
    let mut paths: Vec<String> = Vec::new();
    for entry in entries.map(str::trim).filter(|entry| !entry.is_empty()) {
        if !paths.iter().any(|path| path == entry) {
            paths.push(entry.to_owned());
        }
    }
    paths
}

/// Why one trimmed entry is refused, as the rest of [`SpecError::Path`]'s sentence; the first
/// rule that fires wins.
fn path_refusal(entry: &str, repos: &[Repo]) -> Option<String> {
    if entry.contains('\0') {
        return Some("contains a NUL character".to_owned());
    }
    let (repo, glob) = split_qualifier(entry);
    match repo {
        Some(repo) => {
            if repo.ends_with(char::is_whitespace) || glob.starts_with(char::is_whitespace) {
                return Some("has whitespace next to its `:`".to_owned());
            }
            if glob.is_empty() {
                return Some(format!("has no glob after `{repo}:`"));
            }
            if !repos.iter().any(|row| row.name == repo) {
                return Some(format!(
                    "names `{repo}`, which is not a repo of this project"
                ));
            }
        }
        None if !repos.iter().any(|row| row.is_primary) => {
            return Some(
                "is bare and this project has no primary repo; write it as `<repo>:<glob>`"
                    .to_owned(),
            );
        }
        None => {}
    }
    if glob.starts_with('/') {
        return Some("is absolute; touched paths are repo-relative".to_owned());
    }
    skill_glob::matcher(glob).err().map(|err| {
        let message = match err {
            GlobError::Invalid { message, .. } => message,
            other @ GlobError::UnknownLanguage(_) => other.to_string(),
        };
        format!("is not a valid glob: {message}")
    })
}

impl ItemSpec {
    /// The spec as `item` stores it, unchecked (the edit form's base, A4).
    #[must_use]
    pub fn of(item: &Item) -> Self {
        Self {
            kind_id: item.kind_id,
            title: item.title.clone(),
            body: item.body.clone(),
            priority: item.priority,
            required_tags: item.required_tags.clone(),
            touched_paths: item.touched_paths.clone(),
            step_graph_id: item.step_graph_id,
        }
    }

    /// The `mint_item` arguments for this spec.
    #[must_use]
    pub fn into_new_item(
        self,
        id: ItemId,
        project_id: ProjectId,
        created_by: UserId,
        box_id: Option<BoxId>,
    ) -> NewItem {
        NewItem {
            id,
            project_id,
            kind_id: self.kind_id,
            title: self.title,
            body: self.body,
            required_tags: self.required_tags,
            touched_paths: self.touched_paths,
            priority: self.priority,
            step_graph_id: self.step_graph_id,
            created_by,
            box_id,
        }
    }
}

/// Every field changed: the spec whole.
impl From<ItemSpec> for SpecChanges {
    fn from(spec: ItemSpec) -> Self {
        Self {
            kind_id: Some(spec.kind_id),
            title: Some(spec.title),
            body: Some(spec.body),
            priority: Some(spec.priority),
            required_tags: Some(spec.required_tags),
            touched_paths: Some(spec.touched_paths),
            step_graph_id: Some(spec.step_graph_id),
        }
    }
}

impl SpecChanges {
    /// The fields whose values differ between `base` and `next` (compared as stored values).
    #[must_use]
    pub fn between(base: &ItemSpec, next: &ItemSpec) -> Self {
        /// `Some(next)` when it differs from `base`.
        fn changed<T: PartialEq + Clone>(base: &T, next: &T) -> Option<T> {
            (base != next).then(|| next.clone())
        }
        Self {
            kind_id: changed(&base.kind_id, &next.kind_id),
            title: changed(&base.title, &next.title),
            body: changed(&base.body, &next.body),
            priority: changed(&base.priority, &next.priority),
            required_tags: changed(&base.required_tags, &next.required_tags),
            touched_paths: changed(&base.touched_paths, &next.touched_paths),
            step_graph_id: changed(&base.step_graph_id, &next.step_graph_id),
        }
    }

    /// Whether no field is changed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// The `update_item` patch for these changes, `reason = EDITED`.
    #[must_use]
    pub fn into_patch(self, author_id: UserId, box_id: Option<BoxId>) -> ItemPatch {
        ItemPatch {
            title: self.title,
            body: self.body,
            kind_id: self.kind_id,
            required_tags: self.required_tags,
            priority: self.priority,
            touched_paths: self.touched_paths,
            step_graph_id: self.step_graph_id,
            author_id,
            box_id,
            reason: EDITED.to_owned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, Utc};

    use super::*;
    use crate::model::{RepoId, Status};

    const EPOCH: DateTime<Utc> = DateTime::UNIX_EPOCH;

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|&item| item.to_owned()).collect()
    }

    fn repo(name: &str, primary: bool) -> Repo {
        Repo {
            id: RepoId::new(),
            project_id: ProjectId::default(),
            name: name.to_owned(),
            remote_url: None,
            default_branch: "main".to_owned(),
            is_primary: primary,
            created_at: EPOCH,
            updated_at: EPOCH,
        }
    }

    fn kind(project: ProjectId) -> ItemKind {
        ItemKind {
            id: ItemKindId::new(),
            project_id: project,
            prefix: "ANA".to_owned(),
            name: "Analysis".to_owned(),
            description: String::new(),
            default_graph_id: StepGraphId::new(),
            position: 0,
            updated_at: EPOCH,
        }
    }

    fn graph(project: ProjectId, is_override: bool) -> StepGraph {
        StepGraph {
            id: StepGraphId::new(),
            project_id: project,
            name: "feat".to_owned(),
            description: String::new(),
            is_override,
            created_at: EPOCH,
            updated_at: EPOCH,
        }
    }

    fn ctx<'a>(
        kinds: &'a [ItemKind],
        graphs: &'a [StepGraph],
        repos: &'a [Repo],
    ) -> SpecContext<'a> {
        SpecContext {
            kinds,
            graphs,
            repos,
        }
    }

    /// A spec every rule accepts against `ctx(&[kind], .., &[repo("htui", true)])`.
    fn spec(kind: &ItemKind) -> ItemSpec {
        ItemSpec {
            kind_id: kind.id,
            title: "Fix it".to_owned(),
            body: "the body".to_owned(),
            priority: 3,
            required_tags: strings(&["rust"]),
            touched_paths: strings(&["src/**"]),
            step_graph_id: None,
        }
    }

    /// The `why` of `check_paths` refusing `entry` alone against `repos`.
    fn why(entry: &str, repos: &[Repo]) -> String {
        match check_paths(&strings(&[entry]), repos) {
            Err(SpecError::Path { entry: named, why }) => {
                assert_eq!(named, entry, "the refusal names the entry");
                why
            }
            other => panic!("`{entry:?}` is refused as a path, got {other:?}"),
        }
    }

    /// D4: a title is trimmed and never blank.
    #[test]
    fn a_blank_title_is_refused() {
        for text in ["", "   ", "\t"] {
            assert_eq!(parse_title(text), Err(SpecError::BlankTitle), "{text:?}");
        }
        assert_eq!(parse_title("  Fix it "), Ok("Fix it".to_owned()));
    }

    /// D4: a priority is an `i16`; the refusal names the trimmed text.
    #[test]
    fn priority_parses_as_an_i16() {
        assert_eq!(
            parse_priority("x"),
            Err(SpecError::Priority("x".to_owned()))
        );
        assert_eq!(
            parse_priority("40000"),
            Err(SpecError::Priority("40000".to_owned()))
        );
        assert_eq!(parse_priority("-3"), Ok(-3));
        assert_eq!(parse_priority(" 7 "), Ok(7));
    }

    /// D4: the box-tag rules; a bad tag is named.
    #[test]
    fn tags_are_canonical_and_a_bad_one_is_named() {
        match parse_tags("Rust") {
            Err(SpecError::Tags(sentence)) => assert!(sentence.contains("`Rust`"), "{sentence}"),
            other => panic!("`Rust` is refused, got {other:?}"),
        }
        assert_eq!(
            parse_tags("rust, docker,rust"),
            Ok(strings(&["docker", "rust"]))
        );
        assert_eq!(parse_tags(""), Ok(Vec::new()));
    }

    /// D4: a bare entry means the primary repo.
    #[test]
    fn a_bare_path_is_kept_with_a_primary_repo() {
        assert_eq!(
            check_paths(&strings(&["src/**"]), &[repo("htui", true)]),
            Ok(strings(&["src/**"]))
        );
    }

    /// D4: `overlap::resolve` would drop a bare entry with no primary repo, silently.
    #[test]
    fn a_bare_path_is_refused_without_a_primary_repo() {
        assert_eq!(
            why("src/**", &[repo("web", false)]),
            "is bare and this project has no primary repo; write it as `<repo>:<glob>`"
        );
        assert_eq!(
            why("src/**", &[]),
            "is bare and this project has no primary repo; write it as `<repo>:<glob>`"
        );
    }

    /// E5: a qualified entry resolves by name, primary or not.
    #[test]
    fn a_qualified_path_naming_a_project_repo_is_kept() {
        assert_eq!(
            check_paths(&strings(&["web:src/**"]), &[repo("web", false)]),
            Ok(strings(&["web:src/**"]))
        );
    }

    /// D4: an unknown repo is the orch's `UnknownTouchedRepo`; refused here, by name.
    #[test]
    fn an_unknown_repo_is_refused_by_name() {
        let why = why("nope:src", &[repo("htui", true)]);
        assert!(why.contains("`nope`"), "{why}");
        assert_eq!(why, "names `nope`, which is not a repo of this project");
    }

    /// D4: a qualifier needs a glob after it.
    #[test]
    fn a_qualifier_with_no_glob_is_refused() {
        assert_eq!(
            why("web:", &[repo("web", true)]),
            "has no glob after `web:`"
        );
    }

    /// A2: `web: src` would store a glob with a leading space that never matches.
    #[test]
    fn whitespace_next_to_the_colon_is_refused() {
        for entry in ["web: src", "web :src"] {
            assert_eq!(
                why(entry, &[repo("web", true)]),
                "has whitespace next to its `:`",
                "{entry:?}"
            );
        }
    }

    /// D4: touched paths are repo-relative.
    #[test]
    fn an_absolute_path_is_refused() {
        assert_eq!(
            why("/abs", &[repo("htui", true)]),
            "is absolute; touched paths are repo-relative"
        );
        assert_eq!(
            why("web:/abs", &[repo("web", false)]),
            "is absolute; touched paths are repo-relative",
            "the glob after the qualifier is checked too"
        );
    }

    /// D4: the glob must compile under `skill_glob`'s one builder; `globset`'s words say why.
    #[test]
    fn a_glob_that_does_not_compile_is_refused() {
        let why = why("src/[", &[repo("htui", true)]);
        assert!(why.contains("unclosed character class"), "{why}");
        assert_eq!(
            SpecError::Path {
                entry: "src/[".to_owned(),
                why: why.clone(),
            }
            .to_string(),
            format!("touched path `src/[` {why}")
        );
    }

    /// C1: a `:` after a `/` is part of the path, so the entry is bare and kept verbatim.
    #[test]
    fn a_colon_after_a_slash_is_bare() {
        assert_eq!(
            check_paths(&strings(&["a/b:c"]), &[repo("htui", true)]),
            Ok(strings(&["a/b:c"]))
        );
    }

    /// D4: a NUL is refused before any other rule.
    #[test]
    fn a_nul_is_refused() {
        assert_eq!(
            why("src/\0x", &[repo("htui", true)]),
            "contains a NUL character"
        );
        assert_eq!(
            why("nope:\0", &[]),
            "contains a NUL character",
            "the NUL rule fires first"
        );
    }

    /// D4: Postgres `text` cannot hold U+0000 (`22021`), so a NUL in the title or the body is
    /// refused by rule, not left to fail as a store error on `PgStore` only (`has_nul`).
    #[test]
    fn a_nul_in_the_title_or_body_is_refused() {
        let ours = kind(ProjectId::new());
        let kinds = [ours.clone()];
        let repos = [repo("htui", true)];
        let context = ctx(&kinds, &[], &repos);
        let title = SpecError::Nul("item.title");
        let body = SpecError::Nul("item.body");
        assert_eq!(
            title.to_string(),
            "item.title must not contain a NUL character"
        );
        assert_eq!(parse_title("Fix\0it"), Err(title.clone()));
        assert_eq!(
            parse_title("\0"),
            Err(title.clone()),
            "a lone NUL is not blank"
        );
        let spec_with = |title: &str, body: &str| ItemSpec {
            title: title.to_owned(),
            body: body.to_owned(),
            ..spec(&ours)
        };
        assert_eq!(
            check_spec(&spec_with("a\0b", "ok"), &context),
            Err(title.clone())
        );
        assert_eq!(
            check_spec(&spec_with("ok", "a\0b"), &context),
            Err(body.clone())
        );
        assert_eq!(
            check_spec(&spec_with("a\0b", "a\0b"), &context),
            Err(title.clone()),
            "the title is checked first"
        );
        assert_eq!(
            check_changes(
                &SpecChanges {
                    title: Some("a\0b".to_owned()),
                    ..SpecChanges::default()
                },
                &context
            ),
            Err(title)
        );
        assert_eq!(
            check_changes(
                &SpecChanges {
                    body: Some("a\0b".to_owned()),
                    ..SpecChanges::default()
                },
                &context
            ),
            Err(body)
        );
    }

    /// D4: one entry per line, trimmed, blanks and duplicates dropped, order kept.
    #[test]
    fn blank_and_duplicate_paths_are_dropped_in_order() {
        assert_eq!(
            parse_paths("src/**\n\n  docs/*  \nsrc/**\n"),
            strings(&["src/**", "docs/*"])
        );
        assert_eq!(
            check_paths(&strings(&[" src/** ", "src/**"]), &[repo("htui", true)]),
            Ok(strings(&["src/**"]))
        );
        assert_eq!(
            check_paths(&strings(&["", "  "]), &[]),
            Ok(Vec::new()),
            "no entry needs no repo"
        );
    }

    /// D4: the kind must be one of the context's.
    #[test]
    fn a_kind_of_another_project_is_refused() {
        let ours = kind(ProjectId::new());
        let theirs = kind(ProjectId::new());
        let repos = [repo("htui", true)];
        let context = ctx(core::slice::from_ref(&ours), &[], &repos);
        assert_eq!(check_spec(&spec(&ours), &context), Ok(spec(&ours)));
        assert_eq!(
            check_spec(&spec(&theirs), &context),
            Err(SpecError::Kind(theirs.id))
        );
        let changes = SpecChanges {
            kind_id: Some(theirs.id),
            ..SpecChanges::default()
        };
        assert_eq!(
            check_changes(&changes, &context),
            Err(SpecError::Kind(theirs.id))
        );
    }

    /// D4: `None` is the kind's default graph; a graph must be one of the context's.
    #[test]
    fn a_graph_outside_the_project_is_refused_and_none_is_the_kind_default() {
        let project = ProjectId::new();
        let ours = kind(project);
        let own = graph(project, false);
        let other = graph(ProjectId::new(), false);
        let repos = [repo("htui", true)];
        let graphs = [own.clone()];
        let context = ctx(core::slice::from_ref(&ours), &graphs, &repos);

        assert_eq!(check_spec(&spec(&ours), &context), Ok(spec(&ours)));
        let with_own = ItemSpec {
            step_graph_id: Some(own.id),
            ..spec(&ours)
        };
        assert_eq!(check_spec(&with_own, &context), Ok(with_own.clone()));
        let with_other = ItemSpec {
            step_graph_id: Some(other.id),
            ..spec(&ours)
        };
        assert_eq!(
            check_spec(&with_other, &context),
            Err(SpecError::Graph(other.id))
        );
        let clear = SpecChanges {
            step_graph_id: Some(None),
            ..SpecChanges::default()
        };
        assert_eq!(check_changes(&clear, &context), Ok(clear.clone()));
    }

    /// A3: an override graph is never accepted as a change, on a mint or an edit.
    #[test]
    fn an_override_graph_is_refused_as_a_change() {
        let project = ProjectId::new();
        let ours = kind(project);
        let clone = graph(project, true);
        let repos = [repo("htui", true)];
        let graphs = [clone.clone()];
        let context = ctx(core::slice::from_ref(&ours), &graphs, &repos);

        let minted = ItemSpec {
            step_graph_id: Some(clone.id),
            ..spec(&ours)
        };
        assert_eq!(
            check_spec(&minted, &context),
            Err(SpecError::Graph(clone.id))
        );
        let changes = SpecChanges {
            step_graph_id: Some(Some(clone.id)),
            ..SpecChanges::default()
        };
        assert_eq!(
            check_changes(&changes, &context),
            Err(SpecError::Graph(clone.id))
        );
    }

    /// A4: an item with legacy values can still have its title fixed.
    #[test]
    fn only_changed_fields_are_checked() {
        let base = ItemSpec {
            required_tags: strings(&["Rust"]),
            touched_paths: strings(&["nope:x"]),
            ..spec(&kind(ProjectId::new()))
        };
        let context = ctx(&[], &[], &[]);
        let retitled = ItemSpec {
            title: "Fixed".to_owned(),
            ..base.clone()
        };
        let changes = SpecChanges::between(&base, &retitled);
        assert_eq!(
            changes,
            SpecChanges {
                title: Some("Fixed".to_owned()),
                ..SpecChanges::default()
            },
            "only the title differs"
        );
        assert_eq!(check_changes(&changes, &context), Ok(changes.clone()));
        assert_eq!(
            check_changes(
                &SpecChanges {
                    title: Some("  Fixed  ".to_owned()),
                    ..SpecChanges::default()
                },
                &context
            ),
            Ok(changes),
            "a changed field comes back canonical"
        );
        assert!(
            check_changes(&SpecChanges::from(base), &context).is_err(),
            "the same legacy values are refused as changes"
        );
    }

    /// D5: an edit names only the fields that differ; equal specs change nothing.
    #[test]
    fn between_names_only_the_fields_that_differ() {
        let base = spec(&kind(ProjectId::new()));
        let graph_id = StepGraphId::new();
        let next = ItemSpec {
            body: "a new body".to_owned(),
            priority: -1,
            touched_paths: strings(&["src/**", "docs/*"]),
            step_graph_id: Some(graph_id),
            ..base.clone()
        };
        assert_eq!(
            SpecChanges::between(&base, &next),
            SpecChanges {
                body: Some("a new body".to_owned()),
                priority: Some(-1),
                touched_paths: Some(strings(&["src/**", "docs/*"])),
                step_graph_id: Some(Some(graph_id)),
                ..SpecChanges::default()
            }
        );
        assert_eq!(
            SpecChanges::between(&next, &base).step_graph_id,
            Some(None),
            "back to the kind default is a change"
        );
        let other_kind = ItemSpec {
            kind_id: ItemKindId::new(),
            title: "Other".to_owned(),
            required_tags: Vec::new(),
            ..base.clone()
        };
        assert_eq!(
            SpecChanges::between(&base, &other_kind),
            SpecChanges {
                kind_id: Some(other_kind.kind_id),
                title: Some("Other".to_owned()),
                required_tags: Some(Vec::new()),
                ..SpecChanges::default()
            }
        );
        assert!(SpecChanges::between(&base, &base.clone()).is_empty());
        assert!(SpecChanges::default().is_empty());
        assert!(!SpecChanges::from(base).is_empty());
    }

    /// D5 and the mint: every field reaches the store arguments; `reason = edited`.
    #[test]
    fn into_patch_and_into_new_item_carry_every_field() {
        let base = spec(&kind(ProjectId::new()));
        let graph_id = StepGraphId::new();
        let full = ItemSpec {
            step_graph_id: Some(graph_id),
            ..base.clone()
        };
        let (id, project, user, box_id) = (
            ItemId::new(),
            ProjectId::new(),
            UserId::new(),
            Some(BoxId::new()),
        );
        assert_eq!(
            full.clone().into_new_item(id, project, user, box_id),
            NewItem {
                id,
                project_id: project,
                kind_id: full.kind_id,
                title: full.title.clone(),
                body: full.body.clone(),
                required_tags: full.required_tags.clone(),
                touched_paths: full.touched_paths.clone(),
                priority: full.priority,
                step_graph_id: Some(graph_id),
                created_by: user,
                box_id,
            }
        );

        assert_eq!(
            SpecChanges::from(full.clone()).into_patch(user, box_id),
            ItemPatch {
                title: Some(full.title.clone()),
                body: Some(full.body.clone()),
                kind_id: Some(full.kind_id),
                required_tags: Some(full.required_tags.clone()),
                priority: Some(full.priority),
                touched_paths: Some(full.touched_paths.clone()),
                step_graph_id: Some(Some(graph_id)),
                author_id: user,
                box_id,
                reason: "edited".to_owned(),
            }
        );
        let clear = SpecChanges::between(&full, &base).into_patch(user, None);
        assert_eq!(clear.step_graph_id, Some(None), "`Some(None)` survives");
        assert_eq!(clear.reason, EDITED);
        assert_eq!(clear.title, None);
    }

    /// The edit form's base is the item as stored.
    #[test]
    fn of_reads_the_stored_columns() {
        let ours = kind(ProjectId::new());
        let graph_id = StepGraphId::new();
        let item = Item {
            id: ItemId::new(),
            project_id: ours.project_id,
            kind_id: ours.id,
            key_prefix: "ANA".to_owned(),
            key_number: 1,
            key: "ANA-1".to_owned(),
            title: "Fix it".to_owned(),
            body: "the body".to_owned(),
            status: Status::Open,
            priority: 3,
            required_tags: strings(&["Rust"]),
            touched_paths: strings(&["nope:x"]),
            step_graph_id: Some(graph_id),
            version: 4,
            created_by: UserId::new(),
            created_at: EPOCH,
            updated_at: EPOCH,
            closed_at: None,
            resolution: None,
        };
        assert_eq!(
            ItemSpec::of(&item),
            ItemSpec {
                required_tags: strings(&["Rust"]),
                touched_paths: strings(&["nope:x"]),
                step_graph_id: Some(graph_id),
                ..spec(&ours)
            },
            "unchecked: legacy values come through as stored"
        );
    }

    /// The form's text and the typed values round-trip on canonical values.
    #[test]
    fn text_round_trips() {
        let tags = strings(&["docker", "gpu", "rust"]);
        assert_eq!(parse_tags(&tags_text(&tags)), Ok(tags.clone()));
        assert_eq!(tags_text(&tags), "docker, gpu, rust");
        assert_eq!(parse_tags(&tags_text(&[])), Ok(Vec::new()));
        let paths = strings(&["src/**", "web:docs/*.md", "a/b:c"]);
        assert_eq!(parse_paths(&paths_text(&paths)), paths);
        assert_eq!(paths_text(&paths), "src/**\nweb:docs/*.md\na/b:c");
        assert!(parse_paths(&paths_text(&[])).is_empty());
    }

    /// E6: `body` and `touched_paths` print as lengths only.
    #[test]
    fn debug_prints_lengths_not_body_or_paths() {
        let secret = ItemSpec {
            body: "SECRET-BODY".to_owned(),
            touched_paths: strings(&["secret/dir/**"]),
            ..spec(&kind(ProjectId::new()))
        };
        for printed in [
            format!("{secret:?}"),
            format!("{:?}", SpecChanges::from(secret.clone())),
            format!("{secret:#?}"),
        ] {
            assert!(!printed.contains("SECRET-BODY"), "{printed}");
            assert!(!printed.contains("secret/dir"), "{printed}");
            assert!(
                printed.contains("Fix it"),
                "the title is printed: {printed}"
            );
        }
    }
}
