//! `rusty-setup.exe`: install rusty, or — as `uninstall.exe`, carrying no
//! payload — remove it. Double-clicked it is a window (`ui`); run by the
//! updater with `--quiet` or `--silent` it asks nothing and shows nothing
//! but a failure. See the library for what each half does.

#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(not(windows))]
fn main() {
    eprintln!(
        "rusty-setup installs rusty on Windows. On macOS rusty comes as a disk image, \
         on Linux as an AppImage or a .deb."
    );
    std::process::exit(2);
}

#[cfg(windows)]
mod ui;

#[cfg(windows)]
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flags = rusty_setup::Flags::parse(&args);
    let exe = std::env::current_exe()
        .ok()
        .and_then(|path| std::fs::read(path).ok())
        .unwrap_or_default();
    let carried = rusty_setup::payload::find(&exe).map(|(own, payload)| (own.len(), payload.len()));
    let done = match carried {
        Some(_) if !flags.asks_nothing() => ui::install(exe, &flags),
        Some((own, length)) => {
            let (own, payload) = (&exe[..own], &exe[own..own + length]);
            windows::install_quietly(own, payload, &flags)
        }
        None if !flags.asks_nothing() => ui::uninstall(&flags),
        None => windows::uninstall_quietly(&flags),
    };
    if let Err(why) = done {
        if !flags.silent {
            windows::tell(&why);
        }
        std::process::exit(1);
    }
}

#[cfg(windows)]
mod windows {
    use std::path::{Path, PathBuf};

    use rfd::{MessageButtons, MessageDialog, MessageLevel};
    use rusty_setup::{Flags, files, shell};

    pub const VERSION: &str = env!("CARGO_PKG_VERSION");
    pub const UNINSTALLER: &str = "uninstall.exe";

    /// A failure, said where no window of ours is up.
    pub fn tell(text: &str) {
        MessageDialog::new()
            .set_title("rusty")
            .set_level(MessageLevel::Error)
            .set_description(text)
            .set_buttons(MessageButtons::Ok)
            .show();
    }

    /// Where an install goes: what the command line says, else where rusty
    /// already is, else the per-user programs folder.
    pub fn target_dir(flags: &Flags) -> PathBuf {
        flags
            .dir
            .clone()
            .or_else(|| std::env::var_os("RUSTY_SETUP_DIR").map(Into::into))
            .or_else(shell::installed_at)
            .unwrap_or_else(shell::default_dir)
    }

    /// Whether `dir` holds a rusty already — an update rather than an
    /// install.
    pub fn holds_rusty(dir: &Path) -> bool {
        dir.join("rusty.exe").exists()
    }

    /// Whether the desktop has rusty's shortcut.
    pub fn desktop_link() -> PathBuf {
        shell::link_dirs().1.join("rusty.lnk")
    }

    /// What an install does, in order, saying how far it has got. Returns
    /// the app it installed.
    pub fn install_into(
        own: &[u8],
        payload: &[u8],
        dir: &Path,
        desktop: bool,
        progress: &mut dyn FnMut(Step, f32),
    ) -> Result<PathBuf, String> {
        files::install_counted(payload, dir, &[UNINSTALLER], &mut |done| {
            progress(Step::Files, done * 0.96)
        })?;
        progress(Step::Shortcuts, 0.97);
        // The installer without its payload is the uninstaller.
        let uninstaller = dir.join(UNINSTALLER);
        let _ = std::fs::remove_file(&uninstaller);
        std::fs::write(&uninstaller, own).map_err(|e| format!("{UNINSTALLER}: {e}"))?;
        let size_kb = installed_size(dir) / 1024;
        shell::register(dir, VERSION, size_kb).map_err(|e| format!("the uninstall entry: {e}"))?;
        let app = dir.join("rusty.exe");
        let (start, _) = shell::link_dirs();
        shell::shortcut(&start.join("rusty.lnk"), &app)?;
        let on_desktop = desktop_link();
        if desktop {
            let _ = shell::shortcut(&on_desktop, &app);
        } else {
            let _ = std::fs::remove_file(&on_desktop);
        }
        progress(Step::Shortcuts, 1.0);
        Ok(app)
    }

    /// What the progress is busy with, for the window to say.
    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    pub enum Step {
        Files,
        Shortcuts,
    }

    /// rusty, opened with what the command line passed on for it.
    pub fn open(app: &Path, args: &[String]) -> Result<(), String> {
        std::process::Command::new(app)
            .args(args)
            .current_dir(app.parent().unwrap_or(app))
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("rusty.exe: {e}"))
    }

    /// The updater's install: no window, the rusty that asked gone first,
    /// and rusty opened again when `--run` says so. A desktop shortcut is
    /// made on a first install and kept pointing at rusty on an update —
    /// never put back on a desktop somebody cleared.
    pub fn install_quietly(own: &[u8], payload: &[u8], flags: &Flags) -> Result<(), String> {
        let dir = target_dir(flags);
        let desktop = !holds_rusty(&dir) || desktop_link().exists();
        if let Some(pid) = flags.wait {
            shell::wait_for(pid);
        }
        let app = install_into(own, payload, &dir, desktop, &mut |_, _| {})?;
        if flags.run {
            open(&app, &flags.args)?;
        }
        Ok(())
    }

    /// The directory this uninstaller stands in, when it holds a rusty.
    pub fn own_dir() -> Result<PathBuf, String> {
        let me = std::env::current_exe().map_err(|e| e.to_string())?;
        let dir = me.parent().ok_or("no directory")?.to_path_buf();
        if !dir.join(files::MANIFEST).exists() && !dir.join("rusty.exe").exists() {
            return Err(format!(
                "This is rusty's uninstaller, and {} holds no rusty to remove.",
                dir.display()
            ));
        }
        Ok(dir)
    }

    /// What an uninstall does, saying how far it has got.
    pub fn uninstall_from(dir: &Path, progress: &mut dyn FnMut(f32)) -> Result<(), String> {
        let held = files::uninstall_counted(dir, UNINSTALLER, progress);
        if held.iter().any(|rel| rel == "rusty.exe") {
            return Err("rusty is still open. Close it and run the uninstaller again.".into());
        }
        shell::unregister();
        let (start, desktop) = shell::link_dirs();
        for link in [start.join("rusty.lnk"), desktop.join("rusty.lnk")] {
            let _ = std::fs::remove_file(link);
        }
        progress(1.0);
        Ok(())
    }

    pub fn uninstall_quietly(_flags: &Flags) -> Result<(), String> {
        let dir = own_dir()?;
        uninstall_from(&dir, &mut |_| {})?;
        remove_after_exit(&dir);
        Ok(())
    }

    /// The uninstaller cannot delete itself while it runs: a shell does
    /// it a moment after — and whatever was renamed aside because
    /// something held it, the virus scanner reading a fresh `rusty.exe`
    /// for one — and the directory with them once that is empty.
    pub fn remove_after_exit(dir: &Path) {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let me = dir.join(UNINSTALLER);
        let line = format!(
            "ping -n 4 127.0.0.1 > nul & del /f /q \"{}\" & del /f /s /q \"{}\\*.old\" & rmdir \"{}\"",
            me.display(),
            dir.display(),
            dir.display()
        );
        let _ = std::process::Command::new("cmd.exe")
            .arg("/C")
            .raw_arg(line)
            .creation_flags(CREATE_NO_WINDOW)
            .spawn();
    }

    fn installed_size(dir: &Path) -> u64 {
        files::installed(dir)
            .iter()
            .filter_map(|rel| std::fs::metadata(dir.join(rel)).ok())
            .map(|m| m.len())
            .sum()
    }
}
