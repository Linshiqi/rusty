//! The scan, the sweep and the removals, run against a build directory
//! written the way cargo writes one.

use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use super::fs::tree_locked;
use super::judge::{Origin, dep_info_sources, origin_of, split_package_dir, unit_hash};
use super::tree::reason_name;
use super::*;
use crate::error::Error;
use crate::model::{DiskKind, KEEP_VARIANTS, StaleReason, SweepPolicy};

/// A build directory laid out the way cargo lays one out, with the sizes
/// and mtimes the rules read — real files, since the rules read real
/// files.
struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "rusty-disk-{name}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("project")).unwrap();
        Fixture { root }
    }

    fn project(&self) -> PathBuf {
        self.root.join("project")
    }

    fn target(&self) -> PathBuf {
        self.project().join("target")
    }

    fn write(&self, relative: &str, bytes: usize) -> PathBuf {
        let path = self.target().join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, vec![b'x'; bytes]).unwrap();
        path
    }

    fn write_text(&self, relative: &str, text: &str) -> PathBuf {
        let path = self.target().join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, text).unwrap();
        path
    }

    /// A unit in `deps/`: the dep-info naming `source`, and an rlib.
    fn unit(&self, tree: &str, crate_name: &str, hash: &str, source: &str, rlib_bytes: usize) {
        self.write_text(
            &format!("{tree}/deps/{crate_name}-{hash}.d"),
            &format!(
                "E:\\proj\\target\\{tree}\\deps\\{crate_name}-{hash}.d: {source}\n\n{source}:\n"
            ),
        );
        self.write(
            &format!("{tree}/deps/lib{crate_name}-{hash}.rlib"),
            rlib_bytes,
        );
        self.write(&format!("{tree}/deps/lib{crate_name}-{hash}.rmeta"), 10);
        self.write(
            &format!("{tree}/.fingerprint/{crate_name}-{hash}/lib-{crate_name}"),
            16,
        );
    }

    fn age(&self, relative: &str, days: u64) {
        let path = self.target().join(relative);
        let old = SystemTime::now() - Duration::from_secs(days * 86_400);
        fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(old)
            .unwrap();
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

const REGISTRY: &str = "C:\\Users\\me\\.cargo\\registry\\src\\index.crates.io-1949cf8c6b5b557f";

/// The graph the fixture's lockfile resolves: two registry packages at
/// the versions it holds, and the workspace's own package, whose one
/// target is the binary `my-app`.
fn current() -> Current {
    let mut current = Current::default();
    for (name, version) in [("serde", "1.0.229"), ("windows-sys", "0.52.0")] {
        current.names.insert(name.to_string());
        current
            .versions
            .insert((name.to_string(), version.to_string()));
    }
    current.names.insert("my-app".to_string());
    current.add_local_target("my-app");
    current
}

/// The yardstick of `tests/fixtures/target-lab`, read off the real graph:
/// a member with a target of every kind, and a path dependency with a
/// build script of its own.
fn target_lab() -> Current {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/target-lab");
    crate::Workspace::load(path)
        .expect("fixture workspace should load")
        .current()
}

/// A host `debug` tree holding one of target-lab's own units — a tree is
/// known by its `deps/` — for incremental caches to be laid out in.
fn lab_tree(name: &str) -> Fixture {
    let fixture = Fixture::new(name);
    fixture.unit(
        "debug",
        "target_lab",
        "0123456789abcdef",
        "src\\lib.rs",
        100,
    );
    fixture
}

fn lay_out(fixture: &Fixture) {
    // serde: the version the lockfile has, and the one it moved away from.
    fixture.unit(
        "debug",
        "serde",
        "aaaaaaaaaaaaaaaa",
        &format!("{REGISTRY}\\serde-1.0.200\\src\\lib.rs"),
        5_000,
    );
    fixture.unit(
        "debug",
        "serde",
        "bbbbbbbbbbbbbbbb",
        &format!("{REGISTRY}\\serde-1.0.229\\src\\lib.rs"),
        6_000,
    );
    // A dashed name, current.
    fixture.unit(
        "debug",
        "windows_sys",
        "cccccccccccccccc",
        &format!("{REGISTRY}\\windows-sys-0.52.0\\src\\lib.rs"),
        7_000,
    );
    // A dependency dropped from the graph entirely.
    fixture.unit(
        "debug",
        "left_pad",
        "dddddddddddddddd",
        &format!("{REGISTRY}\\left-pad-1.0.0\\src\\lib.rs"),
        800,
    );
    // The workspace's own crate: local, never stale by version.
    fixture.unit("debug", "my_app", "eeeeeeeeeeeeeeee", "src\\main.rs", 900);
    // Debug symbols beside a test binary.
    fixture.write("debug/deps/my_app-eeeeeeeeeeeeeeee.pdb", 4_000);
    // Incremental: a live crate touched today, the same crate idle for a
    // month, and a crate that is no longer in the workspace.
    fixture.write(
        "debug/incremental/my_app-1a2b3c4d5e6f7g/s-fresh-abc/query-cache.bin",
        300,
    );
    fixture.write(
        "debug/incremental/my_app-9z8y7x6w5v4u3t/s-old-def/query-cache.bin",
        400,
    );
    fixture.age(
        "debug/incremental/my_app-9z8y7x6w5v4u3t/s-old-def/query-cache.bin",
        30,
    );
    fixture.write(
        "debug/incremental/gone_crate-0000000000000/s-x-y/query-cache.bin",
        500,
    );
    // Five more variants of the live crate, a day old: with four kept,
    // two of them are superseded and the fresh one never is.
    for variant in [
        "v1v1v1v1v1v1v",
        "v2v2v2v2v2v2v",
        "v3v3v3v3v3v3v",
        "v4v4v4v4v4v4v",
        "v5v5v5v5v5v5v",
    ] {
        let relative = format!("debug/incremental/my_app-{variant}/s-a-b/query-cache.bin");
        fixture.write(&relative, 100);
        fixture.age(&relative, 1);
    }
    // Build scripts: the old serde's compiled script and its run
    // directory, linked through the fingerprint record.
    fixture.write_text(
        "debug/build/serde-aaaaaaaaaaaaaaaa/build_script_build-aaaaaaaaaaaaaaaa.d",
        &format!("x: {REGISTRY}\\serde-1.0.200\\build.rs\n"),
    );
    fixture.write(
        "debug/build/serde-aaaaaaaaaaaaaaaa/build_script_build-aaaaaaaaaaaaaaaa.exe",
        2_000,
    );
    fixture.write_text(
        "debug/.fingerprint/serde-aaaaaaaaaaaaaaaa/build-script-build-script-build",
        "00000000deadbeef",
    );
    fixture.write("debug/build/serde-ffffffffffffffff/out/generated.rs", 3_000);
    fixture.write_text(
        "debug/build/serde-ffffffffffffffff/output",
        "cargo:rerun-if-changed=build.rs\n",
    );
    fixture.write_text(
        "debug/.fingerprint/serde-ffffffffffffffff/run-build-script-build-script-build.json",
        &format!(
            "{{\"rustc\":1,\"deps\":[[42,\"build_script_build\",false,{}]]}}",
            0x0000_0000_dead_beefu64
        ),
    );
    // A run directory whose package is gone, with no readable record.
    fixture.write("debug/build/left-pad-1111111111111111/out/x.rs", 600);
    fixture.write_text("debug/build/left-pad-1111111111111111/output", "");
    // The uplifted binary.
    fixture.write("debug/my-app.exe", 1_000);
    // A second tree, for another target, with one live unit.
    fixture.unit(
        "xtensa-esp32-none-elf/release",
        "serde",
        "bbbbbbbbbbbbbbbb",
        &format!("{REGISTRY}\\serde-1.0.229\\src\\lib.rs"),
        2_500,
    );
    // Extras.
    fixture.write("doc/serde/index.html", 1_200);
    fixture.write("rusty-sim/app.bin", 1_300);
    fixture.write("mystery/thing.bin", 1_400);
    fixture.write_text(".rustc_info.json", "{}");
    fixture.write_text(
        "CACHEDIR.TAG",
        "Signature: 8a477f597d28d172789f06886806bc55",
    );
}

#[test]
fn a_dep_info_names_its_registry_package_and_version() {
    let text = format!(
        "E:\\p\\target\\debug\\deps\\serde-06a2225fab2cda3a.d: {REGISTRY}\\serde-1.0.229\\src\\lib.rs {REGISTRY}\\serde-1.0.229\\src\\de.rs\n"
    );
    let sources = dep_info_sources(&text).unwrap();
    assert_eq!(sources.len(), 2);
    assert_eq!(
        origin_of(&sources),
        Origin::Registry {
            dir: "serde-1.0.229".into()
        }
    );
    // Forward slashes and a git checkout.
    let git =
        vec!["/home/me/.cargo/git/checkouts/esp-hal-3f2a9b/0123abc/esp-hal/src/lib.rs".to_string()];
    assert_eq!(
        origin_of(&git),
        Origin::Git {
            repo: "esp-hal-3f2a9b".into(),
            rev: "0123abc".into()
        }
    );
    // A member: relative paths, no registry.
    assert_eq!(
        origin_of(&["crates\\rusty-git\\src\\lib.rs".to_string()]),
        Origin::Local
    );
    assert!(dep_info_sources("garbage without a separator").is_none());
}

#[test]
fn a_registry_directory_splits_at_every_dash_that_starts_a_version() {
    assert_eq!(
        split_package_dir("windows-sys-0.52.0"),
        vec![("windows-sys".to_string(), "0.52.0".to_string())]
    );
    assert_eq!(
        split_package_dir("foo-1.0.0-beta.1"),
        vec![("foo".to_string(), "1.0.0-beta.1".to_string())]
    );
    // `sha-1` the crate, or `sha` at a version that is not semver.
    assert_eq!(
        split_package_dir("sha-1-0.10.1"),
        vec![("sha-1".to_string(), "0.10.1".to_string())]
    );
    assert!(split_package_dir("no-version-here").is_empty());
    assert_eq!(
        unit_hash("libserde-06a2225fab2cda3a.rlib"),
        Some("06a2225fab2cda3a")
    );
    assert_eq!(
        unit_hash("serde-06a2225fab2cda3a.d"),
        Some("06a2225fab2cda3a")
    );
    assert_eq!(unit_hash("my-app.exe"), None);
    assert_eq!(
        unit_hash("my_app-1a2b3c4d5e6f7g"),
        None,
        "incremental hashes are not hex"
    );
}

#[test]
fn the_scan_marks_gone_versions_gone_packages_and_idle_caches_and_nothing_else() {
    let fixture = Fixture::new("scan");
    lay_out(&fixture);
    let scan = scan(
        &fixture.target(),
        &fixture.project(),
        &current(),
        ScanOptions::default(),
    );
    let report = &scan.report;
    assert!(report.exists);
    assert!(!report.shared);
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    assert_eq!(report.trees.len(), 2);
    let host = report
        .trees
        .iter()
        .find(|t| t.triple.is_none())
        .expect("host tree");
    assert_eq!(host.profile, "debug");
    let xtensa = report
        .trees
        .iter()
        .find(|t| t.triple.is_some())
        .expect("xtensa tree");
    assert_eq!(xtensa.triple.as_deref(), Some("xtensa-esp32-none-elf"));
    assert_eq!(xtensa.profile, "release");

    let stale = scan.stale_paths();
    let names: Vec<String> = stale
        .iter()
        .map(|(p, ..)| p.file_name().unwrap().to_string_lossy().to_string())
        .collect();
    // The old serde, all of it: three deps files, the fingerprint, the
    // compiled script and its run directory.
    for expected in [
        "libserde-aaaaaaaaaaaaaaaa.rlib",
        "libserde-aaaaaaaaaaaaaaaa.rmeta",
        "serde-aaaaaaaaaaaaaaaa.d",
        "serde-aaaaaaaaaaaaaaaa",
        "serde-ffffffffffffffff",
    ] {
        assert!(
            names.contains(&expected.to_string()),
            "{expected} missing from {names:?}"
        );
    }
    assert_eq!(
        names
            .iter()
            .filter(|n| n.as_str() == "serde-aaaaaaaaaaaaaaaa")
            .count(),
        2,
        "the fingerprint directory and the compiled script share the name"
    );
    // The dropped dependency and its run directory, by package.
    assert!(names.contains(&"libleft_pad-dddddddddddddddd.rlib".to_string()));
    assert!(names.contains(&"left-pad-1111111111111111".to_string()));
    // The idle cache and the gone crate's cache, not the fresh one.
    assert!(names.contains(&"my_app-9z8y7x6w5v4u3t".to_string()));
    assert!(names.contains(&"gone_crate-0000000000000".to_string()));
    assert!(!names.contains(&"my_app-1a2b3c4d5e6f7g".to_string()));
    // Live things are not there.
    for live in [
        "libserde-bbbbbbbbbbbbbbbb.rlib",
        "libwindows_sys-cccccccccccccccc.rlib",
        "libmy_app-eeeeeeeeeeeeeeee.rlib",
        "my-app.exe",
    ] {
        assert!(!names.contains(&live.to_string()), "{live} wrongly stale");
    }
    let reasons: HashSet<&str> = stale.iter().map(|(.., r)| reason_name(r)).collect();
    assert_eq!(
        reasons,
        HashSet::from(["version-gone", "package-gone", "idle", "superseded"])
    );
    assert_eq!(
        names.iter().filter(|n| n.starts_with("my_app-v")).count(),
        2,
        "two of five day-old variants fall outside the four kept: {names:?}"
    );

    // The groups add up and say why.
    let deps = host
        .groups
        .iter()
        .find(|g| g.kind == DiskKind::Deps)
        .unwrap();
    assert_eq!(
        deps.stale_bytes,
        5_000
            + 10
            + deps_d_len(&fixture, "serde-aaaaaaaaaaaaaaaa")
            + 800
            + 10
            + deps_d_len(&fixture, "left_pad-dddddddddddddddd")
    );
    assert!(
        deps.stale_by_reason
            .iter()
            .any(|s| s.reason == "version-gone")
    );
    assert!(
        deps.stale_by_reason
            .iter()
            .any(|s| s.reason == "package-gone")
    );
    let incremental = host
        .groups
        .iter()
        .find(|g| g.kind == DiskKind::Incremental)
        .unwrap();
    assert_eq!(incremental.stale_bytes, 400 + 500 + 2 * 100);
    assert_eq!(report.debuginfo_bytes, 4_000);

    // Extras: known ones removable, the unknown one not, files at the top
    // level counted but not listed.
    let labels: Vec<(&str, bool)> = report
        .extras
        .iter()
        .map(|e| (e.label.as_str(), e.removable))
        .collect();
    assert!(labels.contains(&("docs", true)));
    assert!(labels.contains(&("sim-images", true)));
    assert!(labels.contains(&("other", false)));
    assert_eq!(report.extras.len(), 3);
    assert!(report.total_bytes > 0);
}

fn deps_d_len(fixture: &Fixture, stem: &str) -> u64 {
    fs::metadata(fixture.target().join(format!("debug/deps/{stem}.d")))
        .unwrap()
        .len()
}

/// Every stale path's file name with its reason, sorted by name.
fn stale_by_name(scan: &Scan) -> Vec<(String, StaleReason)> {
    let mut stale: Vec<(String, StaleReason)> = scan
        .stale_paths()
        .into_iter()
        .map(|(path, _, reason)| {
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            (name, reason)
        })
        .collect();
    stale.sort_by(|a, b| a.0.cmp(&b.0));
    stale
}

/// Read off a real graph, the yardstick holds every target of every member
/// and path dependency, under the name rustc compiles it as, and how many
/// targets share each name.
#[test]
fn the_yardstick_counts_every_local_target_by_the_crate_it_compiles_as() {
    let current = target_lab();
    assert_eq!(
        current.targets_named("target_lab"),
        2,
        "the library and the binary beside it"
    );
    assert_eq!(
        current.targets_named("build_script_build"),
        2,
        "the member's build script and the path dependency's"
    );
    for single in ["lab_tool", "demo_run", "smoke", "speed", "lab_helper"] {
        assert_eq!(current.targets_named(single), 1, "{single}");
    }
    assert_eq!(
        current.targets_named("lab-tool"),
        0,
        "rustc is given `lab_tool`, and that is what the cache is called"
    );
}

/// An incremental cache is named after the crate rustc compiled, and a
/// crate is a target: an example, an integration test, a binary named
/// apart from its package and a build script each leave caches under a
/// name no package carries. Judged against the package names, every one of
/// those was gone — on this repository's own build directory, the caches of
/// every test and probe compiled minutes before — and a sweep removed them:
/// a slower rebuild, for a reason that was not true.
#[test]
fn an_incremental_cache_is_judged_by_the_package_whose_target_it_is() {
    let fixture = lab_tree("targets");
    for cache in [
        "target_lab-0a0a0a0a0a0a0",
        "lab_tool-1b1b1b1b1b1b1",
        "demo_run-2c2c2c2c2c2c2",
        "smoke-3d3d3d3d3d3d3",
        "speed-4e4e4e4e4e4e4",
        "build_script_build-5f5f5f5f5f5f5",
        "lab_helper-6g6g6g6g6g6g6",
        "dropped_example-7h7h7h7h7h7h7",
    ] {
        fixture.write(
            &format!("debug/incremental/{cache}/s-a-b/query-cache.bin"),
            100,
        );
    }
    let scan = scan(
        &fixture.target(),
        &fixture.project(),
        &target_lab(),
        ScanOptions::default(),
    );
    assert_eq!(
        stale_by_name(&scan),
        [(
            "dropped_example-7h7h7h7h7h7h7".to_string(),
            StaleReason::PackageGone {
                package: "dropped_example".to_string()
            }
        )],
        "only the crate nothing in the graph compiles is gone"
    );
}

/// A name several targets compile under holds every one of their variants
/// — the member's build script and the path dependency's are both
/// `build_script_build`, and a library shares its name with the binary
/// beside it — and nothing in a cache says whose it is. Four kept for the
/// name would call the second target's live caches superseded; the name
/// keeps four for each.
#[test]
fn a_crate_name_several_targets_share_keeps_each_ones_variants() {
    let fixture = lab_tree("shared-name");
    // Nine caches under each name, a day apart and none of them idle.
    for crate_name in ["build_script_build", "target_lab", "lab_tool"] {
        for day in 1..=9u64 {
            let relative =
                format!("debug/incremental/{crate_name}-{day:013}/s-a-b/query-cache.bin");
            fixture.write(&relative, 100);
            fixture.age(&relative, day);
        }
    }
    let options = ScanOptions {
        idle_days: 30,
        keep_variants: 4,
    };
    let scan = scan(
        &fixture.target(),
        &fixture.project(),
        &target_lab(),
        options,
    );
    let superseded = |name: &str, keep| (name.to_string(), StaleReason::Superseded { keep });
    assert_eq!(
        stale_by_name(&scan),
        [
            superseded("build_script_build-0000000000009", 8),
            superseded("lab_tool-0000000000005", 4),
            superseded("lab_tool-0000000000006", 4),
            superseded("lab_tool-0000000000007", 4),
            superseded("lab_tool-0000000000008", 4),
            superseded("lab_tool-0000000000009", 4),
            superseded("target_lab-0000000000009", 8),
        ],
        "two targets share each of the first two names, one has the third"
    );
}

#[test]
fn an_empty_yardstick_judges_no_dependency_and_says_so() {
    let fixture = Fixture::new("empty");
    lay_out(&fixture);
    let scan = scan(
        &fixture.target(),
        &fixture.project(),
        &Current::default(),
        ScanOptions::default(),
    );
    assert_eq!(scan.report.warnings.len(), 1);
    let reasons: HashSet<&str> = scan.stale.iter().map(|s| reason_name(&s.reason)).collect();
    assert_eq!(
        reasons,
        HashSet::from(["idle", "superseded"]),
        "the incremental rules need no graph"
    );
}

#[test]
fn a_sweep_removes_exactly_the_stale_paths_and_reports_the_bytes() {
    let fixture = Fixture::new("sweep");
    lay_out(&fixture);
    let before = scan(
        &fixture.target(),
        &fixture.project(),
        &current(),
        ScanOptions::default(),
    );
    let expected: u64 = before.stale.iter().map(|s| s.bytes).sum();
    let report = sweep(
        &fixture.target(),
        &fixture.project(),
        &current(),
        &SweepPolicy::default(),
    )
    .unwrap();
    assert_eq!(report.removed_bytes, expected);
    assert_eq!(report.removed_items as usize, before.stale.len());
    assert!(report.failed.is_empty(), "{:?}", report.failed);
    assert!(report.locked.is_empty());
    let target = fixture.target();
    assert!(
        !target
            .join("debug/deps/libserde-aaaaaaaaaaaaaaaa.rlib")
            .exists()
    );
    assert!(!target.join("debug/build/serde-ffffffffffffffff").exists());
    assert!(
        !target
            .join("debug/incremental/my_app-9z8y7x6w5v4u3t")
            .exists()
    );
    assert!(
        target
            .join("debug/deps/libserde-bbbbbbbbbbbbbbbb.rlib")
            .exists()
    );
    assert!(
        target
            .join("debug/incremental/my_app-1a2b3c4d5e6f7g")
            .exists()
    );
    assert!(target.join("debug/my-app.exe").exists());
    assert!(
        target
            .join("xtensa-esp32-none-elf/release/deps/libserde-bbbbbbbbbbbbbbbb.rlib")
            .exists()
    );
    // Nothing left to sweep.
    let after = scan(
        &fixture.target(),
        &fixture.project(),
        &current(),
        ScanOptions::default(),
    );
    assert!(after.stale.is_empty(), "{:?}", after.stale_paths());
}

#[test]
fn a_policy_narrows_the_sweep_to_a_reason_and_a_tree() {
    let fixture = Fixture::new("policy");
    lay_out(&fixture);
    let only_idle = SweepPolicy {
        version_gone: false,
        package_gone: false,
        superseded: false,
        keep_variants: KEEP_VARIANTS,
        idle_days: Some(7),
        tree: None,
    };
    let report = sweep(
        &fixture.target(),
        &fixture.project(),
        &current(),
        &only_idle,
    )
    .unwrap();
    assert_eq!(report.removed_bytes, 400, "the one idle cache");
    assert!(
        fixture
            .target()
            .join("debug/deps/libserde-aaaaaaaaaaaaaaaa.rlib")
            .exists()
    );

    let other_tree = SweepPolicy {
        tree: Some(
            fixture
                .target()
                .join("xtensa-esp32-none-elf/release")
                .to_string_lossy()
                .to_string(),
        ),
        ..SweepPolicy::default()
    };
    let report = sweep(
        &fixture.target(),
        &fixture.project(),
        &current(),
        &other_tree,
    )
    .unwrap();
    assert_eq!(report.removed_bytes, 0, "that tree has nothing stale");
    assert!(
        fixture
            .target()
            .join("debug/deps/libserde-aaaaaaaaaaaaaaaa.rlib")
            .exists()
    );
}

/// The sweep keeps as many of each target's caches as the scan that
/// previewed it was told to. It kept four whatever `--keep-variants` said,
/// so `rusty-cli sweep --keep-variants 2` listed four caches for removal and
/// then removed two.
#[test]
fn a_sweep_keeps_as_many_variants_as_the_scan_that_previewed_it() {
    let fixture = Fixture::new("keep");
    lay_out(&fixture);
    let options = ScanOptions {
        keep_variants: 2,
        ..ScanOptions::default()
    };
    let preview = scan(&fixture.target(), &fixture.project(), &current(), options);
    let superseded = preview
        .stale
        .iter()
        .filter(|s| matches!(s.reason, StaleReason::Superseded { keep: 2 }))
        .count();
    assert_eq!(superseded, 4, "six live variants of my_app, two kept");

    let policy = SweepPolicy {
        keep_variants: 2,
        ..SweepPolicy::default()
    };
    let report = sweep(&fixture.target(), &fixture.project(), &current(), &policy).unwrap();
    assert_eq!(
        report.removed_items as usize,
        preview.stale.len(),
        "what the preview listed is what went"
    );
    let left = fs::read_dir(fixture.target().join("debug/incremental"))
        .unwrap()
        .filter(|entry| {
            entry
                .as_ref()
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("my_app-")
        })
        .count();
    assert_eq!(left, 2, "the two newest");
}

#[test]
fn a_locked_tree_is_left_alone() {
    use fs4::fs_std::FileExt;
    let fixture = Fixture::new("locked");
    lay_out(&fixture);
    let lock_path = fixture.target().join("debug/.cargo-lock");
    let lock = fs::File::create(&lock_path).unwrap();
    assert!(lock.try_lock_exclusive().unwrap());
    assert!(tree_locked(&fixture.target().join("debug")));
    let report = sweep(
        &fixture.target(),
        &fixture.project(),
        &current(),
        &SweepPolicy::default(),
    )
    .unwrap();
    assert_eq!(report.removed_bytes, 0);
    assert_eq!(report.locked.len(), 1);
    assert!(
        fixture
            .target()
            .join("debug/deps/libserde-aaaaaaaaaaaaaaaa.rlib")
            .exists()
    );
    let refused = remove_tree(&fixture.target(), &fixture.target().join("debug"));
    assert!(matches!(refused, Err(Error::Refused { .. })));
    FileExt::unlock(&lock).unwrap();
    drop(lock);
    assert!(!tree_locked(&fixture.target().join("debug")));
}

#[test]
fn removing_a_tree_takes_only_what_the_scan_lists() {
    let fixture = Fixture::new("remove");
    lay_out(&fixture);
    let target = fixture.target();
    // Not a tree, not an extra: refused.
    assert!(matches!(
        remove_tree(&target, &target.join("debug/deps")),
        Err(Error::Refused { .. })
    ));
    assert!(matches!(
        remove_tree(&target, &fixture.project().join("src")),
        Err(Error::Refused { .. })
    ));
    // An extra the scan does not know: refused.
    assert!(matches!(
        remove_tree(&target, &target.join("mystery")),
        Err(Error::Refused { .. })
    ));
    assert!(target.join("mystery/thing.bin").exists());
    // A known extra and a whole tree: removed, sizes reported.
    let docs = remove_tree(&target, &target.join("doc")).unwrap();
    assert_eq!(docs.removed_bytes, 1_200);
    assert!(!target.join("doc").exists());
    // A tree's incremental caches alone, leaving its deps.
    let caches = remove_tree(&target, &target.join("debug/incremental")).unwrap();
    assert!(caches.removed_bytes >= 300 + 400 + 500 + 5 * 100);
    assert!(!target.join("debug/incremental").exists());
    assert!(
        target
            .join("debug/deps/libserde-bbbbbbbbbbbbbbbb.rlib")
            .exists()
    );
    let tree = remove_tree(&target, &target.join("xtensa-esp32-none-elf/release")).unwrap();
    assert!(tree.removed_bytes > 2_500);
    assert!(!target.join("xtensa-esp32-none-elf/release").exists());
    assert!(target.join("debug").exists());
}

#[test]
fn a_missing_build_directory_reports_itself_without_a_scan() {
    let fixture = Fixture::new("missing");
    let scan = scan(
        &fixture.target(),
        &fixture.project(),
        &current(),
        ScanOptions::default(),
    );
    assert!(!scan.report.exists);
    assert_eq!(scan.report.total_bytes, 0);
    assert!(scan.report.trees.is_empty());
    assert!(scan.stale.is_empty());
    // The volume is read off the project, which does exist.
    assert!(scan.report.volume.is_some());
}
