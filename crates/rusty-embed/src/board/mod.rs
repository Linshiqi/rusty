//! The board sheet drawn: the parts as the components on the desk, where
//! every lead ends, the wires routed between them, and what editing does to
//! both. Pure and wasm-safe — the old window's sheet editor and the Slint
//! window draw the same board from the same arithmetic.
//!
//! `art` is a part's drawing and where its leads end; `geometry` places
//! parts and pins on the sheet and hit-tests them; `edit` is what placing,
//! wiring, turning and deleting do; `layout` keeps parts off each other and
//! routes wires round them; `glow` is how bright a lamp is drawn; `readout`
//! is a reading as a meter shows it; `picture` is the whole board as one
//! drawing, and the words on it, for a window that draws the live part
//! itself.

pub mod art;
pub mod edit;
pub mod geometry;
pub mod glow;
pub mod layout;
pub mod picture;
pub mod readout;
