//! Which `.rs` files in a project no `mod` declaration reaches.
//!
//! rust-analyzer answers this authoritatively — `unlinked-file` — but only
//! for a file it has been *told about*, which means a file somebody has
//! opened. The tree has to dim a file before anybody opens it, so it needs an
//! answer of its own, and this is it: read the `mod` declarations, and say
//! which files nothing names.
//!
//! **It refuses far more readily than it claims.** Dimming a file the
//! compiler does build is the confident wrong answer this project fears, so
//! the reading is arranged to fail towards "linked":
//!
//! - A crate holding any `#[path]` attribute claims nothing at all. `#[path]`
//!   makes a file reachable under a name that is not its own, and following
//!   it means resolving attributes, which is a parser's job.
//! - A `mod` inside a comment or a string still counts as a declaration. The
//!   scan is lexical; over-counting declarations loses a dim, and
//!   under-counting invents one.
//! - Every entry point cargo knows is a root, declared by nobody:
//!   `src/main.rs`, `src/lib.rs`, `build.rs`, and anything directly under
//!   `src/bin`, `examples`, `tests` and `benches`.
//! - A file outside a crate's `src` — the root of `tests/` or `examples/`
//!   and the trees below them — is left alone, because a `tests/thing/mod.rs`
//!   is reached from a sibling integration test whose own rules differ.
//!
//! The consequence of all that caution is the useful case surviving: a file
//! written into `src/` and never declared, which is the state a user hits by
//! creating a file and is exactly what "rust-analyzer offers nothing here"
//! means.

use std::collections::HashSet;

/// One file as the scan sees it: its project-relative path (`/` separators)
/// and its text.
pub struct SourceFile<'a> {
    pub path: &'a str,
    pub text: &'a str,
}

/// The project-relative paths of `.rs` files under a crate's `src/` that no
/// `mod` declaration names, sorted — or `None` when this reading refuses to
/// claim anything about the project at all.
///
/// The two are different answers and must not be one. An empty list means
/// "every file is declared"; a refusal means "ask somebody else", and the
/// caller then falls back to rust-analyzer's own verdict. Returning an empty
/// list for both would make a refusal read as a clean bill of health.
///
/// `manifests` are the project-relative directories that hold a `Cargo.toml`
/// — every crate root the project has, since a `mod` in one crate does not
/// declare a file in another. Pure over its inputs, so the rules above are
/// tests rather than something discovered on somebody's checkout.
pub fn unlinked(files: &[SourceFile<'_>], manifests: &[String]) -> Option<Vec<String>> {
    // `#[path]` anywhere in the project takes the whole answer away. It is
    // rare, and being silent about a project that uses it costs nothing;
    // dimming a file it pulls in would be a lie.
    if files.iter().any(|file| file.text.contains("#[path")) {
        return None;
    }

    let mut declared: HashSet<&str> = HashSet::new();
    for file in files {
        for name in mod_names(file.text) {
            declared.insert(name);
        }
    }

    let mut out: Vec<String> = files
        .iter()
        .filter(|file| in_a_crates_src(file.path, manifests))
        .filter(|file| !is_root(file.path))
        .filter(|file| !declared.contains(module_name(file.path)))
        .map(|file| file.path.to_string())
        .collect();
    out.sort();
    Some(out)
}

/// The module names a file declares: `mod x;`, `pub mod x;`,
/// `pub(crate) mod x { … }`.
///
/// Lexical and deliberately generous — see the module header. It reads whole
/// lines, so a `mod` in a comment declares a module as far as this is
/// concerned, which costs a dim and never invents one.
fn mod_names(text: &str) -> Vec<&str> {
    let mut names = Vec::new();
    for line in text.lines() {
        let mut rest = line.trim_start();
        // A commented-out declaration counts, on purpose. Somebody who has
        // just written `// mod thing;` is mid-edit, and a file that went dim
        // between two keystrokes reads as the editor losing track of it.
        // Over-counting costs a dim; under-counting invents one.
        while let Some(after) = rest.strip_prefix("//").or_else(|| rest.strip_prefix("/*")) {
            rest = after.trim_start();
        }
        // Strip the visibility, whatever shape it takes.
        if let Some(after) = rest.strip_prefix("pub") {
            rest = match after.strip_prefix('(') {
                Some(scoped) => match scoped.find(')') {
                    Some(at) => scoped[at + 1..].trim_start(),
                    None => continue,
                },
                None => after.trim_start(),
            };
        }
        let Some(after) = rest.strip_prefix("mod") else {
            continue;
        };
        // `mod` and not `module`, `mods`…
        if !after.starts_with(char::is_whitespace) {
            continue;
        }
        let name: &str = after
            .trim_start()
            .split(|c: char| !(c.is_alphanumeric() || c == '_'))
            .next()
            .unwrap_or("");
        if !name.is_empty() {
            names.push(name);
        }
    }
    names
}

/// The name a `mod` declaration would use for this file: its stem, or the
/// directory's name when the file is a `mod.rs`.
fn module_name(path: &str) -> &str {
    let name = path.rsplit('/').next().unwrap_or(path);
    if name != "mod.rs" {
        return name.strip_suffix(".rs").unwrap_or(name);
    }
    let parent = &path[..path.len() - name.len()];
    parent
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(parent)
}

/// An entry point cargo compiles without anybody declaring it.
fn is_root(path: &str) -> bool {
    let after = |dir: &str| -> Option<&str> {
        let at = path.rfind(dir)?;
        // Only at the start or after a separator, so `mysrc/` is not `src/`.
        if at != 0 && !path[..at].ends_with('/') {
            return None;
        }
        Some(&path[at + dir.len()..])
    };
    if path == "build.rs" || path.ends_with("/build.rs") {
        return true;
    }
    if let Some(tail) = after("src/") {
        return tail == "main.rs" || tail == "lib.rs" || is_directly_under(tail, "bin/");
    }
    false
}

/// `bin/thing.rs`, not `bin/thing/helper.rs` — a `src/bin/x/main.rs` is a
/// root too, and that is the one shape this deliberately does not claim, so
/// it is treated as outside a crate's `src` by [`in_a_crates_src`].
fn is_directly_under(tail: &str, dir: &str) -> bool {
    tail.strip_prefix(dir)
        .is_some_and(|rest| !rest.contains('/'))
}

/// Under the `src/` of one of the project's crates, and not under a
/// `src/bin/<name>/` subtree, whose `main.rs` is its own root.
fn in_a_crates_src(path: &str, manifests: &[String]) -> bool {
    if !path.ends_with(".rs") {
        return false;
    }
    let under_src = manifests.iter().any(|dir| {
        let prefix = if dir.is_empty() {
            "src/".to_string()
        } else {
            format!("{}/src/", dir.trim_end_matches('/'))
        };
        path.starts_with(&prefix)
    });
    if !under_src {
        return false;
    }
    // `src/bin/<name>/…` is a binary of its own with its own module tree,
    // rooted at a `main.rs` this scan does not follow. Left alone.
    let Some(at) = path.rfind("src/bin/") else {
        return true;
    };
    !path[at + "src/bin/".len()..].contains('/')
}

/// Walk the project and answer [`unlinked`] for it.
///
/// The same walk the tree uses, so a file the tree does not show cannot be
/// dimmed in it, and `target/` costs nothing. Reads only `.rs` files and only
/// the ones under a crate's `src/`, plus every `.rs` in the project for the
/// `mod` declarations — a declaration can sit anywhere, and a `src/lib.rs`
/// that was not read is a crate whose whole module tree reads as unlinked.
#[cfg(feature = "backend")]
pub fn scan(root: &std::path::Path) -> Option<Vec<String>> {
    let mut manifests: Vec<String> = Vec::new();
    let mut sources: Vec<(String, String)> = Vec::new();

    let walk = crate::hidden::project_walk(root)
        .max_depth(Some(crate::tree::MAX_DEPTH))
        .build();

    for found in walk.flatten() {
        let path = found.path();
        let Ok(relative) = path.strip_prefix(root) else {
            continue;
        };
        let relative = relative.to_string_lossy().replace('\\', "/");
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
        match name.as_deref() {
            Some("Cargo.toml") => {
                let dir = relative.strip_suffix("Cargo.toml").unwrap_or_default();
                manifests.push(dir.trim_end_matches('/').to_string());
            }
            Some(name) if name.ends_with(".rs") => {
                // A file too large to be a module declaration list is still
                // read: it may hold the `mod` that saves its neighbour.
                if let Ok(text) = std::fs::read_to_string(path) {
                    sources.push((relative, text));
                }
            }
            _ => {}
        }
    }

    let files: Vec<SourceFile<'_>> = sources
        .iter()
        .map(|(path, text)| SourceFile { path, text })
        .collect();
    unlinked(&files, &manifests)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What the scan claims, where it claims anything. A refusal is its own
    /// test below.
    fn scan<'a>(files: &'a [(&'a str, &'a str)], manifests: &[&str]) -> Vec<String> {
        claim(files, manifests).expect("the scan claims something")
    }

    fn claim<'a>(files: &'a [(&'a str, &'a str)], manifests: &[&str]) -> Option<Vec<String>> {
        let files: Vec<SourceFile<'_>> = files
            .iter()
            .map(|(path, text)| SourceFile { path, text })
            .collect();
        let manifests: Vec<String> = manifests.iter().map(|m| (*m).to_string()).collect();
        unlinked(&files, &manifests)
    }

    /// The case this exists for: a file written into `src/` that no `mod`
    /// names. rust-analyzer answers nothing at all in it, and until the tree
    /// dims it there is nothing on screen that says why.
    #[test]
    fn a_file_nothing_declares_is_the_one_that_is_named() {
        let files = [
            ("src/main.rs", "mod engine;\nfn main() {}\n"),
            ("src/engine.rs", "pub fn go() {}\n"),
            ("src/orphan.rs", "pub struct Quaternion;\n"),
        ];
        assert_eq!(scan(&files, &[""]), ["src/orphan.rs"]);
    }

    /// Every visibility, and a `mod` with a body rather than a semicolon.
    #[test]
    fn a_declaration_counts_however_it_is_spelled() {
        let files = [
            (
                "src/lib.rs",
                "pub mod a;\npub(crate) mod b;\n    mod c;\npub mod d { }\n",
            ),
            ("src/a.rs", ""),
            ("src/b.rs", ""),
            ("src/c.rs", ""),
            ("src/d.rs", ""),
        ];
        assert!(scan(&files, &[""]).is_empty());
    }

    /// A `mod.rs` is declared by its *directory's* name, and a nested one is
    /// declared from the module above it rather than from the crate root.
    #[test]
    fn a_directory_module_is_declared_by_the_directorys_name() {
        let files = [
            ("src/lib.rs", "mod shapes;\n"),
            ("src/shapes/mod.rs", "mod circle;\n"),
            ("src/shapes/circle.rs", ""),
            ("src/shapes/square.rs", ""),
        ];
        assert_eq!(scan(&files, &[""]), ["src/shapes/square.rs"]);
    }

    /// Roots are declared by nobody, in every crate of a workspace, and a
    /// `mod` in one crate does not reach into another.
    #[test]
    fn every_entry_point_cargo_knows_is_a_root() {
        let files = [
            ("core/src/lib.rs", "\n"),
            ("core/build.rs", "fn main() {}\n"),
            ("firmware/src/main.rs", "\n"),
            ("firmware/src/bin/other.rs", "\n"),
            ("firmware/src/helper.rs", "\n"),
        ];
        assert_eq!(
            scan(&files, &["core", "firmware"]),
            ["firmware/src/helper.rs"],
            "only the undeclared non-root is named"
        );
    }

    /// `#[path]` puts a file in the tree under a name that is not its own,
    /// and following it is a parser's job. The whole answer goes rather than
    /// risk dimming a file the compiler builds.
    #[test]
    fn a_path_attribute_anywhere_takes_the_whole_claim_away() {
        let files = [
            ("src/lib.rs", "#[path = \"weird/name.rs\"]\nmod thing;\n"),
            ("src/orphan.rs", ""),
        ];
        assert_eq!(
            claim(&files, &[""]),
            None,
            "refuse rather than guess, project-wide — and a refusal is not an \
             empty list, which would read as a clean bill of health"
        );
    }

    /// Over-counting a declaration costs a dim; under-counting invents one.
    /// So a `mod` in a comment is a declaration here, deliberately.
    #[test]
    fn a_declaration_in_a_comment_still_counts() {
        let files = [("src/lib.rs", "// mod orphan;\n"), ("src/orphan.rs", "")];
        assert!(scan(&files, &[""]).is_empty());
    }

    /// Nothing outside a crate's `src/` is claimed about: an integration
    /// test's helpers are reached by rules of their own.
    #[test]
    fn files_outside_a_crates_src_are_left_alone() {
        let files = [
            ("src/lib.rs", "\n"),
            ("tests/shared/helper.rs", ""),
            ("examples/demo.rs", ""),
            ("src/bin/tool/helper.rs", ""),
            ("mysrc/orphan.rs", ""),
        ];
        assert!(scan(&files, &[""]).is_empty());
    }

    /// `module` and `mods` are not `mod`.
    #[test]
    fn a_word_beginning_with_mod_is_not_a_declaration() {
        let files = [
            ("src/lib.rs", "mods orphan;\nlet module_orphan = 1;\n"),
            ("src/orphan.rs", ""),
        ];
        assert_eq!(scan(&files, &[""]), ["src/orphan.rs"]);
    }
}
