//! What a language is to the window, in one table: the files that are it,
//! the name the status bar gives it, the line comment Ctrl+/ toggles, the
//! server that reads it and what formats it on save.
//!
//! Rust, C and C++ are the languages rusty is for, and they are rows of the
//! same table rather than a `match` per feature — a feature that asked
//! `ends_with(".rs")` was a feature C did not have, and sixteen such places
//! were once the whole of what kept the old editor Rust-only.

use rusty_lsp::ServerKind;

/// What formats a language on save.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Formatter {
    /// rustfmt, always: every Rust project is formatted the one way.
    Rustfmt,
    /// clang-format, only where the project carries a `.clang-format`: C
    /// has no one style, and a style rusty chose rewritten over a vendor's
    /// file is a diff nobody asked for.
    ClangFormat,
}

pub struct Language {
    /// The status bar's name for it, which is also the painter's.
    pub name: &'static str,
    pub extensions: &'static [&'static str],
    /// The line comment Ctrl+/ toggles; none where the language has none.
    pub comment: Option<&'static str>,
    /// The server told about its files.
    pub server: Option<ServerKind>,
    pub formatter: Option<Formatter>,
}

const fn language(
    name: &'static str,
    extensions: &'static [&'static str],
    comment: Option<&'static str>,
) -> Language {
    Language {
        name,
        extensions,
        comment,
        server: None,
        formatter: None,
    }
}

pub const ALL: &[Language] = &[
    Language {
        server: Some(ServerKind::RustAnalyzer),
        formatter: Some(Formatter::Rustfmt),
        ..language("Rust", &["rs"], Some("//"))
    },
    Language {
        server: Some(ServerKind::Clangd),
        formatter: Some(Formatter::ClangFormat),
        // A header is C here, as clangd takes it: syntect gives `.h` to
        // Objective-C, whose grammar colours firmware wrongly.
        ..language("C", &["c", "h"], Some("//"))
    },
    Language {
        server: Some(ServerKind::Clangd),
        formatter: Some(Formatter::ClangFormat),
        ..language(
            "C++",
            &["cc", "cpp", "cxx", "hh", "hpp", "hxx", "ino"],
            Some("//"),
        )
    },
    language("TOML", &["toml"], Some("#")),
    language("Markdown", &["md"], None),
    language("JSON", &["json"], None),
    language("YAML", &["yml", "yaml"], Some("#")),
    language("Python", &["py"], Some("#")),
    language("Shell", &["sh"], Some("#")),
    language("Linker script", &["x", "ld"], None),
    language("Assembly", &["s", "asm"], Some("//")),
    language("INI", &["ini"], Some(";")),
    language("CMake", &["cmake"], Some("#")),
];

/// The language of `path`, by its name or its extension — `CMakeLists.txt`
/// is CMake whatever `.txt` says.
pub fn of(path: &str) -> Option<&'static Language> {
    let name = path.rsplit('/').next().unwrap_or(path);
    if name == "CMakeLists.txt" {
        return ALL.iter().find(|l| l.name == "CMake");
    }
    let extension = name.rsplit_once('.')?.1;
    ALL.iter().find(|l| {
        l.extensions
            .iter()
            .any(|e| e.eq_ignore_ascii_case(extension))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The table and the client agree on which server reads what: a file
    /// the table hands to clangd that the client would not tell it about
    /// is a C file with no squiggles and no reason given.
    #[test]
    fn every_extension_goes_to_the_server_that_reads_it() {
        for language in ALL {
            for extension in language.extensions {
                let path = format!("src/a.{extension}");
                for server in [ServerKind::RustAnalyzer, ServerKind::Clangd] {
                    if server.serves(&path) {
                        assert_eq!(
                            language.server,
                            Some(server),
                            "{path} is {}'s, and {} says it is {}'s",
                            server.label(),
                            language.name,
                            language.server.map_or("nobody", ServerKind::label)
                        );
                    }
                }
                if let Some(server) = language.server {
                    assert!(
                        server.serves(&path),
                        "{} does not read {path}",
                        server.label()
                    );
                }
            }
        }
    }

    #[test]
    fn a_file_is_its_language_by_name_or_extension() {
        assert_eq!(of("core/src/lib.rs").map(|l| l.name), Some("Rust"));
        assert_eq!(of("firmware/main.c").map(|l| l.name), Some("C"));
        assert_eq!(of("drivers/gpio.H").map(|l| l.name), Some("C"));
        assert_eq!(of("lib/servo.cpp").map(|l| l.name), Some("C++"));
        assert_eq!(of("CMakeLists.txt").map(|l| l.name), Some("CMake"));
        assert_eq!(of("startup.S").map(|l| l.name), Some("Assembly"));
        assert!(of("notes.txt").is_none());
        assert!(of("Makefile").is_none());
    }
}
