//! The shell: header strip, "about this tab", pane layout, help overlay, modals.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph, Wrap};

use crate::app::{App, CHAT, Pane, Rects, TABS};
use crate::data::{Status, WsState};
use crate::theme::{self, FAINT, INK, Palette};
use crate::views;

const SUPERSCRIPT: [&str; 6] = ["¹", "²", "³", "⁴", "⁵", "⁶"];

pub fn draw(f: &mut Frame, app: &mut App) {
    let pal = Palette::new(app.scheme, app.tab);
    let [header, rest] = Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]).areas(f.area());
    let (tab_hits, ws_hit) = draw_header(f, app, header, pal);

    let body = if app.about {
        let [about, body] = Layout::vertical([Constraint::Length(4), Constraint::Fill(1)]).areas(rest);
        draw_about(f, app, about, pal);
        body
    } else {
        rest
    };

    let mut rects = Rects { tabs: tab_hits, workspace: ws_hit, body, screen: f.area(), ..Rects::default() };
    let t = app.cur();
    let l = t.layout;
    if t.zoom {
        match t.focus {
            Pane::List => rects.list = Some(body),
            Pane::Activity => rects.activity = Some(body),
            Pane::Composer => rects.composer = Some(body),
            Pane::Detail => rects.detail = Some(body),
        }
    } else {
        let main = if l.act_on {
            let pct = l.act_pct.clamp(10, 70);
            let [main, act] = if l.act_side {
                Layout::horizontal([Constraint::Fill(1), Constraint::Percentage(pct)]).areas(body)
            } else {
                Layout::vertical([Constraint::Fill(1), Constraint::Percentage(pct)]).areas(body)
            };
            rects.activity = Some(act);
            main
        } else {
            body
        };
        let detail = if l.list_on {
            let pct = l.list_pct.clamp(12, 70);
            let [list, detail] = Layout::horizontal([Constraint::Percentage(pct), Constraint::Fill(1)]).areas(main);
            rects.list = Some(list);
            detail
        } else {
            main
        };
        if app.tab == CHAT {
            let [transcript, composer] = Layout::vertical([Constraint::Fill(1), Constraint::Length(3)]).areas(detail);
            rects.detail = Some(transcript);
            rects.composer = Some(composer);
        } else {
            rects.detail = Some(detail);
        }
    }

    if let Some(list) = rects.list {
        let height = list.height.saturating_sub(2) as usize;
        let tree = &mut app.cur_mut().tree;
        if tree.cursor < tree.offset {
            tree.offset = tree.cursor;
        } else if height > 0 && tree.cursor >= tree.offset + height {
            tree.offset = tree.cursor + 1 - height;
        }
        rects.list_offset = tree.offset;
        views::draw_list(f, app, list, pal);
    }
    if let Some(detail) = rects.detail {
        let out = views::draw_detail(f, app, detail, pal);
        rects.field_rows = out.field_rows;
        rects.flow = out.flow;
        rects.detail_max_scroll = out.max_scroll;
        rects.subtabs = out.subtabs;
        if let Some(scroll) = out.scroll_to {
            app.cur_mut().detail_scroll = scroll;
        }
    }
    if let Some(composer) = rects.composer {
        views::draw_composer(f, app, composer, pal);
    }
    if let Some(act) = rects.activity {
        rects.act_rows = views::draw_activity(f, app, act, pal);
    }

    // Say how to get back what the layout hides.
    if let Some(detail) = rects.detail.or(rects.list).or(rects.activity).or(rects.composer) {
        let t = app.cur();
        let note = if t.zoom {
            Some(hint_line(&[("z", "unzoom")], pal))
        } else if !t.layout.list_on {
            Some(hint_line(&[("F2", "show list"), ("p", "layouts")], pal))
        } else {
            None
        };
        if let Some(note) = note {
            let w = note.width() as u16;
            if detail.width > w + 4 {
                f.render_widget(note, Rect::new(detail.x + 2, detail.bottom() - 1, w, 1));
            }
        }
    }
    // Scrolling stops at the end of the content.
    let max = rects.detail_max_scroll;
    let t = app.cur_mut();
    t.detail_scroll = t.detail_scroll.min(max);
    app.rects = rects;

    if app.modal.is_some() {
        crate::modal::draw(f, app, pal);
    }
    if app.help {
        draw_help(f, pal);
    }
}

/// One row: the workspace, the tabs, then status on the right. Returns tab and workspace hit ranges.
fn draw_header(f: &mut Frame, app: &App, area: Rect, pal: Palette) -> (Vec<(u16, u16, usize)>, (u16, u16)) {
    let line_style = Style::new().fg(FAINT);
    let ws_label = format!(" ▣ {} ", app.data.workspaces[app.workspace].name);
    let ws_width = ws_label.chars().count() as u16;
    let mut spans = vec![
        Span::styled("─", line_style),
        Span::styled(ws_label, Style::new().fg(theme::TEXT).bg(theme::SEL_BG).add_modifier(Modifier::BOLD)),
        Span::styled("─ ", line_style),
    ];
    let ws_hit = (area.x + 1, area.x + 1 + ws_width);
    let mut hits = Vec::new();
    let mut x = area.x + 1 + ws_width + 2;
    for (i, name) in TABS.iter().enumerate() {
        let color = if pal.per_tab { theme::tab_color(i) } else { theme::NEUTRAL };
        let label = format!(" {}{} ", SUPERSCRIPT[i], name);
        let width = label.chars().count() as u16;
        let style = if i == app.tab {
            Style::new().fg(INK).bg(color).add_modifier(Modifier::BOLD)
        } else if pal.per_tab {
            Style::new().fg(color)
        } else {
            theme::dim()
        };
        spans.push(Span::styled(label, style));
        hits.push((x, x + width, i));
        x += width;
        spans.push(Span::styled("─", line_style));
        x += 1;
    }

    let used_left: usize = spans.iter().map(|s| s.width()).sum();
    let right = header_right(app, pal, (area.width as usize).saturating_sub(used_left + 1));
    let used: usize = used_left + right.iter().map(|s| s.width()).sum::<usize>();
    let fill = (area.width as usize).saturating_sub(used);
    spans.push(Span::styled("─".repeat(fill), line_style));
    spans.extend(right);
    f.render_widget(Line::from(spans), area);
    (hits, ws_hit)
}

/// The right side of the header, dropping the least useful parts until it fits.
fn header_right(app: &App, pal: Palette, room: usize) -> Vec<Span<'static>> {
    let line_style = Style::new().fg(FAINT);
    // After a jump: where Backspace / Alt ← goes, and whether Alt → has somewhere to go.
    let mut back: Vec<Span<'static>> = match app.back_label() {
        Some(label) => vec![
            Span::styled(" ⌫ ", Style::new().fg(INK).bg(pal.accent).add_modifier(Modifier::BOLD)),
            Span::styled(format!(" back to {label} "), pal.accent()),
            Span::styled("─", line_style),
        ],
        None => vec![],
    };
    if app.can_forward() {
        back.push(Span::styled(" Alt→ ", pal.accent().add_modifier(Modifier::BOLD)));
        back.push(Span::styled("forward ", theme::dim()));
        back.push(Span::styled("─", line_style));
    }
    if let Some((text, _)) = &app.flash {
        let back_w: usize = back.iter().map(|s| s.width()).sum();
        let room = room.saturating_sub(1 + back_w) as u16;
        let note = fit(Line::from(Span::styled(format!(" {text} "), Style::new().fg(INK).bg(pal.accent).add_modifier(Modifier::BOLD))), room);
        let mut spans = back;
        spans.extend(note.spans);
        spans.push(Span::styled("─", line_style));
        return spans;
    }
    let counts = vec![
        Span::raw(" "),
        theme::glyph(Status::InProgress),
        Span::styled(format!(" {} working ", app.running(app.workspace)), theme::text()),
        theme::glyph(Status::AwaitingApproval),
        Span::styled(format!(" {} waiting ", app.needs_you(app.workspace)), theme::text()),
    ];
    let bg: Vec<usize> = (0..app.data.workspaces.len())
        .filter(|w| *w != app.workspace && matches!(app.data.workspaces[*w].state, WsState::Background | WsState::Stopping))
        .collect();
    let background = if bg.is_empty() {
        vec![]
    } else {
        let running: usize = bg.iter().map(|w| app.running(*w)).sum();
        vec![
            Span::styled("─ ", line_style),
            Span::styled(format!("+{} in background ", bg.len()), theme::dim()),
            theme::glyph(Status::InProgress),
            Span::styled(format!(" {running} "), theme::dim()),
        ]
    };
    let place = vec![Span::styled("─ ", line_style), Span::styled("DESKTOP-HTUI ", theme::dim())];
    let layout = vec![
        Span::styled("─ ", line_style),
        Span::styled(format!("{} ", app.cur().layout.preset.name()), theme::dim()),
    ];
    let keys = vec![
        Span::styled("─ ", line_style),
        Span::styled("F1", pal.accent().add_modifier(Modifier::BOLD)),
        Span::styled(" about ", theme::dim()),
        Span::styled("?", pal.accent().add_modifier(Modifier::BOLD)),
        Span::styled(" keys ", theme::dim()),
        Span::styled("─", line_style),
    ];
    let candidates = [
        [back.clone(), counts.clone(), background.clone(), place.clone(), layout.clone(), keys.clone()].concat(),
        [back.clone(), counts.clone(), background.clone(), place, keys.clone()].concat(),
        [back.clone(), counts.clone(), background, keys.clone()].concat(),
        [back.clone(), counts.clone(), keys].concat(),
        [back.clone(), counts.clone(), vec![Span::styled("─", line_style)]].concat(),
        [back, vec![Span::styled("─", line_style)]].concat(),
        [counts, vec![Span::styled("─", line_style)]].concat(),
    ];
    for c in &candidates {
        if c.iter().map(|s| s.width()).sum::<usize>() <= room {
            return c.clone();
        }
    }
    Vec::new()
}

/// Cuts a line to `width` cells, ending in `…` when anything was cut.
pub fn fit(line: Line<'static>, width: u16) -> Line<'static> {
    let width = width as usize;
    if line.width() <= width {
        return line;
    }
    let style = line.style;
    let mut out = Vec::new();
    let mut used = 0;
    for span in line.spans {
        let w = span.width();
        if used + w < width {
            used += w;
            out.push(span);
            continue;
        }
        let keep: String = span.content.chars().take(width.saturating_sub(used + 1)).collect();
        out.push(Span::styled(format!("{}…", keep.trim_end()), span.style));
        break;
    }
    Line::from(out).style(style)
}

fn draw_about(f: &mut Frame, app: &App, area: Rect, pal: Palette) {
    let (purpose, actions) = views::about(app.tab);
    let mut chips = Vec::new();
    for (key, what) in actions {
        chips.push(theme::chip(key, pal.accent));
        chips.push(Span::styled(format!(" {what}   "), theme::text()));
    }
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(pal.accent))
        .title(Line::from(vec![
            Span::styled(" ⓘ about ", pal.accent().add_modifier(Modifier::BOLD)),
            Span::styled(format!("{} ", TABS[app.tab]), theme::bold()),
        ]))
        .title_bottom(
            Line::from(vec![
                Span::styled(" F1", pal.accent().add_modifier(Modifier::BOLD)),
                Span::styled(" hide this · it stays hidden next time ", theme::dim()),
            ])
            .right_aligned(),
        );
    let width = area.width.saturating_sub(2);
    let text = vec![fit(Line::from(Span::styled(format!(" {purpose}"), theme::text())), width), fit(Line::from(chips), width)];
    f.render_widget(Paragraph::new(text).block(block), area);
}

/// A bordered pane with the title and hints in its border.
pub fn panel<'a>(
    title: Vec<Span<'a>>,
    right: Option<Line<'a>>,
    hints: &[(&str, &str)],
    focused: bool,
    pal: Palette,
) -> Block<'a> {
    let border = if focused { Style::new().fg(pal.accent) } else { Style::new().fg(FAINT) };
    let mut t = vec![Span::styled(" ", border)];
    t.extend(title);
    t.push(Span::raw(" "));
    let mut block = Block::bordered().border_type(BorderType::Rounded).border_style(border).title(Line::from(t));
    if let Some(right) = right {
        block = block.title_top(right.right_aligned());
    }
    if focused && !hints.is_empty() {
        block = block.title_bottom(hint_line(hints, pal).right_aligned());
    }
    block
}

pub fn hint_line<'a>(hints: &[(&str, &str)], pal: Palette) -> Line<'a> {
    let mut spans = vec![Span::raw(" ")];
    for (i, (key, what)) in hints.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(" · ", theme::dim()));
        }
        spans.push(Span::styled(key.to_string(), pal.accent().add_modifier(Modifier::BOLD)));
        spans.push(Span::styled(format!(" {what}"), theme::dim()));
    }
    spans.push(Span::raw(" "));
    Line::from(spans)
}

fn draw_help(f: &mut Frame, pal: Palette) {
    let area = f.area();
    let w = 100.min(area.width.saturating_sub(4));
    let h = 28.min(area.height.saturating_sub(2));
    let rect = Rect::new(area.x + (area.width - w) / 2, area.y + (area.height - h) / 2, w, h);
    f.render_widget(Clear, rect);

    let left: [(&str, &[(&str, &str)]); 3] = [
        ("move", &[
            ("1-6 / click", "switch tab"),
            ("w / click ▣", "workspaces tab"),
            ("Tab  S-Tab", "next / previous pane"),
            ("Ctrl ← →", "pane to the left / right"),
            ("↑↓ j k / wheel", "move inside the pane"),
            ("← →", "fold group · node in a graph"),
            ("Shift ← →  [ ]", "previous / next sub-tab"),
            ("Enter / Esc", "open / back to the list"),
            ("⌫ / Alt ←", "back through jumps"),
            ("Alt →", "forward again"),
            ("/", "filter the list"),
        ]),
        ("look", &[("F1", "about this tab")]),
        ("app", &[("q / Ctrl-C", "leave: work keeps running")]),
    ];
    let right: [(&str, &[(&str, &str)]); 5] = [
        ("layout", &[
            ("p / P", "next / previous preset"),
            ("< >  or drag", "list narrower / wider"),
            ("{ }  or drag", "activity smaller / bigger"),
            ("F2 / F3", "show or hide list / activity"),
            ("z", "zoom the focused pane"),
        ]),
        ("backlog", &[("N", "new item (AI rewrite, or talk it through)"), ("h · g", "done items · grouping"), ("v", "runs: graph / table")]),
        ("graphs", &[
            ("Enter", "open item · its chat · requirement"),
            ("M / drag a node", "move it (links stay as they are)"),
            ("+ - 0 · f · r", "zoom · reset · fit · re-layout"),
            ("L · m · d", "link labels · minimap · direction"),
        ]),
        ("chat", &[("i", "write (when the chat lets you)"), ("a A r", "allow · always · reject")]),
        ("activity / logs", &[("f", "filter events"), ("Enter", "jump to the source"), ("v · F", "log level · follow")]),
    ];
    let column = |sections: &[(&str, &[(&str, &str)])]| -> Vec<Line<'static>> {
        let mut lines = Vec::new();
        for (name, keys) in sections {
            lines.push(Line::from(Span::styled(format!(" {name}"), pal.accent().add_modifier(Modifier::BOLD))));
            for (key, what) in *keys {
                lines.push(Line::from(vec![
                    Span::styled(format!("   {key:<16}"), theme::bold()),
                    Span::styled(what.to_string(), theme::text()),
                ]));
            }
        }
        lines
    };
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(pal.accent))
        .title(Line::from(Span::styled(" keys ", pal.accent().add_modifier(Modifier::BOLD))))
        .title_bottom(Line::from(Span::styled(" Esc / ? close ", theme::dim())).right_aligned());
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    let [l, r] = Layout::horizontal([Constraint::Percentage(50), Constraint::Fill(1)]).areas(inner);
    f.render_widget(Paragraph::new(column(&left)).wrap(Wrap { trim: false }), l);
    f.render_widget(Paragraph::new(column(&right)).wrap(Wrap { trim: false }), r);
}
