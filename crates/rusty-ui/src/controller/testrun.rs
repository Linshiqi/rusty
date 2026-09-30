//! A test run read as it goes (`crate::testrun`): each test's verdict for
//! its lens, and where each failing test stopped for the editor to mark.

use leptos::prelude::*;

use crate::{
    state::AppState,
    testrun::{Heard, Reader},
};

/// A new run: whatever the last one left half-read is not this one's.
pub(super) fn tests_begin(state: AppState) {
    state.tests.reader.set_value(Reader::default());
}

/// One line of a test run, as it passes on its way to the dock.
pub(super) fn tests_heard(state: AppState, text: &str) {
    let heard = state
        .tests
        .reader
        .try_update_value(|reader| reader.read(text))
        .unwrap_or_default();
    record(state, heard);
}

/// The run is over: a panic whose message was still open ends with it.
pub(super) fn tests_end(state: AppState) {
    let heard = state
        .tests
        .reader
        .try_update_value(Reader::finish)
        .unwrap_or_default();
    record(state, heard);
}

/// A test heard from again sheds what its last run left — its verdict is
/// replaced, and its failures go until this run names them — so a test
/// that now passes takes its mark off the line it failed at.
fn record(state: AppState, heard: Vec<Heard>) {
    for heard in heard {
        match heard {
            Heard::Verdict(name, verdict) => {
                if state
                    .tests
                    .verdicts
                    .with_untracked(|known| known.get(&name) != Some(&verdict))
                {
                    state.tests.verdicts.update(|known| {
                        known.insert(name.clone(), verdict);
                    });
                }
                if state
                    .tests
                    .failures
                    .with_untracked(|failures| failures.iter().any(|f| f.test == name))
                {
                    state
                        .tests
                        .failures
                        .update(|failures| failures.retain(|f| f.test != name));
                }
            }
            Heard::Failure(failure) => state.tests.failures.update(|all| all.push(failure)),
        }
    }
}

/// `path` was edited: where a failure stopped in it was a line of the text
/// before, so the mark goes — the lens keeps the verdict, and the dock the
/// message, until the test runs again.
pub(super) fn tests_edited(state: AppState, path: &str) {
    let placed_here = |failures: &Vec<crate::testrun::Failure>| {
        failures
            .iter()
            .any(|f| f.at.as_ref().is_some_and(|(file, ..)| file == path))
    };
    if state.tests.failures.with_untracked(placed_here) {
        state.tests.failures.update(|failures| {
            for failure in failures {
                if failure.at.as_ref().is_some_and(|(file, ..)| file == path) {
                    failure.at = None;
                }
            }
        });
    }
}
