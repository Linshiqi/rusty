//! Clone: the one git command that runs with no project open, because it is
//! how a project arrives. A URL and a folder, the directory it makes said
//! before it runs (what `git clone` itself would name it), the clone in the
//! dock like any other command, and the checkout opened once it succeeds.

use std::{path::PathBuf, rc::Rc};

use rusty_embed::CommandPlan;
use rusty_i18n::t;
use slint::{ComponentHandle, SharedString};

use super::App;
use crate::{Shell, run::Ended, work};

impl App {
    pub(super) fn clone_act(self: &Rc<Self>, verb: &str) {
        let Some(window) = self.window() else { return };
        let shell = window.global::<Shell>();
        match verb {
            "open" => {
                if shell.get_clone_into().is_empty() {
                    // Beside the last project, the likeliest place for another.
                    let near = self
                        .state
                        .borrow()
                        .recents
                        .first()
                        .and_then(|p| PathBuf::from(p).parent().map(|d| d.to_path_buf()));
                    if let Some(dir) = near {
                        shell.set_clone_into(dir.to_string_lossy().as_ref().into());
                    }
                }
                shell.set_clone_open(true);
                self.clone_edited();
            }
            "cancel" => shell.set_clone_open(false),
            "choose" => work::spawn(
                || rfd::FileDialog::new().pick_folder(),
                |app, picked| {
                    if let (Some(dir), Some(window)) = (picked, app.window()) {
                        window
                            .global::<Shell>()
                            .set_clone_into(dir.to_string_lossy().as_ref().into());
                        app.clone_edited();
                    }
                },
            ),
            "run" => self.clone_run(),
            _ => {}
        }
    }

    /// Where the clone will land, said as it is typed.
    pub(super) fn clone_edited(&self) {
        let Some(window) = self.window() else { return };
        let shell = window.global::<Shell>();
        let target = clone_target(&shell.get_clone_url(), &shell.get_clone_into());
        shell.set_clone_creates(
            target
                .as_ref()
                .map(|p| t!("git.clone-creates", path = p.to_string_lossy().to_string()))
                .unwrap_or_default()
                .into(),
        );
        shell.set_clone_ready(target.is_some_and(|p| !p.exists()));
    }

    fn clone_run(self: &Rc<Self>) {
        let Some(window) = self.window() else { return };
        let shell = window.global::<Shell>();
        let url = shell.get_clone_url().trim().to_string();
        let into = PathBuf::from(shell.get_clone_into().trim());
        let Some(target) = clone_target(&url, &into.to_string_lossy()) else {
            return;
        };
        if target.exists() {
            return;
        }
        shell.set_clone_open(false);
        shell.set_clone_url(SharedString::new());
        let name = target
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let plan = CommandPlan::new(
            "git",
            vec!["clone".into(), "--".into(), url, name],
            String::new(),
        );
        self.run(&plan, Some(into), move |app, ended| {
            if ended == Ended::Done {
                app.open_project(target, None);
            }
        });
    }
}

/// The directory a clone of `url` into `into` makes — what `git clone`
/// itself would pick — or `None` while either is missing.
fn clone_target(url: &str, into: &str) -> Option<PathBuf> {
    let name = rusty_git::repo_name(url)?;
    let into = into.trim();
    (!into.is_empty()).then(|| PathBuf::from(into).join(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_clone_lands_where_git_would_put_it() {
        assert_eq!(
            clone_target("https://github.com/user/repo.git", "/work"),
            Some(PathBuf::from("/work").join("repo"))
        );
        assert_eq!(clone_target("", "/work"), None);
        assert_eq!(clone_target("https://github.com/user/repo.git", " "), None);
    }
}
