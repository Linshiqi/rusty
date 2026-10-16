//! The Output dock and Problems as a run sees them: every run somebody
//! asks for starts on an empty dock with the last build's problems gone,
//! and both panels narrow under the filter box in the dock's title row.
//!
//! A run starts afresh once, at its start (`App::begin_run`): Build, the
//! simulator's Run and its restart, Flash, Monitor, a test or its lens, an
//! example's ▶ Run, Debug. Flash builds first and Run builds and then
//! boots; neither clears between its steps, because the build is what the
//! run began with and its errors are what somebody reads when the write
//! never comes. Before this, a test, an example or a simulation left the
//! last Build's errors listed in Problems over code that had since been
//! mended, and a monitor's lines landed under the build before it.

use std::rc::Rc;

use rusty_embed::activity::Kind;
use rusty_i18n::t;
use slint::ComponentHandle;

use super::App;
use crate::{
    FilterState, Shell,
    filter::{Filter, Level},
    keys, run,
};

/// A panel's filter as the title row draws it.
fn state_of(filter: &Filter, (shown, total): (usize, usize)) -> FilterState {
    FilterState {
        errors: filter.is_on(Level::Error),
        warnings: filter.is_on(Level::Warning),
        other: filter.is_on(Level::Other),
        count: if filter.narrows() {
            t!(
                "desk.filter-count",
                shown = shown.to_string(),
                total = total.to_string()
            )
            .into()
        } else {
            Default::default()
        },
    }
}

fn level_named(name: &str) -> Option<Level> {
    match name {
        "errors" => Some(Level::Error),
        "warnings" => Some(Level::Warning),
        "other" => Some(Level::Other),
        _ => None,
    }
}

impl App {
    /// A run of `kind` somebody asked for begins: the dock empty and the
    /// last build's problems gone, when the kind starts afresh
    /// (`run::starts_afresh`). Refused — `false`, and said — while another
    /// run is going, before anything is cleared: wiping a monitor's lines
    /// for a test that then could not start would lose them for nothing.
    pub(super) fn begin_run(self: &Rc<Self>, kind: Kind) -> bool {
        if self.runner.is_running() {
            self.note_transient(1, t!("desk.already-running"));
            return false;
        }
        if run::starts_afresh(kind) {
            self.runner.clear();
            self.state.borrow_mut().build_log = super::build::BuildLog::default();
            self.sync_problems();
        }
        true
    }

    /// The filter box's text, committed.
    pub(super) fn filter_edited(self: &Rc<Self>, panel: &str, text: &str) {
        match panel {
            "output" => self.runner.set_filter(|filter| filter.set_text(text)),
            "problems" => {
                self.state.borrow_mut().problems_filter.set_text(text);
                self.sync_problems();
            }
            _ => {}
        }
    }

    /// A level's switch beside the box.
    pub(super) fn filter_level(self: &Rc<Self>, panel: &str, level: &str) {
        let Some(level) = level_named(level) else {
            return;
        };
        match panel {
            "output" => self.runner.set_filter(|filter| filter.toggle(level)),
            "problems" => {
                self.state.borrow_mut().problems_filter.toggle(level);
                self.sync_problems();
            }
            _ => {}
        }
    }

    /// A key while Output or Problems has the keys: Find's key goes to the
    /// panel's filter box, as Ctrl+F does in VS Code's Output, and Escape
    /// clears what the box holds.
    pub(super) fn panel_key(
        self: &Rc<Self>,
        panel: &str,
        text: &str,
        (ctrl, alt, shift, meta): (bool, bool, bool, bool),
    ) -> bool {
        let Some(stroke) = keys::chord(text, ctrl, alt, shift, meta) else {
            return false;
        };
        if stroke == "Escape" {
            let typed = match panel {
                "output" => !self.runner.filter().text().is_empty(),
                _ => !self.state.borrow().problems_filter.text().is_empty(),
            };
            if typed {
                self.filter_cleared(panel);
                return true;
            }
            return false;
        }
        let finds =
            self.state.borrow().keys.map.bound_to(&stroke).map(|a| a.id) == Some("find.open");
        if finds {
            self.focus_filter();
            return true;
        }
        false
    }

    /// The box of the panel in front given the keys.
    pub(super) fn focus_filter(&self) {
        if let Some(window) = self.window() {
            let shell = window.global::<Shell>();
            shell.set_filter_focus(shell.get_filter_focus().wrapping_add(1));
        }
    }

    /// The box emptied from outside it: Escape in its panel.
    fn filter_cleared(self: &Rc<Self>, panel: &str) {
        if let Some(window) = self.window() {
            let shell = window.global::<Shell>();
            match panel {
                "output" => shell.set_output_filter_text(Default::default()),
                _ => shell.set_problems_filter_text(Default::default()),
            }
        }
        self.filter_edited(panel, "");
    }

    /// Output's filter and its count, as the title row draws them.
    pub(super) fn sync_output_filter(&self, counts: (usize, usize)) {
        if let Some(window) = self.window() {
            let state = state_of(&self.runner.filter(), counts);
            window.global::<Shell>().set_output_filter(state);
        }
    }

    /// Problems' filter, with how many problems it shows out of how many.
    pub(super) fn problems_filter_state(&self, counts: (usize, usize)) -> FilterState {
        state_of(&self.state.borrow().problems_filter, counts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_count_is_said_only_while_the_filter_narrows() {
        let mut filter = Filter::default();
        assert_eq!(state_of(&filter, (4, 4)).count, "");
        filter.set_text("error");
        let state = state_of(&filter, (2, 40));
        assert!(state.count.contains('2') && state.count.contains("40"));
        filter.set_text("");
        filter.toggle(Level::Warning);
        let state = state_of(&filter, (1, 40));
        assert!(state.warnings && !state.errors);
        assert!(!state.count.is_empty(), "a switch narrows too");
        assert_eq!(level_named("other"), Some(Level::Other));
        assert_eq!(level_named("info"), None);
    }
}
