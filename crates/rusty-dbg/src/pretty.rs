//! Making gdb's one-line values readable.
//!
//! `-stack-list-variables --all-values` prints a whole aggregate on one
//! line. For a `u32` that is perfect and for an embedded HAL handle it is
//! four hundred characters of `Uart0(Inner(PhantomData<…>))` — the row you
//! cannot read and cannot widen, which is what a variables panel is *for*.
//!
//! The structure is already there in the text, so nothing needs asking of
//! the target: this parses gdb's value syntax and lays it out, breaking a
//! group across lines only when it does not fit. Short values stay exactly
//! as they were, because `[1, 1, 1, 1, 1, 0]` on six lines is worse than on
//! one.
//!
//! Two things must stay opaque or the layout cuts them in half:
//!
//! - `<...>` — type parameters. `Instant<u64, 1, 1000000>` has commas that
//!   are not element separators.
//! - `"..."` — a `&str` local can contain any delimiter at all.

/// One value, laid out to fit `width` columns.
///
/// Returns the input unchanged when it already fits — the common case, and
/// the one where any reformatting would be noise.
pub fn pretty(value: &str, width: usize) -> String {
    if value.chars().count() <= width {
        return value.to_owned();
    }
    let source: Vec<char> = value.chars().collect();
    let mut cursor = 0;
    let items = parse(&source, &mut cursor, None);
    let mut out = String::new();
    render(&items, 0, width, &mut out);
    out
}

/// A run of characters, or a bracketed group of comma-separated items.
enum Node {
    Text(String),
    Group(char, Vec<Item>, char),
}

/// One comma-separated element: `uart: AnyUart (…)` is text then a group.
type Item = Vec<Node>;

fn parse(source: &[char], cursor: &mut usize, close: Option<char>) -> Vec<Item> {
    let mut items: Vec<Item> = Vec::new();
    let mut current: Item = Vec::new();
    let mut text = String::new();

    while *cursor < source.len() {
        let c = source[*cursor];
        if Some(c) == close {
            *cursor += 1;
            break;
        }
        match c {
            '{' | '(' | '[' => {
                flush(&mut text, &mut current);
                *cursor += 1;
                let closer = match c {
                    '{' => '}',
                    '(' => ')',
                    _ => ']',
                };
                let inner = parse(source, cursor, Some(closer));
                current.push(Node::Group(c, inner, closer));
            }
            ',' => {
                flush(&mut text, &mut current);
                *cursor += 1;
                // gdb writes ", " — the space belongs to the separator, not
                // to the next element, or every broken line starts indented
                // by one.
                if source.get(*cursor) == Some(&' ') {
                    *cursor += 1;
                }
                items.push(core::mem::take(&mut current));
            }
            '"' => {
                text.push(c);
                *cursor += 1;
                while *cursor < source.len() {
                    let d = source[*cursor];
                    text.push(d);
                    *cursor += 1;
                    if d == '\\' {
                        if let Some(escaped) = source.get(*cursor) {
                            text.push(*escaped);
                            *cursor += 1;
                        }
                    } else if d == '"' {
                        break;
                    }
                }
            }
            '<' => {
                // Type parameters, and gdb's own `<repeats 64 times>`.
                let mut depth = 0usize;
                while *cursor < source.len() {
                    let d = source[*cursor];
                    text.push(d);
                    *cursor += 1;
                    if d == '<' {
                        depth += 1;
                    } else if d == '>' {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                }
            }
            _ => {
                text.push(c);
                *cursor += 1;
            }
        }
    }

    flush(&mut text, &mut current);
    if !current.is_empty() || !items.is_empty() {
        items.push(current);
    }
    items
}

fn flush(text: &mut String, into: &mut Item) {
    if !text.is_empty() {
        into.push(Node::Text(core::mem::take(text)));
    }
}

/// The whole thing on one line — what decides whether it needs breaking.
fn flat(items: &[Item], out: &mut String) {
    for (index, item) in items.iter().enumerate() {
        if index > 0 {
            out.push_str(", ");
        }
        for node in item {
            match node {
                Node::Text(text) => out.push_str(text),
                Node::Group(open, inner, close) => {
                    out.push(*open);
                    flat(inner, out);
                    out.push(*close);
                }
            }
        }
    }
}

fn render(items: &[Item], indent: usize, width: usize, out: &mut String) {
    for (index, item) in items.iter().enumerate() {
        if index > 0 {
            out.push_str(", ");
        }
        for node in item {
            match node {
                Node::Text(text) => out.push_str(text),
                Node::Group(open, inner, close) => {
                    let mut one_line = String::new();
                    one_line.push(*open);
                    flat(inner, &mut one_line);
                    one_line.push(*close);
                    // Measured against where this group *starts*, not from
                    // column zero: a group nested four deep has far less
                    // room than the same text at the margin.
                    // In characters, like `width` and the line's own count:
                    // a byte column made a value with CJK text in it break
                    // three columns early per ideograph.
                    let line_start = out.rfind('\n').map_or(0, |at| at + 1);
                    let column = out[line_start..].chars().count();
                    if column + one_line.chars().count() <= width {
                        out.push_str(&one_line);
                        continue;
                    }
                    out.push(*open);
                    for (nth, element) in inner.iter().enumerate() {
                        out.push('\n');
                        out.push_str(&" ".repeat(indent + 2));
                        render(core::slice::from_ref(element), indent + 2, width, out);
                        if nth + 1 < inner.len() {
                            out.push(',');
                        }
                    }
                    out.push('\n');
                    out.push_str(&" ".repeat(indent));
                    out.push(*close);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The common case, and the one where reformatting would be damage.
    #[test]
    fn a_value_that_already_fits_is_returned_untouched() {
        let short = "[1, 1, 1, 1, 1, 0]";
        assert_eq!(pretty(short, 80), short);
        assert_eq!(pretty("42", 80), "42");
        assert_eq!(pretty("false", 80), "false");
    }

    /// The row from the screenshot that started this. Every field of the
    /// handle has to end up on its own line, at increasing indents.
    #[test]
    fn a_hal_handle_breaks_into_its_fields() {
        let value = "{uart: esp_hal::uart::AnyUart (esp_hal::uart::any::Inner::Uart0(\
                     esp_hal::peripherals::UART0 {_marker: core::marker::PhantomData})), \
                     phantom: core::marker::PhantomData, guard: esp_hal::system::\
                     PeripheralGuard {peripheral: esp_hal::system::Peripheral::Uart0}}";
        let laid_out = pretty(value, 60);

        assert!(
            laid_out.lines().count() > 3,
            "a value four times the width has to break: {laid_out}",
        );
        assert!(
            laid_out.contains("\n  uart: "),
            "each field starts a line at one indent: {laid_out}",
        );
        assert!(
            laid_out.contains("\n  phantom: ") && laid_out.contains("\n  guard: "),
            "all three of them, not just the first: {laid_out}",
        );
        // Layout may add newlines and indentation; it may not lose or invent
        // anything else.
        let flattened: String = laid_out.chars().filter(|c| !c.is_whitespace()).collect();
        let original: String = value.chars().filter(|c| !c.is_whitespace()).collect();
        assert_eq!(flattened, original, "nothing added, nothing dropped");
    }

    /// The trap this had to be written around: `Instant<u64, 1, 1000000>`
    /// has commas that are type parameters, and breaking there produces
    /// something that reads like three separate values.
    #[test]
    fn commas_inside_type_parameters_are_not_element_separators() {
        let value = format!(
            "{{first: esp_hal::time::Instant (fugit::instant::Instant<u64, 1, 1000000> \
             {{ticks: 25005}}), second: {}}}",
            "x".repeat(80),
        );
        let laid_out = pretty(&value, 60);

        assert!(
            laid_out.contains("Instant<u64, 1, 1000000>"),
            "the type parameters stay on one line: {laid_out}",
        );
        assert!(
            laid_out.contains("\n  first: ") && laid_out.contains("\n  second: "),
            "while the fields around them do break: {laid_out}",
        );
    }

    /// A `&str` local can hold any delimiter there is.
    #[test]
    fn a_quoted_string_is_opaque() {
        let value = format!("{{name: \"a, b {{c}} <d>\", padding: {}}}", "y".repeat(80),);
        let laid_out = pretty(&value, 40);
        assert!(
            laid_out.contains("\"a, b {c} <d>\""),
            "the string survives intact: {laid_out}",
        );
    }

    /// A group that fits stays inline even when its parent had to break —
    /// otherwise every leaf ends up on its own line and the shape is lost.
    #[test]
    fn an_inner_group_that_fits_stays_on_one_line() {
        let value = format!("{{small: {{a: 1, b: 2}}, big: {}}}", "z".repeat(90),);
        let laid_out = pretty(&value, 50);
        assert!(
            laid_out.contains("small: {a: 1, b: 2}"),
            "the small struct is not exploded: {laid_out}",
        );
    }
}
