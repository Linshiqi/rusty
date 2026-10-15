//! Where Windows learns rusty is installed: the entry Settings ▸ Apps lists
//! (`HKCU\…\Uninstall\dev.rusty.workbench`), the Start menu's shortcut and
//! the desktop's.
//!
//! Per user: no elevation, nothing under `Program Files`, nothing another
//! account sees.

use std::path::{Path, PathBuf};

use winreg::{RegKey, enums::HKEY_CURRENT_USER};

/// The entry's key, under `HKCU`. `RUSTY_SETUP_KEY` moves it — the
/// installer's own test puts it somewhere Settings does not look.
pub fn key() -> String {
    std::env::var("RUSTY_SETUP_KEY").unwrap_or_else(|_| {
        r"Software\Microsoft\Windows\CurrentVersion\Uninstall\dev.rusty.workbench".into()
    })
}

/// Where an earlier install says it is, quotes and all taken off.
pub fn installed_at() -> Option<PathBuf> {
    let entry = RegKey::predef(HKEY_CURRENT_USER).open_subkey(key()).ok()?;
    let location: String = entry.get_value("InstallLocation").ok()?;
    let location = location.trim().trim_matches('"');
    (!location.is_empty()).then(|| PathBuf::from(location))
}

/// The directory a fresh install goes to: `%LOCALAPPDATA%\Programs\rusty`,
/// where a per-user install of a program goes.
pub fn default_dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Users\Default\AppData\Local"))
        .join("Programs")
        .join("rusty")
}

/// The entry, written over whatever was there.
pub fn register(dir: &Path, version: &str, size_kb: u64) -> std::io::Result<()> {
    let (entry, _) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(key())?;
    let quoted = |path: &Path| format!("\"{}\"", path.display());
    entry.set_value("DisplayName", &"rusty")?;
    entry.set_value("DisplayVersion", &version)?;
    entry.set_value("Publisher", &"rusty")?;
    entry.set_value("InstallLocation", &quoted(dir))?;
    entry.set_value("DisplayIcon", &dir.join("rusty.exe").display().to_string())?;
    entry.set_value("UninstallString", &quoted(&dir.join("uninstall.exe")))?;
    entry.set_value(
        "QuietUninstallString",
        &format!("{} --silent", quoted(&dir.join("uninstall.exe"))),
    )?;
    entry.set_value("URLInfoAbout", &rusty_embed::model::REPO)?;
    entry.set_value("NoModify", &1u32)?;
    entry.set_value("NoRepair", &1u32)?;
    entry.set_value("EstimatedSize", &(size_kb.min(u64::from(u32::MAX)) as u32))?;
    Ok(())
}

pub fn unregister() {
    let _ = RegKey::predef(HKEY_CURRENT_USER).delete_subkey_all(key());
}

/// Where the shortcuts go: the Start menu's programs and the desktop.
/// `RUSTY_SETUP_LINKS` puts both in one directory of its own, for the test.
pub fn link_dirs() -> (PathBuf, PathBuf) {
    if let Some(dir) = std::env::var_os("RUSTY_SETUP_LINKS") {
        let dir = PathBuf::from(dir);
        return (dir.join("start"), dir.join("desktop"));
    }
    let roaming = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_default();
    let start = roaming.join(r"Microsoft\Windows\Start Menu\Programs");
    let desktop = known_desktop().unwrap_or_else(|| {
        std::env::var_os("USERPROFILE")
            .map(PathBuf::from)
            .unwrap_or_default()
            .join("Desktop")
    });
    (start, desktop)
}

/// The desktop as Windows knows it — moved to OneDrive, say.
#[allow(unsafe_code)] // A shell call, and the string it hands back freed.
fn known_desktop() -> Option<PathBuf> {
    use windows::Win32::UI::Shell::{FOLDERID_Desktop, KF_FLAG_DEFAULT, SHGetKnownFolderPath};
    unsafe {
        let path = SHGetKnownFolderPath(&FOLDERID_Desktop, KF_FLAG_DEFAULT, None).ok()?;
        let text = path.to_string().ok();
        windows::Win32::System::Com::CoTaskMemFree(Some(path.0 as *const _));
        text.map(PathBuf::from)
    }
}

/// A shortcut at `link` that opens `target`.
#[allow(unsafe_code)] // COM: Windows writes a shortcut through IShellLink.
pub fn shortcut(link: &Path, target: &Path) -> Result<(), String> {
    use windows::{
        Win32::System::Com::{
            CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
            IPersistFile,
        },
        Win32::UI::Shell::{IShellLinkW, ShellLink},
        core::{HSTRING, Interface},
    };
    if let Some(parent) = link.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let shell: IShellLinkW =
            CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).map_err(|e| e.to_string())?;
        shell
            .SetPath(&HSTRING::from(target.as_os_str()))
            .map_err(|e| e.to_string())?;
        if let Some(dir) = target.parent() {
            shell
                .SetWorkingDirectory(&HSTRING::from(dir.as_os_str()))
                .map_err(|e| e.to_string())?;
        }
        shell
            .SetIconLocation(&HSTRING::from(target.as_os_str()), 0)
            .map_err(|e| e.to_string())?;
        let file: IPersistFile = shell.cast().map_err(|e| e.to_string())?;
        file.Save(&HSTRING::from(link.as_os_str()), true)
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Wait for process `pid` to end — the app that asked for this install —
/// for at most a minute. Whether it did.
#[allow(unsafe_code)] // A process handle, opened, waited on and closed.
pub fn wait_for(pid: u32) -> bool {
    use windows::Win32::{
        Foundation::{CloseHandle, WAIT_OBJECT_0},
        System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject},
    };
    unsafe {
        let Ok(process) = OpenProcess(PROCESS_SYNCHRONIZE, false, pid) else {
            // Gone already, or never ours to wait on.
            return true;
        };
        let ended = WaitForSingleObject(process, 60_000) == WAIT_OBJECT_0;
        let _ = CloseHandle(process);
        ended
    }
}
