//! Nothing here is ever compiled by the tests — `cargo metadata` only reads
//! manifests. The file exists so the fixture is a valid package.

#[cfg(feature = "json")]
pub fn uses_serde() -> bool {
    true
}
