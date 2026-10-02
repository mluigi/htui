//! `Settings > Personas` (MOD-26 milestone 2, D22): the registry list, the fields form, the body
//! and rules editors, the delete question and the import, driven through a [`SectionBench`] with
//! replies built from the demo registry, and the frames a user sees through a [`Harness`].
#![cfg(feature = "testkit")]

use chrono::Duration;
use htui::app::Action;
use htui::persona_settings::PersonaWrite;
use htui::store_worker::{StoreReply, StoreRequest};
use htui::testkit::SectionBench;
use htui::ui::Theme;
use htui::ui::tabs::settings::personas::{
    COMMAND_RUN_IS_Y_OR_N, HINT_BROWSE, HINT_EDITOR, HINT_FORM_EDIT, UNCHANGED, UNSAVED,
};
use htui::ui::tabs::settings::{PersonasSection, SettingsSection};
use htui_core::fixtures::ids;
use htui_core::model::persona::{BLANK_PERSONA_BODY, allow_names_an_mcp_tool};
use htui_core::model::{
    Persona, PersonaAnswer, PersonaDefault, PersonaMatch, PersonaPatch, PersonaPermission,
    PersonaRule, PersonaTools,
};
use htui_core::store::{MemStore, WriteStore as _};
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::{Terminal, TerminalOptions, Viewport};

/// What a compare-and-set miss says under an open form (`settings/mod.rs`, private there).
const CHANGED_ELSEWHERE: &str = "changed elsewhere since you opened it \u{2014} reloaded; Enter retries against the current row";

/// What the section says when the row an editor was open on is gone (`settings/mod.rs`).
const DELETED_ELSEWHERE: &str = "deleted elsewhere \u{2014} the editor was closed";

/// The demo registry by name: `architect`, then `reviewer`.
async fn demo_rows() -> Vec<Persona> {
    MemStore::demo()
        .personas()
        .await
        .expect("the memory store never fails")
}

/// A bench and a section the demo registry has been delivered to.
async fn bench_with(rows: Vec<Persona>) -> (SectionBench, PersonasSection) {
    let bench = SectionBench::new().await;
    let mut section = PersonasSection::new();
    bench.reply(&mut section, &StoreReply::Personas(rows));
    let _ = bench.drained();
    (bench, section)
}

async fn bench_with_demo() -> (SectionBench, PersonasSection) {
    bench_with(demo_rows().await).await
}

/// Types `text` one key at a time.
fn type_text(bench: &SectionBench, section: &mut PersonasSection, text: &str) {
    for c in text.chars() {
        let chord = if c == ' ' {
            "space".to_owned()
        } else {
            c.to_string()
        };
        bench.key(section, &chord);
    }
}

/// Feeds each chord in turn.
fn keys(bench: &SectionBench, section: &mut PersonasSection, chords: &[&str]) {
    for chord in chords {
        bench.key(section, chord);
    }
}

/// The store requests the section emitted since the last drain.
fn requests(bench: &SectionBench) -> Vec<StoreRequest> {
    bench
        .drained()
        .into_iter()
        .filter_map(|action| match action {
            Action::Store(request) => Some(request),
            _ => None,
        })
        .collect()
}

/// The section drawn at 100x30, as text.
fn frame(bench: &SectionBench, section: &PersonasSection) -> String {
    bench.render_section(section, 100)
}

/// The frame's words joined by single spaces, so a wrapped sentence reads as one.
fn flat(frame: &str) -> String {
    frame.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Whether the frame holds `sentence`, wrapped or not.
fn shows(frame: &str, sentence: &str) -> bool {
    flat(frame).contains(&flat(sentence))
}

/// One section drawn into a 100x30 buffer: a refusal's colour is a style, not text.
fn drawn(bench: &SectionBench, section: &dyn SettingsSection) -> Buffer {
    let area = Rect::new(0, 0, 100, 30);
    let mut terminal = Terminal::with_options(
        TestBackend::new(100, 30),
        TerminalOptions {
            viewport: Viewport::Fixed(area),
        },
    )
    .expect("a test terminal");
    let ctx = bench.ctx();
    terminal
        .draw(|frame| section.render(frame, frame.area(), &ctx))
        .expect("the section draws");
    terminal.backend().buffer().clone()
}

/// What the section drew in the theme's error colour, rows joined by spaces.
fn error_text(bench: &SectionBench, section: &dyn SettingsSection) -> String {
    let error = Theme::default().error.fg.unwrap_or(Color::Reset);
    let buffer = drawn(bench, section);
    let rows: Vec<String> = (0..buffer.area.height)
        .filter_map(|y| {
            let text: String = (0..buffer.area.width)
                .filter(|x| buffer[(*x, y)].fg == error)
                .map(|x| buffer[(x, y)].symbol())
                .collect();
            let text = text.trim().to_owned();
            (!text.is_empty()).then_some(text)
        })
        .collect();
    rows.join(" ")
}

/// One form line as the section draws it: the label padded to the widest
/// (`permission-default`, 18), then `: ` and the text.
fn field(label: &str, text: &str) -> String {
    format!("{label:<18}: {text}")
}

/// The `UpdatePersona` the section sent, or a panic naming what went out.
#[track_caller]
fn one_update(
    sent: Vec<StoreRequest>,
) -> (
    htui_core::model::PersonaId,
    chrono::DateTime<chrono::Utc>,
    PersonaPatch,
) {
    match sent.as_slice() {
        [
            StoreRequest::UpdatePersona {
                id,
                expected,
                patch,
            },
        ] => (*id, *expected, patch.clone()),
        other => panic!("expected one `UpdatePersona`: {other:?}"),
    }
}

fn architect(rows: &[Persona]) -> Persona {
    rows.iter()
        .find(|row| row.id == ids::PERSONA_ARCHITECT)
        .expect("the demo registry holds the architect")
        .clone()
}

// ---- 9.3: the list -----------------------------------------------------------------------------

#[tokio::test]
async fn the_registry_lists_one_line_per_persona() {
    let (bench, section) = bench_with_demo().await;
    let frame = frame(&bench, &section);
    let lines: Vec<&str> = frame.lines().collect();

    assert!(
        lines[0].starts_with("architect \u{b7} Designs the change"),
        "{frame}"
    );
    assert!(
        lines[0].ends_with("\u{b7} deny edit,delete,move \u{b7} allow 0 \u{b7} rules 0"),
        "{frame}"
    );
    assert!(
        lines[0].contains('\u{2026}'),
        "the description is cut: {frame}"
    );
    assert_eq!(
        lines[1].trim(),
        "allow all \u{b7} disallowed none \u{b7} command-run y \u{b7} default inherit",
        "the detail line sits under the cursor row: {frame}"
    );
    assert!(lines[2].starts_with("reviewer \u{b7} Reviews"), "{frame}");
    assert!(frame.contains(HINT_BROWSE), "{frame}");
    insta::assert_snapshot!("browse", frame);
}

#[tokio::test]
async fn j_and_k_move_the_cursor_and_stop_at_the_ends() {
    let (bench, mut section) = bench_with_demo().await;
    bench.key(&mut section, "k");
    let at_top = frame(&bench, &section);
    assert!(
        at_top
            .lines()
            .nth(1)
            .is_some_and(|line| line.starts_with("    allow"))
    );

    keys(&bench, &mut section, &["j", "j", "j"]);
    let at_end = frame(&bench, &section);
    let lines: Vec<&str> = at_end.lines().collect();
    assert!(lines[0].starts_with("architect"), "{at_end}");
    assert!(lines[1].starts_with("reviewer"), "{at_end}");
    assert!(
        lines[2].starts_with("    allow"),
        "the detail line moved: {at_end}"
    );
}

#[tokio::test]
async fn wants_requests_is_the_unscoped_read() {
    let (_bench, section) = bench_with_demo().await;
    let scope = bench_scope().await;
    let wanted = section.wants_requests(&scope);
    assert!(
        matches!(wanted.as_slice(), [StoreRequest::Personas]),
        "{wanted:?}"
    );
}

async fn bench_scope() -> htui_core::model::Scope {
    let workspaces = MemStore::demo()
        .workspaces()
        .await
        .expect("the memory store never fails");
    htui_core::model::Scope::from_workspace(workspaces.first().expect("a workspace"))
}

// ---- 9.3: the create form and the new body -----------------------------------------------------

#[tokio::test]
async fn n_opens_the_fields_and_enter_moves_to_the_body() {
    let (bench, mut section) = bench_with_demo().await;
    bench.key(&mut section, "n");
    type_text(&bench, &mut section, "scout");
    bench.key(&mut section, "enter");

    let frame = frame(&bench, &section);
    assert!(frame.contains("body of new persona `scout`"), "{frame}");
    assert!(requests(&bench).is_empty(), "nothing is sent before Ctrl+S");
}

#[tokio::test]
async fn ctrl_s_in_the_new_body_sends_create_persona() {
    let (bench, mut section) = bench_with_demo().await;
    bench.key(&mut section, "n");
    type_text(&bench, &mut section, "scout");
    keys(&bench, &mut section, &["tab", "tab", "tab", "tab"]);
    type_text(&bench, &mut section, "execute");
    bench.key(&mut section, "enter");
    type_text(&bench, &mut section, "You scout.");
    bench.key(&mut section, "ctrl-s");

    let sent = requests(&bench);
    let [StoreRequest::CreatePersona { new }] = sent.as_slice() else {
        panic!("expected one `CreatePersona`: {sent:?}")
    };
    assert_eq!(new.name, "scout");
    assert_eq!(new.description, "");
    assert_eq!(new.body, "You scout.");
    assert_eq!(
        new.tools,
        PersonaTools {
            allow: Vec::new(),
            deny: Vec::new(),
            deny_kinds: vec!["execute".to_owned()],
            command_run: true,
        }
    );
    assert_eq!(new.permission, PersonaPermission::default());
}

#[tokio::test]
async fn esc_in_the_new_body_returns_to_the_fields_and_keeps_the_body() {
    let (bench, mut section) = bench_with_demo().await;
    bench.key(&mut section, "n");
    type_text(&bench, &mut section, "scout");
    bench.key(&mut section, "enter");
    type_text(&bench, &mut section, "kept text");
    bench.key(&mut section, "esc");

    let form = frame(&bench, &section);
    assert!(form.contains("new persona"), "{form}");
    assert!(form.contains("name"), "{form}");

    bench.key(&mut section, "enter");
    let body = frame(&bench, &section);
    assert!(body.contains("body of new persona `scout`"), "{body}");
    assert!(body.contains("kept text"), "{body}");
    assert!(requests(&bench).is_empty());
}

#[tokio::test]
async fn a_blank_new_body_is_refused_with_the_stores_sentence() {
    let (bench, mut section) = bench_with_demo().await;
    bench.key(&mut section, "n");
    type_text(&bench, &mut section, "scout");
    bench.key(&mut section, "enter");
    bench.key(&mut section, "ctrl-s");

    let frame = frame(&bench, &section);
    assert!(frame.contains(BLANK_PERSONA_BODY), "{frame}");
    assert!(requests(&bench).is_empty(), "nothing is sent");
}

#[tokio::test]
async fn a_created_reply_closes_the_editor_and_selects_the_row() {
    let (bench, mut section) = bench_with_demo().await;
    bench.key(&mut section, "n");
    type_text(&bench, &mut section, "scout");
    bench.key(&mut section, "enter");
    type_text(&bench, &mut section, "You scout.");
    bench.key(&mut section, "ctrl-s");
    let sent = requests(&bench);
    let [StoreRequest::CreatePersona { new }] = sent.as_slice() else {
        panic!("expected one `CreatePersona`: {sent:?}")
    };
    let mut rows = demo_rows().await;
    let now = chrono::Utc::now();
    rows.push(Persona {
        id: new.id,
        name: new.name.clone(),
        description: new.description.clone(),
        body: new.body.clone(),
        tools: new.tools.clone(),
        permission: new.permission.clone(),
        created_at: now,
        updated_at: now,
    });

    bench.reply(
        &mut section,
        &StoreReply::PersonaWritten {
            personas: rows,
            outcome: PersonaWrite::Created {
                id: new.id,
                name: "scout".to_owned(),
            },
        },
    );

    assert!(!section.captures_input(), "back to Browse");
    let frame = frame(&bench, &section);
    let lines: Vec<&str> = frame.lines().collect();
    assert!(lines[2].starts_with("scout"), "{frame}");
    assert!(
        lines[3].starts_with("    allow"),
        "the cursor is on the new row: {frame}"
    );
    assert!(frame.contains("created persona `scout`"), "{frame}");
}

// ---- 9.3: the edit form ------------------------------------------------------------------------

#[tokio::test]
async fn e_opens_the_prefilled_form() {
    let (bench, mut section) = bench_with_demo().await;
    bench.key(&mut section, "e");

    let frame = frame(&bench, &section);
    assert!(frame.contains("edit persona `architect`"), "{frame}");
    for line in [
        field("name", "architect"),
        field("deny-kinds", "edit, delete, move"),
        field("command-run (y/n)", "y"),
        field("tools", ""),
        field("permission-default", ""),
    ] {
        assert!(
            frame.lines().any(|row| row == line.trim_end()),
            "{line}: {frame}"
        );
    }
    assert!(frame.contains(HINT_FORM_EDIT), "{frame}");
    insta::assert_snapshot!("form", frame);
}

#[tokio::test]
async fn e_then_enter_sends_only_the_changed_fields() {
    let rows = demo_rows().await;
    let row = architect(&rows);
    let (bench, mut section) = bench_with(rows).await;
    keys(&bench, &mut section, &["e", "down"]);
    type_text(&bench, &mut section, " More.");
    bench.key(&mut section, "enter");

    let (id, expected, patch) = one_update(requests(&bench));
    assert_eq!(id, row.id);
    assert_eq!(expected, row.updated_at);
    assert_eq!(
        patch,
        PersonaPatch {
            description: Some(format!("{} More.", row.description)),
            ..PersonaPatch::default()
        }
    );
}

#[tokio::test]
async fn an_unchanged_form_sends_nothing() {
    let (bench, mut section) = bench_with_demo().await;
    keys(&bench, &mut section, &["e", "enter"]);

    assert!(requests(&bench).is_empty());
    assert!(!section.captures_input(), "the form closed");
    let frame = frame(&bench, &section);
    assert!(frame.contains(UNCHANGED), "{frame}");
    assert!(frame.contains(HINT_BROWSE), "{frame}");
}

fn one_rule() -> PersonaRule {
    PersonaRule {
        matcher: PersonaMatch {
            tool_kind: Some("execute".to_owned()),
            command_prefix: Some("rm -rf".to_owned()),
            ..PersonaMatch::default()
        },
        answer: PersonaAnswer::RejectOnce,
        reason: "never wipe".to_owned(),
    }
}

#[tokio::test]
async fn a_permission_default_edit_keeps_the_rules() {
    let mut rows = demo_rows().await;
    for row in &mut rows {
        if row.id == ids::PERSONA_ARCHITECT {
            row.permission.rules = vec![one_rule()];
        }
    }
    let (bench, mut section) = bench_with(rows).await;
    bench.key(&mut section, "e");
    keys(&bench, &mut section, &["tab"; 6]);
    type_text(&bench, &mut section, "deny");
    bench.key(&mut section, "enter");

    let (_, _, patch) = one_update(requests(&bench));
    assert_eq!(
        patch.permission,
        Some(PersonaPermission {
            default: Some(PersonaDefault::Deny),
            rules: vec![one_rule()],
        })
    );
    assert_eq!(patch.tools, None);
    assert_eq!(
        (patch.name, patch.description, patch.body),
        (None, None, None)
    );
}

#[tokio::test]
async fn a_form_refusal_is_the_stores_sentence_and_focuses_its_field() {
    let (bench, mut section) = bench_with_demo().await;
    keys(&bench, &mut section, &["e", "tab", "tab"]);
    type_text(&bench, &mut section, "mcp__x__y");
    keys(&bench, &mut section, &["tab", "tab"]);
    bench.key(&mut section, "enter");

    assert!(requests(&bench).is_empty(), "nothing is sent");
    let refused = frame(&bench, &section);
    assert!(
        shows(&refused, &allow_names_an_mcp_tool("mcp__x__y")),
        "{refused}"
    );
    type_text(&bench, &mut section, "Z");
    let typed = frame(&bench, &section);
    assert!(
        typed
            .lines()
            .any(|line| line.starts_with("tools") && line.contains("mcp__x__yZ")),
        "the focus moved to `tools`: {typed}"
    );
}

#[tokio::test]
async fn command_run_must_be_y_or_n() {
    let (bench, mut section) = bench_with_demo().await;
    bench.key(&mut section, "e");
    keys(&bench, &mut section, &["tab"; 5]);
    bench.key(&mut section, "backspace");
    type_text(&bench, &mut section, "maybe");
    keys(&bench, &mut section, &["tab", "enter"]);

    assert!(requests(&bench).is_empty(), "nothing is sent");
    assert!(frame(&bench, &section).contains(COMMAND_RUN_IS_Y_OR_N));
    type_text(&bench, &mut section, "Z");
    let typed = frame(&bench, &section);
    assert!(
        typed.contains(&field("command-run (y/n)", "maybeZ")),
        "{typed}"
    );
}

// ---- 9.3: the body editor ----------------------------------------------------------------------

#[tokio::test]
async fn b_opens_the_body_and_ctrl_s_sends_the_body_only() {
    let rows = demo_rows().await;
    let row = architect(&rows);
    let (bench, mut section) = bench_with(rows).await;
    bench.key(&mut section, "b");

    let opened = frame(&bench, &section);
    assert!(opened.contains("body of `architect`"), "{opened}");
    assert!(opened.contains(HINT_EDITOR), "{opened}");
    insta::assert_snapshot!("body", opened);

    type_text(&bench, &mut section, "Extra.");
    bench.key(&mut section, "ctrl-s");
    let (id, expected, patch) = one_update(requests(&bench));
    assert_eq!((id, expected), (row.id, row.updated_at));
    assert_eq!(
        patch,
        PersonaPatch {
            body: Some(format!("{}Extra.", row.body)),
            ..PersonaPatch::default()
        }
    );
}

#[tokio::test]
async fn h_and_l_type_into_the_body() {
    let rows = demo_rows().await;
    let row = architect(&rows);
    let (bench, mut section) = bench_with(rows).await;
    bench.key(&mut section, "b");
    assert!(section.captures_input(), "R-10: letters are text here");
    keys(&bench, &mut section, &["h", "l", "ctrl-s"]);

    let (_, _, patch) = one_update(requests(&bench));
    assert_eq!(patch.body, Some(format!("{}hl", row.body)));
}

#[tokio::test]
async fn esc_on_an_edited_body_warns_once() {
    let (bench, mut section) = bench_with_demo().await;
    bench.key(&mut section, "b");
    type_text(&bench, &mut section, "x");
    bench.key(&mut section, "esc");

    let warned = frame(&bench, &section);
    assert!(warned.contains(UNSAVED), "{warned}");
    assert!(section.captures_input(), "still open");

    bench.key(&mut section, "esc");
    assert!(!section.captures_input(), "the second Esc discards");
    assert!(requests(&bench).is_empty());
}

// ---- 9.3: replies --------------------------------------------------------------------------------

/// The demo rows with the architect's description and token moved by another writer.
async fn moved_elsewhere(description: &str) -> (Vec<Persona>, Persona) {
    let mut rows = demo_rows().await;
    for row in &mut rows {
        if row.id == ids::PERSONA_ARCHITECT {
            row.description = description.to_owned();
            row.updated_at += Duration::seconds(5);
        }
    }
    let row = architect(&rows);
    (rows, row)
}

#[tokio::test]
async fn a_stale_form_rebases_untouched_fields() {
    let (bench, mut section) = bench_with_demo().await;
    bench.key(&mut section, "e");
    type_text(&bench, &mut section, "-x");
    bench.key(&mut section, "enter");
    let _ = one_update(requests(&bench));

    let (rows, row) = moved_elsewhere("Moved elsewhere.").await;
    bench.reply(
        &mut section,
        &StoreReply::PersonaWritten {
            personas: rows,
            outcome: PersonaWrite::Stale { id: row.id },
        },
    );

    let frame = frame(&bench, &section);
    assert!(
        frame.contains(&field("description", "Moved elsewhere.")),
        "{frame}"
    );
    assert!(frame.contains(&field("name", "architect-x")), "{frame}");
    assert!(shows(&frame, CHANGED_ELSEWHERE), "{frame}");

    bench.key(&mut section, "enter");
    let (_, expected, patch) = one_update(requests(&bench));
    assert_eq!(
        expected, row.updated_at,
        "the retry carries the row's token"
    );
    assert_eq!(
        patch,
        PersonaPatch {
            name: Some("architect-x".to_owned()),
            ..PersonaPatch::default()
        }
    );
}

#[tokio::test]
async fn a_stale_form_names_a_field_changed_on_both_sides() {
    let (bench, mut section) = bench_with_demo().await;
    keys(&bench, &mut section, &["e", "down"]);
    type_text(&bench, &mut section, " mine");
    bench.key(&mut section, "enter");
    let _ = requests(&bench);

    let (rows, row) = moved_elsewhere("Theirs.").await;
    bench.reply(
        &mut section,
        &StoreReply::PersonaWritten {
            personas: rows,
            outcome: PersonaWrite::Stale { id: row.id },
        },
    );

    let frame = frame(&bench, &section);
    assert!(
        shows(
            &frame,
            "changed elsewhere \u{2014} reloaded; Enter retries \u{b7} also changed elsewhere: \
             description"
        ),
        "{frame}"
    );
    assert!(frame.contains(" mine"), "the typed text is kept: {frame}");
}

#[tokio::test]
async fn gone_closes_the_editor() {
    let (bench, mut section) = bench_with_demo().await;
    keys(&bench, &mut section, &["e", "down"]);
    type_text(&bench, &mut section, "!");
    bench.key(&mut section, "enter");
    let _ = requests(&bench);

    let rows: Vec<Persona> = demo_rows()
        .await
        .into_iter()
        .filter(|row| row.id != ids::PERSONA_ARCHITECT)
        .collect();
    bench.reply(
        &mut section,
        &StoreReply::PersonaWritten {
            personas: rows,
            outcome: PersonaWrite::Gone {
                id: ids::PERSONA_ARCHITECT,
            },
        },
    );

    assert!(!section.captures_input());
    assert!(frame(&bench, &section).contains(DELETED_ELSEWHERE));
}

#[tokio::test]
async fn a_write_key_while_a_write_is_in_flight_is_refused() {
    let (bench, mut section) = bench_with_demo().await;
    keys(&bench, &mut section, &["e", "down"]);
    type_text(&bench, &mut section, "!");
    keys(&bench, &mut section, &["enter", "esc"]);
    let _ = requests(&bench);

    bench.key(&mut section, "n");

    assert!(!section.captures_input(), "no form opened");
    assert!(
        error_text(&bench, &section).contains("`update_persona` is still in flight"),
        "{}",
        frame(&bench, &section)
    );
}

#[tokio::test]
async fn a_refused_write_keeps_the_editor_over_its_text() {
    let (bench, mut section) = bench_with_demo().await;
    keys(&bench, &mut section, &["e", "down"]);
    type_text(&bench, &mut section, "!");
    bench.key(&mut section, "enter");
    let _ = requests(&bench);

    bench.reply(
        &mut section,
        &StoreReply::Failed {
            request: "update_persona",
            message: "persona.description holds a NUL".to_owned(),
        },
    );

    assert!(section.captures_input(), "the form stays open");
    assert!(frame(&bench, &section).contains("edit persona `architect`"));
    assert!(
        error_text(&bench, &section).contains("persona.description holds a NUL"),
        "{}",
        frame(&bench, &section)
    );
    bench.key(&mut section, "enter");
    let _ = one_update(requests(&bench));
}

#[tokio::test]
async fn a_section_debug_prints_no_typed_text() {
    let (bench, mut section) = bench_with_demo().await;
    bench.key(&mut section, "n");
    type_text(&bench, &mut section, "secret");
    let shown = format!("{section:?}");
    assert!(!shown.contains("secret"), "{shown}");

    bench.key(&mut section, "enter");
    type_text(&bench, &mut section, "classified");
    let shown = format!("{section:?}");
    assert!(
        !shown.contains("secret") && !shown.contains("classified"),
        "{shown}"
    );
}
