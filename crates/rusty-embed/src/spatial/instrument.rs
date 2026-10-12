//! What a pilot's instruments would show for an attitude: the nose above
//! the horizon, the right wing below it, and the heading — read off where
//! the body's axes point in the world, not off the Euler angles, whose
//! signs mean opposite things in the two frames. A positive pitch is the
//! nose down with Z up and the nose up with Z down; the attitude indicator
//! is the same instrument in both.
//!
//! The Math panel draws it and the `math_sheet` tool reports it, so the
//! picture and the words a model uses about an attitude are one reading.

use super::frame::Frame;
use super::rotation::Quat;
use super::vector::Vec3;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Reading {
    /// The nose's elevation above the horizon, radians.
    pub nose_up: f64,
    /// How far the right wing dips below the horizon: a bank to the right,
    /// radians, in `(−π, π]`.
    pub bank_right: f64,
    /// Where the nose points round the horizon, clockwise seen from above
    /// from the world's X, radians in `[0, 2π)`.
    pub heading: f64,
}

/// The body's right wing in its own axes.
fn right_wing(frame: Frame) -> Vec3 {
    match frame {
        Frame::ZUp => -Vec3::Y,
        Frame::ZDown => Vec3::Y,
    }
}

pub fn read(q: Quat, frame: Frame) -> Reading {
    let q = q.normalized().unwrap_or(Quat::IDENTITY);
    let up = frame.up();
    let nose = q.rotate(Vec3::X);
    let wing = q.rotate(right_wing(frame));
    let body_up = q.rotate(up);
    let nose_up = nose.dot(up).clamp(-1.0, 1.0).asin();
    // The bank is the wing's dip, measured in the plane across the nose so
    // it runs the whole way round: upright and rolled past ninety degrees
    // are told apart by which way the body's up points.
    let bank_right = (-wing.dot(up)).atan2(body_up.dot(up));
    // Clockwise from above: from X towards the right-hand side, which is −Y
    // with Z up and +Y with Z down.
    let across = right_wing(frame);
    let heading = nose.dot(across).atan2(nose.dot(Vec3::X));
    Reading {
        nose_up,
        bank_right,
        heading: heading.rem_euclid(std::f64::consts::TAU),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spatial::Euler;

    fn deg(d: f64) -> f64 {
        d.to_radians()
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    /// The same Euler angles are two physical attitudes: a positive pitch
    /// is the nose down with Z up and up with Z down.
    #[test]
    fn a_positive_pitch_reads_as_the_nose_going_down_or_up_by_frame() {
        let q = Quat::from_euler(Euler::new(0.0, deg(30.0), 0.0));
        assert!(close(read(q, Frame::ZUp).nose_up, deg(-30.0)));
        assert!(close(read(q, Frame::ZDown).nose_up, deg(30.0)));
    }

    /// A positive roll puts the right wing down in both.
    #[test]
    fn a_positive_roll_banks_right_in_either_frame() {
        let q = Quat::from_euler(Euler::new(deg(20.0), 0.0, 0.0));
        for frame in [Frame::ZUp, Frame::ZDown] {
            let r = read(q, frame);
            assert!(close(r.bank_right, deg(20.0)), "{frame:?} {r:?}");
            assert!(close(r.nose_up, 0.0));
        }
        // Past ninety, upside down, the bank keeps counting.
        let over = Quat::from_euler(Euler::new(deg(150.0), 0.0, 0.0));
        assert!(close(read(over, Frame::ZUp).bank_right, deg(150.0)));
    }

    /// A positive yaw turns the nose left with Z up (about an up axis) and
    /// right with Z down (about a down one) — the heading says which.
    #[test]
    fn the_heading_is_clockwise_from_above_in_either_frame() {
        let q = Quat::from_euler(Euler::new(0.0, 0.0, deg(30.0)));
        assert!(close(read(q, Frame::ZUp).heading, deg(330.0)));
        assert!(close(read(q, Frame::ZDown).heading, deg(30.0)));
        assert!(close(read(Quat::IDENTITY, Frame::ZUp).heading, 0.0));
    }

    /// The same physical attitude written in the other frame reads the same
    /// on every instrument.
    #[test]
    fn one_attitude_reads_the_same_in_both_frames() {
        let q = Quat::from_euler(Euler::new(deg(-35.0), deg(12.0), deg(100.0)));
        let a = read(q, Frame::ZUp);
        let b = read(Frame::crossed(q), Frame::ZDown);
        assert!(close(a.nose_up, b.nose_up));
        assert!(close(a.bank_right, b.bank_right));
        assert!(close(a.heading, b.heading));
    }
}
