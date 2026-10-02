//! A directory picker a section owns as a mode (MOD-49 P2): not a registered overlay, because an
//! overlay factory takes no start path and has no way to hand a chosen path back.
//!
//! It holds no store handle and no channel: listings arrive through [`PathPicker::on_reply`], and
//! requests leave as [`PickerOutcome::Request`] for the owning section to send (`R-NF-3`).
//! Navigation is lexical and never canonicalises, so nothing on screen names a link's target (P5,
//! `R-BOX-4`): the one canonicalisation is the write the section sends on [`PickerOutcome::Chosen`].

use std::path::{Component, Path, PathBuf};

use crossterm::event::{KeyCode, KeyEvent};
use htui_core::root_path::DirListing;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::store_worker::{LIST_DIR, StoreReply, StoreRequest};
use crate::ui::cells::{cell_width, graphemes};
use crate::ui::layout::centered;
use crate::ui::{FieldOutcome, TextField, Theme};

/// Marker in front of the highlighted entry, visible in a snapshot (the workspace switcher's).
const CURSOR: &str = "> ";

/// The marker's width, in front of every other line, so names stay in one column.
const NO_CURSOR: &str = "  ";

/// The first half of the hint under the entries: the keys that move and choose (P8). The two
/// halves share one line when it fits and take one line each when it doesn't (review L-2).
const HINT_MOVE: &str = "j/k move \u{b7} Enter open \u{b7} h up \u{b7} s choose \u{b7} S this dir";

/// The hint's second half: the keys that change what is listed, and the way out.
const HINT_MORE: &str = "/ go to \u{b7} . hidden \u{b7} Esc cancel";

/// Between the two halves when they share a line.
const HINT_GAP: &str = " \u{b7} ";

/// The label in front of the go-to field (P6).
const GOTO: &str = "go to: ";

/// The widest the popup gets (blueprint D14): a listing can't be sized to its content.
const MAX_WIDTH: u16 = 96;

/// What one key did (MOD-49 P8).
#[derive(Debug, Clone)]
pub enum PickerOutcome {
    /// Handled here (moved, typed, swallowed): nothing for the section to do.
    None,
    /// A listing to send through `ctx.request` — a read, never marked busy (P11).
    Request(StoreRequest),
    /// The directory the user chose, as navigated (never canonicalised; the write does that).
    Chosen(String),
    /// `Esc` outside go-to: close the picker, write nothing.
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

    /// One key (P8, blueprint D11, D12, D15). The caller passes `CONTROL` chords on before
    /// calling this; every key not bound here is swallowed (`PickerOutcome::None`).
    pub fn on_key(&mut self, key: KeyEvent) -> PickerOutcome {
        if let Some(field) = &mut self.goto {
            return match field.on_key(key) {
                FieldOutcome::Submit => {
                    let typed = field.text().unwrap_or_default().trim().to_owned();
                    self.goto = None;
                    if typed.is_empty() {
                        PickerOutcome::None
                    } else {
                        PickerOutcome::Request(self.ask(typed))
                    }
                }
                FieldOutcome::Cancel => {
                    self.goto = None;
                    PickerOutcome::None
                }
                FieldOutcome::Consumed | FieldOutcome::Pass => PickerOutcome::None,
            };
        }
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                let last = self.entry_count().saturating_sub(1);
                self.cursor = (self.cursor + 1).min(last);
                PickerOutcome::None
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.cursor = self.cursor.saturating_sub(1);
                PickerOutcome::None
            }
            KeyCode::Enter | KeyCode::Char('l') => match self.highlighted() {
                Some(next) => PickerOutcome::Request(self.ask(next)),
                None => PickerOutcome::None,
            },
            KeyCode::Char('h') | KeyCode::Backspace => match parent(&self.path) {
                Some(up) => {
                    self.reselect = Path::new(&self.path)
                        .file_name()
                        .and_then(|name| name.to_str())
                        .map(str::to_owned);
                    PickerOutcome::Request(self.ask(up))
                }
                None => PickerOutcome::None,
            },
            KeyCode::Char('s') => self
                .highlighted()
                .map_or(PickerOutcome::None, PickerOutcome::Chosen),
            KeyCode::Char('S') if self.listing.is_some() => {
                PickerOutcome::Chosen(self.path.clone())
            }
            KeyCode::Char('/') => {
                self.goto = Some(TextField::with_text("/"));
                PickerOutcome::None
            }
            KeyCode::Char('.') => {
                self.show_hidden = !self.show_hidden;
                PickerOutcome::Request(self.ask(self.path.clone()))
            }
            KeyCode::Esc => PickerOutcome::Cancelled,
            // Swallowed (D15): `q` must not quit and `?` must not open help mid-pick.
            _ => PickerOutcome::None,
        }
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

    /// The popup, centred over `area` (blueprint D14): a fixed box, because a listing of up to
    /// `LIST_CAP` entries can't be sized to its content.
    pub fn render(&self, frame: &mut Frame<'_>, area: Rect, theme: &Theme) {
        let box_area = centered(
            area,
            area.width.saturating_sub(4).min(MAX_WIDTH),
            area.height.saturating_sub(2),
        );
        let lines = self.lines(
            box_area.width.saturating_sub(2),
            usize::from(box_area.height.saturating_sub(2)),
            theme,
        );
        frame.render_widget(Clear, box_area);
        frame.render_widget(
            Paragraph::new(Text::from(lines)).block(
                Block::new()
                    .borders(Borders::ALL)
                    .title(Span::styled(format!(" {} ", self.title), theme.title)),
            ),
            box_area,
        );
    }

    /// `ask(p)`: remember `p` as the newest path asked for (P9) and build its request. `.` and
    /// `..` are resolved lexically first (review L-1), so the kernel never resolves a `..` after a
    /// link and lists the link target's parent (P5, `R-BOX-4`).
    fn ask(&mut self, path: String) -> StoreRequest {
        let path = lexical(path);
        self.asked.clone_from(&path);
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

    /// The box's contents for an inner area of `width` × `height`: the header, the entries
    /// window (or one of the three empty texts), go-to while open, a blank line and the hint.
    fn lines(&self, width: u16, height: usize, theme: &Theme) -> Vec<Line<'static>> {
        let mut footer = Vec::new();
        if let Some(field) = &self.goto {
            let room = width.saturating_sub(u16::try_from(GOTO.len()).unwrap_or(u16::MAX));
            let mut spans = vec![Span::styled(GOTO, theme.base)];
            spans.extend(field.line(room, true, theme).spans);
            footer.push(Line::from(spans));
        }
        footer.push(Line::raw(""));
        let one_line = format!("{NO_CURSOR}{HINT_MOVE}{HINT_GAP}{HINT_MORE}");
        if cell_width(&one_line) <= usize::from(width) {
            footer.push(Line::styled(one_line, theme.dim));
        } else {
            footer.push(Line::styled(format!("{NO_CURSOR}{HINT_MOVE}"), theme.dim));
            footer.push(Line::styled(format!("{NO_CURSOR}{HINT_MORE}"), theme.dim));
        }

        let rows = height.saturating_sub(1 + footer.len());
        let mut lines = vec![Line::styled(
            clip_left(&self.path, usize::from(width)),
            theme.accent,
        )];
        lines.extend(self.body(width, rows, theme));
        lines.extend(footer);
        lines
    }

    /// The entries in a window of `rows` lines that always holds the cursor, then `+N more`; or
    /// the error, "reading…", or the empty text — three different screens (the switcher's rule).
    /// The error is wrapped to `width` (review L-2): it names a path, and a long one would push
    /// the reason off the edge.
    fn body(&self, width: u16, rows: usize, theme: &Theme) -> Vec<Line<'static>> {
        if let Some(error) = &self.error {
            let room = usize::from(width).saturating_sub(cell_width(NO_CURSOR));
            return wrap(error, room)
                .into_iter()
                .map(|line| Line::styled(format!("{NO_CURSOR}{line}"), theme.error))
                .collect();
        }
        let Some(listing) = &self.listing else {
            return vec![Line::styled(
                format!("{NO_CURSOR}reading\u{2026}"),
                theme.dim,
            )];
        };
        if listing.entries.is_empty() {
            let hidden = if self.show_hidden {
                ""
            } else {
                " \u{b7} . shows hidden"
            };
            return vec![Line::styled(
                format!("{NO_CURSOR}no directories here{hidden}"),
                theme.dim,
            )];
        }
        let window = rows.saturating_sub(usize::from(listing.more > 0)).max(1);
        // Stateless scroll: the window ends on the cursor once it passes the bottom, so `render`
        // stays `&self` and the cursor is always on screen.
        let offset = self.cursor.saturating_sub(window - 1);
        let mut lines: Vec<Line<'static>> = listing
            .entries
            .iter()
            .enumerate()
            .skip(offset)
            .take(window)
            .map(|(index, entry)| {
                let selected = index == self.cursor;
                let marker = if selected { CURSOR } else { NO_CURSOR };
                // `ls -F` marks: a link is `@` and never shows where it points (`R-BOX-4`).
                let kind = if entry.is_link { '@' } else { '/' };
                let style = if selected { theme.accent } else { theme.base };
                Line::styled(format!("{marker}{}{kind}", entry.name), style)
            })
            .collect();
        if listing.more > 0 {
            lines.push(Line::styled(
                format!("{NO_CURSOR}+{} more", listing.more),
                theme.dim,
            ));
        }
        lines
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
    use crate::store_worker::{LIST_DIR, StoreReply, StoreRequest};
    use crate::ui::Theme;
    use crate::ui::cells::cell_width;
    use crossterm::event::{KeyCode, KeyEvent};
    use htui_core::root_path::{DirEntry, DirListing};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    /// A key with no modifiers.
    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::from(code)
    }

    /// A plain character key.
    fn ch(c: char) -> KeyEvent {
        key(KeyCode::Char(c))
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

    /// Whether an outcome is `None`.
    fn none(outcome: &PickerOutcome) -> bool {
        matches!(outcome, PickerOutcome::None)
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
        let mut term = Terminal::new(TestBackend::new(width, height)).expect("a test backend");
        term.draw(|frame| picker.render(frame, frame.area(), &Theme::default()))
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
            none(&picker.on_key(ch('s'))),
            "nothing landed to choose from"
        );
    }

    #[test]
    fn j_and_k_clamp_at_both_ends() {
        let mut picker = at_srv();
        assert!(none(&picker.on_key(ch('k'))));
        assert_eq!(chosen(picker.on_key(ch('s'))), "/srv/htui");
        for _ in 0..5 {
            picker.on_key(ch('j'));
        }
        assert_eq!(chosen(picker.on_key(ch('s'))), "/srv/shared");
        picker.on_key(key(KeyCode::Up));
        assert_eq!(chosen(picker.on_key(ch('s'))), "/srv/notes");
        picker.on_key(key(KeyCode::Down));
        assert_eq!(chosen(picker.on_key(ch('s'))), "/srv/shared");
    }

    #[test]
    fn enter_and_l_open_the_highlighted_directory() {
        let mut picker = at_srv();
        assert_eq!(
            asked(picker.on_key(key(KeyCode::Enter))),
            ("/srv/htui".to_owned(), false)
        );
        let mut picker = at_srv();
        assert_eq!(
            asked(picker.on_key(ch('l'))),
            ("/srv/htui".to_owned(), false)
        );
    }

    /// P5: up from a link is the link's lexical parent, never its target's.
    #[test]
    fn h_and_backspace_go_to_the_lexical_parent() {
        for up in [ch('h'), key(KeyCode::Backspace)] {
            let mut picker = at_srv();
            picker.on_key(ch('j'));
            picker.on_key(ch('j'));
            let (shared, _) = asked(picker.on_key(ch('l')));
            assert_eq!(shared, "/srv/shared");
            assert!(picker.on_reply(&dirs("/srv/shared", &[("inside", false)], 0)));
            assert_eq!(picker.path(), "/srv/shared");
            assert_eq!(asked(picker.on_key(up)), ("/srv".to_owned(), false));
        }
    }

    #[test]
    fn h_at_the_root_stays_at_the_root() {
        let (mut picker, _) = PathPicker::open("Root", "/".to_owned());
        assert!(picker.on_reply(&dirs("/", &[("srv", false)], 0)));
        assert!(none(&picker.on_key(ch('h'))));
        assert_eq!(picker.path(), "/");
    }

    /// Blueprint D13: back and forth keeps its place.
    #[test]
    fn going_up_highlights_the_directory_you_came_from() {
        let (mut picker, _) = PathPicker::open("Root", "/srv/notes".to_owned());
        assert!(picker.on_reply(&dirs("/srv/notes", &[], 0)));
        assert_eq!(asked(picker.on_key(ch('h'))), ("/srv".to_owned(), false));
        assert!(picker.on_reply(&dirs(
            "/srv",
            &[("htui", false), ("notes", false), ("shared", true)],
            0
        )));
        assert_eq!(chosen(picker.on_key(ch('s'))), "/srv/notes");
    }

    #[test]
    fn s_chooses_the_highlighted_directory() {
        let mut picker = at_srv();
        picker.on_key(ch('j'));
        assert_eq!(chosen(picker.on_key(ch('s'))), "/srv/notes");
    }

    #[test]
    fn capital_s_chooses_the_listed_directory() {
        let mut picker = at_srv();
        picker.on_key(ch('j'));
        assert_eq!(chosen(picker.on_key(ch('S'))), "/srv");
    }

    /// Blueprint D11: nothing is chosen from a listing that hasn't landed or was refused.
    #[test]
    fn s_and_capital_s_need_a_listing() {
        let (mut picker, _) = PathPicker::open("Root", "/srv".to_owned());
        assert!(none(&picker.on_key(ch('s'))));
        assert!(none(&picker.on_key(ch('S'))));
        assert!(picker.on_reply(&StoreReply::Failed {
            request: LIST_DIR,
            message: "`/srv` does not exist on this box".to_owned(),
        }));
        assert!(none(&picker.on_key(ch('s'))));
        assert!(none(&picker.on_key(ch('S'))));

        // An empty listing has a directory to choose and no entry to open.
        let (mut picker, _) = PathPicker::open("Root", "/empty".to_owned());
        assert!(picker.on_reply(&dirs("/empty", &[], 0)));
        assert!(none(&picker.on_key(ch('s'))));
        assert!(none(&picker.on_key(ch('l'))));
        assert_eq!(chosen(picker.on_key(ch('S'))), "/empty");
    }

    /// P6, blueprint D12: go-to opens holding `/`, so the typed rest is absolute.
    #[test]
    fn slash_opens_go_to_and_enter_lists_what_was_typed() {
        let mut picker = at_srv();
        assert!(none(&picker.on_key(ch('/'))));
        assert!(render(&picker).contains("go to: /"));
        for c in "srv/htui".chars() {
            // Bound letters type into go-to instead of acting (`h`, `s`, `j` among them).
            assert!(none(&picker.on_key(ch(c))));
        }
        assert_eq!(
            asked(picker.on_key(key(KeyCode::Enter))),
            ("/srv/htui".to_owned(), false)
        );
        assert!(!render(&picker).contains("go to:"), "a submit closes go-to");
    }

    #[test]
    fn a_paste_with_go_to_closed_opens_it() {
        let mut picker = at_srv();
        picker.on_paste("/srv/htui\n");
        assert_eq!(
            asked(picker.on_key(key(KeyCode::Enter))),
            ("/srv/htui".to_owned(), false)
        );
    }

    #[test]
    fn a_paste_into_a_lone_slash_replaces_it() {
        let mut picker = at_srv();
        picker.on_key(ch('/'));
        picker.on_paste("/opt/data");
        assert_eq!(
            asked(picker.on_key(key(KeyCode::Enter))),
            ("/opt/data".to_owned(), false)
        );

        // Anywhere else the paste is inserted at the cursor.
        let mut picker = at_srv();
        picker.on_key(ch('/'));
        picker.on_key(ch('o'));
        picker.on_paste("pt");
        assert_eq!(
            asked(picker.on_key(key(KeyCode::Enter))),
            ("/opt".to_owned(), false)
        );
    }

    #[test]
    fn esc_in_go_to_closes_only_go_to() {
        let mut picker = at_srv();
        picker.on_key(ch('/'));
        assert!(none(&picker.on_key(key(KeyCode::Esc))));
        assert!(!render(&picker).contains("go to:"));
        assert!(matches!(
            picker.on_key(key(KeyCode::Esc)),
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
            picker.on_key(ch('/'));
            for c in typed.chars() {
                picker.on_key(ch(c));
            }
            assert_eq!(
                asked(picker.on_key(key(KeyCode::Enter))),
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
            asked(picker.on_key(key(KeyCode::Enter))),
            ("/srv/htui/".to_owned(), false)
        );
    }

    /// An empty go-to submit closes go-to and asks for nothing.
    #[test]
    fn an_empty_go_to_asks_for_nothing() {
        let mut picker = at_srv();
        picker.on_key(ch('/'));
        picker.on_key(key(KeyCode::Backspace));
        assert!(none(&picker.on_key(key(KeyCode::Enter))));
        assert!(!render(&picker).contains("go to:"));
    }

    /// Blueprint D1: `.` re-asks the worker, which filters before the cap.
    #[test]
    fn dot_relists_with_hidden_shown() {
        let mut picker = at_srv();
        assert_eq!(asked(picker.on_key(ch('.'))), ("/srv".to_owned(), true));
        assert!(picker.on_reply(&dirs("/srv", &[(".git", false), ("htui", false)], 0)));
        assert_eq!(
            asked(picker.on_key(ch('l'))),
            ("/srv/.git".to_owned(), true)
        );
        assert_eq!(asked(picker.on_key(ch('.'))), ("/srv".to_owned(), false));
    }

    #[test]
    fn esc_cancels() {
        let mut picker = at_srv();
        assert!(matches!(
            picker.on_key(key(KeyCode::Esc)),
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
        assert_eq!(asked(picker.on_key(ch('h'))), ("/srv".to_owned(), false));
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

    /// Blueprint D15 (H-4): `q` can't quit and `?` can't open help from inside the popup.
    #[test]
    fn unbound_keys_are_swallowed() {
        let mut picker = at_srv();
        for c in ['q', '?', '1'] {
            assert!(none(&picker.on_key(ch(c))));
        }
        assert_eq!(chosen(picker.on_key(ch('s'))), "/srv/htui");
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
        assert!(frame.contains("> htui/"), "{frame}");
        assert!(frame.contains("  notes/"), "{frame}");
        assert!(frame.contains("  shared@"), "{frame}");
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
            picker.on_key(ch('j'));
        }
        let frame = render(&picker);
        assert!(frame.contains("> d50/"), "{frame}");
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

        picker.on_key(ch('l'));
        let (_, _) = asked(picker.on_key(ch('h')));
        assert!(picker.on_reply(&StoreReply::Failed {
            request: LIST_DIR,
            message: "`/` cannot be read on this box".to_owned(),
        }));
        let refused = render(&picker);
        assert!(refused.contains("cannot be read on this box"), "{refused}");
        assert!(!refused.contains("no directories here"), "{refused}");
    }
}
