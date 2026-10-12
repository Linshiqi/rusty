//! KiCad's S-expressions: the syntax both of its files are written in.
//!
//! `.kicad_sym` and `.kicad_sch` share a reader and share nothing else —
//! one is a library of drawings, the other a sheet of placed instances and
//! wires — so the tokeniser lives here and each vocabulary lives beside the
//! thing it describes.
//!
//! Read, never trusted: the reader keeps every node it meets, including the
//! ones neither vocabulary knows, because a node a reader skips is a node a
//! writer would delete.

/// Why a file could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub detail: String,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.detail)
    }
}

impl std::error::Error for ParseError {}

/// One S-expression node.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Sx {
    List(Vec<Sx>),
    /// A bare token: a keyword, a number, `hide`.
    Atom(String),
    /// A double-quoted string, unescaped.
    Str(String),
}

impl Sx {
    pub(crate) fn head(&self) -> Option<&str> {
        match self {
            Sx::List(items) => match items.first() {
                Some(Sx::Atom(name)) => Some(name),
                _ => None,
            },
            _ => None,
        }
    }

    pub(crate) fn items(&self) -> &[Sx] {
        match self {
            Sx::List(items) => items,
            _ => &[],
        }
    }

    /// The `n`th element as text, whichever way it was written.
    pub(crate) fn text(&self, n: usize) -> Option<&str> {
        match self.items().get(n)? {
            Sx::Atom(s) | Sx::Str(s) => Some(s),
            Sx::List(_) => None,
        }
    }

    pub(crate) fn number(&self, n: usize) -> Option<f64> {
        self.text(n)?.parse().ok()
    }

    /// The first child list headed `name`.
    pub(crate) fn child(&self, name: &str) -> Option<&Sx> {
        self.items().iter().find(|item| item.head() == Some(name))
    }

    pub(crate) fn children<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Sx> + 'a {
        self.items()
            .iter()
            .filter(move |item| item.head() == Some(name))
    }
}

/// Tokenise and nest the whole text. Strings keep their escapes resolved;
/// everything else is an atom.
pub(crate) fn read(text: &str) -> Result<Sx, ParseError> {
    let mut stack: Vec<Vec<Sx>> = vec![Vec::new()];
    let mut chars = text.char_indices().peekable();
    let mut line = 1usize;
    while let Some((_, c)) = chars.next() {
        match c {
            '\n' => line += 1,
            c if c.is_whitespace() => {}
            '(' => stack.push(Vec::new()),
            ')' => {
                let list = stack.pop().ok_or_else(|| ParseError {
                    detail: format!("line {line}: a `)` with nothing open"),
                })?;
                match stack.last_mut() {
                    Some(parent) => parent.push(Sx::List(list)),
                    None => {
                        return Err(ParseError {
                            detail: format!("line {line}: a `)` closing the file itself"),
                        });
                    }
                }
            }
            '"' => {
                let mut s = String::new();
                loop {
                    match chars.next() {
                        Some((_, '"')) => break,
                        Some((_, '\\')) => match chars.next() {
                            Some((_, 'n')) => s.push('\n'),
                            Some((_, 't')) => s.push('\t'),
                            Some((_, other)) => s.push(other),
                            None => break,
                        },
                        Some((_, '\n')) => {
                            line += 1;
                            s.push('\n');
                        }
                        Some((_, other)) => s.push(other),
                        None => {
                            return Err(ParseError {
                                detail: format!("line {line}: an unterminated string"),
                            });
                        }
                    }
                }
                stack
                    .last_mut()
                    .expect("the root list is always open")
                    .push(Sx::Str(s));
            }
            first => {
                let mut atom = String::from(first);
                while let Some((_, next)) = chars.peek() {
                    if next.is_whitespace() || *next == '(' || *next == ')' {
                        break;
                    }
                    atom.push(*next);
                    chars.next();
                }
                stack
                    .last_mut()
                    .expect("the root list is always open")
                    .push(Sx::Atom(atom));
            }
        }
    }
    if stack.len() != 1 {
        return Err(ParseError {
            detail: format!("{} list(s) never closed", stack.len() - 1),
        });
    }
    Ok(Sx::List(stack.pop().unwrap_or_default()))
}
