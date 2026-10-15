//! New Window, and a project opened in one: VS Code's choice of here or a
//! new window, from the project menu, the welcome page's recents and the
//! Open dialog. `crate::instances` starts the process and keeps the count;
//! this is when to ask it.
//!
//! The windows share one `workbench.toml`, written through `config::update`
//! under its lock, so each writes only what it changed — its own project's
//! tab strip, a recent project, a key. What another window wrote is read
//! back where it is shown: the recents when the project menu or the welcome
//! page is looked at, the keys when the shortcuts page opens.

use std::{path::PathBuf, rc::Rc};

use rusty_i18n::t;
use slint::ComponentHandle;

use super::{App, Mode};
use crate::{instances, work};

impl App {
    /// Another rusty: on the welcome page, or with `project` open. A
    /// project open in this window or another is said, not opened twice.
    pub(super) fn new_window(self: &Rc<Self>, project: Option<PathBuf>) {
        let here = self.root();
        work::spawn(
            move || {
                let name = |root: &PathBuf| {
                    root.file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| root.display().to_string())
                };
                match &project {
                    Some(root)
                        if here
                            .as_ref()
                            .is_some_and(|open| rusty_embed::config::same_dir(open, root)) =>
                    {
                        Err(t!("windows.open-here", name = name(root)))
                    }
                    Some(root) if instances::holding(root) => {
                        Err(t!("windows.open-elsewhere", name = name(root)))
                    }
                    _ => instances::open(project.as_deref())
                        .map_err(|reason| t!("windows.new-failed", reason = reason)),
                }
            },
            |app, started| {
                if let Err(said) = started {
                    app.note_transient(1, said);
                }
            },
        );
    }

    /// Open… for a new window: the folder chosen opens there.
    pub(super) fn choose_project_new_window(self: &Rc<Self>) {
        work::spawn(
            || rfd::FileDialog::new().pick_folder(),
            |app, picked| {
                if let Some(root) = picked {
                    app.new_window(Some(root));
                }
            },
        );
    }

    /// Tell the other windows which project this one has open.
    pub(super) fn tell_instances(&self) {
        let root = self.root();
        std::thread::spawn(move || instances::opened(root.as_deref()));
    }

    /// Read `workbench.toml` again for what another window may have
    /// changed: the recents and the keys.
    pub(super) fn reread_workbench(self: &Rc<Self>) {
        work::spawn(rusty_embed::config::workbench, |app, saved| {
            app.state.borrow_mut().recents = saved.recent_projects;
            app.sync_project();
            app.keys_read(&saved.keybinds);
            let listing = app
                .state
                .borrow()
                .picker
                .as_ref()
                .is_some_and(|(mode, _)| *mode == Mode::Projects);
            if listing && let Some(window) = app.window() {
                let text = window.global::<crate::Shell>().get_picker_text();
                app.picker_query(&text);
            }
        });
    }
}
