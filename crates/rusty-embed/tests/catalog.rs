//! Catalogue layering.
//!
//! The whole point of making this a file format is that a user can add their
//! board without forking rusty. These tests pin the properties that makes true:
//! a project file is found, it wins over a built-in, and one malformed file
//! does not take the catalogue down with it.

use std::fs;

use rusty_embed::{catalog::Catalog, model::CatalogSource};
use tempfile::TempDir;

/// A project directory with `.rusty/boards/<name>.toml` written for it.
fn project_with_boards(files: &[(&str, &str)]) -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    let boards = dir.path().join(".rusty").join("boards");
    fs::create_dir_all(&boards).unwrap();
    for (name, body) in files {
        fs::write(boards.join(name), body).unwrap();
    }
    dir
}

const CUSTOM_BOARD: &str = r#"
[[board]]
id = "acme-sensor-node"
name = "ACME Sensor Node rev C"
chip = "esp32c6"
flash_bytes = 16777216
flash_baud = 460800
[[board.usb]]
vendor_id = 0x1A86
product_id = 0x55D4
note = "CH9102 bridge"
[board.pins]
led = 21
status = 22
"#;

#[test]
fn a_project_board_is_found_and_marked_as_such() {
    let dir = project_with_boards(&[("acme.toml", CUSTOM_BOARD)]);
    let catalog = Catalog::load(Some(dir.path()));

    assert!(catalog.problems().is_empty(), "{:?}", catalog.problems());

    let board = catalog
        .board("acme-sensor-node")
        .expect("project board should load");
    assert_eq!(board.name, "ACME Sensor Node rev C");
    assert_eq!(board.chip, "esp32c6");
    assert_eq!(board.flash_bytes, Some(16 * 1024 * 1024));
    assert_eq!(board.flash_baud, Some(460_800));
    // Provenance is shown in the UI so a user can tell their own entries from
    // the shipped ones.
    assert_eq!(board.source, CatalogSource::Project);

    // Pins arrive as a list, sorted by the map that parsed them.
    let led = board.pins.iter().find(|p| p.name == "led").unwrap();
    assert_eq!(led.gpio, 21);

    // Built-ins are still there — a project file adds, it does not replace the
    // catalogue.
    assert!(catalog.board("esp32c3-devkitm-1").is_some());
}

/// The reason the layering exists: a team's board list is authoritative for
/// that team, including when it corrects something rusty ships.
#[test]
fn a_project_file_overrides_a_builtin_of_the_same_id() {
    let builtin = Catalog::builtin();
    let original = builtin.board("esp32c3-devkitm-1").unwrap();
    assert_eq!(original.flash_bytes, Some(4 * 1024 * 1024));
    assert_eq!(original.source, CatalogSource::Builtin);

    let dir = project_with_boards(&[(
        "override.toml",
        r#"
[[board]]
id = "esp32c3-devkitm-1"
name = "ESP32-C3-DevKitM-1 (8MB variant)"
chip = "esp32c3"
flash_bytes = 8388608
"#,
    )]);
    let catalog = Catalog::load(Some(dir.path()));

    let overridden = catalog.board("esp32c3-devkitm-1").unwrap();
    assert_eq!(overridden.flash_bytes, Some(8 * 1024 * 1024));
    assert_eq!(overridden.source, CatalogSource::Project);
    // Replaced, not duplicated.
    assert_eq!(
        catalog
            .boards()
            .iter()
            .filter(|b| b.id == "esp32c3-devkitm-1")
            .count(),
        1
    );
}

/// One bad file must not blank the catalogue. Silently ignoring it would be
/// just as bad — the user would stare at a board that never appears — so the
/// failure is kept and reported.
#[test]
fn a_malformed_file_is_reported_without_losing_the_rest() {
    let dir = project_with_boards(&[
        ("good.toml", CUSTOM_BOARD),
        (
            "broken.toml",
            "[[board]]\nid = \"oops\"\n# no name, no chip\n",
        ),
    ]);
    let catalog = Catalog::load(Some(dir.path()));

    assert!(
        catalog.board("acme-sensor-node").is_some(),
        "good file still loaded"
    );
    assert!(
        catalog.board("esp32c3-devkitm-1").is_some(),
        "built-ins survive"
    );

    let problem = catalog
        .problems()
        .iter()
        .find(|p| p.path.contains("broken.toml"))
        .expect("the bad file must be reported");
    assert!(
        problem.detail.contains("name") || problem.detail.contains("missing"),
        "the message should say what is wrong: {}",
        problem.detail
    );
}

/// A typo in a field name is the most likely mistake in a hand-written file,
/// and the least helpful to ignore.
#[test]
fn an_unknown_field_is_an_error_rather_than_being_dropped() {
    let dir = project_with_boards(&[(
        "typo.toml",
        r#"
[[board]]
id = "typo-board"
name = "Typo Board"
chip = "esp32c3"
flash_size = 4194304
"#,
    )]);
    let catalog = Catalog::load(Some(dir.path()));

    assert!(catalog.board("typo-board").is_none());
    let problem = &catalog.problems()[0];
    assert!(problem.detail.contains("flash_size"), "{}", problem.detail);
    // The message should point at what was expected.
    assert!(problem.detail.contains("flash_bytes"), "{}", problem.detail);
}

#[test]
fn no_project_and_no_overlays_is_just_the_builtins() {
    let empty = tempfile::tempdir().unwrap();
    let catalog = Catalog::load(Some(empty.path()));

    assert!(catalog.problems().is_empty());
    assert_eq!(catalog.boards().len(), Catalog::builtin().boards().len());
    assert_eq!(catalog.chips().len(), Catalog::builtin().chips().len());
}

#[test]
fn a_project_can_add_a_chip_the_build_does_not_know() {
    let dir = tempfile::tempdir().unwrap();
    let chips = dir.path().join(".rusty").join("chips");
    fs::create_dir_all(&chips).unwrap();
    fs::write(
        chips.join("nordic.toml"),
        r#"
[[chip]]
id = "nrf52840"
name = "nRF52840"
vendor = "st"
arch = "cortex-m"
cores = 1
sram_bytes = 262144
flash_bytes = 1048576
bare_metal_target = "thumbv7em-none-eabihf"
toolchain = "stock"
flashers = ["probe-rs"]
probe_rs_target = "nRF52840_xxAA"
radios = ["BLE 5", "802.15.4"]
"#,
    )
    .unwrap();

    let catalog = Catalog::load(Some(dir.path()));
    assert!(catalog.problems().is_empty(), "{:?}", catalog.problems());

    let chip = catalog.chip("nRF52840").expect("added chip should resolve");
    assert_eq!(chip.id, "nrf52840", "ids are normalized on load");
    assert_eq!(chip.probe_rs_target.as_deref(), Some("nRF52840_xxAA"));
}

#[test]
fn boards_can_be_listed_for_a_chip() {
    let catalog = Catalog::builtin();
    let c3_boards = catalog.boards_for_chip("ESP32-C3");
    assert!(c3_boards.len() >= 2, "{c3_boards:?}");
    assert!(c3_boards.iter().all(|b| b.chip == "esp32c3"));
}
