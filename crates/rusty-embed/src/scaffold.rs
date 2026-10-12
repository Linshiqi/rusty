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

use crate::{
    error::{Error, Result},
    model::{Chip, CommandPlan},
};

/// Which way the calls go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Rust calls C: `cc` compiles `csrc/`, an `extern "C"` block declares it.
    RustCallsC,
    /// Rust calls C++: the same, with `cc` in C++ mode and the C++ behind
    /// an `extern "C"` interface — the only one Rust can call.
    RustCallsCpp,
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

/// The C-compiler precondition for scaffolding, pure: which compiler a chip's
/// C is compiled by, and the refusal when it is missing or unknown.
///
/// [`c_interop`] already refuses rather than lay half a scaffold over
/// somebody's code; this is the same rule applied to the other precondition,
/// which is not about the files at all: `cc` shells out to a cross compiler,
/// and four correct new files whose build cannot find one is a worse answer
/// than a refusal that names it. `on_path` is passed in so the rule is a
/// test.
pub fn c_compiler_gate(
    chip: Option<&Chip>,
    on_path: impl Fn(&str) -> bool,
) -> std::result::Result<(), String> {
    let Some(chip) = chip else {
        // No chip means no cross compiler to require; the host's `cc` is
        // whatever it is and not rusty's to judge.
        return Ok(());
    };
    match chip
        .c_compiler
        .as_ref()
        .map(|c| (c.binary.as_str(), c.install.as_str()))
    {
        Some((binary, install)) if !on_path(binary) => Err(format!(
            "This project builds for {}, so C in it is compiled by `{binary}`, and that is \
             not on PATH. Nothing has been written. Install it — {install} — and the \
             Environment page will show it before you try again.",
            chip.name,
        )),
        None => Err(format!(
            "rusty does not know which C compiler a {} project uses, so it will not scaffold \
             C it cannot say how to build. Nothing has been written.",
            chip.name,
        )),
        Some(_) => Ok(()),
    }
}

/// Write the scaffolding for one direction.
///
/// `chip` is the part the project builds for, when it has one: its C
/// compiler (`Chip::c_compiler`) is named in the build script, since `cc`'s
/// own guess for a bare-metal triple is a generic toolchain — for
/// `riscv32imc-unknown-none-elf`, `riscv32-unknown-elf-gcc` — that may not
/// be the one installed.
///
/// **An existing `build.rs` is joined, not refused.** The compiling goes
/// into a file of its own, `build_c.rs`, and the build script gains exactly
/// two lines: `mod build_c;` and a call to `build_c::compile()` at the top
/// of its `main`. Every embedded template has a build script (the link
/// scripts are passed there), so refusing one refused C for every part but
/// the Espressif ones. A `main` that cannot be found on one line is not
/// guessed at: the refusal names the two lines to add by hand.
///
/// Fails with [`Error::Exists`] before anything is written when a file it
/// would create is there, or [`Error::Write`] for a file that would not land.
pub fn c_interop(root: &Path, direction: Direction, chip: Option<&Chip>) -> Result<Scaffold> {
    let compiler = chip
        .and_then(|c| c.c_compiler.as_ref())
        .map(|c| c.binary.clone());
    let new_files: Vec<(&str, String)> = match direction {
        Direction::RustCallsC => vec![
            ("build_c.rs", build_c(false, compiler.as_deref())),
            ("csrc/vendor.c", VENDOR_C.to_string()),
            ("csrc/vendor.h", VENDOR_H.to_string()),
            ("src/vendor.rs", VENDOR_RS.to_string()),
        ],
        Direction::RustCallsCpp => vec![
            ("build_c.rs", build_c(true, compiler.as_deref())),
            ("csrc/vendor.cpp", VENDOR_CPP.to_string()),
            ("csrc/vendor.h", VENDOR_H.to_string()),
            ("src/vendor.rs", VENDOR_RS.to_string()),
        ],
        Direction::CCallsRust => vec![
            ("include/rusty_export.h", EXPORT_H.to_string()),
            ("src/exports.rs", EXPORTS_RS.to_string()),
        ],
    };

    // Refuse before writing anything: half a scaffold over somebody's code
    // is worse than none, and "it already existed" is only useful before
    // the first file lands.
    for (path, _) in &new_files {
        if root.join(path).exists() {
            return Err(Error::Exists {
                path: (*path).to_string(),
            });
        }
    }
    // The build script, worked out before the first write too: a merge that
    // cannot be made is a refusal, and it has to come while nothing changed.
    let build_script = match direction {
        Direction::CCallsRust => None,
        _ => Some(match std::fs::read_to_string(root.join("build.rs")) {
            Ok(text) => (merge_build_script(&text)?, true),
            Err(_) => (BUILD_RS.to_string(), false),
        }),
    };

    let mut written = Vec::new();
    let mut write = |path: &str, contents: &str| -> Result<()> {
        let full = root.join(path);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent).map_err(Error::writing(Path::new(path)))?;
        }
        std::fs::write(&full, contents).map_err(Error::writing(Path::new(path)))?;
        written.push(path.to_string());
        Ok(())
    };
    for (path, contents) in &new_files {
        write(path, contents)?;
    }
    let joined = match &build_script {
        Some((text, joined)) => {
            write("build.rs", text)?;
            *joined
        }
        None => false,
    };

    let script = if joined {
        "Your build.rs now declares `mod build_c;` and calls `build_c::compile()` first; \
         build_c.rs compiles everything in csrc/."
    } else {
        "build.rs calls build_c.rs, which compiles everything in csrc/."
    };
    Ok(match direction {
        Direction::RustCallsC | Direction::RustCallsCpp => Scaffold {
            written,
            command: Some(CommandPlan::new(
                "cargo",
                vec!["add".into(), "cc".into(), "--build".into()],
                "cc compiles the C sources into the crate; adding it through cargo keeps \
                 your Cargo.toml formatted the way you left it",
            )),
            next: format!(
                "`mod vendor;` in main.rs or lib.rs, then call `vendor::tick()`. {script}"
            ),
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

/// An existing build script with `mod build_c;` declared and
/// `build_c::compile();` called at the top of `main`, or the refusal that
/// names the two lines when its `main` cannot be found as `fn main() {` on
/// one line — exactly once.
pub(crate) fn merge_build_script(text: &str) -> Result<String> {
    let by_hand = || {
        Error::refused(
            "build.rs is there and rusty could not find one `fn main() {` line in it to call \
             the C build from. Nothing has been written. Add `mod build_c;` to build.rs and \
             call `build_c::compile();` from its `main`, then scaffold again.",
        )
    };
    if text.contains("build_c") {
        return Err(Error::refused(
            "build.rs already mentions build_c — the C build is there. Nothing has been written.",
        ));
    }
    let lines: Vec<&str> = text.split('\n').collect();
    let mains: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| {
            let line = line.trim();
            line.starts_with("fn main()") && line.ends_with('{')
        })
        .map(|(at, _)| at)
        .collect();
    let [main] = mains[..] else {
        return Err(by_hand());
    };
    // After the inner doc comments and attributes a crate root opens with:
    // an item before `//!` is a compile error.
    let module_at = lines
        .iter()
        .position(|line| {
            let line = line.trim_start();
            !(line.starts_with("//!") || line.starts_with("#![") || line.is_empty())
        })
        .unwrap_or(lines.len())
        .min(main);
    let mut out: Vec<String> = Vec::with_capacity(lines.len() + 3);
    for (at, line) in lines.iter().enumerate() {
        if at == module_at {
            out.push("mod build_c;".to_string());
            out.push(String::new());
        }
        out.push((*line).to_string());
        if at == main {
            out.push("    build_c::compile();".to_string());
        }
    }
    Ok(out.join("\n"))
}

/// What the C or C++ build script says, for a part whose compiler is
/// `compiler` (or the host's, with none).
fn build_c(cpp: bool, compiler: Option<&str>) -> String {
    let (extension, what) = if cpp { ("cpp", "C++") } else { ("c", "C") };
    let mut out = format!(
        "//! Compiles the {what} in csrc/ into this crate; build.rs calls it.\n\
         //!\n\
         //! Every .{extension} file in csrc/ is built and linked, so adding one needs\n\
         //! no change here. The rerun line matters: without it cargo caches the\n\
         //! object files and edits to the {what} are ignored until a clean build.\n\
         \n\
         pub fn compile() {{\n\
         \x20   println!(\"cargo:rerun-if-changed=csrc\");\n\
         \n\
         \x20   let mut build = cc::Build::new();\n"
    );
    if let Some(compiler) = compiler {
        let compiler = if cpp {
            compiler
                .strip_suffix("gcc")
                .map_or(compiler.to_string(), |stem| format!("{stem}g++"))
        } else {
            compiler.to_string()
        };
        out.push_str(&format!(
            "    // The part's cross compiler, by name: cc's own guess for a bare-metal\n\
             \x20   // target is a generic toolchain that may not be the one installed.\n\
             \x20   build.compiler(\"{compiler}\");\n"
        ));
    }
    if cpp {
        out.push_str(
            "    // Firmware C++: no exceptions, no RTTI, no guarded statics, and no\n\
             \x20   // libstdc++ linked in for a runtime the firmware does not have.\n\
             \x20   build\n\
             \x20       .cpp(true)\n\
             \x20       .cpp_link_stdlib(None)\n\
             \x20       .flag_if_supported(\"-fno-exceptions\")\n\
             \x20       .flag_if_supported(\"-fno-rtti\")\n\
             \x20       .flag_if_supported(\"-fno-threadsafe-statics\");\n",
        );
    }
    out.push_str(&format!(
        "    for entry in std::fs::read_dir(\"csrc\").expect(\"csrc/ exists\").flatten() {{\n\
         \x20       let path = entry.path();\n\
         \x20       if path.extension().is_some_and(|e| e == \"{extension}\") {{\n\
         \x20           build.file(path);\n\
         \x20       }}\n\
         \x20   }}\n\
         \x20   // Firmware {what} is freestanding: no libc, no host headers.\n\
         \x20   build.flag_if_supported(\"-ffreestanding\");\n\
         \x20   build.compile(\"vendor\");\n\
         }}\n"
    ));
    out
}

const BUILD_RS: &str = r#"//! The build script: the C in csrc/, compiled by build_c.rs.

mod build_c;

fn main() {
    build_c::compile();
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

const VENDOR_CPP: &str = r#"// Stand-in for the C++ you actually have, behind a C interface: Rust
// calls C, never C++ directly, so every function it calls is `extern "C"`.
#include "vendor.h"

namespace vendor {

class Counter {
  public:
    unsigned int next() { return ++value_; }

  private:
    unsigned int value_ = 0;
};

// Constant-initialised: no constructor runs before main, which firmware
// with no C++ start-up code would never call.
static Counter counter;

}  // namespace vendor

extern "C" unsigned int vendor_tick(void) {
    return vendor::counter.next();
}
"#;

const VENDOR_H: &str = r#"#ifndef VENDOR_H
#define VENDOR_H

#ifdef __cplusplus
extern "C" {
#endif

/* Increments an internal counter and returns it. */
unsigned int vendor_tick(void);

#ifdef __cplusplus
}
#endif

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

    /// The gate refuses with the compiler's name and its install route, and
    /// says in as many words that nothing was written.
    #[test]
    fn scaffolding_refuses_before_writing_when_the_cross_compiler_is_missing() {
        let xtensa = crate::chip::by_id("esp32").expect("the classic ESP32 is catalogued");
        let riscv = crate::chip::by_id("esp32c3").expect("the C3 is catalogued");
        let cortex = crate::chip::by_id("stm32f103").expect("an STM32 is catalogued");

        let missing = c_compiler_gate(Some(&xtensa), |_| false).unwrap_err();
        assert!(missing.contains("xtensa-esp-elf-gcc"), "{missing}");
        assert!(
            missing.contains("espup"),
            "the install route travels with the refusal: {missing}"
        );
        assert!(missing.contains("Nothing has been written"), "{missing}");

        let missing = c_compiler_gate(Some(&riscv), |_| false).unwrap_err();
        assert!(missing.contains("riscv32-esp-elf-gcc"), "{missing}");

        assert_eq!(
            c_compiler_gate(Some(&xtensa), |binary| binary == "xtensa-esp-elf-gcc"),
            Ok(()),
            "the right compiler on PATH is all it asks",
        );
        assert!(
            c_compiler_gate(Some(&riscv), |binary| binary == "xtensa-esp-elf-gcc").is_err(),
            "the other architecture's compiler does not count",
        );

        let unknown = c_compiler_gate(Some(&cortex), |_| true).unwrap_err();
        assert!(
            unknown.contains("does not know which C compiler"),
            "a part whose compiler rusty has not verified is refused, not guessed: {unknown}",
        );
        assert!(unknown.contains("Nothing has been written"), "{unknown}");

        assert_eq!(
            c_compiler_gate(None, |_| false),
            Ok(()),
            "no chip, no cross compiler to require"
        );
    }

    #[test]
    fn rust_calling_c_lands_a_build_script_and_a_declaration() {
        let dir = tempfile::tempdir().unwrap();
        let scaffold = c_interop(dir.path(), Direction::RustCallsC, None).expect("scaffolded");

        assert!(scaffold.written.contains(&"build.rs".to_string()));
        assert!(scaffold.written.contains(&"build_c.rs".to_string()));
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
        let scaffold = c_interop(dir.path(), Direction::CCallsRust, None).expect("scaffolded");

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
        std::fs::create_dir_all(dir.path().join("csrc")).unwrap();
        std::fs::write(
            dir.path().join("csrc/vendor.c"),
            "/* mine */
",
        )
        .unwrap();

        let error = c_interop(dir.path(), Direction::RustCallsC, None).unwrap_err();
        assert!(matches!(error, Error::Exists { .. }), "{error}");
        assert!(error.to_string().contains("csrc/vendor.c"), "{error}");
        assert!(
            !dir.path().join("build_c.rs").exists() && !dir.path().join("build.rs").exists(),
            "the refusal came before the first write, not after three",
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("csrc/vendor.c")).unwrap(),
            "/* mine */
",
            "the existing file is untouched",
        );
    }

    /// A template's build script — the link scripts are passed there — is
    /// joined with two lines rather than refused: the module, and the call
    /// at the top of `main`, after the inner docs a crate root opens with.
    #[test]
    fn an_existing_build_script_gains_two_lines_and_nothing_else() {
        let template = crate::playground::template("rp2040")
            .unwrap()
            .files
            .iter()
            .find(|(path, _)| *path == "build.rs")
            .unwrap()
            .1;
        let merged = merge_build_script(template).unwrap();
        let added: Vec<&str> = merged
            .lines()
            .filter(|line| !template.lines().any(|t| t == *line))
            .collect();
        assert_eq!(
            added,
            ["mod build_c;", "    build_c::compile();"],
            "{merged}"
        );
        let module = merged.find("mod build_c;").unwrap();
        assert!(
            merged[..module]
                .lines()
                .all(|l| l.starts_with("//!") || l.is_empty()),
            "after the inner docs: {merged}"
        );
        let main = merged.find("fn main() {").unwrap();
        assert!(
            merged[main..]
                .lines()
                .nth(1)
                .unwrap()
                .contains("build_c::compile();")
        );

        // A `main` it cannot find on one line is not guessed at.
        let refused = merge_build_script(
            "fn main()
{
}
",
        )
        .unwrap_err()
        .to_string();
        assert!(refused.contains("mod build_c;"), "{refused}");
        assert!(
            merge_build_script(
                "mod build_c;
fn main() {}
"
            )
            .is_err()
        );

        // And through the scaffold: the build script is joined in place.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("build.rs"), template).unwrap();
        c_interop(dir.path(), Direction::RustCallsC, None).unwrap();
        let joined = std::fs::read_to_string(dir.path().join("build.rs")).unwrap();
        assert!(joined.contains("build_c::compile();"), "{joined}");
        assert!(
            joined.contains("-Tlink.x"),
            "the template's own lines stay: {joined}"
        );
    }

    /// C++ goes through cc in C++ mode, behind an `extern "C"` interface, and
    /// the part's compiler is named — its g++ for C++.
    #[test]
    fn cpp_is_compiled_as_firmware_cpp_by_the_parts_compiler() {
        let chip = crate::chip::by_id("stm32f411ce").unwrap();
        let dir = tempfile::tempdir().unwrap();
        c_interop(dir.path(), Direction::RustCallsCpp, Some(&chip)).unwrap();
        let build = std::fs::read_to_string(dir.path().join("build_c.rs")).unwrap();
        assert!(
            build.contains("build.compiler(\"arm-none-eabi-g++\")"),
            "{build}"
        );
        assert!(build.contains(".cpp(true)"), "{build}");
        assert!(build.contains("-fno-exceptions"), "{build}");
        assert!(build.contains("cpp_link_stdlib(None)"), "{build}");
        let source = std::fs::read_to_string(dir.path().join("csrc/vendor.cpp")).unwrap();
        assert!(
            source.contains("extern \"C\" unsigned int vendor_tick"),
            "{source}"
        );
        let header = std::fs::read_to_string(dir.path().join("csrc/vendor.h")).unwrap();
        assert!(header.contains("#ifdef __cplusplus"), "{header}");

        let c3 = crate::chip::by_id("esp32c3").unwrap();
        let dir = tempfile::tempdir().unwrap();
        c_interop(dir.path(), Direction::RustCallsC, Some(&c3)).unwrap();
        let build = std::fs::read_to_string(dir.path().join("build_c.rs")).unwrap();
        assert!(
            build.contains("build.compiler(\"riscv32-esp-elf-gcc\")"),
            "{build}"
        );
    }
}
