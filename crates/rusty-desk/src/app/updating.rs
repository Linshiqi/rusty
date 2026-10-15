//! rusty updating itself, in the window. The feed, the version comparison
//! and the signature check are `rusty_embed::update`; this is when to ask,
//! what Settings ▸ Updates and the status bar say, and how the verified
//! download replaces the running copy on each system.
//!
//! - **The launch check is quiet; one asked for says what it found.** A
//!   moment after the window opens the feed is asked, and only a newer
//!   release — not skipped — is said, in a note with Download and install.
//!   No network is the ordinary state of a bench, and a note about it at
//!   every launch teaches people to dismiss notes. Check for updates opens
//!   Settings ▸ Updates and says whatever the answer was.
//! - **Download and restart are two gestures.** The download runs in the
//!   background, its progress in the status bar, and is verified before it
//!   is offered; the restart that installs it is asked for, because one
//!   that came by itself when a download happened to finish would take
//!   whatever was running with it.
//! - **How a verified download is put in place is the system's.** On
//!   Windows it is the installer (`rusty-setup`), run with no questions,
//!   this process's id to wait for, and told to open rusty again. On macOS
//!   the app bundle is replaced and opened again; on Linux the AppImage is.
//!   A `.deb` is the package manager's, and the update says so rather than
//!   touching `/usr`.

use std::{path::PathBuf, rc::Rc, time::Duration};

use rusty_embed::update::{self, Feed};
use rusty_i18n::t;
use slint::{ComponentHandle, Timer};

use super::{App, NoteAction, Tab};
use crate::{Shell, work};

/// The running build's version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Where an update stands.
#[derive(Debug, Default, Clone)]
pub(super) enum Stage {
    #[default]
    Idle,
    Checking,
    /// The feed's newest is this build.
    Newest,
    /// The feed could not be read; why.
    Unreachable(String),
    Offered(Feed),
    Downloading {
        feed: Feed,
        received: u64,
        total: Option<u64>,
    },
    Ready {
        feed: Feed,
        file: PathBuf,
    },
    Failed {
        feed: Feed,
        why: String,
    },
}

impl App {
    /// A moment after the window opens: the quiet check.
    pub(super) fn update_soon(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        Timer::single_shot(Duration::from_secs(5), move || {
            if let Some(app) = weak.upgrade() {
                app.update_check(false);
            }
        });
    }

    /// Ask the feed. `manual` is Check for updates: Settings ▸ Updates
    /// comes forward and says the answer whatever it is.
    pub(super) fn update_check(self: &Rc<Self>, manual: bool) {
        if manual {
            self.open_tab(Tab::Page("settings"));
        }
        let busy = matches!(
            self.state.borrow().update,
            Stage::Checking | Stage::Downloading { .. }
        );
        if busy {
            return;
        }
        self.state.borrow_mut().update = Stage::Checking;
        self.sync_update();
        work::spawn(update::fetch_feed, move |app, answer| {
            app.update_answered(answer, manual)
        });
    }

    fn update_answered(self: &Rc<Self>, answer: Result<Feed, String>, manual: bool) {
        let stage = match answer {
            Ok(feed) if update::is_newer(VERSION, &feed.version) && feed.artifact().is_some() => {
                let skipped = rusty_embed::config::workbench().skipped_update.as_deref()
                    == Some(feed.version.as_str());
                if !manual && !skipped {
                    self.note_with(
                        0,
                        t!("update.title-available", version = feed.version.clone()),
                        Some(NoteAction::Act("update.download")),
                    );
                }
                Stage::Offered(feed)
            }
            Ok(_) => Stage::Newest,
            Err(why) => Stage::Unreachable(why),
        };
        self.state.borrow_mut().update = stage;
        self.sync_update();
    }

    /// Download and install: in the background, verified, then offered.
    pub(super) fn update_download(self: &Rc<Self>) {
        let feed = match &self.state.borrow().update {
            Stage::Offered(feed) | Stage::Failed { feed, .. } => feed.clone(),
            _ => return,
        };
        let Some(artifact) = feed.artifact().cloned() else {
            return;
        };
        let name = artifact
            .url
            .rsplit('/')
            .next()
            .filter(|name| !name.is_empty())
            .unwrap_or("rusty-update")
            .to_string();
        let file = std::env::temp_dir().join("rusty-update").join(name);
        self.state.borrow_mut().update = Stage::Downloading {
            feed: feed.clone(),
            received: 0,
            total: None,
        };
        self.sync_update();
        let version = feed.version.clone();
        work::spawn(
            move || -> Result<PathBuf, String> {
                std::fs::create_dir_all(file.parent().unwrap_or(&file))
                    .map_err(|e| e.to_string())?;
                // A delivery per whole percent, not per chunk: the window
                // redraws what it is told.
                let mut told = u64::MAX;
                update::download(&artifact, &file, |progress| {
                    let mark = progress.total.map_or(progress.received >> 20, |total| {
                        progress.received * 100 / total.max(1)
                    });
                    if mark != told {
                        told = mark;
                        work::deliver(progress, |app, progress| app.update_progress(progress));
                    }
                })?;
                let bytes = std::fs::read(&file).map_err(|e| e.to_string())?;
                update::verify(&bytes, &artifact.signature, &version)?;
                Ok(file)
            },
            move |app, done| {
                app.state.borrow_mut().update = match done {
                    Ok(file) => Stage::Ready { feed, file },
                    Err(why) => Stage::Failed { feed, why },
                };
                app.sync_update();
            },
        );
    }

    fn update_progress(self: &Rc<Self>, progress: rusty_embed::model::UpdateProgress) {
        if let Stage::Downloading {
            received, total, ..
        } = &mut self.state.borrow_mut().update
        {
            (*received, *total) = (progress.received, progress.total);
        }
        self.sync_update();
    }

    /// Restart now: what runs is asked about first, then the verified
    /// download is put in place and rusty opened again. Drafts are kept as
    /// at any exit.
    pub(super) fn update_restart(self: &Rc<Self>) {
        let (feed, file) = match &self.state.borrow().update {
            Stage::Ready { feed, file } => (feed.clone(), file.clone()),
            _ => return,
        };
        // The other rusty windows are not closed by an update: on Windows
        // the installer renames aside what they hold, elsewhere the bundle
        // or the image is replaced under them, and each keeps running the
        // version it started with until it is closed — which the question
        // says, since a window still on the old version after "Restart now"
        // reads as an update that did not take.
        let others = crate::instances::others();
        let running = self.runner.is_running();
        if running || others > 0 {
            let mut said = Vec::new();
            if running {
                let what = t!("update.running-something");
                said.push(t!("update.restart-while-running", what = what));
            }
            if others > 0 {
                said.push(t!("update.others-stay", count = others, current = VERSION));
            }
            said.push(if running {
                t!("update.restart-anyway")
            } else {
                t!("update.restart-question")
            });
            let question = said.join("\n\n");
            let go = rfd::MessageDialog::new()
                .set_level(rfd::MessageLevel::Warning)
                .set_title("rusty")
                .set_description(question)
                .set_buttons(rfd::MessageButtons::OkCancel)
                .show();
            if !matches!(
                go,
                rfd::MessageDialogResult::Ok | rfd::MessageDialogResult::Yes
            ) {
                return;
            }
            if running {
                self.runner.stop();
            }
        }
        match put_in_place(&file) {
            Ok(()) => self.act("app.exit"),
            Err(why) => {
                self.state.borrow_mut().update = Stage::Failed { feed, why };
                self.sync_update();
            }
        }
    }

    /// Skip this version: the launch check stays quiet about it.
    pub(super) fn update_skip(self: &Rc<Self>) {
        let version = match &self.state.borrow().update {
            Stage::Offered(feed) | Stage::Failed { feed, .. } => feed.version.clone(),
            _ => return,
        };
        let _ = rusty_embed::config::update(move |saved| saved.skipped_update = Some(version));
        self.state.borrow_mut().update = Stage::Idle;
        self.sync_update();
    }

    /// What Settings ▸ Updates and the status bar show.
    pub(super) fn sync_update(&self) {
        let Some(window) = self.window() else { return };
        let shell = window.global::<Shell>();
        let stage = self.state.borrow().update.clone();
        let size = |bytes: u64| format!("{:.1} MB", bytes as f64 / 1e6);
        let (state, title, detail, progress) = match &stage {
            Stage::Idle => (0, String::new(), String::new(), 0.0),
            Stage::Checking => (1, t!("update.starting"), String::new(), 0.0),
            Stage::Newest => (
                2,
                t!("update.title-newest"),
                t!("update.newest-detail", current = VERSION),
                0.0,
            ),
            Stage::Unreachable(why) => (6, t!("update.title-unreachable"), why.clone(), 0.0),
            Stage::Offered(feed) => (
                3,
                t!("update.title-available", version = feed.version.clone()),
                t!("update.summary", current = VERSION),
                0.0,
            ),
            Stage::Downloading {
                feed,
                received,
                total,
            } => (
                4,
                t!("update.title-available", version = feed.version.clone()),
                match total {
                    Some(total) => t!(
                        "update.downloading",
                        done = size(*received),
                        total = size(*total)
                    ),
                    None => t!("update.downloading-unsized", done = size(*received)),
                },
                total.map_or(-1.0, |total| *received as f32 / total.max(1) as f32),
            ),
            Stage::Ready { feed, .. } => (
                5,
                t!("update.title-available", version = feed.version.clone()),
                t!("update.ready"),
                1.0,
            ),
            Stage::Failed { feed, why } => (
                7,
                t!("update.title-available", version = feed.version.clone()),
                why.clone(),
                0.0,
            ),
        };
        shell.set_update_state(state);
        shell.set_update_title(title.into());
        shell.set_update_detail(detail.into());
        shell.set_update_progress(progress);
        shell.set_update_version(VERSION.into());
        let feed = match &stage {
            Stage::Offered(feed)
            | Stage::Downloading { feed, .. }
            | Stage::Ready { feed, .. }
            | Stage::Failed { feed, .. } => Some(feed.clone()),
            _ => None,
        };
        let notes = feed.map_or(String::new(), |feed| {
            let date = feed
                .date()
                .map(|date| format!("*{}*\n\n", t!("update.published", date = date)))
                .unwrap_or_default();
            if feed.notes.trim().is_empty() {
                format!("{date}{}", t!("update.notes-none"))
            } else {
                format!("{date}{}", feed.notes)
            }
        });
        shell.set_update_notes(super::markdown_view::blocks(&self.files, &notes, None));
    }
}

/// The verified download, put where the running copy is.
fn put_in_place(file: &std::path::Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        // The installer waits for this process, installs where rusty is
        // installed, and opens it again.
        std::process::Command::new(file)
            .args([
                "--quiet",
                "--run",
                "--wait",
                &std::process::id().to_string(),
            ])
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("{}: {e}", file.display()))
    }
    #[cfg(target_os = "macos")]
    {
        replace_bundle(file)
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        replace_appimage(file)
    }
}

/// macOS: the `.app.tar.gz` unpacked beside the running bundle, swapped
/// for it, and opened again.
#[cfg(target_os = "macos")]
fn replace_bundle(archive: &std::path::Path) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let bundle = exe
        .ancestors()
        .find(|dir| dir.extension().is_some_and(|ext| ext == "app"))
        .ok_or("rusty is not running from an app bundle; download the disk image instead")?
        .to_path_buf();
    let parent = bundle.parent().ok_or("the bundle has no folder")?;
    let staging = parent.join(".rusty-update");
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging).map_err(|e| e.to_string())?;
    let gz = flate2::read::GzDecoder::new(std::fs::File::open(archive).map_err(|e| e.to_string())?);
    tar::Archive::new(gz)
        .unpack(&staging)
        .map_err(|e| e.to_string())?;
    let new = std::fs::read_dir(&staging)
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| path.extension().is_some_and(|ext| ext == "app"))
        .ok_or("the update carries no app bundle")?;
    let old = parent.join(".rusty-old.app");
    let _ = std::fs::remove_dir_all(&old);
    std::fs::rename(&bundle, &old).map_err(|e| e.to_string())?;
    if let Err(e) = std::fs::rename(&new, &bundle) {
        let _ = std::fs::rename(&old, &bundle);
        return Err(e.to_string());
    }
    let _ = std::fs::remove_dir_all(&old);
    let _ = std::fs::remove_dir_all(&staging);
    std::process::Command::new("open")
        .arg("-n")
        .arg(&bundle)
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Linux: the AppImage rusty runs from, replaced by the downloaded one and
/// started again. Installed from a `.deb` there is no AppImage, and the
/// files are the package manager's.
#[cfg(all(unix, not(target_os = "macos")))]
fn replace_appimage(file: &std::path::Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let Some(image) = std::env::var_os("APPIMAGE").map(PathBuf::from) else {
        return Err(format!(
            "rusty was installed by the system's package manager: download the new .deb from {}",
            rusty_embed::model::REPO_RELEASES
        ));
    };
    let next = image.with_extension("new");
    std::fs::copy(file, &next).map_err(|e| e.to_string())?;
    std::fs::set_permissions(&next, std::fs::Permissions::from_mode(0o755))
        .map_err(|e| e.to_string())?;
    std::fs::rename(&next, &image).map_err(|e| e.to_string())?;
    std::process::Command::new(&image)
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}
