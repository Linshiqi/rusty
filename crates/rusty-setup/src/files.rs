//! The install directory's files: put in place, kept track of, taken away.
//!
//! **Staged, then moved.** The payload is unpacked beside the install
//! (`.staging`) and each file moved into place, so an install that fails
//! half way has not left half of one version over the other — and a move
//! within one volume is a rename, which Windows allows of a file that is
//! open or running where it refuses a delete or an overwrite. A file the
//! old version still holds is renamed aside (`*.old`) and taken away the
//! next time the installer runs.
//!
//! **What was installed is written down** (`install.txt`), so the next
//! install removes what it no longer carries and the uninstaller removes
//! exactly what was put there — never the directory wholesale, which on a
//! machine where somebody pointed the install at a folder of their own
//! would take their files with it. An install that predates the list — the
//! Tauri installer's — is recognised by its `rusty-app.exe`, and its own
//! files are what the list would have said.

use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use crate::payload;

/// The list of what is installed, one relative path a line.
pub const MANIFEST: &str = "install.txt";
/// Where the payload is unpacked before it is moved into place.
const STAGING: &str = ".staging";

/// What the Tauri installer put in the directory, before there was a list:
/// the app, its uninstaller and the tools under `bundled/`.
pub fn tauri_leftovers(dir: &Path) -> Vec<String> {
    if !dir.join("rusty-app.exe").is_file() {
        return Vec::new();
    }
    let mut old = vec!["rusty-app.exe".to_string()];
    if let Ok(bundled) = payload::files_under(&dir.join("bundled")) {
        old.extend(bundled.into_iter().map(|rel| format!("bundled/{rel}")));
    }
    old
}

/// What an earlier install put in `dir`.
pub fn installed(dir: &Path) -> Vec<String> {
    match std::fs::read_to_string(dir.join(MANIFEST)) {
        Ok(text) => text
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_string)
            .collect(),
        Err(_) => tauri_leftovers(dir),
    }
}

/// Put the payload's files in `dir`, take away what an earlier install
/// left that this one does not carry, and write down what is there now —
/// `extra` too, the files the installer writes itself (`uninstall.exe`).
pub fn install(zip: &[u8], dir: &Path, extra: &[&str]) -> Result<Vec<String>, String> {
    let fail = |what: &str, e: std::io::Error| format!("{what}: {e}");
    std::fs::create_dir_all(dir).map_err(|e| fail(&dir.display().to_string(), e))?;
    sweep_old(dir);
    let before = installed(dir);
    let staging = dir.join(STAGING);
    let _ = std::fs::remove_dir_all(&staging);
    let carried = payload::unpack(zip, &staging).map_err(|e| fail("unpacking", e))?;
    for rel in &carried {
        let target = dir.join(rel);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|e| fail(rel, e))?;
        }
        clear(&target).map_err(|e| fail(rel, e))?;
        std::fs::rename(staging.join(rel), &target).map_err(|e| fail(rel, e))?;
    }
    let _ = std::fs::remove_dir_all(&staging);
    let mut now: Vec<String> = carried.clone();
    now.extend(extra.iter().map(|s| s.to_string()));
    for rel in before.iter().filter(|rel| !now.contains(rel)) {
        let _ = clear(&dir.join(rel));
        prune_parents(dir, rel);
    }
    now.sort();
    now.dedup();
    let list = now.join("\n") + "\n";
    std::fs::write(dir.join(MANIFEST), list).map_err(|e| fail(MANIFEST, e))?;
    Ok(now)
}

/// Take away what an install put in `dir` — its list, and the list itself
/// — leaving the uninstaller, which cannot remove itself while it runs.
/// The files that could not go, because something holds them.
pub fn uninstall(dir: &Path, keep: &str) -> Vec<String> {
    let mut held = Vec::new();
    for rel in installed(dir).iter().filter(|rel| rel.as_str() != keep) {
        if clear(&dir.join(rel)).is_err() {
            held.push(rel.clone());
        }
        prune_parents(dir, rel);
    }
    let _ = std::fs::remove_file(dir.join(MANIFEST));
    sweep_old(dir);
    held
}

/// `path` gone, so something else can take its place: deleted, and when
/// something holds it, renamed aside to be swept up next time. Retried for
/// a while first — the app the installer replaces may take a moment to go.
fn clear(path: &Path) -> std::io::Result<()> {
    if !path.exists() {
        return Ok(());
    }
    let mut last = None;
    for _ in 0..25 {
        match std::fs::remove_file(path) {
            Ok(()) => return Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => last = Some(e),
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    let aside = aside_of(path);
    let _ = std::fs::remove_file(&aside);
    std::fs::rename(path, &aside).map_err(|_| last.unwrap_or_else(|| std::io::Error::other("held")))
}

fn aside_of(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".old");
    path.with_file_name(name)
}

/// What an earlier run renamed aside, now that nothing holds it.
fn sweep_old(dir: &Path) {
    let Ok(files) = payload::files_under(dir) else {
        return;
    };
    for rel in files.iter().filter(|rel| rel.ends_with(".old")) {
        let _ = std::fs::remove_file(dir.join(rel));
        prune_parents(dir, rel);
    }
}

/// The directories `rel` was in, removed while they are empty, up to `dir`.
fn prune_parents(dir: &Path, rel: &str) {
    let mut at = dir.join(rel);
    while let Some(parent) = at.parent() {
        if parent == dir || !parent.starts_with(dir) {
            break;
        }
        if std::fs::remove_dir(parent).is_err() {
            break;
        }
        at = parent.to_path_buf();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rusty-files-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn zip_of(files: &[(&str, &[u8])]) -> Vec<u8> {
        let from = scratch(&format!("zip{}", files.len()));
        for (rel, bytes) in files {
            let path = from.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, bytes).unwrap();
        }
        let mut packed = Vec::new();
        payload::pack(b"", &from, &mut packed).unwrap();
        let _ = std::fs::remove_dir_all(from);
        payload::find(&packed).unwrap().1.to_vec()
    }

    /// Over a Tauri install: the new files in, `rusty-app.exe` and the
    /// tools it carried that this one does not gone, a list written — and
    /// a file of somebody's own in the directory left alone.
    #[test]
    fn an_install_over_the_tauri_one_takes_away_its_files_and_only_its_files() {
        let dir = scratch("over");
        std::fs::write(dir.join("rusty-app.exe"), b"tauri").unwrap();
        std::fs::create_dir_all(dir.join("bundled/old-tool")).unwrap();
        std::fs::write(dir.join("bundled/old-tool/tool.exe"), b"old").unwrap();
        std::fs::write(dir.join("bundled/espflash.exe"), b"old espflash").unwrap();
        std::fs::write(dir.join("notes.txt"), b"mine").unwrap();
        let zip = zip_of(&[("rusty.exe", b"slint"), ("bundled/espflash.exe", b"new")]);

        let now = install(&zip, &dir, &["uninstall.exe"]).unwrap();
        assert_eq!(now, ["bundled/espflash.exe", "rusty.exe", "uninstall.exe"]);
        assert_eq!(std::fs::read(dir.join("rusty.exe")).unwrap(), b"slint");
        assert_eq!(
            std::fs::read(dir.join("bundled/espflash.exe")).unwrap(),
            b"new"
        );
        assert!(!dir.join("rusty-app.exe").exists());
        assert!(
            !dir.join("bundled/old-tool").exists(),
            "an emptied directory goes"
        );
        assert!(
            dir.join("notes.txt").exists(),
            "not the installer's to take"
        );
        assert!(!dir.join(STAGING).exists());

        // The next version drops a file; the list knows it was there.
        let zip = zip_of(&[("rusty.exe", b"slint 2")]);
        install(&zip, &dir, &["uninstall.exe"]).unwrap();
        assert!(!dir.join("bundled").exists());

        std::fs::write(dir.join("uninstall.exe"), b"me").unwrap();
        assert!(uninstall(&dir, "uninstall.exe").is_empty());
        assert!(!dir.join("rusty.exe").exists());
        assert!(!dir.join(MANIFEST).exists());
        assert!(dir.join("uninstall.exe").exists(), "left for itself");
        assert!(dir.join("notes.txt").exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_fresh_directory_has_nothing_installed() {
        let dir = scratch("fresh");
        assert!(installed(&dir).is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }
}
