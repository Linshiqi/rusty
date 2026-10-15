//! What the installer carries: the app's files, zipped and appended to the
//! installer's own executable, with a footer that says where they start.
//!
//! ```text
//! [ the installer itself ][ zip of the install directory ][ length: u64 LE ][ MAGIC ]
//! ```
//!
//! One file to download and run, as an NSIS installer is, with nothing for
//! a packager to get wrong between the build and the release: the release
//! workflow appends the payload (`rusty-release pack`), and the installer
//! finds it by reading its own end. The installer without a payload — its
//! first part alone — is what is left in the install directory as
//! `uninstall.exe`.

use std::{
    io::{self, Read, Seek, Write},
    path::{Path, PathBuf},
};

/// The last eight bytes of an installer that carries a payload.
pub const MAGIC: &[u8; 8] = b"RUSTYPK1";

/// The footer: the payload's length, then [`MAGIC`].
const FOOTER: usize = 16;

/// Zip every file under `dir` and write `stub` with it appended to `out`.
/// The number of files carried.
pub fn pack(stub: &[u8], dir: &Path, out: &mut impl Write) -> io::Result<usize> {
    let mut zip = Vec::new();
    let count = zip_dir(dir, io::Cursor::new(&mut zip))?;
    out.write_all(stub)?;
    out.write_all(&zip)?;
    out.write_all(&(zip.len() as u64).to_le_bytes())?;
    out.write_all(MAGIC)?;
    Ok(count)
}

/// The installer's own part and the payload after it, when `exe` carries
/// one.
pub fn find(exe: &[u8]) -> Option<(&[u8], &[u8])> {
    let footer = exe.len().checked_sub(FOOTER)?;
    if &exe[footer + 8..] != MAGIC {
        return None;
    }
    let length = u64::from_le_bytes(exe[footer..footer + 8].try_into().ok()?) as usize;
    let start = footer.checked_sub(length)?;
    Some((&exe[..start], &exe[start..footer]))
}

/// Every file under `dir`, relative to it with `/` between the parts, in
/// a stable order.
pub fn files_under(dir: &Path) -> io::Result<Vec<String>> {
    let mut out = Vec::new();
    let mut stack = vec![PathBuf::new()];
    while let Some(rel) = stack.pop() {
        for entry in std::fs::read_dir(dir.join(&rel))? {
            let entry = entry?;
            let rel = rel.join(entry.file_name());
            if entry.file_type()?.is_dir() {
                stack.push(rel);
            } else {
                out.push(slashed(&rel));
            }
        }
    }
    out.sort();
    Ok(out)
}

fn slashed(rel: &Path) -> String {
    rel.components()
        .map(|part| part.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn zip_dir(dir: &Path, sink: impl Write + Seek) -> io::Result<usize> {
    let mut writer = zip::ZipWriter::new(sink);
    let files = files_under(dir)?;
    for rel in &files {
        let mut options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .large_file(true);
        // Kept executable where that is a file's mode.
        if let Some(mode) = mode_of(&dir.join(rel)) {
            options = options.unix_permissions(mode);
        }
        writer
            .start_file(rel.as_str(), options)
            .map_err(io::Error::other)?;
        let mut file = std::fs::File::open(dir.join(rel))?;
        io::copy(&mut file, &mut writer)?;
    }
    writer.finish().map_err(io::Error::other)?;
    Ok(files.len())
}

#[cfg(unix)]
fn mode_of(path: &Path) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).ok().map(|m| m.permissions().mode())
}

#[cfg(not(unix))]
fn mode_of(_: &Path) -> Option<u32> {
    None
}

/// Unpack `zip` into `to`: the relative paths written, in its order. A
/// name that would land outside `to` — `..`, an absolute path — is refused,
/// not written somewhere else.
pub fn unpack(zip: &[u8], to: &Path) -> io::Result<Vec<String>> {
    let mut archive = zip::ZipArchive::new(io::Cursor::new(zip)).map_err(io::Error::other)?;
    let mut written = Vec::new();
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(io::Error::other)?;
        if entry.is_dir() {
            continue;
        }
        let Some(rel) = entry.enclosed_name() else {
            return Err(io::Error::other(format!(
                "the payload names a file outside the install: {}",
                entry.name()
            )));
        };
        let path = to.join(&rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut bytes = Vec::with_capacity(entry.size() as usize);
        entry.read_to_end(&mut bytes)?;
        std::fs::write(&path, bytes)?;
        #[cfg(unix)]
        if let Some(mode) = entry.unix_mode() {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode))?;
        }
        written.push(slashed(&rel));
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rusty-setup-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_packed_installer_gives_back_its_own_part_and_every_file() {
        let from = scratch("from");
        std::fs::write(from.join("rusty.exe"), b"the app").unwrap();
        std::fs::create_dir_all(from.join("bundled/qemu/bin")).unwrap();
        std::fs::write(from.join("bundled/qemu/bin/qemu.exe"), vec![7u8; 100_000]).unwrap();
        let stub = b"MZ the installer".to_vec();
        let mut packed = Vec::new();
        assert_eq!(pack(&stub, &from, &mut packed).unwrap(), 2);

        let (own, payload) = find(&packed).expect("a payload");
        assert_eq!(
            own,
            stub.as_slice(),
            "the installer's part, for uninstall.exe"
        );
        let to = scratch("to");
        let written = unpack(payload, &to).unwrap();
        assert_eq!(written.len(), 2);
        assert_eq!(std::fs::read(to.join("rusty.exe")).unwrap(), b"the app");
        assert_eq!(
            std::fs::read(to.join("bundled/qemu/bin/qemu.exe"))
                .unwrap()
                .len(),
            100_000
        );
        assert_eq!(files_under(&to).unwrap(), files_under(&from).unwrap());
        let _ = std::fs::remove_dir_all(from);
        let _ = std::fs::remove_dir_all(to);
    }

    #[test]
    fn an_installer_without_a_payload_is_one() {
        assert!(find(b"MZ just the installer").is_none());
        assert!(find(b"").is_none());
        // A footer claiming more than the file holds is not a payload.
        let mut lying = b"MZ".to_vec();
        lying.extend_from_slice(&1_000u64.to_le_bytes());
        lying.extend_from_slice(MAGIC);
        assert!(find(&lying).is_none());
    }
}
