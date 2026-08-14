//! The C in csrc/, declared for Rust.
//!
//! Hand-written rather than generated: one function does not need bindgen,
//! and a hand-written declaration is one you can read. For a real SDK —
//! hundreds of functions, macros, packed structs — add bindgen as a build
//! dependency and generate this module instead.

unsafe extern "C" {
    fn pattern_step() -> core::ffi::c_uint;
}

/// Safe because the C keeps its state in two statics and touches nothing
/// else, and this is the only caller — the firmware is single-threaded and
/// has no interrupt that reaches here.
///
/// That sentence is the whole job of a wrapper like this: state why the
/// `unsafe` below is sound, or do not write it.
pub fn step() -> u8 {
    (unsafe { pattern_step() }) as u8
}
