//! Finding the tests in a Rust file, so the editor can offer to run them.
//!
//! Model-side and IO-free, like [`crate::lexical`]: the gutter is drawn by the
//! frontend, so the frontend has to be able to ask this question about the
//! text it is holding rather than about the copy the backend last read. A
//! round trip per keystroke would be the alternative, and the answer would
//! always be one edit behind the line numbers it is decorating.
//!
//! Lexical rather than syntactic, and the trade is deliberate. `syn` would be
//! exact and would drag a parser into the wasm bundle for a decoration; a
//! file mid-edit does not parse anyway, and a run arrow that disappears while
//! you are typing inside the function it belongs to is worse than one that is
//! occasionally offered for something that will not compile. What it must not
//! do is *invent* a name — `cargo test` with a filter that matches nothing
//! exits successfully having run nothing, which reads exactly like a passing
//! test.

use serde::{Deserialize, Serialize};

/// Something the editor can offer to run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Runnable {
    /// 0-based line the arrow is drawn on — the `fn` or the `mod`, not the
    /// attribute above it. Clicking a decoration that sits on `#[test]` and
    /// runs the function below reads as an off-by-one.
    pub line: u32,
    /// What to show: `it_works`, or `tests` for a module.
    pub name: String,
    /// What to pass `cargo test`. Module-qualified where the module is known,
    /// because a bare name that two modules share runs both and reports the
    /// pair as one result.
    pub filter: String,
    pub kind: RunnableKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RunnableKind {
    /// One `#[test] fn`.
    Test,
    /// A `mod` holding some — running it runs everything inside.
    Module,
}

/// Every test and test module in `text`.
///
/// Order is by line, so the caller can bisect and so the gutter's decorations
/// come out in the order they are drawn.
pub fn runnables(text: &str) -> Vec<Runnable> {
    let mut found = Vec::new();
    // The `mod` names currently open, and the brace depth each was opened at.
    let mut modules: Vec<(String, i32)> = Vec::new();
    let mut depth: i32 = 0;
    // Set by a `#[test]`-ish attribute, cleared by the next item. Attributes
    // stack — `#[test] #[should_panic]` — so this is sticky until an item
    // consumes it.
    let mut attributed = false;

    for (index, raw) in text.lines().enumerate() {
        let line = strip_comment(raw);
        let trimmed = line.trim();

        if is_test_attribute(trimmed) {
            attributed = true;
        }

        if let Some(name) = declared_fn(trimmed) {
            if attributed {
                found.push(Runnable {
                    line: index as u32,
                    name: name.to_string(),
                    filter: qualify(&modules, name),
                    kind: RunnableKind::Test,
                });
            }
            attributed = false;
        } else if let Some(name) = declared_mod(trimmed) {
            // Every module gets a marker, not only ones called `tests`: a
            // module named `parsing` full of `#[test]`s is exactly as
            // runnable, and keying off the name would offer it for an empty
            // `mod tests` while hiding a real one.
            let opened_at = depth;
            let qualified = qualify(&modules, name);
            modules.push((name.to_string(), opened_at));
            found.push(Runnable {
                line: index as u32,
                name: name.to_string(),
                filter: qualified,
                kind: RunnableKind::Module,
            });
            attributed = false;
        } else if !trimmed.is_empty() && !trimmed.starts_with('#') && !trimmed.starts_with("//") {
            // Any other item consumes a pending attribute. Without this, a
            // `#[test]` on a function inside a macro body would attach itself
            // to whatever function came next.
            if starts_item(trimmed) {
                attributed = false;
            }
        }

        depth += braces(line);
        // Close every module whose body just ended.
        while modules.last().is_some_and(|(_, at)| depth <= *at) {
            modules.pop();
        }
    }

    // A module with nothing runnable inside it is not runnable. `cargo test
    // some::empty::mod` exits zero having run nothing, which on screen is
    // indistinguishable from a module whose tests all passed.
    let tests: Vec<&Runnable> = found
        .iter()
        .filter(|r| r.kind == RunnableKind::Test)
        .collect();
    let keep: Vec<bool> = found
        .iter()
        .map(|r| match r.kind {
            RunnableKind::Test => true,
            RunnableKind::Module => tests
                .iter()
                .any(|t| t.filter.starts_with(&format!("{}::", r.filter))),
        })
        .collect();
    found
        .into_iter()
        .zip(keep)
        .filter_map(|(r, keep)| keep.then_some(r))
        .collect()
}

/// The runnable whose body contains `line`, innermost first.
///
/// What "run the test the caret is in" needs. Approximate on purpose: a
/// lexical pass has no body extents, so the answer is the last runnable
/// declared at or above the caret, which is right except inside the gap
/// between two items.
pub fn enclosing(text: &str, line: u32) -> Option<Runnable> {
    runnables(text).into_iter().rfind(|r| r.line <= line)
}

/// `#[test]`, `#[tokio::test]`, `#[test_case]`-style — anything whose final
/// path segment is `test`.
///
/// Matching the whole attribute rather than a substring: `#[cfg(test)]` is
/// not a test, and treating it as one would put a run arrow on every module
/// in the file.
fn is_test_attribute(trimmed: &str) -> bool {
    let Some(rest) = trimmed.strip_prefix("#[") else {
        return false;
    };
    let Some(body) = rest.strip_suffix(']') else {
        return false;
    };
    body.split(',').any(|attr| {
        let attr = attr.trim();
        // `should_panic(expected = "…")` and friends carry arguments; the
        // path is what is before the parenthesis.
        let path = attr.split('(').next().unwrap_or(attr).trim();
        path.rsplit("::").next().is_some_and(|last| last == "test")
    })
}

fn declared_fn(trimmed: &str) -> Option<&str> {
    let rest = strip_modifiers(trimmed)?;
    let rest = rest.strip_prefix("fn ")?;
    let name = rest
        .trim_start()
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .next()?;
    (!name.is_empty()).then_some(name)
}

fn declared_mod(trimmed: &str) -> Option<&str> {
    let rest = strip_modifiers(trimmed)?;
    let rest = rest.strip_prefix("mod ")?;
    let name = rest
        .trim_start()
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .next()?;
    // `mod foo;` is a file, not a body — nothing here to run, and the tests
    // in it belong to the line numbers of a different file.
    (!name.is_empty() && trimmed.contains('{')).then_some(name)
}

/// Strip `pub`, `pub(crate)`, `async`, `unsafe`, `const`, `extern "C"`.
fn strip_modifiers(trimmed: &str) -> Option<&str> {
    let mut rest = trimmed;
    loop {
        let before = rest;
        for word in ["async ", "unsafe ", "const ", "default "] {
            if let Some(next) = rest.strip_prefix(word) {
                rest = next.trim_start();
            }
        }
        if let Some(next) = rest.strip_prefix("pub") {
            let next = next.trim_start();
            // `pub(crate)`, `pub(super)`, `pub(in path)`
            let next = match next.strip_prefix('(') {
                Some(inner) => inner.split_once(')').map(|(_, after)| after)?.trim_start(),
                None => next,
            };
            rest = next;
        }
        if let Some(next) = rest.strip_prefix("extern ") {
            let next = next.trim_start();
            rest = match next.strip_prefix('"') {
                Some(inner) => inner.split_once('"').map(|(_, after)| after)?.trim_start(),
                None => next,
            };
        }
        if rest == before {
            break;
        }
    }
    Some(rest)
}

/// True for anything that ends an attribute's reach.
fn starts_item(trimmed: &str) -> bool {
    let Some(rest) = strip_modifiers(trimmed) else {
        return false;
    };
    [
        "struct ",
        "enum ",
        "trait ",
        "impl ",
        "type ",
        "use ",
        "static ",
        "macro_rules!",
    ]
    .iter()
    .any(|kw| rest.starts_with(kw))
}

fn qualify(modules: &[(String, i32)], name: &str) -> String {
    if modules.is_empty() {
        return name.to_string();
    }
    let mut path = String::new();
    for (module, _) in modules {
        path.push_str(module);
        path.push_str("::");
    }
    path.push_str(name);
    path
}

/// Braces outside strings, characters and line comments.
///
/// Not a parser — it does not know about raw strings with embedded quotes or
/// block comments — but wrong brace counting only mis-attributes a module
/// path, and a filter that is too broad runs extra tests rather than the
/// wrong ones.
fn braces(line: &str) -> i32 {
    let mut depth = 0;
    let mut in_string = false;
    let mut in_char = false;
    let mut escaped = false;
    for c in line.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        match c {
            '\\' if in_string || in_char => escaped = true,
            '"' if !in_char => in_string = !in_string,
            '\'' if !in_string => in_char = !in_char,
            '{' if !in_string && !in_char => depth += 1,
            '}' if !in_string && !in_char => depth -= 1,
            _ => {}
        }
    }
    depth
}

/// Everything before a `//` that is not inside a string.
fn strip_comment(line: &str) -> &str {
    let bytes = line.as_bytes();
    let mut in_string = false;
    let mut escaped = false;
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if escaped {
            escaped = false;
        } else if c == b'\\' && in_string {
            escaped = true;
        } else if c == b'"' {
            in_string = !in_string;
        } else if c == b'/' && !in_string && bytes.get(i + 1) == Some(&b'/') {
            return &line[..i];
        }
        i += 1;
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(text: &str) -> Vec<(u32, String, RunnableKind)> {
        runnables(text)
            .into_iter()
            .map(|r| (r.line, r.filter, r.kind))
            .collect()
    }

    #[test]
    fn a_plain_test_is_found_on_its_own_line() {
        let text = "#[test]\nfn it_works() {\n    assert!(true);\n}\n";
        assert_eq!(
            names(text),
            vec![(1, "it_works".to_string(), RunnableKind::Test)]
        );
    }

    /// The arrow goes on the `fn`, not on the attribute. A decoration that
    /// sits one line above what it runs reads as an off-by-one every time.
    #[test]
    fn the_line_is_the_declaration_not_the_attribute() {
        let text = "#[test]\n#[should_panic]\nfn boom() {}\n";
        assert_eq!(runnables(text)[0].line, 2);
    }

    /// `#[cfg(test)]` is not `#[test]`. Treating it as one would put a run
    /// arrow on every test module's `#[cfg]` line — and, worse, on the
    /// declaration below it whatever that is.
    #[test]
    fn cfg_test_is_not_a_test() {
        assert!(!is_test_attribute("#[cfg(test)]"));
        assert!(is_test_attribute("#[test]"));
        assert!(is_test_attribute("#[tokio::test]"));
        assert!(is_test_attribute("#[test, ignore]"));
        assert!(!is_test_attribute("#[derive(Debug)]"));
        assert!(!is_test_attribute("#[should_panic]"));
    }

    #[test]
    fn a_module_qualifies_the_tests_inside_it() {
        let text = "\
#[cfg(test)]
mod tests {
    #[test]
    fn one() {}
}
";
        assert_eq!(
            names(text),
            vec![
                (1, "tests".to_string(), RunnableKind::Module),
                (3, "tests::one".to_string(), RunnableKind::Test),
            ]
        );
    }

    #[test]
    fn nested_modules_nest_the_filter() {
        let text = "\
mod outer {
    mod inner {
        #[test]
        fn deep() {}
    }
}
";
        let found = names(text);
        assert!(found.contains(&(3, "outer::inner::deep".to_string(), RunnableKind::Test)));
    }

    /// A module's body ending must pop it, or every test after the closing
    /// brace inherits a path it is not in — and `cargo test tests::later`
    /// matches nothing, which exits zero and looks like a pass.
    #[test]
    fn a_closed_module_stops_qualifying() {
        let text = "\
mod tests {
    #[test]
    fn inside() {}
}

#[test]
fn outside() {}
";
        let found = names(text);
        assert!(found.contains(&(2, "tests::inside".to_string(), RunnableKind::Test)));
        assert!(found.contains(&(6, "outside".to_string(), RunnableKind::Test)));
    }

    /// An empty module is not offered. `cargo test` with a filter that
    /// matches nothing exits successfully having run nothing, and on screen
    /// that is indistinguishable from everything passing.
    #[test]
    fn a_module_with_no_tests_is_not_runnable() {
        let text = "mod helpers {\n    fn helper() {}\n}\n";
        assert_eq!(names(text), vec![]);
    }

    #[test]
    fn modifiers_do_not_hide_a_declaration() {
        let text = "\
#[test]
pub fn public() {}
#[tokio::test]
pub(crate) async fn asynchronous() {}
";
        let found = names(text);
        assert!(found.iter().any(|(_, f, _)| f == "public"));
        assert!(found.iter().any(|(_, f, _)| f == "asynchronous"));
    }

    /// `mod foo;` names a different file. Offering to run it would put an
    /// arrow beside a line whose tests live somewhere else entirely.
    #[test]
    fn a_module_declaration_without_a_body_is_not_offered() {
        assert_eq!(names("#[cfg(test)]\nmod tests;\n"), vec![]);
    }

    /// A brace inside a string must not open a scope, or every module after
    /// it is mis-nested.
    #[test]
    fn braces_in_strings_and_comments_do_not_count() {
        assert_eq!(braces(r#"let s = "{{{";"#), 0);
        assert_eq!(braces("let c = '{';"), 0);
        assert_eq!(strip_comment("code(); // } } }"), "code(); ");
        assert_eq!(
            strip_comment(r#"let s = "// not a comment";"#),
            r#"let s = "// not a comment";"#
        );
    }

    /// The attribute must not float past the item it was written for.
    #[test]
    fn an_attribute_does_not_attach_to_a_later_function() {
        let text = "\
#[test]
struct NotATest;

fn ordinary() {}
";
        assert_eq!(names(text), vec![]);
    }

    #[test]
    fn the_caret_finds_the_test_it_is_inside() {
        let text = "\
#[cfg(test)]
mod tests {
    #[test]
    fn first() {
        assert!(true);
    }

    #[test]
    fn second() {}
}
";
        assert_eq!(enclosing(text, 4).unwrap().filter, "tests::first");
        assert_eq!(enclosing(text, 8).unwrap().filter, "tests::second");
        assert_eq!(enclosing(text, 0), None);
    }

    #[test]
    fn a_file_with_no_tests_offers_nothing() {
        assert_eq!(runnables("fn main() {\n    println!(\"hi\");\n}\n"), vec![]);
        assert_eq!(runnables(""), vec![]);
    }
}
