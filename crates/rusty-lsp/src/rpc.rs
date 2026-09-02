//! JSON-RPC over a pipe, framed the way LSP frames it.
//!
//! Each message is `Content-Length: N\r\n`, optionally more headers, a blank
//! line, then exactly N bytes of JSON. Getting the framing wrong does not fail
//! loudly — the stream just drifts and every later message parses as garbage —
//! which is why this file is small, boring, and tested.

use std::io::{BufRead, Write};

use serde_json::Value;

/// The largest frame the reader will allocate for.
///
/// A whole-project semantic-tokens reply is a few megabytes; nothing
/// rust-analyzer sends approaches this. A `Content-Length` beyond it means
/// the stream has drifted — a header parsed out of the middle of a body —
/// and the honest answer is an error, not a request for gigabytes the
/// allocator would grant and the machine would feel.
const MAX_FRAME: usize = 256 * 1024 * 1024;

pub fn write_message(writer: &mut dyn Write, body: &Value) -> std::io::Result<()> {
    let text = serde_json::to_string(body)?;
    write!(writer, "Content-Length: {}\r\n\r\n", text.len())?;
    writer.write_all(text.as_bytes())?;
    writer.flush()
}

/// The next message, or `None` on a clean end of stream.
pub fn read_message(reader: &mut impl BufRead) -> std::io::Result<Option<Value>> {
    let mut length: Option<usize> = None;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some(rest) = line.strip_prefix("Content-Length:") {
            length = rest.trim().parse().ok();
        }
        // Content-Type is the only other header the spec allows; it says
        // "utf-8" and nothing else in practice.
    }
    let Some(length) = length else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "a message frame with no Content-Length",
        ));
    };
    if length > MAX_FRAME {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("a message frame claiming {length} bytes — the stream has drifted"),
        ));
    }

    let mut buffer = vec![0u8; length];
    reader.read_exact(&mut buffer)?;
    serde_json::from_slice(&buffer)
        .map(Some)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn frames_round_trip_including_multibyte_payloads() {
        // The length is bytes, not characters. A CJK payload counted in chars
        // under-reads and every following message is misframed.
        let mut wire = Vec::new();
        for body in [json!({"id": 1}), json!({"msg": "中文", "n": 2})] {
            write_message(&mut wire, &body).unwrap();
        }

        let mut reader = std::io::BufReader::new(wire.as_slice());
        assert_eq!(read_message(&mut reader).unwrap(), Some(json!({"id": 1})));
        assert_eq!(
            read_message(&mut reader).unwrap(),
            Some(json!({"msg": "中文", "n": 2})),
        );
        assert_eq!(read_message(&mut reader).unwrap(), None, "clean EOF");
    }

    /// A header read out of the middle of a body can claim any length at
    /// all. Allocating it would be honouring garbage with gigabytes.
    #[test]
    fn an_absurd_content_length_is_an_error_not_an_allocation() {
        let wire = b"Content-Length: 99999999999\r\n\r\n{}";
        let mut reader = std::io::BufReader::new(wire.as_slice());
        let error = read_message(&mut reader).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("drifted"), "{error}");
    }
}
