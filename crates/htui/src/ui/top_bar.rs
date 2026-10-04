//! The one-line header: `workspace · box · store · N working · M waiting` (`R-TUI-1`,
//! `R-TUI-11`).
//!
//! Every field comes from [`TopBarState`], which only [`App::observe_reply`](crate::app::App)
//! writes, so the header cannot disagree with what the store answered.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::app::TopBarState;
use crate::ui::Theme;

/// Separator between the header's fields.
const SEP: &str = " · ";

/// MOD-80 D6: the store label in `warning` while the store is degraded — `connecting`, or
/// `offline · <age>` (`Backend::label`) — and `base` for `online` and `memory`.
fn store_style(label: &str, theme: &Theme) -> Style {
    if label == "connecting" || label.starts_with("offline") {
        theme.warning
    } else {
        theme.base
    }
}

/// Draws the top bar into a one-line area.
pub fn render(frame: &mut Frame<'_>, area: Rect, state: &TopBarState, theme: &Theme) {
    let workspace = if state.workspace.is_empty() {
        "no workspace"
    } else {
        &state.workspace
    };
    let box_name = if state.box_name.is_empty() {
        "no box"
    } else {
        &state.box_name
    };
    let (working, waiting) = state
        .waiting
        .as_ref()
        .map_or((0, 0), |view| (view.working, view.waiting()));
    // MOD-69 plan T3.3, MOD-80 D6: the waiting part in `warning` when a person is owed
    // something; no plural rule, so "1 working · 1 waiting" reads as written.
    let waiting_style = if waiting > 0 {
        theme.warning
    } else {
        theme.base
    };
    let line = Line::from(vec![
        Span::styled(workspace.to_owned(), theme.title),
        Span::styled(SEP, theme.dim),
        Span::styled(box_name.to_owned(), theme.base),
        Span::styled(SEP, theme.dim),
        Span::styled(state.store.clone(), store_style(&state.store, theme)),
        Span::styled(SEP, theme.dim),
        Span::styled(format!("{working} working"), theme.base),
        Span::styled(SEP, theme.dim),
        Span::styled(format!("{waiting} waiting"), waiting_style),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use htui_core::model::ItemId;
    use htui_worker::{WaitingReason, WaitingRow, WaitingView};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::style::Color;

    /// The top bar of `state`, drawn 80 cells wide.
    fn draw(state: &TopBarState, theme: &Theme) -> Buffer {
        let mut terminal = Terminal::new(TestBackend::new(80, 1)).expect("a test terminal");
        terminal
            .draw(|frame| render(frame, frame.area(), state, theme))
            .expect("draw");
        terminal.backend().buffer().clone()
    }

    fn row(key: &str) -> WaitingRow {
        WaitingRow {
            item: ItemId::new(),
            item_key: key.to_owned(),
            run: None,
            step: None,
            step_label: String::new(),
            reason: WaitingReason::Unblock,
            text: "blocked".to_owned(),
        }
    }

    fn line(buffer: &Buffer) -> String {
        (0..buffer.area.width)
            .map(|x| buffer[(x, 0)].symbol())
            .collect::<String>()
            .trim_end()
            .to_owned()
    }

    /// MOD-69 plan T3.3, MOD-80 D6: `M waiting` is a warning exactly when a person is owed
    /// something.
    #[test]
    fn the_waiting_count_is_a_warning_only_when_non_zero() {
        let theme = Theme::default();
        let mut state = TopBarState {
            workspace: "Platform".to_owned(),
            box_name: "DESKTOP-HTUI".to_owned(),
            store: "memory".to_owned(),
            waiting: Some(WaitingView {
                working: 1,
                rows: Vec::new(),
                permissions_known: true,
                offline: false,
            }),
        };
        let idle = draw(&state, &theme);
        let text = line(&idle);
        assert_eq!(
            text,
            "Platform · DESKTOP-HTUI · memory · 1 working · 0 waiting"
        );
        let at = u16::try_from(text.chars().count() - "0 waiting".len()).expect("fits");
        assert_ne!(
            idle[(at, 0)].style().fg,
            theme.warning.fg,
            "nothing owed, no warning"
        );

        state.waiting = Some(WaitingView {
            working: 0,
            rows: vec![row("FEAT-2"), row("FEAT-3")],
            permissions_known: true,
            offline: false,
        });
        let owed = draw(&state, &theme);
        let text = line(&owed);
        assert_eq!(
            text,
            "Platform · DESKTOP-HTUI · memory · 0 working · 2 waiting"
        );
        let at = u16::try_from(text.chars().count() - "2 waiting".len()).expect("fits");
        assert_eq!(owed[(at, 0)].symbol(), "2");
        assert_eq!(owed[(at, 0)].style().fg, theme.warning.fg);
        let byte = text.find("0 working").expect("working");
        let working = u16::try_from(text[..byte].chars().count()).expect("fits");
        assert_ne!(
            owed[(working, 0)].style().fg,
            theme.warning.fg,
            "working is never a warning"
        );
    }

    /// MOD-80 D6: `connecting` and `offline · <age>` are degraded; `online` and `memory` are not.
    #[test]
    fn store_style_warns_only_while_degraded() {
        let theme = Theme::default();
        assert_eq!(store_style("online", &theme), theme.base);
        assert_eq!(store_style("memory", &theme), theme.base);
        assert_eq!(store_style("connecting", &theme), theme.warning);
        assert_eq!(store_style("offline · 3m", &theme), theme.warning);
    }

    /// MOD-80 review L3: `store_style` keys on the text `Backend::label` writes, so it is fed
    /// the real labels: a renamed label fails here rather than silently drawing `base`.
    #[tokio::test]
    async fn store_style_reads_the_labels_the_backend_writes() {
        use chrono::{TimeDelta, Utc};
        use htui_core::model::BoxId;
        use htui_core::store::MemStore;
        use htui_store::{Backend, CacheStore, Identity, PgStore};

        let theme = Theme::default();
        let root = tempfile::tempdir().expect("temp root");
        let cache = CacheStore::open(root.path(), "top-bar-labels", 1)
            .await
            .expect("open a throwaway mirror");
        let identity = Identity {
            box_id: BoxId::new(),
            hostname: "HTUI-TEST".to_owned(),
        };
        let pg = PgStore::lazy(
            "postgres://nobody:nothing@127.0.0.1:1/none",
            &identity,
            std::time::Duration::from_millis(250),
        )
        .expect("a lazy pool opens no socket");
        let cases = [
            (Backend::memory(MemStore::new()), theme.base),
            (
                Backend::Online {
                    pg,
                    cache: cache.clone(),
                },
                theme.base,
            ),
            (
                Backend::Offline {
                    cache: cache.clone(),
                    since: None,
                },
                theme.warning,
            ),
            (
                Backend::Offline {
                    cache,
                    since: Some(Utc::now() - TimeDelta::seconds(220)),
                },
                theme.warning,
            ),
        ];
        for (backend, style) in cases {
            let label = backend.label();
            assert_eq!(store_style(&label, &theme), style, "{label}");
        }
    }

    /// MOD-80 D6: the top bar draws a degraded store label as a warning.
    #[test]
    fn a_degraded_store_label_is_drawn_as_a_warning() {
        let theme = Theme::default();
        let warning = theme.warning.fg.expect("warning has a colour");
        let x = u16::try_from("Platform · DESKTOP-HTUI · ".chars().count()).expect("fits");
        let state = |store: &str| TopBarState {
            workspace: "Platform".to_owned(),
            box_name: "DESKTOP-HTUI".to_owned(),
            store: store.to_owned(),
            waiting: None,
        };
        for (store, first, fg) in [
            ("offline · 3m", "o", warning),
            ("connecting", "c", warning),
            ("online", "o", Color::Reset),
        ] {
            let buffer = draw(&state(store), &theme);
            assert_eq!(buffer[(x, 0)].symbol(), first, "{store}: {}", line(&buffer));
            assert_eq!(buffer[(x, 0)].fg, fg, "{store}");
        }
        assert_eq!(warning, Color::Yellow);
    }

    /// MOD-69 blueprint A-6: before the first reply the bar reads zero for both counts.
    #[test]
    fn before_the_first_reply_both_counts_read_zero() {
        let state = TopBarState::default();
        assert_eq!(
            line(&draw(&state, &Theme::default())),
            "no workspace · no box ·  · 0 working · 0 waiting"
        );
    }
}
