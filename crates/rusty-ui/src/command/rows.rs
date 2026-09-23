//! The palette's rows: every action available right now, with its heading
//! and its key.

use super::*;

/// Everything available right now.
///
/// Panels come from the registry rather than a second hard-coded list, so a
/// contributed panel is reachable from the palette without anyone remembering
/// to add it — the same reason the shell renders from the registry.
pub fn all(state: AppState) -> Vec<Command> {
    let mut out = Vec::new();
    let chord = crate::view::palette::chords(state);

    for panel in panels::all() {
        // Disabled panels stay listed but say why, rather than vanishing —
        // a palette that hides things teaches people it cannot be trusted.
        let blocked = panel.needs_project && !state.has_project();
        out.push(Command {
            action: Action::ShowPanel(panel.id),
            title: if blocked {
                t!("panel.needs-project", panel = panel.title.clone())
            } else {
                panel.title.to_string()
            },
            group: t!("palette.group-panels"),
            shortcut: chord(Action::ShowPanel(panel.id)),
        });
    }

    let action = |action, title: &str, shortcut| Command {
        action,
        title: title.to_string(),
        group: t!("palette.group-project"),
        shortcut,
    };

    out.push(action(
        Action::OpenProject,
        &t!("menu.file.open-project"),
        chord(Action::OpenProject),
    ));
    out.push(action(
        Action::CloneRepository,
        &t!("menu.file.clone"),
        chord(Action::CloneRepository),
    ));
    for chip in rusty_embed::PLAYGROUND_CHIPS {
        out.push(action(
            Action::OpenPlayground(chip),
            &t!("palette.playground", chip = chip_name(state, chip)),
            None,
        ));
    }
    // Only in a playground: anywhere else there is no example to put back.
    if state.playground_now().is_some() {
        out.push(action(
            Action::ResetPlayground,
            &t!("menu.file.playground-reset"),
            None,
        ));
        out.push(action(
            Action::KeepPlayground,
            &t!("menu.file.playground-keep"),
            None,
        ));
    }
    out.push(action(
        Action::RefreshProject,
        &t!("menu.project.recheck"),
        chord(Action::RefreshProject),
    ));
    out.push(action(
        Action::RefreshToolchain,
        &t!("menu.project.rescan-toolchain"),
        None,
    ));
    out.push(action(
        Action::ReloadCatalog,
        &t!("menu.project.reload-catalogue"),
        None,
    ));
    for (verb, title) in [
        (Action::Build, t!("menu.project.build")),
        (Action::Test, t!("menu.project.test")),
        (Action::Run, t!("menu.project.run")),
        (Action::Debug, t!("menu.project.debug")),
        (Action::Stop, t!("menu.project.stop")),
        (Action::Restart, t!("menu.project.restart")),
        (Action::Pause, t!("debugger.pause")),
        (Action::StepOver, t!("debugger.step-over")),
        (Action::StepInto, t!("debugger.step-into")),
        (Action::StepOut, t!("debugger.step-out")),
        (Action::Flash, t!("menu.device.flash")),
        (Action::FlashOnly, t!("menu.device.flash-only")),
        (Action::Monitor, t!("menu.device.monitor")),
        (Action::PickDevice, t!("menu.device.pick")),
        (Action::ScanDevices, t!("menu.device.rescan")),
    ] {
        out.push(action(verb, &title, chord(verb)));
    }

    let view = |action, title: &str, shortcut| Command {
        action,
        title: title.to_string(),
        group: t!("palette.group-view"),
        shortcut,
    };

    out.push(view(
        Action::ToggleDock,
        &t!("palette.toggle-dock"),
        chord(Action::ToggleDock),
    ));
    // Every dock tab, from the one list the dock itself renders. Five of
    // the nine were spelled out here once; the other four were reachable
    // from nowhere but a click on the strip, which this module's header
    // says is exactly the drift it exists to prevent.
    for tab in DockTab::ALL {
        out.push(view(
            Action::ShowDock(tab),
            &t!("palette.show-dock", name = tab.label()),
            None,
        ));
    }
    out.push(view(
        Action::ResetLayout,
        &t!("menu.view.reset-layout"),
        None,
    ));
    out.push(view(
        Action::NavBack,
        &t!("menu.view.back"),
        chord(Action::NavBack),
    ));
    out.push(view(
        Action::NavForward,
        &t!("menu.view.forward"),
        chord(Action::NavForward),
    ));
    out.push(view(
        Action::SwitchEditor,
        &t!("menu.view.switch-editor"),
        chord(Action::SwitchEditor),
    ));
    out.push(view(
        Action::SwitchEditorBack,
        &t!("menu.view.switch-editor-back"),
        chord(Action::SwitchEditorBack),
    ));
    out.push(view(Action::ToggleVim, &t!("menu.view.vim"), None));
    out.push(view(
        Action::QuickOpen,
        &t!("menu.view.quick-open"),
        chord(Action::QuickOpen),
    ));
    for (action, title) in [
        (Action::GoToLine, t!("menu.view.go-to-line")),
        (Action::GoToSymbolInFile, t!("menu.view.symbol-in-file")),
        (
            Action::GoToSymbolInWorkspace,
            t!("menu.view.symbol-in-workspace"),
        ),
        (Action::FindReferences, t!("menu.view.references")),
        (Action::GoToImplementations, t!("menu.view.implementations")),
        (Action::GoToTypeDefinition, t!("menu.view.type-definition")),
        (Action::ShowCallHierarchy, t!("menu.view.call-hierarchy")),
        (Action::ExpandMacro, t!("menu.view.expand-macro")),
    ] {
        out.push(view(action, &title, chord(action)));
    }
    out.push(view(
        Action::ToggleTree,
        &t!("menu.view.toggle-tree"),
        chord(Action::ToggleTree),
    ));
    out.push(view(
        Action::SplitEditor,
        &t!("menu.view.split"),
        chord(Action::SplitEditor),
    ));
    out.push(view(
        Action::ToggleBoard,
        &t!("menu.view.board-beside"),
        chord(Action::ToggleBoard),
    ));

    for theme in Theme::ALL {
        out.push(Command {
            action: Action::SetTheme(theme),
            title: t!("palette.theme", name = theme.label()),
            group: t!("palette.group-settings"),
            shortcut: None,
        });
    }
    out.push(Command {
        action: Action::OpenSettings,
        title: t!("palette.settings"),
        group: t!("palette.group-settings"),
        shortcut: chord(Action::OpenSettings),
    });

    out
}
