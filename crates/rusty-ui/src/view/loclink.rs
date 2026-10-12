//! Source locations inside log lines.
//!
//! Cargo writes ` --> src\bin\main.rs:62:33`; panics write
//! `src/main.rs:12:5`; both deserve to be a click, not a retype. The scanner
//! is hand-rolled rather than a regex because this crate compiles to wasm and
//! a regex engine is a lot of binary for one pattern.

/// A run of log text: either plain, or a location the editor can jump to.
#[derive(Debug, PartialEq, Eq)]
pub enum Piece {
    Text(String),
    /// `path` is project-relative with forward slashes — the identity the
    /// tree and the tabs use. `line`/`col` are one-based as printed;
    /// `open_at` wants them zero-based, so the caller subtracts.
    Loc {
        display: String,
        path: String,
        line: u32,
        col: u32,
    },
}

/// True for the characters a relative path is made of. Space is excluded on
/// purpose: cargo never prints paths with spaces unquoted, and including it
/// would swallow the words before a match.
fn is_path_char(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-' | b'/' | b'\\')
}

/// Split one log line into text and clickable locations.
///
/// A location is `<path>.rs:<line>` with an optional `:<col>`, where the path
/// is relative — absolute paths (a drive letter, a leading slash) stay text,
/// because they point outside the project and `open_file` could not serve
/// them anyway.
pub fn split_locations(text: &str) -> Vec<Piece> {
    let bytes = text.as_bytes();
    let mut pieces = Vec::new();
    let mut plain_from = 0;
    let mut cursor = 0;

    while let Some(found) = text[cursor..].find(".rs:") {
        let dot = cursor + found;

        // Walk left over path characters to find where the path starts.
        let mut start = dot;
        while start > 0 && is_path_char(bytes[start - 1]) {
            start -= 1;
        }

        // Digits after ".rs:" — no digits, no location.
        let mut pos = dot + ".rs:".len();
        let line_start = pos;
        while pos < bytes.len() && bytes[pos].is_ascii_digit() {
            pos += 1;
        }
        if pos == line_start {
            cursor = dot + ".rs:".len();
            continue;
        }
        let line: u32 = text[line_start..pos].parse().unwrap_or(0);

        // An optional `:col`.
        let mut col = 0u32;
        let mut end = pos;
        if end < bytes.len() && bytes[end] == b':' {
            let col_start = end + 1;
            let mut col_end = col_start;
            while col_end < bytes.len() && bytes[col_end].is_ascii_digit() {
                col_end += 1;
            }
            if col_end > col_start {
                col = text[col_start..col_end].parse().unwrap_or(0);
                end = col_end;
            }
        }

        let raw_path = &text[start..dot + ".rs".len()];
        // Absolute paths point outside the project and stay text. `:` is not
        // a path character, so `C:\…` walks back only to the backslash — the
        // leading-separator test catches every absolute spelling, drive
        // letters and UNC included.
        let absolute = raw_path.starts_with('/') || raw_path.starts_with('\\');
        if absolute || line == 0 || start == dot {
            cursor = dot + ".rs:".len();
            continue;
        }

        if start > plain_from {
            pieces.push(Piece::Text(text[plain_from..start].to_string()));
        }
        pieces.push(Piece::Loc {
            display: text[start..end].to_string(),
            path: raw_path.replace('\\', "/"),
            line,
            col,
        });
        plain_from = end;
        cursor = end;
    }

    if plain_from < text.len() {
        pieces.push(Piece::Text(text[plain_from..].to_string()));
    }
    pieces
}

#[cfg(test)]
mod tests {
    use super::*;

    fn loc(display: &str, path: &str, line: u32, col: u32) -> Piece {
        Piece::Loc {
            display: display.to_string(),
            path: path.to_string(),
            line,
            col,
        }
    }

    #[test]
    fn cargo_arrow_line_with_backslashes_becomes_a_link() {
        // Verbatim from a Windows build — the exact line the panel shows.
        let pieces = split_locations("  --> src\\bin\\main.rs:62:33");
        assert_eq!(
            pieces,
            vec![
                Piece::Text("  --> ".to_string()),
                loc("src\\bin\\main.rs:62:33", "src/bin/main.rs", 62, 33),
            ]
        );
    }

    #[test]
    fn a_location_without_a_column_still_links() {
        let pieces = split_locations("note: src/main.rs:12 has the answer");
        assert_eq!(
            pieces,
            vec![
                Piece::Text("note: ".to_string()),
                loc("src/main.rs:12", "src/main.rs", 12, 0),
                Piece::Text(" has the answer".to_string()),
            ]
        );
    }

    #[test]
    fn absolute_paths_stay_text() {
        // Registry sources are outside the project; a click could not open them.
        let text = "at C:\\Users\\x\\.cargo\\registry\\src\\esp-hal-1.0.0\\src\\gpio.rs:100:5";
        assert_eq!(split_locations(text), vec![Piece::Text(text.to_string())]);
        let unix = "at /home/x/.cargo/registry/src/gpio.rs:9:1";
        assert_eq!(split_locations(unix), vec![Piece::Text(unix.to_string())]);
    }

    #[test]
    fn a_bare_rs_mention_without_digits_stays_text() {
        let text = "opened main.rs: syntax ok";
        assert_eq!(split_locations(text), vec![Piece::Text(text.to_string())]);
    }

    #[test]
    fn two_locations_in_one_line_both_link() {
        let pieces = split_locations("src/a.rs:1:2 and src/b.rs:3:4");
        assert_eq!(
            pieces,
            vec![
                loc("src/a.rs:1:2", "src/a.rs", 1, 2),
                Piece::Text(" and ".to_string()),
                loc("src/b.rs:3:4", "src/b.rs", 3, 4),
            ]
        );
    }

    #[test]
    fn plain_lines_come_back_whole() {
        let text = "Compiling blinky v0.1.0 (E:\\embeded\\blinky)";
        assert_eq!(split_locations(text), vec![Piece::Text(text.to_string())]);
    }
}
