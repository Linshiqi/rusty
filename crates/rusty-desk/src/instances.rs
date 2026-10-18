//! More than one rusty: New Window, and the other windows on this machine.
//!
//! **A second project is a second process.** One rusty holds one project —
//! its language servers, its watcher, its runs, the emulator — so New
//! Window starts this executable again, with [`NEW_WINDOW`] to open on the
//! welcome page rather than on the project the last session had: that one
//! is open in the window it was asked from, and two windows on one project
//! are two rust-analyzers taking turns at one build directory, and two
//! hot exits keeping drafts of the same files.
//!
//! **The windows know of each other through files.** Each instance holds an
//! exclusive lock on `<pid>.lock` in [`directory`] for as long as it runs,
//! and names the project it has open in `<pid>.project` beside it. The
//! system lets go of a lock when its process ends, however it ends — a
//! crash included — so a file nobody holds is a window that has gone, and
//! the next look removes it: nothing has to tidy up on the way out, which
//! is the moment least certain to happen. Two things read them: opening a
//! project another window has open says so instead (`holding`), and the
//! restart that installs an update says how many other windows stay open
//! on the version they started with (`others`) — the installer replaces
//! what they hold by renaming it aside, so they keep running and are not
//! closed.

use std::{
    ffi::OsString,
    fs::{self, File, OpenOptions, TryLockError},
    io,
    path::{Path, PathBuf},
    sync::OnceLock,
};

/// What New Window starts this app with: open on the welcome page.
pub const NEW_WINDOW: &str = "--new-window";

/// Another rusty: on the welcome page, or with `project` open.
///
/// The child is nobody's to wait on, so a thread reaps it — on Unix one
/// left unwaited is a zombie until this instance exits — and it starts in a
/// process group of its own there, so a Ctrl+C in the terminal that ran the
/// first does not end the second. It opens no console window: a debug build
/// is a console program.
pub fn open(project: Option<&Path>) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let program = program(exe, std::env::var_os("APPIMAGE"));
    // A Mac bundle's binary is started through `open`, so the Dock shows
    // one rusty and not a second, nameless icon for a bare binary.
    let (program, leading) = crate::platform::launcher(&program);
    let mut command = std::process::Command::new(&program);
    command.args(&leading);
    match project {
        Some(root) => command.arg(root),
        None => command.arg(NEW_WINDOW),
    };
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    rusty_embed::process::no_console_window(&mut command);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command
        .spawn()
        .map_err(|e| format!("{}: {e}", program.display()))?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

/// What to start again. From an AppImage, the AppImage: this executable
/// lives in the mount the running image made, which goes when this instance
/// does and would take a second one started from it along. Anything else is
/// this executable — an installed app, a macOS bundle's binary, a dev build.
fn program(exe: PathBuf, appimage: Option<OsString>) -> PathBuf {
    appimage
        .filter(|image| !image.is_empty())
        .map(PathBuf::from)
        .unwrap_or(exe)
}

/// Where the instances register: beside the data directory a run was given
/// (`RUSTY_CONFIG_DIR`, so a test counts only its own windows), else a
/// machine-local directory — never the data directory, which may be a
/// folder another machine syncs.
pub fn directory() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("RUSTY_CONFIG_DIR") {
        return Some(PathBuf::from(dir).join("instances"));
    }
    #[cfg(windows)]
    let base = std::env::var_os("LOCALAPPDATA").map(|base| PathBuf::from(base).join("rusty"));
    #[cfg(not(windows))]
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(|run| PathBuf::from(run).join("rusty"))
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache/rusty")));
    Some(base?.join("instances"))
}

/// This process's registration, made once as it starts.
static THIS: OnceLock<Instance> = OnceLock::new();

/// Register this process, so the other windows count it. Best effort: a
/// machine with nowhere to put the file counts no other window, which
/// leaves every question about them unasked rather than asked wrongly.
pub fn register() {
    if let Some(dir) = directory()
        && let Ok(instance) = Instance::register(&dir)
    {
        let _ = THIS.set(instance);
    }
}

/// Say which project this window has open, or that it has none.
pub fn opened(project: Option<&Path>) {
    if let Some(this) = THIS.get() {
        this.opened(project);
    }
}

/// How many other rusty windows are open.
pub fn others() -> usize {
    THIS.get().map_or(0, |this| this.others().len())
}

/// Whether another rusty window has `root` open.
pub fn holding(root: &Path) -> bool {
    THIS.get().is_some_and(|this| {
        this.others()
            .iter()
            .flatten()
            .any(|open| rusty_embed::config::same_dir(open, root))
    })
}

/// This instance, counted by the others: the lock on its file is the
/// registration, held for as long as the value lives.
pub struct Instance {
    _held: File,
    path: PathBuf,
}

impl Instance {
    fn register(dir: &Path) -> io::Result<Instance> {
        Instance::register_as(dir, &std::process::id().to_string())
    }

    /// The file is locked under a name the count never reads and only then
    /// renamed into place: an instance counting in that moment would
    /// otherwise find it unheld, take it for a window that had gone, and
    /// delete it before this one had locked it.
    fn register_as(dir: &Path, name: &str) -> io::Result<Instance> {
        fs::create_dir_all(dir)?;
        let pending = dir.join(format!("{name}.pending"));
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&pending)?;
        file.try_lock().map_err(io::Error::from)?;
        let path = dir.join(format!("{name}.lock"));
        fs::rename(&pending, &path)?;
        let _ = fs::remove_file(path.with_extension("project"));
        Ok(Instance { _held: file, path })
    }

    /// Write down the project this window has open, beside its lock — not
    /// in it: a locked file is one nobody else can read on Windows.
    fn opened(&self, project: Option<&Path>) {
        let marker = self.path.with_extension("project");
        let _ = match project {
            Some(root) => fs::write(&marker, root.to_string_lossy().as_bytes()),
            None => fs::remove_file(&marker),
        };
    }

    /// Every other instance that is open, with the project it has open.
    /// A file nobody holds is removed on the way, with its project, and so
    /// is a registration a crash left half made.
    fn others(&self) -> Vec<Option<PathBuf>> {
        let Some(dir) = self.path.parent() else {
            return Vec::new();
        };
        let Ok(entries) = fs::read_dir(dir) else {
            return Vec::new();
        };
        entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| *path != self.path)
            .filter(|path| match path.extension().and_then(|e| e.to_str()) {
                Some("lock") => held(path),
                // Looked at only to tidy: a registration still being made
                // is a window a moment from counting, not one yet.
                Some("pending") => {
                    let _ = held(path);
                    false
                }
                _ => false,
            })
            .map(|lock| {
                fs::read_to_string(lock.with_extension("project"))
                    .ok()
                    .filter(|text| !text.is_empty())
                    .map(PathBuf::from)
            })
            .collect()
    }
}

/// Whether somebody holds `path`. When nobody does, the file was left by an
/// instance that has gone, and it goes too, with what it said was open.
fn held(path: &Path) -> bool {
    let Ok(file) = OpenOptions::new().write(true).open(path) else {
        return false;
    };
    match file.try_lock() {
        Ok(()) => {
            drop(file);
            let _ = fs::remove_file(path);
            let _ = fs::remove_file(path.with_extension("project"));
            false
        }
        Err(TryLockError::WouldBlock) => true,
        // Neither answer: counted as no window, since a question about a
        // window that is not there is the worse of the two.
        Err(TryLockError::Error(_)) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_appimage_starts_the_image_and_everything_else_itself() {
        let exe = PathBuf::from("/tmp/.mount_rustyX/usr/bin/rusty");
        assert_eq!(
            program(exe.clone(), Some("/home/me/rusty.AppImage".into())),
            PathBuf::from("/home/me/rusty.AppImage"),
            "the mount goes with the instance that made it"
        );
        assert_eq!(program(exe.clone(), None), exe);
        assert_eq!(program(exe.clone(), Some(OsString::new())), exe);
    }

    /// Two registrations in one process are two owners to the system — a
    /// lock belongs to its handle — which is what lets one test stand in
    /// for two windows.
    #[test]
    fn each_window_counts_the_others_and_not_itself() {
        let dir = tempfile::tempdir().unwrap();
        let first = Instance::register_as(dir.path(), "first").unwrap();
        assert!(first.others().is_empty());

        let second = Instance::register_as(dir.path(), "second").unwrap();
        assert_eq!(first.others().len(), 1);
        assert_eq!(second.others().len(), 1);

        drop(second);
        assert!(
            first.others().is_empty(),
            "a window that closed is not counted"
        );
        assert!(
            !dir.path().join("second.lock").exists(),
            "and its file went with it"
        );
    }

    /// What a window has open is what the others read, and a window that
    /// went takes its project with it.
    #[test]
    fn each_window_reads_what_the_others_have_open() {
        let dir = tempfile::tempdir().unwrap();
        let first = Instance::register_as(dir.path(), "first").unwrap();
        let second = Instance::register_as(dir.path(), "second").unwrap();
        assert_eq!(first.others(), [None], "the welcome page");

        second.opened(Some(Path::new("/work/blinky")));
        assert_eq!(first.others(), [Some(PathBuf::from("/work/blinky"))]);
        second.opened(None);
        assert_eq!(first.others(), [None]);

        second.opened(Some(Path::new("/work/blinky")));
        drop(second);
        assert!(first.others().is_empty());
        assert!(!dir.path().join("second.project").exists());
    }

    /// A crash leaves the file and not the lock: nothing counted, and
    /// nothing left behind — a half-made registration included.
    #[test]
    fn a_window_that_crashed_is_not_counted() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("gone.lock"), "").unwrap();
        fs::write(dir.path().join("gone.project"), "/work/blinky").unwrap();
        fs::write(dir.path().join("half.pending"), "").unwrap();
        let this = Instance::register_as(dir.path(), "this").unwrap();
        assert!(this.others().is_empty());
        assert!(!dir.path().join("gone.lock").exists());
        assert!(!dir.path().join("gone.project").exists());
        assert!(!dir.path().join("half.pending").exists());
    }
}
