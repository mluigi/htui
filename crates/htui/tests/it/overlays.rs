//! The workspace switcher and the migration prompt at 80 columns (MOD-115 M4 E6), end to end
//! through a `Harness` over the demo world: on the kit's modal each keeps every datum it shows at
//! 100 columns, and its keys on the bottom border. `tests/shell.rs` covers their behaviour at the
//! default 100x30.
#![cfg(feature = "testkit")]

use htui::agent_worker::AgentRuntime;
use htui::app::register_all;
use htui::testkit::Harness;
use htui::ui::overlay::workspace_switcher::WorkspaceSwitcher;
use htui_agent::registry::DriverFactory;

/// `harness` with the agent runtime and every view, settled.
async fn settled(harness: Harness) -> Harness {
    let mut harness = harness.with_agent_runtime(AgentRuntime::new(DriverFactory::new()));
    register_all(harness.app());
    harness.drive_to_end().await;
    harness
}

/// The lines of the modal titled `title`: from its title row to its bottom border, cut at the
/// box's edges, so the Backlog drawn around it is not read.
fn modal(frame: &str, title: &str) -> Vec<String> {
    let lines: Vec<&str> = frame.lines().collect();
    let (top, column, width) = lines
        .iter()
        .enumerate()
        .find_map(|(index, line)| {
            let byte = line.find(&format!("\u{256d} {title} "))?;
            let column = line[..byte].chars().count();
            let width = line[byte..].chars().position(|c| c == '\u{256e}')? + 1;
            Some((index, column, width))
        })
        .unwrap_or_else(|| panic!("the {title} box is drawn: {frame}"));
    let cut: Vec<String> = lines[top..]
        .iter()
        .map(|line| line.chars().skip(column).take(width).collect::<String>())
        .collect();
    let bottom = cut
        .iter()
        .position(|line| line.starts_with('\u{2570}'))
        .expect("the box's bottom border");
    cut[..=bottom].to_vec()
}

#[tokio::test]
async fn the_switcher_at_80_columns_keeps_every_row_and_its_keys() {
    let mut harness =
        settled(Harness::demo().with_overlay(Box::new(WorkspaceSwitcher::new()))).await;
    harness.resize(80, 30);
    let frame = harness.render();
    let inside = modal(&frame, "Workspaces");
    let text = inside.join("\n");
    for datum in [
        "Graphics  1 project",
        "Platform  2 projects",
        "j/k move \u{b7} Enter switch \u{b7} Esc close",
    ] {
        assert!(text.contains(datum), "`{datum}`: {frame}");
    }
    assert!(
        inside.last().is_some_and(|line| line.contains("Esc close")),
        "the keys ride the bottom border: {frame}"
    );
    insta::assert_snapshot!("switcher_80", frame);
}

#[tokio::test]
async fn the_migration_prompt_at_80_columns_keeps_its_question_and_keys() {
    let mut harness = settled(Harness::demo().with_store_state("online", Some(3))).await;
    harness.resize(80, 30);
    let frame = harness.render();
    let inside = modal(&frame, "Schema");
    let text = inside.join("\n");
    assert!(
        text.contains("3 schema migrations are pending. Apply them now?"),
        "{frame}"
    );
    assert!(
        inside
            .last()
            .is_some_and(|line| line.contains("y apply \u{b7} n/Esc stay offline")),
        "the keys ride the bottom border: {frame}"
    );
    insta::assert_snapshot!("migration_80", frame);
}
