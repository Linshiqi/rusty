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
//!   `src/main.rs`, `src/lib.rs`, `build.rs`, anything directly under
//!   `src/bin`, `examples`, `tests` and `benches`, and every target a
//!   manifest names by `path` (`[lib] path = "src/core.rs"`). A manifest
//!   that does not parse leaves its crate unjudged.
//! - A file outside a crate's `src` — the root of `tests/` or `examples/`
//!   and the trees below them — is left alone, because a `tests/thing/mod.rs`
//!   is reached from a sibling integration test whose own rules differ.
//!
//! The consequence of all that caution is the useful case surviving: a file
//! written into `src/` and never declared, which is the state a user hits by
//! creating a file and is exactly what "rust-analyzer offers nothing here"
//! means.

use std::collections::{BTreeSet, HashSet};

/// One file as the scan sees it: its project-relative path (`/` separators)
/// and its text.
pub struct SourceFile<'a> {
    pub path: &'a str,
    pub text: &'a str,
}

/// What a scan of a project said, kept with where it is entitled to say
/// anything: the crates it found, and of the files under their `src/` the
/// ones nothing declares — or that it refused the whole project.
///
/// The crates travel with the answer because the answer is about them
/// alone. A file outside every crate's `src/` — an integration test's
/// helper, a `src/bin/<name>/` tree — is one the scan never judged, so
/// asked about it the reading has no opinion, and somebody else's verdict
/// stands there.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Reading {
    crates: Vec<String>,
    unlinked: Option<Vec<String>>,
}

impl Reading {
    /// A reading of `files` in the crates at `manifests`, with the targets
    /// their manifests name by path as `roots`: [`unlinked_rooted`]'s
    /// answer, kept with what it was about.
    pub fn of(files: &[SourceFile<'_>], manifests: &[String], roots: &[String]) -> Reading {
        Reading {
            crates: manifests.to_vec(),
            unlinked: unlinked_rooted(files, manifests, roots),
        }
    }

    /// The files nothing declares, sorted, or `None` for a refusal.
    pub fn unlinked(&self) -> Option<&[String]> {
        self.unlinked.as_deref()
    }

    /// What the reading says about one file: `Some(true)` that nothing
    /// declares it, `Some(false)` that something does (or that it is a root
    /// cargo compiles), and `None` that it has no opinion — the whole
    /// project refused, or the file is not one it judges.
    pub fn about(&self, path: &str) -> Option<bool> {
        let unlinked = self.unlinked.as_ref()?;
        if !in_a_crates_src(path, &self.crates) {
            return None;
        }
        Some(unlinked.binary_search_by(|p| p.as_str().cmp(path)).is_ok())
    }
}

/// The files to draw dimmed: the reading's verdict wherever it has one, and
/// the language server's (`server`, the files rust-analyzer last called
/// `unlinked-file`) only where it has none — a refusal, a file the reading
/// does not judge, or no reading at all (`None`: not scanned yet, or not a
/// Cargo project).
///
/// **One wins; they are never or-ed.** The two disagree in one ordinary
/// moment: a `mod` line has just been added, the scan has read it, and the
/// server's diagnostic still stands, because it stays until the server
/// analyses *that* file again and nothing is going to make it. Or-ed, the
/// file stays dim under a declaration that reaches it — a fact that can be
/// asserted and never taken back. The scan has just read the files; where
/// it speaks, it is the newer word.
pub fn dimmed<'a>(
    reading: Option<&Reading>,
    server: impl IntoIterator<Item = &'a str>,
) -> BTreeSet<String> {
    let mut out: BTreeSet<String> = reading
        .and_then(Reading::unlinked)
        .into_iter()
        .flatten()
        .cloned()
        .collect();
    for path in server {
        if reading.and_then(|r| r.about(path)).is_none() {
            out.insert(path.to_string());
        }
    }
    out
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
    unlinked_rooted(files, manifests, &[])
}

/// [`unlinked`], with `roots` — the project-relative paths of targets a
/// manifest names by `path` ([`manifest_roots`]) — compiled by cargo and so
/// declared by nobody, wherever they sit.
pub fn unlinked_rooted(
    files: &[SourceFile<'_>],
    manifests: &[String],
    roots: &[String],
) -> Option<Vec<String>> {
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
        .filter(|file| !is_root(file.path) && !roots.iter().any(|root| root == file.path))
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

/// The targets a manifest names by `path`, as project-relative paths — the
/// library, every binary, example, test and bench, and a build script
/// moved by `package.build` — or `None` when the manifest does not parse,
/// which leaves its crate unjudged: a root this reading could not see is a
/// file it would call unlinked. `dir` is the manifest's directory (`""` the
/// project's root).
///
/// Cargo's own spelling is followed: a path relative to the manifest, with
/// `./`, `..` and either separator. A target outside the project is no file
/// of the project's, and is left out.
pub fn manifest_roots(dir: &str, text: &str) -> Option<Vec<String>> {
    let manifest: toml::Table = text.parse().ok()?;
    let mut named: Vec<&str> = Vec::new();
    fn path_of(target: &toml::Value) -> Option<&str> {
        target.get("path").and_then(toml::Value::as_str)
    }
    if let Some(path) = manifest.get("lib").and_then(path_of) {
        named.push(path);
    }
    for kind in ["bin", "example", "test", "bench"] {
        if let Some(targets) = manifest.get(kind).and_then(toml::Value::as_array) {
            named.extend(targets.iter().filter_map(path_of));
        }
    }
    if let Some(build) = manifest
        .get("package")
        .and_then(|package| package.get("build"))
        .and_then(toml::Value::as_str)
    {
        named.push(build);
    }
    Some(
        named
            .into_iter()
            .filter_map(|path| beside(dir, path))
            .collect(),
    )
}

/// `path` relative to the directory `dir`, as a project-relative path with
/// `/` separators and no `.` or `..` in it — or `None` when it climbs out
/// of the project.
fn beside(dir: &str, path: &str) -> Option<String> {
    let mut parts: Vec<&str> = Vec::new();
    let joined = format!("{dir}/{}", path.replace('\\', "/"));
    for part in joined.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            part => parts.push(part),
        }
    }
    Some(parts.join("/"))
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
/// [`read`]'s answer without the crates it was about.
pub fn scan(root: &std::path::Path) -> Option<Vec<String>> {
    read(root).unlinked
}

/// Walk the project and read it: [`Reading::of`] over every `.rs` file and
/// every crate.
///
/// The same walk the tree uses, so a file the tree does not show cannot be
/// dimmed in it, and `target/` costs nothing. Reads every `.rs` in the
/// project for the `mod` declarations — a declaration can sit anywhere, and a
/// `src/lib.rs` that was not read is a crate whose whole module tree reads as
/// unlinked — and every manifest, for the targets it names by path.
pub fn read(root: &std::path::Path) -> Reading {
    let mut manifests: Vec<String> = Vec::new();
    let mut roots: Vec<String> = Vec::new();
    let mut sources: Vec<(String, String)> = Vec::new();

    let walk = crate::hidden::project_walk(root).build();

    for found in walk.flatten() {
        let path = found.path();
        let Some(relative) = crate::hidden::relative_slashed(root, path) else {
            continue;
        };
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
        match name.as_deref() {
            Some("Cargo.toml") => {
                let dir = relative.strip_suffix("Cargo.toml").unwrap_or_default();
                let dir = dir.trim_end_matches('/');
                // A manifest that cannot be read leaves its crate unjudged.
                let named = std::fs::read_to_string(path)
                    .ok()
                    .and_then(|text| manifest_roots(dir, &text));
                if let Some(named) = named {
                    manifests.push(dir.to_string());
                    roots.extend(named);
                }
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
    Reading::of(&files, &manifests, &roots)
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

    fn reading<'a>(files: &'a [(&'a str, &'a str)], manifests: &[&str]) -> Reading {
        let files: Vec<SourceFile<'_>> = files
            .iter()
            .map(|(path, text)| SourceFile { path, text })
            .collect();
        let manifests: Vec<String> = manifests.iter().map(|m| (*m).to_string()).collect();
        Reading::of(&files, &manifests, &[])
    }

    fn dim(reading: Option<&Reading>, server: &[&str]) -> Vec<String> {
        dimmed(reading, server.iter().copied())
            .into_iter()
            .collect()
    }

    /// The reading speaks for the files under a crate's `src/` and for no
    /// others: a root is linked, an undeclared file is not, and an
    /// integration test's helper or a binary's own tree is nobody's word of
    /// its.
    #[test]
    fn a_reading_has_an_opinion_only_where_it_judged() {
        let files = [
            ("src/main.rs", "mod engine;\n"),
            ("src/engine.rs", ""),
            ("src/orphan.rs", ""),
            ("tests/common/helper.rs", ""),
            ("src/bin/tool/helper.rs", ""),
        ];
        let read = reading(&files, &[""]);
        assert_eq!(read.about("src/orphan.rs"), Some(true));
        assert_eq!(read.about("src/engine.rs"), Some(false));
        assert_eq!(read.about("src/main.rs"), Some(false), "a root is linked");
        assert_eq!(read.about("tests/common/helper.rs"), None);
        assert_eq!(read.about("src/bin/tool/helper.rs"), None);
    }

    /// The case the rule is for: the `mod` line has just been added, the scan
    /// has read it, and rust-analyzer's `unlinked-file` still stands for the
    /// file, as it will until the server looks at that file again. Or-ed,
    /// the file would stay dim under the declaration that reaches it.
    #[test]
    fn where_the_scan_speaks_the_server_is_not_heard() {
        let declared = [("src/main.rs", "mod orphan;\n"), ("src/orphan.rs", "")];
        let read = reading(&declared, &[""]);
        assert!(
            dim(Some(&read), &["src/orphan.rs"]).is_empty(),
            "the scan says linked, and it wins over the server's stale word"
        );
        let undeclared = [("src/main.rs", "\n"), ("src/orphan.rs", "")];
        let read = reading(&undeclared, &[""]);
        assert_eq!(
            dim(Some(&read), &[]),
            ["src/orphan.rs"],
            "dimmed before the server has been told of the file at all"
        );
    }

    /// A refusal is not a clean bill: where the scan refused — `#[path]`
    /// somewhere — the server's verdict is the only one, and it is shown.
    #[test]
    fn a_refusal_leaves_the_server_to_answer() {
        let files = [
            ("src/lib.rs", "#[path = \"x.rs\"]\nmod thing;\n"),
            ("src/orphan.rs", ""),
        ];
        let read = reading(&files, &[""]);
        assert_eq!(read.unlinked(), None);
        assert_eq!(read.about("src/orphan.rs"), None);
        assert_eq!(dim(Some(&read), &["src/orphan.rs"]), ["src/orphan.rs"]);
        assert!(
            dim(Some(&read), &[]).is_empty(),
            "and with the server silent, nothing is claimed"
        );
    }

    /// No reading at all — not scanned yet, or not a Cargo project — and a
    /// file the reading does not judge are the server's alone.
    #[test]
    fn where_the_scan_is_silent_the_server_answers() {
        assert_eq!(dim(None, &["src/orphan.rs"]), ["src/orphan.rs"]);
        let files = [("src/lib.rs", "\n"), ("tests/common/helper.rs", "")];
        let read = reading(&files, &[""]);
        assert_eq!(
            dim(Some(&read), &["tests/common/helper.rs"]),
            ["tests/common/helper.rs"]
        );
    }

    /// A target a manifest names by path is an entry point cargo compiles,
    /// declared by nobody: `[lib] path = "src/core.rs"` is the crate's root,
    /// not a file in no crate — and the module it declares is linked.
    #[test]
    fn a_target_named_by_path_is_a_root() {
        let manifest = r#"
            [package]
            name = "flight"
            build = "tools/build.rs"

            [lib]
            path = "src/core.rs"

            [[bin]]
            name = "tool"
            path = "./src/tools/run.rs"

            [[example]]
            name = "far"
            path = "../shared/src/far.rs"

            [dependencies]
            other = { path = "../other" }
        "#;
        let roots = manifest_roots("flight", manifest).expect("the manifest parses");
        assert_eq!(
            roots,
            [
                "flight/src/core.rs",
                "flight/src/tools/run.rs",
                "shared/src/far.rs",
                "flight/tools/build.rs"
            ],
            "every target's path, the manifest's own spelling resolved, and no dependency's"
        );
        let files = [
            ("flight/src/core.rs", "mod attitude;\n"),
            ("flight/src/attitude.rs", ""),
            ("flight/src/tools/run.rs", ""),
            ("flight/src/orphan.rs", ""),
        ];
        let files: Vec<SourceFile<'_>> = files
            .iter()
            .map(|(path, text)| SourceFile { path, text })
            .collect();
        let read = Reading::of(&files, &["flight".to_string()], &roots);
        assert_eq!(
            read.unlinked(),
            Some(&["flight/src/orphan.rs".to_string()][..])
        );
        assert_eq!(read.about("flight/src/core.rs"), Some(false));
    }

    /// A manifest that does not parse cannot say where its targets are, and
    /// a root it hides would read as a file in no crate: its crate is left
    /// unjudged. And a target outside the project is nobody's file here.
    #[test]
    fn a_manifest_that_does_not_parse_names_nothing() {
        assert_eq!(manifest_roots("", "[package\nname = "), None);
        assert_eq!(
            manifest_roots("", "[[bin]]\nname = \"x\"\npath = \"../../x.rs\"\n"),
            Some(vec![])
        );
        assert_eq!(
            manifest_roots("", "[package]\nname = \"plain\"\n"),
            Some(vec![]),
            "no target named by path is no root, not a refusal"
        );
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
