//! What the installer carries: the app's files, in one tar compressed whole
//! with xz and appended to the installer's own executable, with a footer
//! that says where they start.
//!
//! ```text
//! [ the installer itself ][ tar.xz of the install directory ][ length: u64 LE ][ MAGIC ]
//! ```
//!
//! **Compressed whole, not per file.** The tools carry many files that are
//! nearly each other — a gdb per chip, a ROM per revision — and LZMA over
//! the whole tar compresses them against each other: measured on the
//! bundle, 59 MB where per-file deflate made 124 and per-file xz 107.
//!
//! One file to download and run, as an NSIS installer is, with nothing for
//! a packager to get wrong between the build and the release: the release
//! workflow appends the payload (`rusty-release pack`), and the installer
//! finds it by reading its own end. The installer without a payload — its
//! first part alone — is what is left in the install directory as
//! `uninstall.exe`.

use std::{
    io::{self, Write},
    path::{Path, PathBuf},
};

/// The last eight bytes of an installer that carries a payload.
pub const MAGIC: &[u8; 8] = b"RUSTYPK2";

/// The xz preset: the strongest without `--extreme`, which costs minutes in
/// the release workflow for a percent.
const PRESET: u32 = 9;

/// The footer: the payload's length, then [`MAGIC`].
const FOOTER: usize = 16;

/// Every file under `dir` in one tar, xz-compressed, written to `out` after
/// `stub`. The number of files carried.
pub fn pack(stub: &[u8], dir: &Path, out: &mut impl Write) -> io::Result<usize> {
    let mut packed = Vec::new();
    let count = tar_xz(dir, &mut packed)?;
    out.write_all(stub)?;
    out.write_all(&packed)?;
    out.write_all(&(packed.len() as u64).to_le_bytes())?;
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

fn tar_xz(dir: &Path, sink: &mut Vec<u8>) -> io::Result<usize> {
    let files = files_under(dir)?;
    let encoder = liblzma::write::XzEncoder::new_parallel(sink, PRESET);
    let mut tar = tar::Builder::new(encoder);
    // Names, sizes and modes; no owners or times, so the same tree packs to
    // the same bytes.
    tar.mode(tar::HeaderMode::Deterministic);
    for rel in &files {
        tar.append_path_with_name(dir.join(rel), rel)?;
    }
    tar.into_inner()?.finish()?;
    Ok(files.len())
}

/// Unpack `payload` into `to`: the relative paths written, in its order. A
/// name that would land outside `to` — `..`, an absolute path — is refused,
/// not written somewhere else.
pub fn unpack(payload: &[u8], to: &Path) -> io::Result<Vec<String>> {
    unpack_counted(payload, to, &mut |_| {})
}

/// [`unpack`], saying how far it has got as it goes: the share of the
/// compressed payload read, 0 to 1, which is as even a measure of the work
/// as there is — the files it makes are anything from a few bytes to sixty
/// megabytes.
pub fn unpack_counted(
    payload: &[u8],
    to: &Path,
    progress: &mut dyn FnMut(f32),
) -> io::Result<Vec<String>> {
    let read = Counted {
        rest: payload,
        total: payload.len().max(1),
        progress,
    };
    let mut archive = tar::Archive::new(liblzma::read::XzDecoder::new_parallel(read));
    archive.set_preserve_permissions(true);
    let mut written = Vec::new();
    for entry in archive.entries()? {
        let mut entry = entry?;
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let rel = entry.path()?.into_owned();
        if !rel
            .components()
            .all(|part| matches!(part, std::path::Component::Normal(_)))
        {
            return Err(io::Error::other(format!(
                "the payload names a file outside the install: {}",
                rel.display()
            )));
        }
        let path = to.join(&rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        entry.unpack(&path)?;
        written.push(slashed(&rel));
    }
    Ok(written)
}

/// The payload as it is read, reporting the share read so far.
struct Counted<'a, 'p> {
    rest: &'a [u8],
    total: usize,
    progress: &'p mut dyn FnMut(f32),
}

impl io::Read for Counted<'_, '_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.rest.read(buf)?;
        (self.progress)(1.0 - self.rest.len() as f32 / self.total as f32);
        Ok(n)
    }
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
