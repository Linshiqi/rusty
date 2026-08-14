//! Meeting C, in whichever direction.
//!
//! Two things people actually need, and they are opposites:
//!
//! - **Rust calls C.** A vendor driver, a legacy algorithm, an SDK. `cc`
//!   compiles the sources into the crate and an `extern "C"` block declares
//!   them.
//! - **C calls Rust.** A Rust module inside existing C firmware — the
//!   incremental-migration path every real team takes. The crate becomes a
//!   `staticlib` and exports `#[unsafe(no_mangle)] extern "C"` functions
//!   behind a header.
//!
//! This writes the scaffolding and *refuses to touch anything that exists*.
//! It also does not edit `Cargo.toml`: `cargo add cc --build` is the
//! official way to add a build dependency, it is visible in the dock like
//! every other command rusty runs, and a manifest rewriter that eats a
//! comment or reorders a table is a workbench that loses somebody's work.

use std::path::Path;

use crate::model::CommandPlan;

/// Which way the calls go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Rust calls C: `cc` compiles `csrc/`, an `extern "C"` block declares it.
    RustCallsC,
    /// C calls Rust: a `staticlib` and a header for the C side to include.
    CCallsRust,
}

/// What a scaffolding run did, and what has to run next.
#[derive(Debug, Clone)]
pub struct Scaffold {
    /// Project-relative paths written, in the order they were created.
    pub written: Vec<String>,
    /// The dependency this needs, as a command the user can watch run —
    /// `None` when nothing has to be added.
    pub command: Option<CommandPlan>,
    /// What to do next, in one sentence: scaffolding that leaves someone
    /// guessing at the next step has not finished the job.
    pub next: String,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{path} already exists — rusty will not overwrite code you wrote")]
    Exists { path: String },
    #[error("could not write {path}: {source}")]
    Write {
        path: String,
        #[source]
        source: std::io::Error,
    },
}

/// Write the scaffolding for one direction.
pub fn c_interop(root: &Path, direction: Direction) -> Result<Scaffold, Error> {
    let files: &[(&str, &str)] = match direction {
        Direction::RustCallsC => &[
            ("build.rs", BUILD_RS),
            ("csrc/vendor.c", VENDOR_C),
            ("csrc/vendor.h", VENDOR_H),
            ("src/vendor.rs", VENDOR_RS),
        ],
        Direction::CCallsRust => &[
            ("include/rusty_export.h", EXPORT_H),
            ("src/exports.rs", EXPORTS_RS),
        ],
    };

    // Refuse before writing anything: half a scaffold over somebody's code
    // is worse than none, and "it already existed" is only useful before
    // the first file lands.
    for (path, _) in files {
        if root.join(path).exists() {
            return Err(Error::Exists {
                path: (*path).to_string(),
            });
        }
    }

    let mut written = Vec::new();
    for (path, contents) in files {
        let full = root.join(path);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent).map_err(|source| Error::Write {
                path: (*path).to_string(),
                source,
            })?;
        }
        std::fs::write(&full, contents).map_err(|source| Error::Write {
            path: (*path).to_string(),
            source,
        })?;
        written.push((*path).to_string());
    }

    Ok(match direction {
        Direction::RustCallsC => Scaffold {
            written,
            command: Some(CommandPlan {
                program: "cargo".to_string(),
                args: vec!["add".into(), "cc".into(), "--build".into()],
                display: "cargo add cc --build".to_string(),
                rationale: "cc compiles the C sources into the crate; adding it \
                            through cargo keeps your Cargo.toml formatted the way \
                            you left it"
                    .to_string(),
                warning: None,
            }),
            next: "`mod vendor;` in main.rs or lib.rs, then call \
                   `vendor::tick()`. The C is in csrc/ and build.rs compiles \
                   everything there."
                .to_string(),
        },
        Direction::CCallsRust => Scaffold {
            written,
            command: None,
            next: "Add `[lib]` with `crate-type = [\"staticlib\"]` to Cargo.toml, \
                   `mod exports;` beside it, and link the built .a from your C \
                   build with include/rusty_export.h on the include path."
                .to_string(),
        },
    })
}

const BUILD_RS: &str = r#"//! Compiles the C in csrc/ into this crate.
//!
//! Every .c file in csrc/ is built and linked; adding one needs no change
//! here. The rerun line matters: without it cargo caches the object files
//! and edits to the C are silently ignored until a clean build.

fn main() {
    println!("cargo:rerun-if-changed=csrc");

    let mut build = cc::Build::new();
    for entry in std::fs::read_dir("csrc").expect("csrc/ exists").flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "c") {
            build.file(path);
        }
    }
    // Firmware C is freestanding: no libc, no host headers.
    build.flag_if_supported("-ffreestanding");
    build.compile("vendor");
}
"#;

const VENDOR_C: &str = r#"/* Stand-in for the C you actually have: a vendor driver, a legacy
 * algorithm, a checksum somebody validated a decade ago. */
#include "vendor.h"

static unsigned int ticks;

unsigned int vendor_tick(void) {
    return ++ticks;
}
"#;

const VENDOR_H: &str = r#"#ifndef VENDOR_H
#define VENDOR_H

/* Increments an internal counter and returns it. */
unsigned int vendor_tick(void);

#endif /* VENDOR_H */
"#;

const VENDOR_RS: &str = r#"//! The C in csrc/, declared for Rust.
//!
//! Hand-written rather than generated: two functions do not need bindgen,
//! and a hand-written declaration is one you can read. For a real SDK —
//! hundreds of functions, macros, packed structs — add bindgen as a build
//! dependency and generate this module instead.

unsafe extern "C" {
    fn vendor_tick() -> core::ffi::c_uint;
}

/// Safe because the C keeps its counter in a static and touches nothing
/// else. That sentence is the whole job of a wrapper like this one: state
/// why the `unsafe` below is sound, or do not write it.
pub fn tick() -> u32 {
    unsafe { vendor_tick() }
}
"#;

const EXPORT_H: &str = r#"#ifndef RUSTY_EXPORT_H
#define RUSTY_EXPORT_H

#include <stdint.h>

/* Implemented in Rust, linked from the staticlib this crate builds.
 *
 * For an API larger than this, generate the header with cbindgen instead
 * of maintaining two declarations of the same thing by hand. */
uint32_t rust_tick(void);

#endif /* RUSTY_EXPORT_H */
"#;

const EXPORTS_RS: &str = r#"//! What C is allowed to call.
//!
//! Every function here is a public API in a language with no namespaces,
//! so the names carry a prefix and the header declares exactly these.

/// Increments a counter and returns it — the shape of the smallest useful
/// export: no allocation, no panics, and a return type C has.
///
/// Panicking across an FFI boundary is undefined behaviour, so anything
/// that can fail returns a code rather than unwinding.
#[unsafe(no_mangle)]
pub extern "C" fn rust_tick() -> u32 {
    use core::sync::atomic::{AtomicU32, Ordering};
    static TICKS: AtomicU32 = AtomicU32::new(0);
    TICKS.fetch_add(1, Ordering::Relaxed) + 1
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rust_calling_c_lands_a_build_script_and_a_declaration() {
        let dir = tempfile::tempdir().unwrap();
        let scaffold = c_interop(dir.path(), Direction::RustCallsC).expect("scaffolded");

        assert!(scaffold.written.contains(&"build.rs".to_string()));
        assert!(scaffold.written.contains(&"csrc/vendor.c".to_string()));
        assert!(dir.path().join("csrc/vendor.h").is_file());

        let command = scaffold.command.expect("a dependency to add");
        assert_eq!(
            command.display, "cargo add cc --build",
            "the manifest is edited by cargo, not by rusty",
        );

        let declaration = std::fs::read_to_string(dir.path().join("src/vendor.rs")).unwrap();
        assert!(
            declaration.contains("unsafe extern \"C\""),
            "the declaration is what makes the C callable: {declaration}",
        );
    }

    #[test]
    fn c_calling_rust_exports_behind_a_header() {
        let dir = tempfile::tempdir().unwrap();
        let scaffold = c_interop(dir.path(), Direction::CCallsRust).expect("scaffolded");

        let exports = std::fs::read_to_string(dir.path().join("src/exports.rs")).unwrap();
        assert!(exports.contains("#[unsafe(no_mangle)]"));
        assert!(exports.contains("pub extern \"C\" fn rust_tick"));

        let header = std::fs::read_to_string(dir.path().join("include/rusty_export.h")).unwrap();
        assert!(
            header.contains("uint32_t rust_tick(void);"),
            "the header declares exactly what Rust exports: {header}",
        );
        assert!(
            scaffold.next.contains("staticlib"),
            "and it says what is still needed: {}",
            scaffold.next,
        );
    }

    /// The refusal is the important behaviour: half a scaffold written over
    /// somebody's code cannot be undone by an error message.
    #[test]
    fn nothing_is_written_when_anything_would_be_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("build.rs"), "fn main() {}\n").unwrap();

        let error = c_interop(dir.path(), Direction::RustCallsC).unwrap_err();
        assert!(matches!(error, Error::Exists { .. }), "{error}");
        assert!(
            !dir.path().join("csrc").exists(),
            "the refusal came before the first write, not after three",
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("build.rs")).unwrap(),
            "fn main() {}\n",
            "the existing file is untouched",
        );
    }
}
