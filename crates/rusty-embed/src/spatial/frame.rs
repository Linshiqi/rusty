//! Which way is up — the one thing flight code disagrees about.

use serde::{Deserialize, Serialize};

use super::rotation::{Euler, Quat};
use super::vector::{TINY, Vec3};

/// The two families of flight code, told apart by where the world's Z
/// points. Everything else — the quaternion, the product, the Euler order —
/// is the same arithmetic in both, which is exactly why mixing them up
/// produces a controller that compiles, runs and flies upside down.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Frame {
    /// World Z up, the body forward-left-up: Crazyflie, ROS, cf-drone-rs.
    /// A positive pitch (about the left wing) puts the nose *down*.
    #[default]
    ZUp,
    /// World Z down (north-east-down), the body forward-right-down: PX4,
    /// ArduPilot, and rusty's plant. A positive pitch (about the right
    /// wing) puts the nose *up*.
    ZDown,
}

impl Frame {
    /// The world's up, in this frame's world axes.
    pub fn up(self) -> Vec3 {
        match self {
            Frame::ZUp => Vec3::Z,
            Frame::ZDown => -Vec3::Z,
        }
    }

    /// What an accelerometer reads at rest, in the body's axes: the specific
    /// force, `g` pointing *up* — an accelerometer feels the floor pushing,
    /// not gravity pulling — seen from a body at attitude `q`.
    pub fn at_rest(self, q: Quat, g: f64) -> Vec3 {
        q.conj().rotate(self.up() * g)
    }

    /// Roll and pitch read off an accelerometer at rest, yaw zero because
    /// gravity says nothing about heading; `None` for a reading with no
    /// direction.
    ///
    /// The inverse of [`at_rest`](Frame::at_rest) for any roll and a pitch
    /// short of the poles, in either frame — the signs are the frame's.
    pub fn tilt(self, acc: Vec3) -> Option<Euler> {
        if acc.norm() < TINY {
            return None;
        }
        let s = self.up().z;
        Some(Euler::new(
            (s * acc.y).atan2(s * acc.z),
            (-s * acc.x).atan2(acc.y.hypot(acc.z)),
            0.0,
        ))
    }

    /// The same physical attitude, written in the other frame: half a turn
    /// about the forward axis at both ends (forward-left-up to
    /// forward-right-down, and the same for the world), which leaves roll
    /// and negates pitch and yaw.
    pub fn crossed(q: Quat) -> Quat {
        Quat::new(q.w, q.x, -q.y, -q.z)
    }

    /// A vector in one frame's axes written in the other's: Y and Z
    /// negated, X shared.
    pub fn crossed_vec(v: Vec3) -> Vec3 {
        Vec3::new(v.x, -v.y, -v.z)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deg(d: f64) -> f64 {
        d.to_radians()
    }

    /// Level, an accelerometer reads `g` along the body's up — `+Z` in a
    /// forward-left-up body, `−Z` in a forward-right-down one.
    #[test]
    fn level_reads_g_along_the_bodys_up() {
        let level = Quat::IDENTITY;
        assert_eq!(Frame::ZUp.at_rest(level, 9.81), Vec3::new(0.0, 0.0, 9.81));
        assert_eq!(
            Frame::ZDown.at_rest(level, 9.81),
            Vec3::new(0.0, 0.0, -9.81)
        );
    }

    #[test]
    fn tilt_reads_back_the_roll_and_pitch_it_was_given_in_both_frames() {
        for frame in [Frame::ZUp, Frame::ZDown] {
            for roll in [-170.0, -60.0, 0.0, 25.0, 100.0] {
                for pitch in [-80.0, -30.0, 0.0, 15.0, 75.0] {
                    let q = Quat::from_euler(Euler::new(deg(roll), deg(pitch), deg(40.0)));
                    let tilt = frame.tilt(frame.at_rest(q, 9.81)).unwrap();
                    assert!(
                        (tilt.roll - deg(roll)).abs() < 1e-9,
                        "{frame:?} {roll} {pitch}: {tilt:?}"
                    );
                    assert!((tilt.pitch - deg(pitch)).abs() < 1e-9);
                    assert_eq!(tilt.yaw, 0.0);
                }
            }
        }
        assert_eq!(Frame::ZUp.tilt(Vec3::ZERO), None);
    }

    /// What each frame means by a positive pitch, which is the whole of the
    /// difference: the nose goes down about a left wing and up about a right
    /// one.
    #[test]
    fn a_positive_pitch_points_the_nose_down_with_z_up_and_up_with_z_down() {
        let q = Quat::from_euler(Euler::new(0.0, deg(30.0), 0.0));
        let nose = q.rotate(Vec3::X);
        assert!(nose.dot(Frame::ZUp.up()) < 0.0, "z up: {nose:?}");
        assert!(nose.dot(Frame::ZDown.up()) > 0.0, "z down: {nose:?}");
    }

    /// The same physical attitude in the two frames: the nose and the up
    /// axis land in the same place once each is written in the other's
    /// axes.
    #[test]
    fn crossing_frames_keeps_the_physical_attitude() {
        let q = Quat::from_euler(Euler::new(deg(20.0), deg(-35.0), deg(70.0)));
        let other = Frame::crossed(q);
        for body in [Vec3::X, Vec3::Y, Vec3::Z] {
            let here = q.rotate(body);
            let there = other.rotate(Frame::crossed_vec(body));
            assert!(
                (Frame::crossed_vec(here) - there).norm() < 1e-12,
                "{body:?}"
            );
        }
        // Roll is shared; pitch and yaw change sign.
        let (a, b) = (q.to_euler(), other.to_euler());
        assert!((a.roll - b.roll).abs() < 1e-12);
        assert!((a.pitch + b.pitch).abs() < 1e-12);
        assert!((a.yaw + b.yaw).abs() < 1e-12);
        // And so does what an accelerometer reads.
        let acc = Frame::ZUp.at_rest(q, 1.0);
        assert!((Frame::crossed_vec(acc) - Frame::ZDown.at_rest(other, 1.0)).norm() < 1e-12);
    }
}
