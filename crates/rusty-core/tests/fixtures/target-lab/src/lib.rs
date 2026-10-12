//! Nothing here is ever compiled by the tests — `cargo metadata` only reads
//! manifests. The files exist so that cargo finds the targets they stand for.

pub fn answer() -> u32 {
    lab_helper::half() * 2
}
