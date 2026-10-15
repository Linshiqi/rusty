//! The window is drawn by a patched copy of Slint's OpenGL renderer
//! (`vendor/i-slint-renderer-femtovg`, see `[patch.crates-io]` in the
//! workspace manifest): a rounded clip's contents are put on the pixel grid,
//! or a code block glides behind frosted glass. A `[patch]` whose version
//! no longer matches what slint pins is not an error — cargo warns that it
//! was not used and builds the published renderer — so this is what notices
//! an upgrade of slint that left the patch behind.

#[test]
fn the_window_is_drawn_by_the_patched_renderer() {
    let lock = include_str!("../../../Cargo.lock");
    let start = lock
        .find("name = \"i-slint-renderer-femtovg\"")
        .expect("the window is drawn by i-slint-renderer-femtovg");
    let entry: Vec<&str> = lock[start..]
        .lines()
        .take_while(|line| !line.is_empty())
        .collect();
    assert!(
        !entry.iter().any(|line| line.starts_with("source =")),
        "i-slint-renderer-femtovg comes from a registry, not vendor/: slint has moved past the \
         version the patch was copied from. Check whether upstream's `visit_clip` now draws the \
         layer on the pixel grid; copy the new version into vendor/ with the change, or drop the \
         patch. {entry:?}"
    );
}
