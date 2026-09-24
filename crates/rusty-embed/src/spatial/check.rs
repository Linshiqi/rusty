//! Two answers that should agree, and — when they do not — which of the
//! usual mistakes turns one into the other.
//!
//! A flight controller's attitude code is seldom *nearly* right. It is
//! right, or it is right with one convention crossed: the inverse rotation,
//! `w` written last, the angles applied in the other order, the other
//! family's frame, degrees for radians. Each of those produces numbers that
//! look entirely plausible, and each is a closed form away from the right
//! ones — so rather than "differs by 0.83", the check says which.

use super::frame::Frame;
use super::rotation::{Euler, Quat, wrap};
use super::vector::Vec3;

/// What `mine` is, next to the reference it should have equalled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Relation {
    /// The same, within the tolerance.
    Same,
    /// The same rotation written with the other sign: `q` and `−q` are one
    /// attitude, so this is not a mistake.
    OtherSign,
    /// The inverse rotation: world-to-body where body-to-world was meant
    /// (or the reverse), or a JPL quaternion read as Hamilton's.
    Inverse,
    /// The reference with `w` written last: `(x, y, z, w)` read as
    /// `(w, x, y, z)`.
    WLast,
    /// The same three angles applied roll first — X, then Y, then Z —
    /// where yaw first was meant.
    OrderXyz,
    /// The same physical attitude written in the other frame: pitch and yaw
    /// (or a vector's Y and Z) with the other sign.
    OtherFrame,
    /// Pointing the opposite way.
    Opposite,
    /// Degrees where radians were meant: 57.3 times too big.
    Degrees,
    /// Radians where degrees were meant: 57.3 times too small.
    Radians,
    /// None of the above.
    Unexplained,
}

/// The finding: how far apart the two are, and what explains it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Verdict {
    pub relation: Relation,
    /// The distance between the two as given: the angle between rotations,
    /// the length of the difference between vectors, the difference between
    /// numbers.
    pub error: f64,
    /// Whether `error` is an angle, in radians — between rotations or
    /// angles — rather than a length or a plain difference.
    pub angle: bool,
}

/// How close two rotations have to be to be called the same: a tenth of a
/// degree, the precision a firmware printing four decimals of radians and
/// computing in `f32` can be held to.
pub const ROTATION_TOLERANCE: f64 = 2e-3;

/// How close two vectors or numbers have to be, relative to their size.
pub const RELATIVE_TOLERANCE: f64 = 1e-3;

pub fn quaternions(mine: Quat, reference: Quat) -> Verdict {
    let error = mine.angle_to(reference);
    let same = |q: Quat| mine.angle_to(q) < ROTATION_TOLERANCE;
    let relation = if error < ROTATION_TOLERANCE {
        // Equal as rotations; say whether it was written with the other sign.
        if (mine - reference).norm() > (mine + reference).norm() {
            Relation::OtherSign
        } else {
            Relation::Same
        }
    } else if same(reference.conj()) {
        Relation::Inverse
    } else if written_w_last(mine, reference) {
        Relation::WLast
    } else if same(xyz(reference.to_euler())) {
        Relation::OrderXyz
    } else if same(Frame::crossed(reference)) {
        Relation::OtherFrame
    } else {
        Relation::Unexplained
    };
    Verdict {
        relation,
        error,
        angle: true,
    }
}

/// `mine` holds `(x, y, z, w)` of the reference in its `(w, x, y, z)`.
fn written_w_last(mine: Quat, reference: Quat) -> bool {
    Quat::new(mine.z, mine.w, mine.x, mine.y).angle_to(reference) < ROTATION_TOLERANCE
}

/// The three angles applied roll first: `q_x(roll) ⊗ q_y(pitch) ⊗ q_z(yaw)`.
fn xyz(e: Euler) -> Quat {
    Quat::about_x(e.roll) * Quat::about_y(e.pitch) * Quat::about_z(e.yaw)
}

pub fn vectors(mine: Vec3, reference: Vec3) -> Verdict {
    let error = (mine - reference).norm();
    let scale = reference.norm().max(mine.norm()).max(1e-9);
    let near = |v: Vec3| (mine - v).norm() <= RELATIVE_TOLERANCE * scale;
    let relation = if near(reference) {
        Relation::Same
    } else if near(-reference) {
        Relation::Opposite
    } else if near(Frame::crossed_vec(reference)) {
        Relation::OtherFrame
    } else if near(reference * DEGREES_PER_RADIAN) {
        Relation::Degrees
    } else if near(reference / DEGREES_PER_RADIAN) {
        Relation::Radians
    } else {
        Relation::Unexplained
    };
    Verdict {
        relation,
        error,
        angle: false,
    }
}

pub fn eulers(mine: Euler, reference: Euler) -> Verdict {
    let q_mine = Quat::from_euler(mine);
    let q_ref = Quat::from_euler(reference);
    let error = q_mine.angle_to(q_ref);
    let scaled = |k: f64| {
        let e = Euler::new(reference.roll * k, reference.pitch * k, reference.yaw * k);
        close_angles(mine, e, k)
    };
    let relation = if error < ROTATION_TOLERANCE {
        // Equal as rotations, even when the three angles are not: past the
        // poles `(r + π, π − p, y + π)` is the same attitude as `(r, p, y)`.
        Relation::Same
    } else if scaled(DEGREES_PER_RADIAN) {
        Relation::Degrees
    } else if scaled(1.0 / DEGREES_PER_RADIAN) {
        Relation::Radians
    } else if q_mine.angle_to(Frame::crossed(q_ref)) < ROTATION_TOLERANCE {
        Relation::OtherFrame
    } else if q_mine.angle_to(xyz(reference)) < ROTATION_TOLERANCE {
        Relation::OrderXyz
    } else {
        Relation::Unexplained
    };
    Verdict {
        relation,
        error,
        angle: true,
    }
}

/// The three angles one by one, at the scale they were written in: the
/// rotation tolerance, scaled by `k` as the angles were.
fn close_angles(a: Euler, b: Euler, k: f64) -> bool {
    let tolerance = ROTATION_TOLERANCE * k;
    [(a.roll, b.roll), (a.pitch, b.pitch), (a.yaw, b.yaw)]
        .iter()
        .all(|(x, y)| (x - y).abs() <= tolerance)
}

pub fn numbers(mine: f64, reference: f64) -> Verdict {
    let error = (mine - reference).abs();
    let scale = reference.abs().max(mine.abs()).max(1e-9);
    let near = |v: f64| (mine - v).abs() <= RELATIVE_TOLERANCE * scale;
    let relation = if near(reference) {
        Relation::Same
    } else if near(-reference) {
        Relation::Opposite
    } else if near(reference * DEGREES_PER_RADIAN) {
        Relation::Degrees
    } else if near(reference / DEGREES_PER_RADIAN) {
        Relation::Radians
    } else {
        Relation::Unexplained
    };
    Verdict {
        relation,
        error,
        angle: false,
    }
}

/// Two angles, in radians: numbers, except that a whole number of turns
/// apart is the same angle.
pub fn angles(mine: f64, reference: f64) -> Verdict {
    let turned = wrap(mine - reference).abs();
    if turned <= ROTATION_TOLERANCE {
        return Verdict {
            relation: Relation::Same,
            error: turned,
            angle: true,
        };
    }
    let verdict = numbers(mine, reference);
    Verdict {
        error: turned,
        angle: true,
        ..verdict
    }
}

const DEGREES_PER_RADIAN: f64 = 180.0 / std::f64::consts::PI;

#[cfg(test)]
mod tests {
    use super::*;

    fn deg(d: f64) -> f64 {
        d.to_radians()
    }

    fn reference() -> Quat {
        Quat::from_euler(Euler::new(deg(20.0), deg(-15.0), deg(60.0)))
    }

    #[test]
    fn each_crossed_convention_is_named() {
        let q = reference();
        let e = q.to_euler();
        let cases = [
            (q, Relation::Same),
            (-q, Relation::OtherSign),
            (q.conj(), Relation::Inverse),
            (Quat::new(q.x, q.y, q.z, q.w), Relation::WLast),
            (xyz(e), Relation::OrderXyz),
            (Frame::crossed(q), Relation::OtherFrame),
            (Quat::about_x(1.0), Relation::Unexplained),
        ];
        for (mine, want) in cases {
            assert_eq!(quaternions(mine, q).relation, want, "{mine:?}");
        }
        assert!(quaternions(q.conj(), q).error > 0.1);
    }

    /// A tenth of a degree of rounding is the same attitude — a firmware
    /// printing four decimals of radians is not wrong.
    #[test]
    fn rounding_a_firmware_prints_is_the_same() {
        let q = reference();
        let e = q.to_euler();
        let printed = |v: f64| (v * 1e4).round() / 1e4;
        let mine = Quat::from_euler(Euler::new(
            printed(e.roll),
            printed(e.pitch),
            printed(e.yaw),
        ));
        assert_eq!(quaternions(mine, q).relation, Relation::Same);
    }

    #[test]
    fn a_vector_names_its_sign_its_frame_and_its_units() {
        let r = Vec3::new(1.5, -2.0, 9.0);
        assert_eq!(vectors(r, r).relation, Relation::Same);
        assert_eq!(vectors(-r, r).relation, Relation::Opposite);
        assert_eq!(
            vectors(Frame::crossed_vec(r), r).relation,
            Relation::OtherFrame
        );
        assert_eq!(
            vectors(r * DEGREES_PER_RADIAN, r).relation,
            Relation::Degrees
        );
        assert_eq!(
            vectors(r / DEGREES_PER_RADIAN, r).relation,
            Relation::Radians
        );
        assert_eq!(
            vectors(Vec3::new(1.0, 0.0, 0.0), r).relation,
            Relation::Unexplained
        );
    }

    #[test]
    fn euler_angles_name_units_frames_and_order() {
        let r = Euler::new(deg(20.0), deg(-15.0), deg(60.0));
        assert_eq!(eulers(r, r).relation, Relation::Same);
        let in_degrees = Euler::new(20.0, -15.0, 60.0);
        assert_eq!(eulers(in_degrees, r).relation, Relation::Degrees);
        let tiny = Euler::new(
            r.roll / 57.29577951308232,
            r.pitch / 57.29577951308232,
            r.yaw / 57.29577951308232,
        );
        assert_eq!(eulers(tiny, r).relation, Relation::Radians);
        let crossed = Euler::new(r.roll, -r.pitch, -r.yaw);
        assert_eq!(eulers(crossed, r).relation, Relation::OtherFrame);
        let applied_the_other_way = xyz(r).to_euler();
        assert_eq!(
            eulers(applied_the_other_way, r).relation,
            Relation::OrderXyz
        );
        // Past the pole, three different angles for one attitude.
        let flipped = Euler::new(
            r.roll + deg(180.0),
            deg(180.0) - r.pitch,
            r.yaw + deg(180.0),
        );
        assert_eq!(eulers(flipped, r).relation, Relation::Same);
    }

    #[test]
    fn numbers_name_signs_and_units() {
        assert_eq!(numbers(0.5, 0.5).relation, Relation::Same);
        assert_eq!(numbers(-0.5, 0.5).relation, Relation::Opposite);
        assert_eq!(numbers(30.0, deg(30.0)).relation, Relation::Degrees);
        assert_eq!(numbers(deg(30.0), 30.0).relation, Relation::Radians);
        assert_eq!(numbers(1.0, 2.0).relation, Relation::Unexplained);
        // A number is not an angle: two pi apart is two pi apart.
        let tau = std::f64::consts::TAU;
        assert_eq!(numbers(1.0 + tau, 1.0).relation, Relation::Unexplained);
    }

    #[test]
    fn angles_a_whole_turn_apart_are_the_same() {
        let tau = std::f64::consts::TAU;
        assert_eq!(angles(deg(30.0) + tau, deg(30.0)).relation, Relation::Same);
        assert_eq!(angles(deg(179.99), deg(-179.99)).relation, Relation::Same);
        assert!(angles(deg(179.99), deg(-179.99)).error < 1e-3);
        assert_eq!(angles(30.0, deg(30.0)).relation, Relation::Degrees);
        assert_eq!(angles(-deg(30.0), deg(30.0)).relation, Relation::Opposite);
    }
}
