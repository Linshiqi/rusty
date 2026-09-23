//! Turning source into semantic runs.
//!
//! syntect is used for its grammars, not its themes. A theme is a fixed palette,
//! and baking one in would paint a light-theme window with dark-theme colours —
//! so what travels is what a run *means* and the stylesheet decides how it
//! looks, exactly as the terminal's indexed colours do.
//!
//! That means parsing to scopes rather than styles: `ParseState` yields scope
//! stack operations, and the top of the stack at each byte is what the grammar
//! thinks that byte is.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;

use syntect::parsing::{ParseState, Scope, ScopeStack, SyntaxSet};

use crate::model::{Line, Span, Token};

/// A fenced code block longer than this is shown plain past the cut. A page
/// is read, not edited, and nobody puts a register map in a README.
const SNIPPET_LINES: usize = 5_000;

/// A line longer than this is painted plain, and the parser goes on past it
/// as though it were not there — VS Code's `maxTokenizationLineLength`, for
/// its reason: one minified line can hold a grammar's regexes for seconds,
/// and colour on a line nobody can read buys nothing.
const LONGEST_PAINTED_LINE: usize = 20_000;

/// What paints a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Grammar {
    /// A syntect grammar, by its index in the set.
    Syntax(usize),
    /// syntect's bundled grammars have no TOML, and TOML is what an embedded
    /// project is configured with: Cargo.toml, .cargo/config.toml,
    /// rust-toolchain.toml. Rust in colour beside those in flat grey reads as
    /// broken, so the one gap that matters here is filled by hand.
    Toml,
    /// Nothing matched: shown unstyled rather than guessed at.
    Plain,
}

/// The grammar for `path`, by its extension.
fn grammar_for(syntaxes: &SyntaxSet, path: &str) -> Grammar {
    let extension = path.rsplit('.').next().unwrap_or_default();
    // Cargo.lock is TOML that does not say so in its name — and it is a
    // file people actually open, where all-grey next to coloured Rust reads
    // as broken highlighting rather than as a plain file.
    if extension.eq_ignore_ascii_case("toml") || extension.eq_ignore_ascii_case("lock") {
        return Grammar::Toml;
    }
    // syntect hands `.h` to Objective-C, which claims the extension. In a
    // firmware project a header is C — the vendor SDK's, or the one
    // cbindgen just wrote — and Objective-C's grammar colours it wrong in
    // ways that read as a broken highlighter.
    let syntax = if extension == "h" {
        syntaxes.find_syntax_by_name("C")
    } else {
        None
    }
    .or_else(|| syntaxes.find_syntax_by_extension(extension))
    .or_else(|| syntaxes.find_syntax_by_name(extension));
    syntax
        .and_then(|found| {
            syntaxes
                .syntaxes()
                .iter()
                .position(|each| std::ptr::eq(each, found))
        })
        .map_or(Grammar::Plain, Grammar::Syntax)
}

/// The parser between two lines. Shared, because most lines leave it exactly
/// as they found it, and a copy per line of a state that never moved would be
/// most of what a painting weighs.
type Between = Arc<(ParseState, ScopeStack)>;

/// A file's painting, kept so that the next edit repaints what it changed
/// rather than the file.
///
/// A grammar's state at a line is whatever every line above it left, so
/// painting a line means scanning from the top. Painting a whole file after
/// every pause in typing measured 250 ms for 5,000 lines of Rust — which is
/// why files used to stop being coloured, and start being read-only, at
/// 5,000 lines. So the parser is kept as it stood between every two lines.
/// [`Painting::repaint`] starts at the first line an edit touched, with the
/// parser as it stood there, and stops at the first line below the edit where
/// the parser stands exactly as it stood before: from there down, the text
/// and the parser are both what they were, so the painting is too. Typing on
/// a line repaints that line; opening a block comment repaints as far as the
/// comment now reaches, which is what it changed.
pub struct Painting {
    grammar: Grammar,
    /// The text as last painted, a line each. Split at `\n`, the way the
    /// editor counts, so a text ending in a newline has an empty last line —
    /// the one the caret stands on after it — and a `\r` stays on its line.
    lines: Vec<String>,
    /// The parser before each line, and after the last: one longer than
    /// `lines`. Empty for a grammar whose lines carry nothing to the next.
    between: Vec<Between>,
    /// What each scope reads as. [`token_for`] builds a string per scope, and
    /// a file pushes a few hundred distinct scopes several hundred thousand
    /// times.
    tokens: HashMap<Scope, Option<Token>>,
}

/// What a repaint changed: the lines from `from` on, painted for the text
/// that was asked about. Every other line paints as it did — moved, below
/// these, by however many lines the edit added or removed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repainted {
    pub from: usize,
    pub lines: Vec<Line>,
}

impl Painting {
    /// Paint `text` whole.
    pub fn new(syntaxes: &SyntaxSet, path: &str, text: &str) -> (Painting, Vec<Line>) {
        let mut painting = Painting::blank(syntaxes, path);
        let lines = painting.repaint(syntaxes, text, 0..0).lines;
        (painting, lines)
    }

    /// A painting of no lines, which a repaint turns into one of all of them.
    pub fn blank(syntaxes: &SyntaxSet, path: &str) -> Painting {
        let grammar = grammar_for(syntaxes, path);
        let between = match grammar {
            Grammar::Syntax(index) => vec![Arc::new((
                ParseState::new(&syntaxes.syntaxes()[index]),
                ScopeStack::new(),
            ))],
            Grammar::Toml | Grammar::Plain => Vec::new(),
        };
        Painting {
            grammar,
            lines: Vec::new(),
            between,
            tokens: HashMap::new(),
        }
    }

    /// The grammar's name — `None` for a file shown plain.
    pub fn language(&self, syntaxes: &SyntaxSet) -> Option<String> {
        match self.grammar {
            Grammar::Syntax(index) => Some(syntaxes.syntaxes()[index].name.clone()),
            Grammar::Toml => Some("TOML".into()),
            Grammar::Plain => None,
        }
    }

    /// Repaint for `text`, and keep what the scan leaves for the next one.
    ///
    /// `stale` names lines of `text` to paint whether or not they changed:
    /// the editor shows the lines it edits plain until they are repainted, so
    /// a line typed on and put back as it was is unchanged here and still
    /// plain there. The part of it past the end of `text` asks for nothing.
    pub fn repaint(&mut self, syntaxes: &SyntaxSet, text: &str, stale: Range<usize>) -> Repainted {
        let new: Vec<&str> = text.split('\n').collect();
        let (old_len, new_len) = (self.lines.len(), new.len());
        let prefix = self
            .lines
            .iter()
            .zip(&new)
            .take_while(|(old, new)| old == new)
            .count();
        let suffix = self.lines[prefix..]
            .iter()
            .rev()
            .zip(new[prefix..].iter().rev())
            .take_while(|(old, new)| old == new)
            .count();
        // The first line of the tail the edit did not touch. Every line from
        // here down is the old line `old_len - new_len` further down. A text
        // put back exactly as it was touched nothing, and is all tail.
        let tail = if old_len == new_len && prefix == new_len {
            0
        } else {
            new_len - suffix
        };
        let stale = stale.start.min(new_len)..stale.end.min(new_len);
        let (from, must) = if stale.is_empty() {
            (prefix, tail)
        } else {
            (prefix.min(stale.start), tail.max(stale.end))
        };
        let must = must.max(from);

        let (lines, stop) = match self.grammar {
            Grammar::Syntax(_) => {
                let mut scan = Scan::new(syntaxes, &mut self.tokens, &self.between[from]);
                let mut lines = Vec::new();
                let mut after = Vec::new();
                let mut at = from;
                while at < new_len {
                    if at >= must {
                        let before = &self.between[at + old_len - new_len];
                        if Arc::ptr_eq(&scan.shared, before) || *scan.shared == **before {
                            break;
                        }
                    }
                    let (line, state) = scan.line(new[at]);
                    lines.push(line);
                    after.push(state);
                    at += 1;
                }
                let old_stop = at + old_len - new_len;
                self.between.splice(from + 1..old_stop + 1, after);
                (lines, at)
            }
            Grammar::Toml | Grammar::Plain => {
                let paint = if self.grammar == Grammar::Toml {
                    toml_line
                } else {
                    plain_line
                };
                let lines = new[from..must]
                    .iter()
                    .map(|line| paint(line.strip_suffix('\r').unwrap_or(line)))
                    .collect();
                (lines, must)
            }
        };
        let old_stop = stop + old_len - new_len;
        self.lines.splice(
            from..old_stop,
            new[from..stop].iter().map(|line| line.to_string()),
        );
        Repainted { from, lines }
    }
}

/// Highlight a fenced code block by the language its fence names.
///
/// `rust`, `toml`, `bash`, `py`, `c` — an info string is resolved the way a
/// Markdown renderer resolves one: by extension first, then by name without
/// regard to case (syntect's `find_syntax_by_token`, written for exactly
/// this). A block with no language, or one no grammar answers to, comes back
/// plain rather than guessed at: the page shows it as it was written.
pub fn snippet(syntaxes: &SyntaxSet, lang: &str, text: &str) -> Vec<Line> {
    let lang = lang.trim();
    let source = text.lines().take(SNIPPET_LINES);
    if lang.eq_ignore_ascii_case("toml") {
        return source.map(toml_line).collect();
    }
    match syntaxes.find_syntax_by_token(lang) {
        Some(syntax) if !lang.is_empty() => {
            let mut tokens = HashMap::new();
            let start = Arc::new((ParseState::new(syntax), ScopeStack::new()));
            let mut scan = Scan::new(syntaxes, &mut tokens, &start);
            source.map(|line| scan.line(line).0).collect()
        }
        _ => source.map(plain_line).collect(),
    }
}

/// A line as one unstyled run.
fn plain_line(line: &str) -> Line {
    Line {
        spans: vec![Span {
            text: line.to_string(),
            token: Token::Plain,
        }],
    }
}

/// A grammar's scan in progress, a line at a time, carrying the parse state
/// across lines as syntect requires.
struct Scan<'a> {
    syntaxes: &'a SyntaxSet,
    tokens: &'a mut HashMap<Scope, Option<Token>>,
    state: ParseState,
    stack: ScopeStack,
    /// The parser as last handed out. The next line most likely leaves it as
    /// it is, and then this is handed out again rather than copied.
    shared: Between,
}

impl<'a> Scan<'a> {
    fn new(
        syntaxes: &'a SyntaxSet,
        tokens: &'a mut HashMap<Scope, Option<Token>>,
        before: &Between,
    ) -> Scan<'a> {
        Scan {
            syntaxes,
            tokens,
            state: before.0.clone(),
            stack: before.1.clone(),
            shared: before.clone(),
        }
    }

    /// Paint one line, and say where it leaves the parser.
    fn line(&mut self, line: &str) -> (Line, Between) {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.len() > LONGEST_PAINTED_LINE {
            return (plain_line(line), self.shared.clone());
        }
        // syntect wants the newline: several grammars end a construct on it,
        // and without it a line comment never closes.
        let owned = format!("{line}\n");
        let ops = self
            .state
            .parse_line(&owned, self.syntaxes)
            .unwrap_or_default();

        let mut spans: Vec<Span> = Vec::new();
        let mut at = 0usize;
        for (offset, op) in ops {
            let text = &owned[at..offset.min(owned.len())];
            push(&mut spans, text, &self.stack, self.tokens);
            let _ = self.stack.apply(&op);
            at = offset;
        }
        push(
            &mut spans,
            &owned[at.min(owned.len())..],
            &self.stack,
            self.tokens,
        );

        // The newline was only for the parser.
        if let Some(last) = spans.last_mut() {
            while last.text.ends_with('\n') || last.text.ends_with('\r') {
                last.text.pop();
            }
            if last.text.is_empty() {
                spans.pop();
            }
        }
        if self.shared.0 != self.state || self.shared.1 != self.stack {
            self.shared = Arc::new((self.state.clone(), self.stack.clone()));
        }

        // syntect styles declarations and leaves most expressions plain —
        // `TimerGroup::new(x)` came back one white run. The lexical pass
        // splits those by the conventions the language enforces anyway.
        let line = Line {
            spans: crate::lexical::refine(spans),
        };
        (line, self.shared.clone())
    }
}

/// One line of TOML.
///
/// Deliberately shallow — it colours comments, table headers, keys, strings,
/// numbers and booleans, and nothing else. TOML's remaining subtleties (nested
/// inline tables, multi-line literals) would show as a slightly plain line
/// rather than a wrong one, which is the right way for a fallback to fail.
fn toml_line(line: &str) -> Line {
    let mut spans = Vec::new();
    let trimmed = line.trim_start();
    let indent = &line[..line.len() - trimmed.len()];

    if !indent.is_empty() {
        spans.push(Span {
            text: indent.to_string(),
            token: Token::Plain,
        });
    }

    if trimmed.starts_with('#') {
        spans.push(Span {
            text: trimmed.to_string(),
            token: Token::Comment,
        });
        return Line { spans };
    }
    if trimmed.starts_with('[') {
        spans.push(Span {
            text: trimmed.to_string(),
            token: Token::Type,
        });
        return Line { spans };
    }

    // `key = value`. Everything after the first `=` is the value.
    let Some((key, value)) = trimmed.split_once('=') else {
        spans.push(Span {
            text: trimmed.to_string(),
            token: Token::Plain,
        });
        return Line { spans };
    };

    spans.push(Span {
        text: key.to_string(),
        token: Token::Variable,
    });
    spans.push(Span {
        text: "=".into(),
        token: Token::Punctuation,
    });

    let mut rest = value;
    // A trailing comment is a comment wherever it appears — but only outside a
    // string, or a `#` in a URL would swallow the rest of the line. The
    // string is closed by the quote that opened it and nothing else: an
    // apostrophe inside `"it's"` used to flip the state, and the comment
    // after it was painted as string.
    let comment_at = {
        let mut in_string: Option<char> = None;
        let mut found = None;
        for (index, ch) in rest.char_indices() {
            match (in_string, ch) {
                (Some(open), close) if close == open => in_string = None,
                (Some(_), _) => {}
                (None, '"' | '\'') => in_string = Some(ch),
                (None, '#') => {
                    found = Some(index);
                    break;
                }
                (None, _) => {}
            }
        }
        found
    };
    let comment = comment_at.map(|at| {
        let (head, tail) = rest.split_at(at);
        rest = head;
        tail.to_string()
    });

    let token = match rest.trim() {
        v if v.starts_with('"') || v.starts_with('\'') => Token::Str,
        "true" | "false" => Token::Keyword,
        v if v.starts_with(|c: char| c.is_ascii_digit()) => Token::Number,
        _ => Token::Plain,
    };
    spans.push(Span {
        text: rest.to_string(),
        token,
    });

    if let Some(comment) = comment {
        spans.push(Span {
            text: comment,
            token: Token::Comment,
        });
    }

    Line { spans }
}

/// Append text, merging into the previous run when it means the same thing.
fn push(
    spans: &mut Vec<Span>,
    text: &str,
    stack: &ScopeStack,
    tokens: &mut HashMap<Scope, Option<Token>>,
) {
    if text.is_empty() {
        return;
    }
    let token = classify(stack, tokens);
    match spans.last_mut() {
        Some(last) if last.token == token => last.text.push_str(text),
        _ => spans.push(Span {
            text: text.to_string(),
            token,
        }),
    }
}

/// What the grammar thinks this is.
///
/// Read from the top of the stack down, because the most specific scope is the
/// one that matters: `meta.function.rust entity.name.function.rust` is a
/// function name, and stopping at `meta.function` would paint the whole body.
fn classify(stack: &ScopeStack, tokens: &mut HashMap<Scope, Option<Token>>) -> Token {
    for scope in stack.as_slice().iter().rev() {
        if let Some(token) = *tokens.entry(*scope).or_insert_with(|| token_for(*scope)) {
            return token;
        }
    }
    Token::Plain
}

fn token_for(scope: Scope) -> Option<Token> {
    // Sublime scope names are dotted and hierarchical, so prefix matching is
    // how they are meant to be read.
    let name = scope.build_string();
    let kind = match () {
        _ if name.starts_with("comment") => Token::Comment,
        _ if name.starts_with("string") => Token::Str,
        _ if name.starts_with("constant.numeric") => Token::Number,
        _ if name.starts_with("constant.character.escape") => Token::Str,
        // Attributes and macros before the general keyword rule, which would
        // otherwise swallow `#[derive]`.
        _ if name.starts_with("meta.annotation") || name.starts_with("meta.attribute") => {
            Token::Macro
        }
        _ if name.contains("macro") => Token::Macro,
        _ if name.starts_with("keyword") || name.starts_with("storage") => Token::Keyword,
        _ if name.starts_with("entity.name.function") || name.starts_with("support.function") => {
            Token::Function
        }
        _ if name.starts_with("entity.name.type")
            || name.starts_with("entity.name.class")
            || name.starts_with("entity.name.struct")
            || name.starts_with("entity.name.enum")
            || name.starts_with("entity.name.trait")
            || name.starts_with("support.type") =>
        {
            Token::Type
        }
        _ if name.starts_with("variable") => Token::Variable,
        _ if name.starts_with("constant") => Token::Number,
        _ if name.starts_with("punctuation") => Token::Punctuation,
        _ => return None,
    };
    Some(kind)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A whole painting and the grammar's name, as opening a file makes them.
    fn lines(syntaxes: &SyntaxSet, path: &str, text: &str) -> (Vec<Line>, Option<String>) {
        let (painting, lines) = Painting::new(syntaxes, path, text);
        (lines, painting.language(syntaxes))
    }

    fn source() -> String {
        [
            "// a comment",
            "use core::fmt;",
            "",
            "pub struct Point {",
            "    pub x: i32,",
            "    pub y: i32,",
            "}",
            "",
            "impl Point {",
            "    pub fn new(x: i32, y: i32) -> Self {",
            "        let s = \"text\";",
            "        Point { x, y }",
            "    }",
            "}",
            "",
        ]
        .join("\n")
    }

    /// What the editor does with a repaint: the new lines in, the rest of
    /// what it held kept, and the tail moved by the lines the edit added.
    fn applied(held: &[Line], old_len: usize, new_len: usize, repainted: Repainted) -> Vec<Line> {
        let stop = repainted.from + repainted.lines.len();
        let mut out = held[..repainted.from].to_vec();
        out.extend(repainted.lines);
        out.extend_from_slice(&held[stop + old_len - new_len..]);
        out
    }

    /// The property the whole design stands on: an edit repainted from where
    /// it starts to where the parser falls back into step paints exactly what
    /// painting the new text from the top would — through lines added and
    /// removed, a comment opened and closed further down, a string left open,
    /// every line ending changed, and the text emptied and filled again.
    #[test]
    fn a_repainted_edit_paints_what_painting_the_whole_text_would() {
        let syntaxes = SyntaxSet::load_defaults_newlines();
        let start = source();
        let opened = start.replace("// a comment", "/* a comment");
        let edits = [
            start.replace("i32, y", "i64, y"),
            start.replace("use core::fmt;\n", "use core::fmt;\nuse core::ops;\n"),
            start.replace("    pub y: i32,\n", ""),
            opened.clone(),
            opened.replace("}\n\nimpl", "}\n*/\nimpl"),
            start.replace("\"text\"", "\"text"),
            format!("{start}fn tail() {{}}"),
            start.trim_end().to_string(),
            start.replace('\n', "\r\n"),
            String::new(),
            start.clone(),
        ];
        let (mut painting, mut held) = Painting::new(&syntaxes, "src/lib.rs", &start);
        let mut text = start.clone();
        for next in edits {
            let repainted = painting.repaint(&syntaxes, &next, 0..0);
            let (old_len, new_len) = (text.split('\n').count(), next.split('\n').count());
            held = applied(&held, old_len, new_len, repainted);
            let (_, whole) = Painting::new(&syntaxes, "src/lib.rs", &next);
            assert_eq!(held, whole, "after the edit to {next:?}");
            text = next;
        }
    }

    /// Why the parser is kept at all: typing on one line of a long file
    /// repaints that line, and opening a comment repaints everything the
    /// comment now covers — to the end of the file, in this case.
    #[test]
    fn a_repaint_costs_the_lines_an_edit_changed_not_the_file() {
        let syntaxes = SyntaxSet::load_defaults_newlines();
        let text = source().repeat(200);
        let (mut painting, _) = Painting::new(&syntaxes, "src/lib.rs", &text);

        let typed = text.replacen("i32, y", "i64, y", 1);
        let repainted = painting.repaint(&syntaxes, &typed, 0..0);
        assert_eq!((repainted.from, repainted.lines.len()), (9, 1));

        let opened = typed.replacen("// a comment", "/* a comment", 1);
        let repainted = painting.repaint(&syntaxes, &opened, 0..0);
        assert_eq!(
            (repainted.from, repainted.lines.len()),
            (0, opened.split('\n').count())
        );
    }

    /// The editor shows the lines it edits plain until they come back, and a
    /// letter typed and deleted leaves the text as it was with the line still
    /// plain on screen — so lines it names are painted, changed or not, and
    /// only those.
    #[test]
    fn stale_lines_are_painted_though_nothing_changed() {
        let syntaxes = SyntaxSet::load_defaults_newlines();
        let text = source();
        for path in ["src/lib.rs", "Cargo.toml", "notes.zzz"] {
            let (mut painting, whole) = Painting::new(&syntaxes, path, &text);
            let repainted = painting.repaint(&syntaxes, &text, 4..6);
            assert_eq!(
                repainted,
                Repainted {
                    from: 4,
                    lines: whole[4..6].to_vec()
                },
                "{path}"
            );
            for stale in [0..0, 100..120] {
                let nothing = painting.repaint(&syntaxes, &text, stale);
                assert!(nothing.lines.is_empty(), "{path}: {nothing:?}");
            }
        }
    }

    /// A minified line is not parsed, and the parser carries on past it as
    /// though it were not there: the line after it is still Rust.
    #[test]
    fn a_line_too_long_to_paint_is_plain_and_skipped_by_the_parser() {
        let syntaxes = SyntaxSet::load_defaults_newlines();
        let long = format!("let s = \"{}", "x".repeat(LONGEST_PAINTED_LINE));
        let text = format!("fn a() {{}}\n{long}\nfn b() {{}}");
        let (lines, _) = lines(&syntaxes, "main.rs", &text);
        assert_eq!(lines[1].spans.len(), 1);
        assert_eq!(lines[1].spans[0].token, Token::Plain);
        assert_eq!(
            lines[2]
                .spans
                .first()
                .map(|span| (span.text.as_str(), span.token)),
            Some(("fn", Token::Keyword)),
            "{:?}",
            lines[2]
        );
    }

    /// A fence's info string is a language, not a path: `rust` has no
    /// extension called `rust`, and `Rust` is how the grammar spells itself.
    /// Both must land on the Rust grammar, TOML on the hand-rolled one, and
    /// a language nobody has a grammar for on plain text — never a guess.
    #[test]
    fn a_fence_names_its_language_the_way_a_markdown_renderer_reads_it() {
        let syntaxes = SyntaxSet::load_defaults_newlines();
        for lang in ["rust", "Rust", "rs"] {
            let lines = snippet(&syntaxes, lang, "fn main() {}\n");
            let keyword = lines[0]
                .spans
                .iter()
                .find(|span| span.token == Token::Keyword)
                .unwrap_or_else(|| panic!("{lang}: no keyword in {:?}", lines[0].spans));
            assert_eq!(keyword.text, "fn", "{lang}");
        }
        let toml = snippet(&syntaxes, "toml", "name = \"demo\"\n");
        assert!(
            toml[0]
                .spans
                .iter()
                .any(|span| span.token == Token::Variable && span.text == "name "),
            "{:?}",
            toml[0].spans
        );
        for lang in ["", "nonesuch"] {
            let plain = snippet(&syntaxes, lang, "let x = 1;\n");
            assert_eq!(plain[0].spans.len(), 1, "{lang:?}: {:?}", plain[0].spans);
            assert_eq!(plain[0].spans[0].token, Token::Plain, "{lang:?}");
        }
    }

    /// A whole real book, when `RUSTY_MD_CORPUS` names its `src/`: every
    /// chapter highlights in well under a second. Markdown grammars have
    /// pathological inputs — a long line of pipes, a run of asterisks — and
    /// a window that opens a chapter and stops answering is what one looks
    /// like from the outside. Skipped, and said so, without a corpus.
    #[test]
    fn a_book_corpus_highlights_in_bounded_time() {
        let Ok(dir) = std::env::var("RUSTY_MD_CORPUS") else {
            eprintln!("skipping: RUSTY_MD_CORPUS is not set");
            return;
        };
        let syntaxes = SyntaxSet::load_defaults_newlines();
        for entry in std::fs::read_dir(&dir).expect("the corpus directory") {
            let path = entry.expect("an entry").path();
            if path.extension().is_none_or(|e| e != "md") {
                continue;
            }
            let text = std::fs::read_to_string(&path).expect("a readable chapter");
            let start = std::time::Instant::now();
            let (out, _) = lines(&syntaxes, &path.to_string_lossy(), &text);
            let took = start.elapsed();
            eprintln!(
                "{}: {} lines, {} bytes, {took:?}",
                path.file_name().unwrap_or_default().to_string_lossy(),
                out.len(),
                text.len()
            );
            assert!(took.as_secs() < 2, "{} took {took:?}", path.display());
        }
    }

    /// A header in a firmware project is C. syntect gives `.h` to
    /// Objective-C, whose grammar colours `struct` and `#define` wrongly —
    /// which reads as a broken highlighter, not as a misfiled grammar.
    #[test]
    fn a_header_is_c_not_objective_c() {
        let syntaxes = SyntaxSet::load_defaults_newlines();
        let (_, language) = lines(
            &syntaxes,
            "include/blinky.h",
            "#define LED 26
void blinky_tick(void);
",
        );
        assert_eq!(language.as_deref(), Some("C"));
    }

    fn tokens(path: &str, source: &str) -> Vec<(String, Token)> {
        let syntaxes = SyntaxSet::load_defaults_newlines();
        let (lines, _) = lines(&syntaxes, path, source);
        lines
            .into_iter()
            .flat_map(|line| line.spans)
            .map(|span| (span.text, span.token))
            .collect()
    }

    fn token_of(spans: &[(String, Token)], needle: &str) -> Option<Token> {
        spans
            .iter()
            .find(|(text, _)| text.trim() == needle)
            .map(|(_, token)| *token)
    }

    #[test]
    fn rust_keywords_strings_and_comments_are_told_apart() {
        let spans = tokens("main.rs", "// hi\nfn main() { let s = \"x\"; }\n");

        assert!(
            spans
                .iter()
                .any(|(t, k)| t.contains("hi") && *k == Token::Comment),
            "the comment must be a comment: {spans:?}",
        );
        assert_eq!(token_of(&spans, "fn"), Some(Token::Keyword));
        assert_eq!(token_of(&spans, "main"), Some(Token::Function));
        assert!(
            spans
                .iter()
                .any(|(t, k)| t.contains('x') && *k == Token::Str),
            "the string literal must be a string: {spans:?}",
        );
    }

    /// The trailing newline is fed to the parser but must not reach the view,
    /// or every line gains a blank cell and the gutter stops lining up. What
    /// follows the last newline is a line of its own — empty, and the one the
    /// caret stands on — so a painting has as many lines as the textarea.
    #[test]
    fn the_newline_fed_to_the_parser_is_not_returned() {
        let syntaxes = SyntaxSet::load_defaults_newlines();
        let (lines, _) = lines(&syntaxes, "main.rs", "fn a() {}\nfn b() {}\n");

        assert_eq!(lines.len(), 3);
        assert!(lines[2].spans.is_empty(), "{:?}", lines[2]);
        for line in &lines {
            for span in &line.spans {
                assert!(!span.text.contains('\n'), "{span:?}");
            }
        }
    }

    #[test]
    fn an_unknown_extension_is_shown_plainly_rather_than_guessed_at() {
        let syntaxes = SyntaxSet::load_defaults_newlines();
        let (lines, language) = lines(&syntaxes, "firmware.bin.txt.zzz", "anything at all\n");

        assert_eq!(language, None);
        assert_eq!(lines[0].spans[0].token, Token::Plain);
        assert_eq!(lines[0].spans[0].text, "anything at all");
    }

    #[test]
    fn toml_is_highlighted_by_the_fallback() {
        // The files people most often open in an embedded project are not Rust,
        // and syntect's bundle has no TOML at all.
        let spans = tokens(
            "config.toml",
            "# a note\n[build]\ntarget = \"riscv32imc-unknown-none-elf\"\nlto = true\n",
        );

        assert_eq!(token_of(&spans, "# a note"), Some(Token::Comment));
        assert_eq!(token_of(&spans, "[build]"), Some(Token::Type));
        assert_eq!(token_of(&spans, "target"), Some(Token::Variable));
        assert!(
            spans
                .iter()
                .any(|(t, k)| t.contains("riscv32") && *k == Token::Str),
            "{spans:?}",
        );
        assert!(
            spans
                .iter()
                .any(|(t, k)| t.trim() == "true" && *k == Token::Keyword),
            "{spans:?}",
        );
    }

    /// Cargo.lock is TOML that does not say so in its name, and it is a file
    /// people actually open.
    #[test]
    fn cargo_lock_is_highlighted_as_toml() {
        let syntaxes = SyntaxSet::load_defaults_newlines();
        let (lines, language) = lines(
            &syntaxes,
            "Cargo.lock",
            "[[package]]
name = \"serde\"
",
        );
        assert_eq!(language.as_deref(), Some("TOML"));
        assert_eq!(lines[0].spans[0].token, Token::Type);
    }

    /// A `#` inside a string is not a comment. Getting this wrong greys out the
    /// rest of every line holding a URL, and `Cargo.toml` is full of them.
    #[test]
    fn a_hash_inside_a_toml_string_is_not_a_comment() {
        let spans = tokens("Cargo.toml", "repo = \"https://x/y#frag\"  # real one\n");

        assert!(
            spans
                .iter()
                .any(|(t, k)| t.contains("#frag") && *k == Token::Str),
            "the fragment belongs to the string: {spans:?}",
        );
        assert!(
            spans
                .iter()
                .any(|(t, k)| t.contains("real one") && *k == Token::Comment),
            "{spans:?}",
        );
    }

    /// An apostrophe inside a double-quoted string is a letter, not a quote.
    /// Toggling on it left the state "in string" at the `#`, and the comment
    /// was painted as string — on every `description = "it's …"` line.
    #[test]
    fn a_quote_of_the_other_kind_does_not_close_a_toml_string() {
        let spans = tokens("Cargo.toml", "desc = \"it's\"  # note\n");

        assert!(
            spans
                .iter()
                .any(|(t, k)| t.contains("it's") && *k == Token::Str),
            "the string is a string: {spans:?}",
        );
        assert!(
            spans
                .iter()
                .any(|(t, k)| t.contains("# note") && *k == Token::Comment),
            "the comment after it is a comment: {spans:?}",
        );

        // And the other way round: a double quote inside a literal string.
        let spans = tokens("Cargo.toml", "path = 'say \"hi\"'  # note\n");
        assert!(
            spans
                .iter()
                .any(|(t, k)| t.contains("# note") && *k == Token::Comment),
            "{spans:?}",
        );
    }
}
