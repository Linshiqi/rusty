//! A row's tokens as an expression.
//!
//! ```text
//! row      := name '=' expr | expr | (nothing, or a # remark)
//! expr     := product (('+' | '-') product)*
//! product  := unary (('*' | '/') unary)*
//! unary    := '-' unary | power
//! power    := postfix ('^' unary)?
//! postfix  := primary ('°' | 'deg' | 'rad' | '.' field)*
//! primary  := number | "text" | name | name '(' args ')' | '(' expr (',' expr)* ')'
//! ```

use super::lex::{Spanned, Token, tokens};
use super::value::Problem;

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Number(f64),
    Text(String),
    Name(String),
    Call {
        name: String,
        args: Vec<Expr>,
    },
    /// Two or three numbers in brackets: a vector.
    Tuple(Vec<Expr>),
    Neg(Box<Expr>),
    Binary {
        op: Op,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    Field {
        of: Box<Expr>,
        field: String,
    },
    /// `30°`, `30 deg`: a number of degrees, as an angle.
    Degrees(Box<Expr>),
    /// `0.5 rad`: a number of radians, marked as an angle.
    Radians(Box<Expr>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Add,
    Sub,
    Mul,
    Div,
    Pow,
}

impl Op {
    pub fn symbol(self) -> char {
        match self {
            Op::Add => '+',
            Op::Sub => '-',
            Op::Mul => '*',
            Op::Div => '/',
            Op::Pow => '^',
        }
    }
}

/// One row of a sheet, read.
#[derive(Debug, Clone, PartialEq)]
pub enum Line {
    /// Nothing, or only a remark.
    Blank,
    /// An expression whose value is shown and not named.
    Bare(Expr),
    Named {
        name: String,
        expr: Expr,
    },
}

pub fn line(text: &str) -> Result<Line, Problem> {
    let tokens = tokens(text)?;
    if tokens.is_empty() {
        return Ok(Line::Blank);
    }
    let named = match tokens.as_slice() {
        [
            Spanned {
                token: Token::Ident(name),
                ..
            },
            Spanned {
                token: Token::Equals,
                ..
            },
            ..,
        ] => Some(name.clone()),
        _ => None,
    };
    let mut parser = Parser {
        tokens,
        pos: if named.is_some() { 2 } else { 0 },
    };
    if parser.peek().is_none() {
        return Err(Problem::Incomplete);
    }
    let expr = parser.expr()?;
    if let Some(stray) = parser.tokens.get(parser.pos) {
        return Err(Problem::Unexpected {
            found: describe(&stray.token),
            at: stray.at,
        });
    }
    Ok(match named {
        Some(name) => Line::Named { name, expr },
        None => Line::Bare(expr),
    })
}

/// The characters a token was written as, near enough to point at.
fn describe(token: &Token) -> String {
    match token {
        Token::Number(v) => v.to_string(),
        Token::Ident(name) => name.clone(),
        Token::Str(text) => format!("\"{text}\""),
        Token::Degree => "°".into(),
        Token::Plus => "+".into(),
        Token::Minus => "-".into(),
        Token::Star => "*".into(),
        Token::Slash => "/".into(),
        Token::Caret => "^".into(),
        Token::Open => "(".into(),
        Token::Close => ")".into(),
        Token::Comma => ",".into(),
        Token::Dot => ".".into(),
        Token::Equals => "=".into(),
    }
}

struct Parser {
    tokens: Vec<Spanned>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos).map(|s| &s.token)
    }

    fn peek_at(&self, ahead: usize) -> Option<&Token> {
        self.tokens.get(self.pos + ahead).map(|s| &s.token)
    }

    fn bump(&mut self) -> Option<Spanned> {
        let token = self.tokens.get(self.pos).cloned();
        self.pos += 1;
        token
    }

    fn expr(&mut self) -> Result<Expr, Problem> {
        let mut left = self.product()?;
        loop {
            let op = match self.peek() {
                Some(Token::Plus) => Op::Add,
                Some(Token::Minus) => Op::Sub,
                _ => return Ok(left),
            };
            self.pos += 1;
            let right = self.product()?;
            left = Expr::Binary {
                op,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
    }

    fn product(&mut self) -> Result<Expr, Problem> {
        let mut left = self.unary()?;
        loop {
            let op = match self.peek() {
                Some(Token::Star) => Op::Mul,
                Some(Token::Slash) => Op::Div,
                _ => return Ok(left),
            };
            self.pos += 1;
            let right = self.unary()?;
            left = Expr::Binary {
                op,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
    }

    fn unary(&mut self) -> Result<Expr, Problem> {
        if self.peek() == Some(&Token::Minus) {
            self.pos += 1;
            return Ok(Expr::Neg(Box::new(self.unary()?)));
        }
        if self.peek() == Some(&Token::Plus) {
            self.pos += 1;
            return self.unary();
        }
        self.power()
    }

    fn power(&mut self) -> Result<Expr, Problem> {
        let base = self.postfix()?;
        if self.peek() == Some(&Token::Caret) {
            self.pos += 1;
            let exponent = self.unary()?;
            return Ok(Expr::Binary {
                op: Op::Pow,
                left: Box::new(base),
                right: Box::new(exponent),
            });
        }
        Ok(base)
    }

    fn postfix(&mut self) -> Result<Expr, Problem> {
        let mut expr = self.primary()?;
        loop {
            match self.peek() {
                Some(Token::Degree) => {
                    self.pos += 1;
                    expr = Expr::Degrees(Box::new(expr));
                }
                // A unit written as a word, which is only a unit when it is
                // not being called: `30 deg`, but `deg(x)` is a function.
                Some(Token::Ident(word))
                    if (word == "deg" || word == "rad")
                        && self.peek_at(1) != Some(&Token::Open) =>
                {
                    let degrees = word == "deg";
                    self.pos += 1;
                    expr = if degrees {
                        Expr::Degrees(Box::new(expr))
                    } else {
                        Expr::Radians(Box::new(expr))
                    };
                }
                Some(Token::Dot) => {
                    self.pos += 1;
                    match self.bump() {
                        Some(Spanned {
                            token: Token::Ident(field),
                            ..
                        }) => {
                            expr = Expr::Field {
                                of: Box::new(expr),
                                field,
                            };
                        }
                        Some(other) => {
                            return Err(Problem::Unexpected {
                                found: describe(&other.token),
                                at: other.at,
                            });
                        }
                        None => return Err(Problem::Incomplete),
                    }
                }
                _ => return Ok(expr),
            }
        }
    }

    fn primary(&mut self) -> Result<Expr, Problem> {
        let Some(Spanned { token, at }) = self.bump() else {
            return Err(Problem::Incomplete);
        };
        match token {
            Token::Number(v) => Ok(Expr::Number(v)),
            Token::Str(text) => Ok(Expr::Text(text)),
            Token::Ident(name) => {
                if self.peek() == Some(&Token::Open) {
                    let open = self.bump().map_or(at, |s| s.at);
                    let args = self.list(open)?;
                    Ok(Expr::Call { name, args })
                } else {
                    Ok(Expr::Name(name))
                }
            }
            Token::Open => {
                let items = self.list(at)?;
                match items.len() {
                    0 => Err(Problem::Incomplete),
                    1 => Ok(items.into_iter().next().expect("one item")),
                    _ => Ok(Expr::Tuple(items)),
                }
            }
            other => Err(Problem::Unexpected {
                found: describe(&other),
                at,
            }),
        }
    }

    /// Expressions separated by commas up to the bracket that closes the one
    /// opened at `open`.
    fn list(&mut self, open: usize) -> Result<Vec<Expr>, Problem> {
        let mut items = Vec::new();
        if self.peek() == Some(&Token::Close) {
            self.pos += 1;
            return Ok(items);
        }
        loop {
            if self.peek().is_none() {
                return Err(Problem::Unclosed { at: open });
            }
            items.push(self.expr()?);
            match self.bump() {
                Some(Spanned {
                    token: Token::Comma,
                    ..
                }) => continue,
                Some(Spanned {
                    token: Token::Close,
                    ..
                }) => return Ok(items),
                Some(other) => {
                    return Err(Problem::Unexpected {
                        found: describe(&other.token),
                        at: other.at,
                    });
                }
                None => return Err(Problem::Unclosed { at: open }),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(v: f64) -> Expr {
        Expr::Number(v)
    }

    fn name(s: &str) -> Expr {
        Expr::Name(s.into())
    }

    fn bin(op: Op, l: Expr, r: Expr) -> Expr {
        Expr::Binary {
            op,
            left: Box::new(l),
            right: Box::new(r),
        }
    }

    fn expr(text: &str) -> Expr {
        match line(text).unwrap() {
            Line::Bare(e) | Line::Named { expr: e, .. } => e,
            Line::Blank => panic!("blank"),
        }
    }

    #[test]
    fn a_named_row_and_a_bare_one() {
        assert_eq!(
            line("q = a * b").unwrap(),
            Line::Named {
                name: "q".into(),
                expr: bin(Op::Mul, name("a"), name("b"))
            }
        );
        assert_eq!(
            line("a + 1").unwrap(),
            Line::Bare(bin(Op::Add, name("a"), n(1.0)))
        );
        assert_eq!(line("   # a remark").unwrap(), Line::Blank);
        assert_eq!(line("q = "), Err(Problem::Incomplete));
    }

    #[test]
    fn precedence_is_arithmetic_s() {
        // 1 + 2 * 3 ^ 2 = 1 + (2 * (3 ^ 2))
        assert_eq!(
            expr("1 + 2 * 3 ^ 2"),
            bin(
                Op::Add,
                n(1.0),
                bin(Op::Mul, n(2.0), bin(Op::Pow, n(3.0), n(2.0)))
            )
        );
        // −x² is −(x²), and a − b − c is (a − b) − c.
        assert_eq!(
            expr("-x^2"),
            Expr::Neg(Box::new(bin(Op::Pow, name("x"), n(2.0))))
        );
        assert_eq!(
            expr("a - b - c"),
            bin(Op::Sub, bin(Op::Sub, name("a"), name("b")), name("c"))
        );
    }

    #[test]
    fn units_calls_tuples_and_fields() {
        assert_eq!(expr("30°"), Expr::Degrees(Box::new(n(30.0))));
        assert_eq!(expr("30 deg"), Expr::Degrees(Box::new(n(30.0))));
        assert_eq!(expr("0.5rad"), Expr::Radians(Box::new(n(0.5))));
        assert_eq!(
            expr("deg(x)"),
            Expr::Call {
                name: "deg".into(),
                args: vec![name("x")]
            }
        );
        assert_eq!(expr("(1, 2, 3)"), Expr::Tuple(vec![n(1.0), n(2.0), n(3.0)]));
        assert_eq!(expr("(1 + 2)"), bin(Op::Add, n(1.0), n(2.0)));
        assert_eq!(
            expr("to_euler(q).roll"),
            Expr::Field {
                of: Box::new(Expr::Call {
                    name: "to_euler".into(),
                    args: vec![name("q")]
                }),
                field: "roll".into()
            }
        );
        assert_eq!(
            expr("truth()"),
            Expr::Call {
                name: "truth".into(),
                args: vec![]
            }
        );
    }

    #[test]
    fn a_broken_row_says_where() {
        assert_eq!(line("euler(1, 2"), Err(Problem::Unclosed { at: 5 }));
        assert_eq!(
            line("a b"),
            Err(Problem::Unexpected {
                found: "b".into(),
                at: 2
            })
        );
        assert_eq!(
            line("(1, 2))"),
            Err(Problem::Unexpected {
                found: ")".into(),
                at: 6
            })
        );
        assert_eq!(line("a +"), Err(Problem::Incomplete));
        assert_eq!(line("()"), Err(Problem::Incomplete));
    }
}
