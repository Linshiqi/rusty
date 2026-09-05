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

use syntect::parsing::{ParseState, Scope, ScopeStack, SyntaxSet};

use crate::model::{Line, Span, Token};

/// Files longer than this are shown but not highlighted past the cut.
///
/// A generated register map can be tens of thousands of lines, and highlighting
/// one costs more than reading it is worth. The cut is reported rather than
/// silently applied.
const MAX_LINES: usize = 5_000;

/// Highlight `text`, using the grammar matching `path`'s extension.
///
/// Returns the lines and the grammar's name, which is `None` when nothing
/// matched — a plain-text file is shown unstyled rather than guessed at.
pub fn lines(syntaxes: &SyntaxSet, path: &str, text: &str) -> (Vec<Line>, Option<String>, bool) {
    let extension = path.rsplit('.').next().unwrap_or_default();
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

    let truncated = text.lines().count() > MAX_LINES;
    let source = text.lines().take(MAX_LINES);

    // syntect's bundled grammars have no TOML, and TOML is what an embedded
    // project is configured with: Cargo.toml, .cargo/config.toml,
    // rust-toolchain.toml. Rust in colour beside those in flat grey reads as
    // broken, so the one gap that matters here is filled by hand.
    // Cargo.lock is TOML that does not say so in its name — and it is a
    // file people actually open, where all-grey next to coloured Rust reads
    // as broken highlighting rather than as a plain file.
    if extension.eq_ignore_ascii_case("toml") || extension.eq_ignore_ascii_case("lock") {
        return (
            source.map(toml_line).collect(),
            Some("TOML".into()),
            truncated,
        );
    }

    let Some(syntax) = syntax else {
        let plain = source
            .map(|line| Line {
                spans: vec![Span {
                    text: line.to_string(),
                    token: Token::Plain,
                }],
            })
            .collect();
        return (plain, None, truncated);
    };

    let mut state = ParseState::new(syntax);
    let mut stack = ScopeStack::new();
    let mut out = Vec::new();

    for line in source {
        // syntect wants the newline: several grammars end a construct on it,
        // and without it a line comment never closes.
        let owned = format!("{line}\n");
        let ops = state.parse_line(&owned, syntaxes).unwrap_or_default();

        let mut spans: Vec<Span> = Vec::new();
        let mut at = 0usize;

        for (offset, op) in ops {
            push(&mut spans, &owned[at..offset.min(owned.len())], &stack);
            let _ = stack.apply(&op);
            at = offset;
        }
        push(&mut spans, &owned[at.min(owned.len())..], &stack);

        // The newline was only for the parser.
        if let Some(last) = spans.last_mut() {
            while last.text.ends_with('\n') || last.text.ends_with('\r') {
                last.text.pop();
            }
            if last.text.is_empty() {
                spans.pop();
            }
        }

        // syntect styles declarations and leaves most expressions plain —
        // `TimerGroup::new(x)` came back one white run. The lexical pass
        // splits those by the conventions the language enforces anyway.
        out.push(Line {
            spans: crate::lexical::refine(spans),
        });
    }

    (out, Some(syntax.name.clone()), truncated)
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
fn push(spans: &mut Vec<Span>, text: &str, stack: &ScopeStack) {
    if text.is_empty() {
        return;
    }
    let token = classify(stack);
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
fn classify(stack: &ScopeStack) -> Token {
    for scope in stack.as_slice().iter().rev() {
        if let Some(token) = token_for(*scope) {
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
            || name.starts_with("support.type")
            || name.starts_with("storage.type") =>
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
            let (out, _, _) = lines(&syntaxes, &path.to_string_lossy(), &text);
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
        let (_, language, _) = lines(
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
        let (lines, _, _) = lines(&syntaxes, path, source);
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
    /// or every line gains a blank cell and the gutter stops lining up.
    #[test]
    fn the_newline_fed_to_the_parser_is_not_returned() {
        let syntaxes = SyntaxSet::load_defaults_newlines();
        let (lines, _, _) = lines(&syntaxes, "main.rs", "fn a() {}\nfn b() {}\n");

        assert_eq!(lines.len(), 2);
        for line in &lines {
            for span in &line.spans {
                assert!(!span.text.contains('\n'), "{span:?}");
            }
        }
    }

    #[test]
    fn an_unknown_extension_is_shown_plainly_rather_than_guessed_at() {
        let syntaxes = SyntaxSet::load_defaults_newlines();
        let (lines, language, _) = lines(&syntaxes, "firmware.bin.txt.zzz", "anything at all\n");

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
        let (lines, language, _) = lines(
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
