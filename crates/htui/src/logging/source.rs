//! Which part of htui a log line comes from (MOD-122 D3): a fixed table over `tracing` targets,
//! matched by the longest prefix at a module-path boundary.

/// Where a log line comes from (MOD-122 D3), in the requirement's order; `Other` last.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Source {
    /// Any `htui::…` target no other row claims: the shell, its tabs and widgets.
    Ui,
    /// The store: `htui_store`, the store worker, `sqlx`.
    Store,
    /// The orchestrator: `htui_orch`.
    Orchestrator,
    /// The agent runtimes: `htui_agent`, the agent worker.
    Agents,
    /// The MCP host and relay: `htui_mcp`, `htui::mcp_search`, `htui::mcp_cmd`.
    Mcp,
    /// The run runtime: `htui_worker` (the TUI's in-process one too), `htui::run_worker`,
    /// `htui::worker_cmd`.
    Worker,
    /// The secrets code: `htui_secrets`, `htui_store::secret`, `htui::secrets`,
    /// `htui::secrets_settings`.
    Secrets,
    /// Anything else, mostly third-party crates; the Logs tab shows it only when it has lines.
    Other,
}

impl Source {
    /// Every source, in the requirement's order (the Logs tab's list after `all`).
    pub const ALL: [Self; 8] = [
        Self::Ui,
        Self::Store,
        Self::Orchestrator,
        Self::Agents,
        Self::Mcp,
        Self::Worker,
        Self::Secrets,
        Self::Other,
    ];

    /// How many sources there are.
    pub const COUNT: usize = Self::ALL.len();

    /// This source's position in [`Self::ALL`].
    #[must_use]
    pub const fn index(self) -> usize {
        match self {
            Self::Ui => 0,
            Self::Store => 1,
            Self::Orchestrator => 2,
            Self::Agents => 3,
            Self::Mcp => 4,
            Self::Worker => 5,
            Self::Secrets => 6,
            Self::Other => 7,
        }
    }

    /// The name the Logs tab shows: `ui`, `store`, …, `other`.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Ui => "ui",
            Self::Store => "store",
            Self::Orchestrator => "orchestrator",
            Self::Agents => "agents",
            Self::Mcp => "mcp",
            Self::Worker => "worker",
            Self::Secrets => "secrets",
            Self::Other => "other",
        }
    }

    /// The source named `name` ([`Self::name`]), if any.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|source| source.name() == name)
    }

    /// The source of a `tracing` target: the longest prefix of the table that `target` equals or
    /// continues with `::`; then any `htui` / `htui::…` target is [`Self::Ui`]; anything else
    /// [`Self::Other`].
    #[must_use]
    pub fn of(target: &str) -> Self {
        TABLE
            .iter()
            .filter(|(prefix, _)| at_boundary(target, prefix))
            .max_by_key(|(prefix, _)| prefix.len())
            .map_or_else(
                || {
                    if at_boundary(target, "htui") {
                        Self::Ui
                    } else {
                        Self::Other
                    }
                },
                |(_, source)| *source,
            )
    }
}

/// `target` is `prefix` or a module path below it: `htui_storex` is not below `htui_store`.
fn at_boundary(target: &str, prefix: &str) -> bool {
    target == prefix
        || target
            .strip_prefix(prefix)
            .is_some_and(|rest| rest.starts_with("::"))
}

/// MOD-122 D3. Matched by longest prefix, so the order here does not matter.
const TABLE: &[(&str, Source)] = &[
    ("htui_secrets", Source::Secrets),
    ("htui_store::secret", Source::Secrets),
    ("htui::secrets", Source::Secrets),
    ("htui::secrets_settings", Source::Secrets),
    ("htui_store", Source::Store),
    ("htui::store_worker", Source::Store),
    ("sqlx", Source::Store),
    ("htui_orch", Source::Orchestrator),
    ("htui_agent", Source::Agents),
    ("htui::agent_worker", Source::Agents),
    ("htui_mcp", Source::Mcp),
    ("htui::mcp_search", Source::Mcp),
    ("htui::mcp_cmd", Source::Mcp),
    ("htui_worker", Source::Worker),
    ("htui::run_worker", Source::Worker),
    ("htui::worker_cmd", Source::Worker),
];

#[cfg(test)]
mod tests {
    use super::{Source, TABLE};

    /// D3: every row, and a module below each row, maps to the row's source.
    #[test]
    fn every_row_of_the_table_maps_to_its_source() {
        let expected = [
            ("htui_secrets", Source::Secrets),
            ("htui_store::secret", Source::Secrets),
            ("htui::secrets", Source::Secrets),
            ("htui::secrets_settings", Source::Secrets),
            ("htui_store", Source::Store),
            ("htui::store_worker", Source::Store),
            ("sqlx", Source::Store),
            ("htui_orch", Source::Orchestrator),
            ("htui_agent", Source::Agents),
            ("htui::agent_worker", Source::Agents),
            ("htui_mcp", Source::Mcp),
            ("htui::mcp_search", Source::Mcp),
            ("htui::mcp_cmd", Source::Mcp),
            ("htui_worker", Source::Worker),
            ("htui::run_worker", Source::Worker),
            ("htui::worker_cmd", Source::Worker),
        ];
        assert_eq!(TABLE.len(), expected.len());
        for (target, source) in expected {
            assert_eq!(Source::of(target), source, "{target}");
            let child = format!("{target}::engine");
            assert_eq!(Source::of(&child), source, "{child}");
        }
        assert_eq!(Source::of("htui_orch::engine"), Source::Orchestrator);
    }

    #[test]
    fn the_longest_prefix_wins() {
        assert_eq!(Source::of("htui_store::secret"), Source::Secrets);
        assert_eq!(Source::of("htui_store::secret::x"), Source::Secrets);
        assert_eq!(Source::of("htui::store_worker"), Source::Store);
        assert_eq!(Source::of("htui::agent_worker"), Source::Agents);
    }

    #[test]
    fn a_prefix_matches_only_at_a_path_boundary() {
        assert_eq!(Source::of("htui_storex"), Source::Other);
        assert_eq!(Source::of("htui::secretsx"), Source::Ui);
        assert_eq!(Source::of("sqlxx"), Source::Other);
        assert_eq!(Source::of("htuix"), Source::Other);
    }

    #[test]
    fn any_other_htui_target_is_ui_and_the_rest_is_other() {
        assert_eq!(Source::of("htui"), Source::Ui);
        assert_eq!(Source::of("htui::app::update"), Source::Ui);
        assert_eq!(Source::of("hyper::client"), Source::Other);
        assert_eq!(Source::of("log"), Source::Other);
    }

    #[test]
    fn sources_list_in_the_requirement_s_order() {
        let names: Vec<&str> = Source::ALL.iter().map(|source| source.name()).collect();
        assert_eq!(
            names,
            [
                "ui",
                "store",
                "orchestrator",
                "agents",
                "mcp",
                "worker",
                "secrets",
                "other"
            ]
        );
        assert_eq!(Source::COUNT, 8);
        for (index, source) in Source::ALL.into_iter().enumerate() {
            assert_eq!(source.index(), index);
            assert_eq!(Source::from_name(source.name()), Some(source));
        }
        assert_eq!(Source::from_name("all"), None);
    }
}
