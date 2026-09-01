//! GDB's machine interface, parsed.
//!
//! MI rather than DAP because MI is what gdb already speaks, and gdb is what
//! both ends of this workbench already have: Espressif's QEMU exposes a
//! gdbstub, and probe-rs serves gdb for real hardware. One protocol, both
//! targets, no adapter to install.
//!
//! Compiled unconditionally — no IO here, only text — so the frontend can
//! share the types and a test can reach the arithmetic. The parser is the
//! part worth testing: MI's grammar is small but its escaping is not, and a
//! debugger that mis-parses a frame line puts the caret on the wrong
//! function while looking like it worked.

use alloc::borrow::ToOwned;
use alloc::string::String;
use alloc::vec::Vec;

/// One value in an MI record: a string, a `{…}` tuple, or a `[…]` list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    Str(String),
    Tuple(Vec<(String, Value)>),
    List(Vec<Value>),
}

impl Value {
    /// The string at `key`, for tuples. The workhorse: nearly every field
    /// this workbench reads out of MI is a string one level down.
    pub fn field(&self, key: &str) -> Option<&str> {
        match self {
            Value::Tuple(fields) => {
                fields
                    .iter()
                    .find(|(name, _)| name == key)
                    .and_then(|(_, value)| match value {
                        Value::Str(text) => Some(text.as_str()),
                        _ => None,
                    })
            }
            _ => None,
        }
    }

    /// The nested value at `key`, for reaching into `frame={…}`.
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Tuple(fields) => fields
                .iter()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value),
            _ => None,
        }
    }

    pub fn items(&self) -> &[Value] {
        match self {
            Value::List(items) => items,
            _ => &[],
        }
    }
}

/// Which stream a line of gdb's chatter came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stream {
    /// `~` — what an interactive gdb would have printed.
    Console,
    /// `@` — output from the program being debugged.
    Target,
    /// `&` — gdb's own log of the command it just ran.
    Log,
}

/// One line of MI.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Record {
    /// `^done`, `^error`, `^running`… — the answer to a command, tagged with
    /// the token the command carried so a caller can match them up.
    Result {
        token: Option<u32>,
        class: String,
        fields: Vec<(String, Value)>,
    },
    /// `*stopped`, `*running` — the target changed state on its own.
    Exec {
        class: String,
        fields: Vec<(String, Value)>,
    },
    /// `=breakpoint-modified`, `=thread-group-exited`… — gdb's own news.
    Notify {
        class: String,
        fields: Vec<(String, Value)>,
    },
    Stream {
        stream: Stream,
        text: String,
    },
    /// `(gdb)` — everything before it has arrived.
    Prompt,
}

/// Parse one line. `None` for anything that is not MI — gdb prints a banner
/// before the interface starts, and a debugger that panicked on it would be
/// a debugger that never started.
pub fn parse(line: &str) -> Option<Record> {
    let line = line.trim_end_matches(['\r', '\n']);
    if line.is_empty() {
        return None;
    }
    if line.starts_with("(gdb)") {
        return Some(Record::Prompt);
    }

    // An optional token precedes the record type: `12^done,…`.
    let digits: String = line.chars().take_while(char::is_ascii_digit).collect();
    let rest = &line[digits.len()..];
    let token = if digits.is_empty() {
        None
    } else {
        digits.parse().ok()
    };

    let (marker, body) = rest.split_at(rest.chars().next().map(char::len_utf8)?);
    match marker {
        "~" | "@" | "&" => {
            let stream = match marker {
                "~" => Stream::Console,
                "@" => Stream::Target,
                _ => Stream::Log,
            };
            let (text, _) = parse_c_string(body)?;
            Some(Record::Stream { stream, text })
        }
        "^" | "*" | "=" => {
            let (class, fields) = parse_class(body);
            Some(match marker {
                "^" => Record::Result {
                    token,
                    class,
                    fields,
                },
                "*" => Record::Exec { class, fields },
                _ => Record::Notify { class, fields },
            })
        }
        _ => None,
    }
}

/// `class,key=value,key=value` — the shape every non-stream record has.
fn parse_class(body: &str) -> (String, Vec<(String, Value)>) {
    match body.split_once(',') {
        None => (body.to_owned(), Vec::new()),
        Some((class, rest)) => (class.to_owned(), parse_fields(rest)),
    }
}

fn parse_fields(mut rest: &str) -> Vec<(String, Value)> {
    let mut out = Vec::new();
    while !rest.is_empty() {
        let Some((key, value, tail)) = parse_field(rest) else {
            break;
        };
        out.push((key, value));
        rest = tail.strip_prefix(',').unwrap_or(tail);
    }
    out
}

fn parse_field(input: &str) -> Option<(String, Value, &str)> {
    let (key, rest) = input.split_once('=')?;
    let (value, tail) = parse_value(rest)?;
    Some((key.to_owned(), value, tail))
}

fn parse_value(input: &str) -> Option<(Value, &str)> {
    match input.chars().next()? {
        '"' => {
            let (text, tail) = parse_c_string(input)?;
            Some((Value::Str(text), tail))
        }
        '{' => {
            let mut rest = &input[1..];
            let mut fields = Vec::new();
            while !rest.starts_with('}') {
                let (key, value, tail) = parse_field(rest)?;
                fields.push((key, value));
                rest = tail.strip_prefix(',').unwrap_or(tail);
            }
            Some((Value::Tuple(fields), &rest[1..]))
        }
        '[' => {
            let mut rest = &input[1..];
            let mut items = Vec::new();
            while !rest.starts_with(']') {
                // A list's elements are values — except gdb also emits
                // `[name=value,…]`, a list of fields wearing list brackets.
                //
                // Only a bare name can start a field. Trying `parse_field`
                // on `{begin="0x…"}` finds the `=` *inside* the braces and
                // silently invents a field called `{begin` — a mis-parse
                // that reads as "no memory came back" three layers away.
                let is_field = !rest.starts_with(['{', '[', '"']);
                if let Some((key, value, tail)) = is_field.then(|| parse_field(rest)).flatten() {
                    items.push(Value::Tuple(alloc::vec![(key, value)]));
                    rest = tail.strip_prefix(',').unwrap_or(tail);
                } else {
                    let (value, tail) = parse_value(rest)?;
                    items.push(value);
                    rest = tail.strip_prefix(',').unwrap_or(tail);
                }
            }
            Some((Value::List(items), &rest[1..]))
        }
        _ => None,
    }
}

/// A quoted MI string, escapes undone. Returns the text and what follows.
fn parse_c_string(input: &str) -> Option<(String, &str)> {
    let mut chars = input.char_indices();
    if chars.next()?.1 != '"' {
        return None;
    }
    let mut out = String::new();
    while let Some((index, ch)) = chars.next() {
        match ch {
            '"' => return Some((out, &input[index + 1..])),
            '\\' => {
                let (_, escaped) = chars.next()?;
                out.push(match escaped {
                    'n' => '\n',
                    'r' => '\r',
                    't' => '\t',
                    '0' => '\0',
                    other => other,
                });
            }
            other => out.push(other),
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;

    /// Real records, captured from gdb 14 against an esp32 image. The frame
    /// is the one that matters: file and line are what put the caret on a
    /// source line, and getting them from the wrong nesting level is how a
    /// debugger lands two functions away while looking like it worked.
    #[test]
    fn a_stop_carries_its_frame() {
        let line = r#"*stopped,reason="breakpoint-hit",disp="keep",bkptno="1",frame={addr="0x400d1a2c",func="blinky::main",args=[],file="src/bin/main.rs",fullname="E:\\embeded\\blinky\\src\\bin\\main.rs",line="68",arch="xtensa"},thread-id="1",stopped-threads="all""#;
        let Some(Record::Exec { class, fields }) = parse(line) else {
            panic!("not an exec record: {line}");
        };
        assert_eq!(class, "stopped");

        let value = Value::Tuple(fields);
        assert_eq!(value.field("reason"), Some("breakpoint-hit"));
        let frame = value.get("frame").expect("a frame");
        assert_eq!(frame.field("func"), Some("blinky::main"));
        assert_eq!(frame.field("line"), Some("68"));
        assert_eq!(
            frame.field("fullname"),
            Some(r"E:\embeded\blinky\src\bin\main.rs"),
            "the doubled backslashes of a Windows path are unescaped exactly once",
        );
    }

    #[test]
    fn a_breakpoint_answer_reports_where_it_landed() {
        let line = r#"3^done,bkpt={number="1",type="breakpoint",disp="keep",enabled="y",addr="0x400d1a2c",func="main",file="src/bin/main.rs",line="68",times="0"}"#;
        let Some(Record::Result {
            token,
            class,
            fields,
        }) = parse(line)
        else {
            panic!("not a result record");
        };
        assert_eq!(token, Some(3), "the token pairs an answer with its command");
        assert_eq!(class, "done");
        let value = Value::Tuple(fields);
        assert_eq!(value.get("bkpt").and_then(|b| b.field("line")), Some("68"));
    }

    #[test]
    fn a_stack_is_a_list_of_frames() {
        let line = r#"^done,stack=[frame={level="0",func="inner",line="12"},frame={level="1",func="outer",line="40"}]"#;
        let Some(Record::Result { fields, .. }) = parse(line) else {
            panic!("not a result");
        };
        let value = Value::Tuple(fields);
        let stack = value.get("stack").expect("a stack");
        let frames = stack.items();
        assert_eq!(frames.len(), 2, "both frames survive: {frames:?}");
        // Each element is `frame={…}` — a field wearing list brackets.
        let inner = frames[0].get("frame").expect("the frame tuple");
        assert_eq!(inner.field("func"), Some("inner"));
        assert_eq!(
            frames[1].get("frame").and_then(|f| f.field("level")),
            Some("1"),
        );
    }

    /// A list of bare tuples, which is how `-data-read-memory-bytes`
    /// answers. The elements start with `{`, and treating them as
    /// `name=value` finds the `=` inside the braces — inventing a field
    /// called `{begin` and losing the read.
    #[test]
    fn a_list_of_tuples_is_not_a_list_of_fields() {
        let Some(Record::Result { fields, .. }) =
            parse(r#"^done,memory=[{begin="0x3ff44004",end="0x3ff44008",contents="0400000f"}]"#)
        else {
            panic!("not a result");
        };
        let value = Value::Tuple(fields);
        let items = value.get("memory").expect("a memory list").items();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].field("begin"), Some("0x3ff44004"));
        assert_eq!(items[0].field("contents"), Some("0400000f"));
    }

    #[test]
    fn console_output_is_unescaped_text() {
        let Some(Record::Stream { stream, text }) =
            parse(r#"~"Breakpoint 1 at 0x400d1a2c: file src/bin/main.rs, line 68.\n""#)
        else {
            panic!("not a stream record");
        };
        assert_eq!(stream, Stream::Console);
        assert!(
            text.ends_with("line 68.\n"),
            "the \\n became a newline: {text:?}"
        );
    }

    #[test]
    fn an_error_says_what_gdb_said() {
        let Some(Record::Result { class, fields, .. }) =
            parse(r#"^error,msg="No symbol \"nope\" in current context.""#)
        else {
            panic!("not a result");
        };
        assert_eq!(class, "error");
        assert_eq!(
            Value::Tuple(fields).field("msg"),
            Some(r#"No symbol "nope" in current context."#),
            "an escaped quote inside the message survives",
        );
    }

    /// gdb prints a banner before the interface opens. Refusing to parse it
    /// is correct; panicking on it would mean never starting.
    #[test]
    fn non_mi_lines_are_not_records() {
        assert_eq!(parse("GNU gdb (esp-gdb) 14.2"), None);
        assert_eq!(parse(""), None);
        assert_eq!(parse("(gdb)"), Some(Record::Prompt));
        assert_eq!(parse("(gdb) "), Some(Record::Prompt));
    }

    #[test]
    fn a_class_without_fields_still_parses() {
        assert_eq!(
            parse("^running"),
            Some(Record::Result {
                token: None,
                class: "running".to_string(),
                fields: Vec::new(),
            }),
        );
    }
}
