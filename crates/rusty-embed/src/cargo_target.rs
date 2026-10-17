//! Which package of a cargo workspace — and which of its targets — a source
//! file is compiled in, and the arguments that select them, as
//! rust-analyzer's runnables pass them: `--package core-part --lib`.
//!
//! A lens over a test in a member of a workspace whose root is itself a
//! package ran `cargo test <filter>` at the root, which tests the root
//! package and nothing else: the filter matched nothing there, and the run
//! exited zero having run nothing — on screen a test that passed. The
//! package is what was missing; the target only narrows what is built.
//!
//! **Read off `cargo metadata --no-deps`**: cargo's own list of the
//! workspace's packages and their targets — each target's kind, name, root
//! file and required features, whether autodiscovered or named by `path` —
//! and the workspace's root, without resolving the dependency graph, so it
//! needs no lockfile and no network: 134 ms on a workspace of two packages
//! and 231 ms on rusty's own, measured on Windows. Not `rusty_core`'s
//! workspace, which resolves the whole graph — 812 ms on rusty's and
//! unbounded where no lockfile exists yet — for a click that only needs
//! the members. And not a walk of the manifests, which would have to
//! re-derive cargo's autodiscovery rules to get the same answer.
//!
//! **The package is the one whose directory holds the file most closely**;
//! a file in none of the workspace's packages is refused by name, never
//! run at the root. **The target is the one the file is the root of**; a
//! module is its target's when exactly one target's root sits in a
//! directory above it — a library's modules under `src/` when the package
//! has no `src/main.rs` beside it. Where two could hold it (`src/foo.rs`
//! with both `lib.rs` and `main.rs`) none is named, and `--package` alone
//! runs every target of the package under the filter: slower, never wrong.
//! Following `mod` lines to decide would be a parser's job, and `#[path]`
//! puts a module anywhere.
//!
//! **The package's directory is where its test runs.** `cargo test` starts
//! each test binary in the directory of the package that built it
//! (`CARGO_MANIFEST_DIR`), so a debugged test is started there too, or a
//! fixture it reads by a relative path is found by one and not the other.
//!
//! **The whole suite is `test --workspace` wherever the root declares a
//! workspace** ([`suite_args`]), for the same reason the lens names its
//! package: plain `cargo test` at a root that is a package with members
//! tests the root package and nothing else.

use std::path::{Path, PathBuf};

use crate::process;

/// A workspace as `cargo metadata --no-deps` lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Workspace {
    /// The workspace's root: where cargo runs rustc, so what every relative
    /// path a compiler or a panic prints is relative to.
    pub root: PathBuf,
    pub packages: Vec<Package>,
}

/// One member of the workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Package {
    pub name: String,
    /// The directory its `Cargo.toml` is in.
    pub dir: PathBuf,
    pub targets: Vec<Target>,
}

/// One of a package's targets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub kind: TargetKind,
    pub name: String,
    /// Its root file: `src/lib.rs`, `tests/it.rs`.
    pub root: PathBuf,
    /// Features cargo refuses to build it without.
    pub required_features: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetKind {
    /// The library, whatever its crate types.
    Lib,
    Bin,
    Test,
    Example,
    Bench,
    /// `build.rs`, which no test runs in.
    BuildScript,
}

/// What a file selects: the package, and the target where the file says
/// which.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    /// The workspace's root, as [`Workspace::root`].
    pub workspace: PathBuf,
    pub package: String,
    /// The directory its `Cargo.toml` is in: where `cargo test` runs a test
    /// binary (`CARGO_MANIFEST_DIR`), so where a debugged test runs too.
    pub package_dir: PathBuf,
    /// `None` where more than one target could hold the file: every target
    /// of the package is run.
    pub target: Option<(TargetKind, String)>,
    /// The target's required features, which cargo would otherwise refuse
    /// to build it without.
    pub features: Vec<String>,
}

impl Selection {
    /// The arguments that select it, after cargo's subcommand:
    /// `--package core-part --lib`, `--package app --test it`,
    /// rust-analyzer's spelling.
    pub fn args(&self) -> Vec<String> {
        let mut args = vec!["--package".to_string(), self.package.clone()];
        match &self.target {
            Some((TargetKind::Lib, _)) => args.push("--lib".to_string()),
            Some((kind, name)) => {
                let flag = match kind {
                    TargetKind::Bin => "--bin",
                    TargetKind::Test => "--test",
                    TargetKind::Example => "--example",
                    TargetKind::Bench => "--bench",
                    TargetKind::Lib | TargetKind::BuildScript => return args,
                };
                args.extend([flag.to_string(), name.clone()]);
            }
            None => {}
        }
        if !self.features.is_empty() {
            args.extend(["--features".to_string(), self.features.join(",")]);
        }
        args
    }

    /// Where to run cargo for it: the opened project, where it lies inside
    /// the workspace — cargo finds the same workspace from there, and reads
    /// the configuration the project's own directory holds — and the
    /// workspace's root otherwise: a workspace of its own inside the
    /// project, or a project opened above it.
    pub fn run_in(&self, opened: &Path) -> PathBuf {
        if within_or_at(opened, &self.workspace) {
            opened.to_path_buf()
        } else {
            self.workspace.clone()
        }
    }
}

/// Why a file selects nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// `cargo metadata` named no workspace, in its own words.
    NoWorkspace(String),
    /// The file is in none of the workspace's packages: an excluded crate's
    /// file seen from outside, a file beside the manifests.
    NoPackage { file: PathBuf, workspace: PathBuf },
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refusal::NoWorkspace(said) => {
                write!(f, "`cargo metadata` named no workspace here: {said}")
            }
            Refusal::NoPackage { file, workspace } => write!(
                f,
                "{} is in none of the packages of the workspace at {}, so there is no package \
                 to run its tests in; running them at the root would run nothing.",
                file.display(),
                workspace.display()
            ),
        }
    }
}

impl std::error::Error for Refusal {}

/// The arguments that test the whole of the project at `root`, after
/// `cargo`: `test --workspace` where its manifest declares a workspace, as
/// rust-analyzer's "test all" runs it, and plain `test` where it is one
/// package. Plain `test` at a root that is a package *and* a workspace
/// tests the root package alone — the members' tests never ran from the
/// title bar's Test, and the run said it had passed. A member cargo
/// `exclude`s, the firmware crate, stays out with `--workspace` as without.
/// A manifest that cannot be read is tested plainly, and cargo says why.
/// Either way with `--no-fail-fast`, as CI runs it: one failing test
/// binary stopped the rest, and the root's tests after a failing member's
/// were never run nor said not to have been.
pub fn suite_args(root: &Path) -> Vec<String> {
    let manifest = std::fs::read_to_string(root.join("Cargo.toml")).unwrap_or_default();
    suite_args_for(&manifest)
}

/// [`suite_args`] for the text of the root's `Cargo.toml`.
pub fn suite_args_for(manifest: &str) -> Vec<String> {
    let workspace = manifest
        .parse::<toml::Table>()
        .is_ok_and(|table| table.contains_key("workspace"));
    let mut args = vec!["test".to_string()];
    if workspace {
        args.push("--workspace".to_string());
    }
    args.push("--no-fail-fast".to_string());
    args
}

/// The workspace `dir` is in, as `cargo metadata --no-deps` lists it.
pub fn workspace_of(dir: &Path) -> Result<Workspace, Refusal> {
    let output = process::command("cargo")
        .args(["metadata", "--no-deps", "--format-version", "1"])
        .current_dir(dir)
        .output()
        .map_err(|e| Refusal::NoWorkspace(format!("cargo could not be run: {e}")))?;
    if !output.status.success() {
        return Err(Refusal::NoWorkspace(
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ));
    }
    parse(&String::from_utf8_lossy(&output.stdout))
}

/// What `file` selects, asked of the workspace its own directory is in —
/// which need not be the opened project's: a workspace of its own nested
/// inside it is found from there.
pub fn selection_for(file: &Path) -> Result<Selection, Refusal> {
    let dir = file.parent().unwrap_or(file);
    select(&workspace_of(dir)?, file)
}

/// `cargo metadata`'s JSON as a [`Workspace`].
pub fn parse(json: &str) -> Result<Workspace, Refusal> {
    let value: serde_json::Value = serde_json::from_str(json)
        .map_err(|e| Refusal::NoWorkspace(format!("its answer did not read as JSON: {e}")))?;
    let root = value["workspace_root"]
        .as_str()
        .map(PathBuf::from)
        .ok_or_else(|| Refusal::NoWorkspace("its answer named no workspace_root".to_string()))?;
    let packages = value["packages"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|package| {
            let name = package["name"].as_str()?.to_string();
            let dir = PathBuf::from(package["manifest_path"].as_str()?)
                .parent()?
                .to_path_buf();
            let targets = package["targets"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(target)
                .collect();
            Some(Package { name, dir, targets })
        })
        .collect();
    Ok(Workspace { root, packages })
}

fn target(value: &serde_json::Value) -> Option<Target> {
    let kinds: Vec<&str> = value["kind"]
        .as_array()?
        .iter()
        .filter_map(serde_json::Value::as_str)
        .collect();
    let kind = kinds.iter().find_map(|kind| match *kind {
        "bin" => Some(TargetKind::Bin),
        "test" => Some(TargetKind::Test),
        "example" => Some(TargetKind::Example),
        "bench" => Some(TargetKind::Bench),
        "custom-build" => Some(TargetKind::BuildScript),
        "lib" | "rlib" | "dylib" | "cdylib" | "staticlib" | "proc-macro" => Some(TargetKind::Lib),
        _ => None,
    })?;
    Some(Target {
        kind,
        name: value["name"].as_str()?.to_string(),
        root: PathBuf::from(value["src_path"].as_str()?),
        required_features: value["required-features"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(serde_json::Value::as_str)
            .map(str::to_string)
            .collect(),
    })
}

/// What `file` selects in `workspace`.
pub fn select(workspace: &Workspace, file: &Path) -> Result<Selection, Refusal> {
    let package = workspace
        .packages
        .iter()
        .filter(|package| within(file, &package.dir))
        .max_by_key(|package| key(&package.dir).len())
        .ok_or_else(|| Refusal::NoPackage {
            file: file.to_path_buf(),
            workspace: workspace.root.clone(),
        })?;
    let targets = package
        .targets
        .iter()
        .filter(|target| target.kind != TargetKind::BuildScript);
    let target = match targets.clone().find(|t| key(&t.root) == key(file)) {
        Some(root) => Some(root),
        None => {
            let mut holding = targets.filter(|t| t.root.parent().is_some_and(|d| within(file, d)));
            match (holding.next(), holding.next()) {
                (Some(only), None) => Some(only),
                _ => None,
            }
        }
    };
    Ok(Selection {
        workspace: workspace.root.clone(),
        package: package.name.clone(),
        package_dir: package.dir.clone(),
        target: target.map(|t| (t.kind, t.name.clone())),
        features: target
            .map(|t| t.required_features.clone())
            .unwrap_or_default(),
    })
}

/// Whether `path` is below `dir`.
fn within(path: &Path, dir: &Path) -> bool {
    let dir = key(dir);
    key(path).starts_with(&format!("{}/", dir.trim_end_matches('/')))
}

fn within_or_at(path: &Path, dir: &Path) -> bool {
    key(path).trim_end_matches('/') == key(dir).trim_end_matches('/') || within(path, dir)
}

/// A path's spelling made comparable: the links on it resolved where it
/// exists, no verbatim prefix, forward slashes, no `.` parts, a drive
/// letter folded — and on Windows, where the whole path folds, all of it.
///
/// Resolved, because the two sides are spelled by two different parties.
/// `cargo metadata` answers with the paths the filesystem holds — the
/// directory it is run in is `getcwd`'s, every link on it followed — while
/// the file is spelled as the project was opened. On macOS a temporary
/// directory is `/var/folders/…` and cargo says `/private/var/folders/…`
/// of the same place, and a project opened through a symlink anywhere is
/// the same two spellings of one directory: compared as text they shared
/// no prefix, and a file was refused as in no package of the workspace it
/// plainly sat in. A path that does not exist is compared as spelled —
/// there is nothing to resolve, and nothing is guessed.
fn key(path: &Path) -> String {
    let resolved = path.canonicalize();
    let path = resolved.as_deref().unwrap_or(path);
    let text = path.to_string_lossy();
    let text = text
        .strip_prefix(r"\\?\")
        .unwrap_or(&text)
        .replace('\\', "/");
    let text: Vec<&str> = text
        .split('/')
        .enumerate()
        .filter(|(at, part)| *at == 0 || (!part.is_empty() && *part != "."))
        .map(|(_, part)| part)
        .collect();
    let text = text.join("/");
    // Folded where the disk folds: Windows, and macOS, whose default volume
    // is case-insensitive too.
    if cfg!(any(windows, target_os = "macos")) {
        return text.to_lowercase();
    }
    match text.as_bytes() {
        [drive, b':', ..] if drive.is_ascii_alphabetic() => {
            format!("{}{}", drive.to_ascii_lowercase() as char, &text[1..])
        }
        _ => text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn package(name: &str, dir: &str, targets: &[(TargetKind, &str, &str)]) -> Package {
        Package {
            name: name.to_string(),
            dir: PathBuf::from(dir),
            targets: targets
                .iter()
                .map(|(kind, name, root)| Target {
                    kind: *kind,
                    name: name.to_string(),
                    root: PathBuf::from(format!("{dir}/{root}")),
                    required_features: Vec::new(),
                })
                .collect(),
        }
    }

    /// A root package with a member beside it, the case that ran nothing.
    fn workspace() -> Workspace {
        Workspace {
            root: PathBuf::from("/w"),
            packages: vec![
                package(
                    "top",
                    "/w",
                    &[
                        (TargetKind::Lib, "top", "src/lib.rs"),
                        (TargetKind::Bin, "top", "src/main.rs"),
                        (TargetKind::Bin, "tool", "src/bin/tool.rs"),
                        (TargetKind::BuildScript, "build-script-build", "build.rs"),
                    ],
                ),
                package(
                    "core-part",
                    "/w/core",
                    &[
                        (TargetKind::Lib, "core_part", "src/lib.rs"),
                        (TargetKind::Test, "it", "tests/it.rs"),
                        (TargetKind::Example, "demo", "examples/demo.rs"),
                        (TargetKind::Bench, "speed", "benches/speed.rs"),
                    ],
                ),
            ],
        }
    }

    fn args(file: &str) -> Vec<String> {
        select(&workspace(), Path::new(file))
            .expect("selected")
            .args()
    }

    /// A member's file selects the member, not the root package that holds
    /// it in its directory too: the closest package wins.
    #[test]
    fn a_members_file_selects_the_member_and_its_target() {
        assert_eq!(
            args("/w/core/src/lib.rs"),
            ["--package", "core-part", "--lib"]
        );
        assert_eq!(
            args("/w/core/src/math/vector.rs"),
            ["--package", "core-part", "--lib"],
            "a module of the only target under src/",
        );
        assert_eq!(
            args("/w/core/tests/it.rs"),
            ["--package", "core-part", "--test", "it"]
        );
        assert_eq!(
            args("/w/core/examples/demo.rs"),
            ["--package", "core-part", "--example", "demo"]
        );
        assert_eq!(
            args("/w/core/benches/speed.rs"),
            ["--package", "core-part", "--bench", "speed"]
        );
        let dir = |file: &str| {
            select(&workspace(), Path::new(file))
                .expect("selected")
                .package_dir
        };
        assert_eq!(dir("/w/core/tests/it.rs"), PathBuf::from("/w/core"));
        assert_eq!(dir("/w/src/main.rs"), PathBuf::from("/w"));
    }

    /// The root package's own files select it — a binary by name, the
    /// library by `--lib`.
    #[test]
    fn the_root_packages_files_select_the_root_package() {
        assert_eq!(args("/w/src/lib.rs"), ["--package", "top", "--lib"]);
        assert_eq!(args("/w/src/main.rs"), ["--package", "top", "--bin", "top"]);
        assert_eq!(
            args("/w/src/bin/tool.rs"),
            ["--package", "top", "--bin", "tool"]
        );
    }

    /// `src/foo.rs` beside both `lib.rs` and `main.rs` could be either's
    /// module: no target is named, and the package runs them all.
    #[test]
    fn a_module_two_targets_could_hold_names_no_target() {
        assert_eq!(args("/w/src/foo.rs"), ["--package", "top"]);
        // Two integration tests share `tests/`, so their helper is neither's.
        let mut shared = workspace();
        shared.packages[1].targets.push(Target {
            kind: TargetKind::Test,
            name: "other".to_string(),
            root: PathBuf::from("/w/core/tests/other.rs"),
            required_features: Vec::new(),
        });
        assert_eq!(
            select(&shared, Path::new("/w/core/tests/common/mod.rs"))
                .expect("selected")
                .args(),
            ["--package", "core-part"]
        );
    }

    /// A file in no package — beside the manifests of a virtual
    /// workspace, or in an excluded crate seen from outside — is refused
    /// by name, never run at the root.
    #[test]
    fn a_file_in_no_package_is_refused_by_name() {
        let refused =
            select(&workspace(), Path::new("/elsewhere/src/lib.rs")).expect_err("in no package");
        assert!(matches!(refused, Refusal::NoPackage { .. }));
        assert!(refused.to_string().contains("/elsewhere/src/lib.rs"));
        // `/w/core2` is not inside `/w/core` because it starts with its name.
        let mut near = workspace();
        near.packages.remove(0);
        assert!(select(&near, Path::new("/w/core2/src/lib.rs")).is_err());
    }

    /// A target cargo refuses to build without features gets them, as
    /// rust-analyzer passes them.
    #[test]
    fn required_features_are_passed() {
        let mut needs = workspace();
        needs.packages[1].targets[1].required_features = vec!["std".into(), "log".into()];
        assert_eq!(
            select(&needs, Path::new("/w/core/tests/it.rs"))
                .expect("selected")
                .args(),
            [
                "--package",
                "core-part",
                "--test",
                "it",
                "--features",
                "std,log"
            ]
        );
    }

    /// Cargo runs where the project was opened when that is inside the
    /// workspace, and at the workspace's root when it is not.
    #[test]
    fn cargo_runs_in_the_project_unless_the_workspace_is_elsewhere() {
        let chosen = select(&workspace(), Path::new("/w/core/src/lib.rs")).expect("selected");
        assert_eq!(chosen.run_in(Path::new("/w")), PathBuf::from("/w"));
        assert_eq!(
            chosen.run_in(Path::new("/w/core")),
            PathBuf::from("/w/core")
        );
        assert_eq!(chosen.run_in(Path::new("/")), PathBuf::from("/w"));
        assert_eq!(chosen.run_in(Path::new("/other")), PathBuf::from("/w"));
    }

    /// A project opened through a symlink — on macOS every temporary
    /// directory is one, `/var` for `/private/var` — spells its files one
    /// way while cargo answers with the other. The file still selects its
    /// package and its target, and cargo still runs where the project was
    /// opened, spelled as it was opened.
    #[cfg(unix)]
    #[test]
    fn a_file_reached_through_a_symlink_selects_its_package() {
        let dir = tempfile::tempdir().expect("a directory");
        let real = dir.path().canonicalize().expect("there");
        std::fs::create_dir_all(real.join("core").join("src")).expect("a directory");
        std::fs::write(real.join("core").join("src").join("lib.rs"), "").expect("written");
        let link = real.join("link");
        std::os::unix::fs::symlink(&real, &link).expect("linked");
        let ws = Workspace {
            root: real.clone(),
            packages: vec![package(
                "core-part",
                real.join("core").to_str().expect("utf-8"),
                &[(TargetKind::Lib, "core_part", "src/lib.rs")],
            )],
        };
        let file = link.join("core").join("src").join("lib.rs");
        let chosen = select(&ws, &file).expect("selected through the link");
        assert_eq!(chosen.args(), ["--package", "core-part", "--lib"]);
        assert_eq!(chosen.run_in(&link), link);
        assert_eq!(chosen.run_in(&link.join("core")), link.join("core"));
        // And the other way about: the workspace as opened, the file as
        // cargo would spell it.
        let mut opened = ws.clone();
        opened.root = link.clone();
        opened.packages[0].dir = link.join("core");
        let chosen = select(&opened, &real.join("core").join("src").join("lib.rs"))
            .expect("selected under the link");
        assert_eq!(chosen.args(), ["--package", "core-part", "--lib"]);
    }

    /// The spelling of a path does not decide: separators, a `.` and on
    /// Windows the case.
    #[test]
    fn paths_compare_as_paths() {
        let ws = Workspace {
            root: PathBuf::from(r"E:\w"),
            packages: vec![package(
                "core-part",
                r"E:\w\core",
                &[(TargetKind::Lib, "core_part", r"src\lib.rs")],
            )],
        };
        let chosen = select(&ws, Path::new("e:/w/./core/src/lib.rs")).expect("selected");
        assert_eq!(chosen.args(), ["--package", "core-part", "--lib"]);
    }

    /// `cargo metadata`'s own answer, abridged from a real run on a root
    /// package with a member: every crate type of a library is the
    /// library, and the build script is no target to test.
    #[test]
    fn metadata_is_read_as_cargo_writes_it() {
        let json = r#"{"packages":[
            {"name":"core-part","manifest_path":"/w/core/Cargo.toml","targets":[
                {"kind":["cdylib","rlib"],"crate_types":["cdylib","rlib"],"name":"core_part","src_path":"/w/core/src/lib.rs","required-features":[]},
                {"kind":["test"],"crate_types":["bin"],"name":"it","src_path":"/w/core/tests/it.rs","required-features":["std"]},
                {"kind":["custom-build"],"crate_types":["bin"],"name":"build-script-build","src_path":"/w/core/build.rs"}
            ]},
            {"name":"top","manifest_path":"/w/Cargo.toml","targets":[
                {"kind":["lib"],"crate_types":["lib"],"name":"top","src_path":"/w/src/lib.rs"}
            ]}],
            "workspace_members":[],"workspace_root":"/w","target_directory":"/w/target","version":1}"#;
        let ws = parse(json).expect("read");
        assert_eq!(ws.root, PathBuf::from("/w"));
        assert_eq!(ws.packages.len(), 2);
        let core = &ws.packages[0];
        assert_eq!(core.dir, PathBuf::from("/w/core"));
        assert_eq!(
            core.targets
                .iter()
                .map(|t| (t.kind, t.name.as_str()))
                .collect::<Vec<_>>(),
            [
                (TargetKind::Lib, "core_part"),
                (TargetKind::Test, "it"),
                (TargetKind::BuildScript, "build-script-build"),
            ]
        );
        assert_eq!(core.targets[1].required_features, ["std"]);
        assert!(matches!(parse("{}"), Err(Refusal::NoWorkspace(_))));
    }

    /// Against cargo itself: a root package with a member, as the lens
    /// meets it. The member's test is selected from the member's file,
    /// from a project opened at the root and at the member alike, and
    /// `cargo test` with the selection runs the test the bare filter at the
    /// root did not.
    #[test]
    fn cargo_selects_the_member_a_file_is_in() {
        let dir = tempfile::tempdir().expect("a directory");
        let root = dir.path();
        let write = |path: &str, text: &str| {
            let path = root.join(path);
            std::fs::create_dir_all(path.parent().expect("a parent")).expect("a directory");
            std::fs::write(path, text).expect("written");
        };
        write(
            "Cargo.toml",
            "[workspace]\nmembers = [\"core\"]\n\n[package]\nname = \"top\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        );
        write("src/lib.rs", "");
        write(
            "core/Cargo.toml",
            "[package]\nname = \"core-part\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        );
        write(
            "core/src/lib.rs",
            "#[cfg(test)]\nmod tests {\n    #[test]\n    fn it_holds() {}\n}\n",
        );
        let file = root.join("core").join("src").join("lib.rs");
        let chosen = selection_for(&file).expect("a selection");
        assert_eq!(chosen.args(), ["--package", "core-part", "--lib"]);
        let canonical = |path: &Path| path.canonicalize().expect("there");
        assert_eq!(canonical(&chosen.workspace), canonical(root));
        assert_eq!(
            canonical(&chosen.package_dir),
            canonical(&root.join("core")),
            "a member's test runs in the member, as cargo test runs it"
        );
        assert_eq!(chosen.run_in(root), root);
        assert_eq!(chosen.run_in(&root.join("core")), root.join("core"));

        let run = |args: &[String]| {
            let output = process::command("cargo")
                .arg("test")
                .args(args)
                .args(["tests::it_holds", "--", "--show-output"])
                .current_dir(root)
                .env("CARGO_TARGET_DIR", root.join("target"))
                .output()
                .expect("cargo ran");
            assert!(output.status.success(), "{output:?}");
            String::from_utf8_lossy(&output.stdout).into_owned()
        };
        assert!(
            run(&[]).contains("running 0 tests"),
            "the bare filter at the root runs nothing"
        );
        assert!(
            run(&chosen.args()).contains("test tests::it_holds ... ok"),
            "the selection runs the member's test"
        );
    }

    /// The suite is the workspace's wherever the root's manifest declares
    /// one — a root package with members, a virtual workspace, a root that
    /// only shares dependencies — and the package's where it does not, a
    /// member inheriting its version from the workspace included.
    #[test]
    fn the_suite_is_the_workspace_wherever_the_root_declares_one() {
        let package = "[package]\nname = \"a\"\nversion = \"0.1.0\"\n";
        assert_eq!(suite_args_for(package), ["test", "--no-fail-fast"]);
        assert_eq!(
            suite_args_for(&format!("[workspace]\nmembers = [\"core\"]\n\n{package}")),
            ["test", "--workspace", "--no-fail-fast"]
        );
        assert_eq!(
            suite_args_for("[workspace]\nmembers = [\"a\"]\nresolver = \"2\"\n"),
            ["test", "--workspace", "--no-fail-fast"]
        );
        assert_eq!(
            suite_args_for(&format!(
                "{package}\n[workspace.dependencies]\nlog = \"0.4\"\n"
            )),
            ["test", "--workspace", "--no-fail-fast"]
        );
        assert_eq!(
            suite_args_for("[package]\nname = \"core\"\nversion.workspace = true\n"),
            ["test", "--no-fail-fast"],
            "a member's inherited key is not a workspace",
        );
        assert_eq!(
            suite_args_for("[package\nname ="),
            ["test", "--no-fail-fast"],
            "cargo says what is wrong"
        );
        assert_eq!(suite_args_for(""), ["test", "--no-fail-fast"]);
    }

    /// Against cargo itself: a root package whose members are a glob, one
    /// of which is a firmware crate that does not build for this machine
    /// and is excluded. The suite runs the member's test, which plain
    /// `cargo test` at the root never did, and leaves the firmware out as
    /// cargo's `exclude` says — without it, `--workspace` would build the
    /// firmware and stop at its `compile_error!`.
    #[test]
    fn the_suite_runs_every_members_tests_and_leaves_the_excluded_out() {
        let dir = tempfile::tempdir().expect("a directory");
        let root = dir.path();
        let write = |path: &str, text: &str| {
            let path = root.join(path);
            std::fs::create_dir_all(path.parent().expect("a parent")).expect("a directory");
            std::fs::write(path, text).expect("written");
        };
        write(
            "Cargo.toml",
            "[workspace]\nmembers = [\"crates/*\"]\nexclude = [\"crates/firmware\"]\n\n[package]\nname = \"top\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        );
        write(
            "src/lib.rs",
            "#[cfg(test)]\nmod tests {\n    #[test]\n    fn top_holds() {}\n}\n",
        );
        write(
            "crates/core/Cargo.toml",
            "[package]\nname = \"core-part\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        );
        write(
            "crates/core/src/lib.rs",
            "#[cfg(test)]\nmod tests {\n    #[test]\n    fn member_holds() {}\n}\n",
        );
        write(
            "crates/firmware/Cargo.toml",
            "[package]\nname = \"firmware\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        );
        write(
            "crates/firmware/src/lib.rs",
            "compile_error!(\"the firmware builds for the chip\");\n",
        );
        let run = |args: &[String]| {
            let output = process::command("cargo")
                .args(args)
                .current_dir(root)
                .env("CARGO_TARGET_DIR", root.join("target"))
                .output()
                .expect("cargo ran");
            assert!(output.status.success(), "{output:?}");
            String::from_utf8_lossy(&output.stdout).into_owned()
        };
        let suite = suite_args(root);
        assert_eq!(suite, ["test", "--workspace", "--no-fail-fast"]);
        let all = run(&suite);
        assert!(all.contains("test tests::top_holds ... ok"), "{all}");
        assert!(
            all.contains("test tests::member_holds ... ok"),
            "the member's test ran: {all}"
        );
        let plain = run(&["test".to_string()]);
        assert!(
            !plain.contains("member_holds"),
            "plain `cargo test` at the root never ran it: {plain}"
        );
    }

    /// A virtual workspace — a root that is no package — selects its
    /// members as well, and a file beside its manifest, in no package, is
    /// refused by name.
    #[test]
    fn a_virtual_workspace_selects_its_members_and_refuses_the_rest() {
        let dir = tempfile::tempdir().expect("a directory");
        let root = dir.path();
        let write = |path: &str, text: &str| {
            let path = root.join(path);
            std::fs::create_dir_all(path.parent().expect("a parent")).expect("a directory");
            std::fs::write(path, text).expect("written");
        };
        write(
            "Cargo.toml",
            "[workspace]\nmembers = [\"a\"]\nresolver = \"2\"\n",
        );
        write(
            "a/Cargo.toml",
            "[package]\nname = \"a\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        );
        write("a/src/lib.rs", "");
        write("notes/loose.rs", "");
        let chosen = selection_for(&root.join("a").join("src").join("lib.rs")).expect("selected");
        assert_eq!(chosen.args(), ["--package", "a", "--lib"]);
        let refused =
            selection_for(&root.join("notes").join("loose.rs")).expect_err("in no package");
        assert!(matches!(refused, Refusal::NoPackage { .. }), "{refused}");
        // Nowhere near a manifest, cargo's own words.
        let nowhere = tempfile::tempdir().expect("a directory");
        let refused = selection_for(&nowhere.path().join("x.rs")).expect_err("no workspace");
        assert!(matches!(refused, Refusal::NoWorkspace(_)), "{refused}");
    }
}
