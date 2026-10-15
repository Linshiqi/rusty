//! `rusty-setup.exe`: install rusty, or — as `uninstall.exe`, carrying no
//! payload — remove it. See the library for what each half does.

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
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flags = rusty_setup::Flags::parse(&args);
    let exe = std::env::current_exe()
        .ok()
        .and_then(|path| std::fs::read(path).ok())
        .unwrap_or_default();
    let done = match rusty_setup::payload::find(&exe) {
        Some((own, payload)) => windows::install(own, payload, &flags),
        None => windows::uninstall(&flags),
    };
    if let Err(why) = done {
        if !flags.silent {
            windows::tell(&why, true);
        }
        std::process::exit(1);
    }
}

#[cfg(windows)]
mod windows {
    use std::path::Path;

    use rfd::{MessageButtons, MessageDialog, MessageDialogResult, MessageLevel};
    use rusty_setup::{Flags, files, shell};

    const VERSION: &str = env!("CARGO_PKG_VERSION");
    const UNINSTALLER: &str = "uninstall.exe";

    pub fn tell(text: &str, error: bool) {
        MessageDialog::new()
            .set_title("rusty")
            .set_level(if error {
                MessageLevel::Error
            } else {
                MessageLevel::Info
            })
            .set_description(text)
            .set_buttons(MessageButtons::Ok)
            .show();
    }

    fn ask(text: &str) -> bool {
        matches!(
            MessageDialog::new()
                .set_title("rusty")
                .set_description(text)
                .set_buttons(MessageButtons::OkCancel)
                .show(),
            MessageDialogResult::Ok | MessageDialogResult::Yes
        )
    }

    pub fn install(own: &[u8], payload: &[u8], flags: &Flags) -> Result<(), String> {
        let dir = flags
            .dir
            .clone()
            .or_else(|| std::env::var_os("RUSTY_SETUP_DIR").map(Into::into))
            .or_else(shell::installed_at)
            .unwrap_or_else(shell::default_dir);
        let fresh = !dir.join("rusty.exe").exists();
        if !flags.asks_nothing()
            && !ask(&format!(
                "Install rusty {VERSION} in {}?\n\nYour settings and projects are not touched.",
                dir.display()
            ))
        {
            return Ok(());
        }
        // The rusty that asked for this goes first.
        if let Some(pid) = flags.wait {
            shell::wait_for(pid);
        }
        files::install(payload, &dir, &[UNINSTALLER])?;
        // The installer without its payload is the uninstaller.
        let uninstaller = dir.join(UNINSTALLER);
        let _ = std::fs::remove_file(&uninstaller);
        std::fs::write(&uninstaller, own).map_err(|e| format!("{UNINSTALLER}: {e}"))?;
        let size_kb = installed_size(&dir) / 1024;
        shell::register(&dir, VERSION, size_kb).map_err(|e| format!("the uninstall entry: {e}"))?;
        let app = dir.join("rusty.exe");
        let (start, desktop) = shell::link_dirs();
        shell::shortcut(&start.join("rusty.lnk"), &app)?;
        // A desktop shortcut is made on a fresh install and kept pointing
        // at rusty on an update — never put back on a desktop somebody
        // cleared.
        let on_desktop = desktop.join("rusty.lnk");
        if fresh || on_desktop.exists() {
            let _ = shell::shortcut(&on_desktop, &app);
        }
        let open = flags.run
            || (!flags.asks_nothing()
                && ask(&format!("rusty {VERSION} is installed. Open it now?")));
        if open {
            std::process::Command::new(&app)
                .args(&flags.args)
                .current_dir(&dir)
                .spawn()
                .map_err(|e| format!("rusty.exe: {e}"))?;
        }
        Ok(())
    }

    pub fn uninstall(flags: &Flags) -> Result<(), String> {
        let me = std::env::current_exe().map_err(|e| e.to_string())?;
        let dir = me.parent().ok_or("no directory")?.to_path_buf();
        if !dir.join(files::MANIFEST).exists() && !dir.join("rusty.exe").exists() {
            return Err(format!(
                "This is rusty's uninstaller, and {} holds no rusty to remove.",
                dir.display()
            ));
        }
        if !flags.asks_nothing()
            && !ask(
                "Remove rusty from this computer?\n\nYour settings, the tools it installed and your projects stay.",
            )
        {
            return Ok(());
        }
        let held = files::uninstall(&dir, UNINSTALLER);
        if held.iter().any(|rel| rel == "rusty.exe") {
            return Err("rusty is still open. Close it and run the uninstaller again.".into());
        }
        shell::unregister();
        let (start, desktop) = shell::link_dirs();
        for link in [start.join("rusty.lnk"), desktop.join("rusty.lnk")] {
            let _ = std::fs::remove_file(link);
        }
        remove_after_exit(&me, &dir);
        if !flags.asks_nothing() {
            tell(
                "rusty was removed. Your settings and projects are where they were.",
                false,
            );
        }
        Ok(())
    }

    /// The uninstaller cannot delete itself while it runs: a shell does
    /// it a moment after — and whatever was renamed aside because
    /// something held it, the virus scanner reading a fresh `rusty.exe`
    /// for one — and the directory with them once that is empty.
    fn remove_after_exit(me: &Path, dir: &Path) {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
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
