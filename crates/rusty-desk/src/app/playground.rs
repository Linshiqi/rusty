//! The playground: a project rusty keeps per chip, so code and the board
//! that runs it need no folder, no generator and no name first.
//! `rusty_embed::playground` writes it from the proven templates; this
//! opens it — out of the recents, which are the projects somebody works on
//! — puts its example back, and keeps it as a project of its own.

use std::rc::Rc;

use rusty_embed::{ToolchainRequirement, playground};
use rusty_i18n::t;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use super::App;
use crate::{PlaygroundCard, Shell, work};

/// What the welcome page says a playground is: the part's name, or the
/// drawing one's; and what it takes, from the catalogue.
fn card(id: &str) -> PlaygroundCard {
    if id == rusty_embed::PLAYGROUND_DRAW {
        return PlaygroundCard {
            id: id.into(),
            name: t!("chrome.playground-draw").into(),
            detail: t!("chrome.playground-draw-detail").into(),
            drawing: true,
        };
    }
    let chip = rusty_embed::chip::by_id(id);
    let detail = chip.as_ref().map(|chip| {
        let toolchain = match chip.toolchain {
            ToolchainRequirement::Stock => t!("chrome.playground-stable"),
            ToolchainRequirement::EspXtensa => t!("chrome.playground-espup"),
            ToolchainRequirement::NightlyBuildStd => t!("chrome.playground-nightly"),
        };
        format!("{} · {toolchain}", chip.arch.label())
    });
    PlaygroundCard {
        id: id.into(),
        name: chip.map_or_else(|| id.to_string(), |chip| chip.name).into(),
        detail: detail.unwrap_or_default().into(),
        drawing: false,
    }
}

/// A question with Yes and No, answered before anything is replaced.
fn confirm(question: &str) -> bool {
    use rfd::{MessageButtons, MessageDialog, MessageDialogResult, MessageLevel};
    MessageDialog::new()
        .set_level(MessageLevel::Warning)
        .set_title(t!("playground.title"))
        .set_description(question)
        .set_buttons(MessageButtons::YesNo)
        .show()
        == MessageDialogResult::Yes
}

impl App {
    /// The playground the open project is, if it is one.
    pub(super) fn playground(&self) -> Option<String> {
        let state = self.state.borrow();
        let project = state.project.as_ref()?.embedded.as_ref().ok()?;
        project.playground.clone()
    }

    /// A playground opened: written the first time, kept after, on its
    /// entry point.
    pub(super) fn open_playground(self: &Rc<Self>, id: &str) {
        let Some(data) = rusty_embed::config::data_dir() else {
            self.note(2, t!("desk.no-data-dir"));
            return;
        };
        let id = id.to_string();
        work::spawn(
            move || {
                playground::prepare(&data, &id)
                    .map(|root| (root, id))
                    .map_err(|error| error.to_string())
            },
            |app, prepared| match prepared {
                Ok((root, id)) => {
                    app.open_project(root, Some(rusty_embed::playground_main(&id).to_string()))
                }
                Err(reason) => app.note(2, reason),
            },
        );
    }

    /// Put the example back, after asking. The drafts of what was written
    /// over it go too: kept, they would come back over the example the
    /// moment it opened.
    pub(super) fn reset_playground(self: &Rc<Self>) {
        let (Some(id), Some(data), Some(root)) = (
            self.playground(),
            rusty_embed::config::data_dir(),
            self.root(),
        ) else {
            return;
        };
        if !confirm(&t!("playground.reset-question")) {
            return;
        }
        for (path, _) in self.editor.backups() {
            self.keeper.remove(&root, &path);
            self.editor.close(&path);
        }
        {
            let mut state = self.state.borrow_mut();
            state.tabs = vec![super::Tab::Page("welcome")];
            state.active = Some(0);
        }
        self.sync_tabs();
        work::spawn(
            move || {
                playground::reset(&data, &id)
                    .map(|root| (root, id))
                    .map_err(|error| error.to_string())
            },
            |app, reset| match reset {
                Ok((root, id)) => {
                    app.open_project(root, Some(rusty_embed::playground_main(&id).to_string()))
                }
                Err(reason) => app.note(2, reason),
            },
        );
    }

    /// Keep the playground as a project of its own: what is on screen
    /// written first, so the copy is what the user sees; an empty folder
    /// picked; everything but the build copied there; that project opened,
    /// in the recents now.
    pub(super) fn keep_playground(self: &Rc<Self>) {
        let (Some(id), Some(data)) = (self.playground(), rusty_embed::config::data_dir()) else {
            return;
        };
        let title = t!("playground.keep-title");
        self.write_all_then(move |_| {
            work::spawn(
                move || rfd::FileDialog::new().set_title(title).pick_folder(),
                move |_, picked| {
                    let Some(dest) = picked else { return };
                    work::spawn(
                        move || {
                            playground::keep(&data, &id, &dest)
                                .map(|kept| (kept, id))
                                .map_err(|error| error.to_string())
                        },
                        |app, kept| match kept {
                            Ok((kept, id)) => app.open_project(
                                kept,
                                Some(rusty_embed::playground_main(&id).to_string()),
                            ),
                            Err(reason) => app.note(2, reason),
                        },
                    );
                },
            );
        });
    }

    pub(super) fn sync_playground(&self) {
        let Some(window) = self.window() else { return };
        let shell = window.global::<Shell>();
        let cards: Vec<PlaygroundCard> = rusty_embed::PLAYGROUNDS.into_iter().map(card).collect();
        shell.set_playgrounds(ModelRc::new(VecModel::from(cards)));
        let name = self.playground().map(|id| card(&id).name.to_string());
        // Its folder is the part's id; the title bar names the part.
        if let Some(name) = &name {
            shell.set_project(name.as_str().into());
        }
        shell.set_playground(SharedString::from(name.unwrap_or_default()));
    }
}
