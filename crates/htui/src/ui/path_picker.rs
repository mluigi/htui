//! A directory picker a section owns as a mode (MOD-49 P2): not a registered overlay, because an
//! overlay factory takes no start path and has no way to hand a chosen path back.
//!
//! It holds no store handle and no channel: listings arrive through [`PathPicker::on_reply`], and
//! requests leave as [`PickerOutcome::Request`] for the owning section to send (`R-NF-3`).
//! Navigation is lexical and never canonicalises, so nothing on screen names a link's target (P5,
//! `R-BOX-4`): the one canonicalisation is the write the section sends on [`PickerOutcome::Chosen`].
//!
//! Its keys resolve through the owner's stack (`views::HIERARCHY_PICKER` for Settings ›
//! Hierarchy, MOD-67 M6), from the chord the user pressed (MOD-115 M3 MD-A); the go-to field's
//! keys are the field's.
//!
//! It draws through the kit (MOD-115 M3): a `kit::modal` box, the entries a `kit::list` (the
//! cursor is the list's gutter and band, MOD-85), the path header in the title role and the go-to
//! field a one-row `kit::form`.

use std::path::{Component, Path, PathBuf};

use crossterm::event::KeyEvent;
use htui_core::root_path::DirListing;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::Paragraph;

use crate::keys::{Act, Hint, HintSpec, KeyChord, Keys, Stack};
use crate::store_worker::{LIST_DIR, StoreReply, StoreRequest};
use crate::ui::cells::{cell_width, graphemes};
use crate::ui::kit::form::{Field, Form, Value};
use crate::ui::kit::list::{self, RowKind};
use crate::ui::kit::modal::{Modal, ModalSize};
use crate::ui::{FieldOutcome, TextField, Theme};

/// The list's gutter width, in front of every line that is not an entry, so the texts stay in the
/// entries' column (the cursor is the gutter's `▌`, MOD-115 M3).
const INDENT: &str = " ";

/// The first half of the hint under the entries: the keys that move and choose (MOD-49 P8;
/// MOD-67 M6 D3), through the owner's stack: `j/k move · Enter open · h up · s choose · S this
/// dir` under the defaults. The two halves share one line when it fits and take one line each when
/// it doesn't (review L-2).
const HINT_MOVE: HintSpec = &[
    Hint::Pair(Act::ListDown, Act::ListUp, "move"),
    Hint::One(Act::PickerOpen, "open"),
    Hint::One(Act::PickerParent, "up"),
    Hint::One(Act::PickerChoose, "choose"),
    Hint::One(Act::PickerChooseDir, "this dir"),
];

/// The hint's second half: the keys that change what is listed, and the way out: `/ go to ·
/// . hidden · Esc cancel` under the defaults.
const HINT_MORE: HintSpec = &[
    Hint::One(Act::PickerGoTo, "go to"),
    Hint::One(Act::PickerHidden, "hidden"),
    Hint::One(Act::Back, "cancel"),
];

/// Between the two halves when they share a line.
const HINT_GAP: &str = " \u{b7} ";

/// The go-to field's label (P6), its one-row form's.
const GOTO: &str = "go to";

/// The widest the popup gets (blueprint D14): a listing can't be sized to its content.
const MAX_WIDTH: u16 = 96;

/// What one key did (MOD-49 P8).
#[derive(Debug, Clone)]
pub enum PickerOutcome {
    /// Handled here (moved, typed, or a declined key of the picker's): nothing for the section to
    /// do.
    Stay,
    /// A listing to send through `ctx.request` — a read, never marked busy (P11).
    Request(StoreRequest),
    /// The directory the user chose, as navigated (never canonicalised; the write does that).
    Chosen(String),
    /// `common.back` outside go-to: close the picker, write nothing.
    Cancelled,
}

/// A directory picker over this box's filesystem (MOD-49).
#[derive(Debug)]
pub struct PathPicker {
    /// The popup's title, e.g. "Root of `graphics` on this box" (blueprint D22).
    title: String,
    /// The directory the header shows and `S` chooses: the last path listed or refused (D10).
    path: String,
    /// The path the newest `ListDir` named; a listing of any other path is ignored (P9).
    asked: String,
    /// The last listing of `path`, or `None` while the first one is read and after a refusal.
    listing: Option<DirListing>,
    /// The refusal of `asked`, shown in the popup (P7).
    error: Option<String>,
    /// Index into `listing.entries`.
    cursor: usize,
    /// Whether the newest request asked for `.`-entries (`.` toggles it, blueprint D1).
    show_hidden: bool,
    /// The one-line go-to field, while open (P6); its `Debug` never prints the text.
    goto: Option<TextField>,
    /// The child to highlight when the next listing lands — set by going up (D13).
    reselect: Option<String>,
}

/// The directory a picker opens on (MOD-49 P7, blueprint D9): `stored` when set, else `fallback`
/// (a repo's workspace root on this box), else `home`, else `/`. Empty strings count as unset.
#[must_use]
pub fn start_dir(stored: Option<&str>, fallback: Option<&str>, home: Option<&str>) -> String {
    [stored, fallback, home]
        .into_iter()
        .flatten()
        .find(|path| !path.is_empty())
        .unwrap_or("/")
        .to_owned()
}

impl PathPicker {
    /// A picker on `start`, and the request that lists it: `ListDir { path: start, show_hidden:
    /// false }`. `path` is `start` from the first frame, so a refused start still has a header and
    /// `h` still goes up (P7).
    #[must_use]
    pub fn open(title: impl Into<String>, start: String) -> (Self, StoreRequest) {
        let mut picker = Self {
            title: title.into(),
            path: start.clone(),
            asked: String::new(),
            listing: None,
            error: None,
            cursor: 0,
            show_hidden: false,
            goto: None,
            reselect: None,
        };
        let request = picker.ask(start);
        (picker, request)
    }

    /// The directory on screen.
    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Whether the go-to field is open: the owner puts the picker on a capture stack then.
    #[must_use]
    pub const fn goto_open(&self) -> bool {
        self.goto.is_some()
    }

    /// One key (MOD-49 P8, blueprint D11, D12, D15; MOD-67 M6 D3). While go-to is open the field
    /// takes it (its keys are the widget's, ANA-26 §6.1) and the answer is always `Some`.
    /// Otherwise `pressed`, the chord the user pressed (the owner's `Ctx::pressed`, MOD-115 M3
    /// MD-A), resolves through `stack` (the owner's picker stack) and the first act the picker
    /// takes wins; a chord equal to no picker act is `None`, for the owner's modal rest (`q`, `?`,
    /// the digits are swallowed there, `F1` and `ctrl-c` pass).
    pub fn on_key(
        &mut self,
        key: KeyEvent,
        pressed: KeyChord,
        keys: &Keys,
        stack: Stack<'_>,
    ) -> Option<PickerOutcome> {
        if let Some(field) = &mut self.goto {
            return Some(match field.on_key(key) {
                FieldOutcome::Submit => {
                    let typed = field.text().unwrap_or_default().trim().to_owned();
                    self.goto = None;
                    if typed.is_empty() {
                        PickerOutcome::Stay
                    } else {
                        PickerOutcome::Request(self.ask(typed))
                    }
                }
                FieldOutcome::Cancel => {
                    self.goto = None;
                    PickerOutcome::Stay
                }
                FieldOutcome::Consumed | FieldOutcome::Pass => PickerOutcome::Stay,
            });
        }
        keys.actions(stack, pressed)
            .into_iter()
            .find_map(|act| self.on_act(act))
    }

    /// One resolved act; `None` for an act that is not the picker's (a global one).
    fn on_act(&mut self, act: Act) -> Option<PickerOutcome> {
        Some(match act {
            Act::ListDown => {
                let last = self.entry_count().saturating_sub(1);
                self.cursor = (self.cursor + 1).min(last);
                PickerOutcome::Stay
            }
            Act::ListUp => {
                self.cursor = self.cursor.saturating_sub(1);
                PickerOutcome::Stay
            }
            Act::PickerOpen => match self.highlighted() {
                Some(next) => PickerOutcome::Request(self.ask(next)),
                None => PickerOutcome::Stay,
            },
            Act::PickerParent => match parent(&self.path) {
                Some(up) => {
                    let came_from = Path::new(&self.path)
                        .file_name()
                        .and_then(|name| name.to_str())
                        .map(str::to_owned);
                    let request = self.ask(up);
                    // After `ask`, which forgets any older one (review N-1).
                    self.reselect = came_from;
                    PickerOutcome::Request(request)
                }
                None => PickerOutcome::Stay,
            },
            Act::PickerChoose => self
                .highlighted()
                .map_or(PickerOutcome::Stay, PickerOutcome::Chosen),
            // A declined `S` (no listing) is still the picker's: consumed, as before.
            Act::PickerChooseDir => {
                if self.listing.is_some() {
                    PickerOutcome::Chosen(self.path.clone())
                } else {
                    PickerOutcome::Stay
                }
            }
            Act::PickerGoTo => {
                self.goto = Some(TextField::with_text("/"));
                PickerOutcome::Stay
            }
            Act::PickerHidden => {
                self.show_hidden = !self.show_hidden;
                PickerOutcome::Request(self.ask(self.path.clone()))
            }
            Act::Back => PickerOutcome::Cancelled,
            // Not the picker's: the owner's modal rest swallows it or passes it (D15).
            _ => return None,
        })
    }

    /// A bracketed paste (P6, blueprint D12): into go-to when it is open, opening it otherwise.
    pub fn on_paste(&mut self, text: &str) {
        match &mut self.goto {
            // Go-to opens holding `/`; an absolute paste over that lone slash replaces it, so a
            // pasted path never reads `//srv`.
            Some(field) if field.text() == Some("/") && text.trim_start().starts_with('/') => {
                *field = TextField::new();
                field.on_paste(text);
            }
            Some(field) => {
                field.on_paste(text);
            }
            None => {
                let mut field = TextField::new();
                field.on_paste(text);
                self.goto = Some(field);
            }
        }
    }

    /// A reply addressed to the section; `true` when it was this picker's (blueprint D10).
    pub fn on_reply(&mut self, reply: &StoreReply) -> bool {
        match reply {
            StoreReply::DirListing(listing) if listing.path == self.asked => {
                let reselect = self.reselect.take();
                self.cursor = reselect
                    .and_then(|name| listing.entries.iter().position(|e| e.name == name))
                    .unwrap_or(0);
                self.path.clone_from(&self.asked);
                self.listing = Some(listing.clone());
                self.error = None;
                true
            }
            StoreReply::Failed {
                request: LIST_DIR,
                message,
            } => {
                self.path.clone_from(&self.asked);
                self.listing = None;
                self.error = Some(message.clone());
                self.cursor = 0;
                self.reselect = None;
                true
            }
            _ => false,
        }
    }

    /// The popup, centred over `area` (blueprint D14): the kit's modal at a fixed size, because a
    /// listing of up to `LIST_CAP` entries can't be sized to its content. Inside it the path header,
    /// the entries (a `kit::list`), then the footer: go-to while open, a blank line and the hint.
    /// Its hint is rendered through `stack` with `keys` (MOD-67 M6), so a rebound picker key shows
    /// its new chord.
    pub fn render(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        theme: &Theme,
        keys: &Keys,
        stack: Stack<'_>,
    ) {
        let hints = (keys.hint(stack, HINT_MOVE), keys.hint(stack, HINT_MORE));
        let hidden = keys.label(stack, Act::PickerHidden);
        let content = Modal::new(&self.title)
            .size(ModalSize::Fixed {
                w: area.width.saturating_sub(4).min(MAX_WIDTH),
                h: area.height.saturating_sub(2),
            })
            .render(frame, area, theme);
        if content.width == 0 || content.height == 0 {
            return;
        }
        let footer = self.footer(content.width, theme, (&hints.0, &hints.1));
        frame.render_widget(
            Paragraph::new(Line::styled(
                clip_left(&self.path, usize::from(content.width)),
                theme.title,
            )),
            Rect {
                height: 1,
                ..content
            },
        );
        let room = usize::from(content.height).saturating_sub(1 + footer.len());
        let (rows, selected) = self.body(content.width, room, theme, hidden.as_deref());
        let body = Rect {
            y: content.y + 1,
            height: u16::try_from(rows.len().min(room)).unwrap_or(u16::MAX),
            ..content
        };
        // The modal's own accent marks the cursor, as its border does.
        list::render(frame, body, &rows, selected, 0, theme.accent, theme);
        let below = Rect {
            y: body.bottom(),
            height: content.bottom().saturating_sub(body.bottom()),
            ..content
        };
        frame.render_widget(Paragraph::new(footer), below);
    }

    /// `ask(p)`: remember `p` as the newest path asked for (P9) and build its request. `.` and
    /// `..` are resolved lexically first (review L-1), so the kernel never resolves a `..` after a
    /// link and lists the link target's parent (P5, `R-BOX-4`).
    ///
    /// Every request forgets the place an older one meant to reselect (review N-1): going up sets
    /// it again after asking, so it only ever applies to the parent it was set for.
    fn ask(&mut self, path: String) -> StoreRequest {
        let path = lexical(path);
        self.asked.clone_from(&path);
        self.reselect = None;
        StoreRequest::ListDir {
            path,
            show_hidden: self.show_hidden,
        }
    }

    /// How many entries the landed listing holds.
    fn entry_count(&self) -> usize {
        self.listing.as_ref().map_or(0, |l| l.entries.len())
    }

    /// The highlighted entry as a path under `path` (lexical, D23), when a listing has one.
    fn highlighted(&self) -> Option<String> {
        let entry = self.listing.as_ref()?.entries.get(self.cursor)?;
        Some(child(&self.path, &entry.name))
    }

    /// The footer for a content `width` wide: go-to while open (a one-row kit form, its marker in
    /// the modal's accent), a blank line and the hint's two rendered halves (`hints`; an empty one
    /// dropped). The two halves share one line when it fits and take one line each when it
    /// doesn't (review L-2).
    fn footer(&self, width: u16, theme: &Theme, hints: (&str, &str)) -> Vec<Line<'static>> {
        let mut footer = Vec::new();
        if let Some(field) = &self.goto {
            footer.extend(
                Form::new()
                    .field(Field::new(GOTO, Value::Text(field)))
                    .focus(0)
                    .editing(true)
                    .lines(width, theme.accent, theme),
            );
        }
        footer.push(Line::raw(""));
        let (first, second) = hints;
        let one_line = format!("{INDENT}{first}{HINT_GAP}{second}");
        if !first.is_empty() && !second.is_empty() && cell_width(&one_line) <= usize::from(width) {
            footer.push(Line::styled(one_line, theme.dim));
        } else {
            for half in [first, second].into_iter().filter(|half| !half.is_empty()) {
                footer.push(Line::styled(format!("{INDENT}{half}"), theme.dim));
            }
        }
        footer
    }

    /// The body's list rows in a window of `rows` lines that always holds the cursor, then `+N
    /// more`, with the cursor's index among them; or the error, "reading…", or the empty text as
    /// note rows — three different screens (the switcher's rule). The error is wrapped to the
    /// list's content width (review L-2): it names a path, and a long one would push the reason
    /// off the edge. The empty text names `hidden`, the label of `picker.hidden`, and no key when
    /// it is unbound (MOD-67 M6 D4).
    fn body(
        &self,
        width: u16,
        rows: usize,
        theme: &Theme,
        hidden: Option<&str>,
    ) -> (Vec<list::Row<'static>>, Option<usize>) {
        let note = |text: String, style| list::Row {
            line: Line::styled(text, style),
            kind: RowKind::Note,
        };
        if let Some(error) = &self.error {
            // The gutter's cell is the list's own.
            let room = usize::from(width).saturating_sub(cell_width(INDENT));
            let notes = wrap(error, room)
                .into_iter()
                .map(|line| note(line, theme.error))
                .collect();
            return (notes, None);
        }
        let Some(listing) = &self.listing else {
            return (vec![note("reading\u{2026}".to_owned(), theme.dim)], None);
        };
        if listing.entries.is_empty() {
            let hidden = match hidden {
                Some(label) if !self.show_hidden => format!(" \u{b7} {label} shows hidden"),
                _ => String::new(),
            };
            return (
                vec![note(format!("no directories here{hidden}"), theme.dim)],
                None,
            );
        }
        let window = rows.saturating_sub(usize::from(listing.more > 0)).max(1);
        // Stateless scroll: the window ends on the cursor once it passes the bottom, so `render`
        // stays `&self` and the cursor is always on screen.
        let offset = self.cursor.saturating_sub(window - 1);
        let mut lines: Vec<list::Row<'static>> = listing
            .entries
            .iter()
            .skip(offset)
            .take(window)
            .map(|entry| {
                // `ls -F` marks: a link is `@` and never shows where it points (`R-BOX-4`).
                let kind = if entry.is_link { '@' } else { '/' };
                list::Row {
                    line: Line::styled(format!("{}{kind}", entry.name), theme.base),
                    kind: RowKind::Item,
                }
            })
            .collect();
        if listing.more > 0 {
            lines.push(note(format!("+{} more", listing.more), theme.dim));
        }
        (lines, Some(self.cursor - offset))
    }
}

/// `path/name`, lexically (blueprint D23): `Path::join`, never a `canonicalize`.
fn child(path: &str, name: &str) -> String {
    Path::new(path).join(name).to_string_lossy().into_owned()
}

/// The lexical parent (blueprint D23): `None` at `/`, and an empty parent (a relative path) is
/// `/`. A path reaches here already [`lexical`], so it holds no `..`.
fn parent(path: &str) -> Option<String> {
    let up = Path::new(path).parent()?;
    if up.as_os_str().is_empty() {
        Some("/".to_owned())
    } else {
        Some(up.to_string_lossy().into_owned())
    }
}

/// `path` with its `.` and `..` components resolved lexically (MOD-49 review L-1, amending
/// blueprint D23): `..` pops one component and never climbs above `/`. A path with neither, or a
/// relative one (the worker refuses it by name), comes back exactly as given, trailing slash and
/// all, so the header shows what was typed.
fn lexical(path: String) -> String {
    let given = Path::new(&path);
    let dotted = given
        .components()
        .any(|part| matches!(part, Component::CurDir | Component::ParentDir));
    if !given.is_absolute() || !dotted {
        return path;
    }
    let mut resolved = PathBuf::from("/");
    for part in given.components() {
        match part {
            Component::ParentDir => {
                resolved.pop();
            }
            Component::Normal(name) => resolved.push(name),
            Component::RootDir | Component::CurDir | Component::Prefix(_) => {}
        }
    }
    resolved.to_string_lossy().into_owned()
}

/// `text` in lines of at most `width` cells (MOD-49 review L-2): broken at spaces, and inside a
/// word only when the word alone is wider than a line (a long path usually is).
fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split(' ') {
        let mut word = word.to_owned();
        loop {
            let room = if line.is_empty() {
                width
            } else {
                width.saturating_sub(cell_width(&line) + 1)
            };
            if cell_width(&word) <= room {
                if !line.is_empty() {
                    line.push(' ');
                }
                line.push_str(&word);
                break;
            }
            if !line.is_empty() {
                lines.push(std::mem::take(&mut line));
                continue;
            }
            // Alone and still too wide: as many clusters as fit (at least one, so this ends).
            let clusters: Vec<&str> = graphemes(&word).collect();
            let mut used = 0;
            let mut cut = 0;
            for cluster in &clusters {
                let w = cell_width(cluster);
                if cut > 0 && used + w > width {
                    break;
                }
                used += w;
                cut += 1;
            }
            lines.push(clusters[..cut].concat());
            word = clusters[cut..].concat();
            if word.is_empty() {
                break;
            }
        }
    }
    if !line.is_empty() || lines.is_empty() {
        lines.push(line);
    }
    lines
}

/// `text` in at most `width` cells, cut from the **left** behind a `…`: the end of a path is the
/// part that tells directories apart.
fn clip_left(text: &str, width: usize) -> String {
    if cell_width(text) <= width {
        return text.to_owned();
    }
    let budget = width.saturating_sub(1);
    let clusters: Vec<&str> = graphemes(text).collect();
    let mut used = 0;
    let mut start = clusters.len();
    for (index, cluster) in clusters.iter().enumerate().rev() {
        let w = cell_width(cluster);
        if used + w > budget {
            break;
        }
        used += w;
        start = index;
    }
    format!("\u{2026}{}", clusters[start..].concat())
}

#[cfg(test)]
mod tests {
    use super::{PathPicker, PickerOutcome, start_dir};
    use crate::keys::{KeyChord, Keys, load_str, views};
    use crate::store_worker::{LIST_DIR, StoreReply, StoreRequest};
    use crate::ui::Theme;
    use crate::ui::cells::cell_width;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use htui_core::root_path::{DirEntry, DirListing};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::style::Modifier;

    /// A key with no modifiers.
    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::from(code)
    }

    /// A plain character key.
    fn ch(c: char) -> KeyEvent {
        key(KeyCode::Char(c))
    }

    /// `c` with `modifiers` held.
    fn chord(c: char, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), modifiers)
    }

    /// `key` through the picker as Hierarchy feeds it: the compiled keys and the picker stack, a
    /// key no picker act takes read as swallowed (`Stay`), as Hierarchy's modal rest does.
    fn press(picker: &mut PathPicker, key: KeyEvent) -> PickerOutcome {
        picker
            .on_key(
                key,
                KeyChord::from_event(key),
                Keys::compiled(),
                views::HIERARCHY_PICKER,
            )
            .unwrap_or(PickerOutcome::Stay)
    }

    /// Keys loaded from a key file's `[picker]` table.
    fn picker_keys(table: &str) -> Keys {
        load_str(&format!("version = 1\n[picker]\n{table}")).expect("the key file loads")
    }

    /// A listing of `path` by hand: `(name, is_link)` per entry.
    fn dirs(path: &str, names: &[(&str, bool)], more: usize) -> StoreReply {
        StoreReply::DirListing(DirListing {
            path: path.to_owned(),
            entries: names
                .iter()
                .map(|(name, is_link)| DirEntry {
                    name: (*name).to_owned(),
                    is_link: *is_link,
                })
                .collect(),
            more,
        })
    }

    /// The `ListDir` an outcome carries, as `(path, show_hidden)`, or a panic naming the outcome.
    #[track_caller]
    fn asked(outcome: PickerOutcome) -> (String, bool) {
        match outcome {
            PickerOutcome::Request(StoreRequest::ListDir { path, show_hidden }) => {
                (path, show_hidden)
            }
            other => panic!("expected a listing request: {other:?}"),
        }
    }

    /// The path an outcome chose, or a panic naming the outcome.
    #[track_caller]
    fn chosen(outcome: PickerOutcome) -> String {
        match outcome {
            PickerOutcome::Chosen(path) => path,
            other => panic!("expected a choice: {other:?}"),
        }
    }

    /// Whether an outcome is `Stay`.
    fn stays(outcome: &PickerOutcome) -> bool {
        matches!(outcome, PickerOutcome::Stay)
    }

    /// The go-to field's value on a rendered frame (MOD-115 M3: a one-row `kit::form`, the marker
    /// then `go to` and the field), or `None` while go-to is closed.
    fn goto_value(frame: &str) -> Option<String> {
        let line = frame.lines().find(|line| line.contains("\u{258c}go to"))?;
        let (_, rest) = line.split_once("\u{258c}go to")?;
        Some(rest.trim_end_matches('\u{2502}').trim().to_owned())
    }

    /// A picker opened on `/srv` with `htui/`, `notes/` and `shared@` listed.
    fn at_srv() -> PathPicker {
        let (mut picker, _) = PathPicker::open("Root", "/srv".to_owned());
        assert!(picker.on_reply(&dirs(
            "/srv",
            &[("htui", false), ("notes", false), ("shared", true)],
            0
        )));
        picker
    }

    /// The popup over a blank 100×30 frame, one trimmed line per row.
    fn render(picker: &PathPicker) -> String {
        render_at(picker, 100, 30)
    }

    /// The popup over a blank `width`×`height` frame, one trimmed line per row.
    fn render_at(picker: &PathPicker, width: u16, height: u16) -> String {
        render_with(picker, Keys::compiled(), width, height)
    }

    /// The popup over a blank `width`×`height` frame with `keys`, one trimmed line per row.
    fn render_with(picker: &PathPicker, keys: &Keys, width: u16, height: u16) -> String {
        let mut term = Terminal::new(TestBackend::new(width, height)).expect("a test backend");
        term.draw(|frame| {
            picker.render(
                frame,
                frame.area(),
                &Theme::default(),
                keys,
                views::HIERARCHY_PICKER,
            );
        })
        .expect("the picker draws");
        let buffer = term.backend().buffer();
        let mut out = String::new();
        for y in 0..buffer.area.height {
            let mut line = String::new();
            let mut x = 0;
            while x < buffer.area.width {
                let symbol = buffer[(x, y)].symbol();
                line.push_str(symbol);
                x += u16::try_from(cell_width(symbol).max(1)).unwrap_or(1);
            }
            out.push_str(line.trim_end());
            out.push('\n');
        }
        out
    }

    #[test]
    fn start_dir_prefers_stored_then_fallback_then_home_then_root() {
        let (s, f, h) = (Some("/stored"), Some("/root"), Some("/home/u"));
        assert_eq!(start_dir(s, f, h), "/stored");
        assert_eq!(start_dir(None, f, h), "/root");
        assert_eq!(start_dir(Some(""), f, h), "/root");
        assert_eq!(start_dir(None, None, h), "/home/u");
        assert_eq!(start_dir(None, Some(""), h), "/home/u");
        assert_eq!(start_dir(None, None, None), "/");
        assert_eq!(start_dir(None, None, Some("")), "/");
    }

    #[test]
    fn open_asks_for_the_start() {
        let (picker, request) = PathPicker::open("Root", "/srv".to_owned());
        assert_eq!(
            asked(PickerOutcome::Request(request)),
            ("/srv".to_owned(), false)
        );
        assert_eq!(picker.path(), "/srv");
    }

    /// P9: a listing of a path the picker no longer wants changes nothing.
    #[test]
    fn a_listing_for_another_path_is_ignored() {
        let (mut picker, _) = PathPicker::open("Root", "/srv".to_owned());
        assert!(!picker.on_reply(&dirs("/elsewhere", &[("stray", false)], 0)));
        assert_eq!(picker.path(), "/srv");
        assert!(!render(&picker).contains("stray"));
        assert!(
            stays(&press(&mut picker, ch('s'))),
            "nothing landed to choose from"
        );
    }

    #[test]
    fn j_and_k_clamp_at_both_ends() {
        let mut picker = at_srv();
        assert!(stays(&press(&mut picker, ch('k'))));
        assert_eq!(chosen(press(&mut picker, ch('s'))), "/srv/htui");
        for _ in 0..5 {
            press(&mut picker, ch('j'));
        }
        assert_eq!(chosen(press(&mut picker, ch('s'))), "/srv/shared");
        press(&mut picker, key(KeyCode::Up));
        assert_eq!(chosen(press(&mut picker, ch('s'))), "/srv/notes");
        press(&mut picker, key(KeyCode::Down));
        assert_eq!(chosen(press(&mut picker, ch('s'))), "/srv/shared");
    }

    #[test]
    fn enter_and_l_open_the_highlighted_directory() {
        let mut picker = at_srv();
        assert_eq!(
            asked(press(&mut picker, key(KeyCode::Enter))),
            ("/srv/htui".to_owned(), false)
        );
        let mut picker = at_srv();
        assert_eq!(
            asked(press(&mut picker, ch('l'))),
            ("/srv/htui".to_owned(), false)
        );
    }

    /// P5: up from a link is the link's lexical parent, never its target's.
    #[test]
    fn h_and_backspace_go_to_the_lexical_parent() {
        for up in [ch('h'), key(KeyCode::Backspace)] {
            let mut picker = at_srv();
            press(&mut picker, ch('j'));
            press(&mut picker, ch('j'));
            let (shared, _) = asked(press(&mut picker, ch('l')));
            assert_eq!(shared, "/srv/shared");
            assert!(picker.on_reply(&dirs("/srv/shared", &[("inside", false)], 0)));
            assert_eq!(picker.path(), "/srv/shared");
            assert_eq!(asked(press(&mut picker, up)), ("/srv".to_owned(), false));
        }
    }

    #[test]
    fn h_at_the_root_stays_at_the_root() {
        let (mut picker, _) = PathPicker::open("Root", "/".to_owned());
        assert!(picker.on_reply(&dirs("/", &[("srv", false)], 0)));
        assert!(stays(&press(&mut picker, ch('h'))));
        assert_eq!(picker.path(), "/");
    }

    /// Blueprint D13: back and forth keeps its place.
    #[test]
    fn going_up_highlights_the_directory_you_came_from() {
        let (mut picker, _) = PathPicker::open("Root", "/srv/notes".to_owned());
        assert!(picker.on_reply(&dirs("/srv/notes", &[], 0)));
        assert_eq!(
            asked(press(&mut picker, ch('h'))),
            ("/srv".to_owned(), false)
        );
        assert!(picker.on_reply(&dirs(
            "/srv",
            &[("htui", false), ("notes", false), ("shared", true)],
            0
        )));
        assert_eq!(chosen(press(&mut picker, ch('s'))), "/srv/notes");
    }

    /// MOD-49 review N-1: the place going up remembers is for the parent it asked for only. A
    /// newer request elsewhere forgets it, so a listing of an unrelated directory that happens to
    /// hold the same name doesn't open on it.
    #[test]
    fn a_newer_request_forgets_where_going_up_came_from() {
        let (mut picker, _) = PathPicker::open("Root", "/srv/notes".to_owned());
        assert!(picker.on_reply(&dirs("/srv/notes", &[], 0)));
        assert_eq!(
            asked(press(&mut picker, ch('h'))),
            ("/srv".to_owned(), false)
        );
        picker.on_paste("/opt");
        assert_eq!(
            asked(press(&mut picker, key(KeyCode::Enter))),
            ("/opt".to_owned(), false)
        );
        // `/srv`'s listing was superseded; `/opt` lands holding a `notes` of its own.
        assert!(picker.on_reply(&dirs("/opt", &[("data", false), ("notes", false)], 0)));
        assert_eq!(chosen(press(&mut picker, ch('s'))), "/opt/data");
    }

    #[test]
    fn s_chooses_the_highlighted_directory() {
        let mut picker = at_srv();
        press(&mut picker, ch('j'));
        assert_eq!(chosen(press(&mut picker, ch('s'))), "/srv/notes");
    }

    #[test]
    fn capital_s_chooses_the_listed_directory() {
        let mut picker = at_srv();
        press(&mut picker, ch('j'));
        assert_eq!(chosen(press(&mut picker, ch('S'))), "/srv");
    }

    /// Blueprint D11: nothing is chosen from a listing that hasn't landed or was refused.
    #[test]
    fn s_and_capital_s_need_a_listing() {
        let (mut picker, _) = PathPicker::open("Root", "/srv".to_owned());
        assert!(stays(&press(&mut picker, ch('s'))));
        assert!(stays(&press(&mut picker, ch('S'))));
        assert!(picker.on_reply(&StoreReply::Failed {
            request: LIST_DIR,
            message: "`/srv` does not exist on this box".to_owned(),
        }));
        assert!(stays(&press(&mut picker, ch('s'))));
        assert!(stays(&press(&mut picker, ch('S'))));

        // An empty listing has a directory to choose and no entry to open.
        let (mut picker, _) = PathPicker::open("Root", "/empty".to_owned());
        assert!(picker.on_reply(&dirs("/empty", &[], 0)));
        assert!(stays(&press(&mut picker, ch('s'))));
        assert!(stays(&press(&mut picker, ch('l'))));
        assert_eq!(chosen(press(&mut picker, ch('S'))), "/empty");
    }

    /// P6, blueprint D12: go-to opens holding `/`, so the typed rest is absolute.
    #[test]
    fn slash_opens_go_to_and_enter_lists_what_was_typed() {
        let mut picker = at_srv();
        assert!(stays(&press(&mut picker, ch('/'))));
        assert_eq!(goto_value(&render(&picker)).as_deref(), Some("/"));
        for c in "srv/htui".chars() {
            // Bound letters type into go-to instead of acting (`h`, `s`, `j` among them).
            assert!(stays(&press(&mut picker, ch(c))));
        }
        assert_eq!(
            asked(press(&mut picker, key(KeyCode::Enter))),
            ("/srv/htui".to_owned(), false)
        );
        assert_eq!(goto_value(&render(&picker)), None, "a submit closes go-to");
    }

    #[test]
    fn a_paste_with_go_to_closed_opens_it() {
        let mut picker = at_srv();
        picker.on_paste("/srv/htui\n");
        assert_eq!(
            asked(press(&mut picker, key(KeyCode::Enter))),
            ("/srv/htui".to_owned(), false)
        );
    }

    #[test]
    fn a_paste_into_a_lone_slash_replaces_it() {
        let mut picker = at_srv();
        press(&mut picker, ch('/'));
        picker.on_paste("/opt/data");
        assert_eq!(
            asked(press(&mut picker, key(KeyCode::Enter))),
            ("/opt/data".to_owned(), false)
        );

        // Anywhere else the paste is inserted at the cursor.
        let mut picker = at_srv();
        press(&mut picker, ch('/'));
        press(&mut picker, ch('o'));
        picker.on_paste("pt");
        assert_eq!(
            asked(press(&mut picker, key(KeyCode::Enter))),
            ("/opt".to_owned(), false)
        );
    }

    #[test]
    fn esc_in_go_to_closes_only_go_to() {
        let mut picker = at_srv();
        press(&mut picker, ch('/'));
        assert!(stays(&press(&mut picker, key(KeyCode::Esc))));
        assert_eq!(goto_value(&render(&picker)), None);
        assert!(matches!(
            press(&mut picker, key(KeyCode::Esc)),
            PickerOutcome::Cancelled
        ));
    }

    /// MOD-49 review L-1 (`R-BOX-4`, P5): `..` in a typed path is resolved lexically before it
    /// is asked for. The kernel resolves `..` after following a link, so `/srv/shared/..` with
    /// `shared` a link would list the link target's parent and show the target among its siblings.
    #[test]
    fn go_to_resolves_dot_dot_lexically() {
        for (typed, asked_for) in [
            ("srv/shared/..", "/srv"),
            ("srv/./shared/../notes", "/srv/notes"),
            ("../..", "/"),
            ("srv/shared/../", "/srv"),
        ] {
            let mut picker = at_srv();
            press(&mut picker, ch('/'));
            for c in typed.chars() {
                press(&mut picker, ch(c));
            }
            assert_eq!(
                asked(press(&mut picker, key(KeyCode::Enter))),
                (asked_for.to_owned(), false),
                "`/{typed}`"
            );
        }
    }

    /// A path with no `.` or `..` component is asked for exactly as typed, trailing slash and all:
    /// the header shows what was typed, and the write's guard is what canonicalises.
    #[test]
    fn go_to_keeps_a_plain_path_as_typed() {
        let mut picker = at_srv();
        picker.on_paste("/srv/htui/");
        assert_eq!(
            asked(press(&mut picker, key(KeyCode::Enter))),
            ("/srv/htui/".to_owned(), false)
        );
    }

    /// An empty go-to submit closes go-to and asks for nothing.
    #[test]
    fn an_empty_go_to_asks_for_nothing() {
        let mut picker = at_srv();
        press(&mut picker, ch('/'));
        press(&mut picker, key(KeyCode::Backspace));
        assert!(stays(&press(&mut picker, key(KeyCode::Enter))));
        assert_eq!(goto_value(&render(&picker)), None);
    }

    /// Blueprint D1: `.` re-asks the worker, which filters before the cap.
    #[test]
    fn dot_relists_with_hidden_shown() {
        let mut picker = at_srv();
        assert_eq!(
            asked(press(&mut picker, ch('.'))),
            ("/srv".to_owned(), true)
        );
        assert!(picker.on_reply(&dirs("/srv", &[(".git", false), ("htui", false)], 0)));
        assert_eq!(
            asked(press(&mut picker, ch('l'))),
            ("/srv/.git".to_owned(), true)
        );
        assert_eq!(
            asked(press(&mut picker, ch('.'))),
            ("/srv".to_owned(), false)
        );
    }

    #[test]
    fn esc_cancels() {
        let mut picker = at_srv();
        assert!(matches!(
            press(&mut picker, key(KeyCode::Esc)),
            PickerOutcome::Cancelled
        ));
    }

    /// P7: a refused listing shows its sentence in the popup, and `h` still climbs out.
    #[test]
    fn a_refused_listing_shows_its_message_and_h_still_works() {
        let (mut picker, _) = PathPicker::open("Root", "/srv/gone".to_owned());
        assert!(picker.on_reply(&StoreReply::Failed {
            request: LIST_DIR,
            message: "constraint violated: `/srv/gone` does not exist on this box".to_owned(),
        }));
        assert!(render(&picker).contains("`/srv/gone` does not exist on this box"));
        assert_eq!(picker.path(), "/srv/gone");
        assert_eq!(
            asked(press(&mut picker, ch('h'))),
            ("/srv".to_owned(), false)
        );
    }

    /// MOD-49 review L-2: on 80 columns (an inner width of 74) the hint splits onto two lines
    /// instead of losing its tail, and a refusal naming a long path wraps instead of running off
    /// the edge.
    #[test]
    fn a_narrow_popup_wraps_the_refusal_and_splits_the_hint() {
        let path =
            "/srv/a-rather-long-directory-name/with-another-long-component/and-one-more-level";
        let message = format!("constraint violated: `{path}` does not exist on this box");
        let (mut picker, _) = PathPicker::open("Root", path.to_owned());
        assert!(picker.on_reply(&StoreReply::Failed {
            request: LIST_DIR,
            message: message.clone(),
        }));

        let frame = render_at(&picker, 80, 24);
        assert!(frame.contains("S this dir"), "{frame}");
        assert!(frame.contains(". hidden \u{b7} Esc cancel"), "{frame}");
        let flat: String = frame
            .chars()
            .filter(|c| !c.is_whitespace() && *c != '\u{2502}')
            .collect();
        assert!(
            flat.contains(&message.replace(' ', "")),
            "the whole refusal is on screen: {frame}"
        );
    }

    /// A refusal of another request isn't the picker's.
    #[test]
    fn another_requests_refusal_is_not_the_pickers() {
        let (mut picker, _) = PathPicker::open("Root", "/srv".to_owned());
        assert!(!picker.on_reply(&StoreReply::Failed {
            request: "set_repo_path",
            message: "nope".to_owned(),
        }));
        assert!(!render(&picker).contains("nope"));
    }

    /// Blueprint D15 (H-4): `q` can't quit and `?` can't open help from inside the popup. No
    /// picker act takes them, so the picker answers `None` and the owner swallows them (MOD-67 M6).
    #[test]
    fn unbound_keys_are_swallowed() {
        let mut picker = at_srv();
        for c in ['q', '?', '1'] {
            assert!(
                picker
                    .on_key(
                        ch(c),
                        KeyChord::from_event(ch(c)),
                        Keys::compiled(),
                        views::HIERARCHY_PICKER,
                    )
                    .is_none(),
                "{c}"
            );
        }
        assert_eq!(chosen(press(&mut picker, ch('s'))), "/srv/htui");
    }

    /// MOD-67 M6 D3: a chord matches with its modifiers, so `ctrl-j`, `alt-l`, `ctrl-s` and a
    /// SUPER `j` are no picker key: the picker answers `None` and neither moves nor chooses.
    #[test]
    fn a_chord_is_no_picker_key() {
        let mut picker = at_srv();
        for key in [
            chord('j', KeyModifiers::CONTROL),
            chord('l', KeyModifiers::ALT),
            chord('s', KeyModifiers::CONTROL),
            chord('j', KeyModifiers::SUPER),
        ] {
            assert!(
                picker
                    .on_key(
                        key,
                        KeyChord::from_event(key),
                        Keys::compiled(),
                        views::HIERARCHY_PICKER,
                    )
                    .is_none(),
                "{key:?}"
            );
        }
        assert_eq!(picker.path(), "/srv");
        assert_eq!(
            chosen(press(&mut picker, ch('s'))),
            "/srv/htui",
            "the cursor did not move"
        );
    }

    /// MOD-67 M6 D3: `[picker] open = "o"` opens on `o`, `Enter` is no picker key any more, and
    /// the popup's hint names `o`.
    #[test]
    fn a_rebound_open_key_opens_and_enter_is_inert() {
        let keys = picker_keys("open = \"o\"\n");
        let mut picker = at_srv();
        let opened = picker.on_key(
            ch('o'),
            KeyChord::from_event(ch('o')),
            &keys,
            views::HIERARCHY_PICKER,
        );
        assert_eq!(
            asked(opened.expect("`o` is the picker's")),
            ("/srv/htui".to_owned(), false)
        );

        let mut picker = at_srv();
        assert!(
            picker
                .on_key(
                    key(KeyCode::Enter),
                    KeyChord::from_event(key(KeyCode::Enter)),
                    &keys,
                    views::HIERARCHY_PICKER,
                )
                .is_none()
        );
        let frame = render_with(&picker, &keys, 100, 30);
        assert!(
            frame.contains("j/k move \u{b7} o open \u{b7} h up"),
            "{frame}"
        );
    }

    /// The go-to field's keys are the field's (ANA-26 §6.1): every key lands in it or edits it,
    /// and `Esc` closes only the field.
    #[test]
    fn the_go_to_field_takes_every_key() {
        let mut picker = at_srv();
        assert!(stays(&press(&mut picker, ch('/'))));
        for key in [ch('q'), ch('j'), chord('a', KeyModifiers::CONTROL)] {
            let outcome = picker.on_key(
                key,
                KeyChord::from_event(key),
                Keys::compiled(),
                views::HIERARCHY_PICKER,
            );
            assert!(
                matches!(outcome, Some(PickerOutcome::Stay)),
                "{key:?}: {outcome:?}"
            );
        }
        assert_eq!(
            goto_value(&render(&picker)).as_deref(),
            Some("/qj"),
            "{}",
            render(&picker)
        );
        let closed = picker.on_key(
            key(KeyCode::Esc),
            KeyChord::from_event(key(KeyCode::Esc)),
            Keys::compiled(),
            views::HIERARCHY_PICKER,
        );
        assert!(matches!(closed, Some(PickerOutcome::Stay)), "{closed:?}");
        assert_eq!(goto_value(&render(&picker)), None);
        assert_eq!(
            picker.path(),
            "/srv",
            "the picker is still open on its listing"
        );
    }

    /// MOD-67 M6 D3: the hint is rendered from its spec through the picker stack, and with the
    /// defaults it is MOD-49's text byte for byte: one line at 100 columns, two at 80.
    #[test]
    fn the_hint_is_the_rendered_spec() {
        let first = "j/k move \u{b7} Enter open \u{b7} h up \u{b7} s choose \u{b7} S this dir";
        let second = "/ go to \u{b7} . hidden \u{b7} Esc cancel";
        let hint_rows = |frame: &str| -> Vec<String> {
            frame
                .lines()
                .map(|row| row.trim_matches(|c: char| c == '\u{2502}' || c.is_whitespace()))
                .filter(|row| row.contains("move") || row.contains("go to"))
                .map(str::to_owned)
                .collect()
        };
        let picker = at_srv();
        assert_eq!(
            hint_rows(&render(&picker)),
            [format!("{first} \u{b7} {second}")]
        );
        assert_eq!(
            hint_rows(&render_at(&picker, 80, 24)),
            [first.to_owned(), second.to_owned()]
        );
    }

    /// MOD-67 M6 D4: the empty listing names the hidden key in force, and none when it is unbound.
    #[test]
    fn the_empty_listing_names_the_hidden_key() {
        let (mut picker, _) = PathPicker::open("Root", "/empty".to_owned());
        assert!(picker.on_reply(&dirs("/empty", &[], 0)));

        let rebound = render_with(&picker, &picker_keys("hidden = \"H\"\n"), 100, 30);
        assert!(
            rebound.contains("no directories here \u{b7} H shows hidden"),
            "{rebound}"
        );
        let unbound = render_with(&picker, &picker_keys("hidden = []\n"), 100, 30);
        assert!(unbound.contains("no directories here"), "{unbound}");
        assert!(!unbound.contains("shows hidden"), "{unbound}");
    }

    /// Blueprint D14: `name/` for a directory, `name@` for a link, `+N more`, the hint.
    #[test]
    fn the_popup_marks_links_and_counts_the_rest() {
        let (mut picker, _) = PathPicker::open("Root of `graphics` on this box", "/srv".to_owned());
        assert!(picker.on_reply(&dirs(
            "/srv",
            &[("htui", false), ("notes", false), ("shared", true)],
            3
        )));
        let frame = render(&picker);
        assert!(frame.contains("Root of `graphics` on this box"), "{frame}");
        assert!(frame.contains("/srv"), "{frame}");
        assert!(frame.contains("\u{258c}htui/"), "{frame}");
        assert!(frame.contains("\u{2502} notes/"), "{frame}");
        assert!(frame.contains("\u{2502} shared@"), "{frame}");
        assert!(frame.contains("+3 more"), "{frame}");
        assert!(frame.contains("j/k move"), "{frame}");
        assert!(frame.contains("Esc cancel"), "{frame}");
    }

    /// The cursor stays on screen past the window, which scrolls without state.
    #[test]
    fn a_long_listing_scrolls_to_the_cursor() {
        let names: Vec<String> = (0..60).map(|n| format!("d{n:02}")).collect();
        let entries: Vec<(&str, bool)> = names.iter().map(|n| (n.as_str(), false)).collect();
        let (mut picker, _) = PathPicker::open("Root", "/many".to_owned());
        assert!(picker.on_reply(&dirs("/many", &entries, 0)));
        for _ in 0..50 {
            press(&mut picker, ch('j'));
        }
        let frame = render(&picker);
        assert!(frame.contains("\u{258c}d50/"), "{frame}");
        assert!(!frame.contains("d00/"), "{frame}");
    }

    /// The workspace switcher's rule: "still reading" and "nothing there" are different screens,
    /// and a refusal is a third.
    #[test]
    fn reading_and_empty_and_refused_are_three_texts() {
        let (mut picker, _) = PathPicker::open("Root", "/srv".to_owned());
        let reading = render(&picker);
        assert!(reading.contains("reading"), "{reading}");

        assert!(picker.on_reply(&dirs("/srv", &[], 0)));
        let empty = render(&picker);
        assert!(empty.contains("no directories here"), "{empty}");
        assert!(!empty.contains("reading"), "{empty}");

        press(&mut picker, ch('l'));
        let (_, _) = asked(press(&mut picker, ch('h')));
        assert!(picker.on_reply(&StoreReply::Failed {
            request: LIST_DIR,
            message: "`/` cannot be read on this box".to_owned(),
        }));
        let refused = render(&picker);
        assert!(refused.contains("cannot be read on this box"), "{refused}");
        assert!(!refused.contains("no directories here"), "{refused}");
    }

    /// MOD-115 M3 MD-A: the picker's acts resolve from the chord the user pressed, which the owner
    /// hands it ([`crate::app::Ctx::pressed`]), not from the delivered event: a `[field]` row's
    /// chord the shell translated to a canonical key moves nothing it was not bound to.
    #[test]
    fn the_acts_resolve_from_the_pressed_chord() {
        let mut picker = at_srv();
        let moved = picker.on_key(
            key(KeyCode::Down),
            KeyChord::from_event(ch('j')),
            Keys::compiled(),
            views::HIERARCHY_PICKER,
        );
        assert!(matches!(moved, Some(PickerOutcome::Stay)), "{moved:?}");
        assert_eq!(
            chosen(press(&mut picker, ch('s'))),
            "/srv/notes",
            "`j` moved"
        );

        let mut picker = at_srv();
        let inert = picker.on_key(
            key(KeyCode::Down),
            KeyChord::from_event(chord('b', KeyModifiers::CONTROL)),
            Keys::compiled(),
            views::HIERARCHY_PICKER,
        );
        assert!(inert.is_none(), "`ctrl-b` is no picker key: {inert:?}");
        assert_eq!(chosen(press(&mut picker, ch('s'))), "/srv/htui");
    }

    /// MOD-115 M3, MOD-85: the popup is the kit's modal, rounded and titled, and its cursor is the
    /// list's gutter and band; the path header is in the title role, never the accent.
    #[test]
    fn the_popup_is_a_kit_modal_with_the_cursor_in_the_band() {
        let picker = at_srv();
        let theme = Theme::default();
        let mut term = Terminal::new(TestBackend::new(60, 12)).expect("a test backend");
        term.draw(|frame| {
            picker.render(
                frame,
                frame.area(),
                &theme,
                Keys::compiled(),
                views::HIERARCHY_PICKER,
            );
        })
        .expect("the picker draws");
        let buffer = term.backend().buffer();
        let row = |y: u16| -> String { (0..60).map(|x| buffer[(x, y)].symbol()).collect() };
        assert!(row(1).starts_with("  \u{256d} Root "), "{}", row(1));
        assert!(row(10).starts_with("  \u{2570}"), "{}", row(10));
        // Content starts at (3, 2): the header, then the entries behind their gutter.
        assert!(row(2).contains("\u{2502}/srv"), "{}", row(2));
        assert!(buffer[(3, 2)].modifier.contains(Modifier::BOLD));
        assert_ne!(buffer[(3, 2)].fg, theme.accent.fg.unwrap_or_default());
        assert_eq!(buffer[(3, 3)].symbol(), "\u{258c}");
        assert_eq!(buffer[(3, 3)].fg, theme.accent.fg.unwrap_or_default());
        assert_eq!(buffer[(4, 3)].bg, theme.selected.bg.unwrap_or_default());
        assert!(row(3).contains("htui/"), "{}", row(3));
        assert_eq!(buffer[(3, 4)].symbol(), " ", "no marker off the cursor");
        assert_ne!(buffer[(4, 4)].bg, theme.selected.bg.unwrap_or_default());
    }
}
