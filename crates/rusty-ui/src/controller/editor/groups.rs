//! The second editor group: opening a file beside, splitting, and keeping
//! the layout from ever showing an empty pane.

use super::*;

/// Open a file in the right-hand group — VS Code's "Open to the Side" —
/// splitting the editor if it is not split yet.
///
/// "Beside" is the right group from *either* side. With two groups there is
/// nothing further right, and the first version sent a file to "the other
/// group": from the right group that moved it left, and when it was the
/// right group's last file the right group vanished under the click — read,
/// correctly, as the split closing for no reason. A file the left group
/// holds opens on the right as a second view of the same document and stays
/// on the left, as VS Code's split does (`views.rs`); one the right group
/// holds is fronted there.
pub fn open_beside(state: AppState, path: String) {
    let second = state.group(crate::state::Group::Second);
    state.layout.split.set(true);
    state.layout.focus.set(second.group);
    open_file(second, path);
}

/// The left strip's split button and Ctrl+\: the left group's file opens on
/// the right as well. Only the left group has the button — there is nothing
/// further right of the right group.
pub fn split_active(state: AppState) {
    let first = state.group(crate::state::Group::First);
    let Some(path) = first.active_path_now() else {
        return;
    };
    open_beside(first, path);
}

/// After a group lost a file. A second group with nothing left closes; a
/// first group with nothing left while the second has files takes them, so
/// the split never shows an empty pane on the left with the work on the
/// right, and never outlives having two things to compare.
pub(super) fn settle_groups(state: AppState) {
    if !state.layout.split.get_untracked() {
        return;
    }
    let first = state.group(crate::state::Group::First);
    let second = state.group(crate::state::Group::Second);
    if second.editor.tabs.with_untracked(Vec::is_empty) {
        state.layout.split.set(false);
        state.layout.focus.set(crate::state::Group::First);
    } else if first.editor.tabs.with_untracked(Vec::is_empty) {
        let active = second.active_path_now();
        park_active(second);
        first.editor.tabs.set(second.editor.tabs.get_untracked());
        first
            .editor
            .parked
            .set(second.editor.parked.get_untracked());
        second.editor.tabs.set(Vec::new());
        second.editor.parked.set(Vec::new());
        clear_editor_transients(second);
        clear_screen(second);
        state.layout.split.set(false);
        state.layout.focus.set(crate::state::Group::First);
        if let Some(active) = active
            && !front_parked(first, &active)
        {
            open_file(first, active);
        }
    }
}

/// Forget everything a group holds: the project is changing under it.
pub fn reset_group(state: AppState) {
    state.editor.tabs.set(Vec::new());
    state.editor.parked.set(Vec::new());
    // Keyed by path, and the next project has a `src/main.rs` too.
    state.editor.histories.update_value(|all| all.clear());
    clear_editor_transients(state);
    clear_screen(state);
    state.editor.reveal.set(None);
    state.find.open.set(false);
    state.find.replace_open.set(false);
    state.find.query.set(String::new());
    state.find.index.set(0);
}
