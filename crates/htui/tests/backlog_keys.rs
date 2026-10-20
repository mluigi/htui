//! MOD-67 M5 (D12): the Backlog's keys through its context stacks, end to end through
//! `Harness`. One module per lane, each in its own file under `tests/backlog_keys/`, so the
//! lanes' edits never meet: `l_a` (the tab, its forms, Notes and Docs), `l_b` (Runs), `l_c`
//! (Body, Graph, Prompt, Reqs). T-close adds the merged-tree tests after the three modules.
#![cfg(feature = "testkit")]

#[path = "backlog_keys/l_a.rs"]
mod l_a;
#[path = "backlog_keys/l_b.rs"]
mod l_b;
#[path = "backlog_keys/l_c.rs"]
mod l_c;

// The merged-tree tests (blueprint §7 step 2). Two of its four named cases are re-runs, not new
// code: L-B's and L-C's rebinding tests
// (`l_b::a_rebound_next_step_moves_the_cursor_and_j_capital_is_inert`,
// `l_c::a_rebound_cite_key_opens_the_picker_and_c_is_inert`) and L-A's
// `l_a::the_replay_miss_names_a_rebound_step_key` hold here through the converted tab.

use htui::testkit::Harness;
use htui_agent::fake::FakeAdapter;
use htui_core::store::MemStore;

/// The keys line the status row listed before MOD-115 (nothing draws it any more).
fn status_line(harness: &mut Harness) -> String {
    harness.app().keys_line()
}

/// The `?` box's lines, one per layer: the rows between its borders, each continuation row
/// (indented two cells) joined onto the line it wraps.
fn help_box(frame: &str) -> Vec<String> {
    let rows: Vec<Vec<char>> = frame.lines().map(|row| row.chars().collect()).collect();
    let (top, left) = rows
        .iter()
        .enumerate()
        .find_map(|(at, row)| {
            let text: String = row.iter().collect();
            let byte = text.find("┌ Keys")?;
            Some((at, text[..byte].chars().count()))
        })
        .unwrap_or_else(|| panic!("the `?` box is open:\n{frame}"));
    let mut lines: Vec<String> = Vec::new();
    for row in &rows[top + 1..] {
        if row.get(left) == Some(&'└') {
            break;
        }
        let inner: String = row[left + 1..].iter().take_while(|&&c| c != '│').collect();
        let inner = inner.trim_end();
        match (inner.strip_prefix("  "), lines.last_mut()) {
            (Some(rest), Some(line)) => {
                line.push_str(" · ");
                line.push_str(rest);
            }
            _ => lines.push(inner.to_owned()),
        }
    }
    lines
}

/// The browse status line of a lone Backlog tab: `Stack::BASE`'s, and the pane keys of a list +
/// detail tab (MOD-115 M2 T7).
const BROWSE: &str = "q quit · Ctrl+PgDn next tab · Ctrl+PgUp previous tab · 1 select tab · \
                      ? help · F9 about · Tab next pane · Shift+Tab previous pane";

/// The status line of a capturing mode in a lone Backlog tab (D9).
const CAPTURING: &str = "Ctrl+c quit · F1 help · F9 about";

/// D12: on the Runs pane with a step under the cursor, the status line is the browse one and the
/// `?` box lists every layer of `RUNS_BROWSE`, in order: the pane's own verbs, the answers
/// (MOD-115 M2 D7: `answer.option_*`, a parked request's digits), the pane scroll and sub-tab
/// keys, the Backlog's verbs, `Common`, the list, the global keys, the closer.
#[tokio::test]
async fn the_runs_pane_status_line_and_box_list_every_layer() {
    let adapter = FakeAdapter::new();
    let mut harness = l_b::driving(MemStore::demo(), &adapter).await;
    for _ in 0..l_b::TO_FEAT_1 {
        harness.key("j");
        harness.drive().await;
    }
    harness.key("l");
    harness.drive().await;
    let frame = harness.render();
    assert!(
        frame.contains("┌ FEAT-1") && frame.contains("▸ 0.1"),
        "FEAT-1's Runs pane, a step under the cursor:\n{frame}"
    );
    assert_eq!(status_line(&mut harness), BROWSE, "the browse status line");

    harness.key("?");
    let frame = harness.render();
    assert_eq!(
        help_box(&frame),
        [
            "Runs: J next step · K previous step · a approve · x reject · r retry step · \
             p promote to chat · s select winner · A accept output · o open output · \
             c cancel run · T retry cleanup · u unblock · R start run · C close out · \
             i follow up · v list / flow · + zoom in · - zoom out · = fit the run",
            "Answer: 1-9 answer",
            "Pane: PgDn page down · PgUp page up · l/]/Shift+Right next sub-tab · \
             h/[/Shift+Left previous sub-tab",
            "Backlog: Enter replay step · m open graph · f filter · F clear filter · \
             N new item · Q queue / dequeue · P pause / resume queue",
            "Common: e edit",
            "List: j/Down down · k/Up up · g/Home top · G/End bottom · Enter fold",
            // MOD-115 M2 T7: the Backlog claims a list and a detail, so the layout and pane keys
            // act here; Runs keeps `p` (promote) and `P` (pause queue), so no preset key is
            // offered (blueprint D-1). The history keys act on every tab.
            "Layout: > wider list · < narrower list · }/Ctrl+Down larger activity · \
             {/Ctrl+Up smaller activity · F2/Alt+1 show/hide list · F3/Alt+2 show/hide activity · \
             z zoom",
            "History: Backspace/Alt+Left back · Alt+Right forward",
            "Global: q/Ctrl+c quit · Ctrl+PgDn next tab · Ctrl+PgUp previous tab · 1-9 select tab · \
             ?/F1 help · F9 about · Tab/Ctrl+Right next pane · Shift+Tab/Ctrl+Left previous pane",
            "?/F1 closes this box",
        ],
        "the box:\n{frame}"
    );
    assert_eq!(
        status_line(&mut harness),
        BROWSE,
        "the box keeps the status line"
    );
}

/// D9, D12: a capturing pane reports its modal stack. The Runs rejection note shows the capturing
/// status line and a box of the one `Global` line; the Reqs picker's box lists its own layer,
/// `Common`, the list moves it admits and the global keys (the lanes only saw `F1` open a box).
#[tokio::test]
async fn a_capturing_pane_drives_the_status_line_and_box() {
    let mut harness = l_b::parked(l_b::seeded_store().await).await;
    harness.key("x");
    let frame = harness.render();
    assert!(frame.contains("reject with a note:"), "the note:\n{frame}");
    assert_eq!(status_line(&mut harness), CAPTURING, "the note captures");
    harness.key("f1");
    let frame = harness.render();
    assert_eq!(
        help_box(&frame),
        [
            // MOD-115 M2 T6: the note's field lists its editing keys.
            "Text field: Enter submit · Esc cancel · Left cursor left · Right cursor right · \
             Home line start · End line end · Backspace delete back · Del delete forward · \
             Up line up · Down line down · PgUp page up · PgDn page down",
            "Global: Ctrl+c quit · F1 help · F9 about",
            "F1 closes this box"
        ],
        "the note's box:\n{frame}"
    );

    let mut harness = l_c::backlog().await;
    l_c::on_feat_1_reqs(&mut harness).await;
    harness.key("c");
    harness.drive_to_end().await;
    let frame = harness.render();
    assert!(frame.contains("Enter pick"), "the picker:\n{frame}");
    assert_eq!(status_line(&mut harness), CAPTURING, "the picker captures");
    harness.key("f1");
    let frame = harness.render();
    assert_eq!(
        help_box(&frame),
        [
            "Reqs: J next row · K previous row · Enter pick",
            "Common: Esc back",
            "List: j/Down down · k/Up up",
            "Global: Ctrl+c quit · F1 help · F9 about",
            "F1 closes this box",
        ],
        "the picker's box:\n{frame}"
    );
}
