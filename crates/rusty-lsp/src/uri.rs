//! `file://` URIs, both ways, with one decoder.
//!
//! There were three. Two decoded percent escapes into bytes and validated
//! them as UTF-8; the third pushed each byte as a `char` — Latin-1 — so a
//! rename touching `src/驱动/mod.rs` was told to open a path of mojibake,
//! failed to read it, and reported success with the file silently skipped.
//! Three decoders is three chances to disagree; this module is the one.

use std::path::Path;

/// `E:\x y\src` → `file:///E:/x%20y/src`.
pub(crate) fn path_to_uri(path: &Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    let mut uri = String::from("file://");
    if !text.starts_with('/') {
        uri.push('/');
    }
    for ch in text.chars() {
        match ch {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' | '/' | ':' => uri.push(ch),
            _ => {
                let mut buffer = [0u8; 4];
                for byte in ch.encode_utf8(&mut buffer).bytes() {
                    uri.push_str(&format!("%{byte:02X}"));
                }
            }
        }
    }
    uri
}

/// Percent escapes undone, as UTF-8. `None` for a truncated escape, a
/// non-hex digit, or bytes that do not form text — a path this process could
/// not open anyway, and better refused here than mangled into one it can.
fn percent_decode(text: &str) -> Option<String> {
    let mut bytes = Vec::with_capacity(text.len());
    let mut input = text.bytes();
    while let Some(byte) = input.next() {
        if byte == b'%' {
            let high = (input.next()? as char).to_digit(16)?;
            let low = (input.next()? as char).to_digit(16)?;
            bytes.push((high * 16 + low) as u8);
        } else {
            bytes.push(byte);
        }
    }
    String::from_utf8(bytes).ok()
}

/// A `file://` URI as an absolute path, `/`-separated.
///
/// `file:///E:/x` → `E:/x`; `file:///home/x` stays `/home/x`. Anything that
/// is not a file URI is `None` — rust-analyzer never sends one, and a
/// caller that gets `None` for an `https:` URI has something to say about it.
pub(crate) fn uri_to_absolute(uri: &str) -> Option<String> {
    let rest = uri.strip_prefix("file://")?;
    let mut decoded = percent_decode(rest)?;
    // `/E:/x` → `E:/x` on Windows.
    if decoded.len() >= 3 && decoded.as_bytes()[0] == b'/' && decoded.as_bytes()[2] == b':' {
        decoded.remove(0);
    }
    Some(decoded)
}

/// A `file://` URI as a path relative to `root`, or `None` if it is elsewhere
/// — a dependency's source, or the root itself.
///
/// Tolerant on purpose: rust-analyzer sends `file:///e%3A/...` — lowercased
/// drive, percent-encoded colon — for the same file this side calls
/// `file:///E:/...`. Case folds ASCII only, byte for byte, so the length of
/// the folded root is the length of the real one; `to_lowercase` on a path
/// with a non-ASCII letter can change its byte length, and slicing the
/// original by the folded length would cut a character in half.
///
/// A prefix is not a directory: `E:/proj` must not claim `E:/proj2/src/x.rs`.
/// The character after the root has to be the separator, or the file is
/// somebody else's.
pub(crate) fn uri_to_relative(uri: &str, root: &Path) -> Option<String> {
    let decoded = uri_to_absolute(uri)?;
    let root = root.to_string_lossy().replace('\\', "/");
    let root = root.trim_end_matches('/');
    let (head, tail) = decoded.split_at_checked(root.len())?;
    if !same_path_text(head, root) {
        return None;
    }
    let relative = tail.strip_prefix('/')?.trim_start_matches('/');
    (!relative.is_empty()).then(|| relative.to_string())
}

/// Whether two spellings name the same path on this platform's disk.
///
/// On Windows the whole path is case-insensitive. Elsewhere only a drive
/// letter folds — a path that has one is a Windows path whatever host is
/// reading it, and rust-analyzer's `e%3A` for this side's `E:` is a fact
/// about that path, not about the machine. The tests feed real Windows
/// sessions' spellings on every OS, and `cfg!(windows)` alone made the
/// Linux runner the one place they failed.
fn same_path_text(a: &str, b: &str) -> bool {
    if cfg!(windows) {
        return a.eq_ignore_ascii_case(b);
    }
    match (drive_split(a), drive_split(b)) {
        (Some((a_drive, a_rest)), Some((b_drive, b_rest))) => {
            a_drive.eq_ignore_ascii_case(b_drive) && a_rest == b_rest
        }
        _ => a == b,
    }
}

/// `E:/x` → (`E`, `:/x`); a path with no drive letter is `None`.
fn drive_split(text: &str) -> Option<(&str, &str)> {
    let bytes = text.as_bytes();
    (bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':')
        .then(|| text.split_at(1))
}

/// Whether two file URIs name the same file, tolerating the one difference
/// Windows manufactures: rust-analyzer answers with a lowercase drive letter
/// (`file:///e:/…`) where this client builds an uppercase one. Everything
/// after the drive is compared exactly — only the drive letter is
/// case-insensitive on disk. Without this, every code action's edit looked
/// like it belonged to a different file and was dropped as multi-file.
pub(crate) fn same_file_uri(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    let (Some(a_rest), Some(b_rest)) = (a.strip_prefix("file:///"), b.strip_prefix("file:///"))
    else {
        return false;
    };
    let (Some(a_drive), Some(b_drive)) = (a_rest.as_bytes().first(), b_rest.as_bytes().first())
    else {
        return false;
    };
    a_drive.is_ascii_alphabetic()
        && a_drive.eq_ignore_ascii_case(b_drive)
        && a_rest.as_bytes().get(1) == Some(&b':')
        && b_rest.as_bytes().get(1) == Some(&b':')
        && a_rest[2..] == b_rest[2..]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drive_letter_case_does_not_split_a_file_in_two() {
        assert!(same_file_uri(
            "file:///E:/proj/src/main.rs",
            "file:///e:/proj/src/main.rs",
        ));
        assert!(!same_file_uri(
            "file:///E:/proj/src/main.rs",
            "file:///e:/proj/src/lib.rs",
        ));
        // The path half stays case-sensitive — only the drive folds.
        assert!(!same_file_uri(
            "file:///E:/Proj/a.rs",
            "file:///e:/proj/a.rs"
        ));
        assert!(same_file_uri("file:///home/x/a.rs", "file:///home/x/a.rs"));
    }

    #[test]
    fn windows_uris_round_trip_through_rust_analyzers_spelling() {
        let root = Path::new(r"E:\CodeBase\proj");
        assert_eq!(path_to_uri(root), "file:///E:/CodeBase/proj");

        // The server's own spelling of a file under that root: lowercased
        // drive, colon percent-encoded.
        assert_eq!(
            uri_to_relative("file:///e%3A/CodeBase/proj/src/main.rs", root).as_deref(),
            Some("src/main.rs"),
        );
        assert_eq!(
            uri_to_relative("file:///E:/CodeBase/proj/src/main.rs", root).as_deref(),
            Some("src/main.rs"),
        );
        // A dependency's source is not in the project.
        assert_eq!(uri_to_relative("file:///E:/other/place/lib.rs", root), None);
    }

    #[test]
    fn spaces_survive_the_uri() {
        let root = Path::new(r"E:\code base\p");
        let uri = path_to_uri(root);
        assert_eq!(uri, "file:///E:/code%20base/p");
        assert_eq!(
            uri_to_relative(&format!("{uri}/src/a.rs"), root).as_deref(),
            Some("src/a.rs"),
        );
    }

    /// The decoder that pushed each escaped byte as a `char` turned a CJK
    /// directory into Latin-1 mojibake — a path that exists nowhere, so the
    /// rename skipped the file and called itself a success.
    #[test]
    fn a_cjk_path_component_decodes_as_utf8() {
        let root = Path::new(r"E:\proj");
        let uri = path_to_uri(&root.join("src").join("驱动").join("mod.rs"));
        assert_eq!(uri, "file:///E:/proj/src/%E9%A9%B1%E5%8A%A8/mod.rs");
        assert_eq!(
            uri_to_absolute(&uri).as_deref(),
            Some("E:/proj/src/驱动/mod.rs"),
        );
        assert_eq!(
            uri_to_relative(&uri, root).as_deref(),
            Some("src/驱动/mod.rs"),
        );
        // Escapes that do not form text are refused rather than mangled.
        assert_eq!(uri_to_absolute("file:///E:/x/%FF%FE.rs"), None);
        assert_eq!(
            uri_to_absolute("file:///E:/x/%4.rs"),
            None,
            "a truncated escape"
        );
        assert_eq!(uri_to_absolute("https://example.invalid/x.rs"), None);
    }

    /// `E:/proj` is a prefix of `E:/proj2/src/x.rs` and not its directory.
    /// A prefix test alone claimed the neighbour's file as `2/src/x.rs`.
    #[test]
    fn a_neighbouring_directory_with_the_same_prefix_is_not_the_project() {
        let root = Path::new(r"E:\proj");
        assert_eq!(uri_to_relative("file:///E:/proj2/src/x.rs", root), None);
        assert_eq!(
            uri_to_relative("file:///E:/proj/src/x.rs", root).as_deref(),
            Some("src/x.rs"),
        );
        // The root itself is not a file in the project.
        assert_eq!(uri_to_relative("file:///E:/proj", root), None);
        assert_eq!(uri_to_relative("file:///E:/proj/", root), None);
        // A root spelled with a trailing separator still owns its files.
        assert_eq!(
            uri_to_relative("file:///E:/proj/src/x.rs", Path::new("E:/proj/")).as_deref(),
            Some("src/x.rs"),
        );
    }

    /// The folded root must be sliced by the *original's* length. A root
    /// with a non-ASCII letter whose lowercase form is longer in bytes made
    /// the old slice land mid-character — or claim the wrong tail.
    #[test]
    fn folding_case_never_changes_where_the_root_ends() {
        // `İ` (U+0130) lowercases to `i̇`, two scalars and three bytes for
        // the original's two. ASCII folding leaves it alone, so the root's
        // length is the root's length.
        let root = Path::new("E:/İş/proj");
        let uri = path_to_uri(&root.join("src").join("a.rs"));
        assert_eq!(uri_to_relative(&uri, root).as_deref(), Some("src/a.rs"));
    }
}
