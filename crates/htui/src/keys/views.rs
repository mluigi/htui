//! Every converted view mode's context stack (MOD-67 D3): views import them, never declare one.
//!
//! Each stack opens with the view's own layer ([`Layer::view`]: the mode's own acts plus the
//! shared acts a wider layer offers, PA-3), except [`SETTINGS_TAB`], [`CAPTURE`],
//! [`QUIT_CONFIRM`] and [`ABORT_CONFIRM`] (only `editor.abort`), which have none, and ends
//! with the global layer: unfiltered in a
//! browse mode, [`Layer::modal`] in a capturing or confirming one (D5). Layer order is fixed:
//! view, `settings`, `confirm`, `form`, `common`, `list`, `overlay`, `global` (only the layers
//! present). `keys::DECLARED` lists every one of them with the phrase its collision errors end
//! with. M3 lands them all in T1; the views convert to them lane by lane.
//!
//! M4 adds the Skills and Requirements stacks. Layer order: view, a parent view's `only` layer
//! (the attachments pane over the Library), `settings`/`skills`, `confirm`, `form`, `common`,
//! `list`, `pane`, `overlay`, then the global layers; a capturing Skills mode whose widget passes
//! `Tab` puts [`TABS`] before [`MODAL`] (PA-2).
//!
//! M5 adds the Backlog and Chat stacks. A Backlog browse stack is the pane's view layer, then the
//! tail every sub-tab shares: `pane` (narrowed per pane), `backlog`, `common ∩ {edit}`, `list`,
//! global (ANA-26 §7.3); a Backlog capturing or asking mode is its view layer, the shared layers
//! it offers, `MODAL`. Layer order: view, `pane`, `backlog`, `confirm`, `form`, `common`, `list`,
//! global. Each browse stack's tail is written out (a `const` slice cannot be concatenated);
//! `every_backlog_browse_stack_offers_the_tab_and_the_list` pins that all seven share it.
//!
//! M6 adds the queue overlay's and the directory picker's stacks.

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

/// `global ∩ {next_tab, prev_tab}`, unfiltered: a capturing Skills mode still hands `Tab` and
/// `Shift+Tab` to the shell, the draft kept (MOD-67 M4 PA-2). Always just before `MODAL`.
const TABS: Layer = Layer::only(Context::Global, &[Act::NextTab, Act::PrevTab]);

/// `pane ∩ {scroll_down, scroll_up, page_down, page_up}`.
const PANE_SCROLL: Layer = Layer::only(
    Context::Pane,
    &[
        Act::PaneScrollDown,
        Act::PaneScrollUp,
        Act::PanePageDown,
        Act::PanePageUp,
    ],
);

/// `skills ∩ {switch_view}`.
const SKILLS_SWITCH: Layer = Layer::only(Context::Skills, &[Act::SkillsSwitchView]);

/// `skills ∩` the verbs Library and Templates browse share.
const SKILLS_BROWSE: Layer = Layer::only(
    Context::Skills,
    &[
        Act::SkillsSwitchView,
        Act::SkillsPrevVersion,
        Act::SkillsNextVersion,
        Act::SkillsBase,
        Act::SkillsDiff,
        Act::SkillsEditExternally,
    ],
);

/// The editors' own chords: `skills ∩ {ask_agent}`.
const SKILLS_ASK: Layer = Layer::only(Context::Skills, &[Act::SkillsAskAgent]);

/// `form ∩ {save, external_editor}`: a `TextArea` editor's chords.
const FORM_EDITOR: Layer = Layer::only(Context::Form, &[Act::FormSave, Act::FormExternalEditor]);

/// `form ∩ {next_field, prev_field, save}`: a multi-field form that saves on a chord. Despite
/// the name, three of the four `form` acts (no `external_editor`); [`FORM`] holds all four.
const FORM_ALL: Layer = Layer::only(
    Context::Form,
    &[Act::FormNextField, Act::FormPrevField, Act::FormSave],
);

/// Every `pane` act: a sub-tab that scrolls (Body, Notes, Prompt) and cycles sub-tabs.
const PANE: Layer = Layer::all(Context::Pane);

/// `pane ∩ {page_down, page_up, next_subtab, prev_subtab}`: a sub-tab whose `J`/`K` move a cursor
/// of its own (Runs, Graph, Docs, Reqs).
const PANE_PAGE: Layer = Layer::only(
    Context::Pane,
    &[
        Act::PanePageDown,
        Act::PanePageUp,
        Act::PaneNextSubtab,
        Act::PanePrevSubtab,
    ],
);

/// `pane ∩ {page_down, page_up}`.
const PAGES: Layer = Layer::only(Context::Pane, &[Act::PanePageDown, Act::PanePageUp]);

/// The Backlog tab's own verbs (MOD-67 M5 D1), a shared context every Backlog browse stack has.
const BACKLOG: Layer = Layer::all(Context::Backlog);

/// `common ∩ {edit}`: the Backlog's `e`.
const EDIT: Layer = Layer::only(Context::Common, &[Act::Edit]);

/// `common ∩ {back}`.
const BACK: Layer = Layer::only(Context::Common, &[Act::Back]);

/// Every `list` act: the Backlog list's moves and fold.
const LIST_ALL: Layer = Layer::all(Context::List);

/// `confirm ∩ {no}`: a wait that can only be left.
const NO: Layer = Layer::only(Context::Confirm, &[Act::ConfirmNo]);

/// Every `form` act: a multi-field form with a `TextArea` (the item form, the Docs compose area).
/// All four, `external_editor` included; [`FORM_ALL`] is the three without it (renamed in M6).
const FORM: Layer = Layer::all(Context::Form);

/// The Settings tab while its section has no stack of its own: section cycling, then global
/// (L-A Q11). `SettingsTab` resolves `settings.next_section`/`prev_section` through it.
pub const SETTINGS_TAB: Stack<'static> = Stack::new(&[SETTINGS, GLOBAL]);

/// A mode that is a text field or an opaque widget and nothing else (PA-7): the agents paste
/// field, the Qdrant URL and key editors, the connection, prompt and queue editors, the boxes
/// tags editor, hierarchy's typed delete, its in-flight delete and its path picker's go-to field,
/// the personas
/// import path and the secrets URL; the Requirements filter (MOD-67 M4); the Runs note,
/// follow-up and typed key, a close-out in flight and the chat composer (MOD-67 M5).
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

/// The quit question (MOD-57 M2 E4): `confirm.yes` quits, `confirm.no` and `overlay.close` keep
/// the edit, `global.help` toggles the box. No view layer: the question has no keys of its own,
/// so `[confirm]`, `[overlay]` and `[global] help` rebind it, and no `[quit]` table exists.
pub const QUIT_CONFIRM: Stack<'static> = Stack::new(&[CONFIRM, OVERLAY, HELP]);

/// The abort question (MOD-57 M2 R1 M-2): `editor.abort` again aborts; every other key keeps the
/// edit, so no other layer: not even `overlay.close` or `global.help` act there.
pub const ABORT_CONFIRM: Stack<'static> =
    Stack::new(&[Layer::only(Context::Editor, &[Act::EditorAbort])]);

/// The waiting list overlay.
pub const WAITING_LIST: Stack<'static> = Stack::new(&[
    Layer::view(Context::Waiting, &[Act::WaitingOpen]),
    LIST,
    OVERLAY,
    HELP,
]);

/// Skills > Library, browsing (also with a draft handed off to `$EDITOR`).
pub const LIBRARY_BROWSE: Stack<'static> = Stack::new(&[
    Layer::view(
        Context::SkillsLibrary,
        &[Act::LibraryImport, Act::LibraryInfo, Act::LibraryAttach],
    ),
    SKILLS_BROWSE,
    Layer::only(Context::Common, &[Act::New, Act::Edit, Act::Reload]),
    LIST,
    PANE_SCROLL,
    GLOBAL,
]);

/// Skills > Library, the name, description and import path prompts.
pub const LIBRARY_PROMPT: Stack<'static> =
    Stack::new(&[Layer::view(Context::SkillsLibrary, &[]), TABS, MODAL]);

/// Skills > Library, the rename form (`Info`).
pub const LIBRARY_INFO: Stack<'static> =
    Stack::new(&[Layer::view(Context::SkillsLibrary, &[]), FORM_ALL, MODAL]);

/// Skills > Library, the editor with no agent help open.
pub const LIBRARY_EDITOR: Stack<'static> = Stack::new(&[
    Layer::view(Context::SkillsLibrary, &[]),
    SKILLS_ASK,
    FORM_EDITOR,
    TABS,
    MODAL,
]);

/// Skills > Library, the import report.
pub const LIBRARY_REPORT: Stack<'static> = Stack::new(&[
    Layer::view(Context::SkillsLibrary, &[]),
    SKILLS_SWITCH,
    Layer::only(Context::Common, &[Act::Reload, Act::Back]),
    LIST,
    PANE_SCROLL,
    GLOBAL,
]);

/// Skills > Library > Attachments, browsing the pane (`a` closes it: the Library's toggle).
pub const ATTACH_BROWSE: Stack<'static> = Stack::new(&[
    Layer::view(
        Context::SkillsAttach,
        &[Act::AttachChoose, Act::AttachDetach],
    ),
    Layer::only(Context::SkillsLibrary, &[Act::LibraryAttach]),
    SKILLS_SWITCH,
    Layer::only(Context::Common, &[Act::Reload, Act::Back]),
    LIST,
    GLOBAL,
]);

/// Skills > Library > Attachments, the attachment form.
pub const ATTACH_FORM: Stack<'static> = Stack::new(&[
    Layer::view(Context::SkillsAttach, &[Act::AttachRepo]),
    FORM_ALL,
    MODAL,
]);

/// Skills > Library > Attachments, the repo picker.
pub const ATTACH_PICKER: Stack<'static> = Stack::new(&[
    Layer::view(Context::SkillsAttach, &[Act::AttachChoose]),
    Layer::only(Context::Common, &[Act::Back]),
    LIST,
    MODAL,
]);

/// Skills > Library > Attachments, the detach question.
pub const ATTACH_CONFIRM: Stack<'static> =
    Stack::new(&[Layer::view(Context::SkillsAttach, &[]), CONFIRM, MODAL]);

/// Skills > Templates, browsing (also with a draft handed off to `$EDITOR`).
pub const TEMPLATES_BROWSE: Stack<'static> = Stack::new(&[
    Layer::view(Context::SkillsTemplates, &[Act::TemplatesDiffDefault]),
    SKILLS_BROWSE,
    Layer::only(Context::Common, &[Act::New, Act::Edit, Act::Reload]),
    LIST,
    PANE_SCROLL,
    GLOBAL,
]);

/// Skills > Templates, the name prompt.
pub const TEMPLATES_PROMPT: Stack<'static> =
    Stack::new(&[Layer::view(Context::SkillsTemplates, &[]), TABS, MODAL]);

/// Skills > Templates, the editor with no agent help open.
pub const TEMPLATES_EDITOR: Stack<'static> = Stack::new(&[
    Layer::view(Context::SkillsTemplates, &[]),
    SKILLS_ASK,
    FORM_EDITOR,
    TABS,
    MODAL,
]);

/// The editors' agent help, its prompt (`Asking`).
pub const HELP_ASKING: Stack<'static> = Stack::new(&[
    Layer::view(
        Context::SkillsHelp,
        &[Act::SkillsHelpPrevAgent, Act::SkillsHelpNextAgent],
    ),
    TABS,
    MODAL,
]);

/// The editors' agent help while the agent works (starting, streaming, cancelling).
pub const HELP_WAITING: Stack<'static> = Stack::new(&[
    Layer::view(Context::SkillsHelp, &[Act::SkillsHelpCancel]),
    TABS,
    MODAL,
]);

/// The editors' agent help, its proposal or answer.
pub const HELP_PROPOSAL: Stack<'static> = Stack::new(&[
    Layer::view(Context::SkillsHelp, &[Act::SkillsHelpAccept]),
    Layer::only(Context::Confirm, &[Act::ConfirmNo]),
    PANE_SCROLL,
    TABS,
    MODAL,
]);

/// Requirements, browsing the tree.
pub const REQUIREMENTS_BROWSE: Stack<'static> = Stack::new(&[
    Layer::view(
        Context::Requirements,
        &[
            Act::RequirementsNewArea,
            Act::RequirementsAmend,
            Act::RequirementsWithdraw,
            Act::RequirementsFilter,
        ],
    ),
    Layer::only(Context::Common, &[Act::New, Act::Reload, Act::Back]),
    Layer::only(
        Context::List,
        &[
            Act::ListDown,
            Act::ListUp,
            Act::ListTop,
            Act::ListBottom,
            Act::ListFold,
        ],
    ),
    PANE_SCROLL,
    GLOBAL,
]);

/// Requirements, the area form and the requirement form (mint and amend).
pub const REQUIREMENTS_FORM: Stack<'static> =
    Stack::new(&[Layer::view(Context::Requirements, &[]), FORM_ALL, MODAL]);

/// Requirements, the withdraw form (both stages).
pub const REQUIREMENTS_WITHDRAW: Stack<'static> =
    Stack::new(&[Layer::view(Context::Requirements, &[]), FORM_SAVE, MODAL]);

/// The Backlog's Body sub-tab (MOD-67 M5 PA-1), and `DetailTab::key_stack`'s default: no own
/// verb, the pane's scroll and sub-tab cycling, then the tail every Backlog browse stack shares.
pub const BACKLOG_BODY: Stack<'static> = Stack::new(&[
    Layer::view(Context::BacklogBody, &[]),
    PANE,
    BACKLOG,
    EDIT,
    LIST_ALL,
    GLOBAL,
]);

/// Backlog > Runs, browsing (the run list and the flow). `J`/`K` are the step cursor, so the
/// pane layer offers only paging and sub-tab cycling.
pub const RUNS_BROWSE: Stack<'static> = Stack::new(&[
    Layer::view(
        Context::BacklogRuns,
        &[
            Act::RunsNextStep,
            Act::RunsPrevStep,
            Act::RunsApprove,
            Act::RunsReject,
            Act::RunsRetry,
            Act::RunsPromote,
            Act::RunsSelect,
            Act::RunsAccept,
            Act::RunsOpen,
            Act::RunsCancelRun,
            Act::RunsCleanup,
            Act::RunsUnblock,
            Act::RunsStartRun,
            Act::RunsCloseOut,
            Act::RunsFollowUp,
            Act::RunsToggleView,
            Act::RunsZoomIn,
            Act::RunsZoomOut,
            Act::RunsZoomFit,
        ],
    ),
    PANE_PAGE,
    BACKLOG,
    EDIT,
    LIST_ALL,
    GLOBAL,
]);

/// Backlog > Runs, the cancel question and the close-out warning.
pub const RUNS_CONFIRM: Stack<'static> =
    Stack::new(&[Layer::view(Context::BacklogRuns, &[]), CONFIRM, MODAL]);

/// Backlog > Runs, while the close-out counts (it can only be left).
pub const RUNS_COUNTING: Stack<'static> =
    Stack::new(&[Layer::view(Context::BacklogRuns, &[]), NO, MODAL]);

/// Backlog > Runs, the document view (`j`/`k` scroll too: `VIEW_DEFAULTS`).
pub const RUNS_ARTIFACT: Stack<'static> = Stack::new(&[
    Layer::view(Context::BacklogRuns, &[]),
    PANE_SCROLL,
    BACK,
    MODAL,
]);

/// Backlog > Graph.
pub const BACKLOG_GRAPH: Stack<'static> = Stack::new(&[
    Layer::view(
        Context::BacklogGraph,
        &[
            Act::GraphNextRow,
            Act::GraphPrevRow,
            Act::GraphDeeper,
            Act::GraphShallower,
            Act::GraphReroot,
        ],
    ),
    PANE_PAGE,
    BACKLOG,
    EDIT,
    LIST_ALL,
    GLOBAL,
]);

/// Backlog > Docs, no compose area open.
pub const BACKLOG_DOCS: Stack<'static> = Stack::new(&[
    Layer::view(
        Context::BacklogDocs,
        &[
            Act::DocsNextRow,
            Act::DocsPrevRow,
            Act::DocsAdd,
            Act::DocsNewVersion,
        ],
    ),
    PANE_PAGE,
    BACKLOG,
    EDIT,
    LIST_ALL,
    GLOBAL,
]);

/// Backlog > Docs, the compose area (kind, title and body).
pub const DOCS_COMPOSE: Stack<'static> =
    Stack::new(&[Layer::view(Context::BacklogDocs, &[]), FORM, MODAL]);

/// Backlog > Notes, no compose area open.
pub const BACKLOG_NOTES: Stack<'static> = Stack::new(&[
    Layer::view(Context::BacklogNotes, &[Act::NotesAdd]),
    PANE,
    BACKLOG,
    EDIT,
    LIST_ALL,
    GLOBAL,
]);

/// Backlog > Notes, the compose area (a body only: its `TextArea` takes `Up`/`Down`).
pub const NOTES_COMPOSE: Stack<'static> =
    Stack::new(&[Layer::view(Context::BacklogNotes, &[]), FORM_EDITOR, MODAL]);

/// Backlog > Prompt (MOD-67 M5 PA-1).
pub const BACKLOG_PROMPT: Stack<'static> = Stack::new(&[
    Layer::view(
        Context::BacklogPrompt,
        &[Act::PromptNextTemplate, Act::PromptPrevTemplate],
    ),
    PANE,
    BACKLOG,
    EDIT,
    LIST_ALL,
    GLOBAL,
]);

/// Backlog > Reqs, browsing the citations.
pub const BACKLOG_REQS: Stack<'static> = Stack::new(&[
    Layer::view(
        Context::BacklogReqs,
        &[
            Act::ReqsNextRow,
            Act::ReqsPrevRow,
            Act::ReqsReconfirm,
            Act::ReqsCite,
            Act::ReqsUncite,
        ],
    ),
    PANE_PAGE,
    BACKLOG,
    EDIT,
    LIST_ALL,
    GLOBAL,
]);

/// Backlog > Reqs, the requirement picker.
pub const REQS_PICK: Stack<'static> = Stack::new(&[
    Layer::view(
        Context::BacklogReqs,
        &[Act::ReqsNextRow, Act::ReqsPrevRow, Act::ReqsPick],
    ),
    BACK,
    LIST,
    MODAL,
]);

/// Backlog > Reqs, the citation kind question.
pub const REQS_KIND: Stack<'static> = Stack::new(&[
    Layer::view(
        Context::BacklogReqs,
        &[Act::ReqsAddresses, Act::ReqsReserves],
    ),
    BACK,
    MODAL,
]);

/// Backlog > Reqs, the uncite question.
pub const REQS_CONFIRM: Stack<'static> =
    Stack::new(&[Layer::view(Context::BacklogReqs, &[]), CONFIRM, MODAL]);

/// The Backlog's filter form (both row kinds).
pub const BACKLOG_FILTER: Stack<'static> = Stack::new(&[
    Layer::view(
        Context::BacklogFilter,
        &[Act::FilterNextRow, Act::FilterPrevRow, Act::FilterClear],
    ),
    FORM_FIELDS,
    MODAL,
]);

/// The Backlog's item form (also while it saves).
pub const ITEM_FORM: Stack<'static> =
    Stack::new(&[Layer::view(Context::BacklogItem, &[]), FORM, MODAL]);

/// The item form's divergence view.
pub const ITEM_DIVERGENCE: Stack<'static> = Stack::new(&[
    Layer::view(
        Context::BacklogItem,
        &[Act::ItemTheirs, Act::ItemMine, Act::ItemSwitchDiff],
    ),
    PAGES,
    BACK,
    LIST,
    MODAL,
]);

/// Chat, idle, live, parked or ended (MOD-67 M5 PA-4: one key arm today).
pub const CHAT_LIVE: Stack<'static> = Stack::new(&[
    Layer::view(
        Context::Chat,
        &[
            Act::ChatCompose,
            Act::ChatNextAgent,
            Act::ChatEnd,
            Act::ChatThoughts,
            Act::ChatTop,
            Act::ChatBottom,
        ],
    ),
    LIST,
    GLOBAL,
]);

/// Chat, replaying a past chat (read-only).
pub const CHAT_REPLAY: Stack<'static> = Stack::new(&[
    Layer::view(
        Context::Chat,
        &[Act::ChatThoughts, Act::ChatTop, Act::ChatBottom],
    ),
    BACK,
    LIST,
    GLOBAL,
]);

/// The queue overlay (MOD-12 M3 D8; MOD-67 M6 D2): its own verbs, the list moves, then the
/// overlay layers the other list overlays compose (`overlay.close`, `global.help`).
pub const QUEUE_OVERLAY: Stack<'static> = Stack::new(&[
    Layer::view(
        Context::Queue,
        &[
            Act::QueueMoveDown,
            Act::QueueMoveUp,
            Act::QueuePause,
            Act::QueueDequeue,
            Act::QueueOpen,
        ],
    ),
    LIST,
    OVERLAY,
    HELP,
]);

/// Settings > Hierarchy, the directory picker while its go-to field is closed (MOD-49; MOD-67 M6
/// D3). With the field open the section is on [`CAPTURE`]: the field types every printable key.
pub const HIERARCHY_PICKER: Stack<'static> = Stack::new(&[
    Layer::view(
        Context::Picker,
        &[
            Act::PickerOpen,
            Act::PickerParent,
            Act::PickerChoose,
            Act::PickerChooseDir,
            Act::PickerGoTo,
            Act::PickerHidden,
        ],
    ),
    BACK,
    LIST,
    MODAL,
]);

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    use super::*;
    use crate::keys::{CATALOGUE, DECLARED, KeyChord, Keys, VIEW_DEFAULTS};

    /// `DECLARED` entries this module does not declare: the shell's own stacks.
    const SHELL: &[Stack<'static>] = &[
        Stack::BASE,
        Stack::OVERLAY,
        Stack::EDITOR_FOCUSED,
        Stack::EDITOR_OPEN,
    ];

    /// `DECLARED` entries that do not open with a view layer.
    const NOT_VIEW_LED: &[Stack<'static>] = &[
        Stack::BASE,
        Stack::OVERLAY,
        Stack::EDITOR_FOCUSED,
        Stack::EDITOR_OPEN,
        SETTINGS_TAB,
        CAPTURE,
        QUIT_CONFIRM,
        ABORT_CONFIRM,
    ];

    /// The views' sources (MOD-67 M3 T7): every file that names a stack of this module or
    /// handles an act a view stack offers.
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
        include_str!("../ui/overlay/abort_confirm.rs"),
        include_str!("../ui/overlay/concepts_search.rs"),
        include_str!("../ui/overlay/migration_prompt.rs"),
        include_str!("../ui/overlay/queue.rs"),
        include_str!("../ui/overlay/quit_confirm.rs"),
        include_str!("../ui/overlay/waiting_list.rs"),
        include_str!("../ui/overlay/workspace_switcher.rs"),
        include_str!("../ui/tabs/skills/mod.rs"),
        include_str!("../ui/tabs/skills/library.rs"),
        include_str!("../ui/tabs/skills/templates.rs"),
        include_str!("../ui/tabs/skills/attach.rs"),
        include_str!("../ui/tabs/skills/agent_help.rs"),
        include_str!("../ui/tabs/requirements/mod.rs"),
        include_str!("../ui/tabs/requirements/forms.rs"),
        include_str!("../ui/tabs/backlog/mod.rs"),
        include_str!("../ui/tabs/backlog/filter.rs"),
        include_str!("../ui/tabs/backlog/item_form.rs"),
        include_str!("../ui/tabs/backlog/divergence.rs"),
        include_str!("../ui/tabs/backlog/detail/mod.rs"),
        include_str!("../ui/tabs/backlog/detail/compose.rs"),
        include_str!("../ui/tabs/backlog/detail/documents.rs"),
        include_str!("../ui/tabs/backlog/detail/graph.rs"),
        include_str!("../ui/tabs/backlog/detail/notes.rs"),
        include_str!("../ui/tabs/backlog/detail/prompt.rs"),
        include_str!("../ui/tabs/backlog/detail/requirements.rs"),
        include_str!("../ui/tabs/backlog/detail/runs.rs"),
        include_str!("../ui/tabs/chat/mod.rs"),
    ];

    /// Act sites beside [`VIEWS`] (MOD-67 M6 D7): files that handle a view stack's acts without
    /// naming a stack (`ChatTop`/`ChatBottom` are named only in the transcript), and the
    /// directory picker, whose host names its stack.
    const ACT_SITES: &[&str] = &[
        include_str!("../ui/tabs/chat/transcript.rs"),
        include_str!("../ui/path_picker.rs"),
    ];

    /// `source`'s production code: up to its test module, its comment lines dropped, so a name
    /// that occurs only in a test or a comment does not count.
    fn production(source: &str) -> String {
        let code = source.split("\n#[cfg(test)]").next().unwrap_or(source);
        code.lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Whether `wanted` occurs in one of `sources` not followed by an identifier character.
    fn names(sources: &[String], wanted: &str) -> bool {
        sources.iter().any(|source| {
            source.match_indices(wanted).any(|(at, _)| {
                !source[at + wanted.len()..]
                    .starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_')
            })
        })
    }

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
        assert_eq!(resolve(QUIT_CONFIRM, "y"), [Act::ConfirmYes]);
        assert_eq!(resolve(QUIT_CONFIRM, "n"), [Act::ConfirmNo]);
        assert_eq!(
            resolve(QUIT_CONFIRM, "esc"),
            [Act::ConfirmNo, Act::OverlayClose]
        );
        assert_eq!(resolve(QUIT_CONFIRM, "?"), [Act::Help]);
        assert_eq!(resolve(QUIT_CONFIRM, "f1"), [Act::Help]);
        for spec in ["q", "Y", "enter"] {
            assert_eq!(resolve(QUIT_CONFIRM, spec), [], "{spec}");
        }
    }

    /// MOD-67 M4 §4.2: the Skills and Requirements stacks.
    #[test]
    fn the_skills_and_requirements_stacks_resolve_as_designed() {
        assert_eq!(resolve(LIBRARY_BROWSE, "l"), [Act::SkillsSwitchView]);
        assert_eq!(resolve(TEMPLATES_BROWSE, "l"), [Act::SkillsSwitchView]);
        for spec in ["ctrl-l", "ctrl-h", "alt-l"] {
            assert_eq!(resolve(LIBRARY_BROWSE, spec), [], "{spec}");
            assert_eq!(resolve(TEMPLATES_BROWSE, spec), [], "{spec}");
        }
        assert_eq!(resolve(LIBRARY_BROWSE, ","), [Act::SkillsPrevVersion]);
        assert_eq!(resolve(LIBRARY_BROWSE, "E"), [Act::SkillsEditExternally]);
        assert_eq!(resolve(LIBRARY_BROWSE, "ctrl-e"), []);
        assert_eq!(resolve(LIBRARY_BROWSE, "J"), [Act::PaneScrollDown]);
        assert_eq!(resolve(LIBRARY_BROWSE, "q"), [Act::Quit]);
        assert_eq!(resolve(LIBRARY_EDITOR, "tab"), [Act::NextTab]);
        assert_eq!(resolve(LIBRARY_EDITOR, "ctrl-g"), [Act::SkillsAskAgent]);
        assert_eq!(resolve(LIBRARY_EDITOR, "ctrl-s"), [Act::FormSave]);
        assert_eq!(resolve(LIBRARY_EDITOR, "ctrl-e"), [Act::FormExternalEditor]);
        assert_eq!(resolve(LIBRARY_EDITOR, "l"), []);
        assert_eq!(resolve(LIBRARY_EDITOR, "q"), []);
        assert_eq!(resolve(LIBRARY_EDITOR, "f1"), [Act::Help]);
        assert!(!LIBRARY_EDITOR.passes(chord("tab")));
        assert_eq!(resolve(LIBRARY_INFO, "down"), [Act::FormNextField]);
        assert_eq!(resolve(ATTACH_BROWSE, "a"), [Act::LibraryAttach]);
        assert_eq!(resolve(ATTACH_BROWSE, "enter"), [Act::AttachChoose]);
        assert_eq!(resolve(ATTACH_BROWSE, "esc"), [Act::Back]);
        assert_eq!(resolve(ATTACH_FORM, "ctrl-r"), [Act::AttachRepo]);
        assert_eq!(resolve(ATTACH_FORM, "down"), [Act::FormNextField]);
        assert_eq!(resolve(ATTACH_CONFIRM, "ctrl-y"), []);
        assert_eq!(resolve(TEMPLATES_BROWSE, "D"), [Act::TemplatesDiffDefault]);
        assert_eq!(resolve(HELP_ASKING, "down"), [Act::SkillsHelpNextAgent]);
        assert_eq!(resolve(HELP_ASKING, "tab"), [Act::NextTab]);
        assert_eq!(resolve(HELP_PROPOSAL, "y"), [Act::SkillsHelpAccept]);
        assert_eq!(resolve(HELP_PROPOSAL, "esc"), [Act::ConfirmNo]);
        assert_eq!(resolve(HELP_WAITING, "esc"), [Act::SkillsHelpCancel]);
        assert_eq!(resolve(REQUIREMENTS_BROWSE, "/"), [Act::RequirementsFilter]);
        assert_eq!(resolve(REQUIREMENTS_BROWSE, "enter"), [Act::ListFold]);
        assert_eq!(resolve(REQUIREMENTS_BROWSE, "ctrl-j"), []);
        assert_eq!(resolve(REQUIREMENTS_FORM, "down"), [Act::FormNextField]);
        assert_eq!(resolve(REQUIREMENTS_FORM, "q"), []);
        assert_eq!(resolve(REQUIREMENTS_WITHDRAW, "tab"), []);
        assert_eq!(resolve(REQUIREMENTS_WITHDRAW, "ctrl-s"), [Act::FormSave]);
    }

    /// MOD-67 M5 §4.2: the Backlog and Chat stacks.
    #[test]
    fn the_backlog_and_chat_stacks_resolve_as_designed() {
        assert_eq!(resolve(BACKLOG_BODY, "j"), [Act::ListDown]);
        assert_eq!(resolve(BACKLOG_BODY, "J"), [Act::PaneScrollDown]);
        assert_eq!(resolve(BACKLOG_BODY, "l"), [Act::PaneNextSubtab]);
        assert_eq!(resolve(BACKLOG_BODY, "m"), [Act::BacklogOpenGraph]);
        assert_eq!(resolve(BACKLOG_BODY, "f"), [Act::BacklogOpenFilter]);
        assert_eq!(resolve(BACKLOG_BODY, "e"), [Act::Edit]);
        assert_eq!(
            resolve(BACKLOG_BODY, "enter"),
            [Act::BacklogReplay, Act::ListFold]
        );
        assert_eq!(resolve(BACKLOG_BODY, "q"), [Act::Quit]);
        for spec in ["ctrl-l", "alt-j", "ctrl-j"] {
            assert_eq!(resolve(BACKLOG_BODY, spec), [], "{spec}");
        }
        assert_eq!(resolve(RUNS_BROWSE, "J"), [Act::RunsNextStep]);
        assert_eq!(resolve(RUNS_BROWSE, "x"), [Act::RunsReject]);
        assert_eq!(resolve(RUNS_BROWSE, "+"), [Act::RunsZoomIn]);
        assert_eq!(resolve(RUNS_BROWSE, "pgdn"), [Act::PanePageDown]);
        assert_eq!(
            resolve(RUNS_BROWSE, "enter"),
            [Act::BacklogReplay, Act::ListFold]
        );
        assert_eq!(resolve(RUNS_BROWSE, "1"), [Act::SelectTab1]);
        assert_eq!(resolve(RUNS_BROWSE, "j"), [Act::ListDown]);
        assert_eq!(resolve(RUNS_CONFIRM, "y"), [Act::ConfirmYes]);
        assert_eq!(resolve(RUNS_CONFIRM, "esc"), [Act::ConfirmNo]);
        assert_eq!(resolve(RUNS_CONFIRM, "alt-y"), []);
        assert_eq!(resolve(RUNS_CONFIRM, "q"), []);
        assert!(!RUNS_CONFIRM.passes(chord("y")));
        assert!(RUNS_CONFIRM.passes(chord("alt-y")));
        assert!(RUNS_CONFIRM.passes(chord("f1")));
        assert_eq!(resolve(RUNS_COUNTING, "y"), []);
        assert_eq!(resolve(RUNS_COUNTING, "n"), [Act::ConfirmNo]);
        assert_eq!(resolve(RUNS_ARTIFACT, "j"), [Act::PaneScrollDown]);
        assert_eq!(resolve(RUNS_ARTIFACT, "J"), [Act::PaneScrollDown]);
        assert_eq!(resolve(RUNS_ARTIFACT, "esc"), [Act::Back]);
        assert_eq!(resolve(RUNS_ARTIFACT, "down"), []);
        assert_eq!(
            resolve(BACKLOG_GRAPH, "enter"),
            [Act::GraphReroot, Act::BacklogReplay, Act::ListFold]
        );
        assert_eq!(resolve(BACKLOG_GRAPH, "J"), [Act::GraphNextRow]);
        assert_eq!(resolve(BACKLOG_GRAPH, "pgdn"), [Act::PanePageDown]);
        assert_eq!(resolve(BACKLOG_DOCS, "v"), [Act::DocsNewVersion]);
        assert_eq!(resolve(BACKLOG_DOCS, "J"), [Act::DocsNextRow]);
        assert_eq!(resolve(DOCS_COMPOSE, "down"), [Act::FormNextField]);
        assert_eq!(resolve(DOCS_COMPOSE, "ctrl-s"), [Act::FormSave]);
        assert_eq!(resolve(DOCS_COMPOSE, "ctrl-e"), [Act::FormExternalEditor]);
        assert_eq!(resolve(DOCS_COMPOSE, "q"), []);
        assert_eq!(resolve(DOCS_COMPOSE, "f1"), [Act::Help]);
        assert_eq!(resolve(NOTES_COMPOSE, "tab"), []);
        assert_eq!(resolve(NOTES_COMPOSE, "ctrl-s"), [Act::FormSave]);
        assert_eq!(resolve(BACKLOG_NOTES, "a"), [Act::NotesAdd]);
        assert_eq!(resolve(BACKLOG_NOTES, "K"), [Act::PaneScrollUp]);
        assert_eq!(resolve(BACKLOG_PROMPT, "n"), [Act::PromptNextTemplate]);
        assert_eq!(resolve(BACKLOG_PROMPT, "ctrl-n"), []);
        assert_eq!(resolve(BACKLOG_REQS, "c"), [Act::ReqsCite]);
        assert_eq!(resolve(REQS_PICK, "J"), [Act::ReqsNextRow]);
        assert_eq!(resolve(REQS_PICK, "j"), [Act::ListDown]);
        assert_eq!(resolve(REQS_PICK, "enter"), [Act::ReqsPick]);
        assert_eq!(resolve(REQS_PICK, "esc"), [Act::Back]);
        assert_eq!(resolve(REQS_KIND, "a"), [Act::ReqsAddresses]);
        assert_eq!(resolve(REQS_KIND, "esc"), [Act::Back]);
        assert_eq!(resolve(REQS_CONFIRM, "alt-y"), []);
        assert_eq!(resolve(BACKLOG_FILTER, "x"), [Act::FilterClear]);
        assert_eq!(resolve(BACKLOG_FILTER, "j"), [Act::FilterNextRow]);
        assert_eq!(resolve(BACKLOG_FILTER, "down"), [Act::FormNextField]);
        assert_eq!(resolve(BACKLOG_FILTER, "tab"), [Act::FormNextField]);
        assert_eq!(resolve(BACKLOG_FILTER, "q"), []);
        assert_eq!(resolve(ITEM_FORM, "down"), [Act::FormNextField]);
        assert_eq!(resolve(ITEM_FORM, "ctrl-s"), [Act::FormSave]);
        assert_eq!(resolve(ITEM_FORM, "f1"), [Act::Help]);
        assert_eq!(resolve(ITEM_FORM, "q"), []);
        assert_eq!(resolve(ITEM_DIVERGENCE, "T"), [Act::ItemTheirs]);
        assert_eq!(resolve(ITEM_DIVERGENCE, "tab"), [Act::ItemSwitchDiff]);
        assert_eq!(resolve(ITEM_DIVERGENCE, "j"), [Act::ListDown]);
        assert_eq!(resolve(ITEM_DIVERGENCE, "esc"), [Act::Back]);
        assert_eq!(resolve(ITEM_DIVERGENCE, "ctrl-s"), []);
        assert_eq!(resolve(CHAT_LIVE, "enter"), [Act::ChatCompose]);
        assert_eq!(resolve(CHAT_LIVE, "esc"), [Act::ChatEnd]);
        assert_eq!(resolve(CHAT_LIVE, "home"), []);
        assert_eq!(resolve(CHAT_LIVE, "1"), [Act::SelectTab1]);
        assert_eq!(resolve(CHAT_LIVE, "q"), [Act::Quit]);
        assert_eq!(resolve(CHAT_REPLAY, "esc"), [Act::Back]);
        assert_eq!(resolve(CHAT_REPLAY, "i"), []);
        assert_eq!(resolve(CHAT_REPLAY, "t"), [Act::ChatThoughts]);
    }

    /// MOD-67 M6 §4.3: the queue overlay's and the directory picker's stacks.
    #[test]
    fn the_queue_overlay_and_picker_stacks_resolve_as_designed() {
        assert_eq!(resolve(QUEUE_OVERLAY, "J"), [Act::QueueMoveDown]);
        assert_eq!(resolve(QUEUE_OVERLAY, "K"), [Act::QueueMoveUp]);
        assert_eq!(resolve(QUEUE_OVERLAY, "P"), [Act::QueuePause]);
        assert_eq!(resolve(QUEUE_OVERLAY, "Q"), [Act::QueueDequeue]);
        assert_eq!(resolve(QUEUE_OVERLAY, "enter"), [Act::QueueOpen]);
        assert_eq!(resolve(QUEUE_OVERLAY, "j"), [Act::ListDown]);
        assert_eq!(resolve(QUEUE_OVERLAY, "down"), [Act::ListDown]);
        assert_eq!(resolve(QUEUE_OVERLAY, "esc"), [Act::OverlayClose]);
        assert_eq!(resolve(QUEUE_OVERLAY, "?"), [Act::Help]);
        assert_eq!(resolve(QUEUE_OVERLAY, "f1"), [Act::Help]);
        for spec in ["ctrl-j", "alt-J", "q", "1"] {
            assert_eq!(resolve(QUEUE_OVERLAY, spec), [], "{spec}");
        }
        // The global layer is the unfiltered `HELP`; the overlay is modal at the shell.
        assert!(QUEUE_OVERLAY.passes(chord("q")));

        for spec in ["enter", "l"] {
            assert_eq!(resolve(HIERARCHY_PICKER, spec), [Act::PickerOpen], "{spec}");
        }
        for spec in ["h", "backspace"] {
            assert_eq!(
                resolve(HIERARCHY_PICKER, spec),
                [Act::PickerParent],
                "{spec}"
            );
        }
        assert_eq!(resolve(HIERARCHY_PICKER, "s"), [Act::PickerChoose]);
        assert_eq!(resolve(HIERARCHY_PICKER, "S"), [Act::PickerChooseDir]);
        assert_eq!(resolve(HIERARCHY_PICKER, "/"), [Act::PickerGoTo]);
        assert_eq!(resolve(HIERARCHY_PICKER, "."), [Act::PickerHidden]);
        assert_eq!(resolve(HIERARCHY_PICKER, "esc"), [Act::Back]);
        assert_eq!(resolve(HIERARCHY_PICKER, "j"), [Act::ListDown]);
        for spec in ["q", "?", "1"] {
            assert_eq!(resolve(HIERARCHY_PICKER, spec), [], "{spec}");
        }
        assert_eq!(resolve(HIERARCHY_PICKER, "f1"), [Act::Help]);
        assert_eq!(resolve(HIERARCHY_PICKER, "ctrl-f"), [Act::Find]);
        assert!(!HIERARCHY_PICKER.passes(chord("q")));
        assert!(HIERARCHY_PICKER.passes(chord("ctrl-j")));
    }

    /// MOD-67 M5 §4 (D5): every Backlog browse stack ends in the same tail, so the Backlog
    /// tab's own verbs and the list's resolve on every sub-tab.
    #[test]
    fn every_backlog_browse_stack_offers_the_tab_and_the_list() {
        for (name, stack) in [
            ("BACKLOG_BODY", BACKLOG_BODY),
            ("RUNS_BROWSE", RUNS_BROWSE),
            ("BACKLOG_GRAPH", BACKLOG_GRAPH),
            ("BACKLOG_DOCS", BACKLOG_DOCS),
            ("BACKLOG_NOTES", BACKLOG_NOTES),
            ("BACKLOG_PROMPT", BACKLOG_PROMPT),
            ("BACKLOG_REQS", BACKLOG_REQS),
        ] {
            assert_eq!(resolve(stack, "f"), [Act::BacklogOpenFilter], "{name}");
            assert_eq!(resolve(stack, "N"), [Act::BacklogNewItem], "{name}");
            assert_eq!(resolve(stack, "e"), [Act::Edit], "{name}");
            assert_eq!(resolve(stack, "l"), [Act::PaneNextSubtab], "{name}");
            assert_eq!(resolve(stack, "G"), [Act::ListBottom], "{name}");
            let enter = resolve(stack, "enter");
            assert!(
                enter.contains(&Act::BacklogReplay) && enter.contains(&Act::ListFold),
                "{name}: {enter:?}"
            );
            assert_eq!(resolve(stack, "q"), [Act::Quit], "{name}");
            let layers = stack.layers();
            assert_eq!(
                layers[layers.len() - 4..],
                [BACKLOG, EDIT, LIST_ALL, GLOBAL],
                "{name}"
            );
        }
    }

    /// MOD-67 M4 D5: a capturing or confirming mode never offers the Skills view switch, so a
    /// letter it types never switches views.
    #[test]
    fn no_capturing_stack_offers_the_view_switch() {
        for &(phrase, stack) in DECLARED {
            let Some((_, global)) = stack.global() else {
                continue;
            };
            if !global.is_modal() {
                continue;
            }
            // Where a row of the act could sit: a `skills` layer, or a view layer inheriting it.
            for (index, layer) in stack.layers().iter().enumerate() {
                let holds = layer.context() == Context::Skills || layer.inherits();
                assert!(
                    !(holds && stack.admits(index, Act::SkillsSwitchView)),
                    "{phrase}: layer {index} offers skills.switch_view"
                );
            }
        }
    }

    #[test]
    fn every_view_layer_comes_first_and_every_modal_stack_ends_modal() {
        let views: Vec<_> = DECLARED
            .iter()
            .filter(|(_, stack)| !NOT_VIEW_LED.contains(stack))
            .collect();
        assert_eq!(views.len(), 72);
        for &&(phrase, stack) in &views {
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
            // MOD-67 M4 PA-2: a second global layer is `TABS`, just before the modal one.
            let globals: Vec<usize> = (0..stack.layers().len())
                .filter(|&at| stack.layers()[at].context() == Context::Global)
                .collect();
            if let [tabs, _] = globals[..] {
                assert_eq!(tabs, index - 1, "{phrase}: TABS is not just before MODAL");
                assert!(
                    global.is_modal(),
                    "{phrase}: TABS before an unfiltered layer"
                );
                let offered: Vec<Act> = CATALOGUE
                    .iter()
                    .filter(|row| row.context == Context::Global && stack.admits(tabs, row.act))
                    .map(|row| row.act)
                    .collect();
                assert_eq!(offered, [Act::NextTab, Act::PrevTab], "{phrase}");
                assert!(!stack.layers()[tabs].is_modal(), "{phrase}");
            } else {
                assert_eq!(globals.len(), 1, "{phrase}: more than two global layers");
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
    /// shell's own stacks (`SHELL`) are not declared here; a stack nothing names would be
    /// validated (and refuse key files) for a mode no view is ever in.
    #[test]
    fn every_view_stack_is_named_by_a_view() {
        // Production code only, so a stack named only by a test or a doc comment still counts
        // as unnamed.
        let production: Vec<String> = VIEWS.iter().map(|source| production(source)).collect();
        let named = |name: &str| names(&production, &format!("views::{name}"));
        let declared: Vec<&str> = include_str!("views.rs")
            .lines()
            .filter_map(|line| line.strip_prefix("pub const "))
            .filter_map(|rest| rest.split_once(": Stack<'static>").map(|(name, _)| name))
            .collect();
        // `BASE`, `OVERLAY` and the editor stacks live in `stack.rs`.
        assert_eq!(
            declared.len(),
            DECLARED
                .iter()
                .filter(|(_, stack)| !SHELL.contains(stack))
                .count(),
            "views.rs stacks vs DECLARED"
        );
        for name in declared {
            assert!(named(name), "no view names views::{name}");
        }
    }
    /// MOD-67 M6 D7 (ANA-26 §8, risk "a stack action no view handles"): every catalogue act is
    /// offered by some declared stack in a layer of its own context, and every act a view stack
    /// offers outside the global, overlay and editor contexts is named `Act::X` in some view's
    /// production code (the include-and-strip technique of `every_view_stack_is_named_by_a_view`).
    #[test]
    fn every_act_is_offered_by_a_stack_and_handled_by_a_view() {
        for spec in CATALOGUE {
            assert!(
                DECLARED
                    .iter()
                    .any(|(_, stack)| stack.view_admits(spec.context, spec.act)),
                "no declared stack offers [{}] {}",
                spec.context.table(),
                spec.name
            );
        }
        let production: Vec<String> = VIEWS
            .iter()
            .chain(ACT_SITES)
            .map(|source| production(source))
            .collect();
        for &(phrase, stack) in DECLARED.iter().filter(|(_, stack)| !SHELL.contains(stack)) {
            for (index, layer) in stack.layers().iter().enumerate() {
                if matches!(
                    layer.context(),
                    Context::Global | Context::Overlay | Context::Editor
                ) {
                    continue;
                }
                for spec in CATALOGUE
                    .iter()
                    .filter(|spec| spec.context == layer.context() && stack.admits(index, spec.act))
                {
                    let act = spec.act;
                    assert!(
                        names(&production, &format!("Act::{act:?}")),
                        "{phrase}: no view handles Act::{act:?} ([{}] {})",
                        spec.context.table(),
                        spec.name
                    );
                }
            }
        }
    }
}
