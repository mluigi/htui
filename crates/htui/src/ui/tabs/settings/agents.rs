//! The agent registry section of the Settings tab (`R-TUI-8`, MOD-2 D14, D54).
//!
//! It lists what `agent` and this box's `agent_box` hold, and it is where a probe is asked for:
//! `r` sends [`StoreRequest::ProbeAgents`] and the `on this box` column then says what this box
//! can actually run (`R-AGT-6`). The caps banner is milestone 3's and the quota column milestone
//! 7's — each is a column in this table and a field in the reply, not a rewrite of the section.
//!
//! One cost of answering the probe with the reply this section already reads: **any**
//! [`StoreReply::Agents`] clears the in-flight state, so a re-activation or a scope change while a
//! probe is running puts the pre-probe rows back for a moment. The probe's own reply supersedes
//! them when it lands, and the alternative was a second reply variant nothing else would ever use.

use htui_core::model::{AgentSummary, Scope};
use ratatui::Frame;
use ratatui::layout::{Constraint, Rect};
use ratatui::text::Line;
use ratatui::widgets::{Cell, Row, Table};
use serde_json::Value;

use crate::app::{Action, Ctx, Handled};
use crate::store_worker::{StoreReply, StoreRequest};
use crate::ui::tabs::settings::{SectionId, SettingsSection, message};
use crossterm::event::{KeyCode, KeyEvent};

/// What the `on this box` column reads while this box has no `agent_box` row for the agent.
const NOT_PROBED: &str = "not probed";

/// What the `on this box` column reads while a probe this section asked for is in flight.
const PROBING: &str = "probing\u{2026}";

/// What replaces an absent `default_model`.
const NONE: &str = "\u{2014}";

/// The registry as a table of name, transport, billing, models, default, enabled and per-box
/// state.
#[derive(Debug, Default)]
pub struct AgentsSection {
    /// The registry, ordered by name.
    agents: Vec<AgentSummary>,
    /// Set when the store could not answer. Since MOD-2 milestone 4 mirrored `agent` (plan D31) an
    /// offline backend answers this section, so in practice only a store that is neither online nor
    /// holding a mirror gets here.
    unavailable: Option<String>,
    /// `r` was pressed and no reply has come back yet. A second `r` is refused (plan D54): a probe
    /// spawns a process per agent, and two of them racing to write the same `agent_box` rows would
    /// be paid for twice for one answer.
    probing: bool,
}

impl AgentsSection {
    /// Identity of the agents section.
    pub const ID: SectionId = SectionId("agents");

    /// A section with nothing read yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// What the `on this box` column says about one row, in the order plan D54 sets.
    ///
    /// A probe in flight wins over everything, because none of the rows on screen answer the
    /// question the user just asked. Then a box with no `agent_box` row at all. Then the snapshot's
    /// own verdict when it is not `ready` — `missing`, `unauthenticated` and `failed` are the three
    /// facts a version string cannot express (plan D50). Everything left is a row that works, or
    /// one written before the `probe` column existed, and both are answered by the version.
    fn on_box_cell(&self, summary: &AgentSummary) -> String {
        if self.probing {
            return PROBING.to_owned();
        }
        let Some(row) = summary.on_box.as_ref() else {
            return NOT_PROBED.to_owned();
        };
        let status = row
            .probe
            .as_ref()
            .and_then(|probe| probe.get("status"))
            .and_then(Value::as_str);
        if let Some(status @ ("missing" | "unauthenticated" | "failed")) = status {
            return status.to_owned();
        }
        let version = row.version.as_deref().unwrap_or(NONE);
        if row.enabled {
            version.to_owned()
        } else {
            format!("{version} (off)")
        }
    }
}

impl SettingsSection for AgentsSection {
    fn id(&self) -> SectionId {
        Self::ID
    }

    fn title(&self) -> &str {
        "Agents"
    }

    fn wants_requests(&self, _scope: &Scope) -> Vec<StoreRequest> {
        // Unscoped: `agent` is global, so the read does not change with the workspace.
        vec![StoreRequest::Agents]
    }

    fn on_scope_change(&mut self, _scope: &Scope) {}

    fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        // `r` is free: the global table binds `q`, `?`, the digits, `ctrl-c` and `-`, and the tab
        // itself consumes `h`/`l`/`[`/`]`/arrows before a section is offered the key.
        match key.code {
            KeyCode::Char('r') if !self.probing => {
                self.probing = true;
                ctx.request(StoreRequest::ProbeAgents);
                Handled::Consumed
            }
            KeyCode::Char('r') => {
                ctx.emit(Action::Error("a probe is already running".to_owned()));
                Handled::Consumed
            }
            _ => Handled::Pass,
        }
    }

    fn on_reply(&mut self, reply: &StoreReply, _ctx: &mut Ctx<'_>) {
        match reply {
            StoreReply::Agents(agents) => {
                self.agents = agents.clone();
                self.unavailable = None;
                self.probing = false;
            }
            // The shell has already put the message on the status line (`App::update`), so the
            // section's whole job here is to stop saying a probe is running.
            StoreReply::Failed { request, .. } if *request == "probe_agents" => {
                self.probing = false;
            }
            // `agent` is mirrored since MOD-2 milestone 4 (plan D31), so an offline backend
            // answers this read from the mirror; `agent_box` is not, which is why every offline
            // row's `on this box` column reads `not probed`. A refusal is therefore a store that
            // can reach neither the server nor a mirror, and saying so beats rendering an empty
            // table that reads as "no agents registered".
            StoreReply::Failed { request, message } if *request == "agents" => {
                self.agents.clear();
                self.unavailable = Some(message.clone());
            }
            _ => {}
        }
    }

    fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        if self.unavailable.is_some() {
            message(frame, area, "agent registry needs Postgres", ctx.theme);
            return;
        }
        if self.agents.is_empty() {
            message(frame, area, "no agents registered", ctx.theme);
            return;
        }

        let rows = self.agents.iter().map(|summary| {
            let agent = &summary.agent;
            let on_box = self.on_box_cell(summary);
            let style = if agent.enabled {
                ctx.theme.base
            } else {
                ctx.theme.dim
            };
            Row::new(vec![
                Cell::from(Line::styled(agent.name.clone(), style)),
                Cell::from(Line::styled(agent.transport.as_str(), ctx.theme.dim)),
                Cell::from(Line::styled(agent.billing.as_str(), ctx.theme.dim)),
                Cell::from(Line::styled(agent.models.len().to_string(), ctx.theme.dim)),
                Cell::from(Line::styled(
                    agent
                        .default_model
                        .clone()
                        .unwrap_or_else(|| NONE.to_owned()),
                    ctx.theme.dim,
                )),
                Cell::from(Line::styled(
                    if agent.enabled { "yes" } else { "no" },
                    style,
                )),
                Cell::from(Line::styled(on_box, ctx.theme.dim)),
            ])
        });

        let header = Row::new(vec![
            Cell::from("name"),
            Cell::from("transport"),
            Cell::from("billing"),
            Cell::from("models"),
            Cell::from("default"),
            Cell::from("enabled"),
            Cell::from("on this box"),
        ])
        .style(ctx.theme.title);

        let table = Table::new(
            rows,
            [
                Constraint::Length(12),
                Constraint::Length(9),
                Constraint::Length(12),
                Constraint::Length(6),
                Constraint::Length(9),
                Constraint::Length(7),
                Constraint::Min(11),
            ],
        )
        .header(header);
        frame.render_widget(table, area);
    }
}
