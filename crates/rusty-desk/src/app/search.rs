//! The Search panel: the project searched as it is typed, on ripgrep's own
//! engine (`rusty_edit::search`), every hit under its file with the match
//! picked out, and a click going to it. Literal unless the regex switch is
//! on; a pattern or a glob that does not parse is named rather than
//! matching nothing; more hits than the cap says "first 500".

use std::{rc::Rc, time::Duration};

use rusty_edit::SearchQuery as Query;
use rusty_i18n::t;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use super::App;
use crate::{SearchRow, Shell, work};

/// What the panel holds.
#[derive(Default)]
pub(super) struct Search {
    query: Query,
    /// Counts the asks, so an answer to an older one is dropped.
    asked: u64,
    hits: Vec<rusty_edit::SearchHit>,
    files: u32,
    truncated: bool,
    error: Option<String>,
}

impl App {
    /// The query or a switch changed: searched a moment after the typing
    /// stops.
    pub(super) fn search_edited(self: &Rc<Self>, what: &str, text: &str) {
        let asked = {
            let mut state = self.state.borrow_mut();
            let search = &mut state.search;
            match what {
                "text" => search.query.text = text.to_string(),
                "include" => search.query.include = text.to_string(),
                "exclude" => search.query.exclude = text.to_string(),
                "case" => search.query.case_sensitive = !search.query.case_sensitive,
                "word" => search.query.whole_word = !search.query.whole_word,
                "regex" => search.query.regex = !search.query.regex,
                _ => {}
            }
            search.asked += 1;
            search.asked
        };
        self.sync_search();
        let weak = Rc::downgrade(self);
        slint::Timer::single_shot(Duration::from_millis(200), move || {
            if let Some(app) = weak.upgrade() {
                app.search_run(asked);
            }
        });
    }

    fn search_run(self: &Rc<Self>, asked: u64) {
        let (root, query) = {
            let state = self.state.borrow();
            if state.search.asked != asked {
                return;
            }
            let Some(root) = state.project.as_ref().map(|p| p.root.clone()) else {
                return;
            };
            (root, state.search.query.clone())
        };
        work::spawn(
            move || rusty_edit::search(&root, &query),
            move |app, found| {
                {
                    let mut state = app.state.borrow_mut();
                    if state.search.asked != asked {
                        return;
                    }
                    let search = &mut state.search;
                    search.hits = found.hits;
                    search.files = found.files;
                    search.truncated = found.truncated;
                    search.error = found.error;
                }
                app.sync_search();
            },
        );
    }

    /// A hit clicked: the file at the match.
    pub(super) fn search_open(self: &Rc<Self>, index: i32) {
        let hit = usize::try_from(index)
            .ok()
            .and_then(|i| self.state.borrow().search.hits.get(i).cloned());
        if let Some(hit) = hit {
            self.open_file_at(hit.path, Some((hit.line as usize, hit.col as usize)));
        }
    }

    pub(super) fn sync_search(&self) {
        let Some(window) = self.window() else { return };
        let shell = window.global::<Shell>();
        let state = self.state.borrow();
        let search = &state.search;
        shell.set_search_case(search.query.case_sensitive);
        shell.set_search_word(search.query.whole_word);
        shell.set_search_regex(search.query.regex);
        let mut rows = Vec::new();
        let mut current: Option<&str> = None;
        for (index, hit) in search.hits.iter().enumerate() {
            if current != Some(hit.path.as_str()) {
                current = Some(hit.path.as_str());
                let count = search.hits.iter().filter(|h| h.path == hit.path).count();
                rows.push(SearchRow {
                    file: true,
                    path: hit.path.as_str().into(),
                    count: count as i32,
                    ..SearchRow::default()
                });
            }
            let (start, end) = (hit.span_start as usize, hit.span_end as usize);
            let text = hit.text.as_str();
            let cut = |at: usize| {
                let mut at = at.min(text.len());
                while !text.is_char_boundary(at) {
                    at -= 1;
                }
                at
            };
            let (start, end) = (cut(start), cut(end.max(start)));
            let before = text[..start].trim_start();
            rows.push(SearchRow {
                file: false,
                path: hit.path.as_str().into(),
                line: hit.line as i32 + 1,
                before: before.into(),
                matched: text[start..end].into(),
                after: text[end..].trim_end().into(),
                index: index as i32,
                ..SearchRow::default()
            });
        }
        let summary = if search.query.text.is_empty() {
            t!("search.as-you-type")
        } else if let Some(error) = &search.error {
            error.clone()
        } else if search.hits.is_empty() {
            t!("search.nothing")
        } else {
            let mut said = t!(
                "desk.search-found",
                hits = search.hits.len(),
                files = search.files
            );
            if search.truncated {
                said = t!("desk.search-first", said = said);
            }
            said
        };
        shell.set_search_summary(SharedString::from(summary));
        shell.set_search_trouble(search.error.is_some());
        shell.set_search_rows(ModelRc::new(VecModel::from(rows)));
    }
}
