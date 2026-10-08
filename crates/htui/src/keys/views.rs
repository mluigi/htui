//! Every converted view mode's context stack (MOD-67 D3): views import them, never declare one.
//!
//! Each stack opens with the view's own layer ([`Layer::view`]: the mode's own acts plus the
//! shared acts a wider layer offers, PA-3), and ends with the global layer: unfiltered in a
//! browse mode, [`Layer::modal`] in a capturing or confirming one (D5). Layer order is fixed:
//! view, `settings`, `confirm`, `form`, `common`, `list`, `overlay`, `global` (only the layers
//! present). `keys::DECLARED` lists every one of them with the phrase its collision errors end
//! with. M3 lands them all in T1; the views convert to them lane by lane.

use super::{Act, Context, Layer, Stack};

/// The global layer of a capturing or confirming mode: CONTROL, ALT and function keys only.
const MODAL: Layer = Layer::modal(Context::Global);

/// The unfiltered global layer of a browse mode.
const GLOBAL: Layer = Layer::all(Context::Global);

/// The Settings tab's own layer (section cycling).
const SETTINGS: Layer = Layer::all(Context::Settings);

/// `list ∩ {down, up}`.
const LIST: Layer = Layer::only(Context::List, &[Act::ListDown, Act::ListUp]);

/// `confirm`, both answers.
const CONFIRM: Layer = Layer::all(Context::Confirm);

/// `form ∩ {next_field, prev_field}`.
const FORM_FIELDS: Layer = Layer::only(Context::Form, &[Act::FormNextField, Act::FormPrevField]);

/// `form ∩ {save}`.
const FORM_SAVE: Layer = Layer::only(Context::Form, &[Act::FormSave]);

/// The overlay layer (`overlay.close`).
const OVERLAY: Layer = Layer::all(Context::Overlay);

/// `global ∩ {help}`, unfiltered: a list overlay's global layer.
const HELP: Layer = Layer::only(Context::Global, &[Act::Help]);

/// The Settings tab while its section has no stack of its own: section cycling, then global
/// (L-A Q11). `SettingsTab` resolves `settings.next_section`/`prev_section` through it.
pub const SETTINGS_TAB: Stack<'static> = Stack::new(&[SETTINGS, GLOBAL]);

/// A mode that is a text field or an opaque widget and nothing else (PA-7): the agents paste
/// field, the Qdrant URL and key editors, the connection, prompt and queue editors, the boxes
/// tags editor, hierarchy's typed delete, its in-flight delete and its path picker, the personas
/// import path and the secrets URL.
pub const CAPTURE: Stack<'static> = Stack::new(&[MODAL]);

/// Settings > Agents, browsing the agent list.
pub const AGENTS_BROWSE: Stack<'static> = Stack::new(&[
    Layer::view(
        Context::SettingsAgents,
        &[
            Act::AgentsProbe,
            Act::AgentsInstall,
            Act::AgentsAuthenticate,
            Act::AgentsSwitchBox,
            Act::AgentsEditPaths,
            Act::AgentsOpenLink,
            Act::AgentsPasteRedirect,
            Act::AgentsCancel,
        ],
    ),
    SETTINGS,
    Layer::only(Context::Common, &[Act::New, Act::Edit, Act::Dismiss]),
    LIST,
    GLOBAL,
]);

/// Settings > Agents, the install consent question (install `Pending`).
pub const AGENTS_CONSENT: Stack<'static> = Stack::new(&[
    Layer::view(Context::SettingsAgents, &[]),
    SETTINGS,
    CONFIRM,
    GLOBAL,
]);

/// Settings > Agents, the login method chooser (auth `Choosing`).
pub const AGENTS_CHOOSER: Stack<'static> = Stack::new(&[
    Layer::view(
        Context::SettingsAgents,
        &[Act::AgentsChoose, Act::AgentsProbe, Act::AgentsInstall],
    ),
    SETTINGS,
    Layer::only(Context::Confirm, &[Act::ConfirmNo]),
    LIST,
    GLOBAL,
]);

/// Settings > Agents, the create/edit form and the paths form.
pub const AGENTS_FORM: Stack<'static> = Stack::new(&[
    Layer::view(Context::SettingsAgents, &[]),
    FORM_FIELDS,
    MODAL,
]);

/// Settings > Hierarchy, browsing the tree.
pub const HIERARCHY_BROWSE: Stack<'static> = Stack::new(&[
    Layer::view(
        Context::SettingsHierarchy,
        &[
            Act::HierarchyNewWorkspace,
            Act::HierarchyPrimary,
            Act::HierarchyChoosePath,
            Act::HierarchyInfer,
        ],
    ),
    SETTINGS,
    Layer::only(
        Context::Common,
        &[Act::Edit, Act::New, Act::Delete, Act::Reload, Act::Dismiss],
    ),
    LIST,
    GLOBAL,
]);

/// Settings > Hierarchy, the editor form.
pub const HIERARCHY_EDITOR: Stack<'static> = Stack::new(&[
    Layer::view(Context::SettingsHierarchy, &[]),
    FORM_FIELDS,
    MODAL,
]);

/// Settings > Hierarchy, while a delete counts the rows it would remove.
pub const HIERARCHY_DELETE_COUNTING: Stack<'static> = Stack::new(&[
    Layer::view(Context::SettingsHierarchy, &[]),
    Layer::only(Context::Confirm, &[Act::ConfirmNo]),
    MODAL,
]);

/// Settings > Hierarchy, the delete warning.
pub const HIERARCHY_DELETE_WARN: Stack<'static> =
    Stack::new(&[Layer::view(Context::SettingsHierarchy, &[]), CONFIRM, MODAL]);

/// Settings > Kinds, browsing the kinds.
pub const KINDS_BROWSE: Stack<'static> = Stack::new(&[
    Layer::view(
        Context::SettingsKinds,
        &[Act::KindsNewGraph, Act::KindsGraph],
    ),
    SETTINGS,
    Layer::only(
        Context::Common,
        &[Act::Edit, Act::New, Act::Delete, Act::Reload, Act::Dismiss],
    ),
    LIST,
    GLOBAL,
]);

/// Settings > Kinds, the editor form.
pub const KINDS_EDITOR: Stack<'static> =
    Stack::new(&[Layer::view(Context::SettingsKinds, &[]), FORM_FIELDS, MODAL]);

/// Settings > Kinds, the prefix warning and the delete question (asking and in flight).
pub const KINDS_CONFIRM: Stack<'static> =
    Stack::new(&[Layer::view(Context::SettingsKinds, &[]), CONFIRM, MODAL]);

/// Settings > Prompt, browsing.
pub const PROMPT_BROWSE: Stack<'static> = Stack::new(&[
    Layer::view(Context::SettingsPrompt, &[]),
    SETTINGS,
    Layer::only(Context::Common, &[Act::Edit, Act::Reload, Act::Dismiss]),
    LIST,
    GLOBAL,
]);

/// Settings > Connection, browsing.
pub const CONNECTION_BROWSE: Stack<'static> = Stack::new(&[
    Layer::view(
        Context::SettingsConnection,
        &[Act::ConnectionRebuild, Act::ConnectionActivate],
    ),
    SETTINGS,
    Layer::only(
        Context::Common,
        &[Act::Edit, Act::Clear, Act::Reload, Act::Dismiss],
    ),
    LIST,
    GLOBAL,
]);

/// Settings > Connection, the clear-DSN and rebuild questions (asking and in flight).
pub const CONNECTION_CONFIRM: Stack<'static> = Stack::new(&[
    Layer::view(Context::SettingsConnection, &[]),
    CONFIRM,
    MODAL,
]);

/// Settings > Qdrant, browsing.
pub const QDRANT_BROWSE: Stack<'static> = Stack::new(&[
    Layer::view(Context::SettingsQdrant, &[]),
    SETTINGS,
    Layer::only(Context::Common, &[Act::Edit, Act::Clear, Act::Reload]),
    LIST,
    GLOBAL,
]);

/// Settings > Qdrant, the clear question.
pub const QDRANT_CONFIRM: Stack<'static> =
    Stack::new(&[Layer::view(Context::SettingsQdrant, &[]), CONFIRM, MODAL]);

/// Settings > Boxes, browsing.
pub const BOXES_BROWSE: Stack<'static> = Stack::new(&[
    Layer::view(
        Context::SettingsBoxes,
        &[
            Act::BoxesEditTags,
            Act::BoxesEditQuirks,
            Act::BoxesExecutor,
            Act::BoxesProbe,
            Act::BoxesEditSpec,
        ],
    ),
    SETTINGS,
    Layer::only(Context::Common, &[Act::Reload, Act::Dismiss]),
    LIST,
    GLOBAL,
]);

/// Settings > Boxes, the quirks and probe spec editors.
pub const BOXES_EDITOR: Stack<'static> =
    Stack::new(&[Layer::view(Context::SettingsBoxes, &[]), FORM_SAVE, MODAL]);

/// Settings > Boxes, the executor question.
pub const BOXES_EXECUTOR: Stack<'static> =
    Stack::new(&[Layer::view(Context::SettingsBoxes, &[]), CONFIRM, MODAL]);

/// Settings > Personas, browsing.
pub const PERSONAS_BROWSE: Stack<'static> = Stack::new(&[
    Layer::view(
        Context::SettingsPersonas,
        &[Act::PersonasBody, Act::PersonasRules, Act::PersonasImport],
    ),
    SETTINGS,
    Layer::only(Context::Common, &[Act::New, Act::Edit, Act::Delete]),
    LIST,
    GLOBAL,
]);

/// Settings > Personas, the create/edit form.
pub const PERSONAS_FORM: Stack<'static> = Stack::new(&[
    Layer::view(Context::SettingsPersonas, &[]),
    FORM_FIELDS,
    MODAL,
]);

/// Settings > Personas, the body and rules editors.
pub const PERSONAS_EDITOR: Stack<'static> = Stack::new(&[
    Layer::view(Context::SettingsPersonas, &[]),
    FORM_SAVE,
    MODAL,
]);

/// Settings > Personas, the delete question (asking and in flight).
pub const PERSONAS_DELETE: Stack<'static> =
    Stack::new(&[Layer::view(Context::SettingsPersonas, &[]), CONFIRM, MODAL]);

/// Settings > Personas, the import report.
pub const PERSONAS_REPORT: Stack<'static> = Stack::new(&[
    Layer::view(Context::SettingsPersonas, &[]),
    Layer::only(Context::Common, &[Act::Back]),
    LIST,
    MODAL,
]);

/// Settings > Secrets, browsing.
pub const SECRETS_BROWSE: Stack<'static> = Stack::new(&[
    Layer::view(Context::SettingsSecrets, &[Act::SecretsCheck]),
    SETTINGS,
    Layer::only(
        Context::Common,
        &[Act::Edit, Act::Clear, Act::Reload, Act::Dismiss],
    ),
    LIST,
    GLOBAL,
]);

/// Settings > Secrets, the identity and scope forms.
pub const SECRETS_FORM: Stack<'static> = Stack::new(&[
    Layer::view(Context::SettingsSecrets, &[]),
    FORM_FIELDS,
    MODAL,
]);

/// Settings > Secrets, the three clear questions.
pub const SECRETS_CONFIRM: Stack<'static> =
    Stack::new(&[Layer::view(Context::SettingsSecrets, &[]), CONFIRM, MODAL]);

/// Settings > Queue, browsing.
pub const QUEUE_BROWSE: Stack<'static> = Stack::new(&[
    Layer::view(Context::SettingsQueue, &[]),
    SETTINGS,
    Layer::only(Context::Common, &[Act::Edit, Act::Reload, Act::Dismiss]),
    LIST,
    GLOBAL,
]);

/// The concepts search overlay: its query field captures, so `?` is text and `F1` is help.
pub const CONCEPTS_QUERY: Stack<'static> = Stack::new(&[
    Layer::view(
        Context::Concepts,
        &[
            Act::ConceptsDecisions,
            Act::ConceptsProject,
            Act::ConceptsReindex,
            Act::ConceptsUp,
            Act::ConceptsDown,
        ],
    ),
    OVERLAY,
    HELP.with_modal_filter(),
]);

/// The workspace switcher overlay.
pub const SWITCHER: Stack<'static> = Stack::new(&[
    Layer::view(Context::Switcher, &[Act::SwitcherSwitch]),
    LIST,
    OVERLAY,
    HELP,
]);

/// The schema migration prompt overlay.
pub const MIGRATION: Stack<'static> =
    Stack::new(&[Layer::view(Context::Migration, &[]), CONFIRM, OVERLAY, HELP]);

/// The waiting list overlay.
pub const WAITING_LIST: Stack<'static> = Stack::new(&[
    Layer::view(Context::Waiting, &[Act::WaitingOpen]),
    LIST,
    OVERLAY,
    HELP,
]);

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    use super::*;
    use crate::keys::{CATALOGUE, DECLARED, KeyChord, Keys, VIEW_DEFAULTS};

    fn chord(spec: &str) -> KeyChord {
        KeyChord::parse_strict(spec).expect("a valid spec")
    }

    fn resolve(stack: Stack<'_>, spec: &str) -> Vec<Act> {
        Keys::compiled().actions(stack, chord(spec))
    }

    #[test]
    fn the_view_stacks_resolve_as_designed() {
        assert_eq!(resolve(HIERARCHY_BROWSE, "ctrl-d"), []);
        assert_eq!(resolve(KINDS_BROWSE, "ctrl-d"), []);
        assert_eq!(resolve(HIERARCHY_BROWSE, "d"), [Act::Delete]);
        assert_eq!(resolve(HIERARCHY_BROWSE, "down"), [Act::ListDown]);
        assert_eq!(resolve(HIERARCHY_BROWSE, "tab"), [Act::NextTab]);
        assert_eq!(resolve(AGENTS_FORM, "tab"), [Act::FormNextField]);
        assert_eq!(resolve(AGENTS_FORM, "down"), [Act::FormNextField]);
        assert_eq!(resolve(QUEUE_BROWSE, "enter"), [Act::Edit]);
        assert_eq!(resolve(MIGRATION, "Y"), [Act::ConfirmYes]);
        assert_eq!(resolve(MIGRATION, "N"), [Act::ConfirmNo]);
        assert_eq!(resolve(PERSONAS_REPORT, "enter"), [Act::Back]);
        assert_eq!(resolve(PERSONAS_BROWSE, "enter"), []);
        assert_eq!(resolve(AGENTS_FORM, "q"), []);
        assert_eq!(resolve(AGENTS_FORM, "f1"), [Act::Help]);
        assert_eq!(resolve(CAPTURE, "tab"), []);
        assert!(!CAPTURE.passes(chord("tab")));
        assert_eq!(resolve(SETTINGS_TAB, "ctrl-l"), []);
        assert_eq!(resolve(SETTINGS_TAB, "l"), [Act::NextSection]);
        assert_eq!(
            resolve(BOXES_BROWSE, "w"),
            [Act::BoxesExecutor, Act::Workspaces]
        );
        assert_eq!(resolve(KINDS_BROWSE, "g"), [Act::KindsGraph]);
        assert_eq!(resolve(CONCEPTS_QUERY, "?"), []);
        assert_eq!(resolve(CONCEPTS_QUERY, "f1"), [Act::Help]);
        assert_eq!(resolve(CONCEPTS_QUERY, "esc"), [Act::OverlayClose]);
        let capital =
            KeyChord::from_event(KeyEvent::new(KeyCode::Char('D'), KeyModifiers::CONTROL));
        assert_eq!(
            Keys::compiled().actions(CONCEPTS_QUERY, capital),
            [Act::ConceptsDecisions]
        );
        assert_eq!(
            resolve(MIGRATION, "esc"),
            [Act::ConfirmNo, Act::OverlayClose]
        );
        assert_eq!(resolve(SWITCHER, "enter"), [Act::SwitcherSwitch]);
        assert_eq!(resolve(SWITCHER, "2"), []);
    }

    #[test]
    fn every_view_layer_comes_first_and_every_modal_stack_ends_modal() {
        let views = &DECLARED[6..];
        assert_eq!(views.len(), 32);
        for &(phrase, stack) in views {
            let first = stack.layers()[0];
            assert!(first.inherits(), "{phrase}: layer 0 is not a view layer");
            assert!(first.context().is_view(), "{phrase}");
            let (index, global) = stack.global().expect("a global layer");
            assert_eq!(
                index,
                stack.layers().len() - 1,
                "{phrase}: global is not last"
            );
            if global.is_modal() {
                assert!(
                    stack
                        .layers()
                        .iter()
                        .all(|layer| layer.context() != Context::Settings),
                    "{phrase}: a modal stack has a settings layer"
                );
            }
            for layer in &stack.layers()[1..] {
                assert!(!layer.inherits(), "{phrase}: a view layer below the first");
            }
        }
    }

    #[test]
    fn every_view_default_is_an_extra_chord_some_stack_offers() {
        for &(context, act, extra) in VIEW_DEFAULTS {
            let spec = act.spec().expect("a catalogue row");
            assert!(context.is_view(), "{context:?}");
            assert!(spec.context.is_shared(), "{act:?}");
            for written in extra {
                assert!(
                    KeyChord::parse_strict(written).is_ok(),
                    "{context:?} {act:?} {written:?}"
                );
                assert!(
                    !spec.defaults.contains(written),
                    "{context:?} {act:?}: {written:?} is already a default"
                );
            }
            assert!(
                DECLARED
                    .iter()
                    .any(|(_, stack)| stack.view_admits(context, act)),
                "no declared stack offers {act:?} in {context:?}"
            );
        }
    }

    #[test]
    fn no_view_name_hides_a_shared_name_it_inherits() {
        for &(phrase, stack) in DECLARED {
            for (index, layer) in stack.layers().iter().enumerate() {
                if !layer.inherits() {
                    continue;
                }
                let own: Vec<&str> = CATALOGUE
                    .iter()
                    .filter(|row| row.context == layer.context())
                    .map(|row| row.name)
                    .collect();
                for shared in CATALOGUE.iter().filter(|row| row.context.is_shared()) {
                    if stack.admits(index, shared.act) {
                        assert!(
                            !own.contains(&shared.name),
                            "{phrase}: [{}] {} hides the inherited {}.{}",
                            layer.context().table(),
                            shared.name,
                            shared.context.table(),
                            shared.name
                        );
                    }
                }
            }
        }
    }

    /// MOD-67 M3 T7: every view stack this module declares is the stack of some view. The
    /// shell's own `BASE`, `OVERLAY` and editor stacks are not declared here; a stack nothing names would be
    /// validated (and refuse key files) for a mode no view is ever in.
    #[test]
    fn every_view_stack_is_named_by_a_view() {
        const VIEWS: &[&str] = &[
            include_str!("../ui/tabs/settings/mod.rs"),
            include_str!("../ui/tabs/settings/agents.rs"),
            include_str!("../ui/tabs/settings/boxes.rs"),
            include_str!("../ui/tabs/settings/connection.rs"),
            include_str!("../ui/tabs/settings/hierarchy.rs"),
            include_str!("../ui/tabs/settings/kinds.rs"),
            include_str!("../ui/tabs/settings/personas.rs"),
            include_str!("../ui/tabs/settings/prompt.rs"),
            include_str!("../ui/tabs/settings/qdrant.rs"),
            include_str!("../ui/tabs/settings/queue.rs"),
            include_str!("../ui/tabs/settings/secrets.rs"),
            include_str!("../ui/overlay/concepts_search.rs"),
            include_str!("../ui/overlay/migration_prompt.rs"),
            include_str!("../ui/overlay/waiting_list.rs"),
            include_str!("../ui/overlay/workspace_switcher.rs"),
        ];
        // Production code only: each file up to its test module, its comment lines dropped, so a
        // stack named only by a test or a doc comment still counts as unnamed.
        let production: Vec<String> = VIEWS
            .iter()
            .map(|source| {
                let code = source.split("\n#[cfg(test)]").next().unwrap_or(source);
                code.lines()
                    .filter(|line| !line.trim_start().starts_with("//"))
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .collect();
        let named = |name: &str| {
            let wanted = format!("views::{name}");
            production.iter().any(|source| {
                source.match_indices(&wanted).any(|(at, _)| {
                    !source[at + wanted.len()..]
                        .starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_')
                })
            })
        };
        let declared: Vec<&str> = include_str!("views.rs")
            .lines()
            .filter_map(|line| line.strip_prefix("pub const "))
            .filter_map(|rest| rest.split_once(": Stack<'static>").map(|(name, _)| name))
            .collect();
        // `BASE`, `OVERLAY` and the two editor stacks live in `stack.rs`.
        assert_eq!(
            declared.len(),
            DECLARED.len() - 4,
            "views.rs stacks vs DECLARED"
        );
        for name in declared {
            assert!(named(name), "no view names views::{name}");
        }
    }
}
