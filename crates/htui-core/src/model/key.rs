//! MOD-147 M2: qualified item keys. A key is `PREFIX-N` and a prefix is `[A-Z][A-Z0-9]{1,15}`
//! (`ItemKind::prefix_is_valid`, `kind.rs`), so a key never holds `:`; a qualified key is
//! `<slug>:<KEY>`, and a workspace's own items are `@<workspace>:<KEY>` (the scope row's slug).

use std::collections::BTreeSet;

use crate::model::ids::ProjectId;

/// Splits at the **last** `:`: `(Some("api"), "FEAT-9")`, `(Some("@shop"), "WS-1")`,
/// `(None, "FEAT-9")` for a bare key. Halves are not validated: `":FEAT-1"` is
/// `(Some(""), "FEAT-1")`, `"api:"` is `(Some("api"), "")`.
#[must_use]
pub fn split_qualified(text: &str) -> (Option<&str>, &str) {
    match text.rsplit_once(':') {
        Some((slug, key)) => (Some(slug), key),
        None => (None, text),
    }
}

/// The text before the last `-`: `Some("FEAT")` for `FEAT-12`; `None` without a `-`.
#[must_use]
pub fn key_prefix(key: &str) -> Option<&str> {
    key.rsplit_once('-').map(|(prefix, _)| prefix)
}

/// D11: how one answer prints keys. A key prints `slug:KEY` when **another** project of the set has
/// a kind with the key's prefix, else bare. Built from `item_kinds` of every scope project, the
/// scope row included (its slug is `@<workspace>`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KeyQualifier {
    /// `(project, slug, prefixes)`, in the order given.
    projects: Vec<(ProjectId, String, BTreeSet<String>)>,
}

impl KeyQualifier {
    /// `(project, slug, its kinds' prefixes)` per scope project, taken by value.
    #[must_use]
    pub fn new(projects: impl IntoIterator<Item = (ProjectId, String, Vec<String>)>) -> Self {
        Self {
            projects: projects
                .into_iter()
                .map(|(project, slug, prefixes)| (project, slug, prefixes.into_iter().collect()))
                .collect(),
        }
    }

    /// D11's display form: `slug:KEY` on a shared prefix, else `KEY`; bare for an unknown project
    /// or a key with no prefix.
    #[must_use]
    pub fn display(&self, project: ProjectId, key: &str) -> String {
        let Some(prefix) = key_prefix(key) else {
            return key.to_owned();
        };
        let Some(slug) = self.slug(project) else {
            return key.to_owned();
        };
        let shared = self
            .projects
            .iter()
            .any(|(other, _, prefixes)| *other != project && prefixes.contains(prefix));
        if shared {
            format!("{slug}:{key}")
        } else {
            key.to_owned()
        }
    }

    /// Always `slug:KEY` (`htui waiting --json`'s `qualified_key`); bare for an unknown project.
    #[must_use]
    pub fn qualified(&self, project: ProjectId, key: &str) -> String {
        self.slug(project)
            .map_or_else(|| key.to_owned(), |slug| format!("{slug}:{key}"))
    }

    /// The project's slug, if the set holds it (`--json`'s `project`).
    #[must_use]
    pub fn slug(&self, project: ProjectId) -> Option<&str> {
        self.projects
            .iter()
            .find(|(id, _, _)| *id == project)
            .map(|(_, slug, _)| slug.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn id(n: u128) -> ProjectId {
        ProjectId::from_uuid(Uuid::from_u128(n))
    }

    fn row(project: ProjectId, slug: &str, prefixes: &[&str]) -> (ProjectId, String, Vec<String>) {
        (
            project,
            slug.to_owned(),
            prefixes.iter().map(|p| (*p).to_owned()).collect(),
        )
    }

    #[test]
    fn split_qualified_splits_at_the_last_colon() {
        assert_eq!(split_qualified("api:FEAT-9"), (Some("api"), "FEAT-9"));
        assert_eq!(split_qualified("@shop:WS-1"), (Some("@shop"), "WS-1"));
        assert_eq!(split_qualified("FEAT-9"), (None, "FEAT-9"));
        assert_eq!(split_qualified("a:b:FEAT-1"), (Some("a:b"), "FEAT-1"));
        assert_eq!(split_qualified(":FEAT-1"), (Some(""), "FEAT-1"));
        assert_eq!(split_qualified("api:"), (Some("api"), ""));
    }

    #[test]
    fn key_prefix_is_the_text_before_the_last_dash() {
        assert_eq!(key_prefix("FEAT-12"), Some("FEAT"));
        assert_eq!(key_prefix("WS-1"), Some("WS"));
        assert_eq!(key_prefix("NOPE"), None);
    }

    #[test]
    fn a_shared_prefix_is_qualified_and_a_unique_one_bare() {
        let (api, web) = (id(1), id(2));
        let keys = KeyQualifier::new([
            row(api, "api", &["FEAT", "TOOL"]),
            row(web, "web", &["FEAT"]),
        ]);
        assert_eq!(keys.display(api, "FEAT-1"), "api:FEAT-1");
        assert_eq!(keys.display(web, "FEAT-1"), "web:FEAT-1");
        assert_eq!(keys.display(api, "TOOL-1"), "TOOL-1");
    }

    #[test]
    fn the_scope_row_qualifies_with_its_at_slug() {
        let (shop, api) = (id(1), id(2));
        let keys = KeyQualifier::new([row(shop, "@shop", &["WS"]), row(api, "api", &["WS"])]);
        assert_eq!(keys.display(shop, "WS-1"), "@shop:WS-1");
        assert_eq!(keys.display(api, "WS-2"), "api:WS-2");

        let alone = KeyQualifier::new([row(shop, "@shop", &["WS"])]);
        assert_eq!(alone.display(shop, "WS-1"), "WS-1");
    }

    #[test]
    fn unknown_projects_and_bare_text_print_bare() {
        let (api, web) = (id(1), id(2));
        let keys = KeyQualifier::new([row(api, "api", &["FEAT"]), row(web, "web", &["FEAT"])]);
        assert_eq!(
            keys.display(id(9), "FEAT-1"),
            "FEAT-1",
            "an unknown project"
        );
        assert_eq!(keys.display(api, "FEAT"), "FEAT", "a key with no prefix");
        assert_eq!(KeyQualifier::default().display(api, "FEAT-1"), "FEAT-1");
        assert_eq!(keys.qualified(id(9), "FEAT-1"), "FEAT-1");
        assert_eq!(keys.slug(id(9)), None);
    }

    #[test]
    fn qualified_always_names_the_slug() {
        let (api, web) = (id(1), id(2));
        let keys = KeyQualifier::new([
            row(api, "api", &["FEAT", "TOOL"]),
            row(web, "web", &["FEAT"]),
        ]);
        assert_eq!(keys.qualified(api, "TOOL-1"), "api:TOOL-1");
        assert_eq!(keys.qualified(web, "FEAT-3"), "web:FEAT-3");
        assert_eq!(keys.slug(api), Some("api"));
    }
}
