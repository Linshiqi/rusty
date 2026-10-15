//! The keys: what a press runs, a chord half pressed, and the Keyboard
//! Shortcuts page, where every action's key is listed and rebound.
//!
//! - **One keymap answers everything that shows or runs a key** — the
//!   window's own handler, the palette, the editor's menu, the debugger's
//!   tooltips, the welcome page's line — so a rebound key is the key shown,
//!   and nothing names a key that stopped working.
//! - **A rebinding is `workbench.toml`'s `keybinds`**, the changed keys
//!   only, written at once through `config::update`, whose lock keeps a
//!   second window's writes out of this one's. The table's key set back is
//!   no rebinding, and the entry goes.
//! - **Recorded as VS Code records it**: a click on a row's key, the
//!   strokes pressed — two at most — then Enter to keep them or Escape to
//!   leave the key as it was. A key that types, or Alt with a character —
//!   which AltGr types with — is refused on the row, not stored.
//! - **What a key collides with is said on its row**: another action on the
//!   same key, a key whose first stroke is another's whole key (it could
//!   never be reached), and, on a rebound key, one the code takes for
//!   itself while it has the keys.

use std::{collections::BTreeMap, rc::Rc};

use rusty_i18n::t;
use slint::{ComponentHandle, ModelRc, VecModel};

use super::{App, Tab};
use crate::{
    KeyRow, Shell,
    actions::{self, Keymap, Refusal},
    keys::{self, Stroke},
};

/// The keys in force, and the page that rebinds them.
#[derive(Default)]
pub(super) struct Keys {
    pub map: Keymap,
    /// The first stroke of a chord, pressed, and the note saying so.
    pending: Option<(String, i32)>,
    /// What the page's search holds.
    query: String,
    /// The search takes keys pressed rather than typed.
    search_keys: bool,
    /// The row being rebound, and what has been pressed for it so far.
    recording: Option<(&'static str, Vec<String>)>,
    /// A key refused at Enter, said on its row until the next.
    refused: Option<(&'static str, Refusal)>,
    /// Counts every change of a key, for what shows one to read again.
    version: i32,
}

impl Keys {
    pub fn new(saved: &BTreeMap<String, String>) -> Keys {
        Keys {
            map: Keymap::read(saved),
            ..Keys::default()
        }
    }
}

impl App {
    /// A key the focused element left: the second stroke of a chord, a
    /// stroke that begins one, or an action's key.
    pub(super) fn window_key(
        self: &Rc<Self>,
        text: &str,
        ctrl: bool,
        alt: bool,
        shift: bool,
        meta: bool,
    ) -> bool {
        self.key_in(None, text, (ctrl, alt, shift, meta))
    }

    /// The same, pressed in `torn` — a window a tab was moved into — or
    /// the main window when it is `None`: the one keymap, an action's run
    /// as one asked for there (`torn_act`), and what a chord waits for said
    /// in that window's own status line.
    pub(super) fn key_in(
        self: &Rc<Self>,
        torn: Option<usize>,
        text: &str,
        (ctrl, alt, shift, meta): (bool, bool, bool, bool),
    ) -> bool {
        let Some(stroke) = keys::chord(text, ctrl, alt, shift, meta) else {
            return false;
        };
        let pending = self.state.borrow_mut().keys.pending.take();
        if let Some((first, note)) = pending {
            self.dismiss_note(note);
            let chord = format!("{first} {stroke}");
            let action = self.state.borrow().keys.map.bound_to(&chord);
            match action {
                Some(action) => {
                    if let Some(id) = torn {
                        self.torn_say(id, String::new());
                    }
                    self.act_in(torn, action.id);
                }
                None => {
                    let said = t!("shortcuts.not-a-command", keys = chord.replace(' ', ", "));
                    self.say_keys(torn, said);
                }
            }
            // Taken either way, as VS Code takes it: the second stroke was
            // pressed as part of a chord, not as a key of its own.
            return true;
        }
        let (starts, action) = {
            let state = self.state.borrow();
            (
                state.keys.map.starts(&stroke),
                state.keys.map.bound_to(&stroke),
            )
        };
        if starts {
            let note = self.say_keys(torn, t!("shortcuts.waiting", keys = stroke.clone()));
            self.state.borrow_mut().keys.pending = Some((stroke, note));
            return true;
        }
        match action {
            Some(action) => {
                self.act_in(torn, action.id);
                true
            }
            None => false,
        }
    }

    /// An action a key ran, in the window it was pressed in.
    fn act_in(self: &Rc<Self>, torn: Option<usize>, id: &str) {
        match torn {
            Some(torn) => self.torn_act(torn, id),
            None => self.act(id),
        }
    }

    /// What a key says — a chord waiting, one that is no command — where it
    /// was pressed: a note in the main window, whose number is returned to
    /// take it away again, or the status line of a window of its own (0,
    /// which no note has).
    fn say_keys(self: &Rc<Self>, torn: Option<usize>, text: String) -> i32 {
        match torn {
            Some(id) => {
                self.torn_say(id, text);
                0
            }
            None => {
                self.note_transient(0, text);
                self.state.borrow().next_note
            }
        }
    }

    /// The code's question before every key: while a chord is half
    /// pressed, the key is the window's.
    pub(super) fn chord_key(
        self: &Rc<Self>,
        text: &str,
        ctrl: bool,
        alt: bool,
        shift: bool,
        meta: bool,
    ) -> bool {
        self.chord_key_in(None, text, (ctrl, alt, shift, meta))
    }

    /// The same, for the code of `torn`, a window a tab was moved into.
    pub(super) fn chord_key_in(
        self: &Rc<Self>,
        torn: Option<usize>,
        text: &str,
        held: (bool, bool, bool, bool),
    ) -> bool {
        let pending = self.state.borrow().keys.pending.is_some();
        pending && self.key_in(torn, text, held)
    }

    /// The key an action has now, as a tooltip ends: ` (F10)`, or nothing.
    pub(super) fn key_suffix(&self, id: &str) -> String {
        self.state
            .try_borrow()
            .ok()
            .and_then(|state| state.keys.map.chord(id).map(|c| format!(" ({c})")))
            .unwrap_or_default()
    }

    pub(super) fn open_shortcuts(self: &Rc<Self>) {
        self.open_tab(Tab::Page("keys"));
        self.sync_keys();
        // Another window may have rebound a key since this one read them.
        self.reread_workbench();
    }

    pub(super) fn keys_query(self: &Rc<Self>, query: &str) {
        self.state.borrow_mut().keys.query = query.to_string();
        self.sync_keys();
    }

    /// The search's keyboard button: keys pressed in the field search by
    /// key, as VS Code's Record Keys does.
    pub(super) fn keys_search_keys(self: &Rc<Self>) {
        {
            let mut state = self.state.borrow_mut();
            state.keys.search_keys = !state.keys.search_keys;
        }
        self.sync_keys();
    }

    /// A key pressed in the search while it takes keys: the query becomes
    /// that key, quoted to match it exactly — and a stroke after one that
    /// begins a chord is the chord's second.
    pub(super) fn keys_search_stroke(
        self: &Rc<Self>,
        text: &str,
        ctrl: bool,
        alt: bool,
        shift: bool,
        meta: bool,
    ) -> bool {
        let Some(stroke) = keys::chord(text, ctrl, alt, shift, meta) else {
            return true;
        };
        {
            let mut state = self.state.borrow_mut();
            let keys = &mut state.keys;
            let previous = keys.query.trim().trim_matches('"').to_string();
            let chord = if !previous.contains(' ') && keys.map.starts(&previous) {
                format!("{previous} {stroke}")
            } else {
                stroke
            };
            keys.query = format!("\"{chord}\"");
        }
        self.sync_keys();
        if let Some(window) = self.window() {
            let query = self.state.borrow().keys.query.clone();
            window.global::<Shell>().set_keys_query(query.into());
        }
        true
    }

    /// A row's key clicked: what is pressed next is its new key.
    pub(super) fn keys_record(self: &Rc<Self>, id: &str) {
        let Some(action) = actions::by_id(id) else {
            return;
        };
        {
            let mut state = self.state.borrow_mut();
            state.keys.recording = Some((action.id, Vec::new()));
            state.keys.refused = None;
        }
        self.sync_keys();
    }

    /// A key pressed while a row records: Enter keeps what was pressed,
    /// Escape leaves the key as it was, anything else is a stroke — and a
    /// third starts the chord again.
    pub(super) fn keys_stroke(
        self: &Rc<Self>,
        text: &str,
        ctrl: bool,
        alt: bool,
        shift: bool,
        meta: bool,
    ) -> bool {
        let Some(stroke) = keys::chord(text, ctrl, alt, shift, meta) else {
            return true;
        };
        match stroke.as_str() {
            "Escape" => self.keys_stop(false),
            "Enter" => self.keys_stop(true),
            _ => {
                if let Some((_, strokes)) = self.state.borrow_mut().keys.recording.as_mut() {
                    if strokes.len() == actions::STROKES {
                        strokes.clear();
                    }
                    strokes.push(stroke);
                }
                self.sync_keys();
            }
        }
        true
    }

    /// The page lost the keys while a row recorded: as Escape.
    pub(super) fn keys_lost(self: &Rc<Self>) {
        if self.state.borrow().keys.recording.is_some() {
            self.keys_stop(false);
        }
    }

    fn keys_stop(self: &Rc<Self>, keep: bool) {
        let Some((id, strokes)) = self.state.borrow_mut().keys.recording.take() else {
            return;
        };
        if keep && !strokes.is_empty() {
            let chord = strokes.join(" ");
            match actions::refusal(&chord) {
                Some(why) => self.state.borrow_mut().keys.refused = Some((id, why)),
                None => {
                    self.state.borrow_mut().keys.map.set(id, chord);
                    self.keys_changed(Some(id));
                }
            }
        }
        self.sync_keys();
        if let Some(window) = self.window() {
            window.invoke_focus_window();
        }
    }

    pub(super) fn keys_reset(self: &Rc<Self>, id: &str) {
        let Some(action) = actions::by_id(id) else {
            return;
        };
        self.state.borrow_mut().keys.map.reset(action.id);
        self.keys_changed(Some(action.id));
        self.sync_keys();
    }

    pub(super) fn keys_reset_all(self: &Rc<Self>) {
        self.state.borrow_mut().keys.map.reset_all();
        self.keys_changed(None);
        self.sync_keys();
    }

    /// A key changed: kept in `workbench.toml` — that action's, or with
    /// `None` every one — and everything that shows a key told.
    fn keys_changed(&self, id: Option<&'static str>) {
        let chord = {
            let mut state = self.state.borrow_mut();
            state.keys.version += 1;
            state.keys.refused = None;
            id.filter(|id| state.keys.map.is_rebound(id))
                .and_then(|id| state.keys.map.chord(id).map(str::to_string))
        };
        let _ = rusty_embed::config::update(move |saved| match (id, chord) {
            (Some(id), Some(chord)) => {
                saved.keybinds.insert(id.to_string(), chord);
            }
            (Some(id), None) => {
                saved.keybinds.remove(id);
            }
            (None, _) => saved.keybinds.clear(),
        });
    }

    /// Keys another window rebound: taken, and drawn.
    pub(super) fn keys_read(&self, saved: &BTreeMap<String, String>) {
        let map = Keymap::read(saved);
        {
            let mut state = self.state.borrow_mut();
            if state.keys.map == map {
                return;
            }
            state.keys.map = map;
            state.keys.version += 1;
        }
        self.sync_keys();
    }

    /// The page's rows, the search, and everything else that shows a key.
    pub(super) fn sync_keys(&self) {
        let Some(window) = self.window() else { return };
        let shell = window.global::<Shell>();
        let state = self.state.borrow();
        let keys = &state.keys;
        let title = |action: &actions::Action| rusty_i18n::lookup(action.title);
        let rows: Vec<KeyRow> = actions::ALL
            .iter()
            .filter(|action| action.listed)
            .filter_map(|action| {
                let recording = keys
                    .recording
                    .as_ref()
                    .filter(|(id, _)| *id == action.id)
                    .map(|(_, strokes)| strokes.join(" "));
                let chord = keys.map.chord(action.id).unwrap_or_default();
                if recording.is_none() && !matches(&keys.query, &title(action), action.id, chord) {
                    return None;
                }
                let (note, warn) = match &recording {
                    Some(pressed) => recording_note(&keys.map, action.id, pressed, title),
                    None => match keys.refused {
                        Some((id, why)) if id == action.id => (refused(why), true),
                        _ => {
                            let note = key_note(&keys.map, action.id, title);
                            let warn = !note.is_empty();
                            (note, warn)
                        }
                    },
                };
                Some(KeyRow {
                    id: action.id.into(),
                    title: title(action).into(),
                    chord: recording.as_deref().unwrap_or(chord).into(),
                    rebound: keys.map.is_rebound(action.id),
                    recording: recording.is_some(),
                    note: note.into(),
                    warn,
                })
            })
            .collect();
        shell.set_keys_rows(ModelRc::new(VecModel::from(rows)));
        shell.set_keys_search_keys(keys.search_keys);
        shell.set_keys_recording(keys.recording.is_some());
        shell.set_keys_rebound(keys.map.any_rebound());
        shell.set_keys_version(keys.version);
        let hint = match (
            keys.map.chord("palette.open"),
            keys.map.chord("finder.open"),
        ) {
            (Some(palette), Some(finder)) => {
                t!("desk.keys-hint", palette = palette, finder = finder)
            }
            _ => String::new(),
        };
        shell.set_keys_hint(hint.into());
    }
}

/// Why a key pressed for a row was not kept.
fn refused(why: Refusal) -> String {
    match why {
        Refusal::Types => t!("shortcuts.refused-types"),
        Refusal::AltGr => t!("shortcuts.refused-altgr"),
    }
}

/// What a row says under its key: the actions it collides with, and on a
/// rebound key one the code takes for itself.
fn key_note(map: &Keymap, id: &str, title: impl Fn(&actions::Action) -> String) -> String {
    let sharing = map.sharing(id);
    if !sharing.is_empty() {
        let names: Vec<String> = sharing.into_iter().map(title).collect();
        return t!("shortcuts.shared", names = names.join(" · "));
    }
    let first = map
        .chord(id)
        .and_then(|chord| chord.split(' ').next())
        .and_then(Stroke::parse);
    match first {
        Some(stroke) if map.is_rebound(id) && crate::editor::takes(&stroke) => {
            t!("shortcuts.code-takes", key = stroke.to_string())
        }
        _ => String::new(),
    }
}

/// What a recording row says, and whether it is trouble: how to finish, or
/// what keeping the keys pressed so far would refuse or collide with.
fn recording_note(
    map: &Keymap,
    id: &'static str,
    pressed: &str,
    title: impl Fn(&actions::Action) -> String,
) -> (String, bool) {
    if pressed.is_empty() {
        return (t!("shortcuts.recording-hint"), false);
    }
    if let Some(why) = actions::refusal(pressed) {
        return (refused(why), true);
    }
    let mut would = map.clone();
    would.set(id, pressed.to_string());
    match key_note(&would, id, title) {
        note if note.is_empty() => (t!("shortcuts.recording-hint"), false),
        note => (note, true),
    }
}

/// Whether a row answers the search: a quoted key matches that key, or a
/// chord it begins; a key written out — `ctrl+s` — matches the chords with
/// that stroke, not every key whose spelling contains it (`Ctrl+Shift+P`);
/// anything else is looked for in the action's name, its id and its key,
/// ignoring case — `save`, `F5`.
fn matches(query: &str, title: &str, id: &str, chord: &str) -> bool {
    let query = query.trim();
    if query.is_empty() {
        return true;
    }
    if let Some(quoted) = query
        .strip_prefix('"')
        .map(|rest| rest.strip_suffix('"').unwrap_or(rest))
    {
        return match actions::normal(quoted) {
            Some(wanted) => chord == wanted || chord.starts_with(&format!("{wanted} ")),
            None => false,
        };
    }
    let lower = query.to_lowercase();
    if title.to_lowercase().contains(&lower) || id.contains(&lower) {
        return true;
    }
    match actions::normal(query).filter(|_| query.contains('+')) {
        Some(wanted) => chord == wanted || chord.split(' ').any(|stroke| stroke == wanted),
        None => chord.to_lowercase().contains(&lower),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_search_finds_a_name_an_id_or_a_key() {
        assert!(matches("", "Save", "file.save", "Ctrl+S"));
        assert!(matches("sav", "Save", "file.save", "Ctrl+S"));
        assert!(matches("ctrl+s", "Save", "file.save", "Ctrl+S"));
        assert!(
            !matches("ctrl+s", "Palette", "palette.open", "Ctrl+Shift+P"),
            "a key written out is that stroke, not a prefix of a spelling"
        );
        assert!(matches(
            "ctrl+s",
            "Keyboard Shortcuts",
            "keys.open",
            "Ctrl+K Ctrl+S"
        ));
        assert!(matches("f5", "Debug", "run.debug", "F5"));
        assert!(matches("file.", "Save", "file.save", "Ctrl+S"));
        assert!(matches("保存", "保存", "file.save", "Ctrl+S"));
        assert!(!matches("build", "Save", "file.save", "Ctrl+S"));
    }

    /// A key pressed into the search is that key exactly: `Ctrl+S` is not
    /// `Ctrl+Shift+S`, and a chord's first stroke finds the chord.
    #[test]
    fn a_quoted_key_is_that_key() {
        assert!(matches("\"Ctrl+S\"", "Save", "file.save", "Ctrl+S"));
        assert!(!matches(
            "\"Ctrl+S\"",
            "Save as",
            "file.save-as",
            "Ctrl+Shift+S"
        ));
        assert!(matches(
            "\"ctrl+k\"",
            "Keyboard Shortcuts",
            "keys.open",
            "Ctrl+K Ctrl+S"
        ));
        assert!(matches(
            "\"Ctrl+K Ctrl+S\"",
            "Keyboard Shortcuts",
            "keys.open",
            "Ctrl+K Ctrl+S"
        ));
        assert!(!matches("\"Ctrl+K\"", "Save", "file.save", "Ctrl+S"));
    }
}
