//! Vectors, rotations and the frames a flight controller is written in: the
//! arithmetic under the math toolbox, and the language its sheet is written
//! in ([`sheet`]). Pure and unconditional, so the panel and its tests run
//! one implementation — and in `f64`, because it is the reference a
//! firmware's `f32` is checked against.
//!
//! **Every convention here is somebody's lost afternoon, so each is stated
//! once:**
//!
//! - A quaternion is Hamilton's, `w` first: `i² = j² = k² = ijk = −1`.
//!   PX4, ArduPilot, Crazyflie, nalgebra, glam and rusty's own plant agree;
//!   a JPL quaternion read as Hamilton's is the inverse rotation.
//! - A rotation *turns* a vector (active): `rotate(q, v) = q ⊗ v ⊗ q*`. An
//!   attitude turns the body's axes onto the world's, so `rotate(q, v)`
//!   takes a body-frame vector into the world and `rotate(q*, v)` brings a
//!   world vector into the body.
//! - `a ⊗ b` turns by `b` first and then by `a`, as matrices multiply. Read
//!   in the body's own axes it goes the other way: `a`, then `b` about the
//!   axes `a` left behind — which is why a body rate integrates on the
//!   right, `q ⊗ Δq`.
//! - Euler angles are Z-Y-X intrinsic: yaw about Z, pitch about the new Y,
//!   roll about the newest X, `q = q_z(yaw) ⊗ q_y(pitch) ⊗ q_x(roll)`.
//! - Which way is up is the one thing the two families of flight code
//!   disagree about ([`Frame`]). The arithmetic is the same in both;
//!   gravity, and what a positive pitch looks like, are not.

pub mod check;
mod frame;
mod rotation;
pub mod sheet;
// The sheet in the project, read and written: files, so the backend's.
#[cfg(feature = "backend")]
pub mod sheet_file;
mod vector;

pub use frame::Frame;
pub use rotation::{Euler, Mat3, POLE, Quat, wrap};
pub use vector::{TINY, Vec2, Vec3};
