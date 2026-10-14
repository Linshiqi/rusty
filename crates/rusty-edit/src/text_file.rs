//! A file's text as an editor keeps it: in the encoding it was written in,
//! with the line endings it was written with, both kept for the save.
//!
//! The editor holds text in UTF-8 with `\n` between lines, because every
//! piece of arithmetic in it — a caret, a selection, a painting, a language
//! server's position — counts that way. A file is not always that: a vendor
//! SDK's C carries GBK comments, a Windows checkout CRLF. Read, the file is
//! turned into the editor's form and what it was is remembered
//! ([`TextFile`]); saved, it is turned back ([`save_text`]), so opening a
//! GBK, CRLF file and changing one line changes one line on disk.
//!
//! Which encoding, decided in this order: a byte-order mark says so (UTF-8,
//! UTF-16 either way round); text that is valid UTF-8 is UTF-8; text that
//! decodes as GB18030 without a single replacement is GBK's family — what a
//! Chinese Windows tool writes; anything else is not text this editor will
//! guess at. A NUL in what decoded is binary, as it always was.

use std::path::Path;

use encoding_rs::{Encoding, GB18030, UTF_16BE, UTF_16LE};

use crate::{
    document::resolve,
    error::{Error, Result},
};

/// Files bigger than this are refused rather than read: the editor's limit,
/// [`crate::Files::open`]'s.
pub const MAX_BYTES: u64 = 2 * 1024 * 1024;

/// What a file was written as, for writing it back the same way.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Charset {
    Utf8,
    /// UTF-8 with the byte-order mark Windows tools put at the front.
    Utf8Bom,
    Utf16Le,
    Utf16Be,
    /// GBK, read and written as GB18030, its superset.
    Gbk,
}

impl Charset {
    /// How the status bar names it.
    pub fn label(self) -> &'static str {
        match self {
            Charset::Utf8 => "UTF-8",
            Charset::Utf8Bom => "UTF-8 BOM",
            Charset::Utf16Le => "UTF-16 LE",
            Charset::Utf16Be => "UTF-16 BE",
            Charset::Gbk => "GBK",
        }
    }

    /// The charset a [`Charset::label`] names.
    pub fn from_label(label: &str) -> Option<Charset> {
        [
            Charset::Utf8,
            Charset::Utf8Bom,
            Charset::Utf16Le,
            Charset::Utf16Be,
            Charset::Gbk,
        ]
        .into_iter()
        .find(|charset| charset.label() == label)
    }
}

/// `\n` or `\r\n` between lines.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineEnding {
    Lf,
    Crlf,
}

impl LineEnding {
    pub fn label(self) -> &'static str {
        match self {
            LineEnding::Lf => "LF",
            LineEnding::Crlf => "CRLF",
        }
    }

    pub fn from_label(label: &str) -> Option<LineEnding> {
        [LineEnding::Lf, LineEnding::Crlf]
            .into_iter()
            .find(|ending| ending.label() == label)
    }
}

/// A file read for editing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TextFile {
    Text {
        /// UTF-8, `\n` between lines.
        text: String,
        charset: Charset,
        ending: LineEnding,
    },
    /// Not text this editor will show: a NUL byte, or bytes no encoding
    /// above reads without a guess.
    Binary,
    /// Text, perhaps, but over [`MAX_BYTES`].
    TooLarge,
}

/// Read a file under `root` for editing.
pub fn read_text(root: &Path, relative: &str) -> Result<TextFile> {
    let path = resolve(root, relative)?;
    let read = |source| Error::Read {
        path: relative.to_string(),
        source,
    };
    let metadata = std::fs::metadata(&path).map_err(read)?;
    if metadata.len() > MAX_BYTES {
        return Ok(TextFile::TooLarge);
    }
    let bytes = std::fs::read(&path).map_err(read)?;
    Ok(decode(&bytes))
}

/// The pure half of [`read_text`].
pub fn decode(bytes: &[u8]) -> TextFile {
    let (text, charset) = if let Some(rest) = bytes.strip_prefix(b"\xEF\xBB\xBF") {
        match std::str::from_utf8(rest) {
            Ok(text) => (text.to_string(), Charset::Utf8Bom),
            Err(_) => return TextFile::Binary,
        }
    } else if bytes.starts_with(b"\xFF\xFE") {
        match whole(UTF_16LE, &bytes[2..]) {
            Some(text) => (text, Charset::Utf16Le),
            None => return TextFile::Binary,
        }
    } else if bytes.starts_with(b"\xFE\xFF") {
        match whole(UTF_16BE, &bytes[2..]) {
            Some(text) => (text, Charset::Utf16Be),
            None => return TextFile::Binary,
        }
    } else if let Ok(text) = std::str::from_utf8(bytes) {
        (text.to_string(), Charset::Utf8)
    } else {
        match whole(GB18030, bytes) {
            Some(text) => (text, Charset::Gbk),
            None => return TextFile::Binary,
        }
    };
    if text.contains('\0') {
        return TextFile::Binary;
    }
    // CRLF when most breaks are: a file with one stray LF among CRLFs is
    // still a CRLF file, and is written back as one.
    let breaks = text.matches('\n').count();
    let crlf = text.matches("\r\n").count();
    let (text, ending) = if crlf > 0 && crlf * 2 >= breaks {
        (text.replace("\r\n", "\n"), LineEnding::Crlf)
    } else {
        (text, LineEnding::Lf)
    };
    TextFile::Text {
        text,
        charset,
        ending,
    }
}

/// `bytes` decoded whole by `encoding`, or `None` when any of it needed a
/// replacement character — a guess is not a reading.
fn whole(encoding: &'static Encoding, bytes: &[u8]) -> Option<String> {
    let (text, had_errors) = encoding.decode_without_bom_handling(bytes);
    (!had_errors).then(|| text.into_owned())
}

/// The bytes `text` is written as, with `charset` and `ending`. Every
/// charset here holds every character: GB18030, which GBK is written as,
/// encodes all of Unicode.
pub fn encode(text: &str, charset: Charset, ending: LineEnding) -> Vec<u8> {
    let text = match ending {
        LineEnding::Lf => std::borrow::Cow::Borrowed(text),
        LineEnding::Crlf => std::borrow::Cow::Owned(text.replace('\n', "\r\n")),
    };
    match charset {
        Charset::Utf8 => text.into_owned().into_bytes(),
        Charset::Utf8Bom => [&b"\xEF\xBB\xBF"[..], text.as_bytes()].concat(),
        Charset::Utf16Le => {
            let mut out = vec![0xFF, 0xFE];
            for unit in text.encode_utf16() {
                out.extend_from_slice(&unit.to_le_bytes());
            }
            out
        }
        Charset::Utf16Be => {
            let mut out = vec![0xFE, 0xFF];
            for unit in text.encode_utf16() {
                out.extend_from_slice(&unit.to_be_bytes());
            }
            out
        }
        Charset::Gbk => GB18030.encode(&text).0.into_owned(),
    }
}

/// Write `text` to a file under `root` as `charset` with `ending`.
pub fn save_text(
    root: &Path,
    relative: &str,
    text: &str,
    charset: Charset,
    ending: LineEnding,
) -> Result<()> {
    let path = resolve(root, relative)?;
    std::fs::write(&path, encode(text, charset, ending)).map_err(|source| Error::Write {
        path: relative.to_string(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_of(file: TextFile) -> (String, Charset, LineEnding) {
        match file {
            TextFile::Text {
                text,
                charset,
                ending,
            } => (text, charset, ending),
            other => panic!("not text: {other:?}"),
        }
    }

    /// The report this exists for: a GBK C file from a vendor SDK, read as
    /// binary and refused.
    #[test]
    fn a_gbk_file_is_read_and_written_back_byte_for_byte() {
        let source = "/* 初始化时钟 */\r\nvoid init(void);\r\n";
        let (bytes, _, _) = GB18030.encode(source);
        let (text, charset, ending) = text_of(decode(&bytes));
        assert_eq!(charset, Charset::Gbk);
        assert_eq!(ending, LineEnding::Crlf);
        assert_eq!(text, "/* 初始化时钟 */\nvoid init(void);\n");
        assert_eq!(encode(&text, charset, ending), bytes.to_vec());
    }

    #[test]
    fn utf8_stays_utf8_and_a_bom_is_kept() {
        let (text, charset, ending) = text_of(decode("fn main() {}\n".as_bytes()));
        assert_eq!((charset, ending), (Charset::Utf8, LineEnding::Lf));
        assert_eq!(encode(&text, charset, ending), b"fn main() {}\n");
        let bom = b"\xEF\xBB\xBFfn a() {}\r\n";
        let (text, charset, ending) = text_of(decode(bom));
        assert_eq!(charset, Charset::Utf8Bom);
        assert_eq!(encode(&text, charset, ending), bom.to_vec());
    }

    #[test]
    fn utf16_round_trips_both_ways_round() {
        for charset in [Charset::Utf16Le, Charset::Utf16Be] {
            let bytes = encode("中文\nok", charset, LineEnding::Lf);
            let (text, read, _) = text_of(decode(&bytes));
            assert_eq!((text.as_str(), read), ("中文\nok", charset));
        }
    }

    /// A stray LF among CRLFs is still a CRLF file; mostly LF is LF, and its
    /// lone CR stays in the text rather than being invented away.
    #[test]
    fn the_line_ending_is_the_one_most_breaks_use() {
        let (text, _, ending) = text_of(decode(b"a\r\nb\nc\r\n"));
        assert_eq!(ending, LineEnding::Crlf);
        assert_eq!(text, "a\nb\nc\n");
        let (text, _, ending) = text_of(decode(b"a\nb\nc\r\n"));
        assert_eq!(ending, LineEnding::Lf);
        assert_eq!(text, "a\nb\nc\r\n");
    }

    #[test]
    fn a_nul_or_bytes_nothing_reads_are_binary() {
        assert_eq!(decode(b"ELF\0\x01\x02"), TextFile::Binary);
        assert_eq!(decode(&[b'a', 0xFF, 0xFF, 0xFF]), TextFile::Binary);
    }
}
