//! Rotations: quaternions, Euler angles and rotation matrices, and the ways
//! between them. The conventions are the module's (`spatial`), stated there
//! once.

use std::f64::consts::{FRAC_PI_2, PI};
use std::ops::{Add, Mul, Neg, Sub};

use serde::{Deserialize, Serialize};

use super::vector::{TINY, Vec3};

/// A quaternion, Hamilton's, `w` first. An attitude when it is unit. On the
/// wire its four parts go by name, so no reader has an order to assume.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Quat {
    pub w: f64,
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl Default for Quat {
    /// Level, facing forward.
    fn default() -> Self {
        Quat::IDENTITY
    }
}

/// Roll, pitch and yaw in radians, Z-Y-X intrinsic: yaw about Z, then pitch
/// about the new Y, then roll about the newest X.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Euler {
    pub roll: f64,
    pub pitch: f64,
    pub yaw: f64,
}

/// `sin(pitch)` past which an attitude is at a pole: pointing straight up
/// or down, where roll and yaw turn about the same axis and only their
/// difference (or sum) exists. The same line cf-drone-rs draws.
pub const POLE: f64 = 0.99999;

impl Euler {
    pub const fn new(roll: f64, pitch: f64, yaw: f64) -> Self {
        Euler { roll, pitch, yaw }
    }

    /// Pointing within a hair of straight up or down, where the three angles
    /// stop being three.
    pub fn at_pole(self) -> bool {
        self.pitch.sin().abs() >= POLE
    }
}

/// A 3×3 matrix, by rows. A rotation matrix's columns are the body's axes
/// seen from the world.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mat3 {
    pub m: [[f64; 3]; 3],
}

impl Quat {
    pub const IDENTITY: Quat = Quat::new(1.0, 0.0, 0.0, 0.0);

    pub const fn new(w: f64, x: f64, y: f64, z: f64) -> Self {
        Quat { w, x, y, z }
    }

    /// A pure quaternion, `(0, v)` — how a vector enters a sandwich product.
    pub const fn pure(v: Vec3) -> Self {
        Quat::new(0.0, v.x, v.y, v.z)
    }

    /// The vector part, `(x, y, z)`.
    pub fn vector(self) -> Vec3 {
        Vec3::new(self.x, self.y, self.z)
    }

    /// A turn of `angle` radians about `axis`, anticlockwise looking down the
    /// axis at the origin (right-hand rule); `None` for an axis with no
    /// direction.
    pub fn from_axis_angle(axis: Vec3, angle: f64) -> Option<Quat> {
        let axis = axis.normalized()?;
        let (s, c) = (angle * 0.5).sin_cos();
        Some(Quat::new(c, axis.x * s, axis.y * s, axis.z * s))
    }

    pub fn about_x(angle: f64) -> Quat {
        let (s, c) = (angle * 0.5).sin_cos();
        Quat::new(c, s, 0.0, 0.0)
    }

    pub fn about_y(angle: f64) -> Quat {
        let (s, c) = (angle * 0.5).sin_cos();
        Quat::new(c, 0.0, s, 0.0)
    }

    pub fn about_z(angle: f64) -> Quat {
        let (s, c) = (angle * 0.5).sin_cos();
        Quat::new(c, 0.0, 0.0, s)
    }

    /// The turn a rotation vector names: its direction the axis, its length
    /// the angle — what a body rate times a time step is, and what an
    /// attitude error is in most controllers.
    pub fn from_rotvec(v: Vec3) -> Quat {
        let angle = v.norm();
        if angle < 1e-9 {
            // sin(θ/2)/θ → 1/2: the first term of the series, normalised.
            return Quat::new(1.0, v.x * 0.5, v.y * 0.5, v.z * 0.5)
                .normalized()
                .unwrap_or(Quat::IDENTITY);
        }
        Quat::from_axis_angle(v, angle).unwrap_or(Quat::IDENTITY)
    }

    /// The rotation vector of the shorter of the two turns this quaternion
    /// and its negative both name: axis times an angle in `[0, π]`.
    pub fn to_rotvec(self) -> Vec3 {
        let Some(q) = self.normalized() else {
            return Vec3::ZERO;
        };
        let q = if q.w < 0.0 { -q } else { q };
        let v = q.vector();
        let s = v.norm();
        if s < 1e-12 {
            return v * 2.0;
        }
        v * (2.0 * s.atan2(q.w) / s)
    }

    /// The axis and the angle, the angle in `[0, π]` — the shorter way
    /// round; `None` for no turn at all, which has no axis.
    pub fn axis_angle(self) -> Option<(Vec3, f64)> {
        let rv = self.to_rotvec();
        let angle = rv.norm();
        Some((rv.normalized()?, angle))
    }

    pub fn norm_squared(self) -> f64 {
        self.dot(self)
    }

    pub fn norm(self) -> f64 {
        self.norm_squared().sqrt()
    }

    /// On the unit sphere, or `None` for one too small to put there.
    pub fn normalized(self) -> Option<Quat> {
        let n = self.norm();
        (n > TINY && n.is_finite()).then(|| self * (1.0 / n))
    }

    /// The conjugate, `(w, −x, −y, −z)`: the inverse of a unit quaternion.
    pub fn conj(self) -> Quat {
        Quat::new(self.w, -self.x, -self.y, -self.z)
    }

    /// The inverse, for any quaternion that is not zero.
    pub fn inverse(self) -> Option<Quat> {
        let n2 = self.norm_squared();
        (n2 > TINY * TINY).then(|| self.conj() * (1.0 / n2))
    }

    /// The four-component dot product: `cos(θ/2)` of the turn between two
    /// unit quaternions, up to sign.
    pub fn dot(self, other: Quat) -> f64 {
        self.w * other.w + self.x * other.x + self.y * other.y + self.z * other.z
    }

    /// `q ⊗ v ⊗ q*`: `v` turned by `q`. For a unit `q`; one off the unit
    /// sphere also scales `v` by `|q|²`, which is what an attitude that was
    /// never renormalised does to every vector it turns.
    pub fn rotate(self, v: Vec3) -> Vec3 {
        (self * Quat::pure(v) * self.conj()).vector()
    }

    /// The rotation matrix, whose columns are where `X`, `Y` and `Z` go.
    ///
    /// The homogeneous form, which equals the sandwich product for any
    /// quaternion — so a quaternion off the unit sphere gives a matrix off
    /// the orthonormal ones by the same `|q|²`, rather than quietly a
    /// different rotation.
    pub fn to_mat3(self) -> Mat3 {
        let Quat { w, x, y, z } = self;
        Mat3 {
            m: [
                [
                    w * w + x * x - y * y - z * z,
                    2.0 * (x * y - w * z),
                    2.0 * (x * z + w * y),
                ],
                [
                    2.0 * (x * y + w * z),
                    w * w - x * x + y * y - z * z,
                    2.0 * (y * z - w * x),
                ],
                [
                    2.0 * (x * z - w * y),
                    2.0 * (y * z + w * x),
                    w * w - x * x - y * y + z * z,
                ],
            ],
        }
    }

    /// The unit quaternion of a rotation matrix, by Shepperd's method: the
    /// largest of the four squares taken first, so no branch divides by a
    /// component near zero.
    pub fn from_mat3(r: Mat3) -> Quat {
        let m = r.m;
        let trace = m[0][0] + m[1][1] + m[2][2];
        let q = if trace > 0.0 {
            let s = (trace + 1.0).sqrt() * 2.0;
            Quat::new(
                s / 4.0,
                (m[2][1] - m[1][2]) / s,
                (m[0][2] - m[2][0]) / s,
                (m[1][0] - m[0][1]) / s,
            )
        } else if m[0][0] > m[1][1] && m[0][0] > m[2][2] {
            let s = (1.0 + m[0][0] - m[1][1] - m[2][2]).sqrt() * 2.0;
            Quat::new(
                (m[2][1] - m[1][2]) / s,
                s / 4.0,
                (m[0][1] + m[1][0]) / s,
                (m[0][2] + m[2][0]) / s,
            )
        } else if m[1][1] > m[2][2] {
            let s = (1.0 + m[1][1] - m[0][0] - m[2][2]).sqrt() * 2.0;
            Quat::new(
                (m[0][2] - m[2][0]) / s,
                (m[0][1] + m[1][0]) / s,
                s / 4.0,
                (m[1][2] + m[2][1]) / s,
            )
        } else {
            let s = (1.0 + m[2][2] - m[0][0] - m[1][1]).sqrt() * 2.0;
            Quat::new(
                (m[1][0] - m[0][1]) / s,
                (m[0][2] + m[2][0]) / s,
                (m[1][2] + m[2][1]) / s,
                s / 4.0,
            )
        };
        q.normalized().unwrap_or(Quat::IDENTITY)
    }

    /// `q_z(yaw) ⊗ q_y(pitch) ⊗ q_x(roll)`, multiplied out.
    pub fn from_euler(e: Euler) -> Quat {
        let (sr, cr) = (e.roll * 0.5).sin_cos();
        let (sp, cp) = (e.pitch * 0.5).sin_cos();
        let (sy, cy) = (e.yaw * 0.5).sin_cos();
        Quat::new(
            cr * cp * cy + sr * sp * sy,
            sr * cp * cy - cr * sp * sy,
            cr * sp * cy + sr * cp * sy,
            cr * cp * sy - sr * sp * cy,
        )
    }

    /// Roll, pitch and yaw, with the poles answered rather than left to make
    /// a NaN out of a rounding error at ninety degrees.
    ///
    /// At a pole roll and yaw turn about one axis, so only their difference
    /// (pointing up) or sum (pointing down) is knowable; all of it is given
    /// to yaw and roll is zero. Written for any scale of quaternion — every
    /// term is over `|q|²` or is a ratio — so an unnormalised attitude reads
    /// as the attitude it is.
    pub fn to_euler(self) -> Euler {
        let Quat { w, x, y, z } = self;
        let n2 = self.norm_squared();
        if n2 < TINY * TINY {
            return Euler::default();
        }
        let sin_pitch = 2.0 * (w * y - x * z) / n2;
        if sin_pitch >= POLE {
            Euler::new(0.0, FRAC_PI_2, wrap(2.0 * z.atan2(y)))
        } else if sin_pitch <= -POLE {
            Euler::new(0.0, -FRAC_PI_2, wrap(2.0 * x.atan2(w)))
        } else {
            Euler::new(
                (2.0 * (w * x + y * z)).atan2(w * w - x * x - y * y + z * z),
                sin_pitch.asin(),
                (2.0 * (w * z + x * y)).atan2(w * w + x * x - y * y - z * z),
            )
        }
    }

    /// The angle of the turn from one attitude to the other, in `[0, π]`:
    /// the attitude error a controller closes, whichever sign either
    /// quaternion was written with.
    pub fn angle_to(self, other: Quat) -> f64 {
        let delta = self.conj() * other;
        2.0 * delta.vector().norm().atan2(delta.w.abs())
    }

    /// The turn that takes this attitude to `other`, in this one's own axes:
    /// `self* ⊗ other`, so `self ⊗ delta = other`.
    pub fn delta_to(self, other: Quat) -> Quat {
        self.conj() * other
    }

    /// Along the great circle from `self` (`t = 0`) to `other` (`t = 1`) at
    /// an even rate, the short way round.
    pub fn slerp(self, other: Quat, t: f64) -> Quat {
        let mut other = other;
        let mut d = self.dot(other);
        if d < 0.0 {
            other = -other;
            d = -d;
        }
        if d > 0.9995 {
            // So close that sin θ is rounding: a straight line, renormalised.
            return (self * (1.0 - t) + other * t).normalized().unwrap_or(self);
        }
        let theta = d.clamp(-1.0, 1.0).acos();
        let s = theta.sin();
        self * (((1.0 - t) * theta).sin() / s) + other * ((t * theta).sin() / s)
    }

    /// The shortest turn that takes the direction of `from` onto that of
    /// `to`; `None` when either has no direction. Opposite directions have
    /// every perpendicular as an axis, and one is chosen.
    pub fn from_to(from: Vec3, to: Vec3) -> Option<Quat> {
        let a = from.normalized()?;
        let b = to.normalized()?;
        let d = a.dot(b);
        if d < -1.0 + 1e-12 {
            return Quat::from_axis_angle(a.any_perpendicular(), PI);
        }
        let c = a.cross(b);
        Quat::new(1.0 + d, c.x, c.y, c.z).normalized()
    }

    /// One step of a body-frame rate: `q ⊗ exp(ω·dt/2)`, exactly — the turn
    /// a constant rate makes in `dt`, on the right because the rate is in
    /// the body's axes.
    pub fn integrate(self, rate: Vec3, dt: f64) -> Quat {
        self * Quat::from_rotvec(rate * dt)
    }

    /// The same step to first order, `q + ½·q ⊗ (0, ω)·dt`, and not
    /// renormalised: what most firmware computes before it normalises, so
    /// the distance between this and `integrate` is the error it makes.
    pub fn integrate_linear(self, rate: Vec3, dt: f64) -> Quat {
        self + self * Quat::pure(rate) * (0.5 * dt)
    }

    pub fn is_finite(self) -> bool {
        self.w.is_finite() && self.x.is_finite() && self.y.is_finite() && self.z.is_finite()
    }
}

/// An angle into `(−π, π]`.
pub fn wrap(angle: f64) -> f64 {
    let turned = (angle + PI).rem_euclid(2.0 * PI) - PI;
    if turned <= -PI {
        turned + 2.0 * PI
    } else {
        turned
    }
}

/// The Hamilton product: `a ⊗ b` turns by `b`, then by `a`.
impl Mul for Quat {
    type Output = Quat;
    fn mul(self, b: Quat) -> Quat {
        let a = self;
        Quat::new(
            a.w * b.w - a.x * b.x - a.y * b.y - a.z * b.z,
            a.w * b.x + a.x * b.w + a.y * b.z - a.z * b.y,
            a.w * b.y - a.x * b.z + a.y * b.w + a.z * b.x,
            a.w * b.z + a.x * b.y - a.y * b.x + a.z * b.w,
        )
    }
}

impl Mul<f64> for Quat {
    type Output = Quat;
    fn mul(self, k: f64) -> Quat {
        Quat::new(self.w * k, self.x * k, self.y * k, self.z * k)
    }
}

impl Add for Quat {
    type Output = Quat;
    fn add(self, o: Quat) -> Quat {
        Quat::new(self.w + o.w, self.x + o.x, self.y + o.y, self.z + o.z)
    }
}

impl Sub for Quat {
    type Output = Quat;
    fn sub(self, o: Quat) -> Quat {
        Quat::new(self.w - o.w, self.x - o.x, self.y - o.y, self.z - o.z)
    }
}

impl Neg for Quat {
    type Output = Quat;
    fn neg(self) -> Quat {
        Quat::new(-self.w, -self.x, -self.y, -self.z)
    }
}

impl Mat3 {
    pub const IDENTITY: Mat3 = Mat3 {
        m: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
    };

    pub fn from_rows(r0: Vec3, r1: Vec3, r2: Vec3) -> Mat3 {
        Mat3 {
            m: [r0.to_array(), r1.to_array(), r2.to_array()],
        }
    }

    pub fn from_cols(c0: Vec3, c1: Vec3, c2: Vec3) -> Mat3 {
        Mat3::from_rows(c0, c1, c2).transpose()
    }

    pub fn row(self, i: usize) -> Vec3 {
        let [x, y, z] = self.m[i];
        Vec3::new(x, y, z)
    }

    pub fn col(self, j: usize) -> Vec3 {
        Vec3::new(self.m[0][j], self.m[1][j], self.m[2][j])
    }

    pub fn transpose(self) -> Mat3 {
        let m = self.m;
        Mat3 {
            m: [
                [m[0][0], m[1][0], m[2][0]],
                [m[0][1], m[1][1], m[2][1]],
                [m[0][2], m[1][2], m[2][2]],
            ],
        }
    }

    pub fn det(self) -> f64 {
        self.row(0).dot(self.row(1).cross(self.row(2)))
    }

    pub fn mul_vec(self, v: Vec3) -> Vec3 {
        Vec3::new(self.row(0).dot(v), self.row(1).dot(v), self.row(2).dot(v))
    }

    /// How far from a rotation: the largest entry of `RᵀR − I`. Zero for a
    /// rotation (and a reflection, which `det` tells apart).
    pub fn orthonormal_error(self) -> f64 {
        let p = self.transpose() * self;
        let mut worst: f64 = 0.0;
        for (i, row) in p.m.iter().enumerate() {
            for (j, value) in row.iter().enumerate() {
                let want = if i == j { 1.0 } else { 0.0 };
                worst = worst.max((value - want).abs());
            }
        }
        worst
    }

    pub fn is_finite(self) -> bool {
        self.m.iter().flatten().all(|v| v.is_finite())
    }
}

impl Mul for Mat3 {
    type Output = Mat3;
    fn mul(self, b: Mat3) -> Mat3 {
        let cols = [b.col(0), b.col(1), b.col(2)];
        let mut m = [[0.0; 3]; 3];
        for (i, row) in m.iter_mut().enumerate() {
            for (j, cell) in row.iter_mut().enumerate() {
                *cell = self.row(i).dot(cols[j]);
            }
        }
        Mat3 { m }
    }
}

impl Mul<f64> for Mat3 {
    type Output = Mat3;
    fn mul(self, k: f64) -> Mat3 {
        let mut m = self.m;
        m.iter_mut().flatten().for_each(|v| *v *= k);
        Mat3 { m }
    }
}

impl Add for Mat3 {
    type Output = Mat3;
    fn add(self, b: Mat3) -> Mat3 {
        let mut m = self.m;
        for (row, other) in m.iter_mut().zip(b.m) {
            for (v, o) in row.iter_mut().zip(other) {
                *v += o;
            }
        }
        Mat3 { m }
    }
}

impl Sub for Mat3 {
    type Output = Mat3;
    fn sub(self, b: Mat3) -> Mat3 {
        self + b * -1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f64 = 1e-12;

    fn near(a: Vec3, b: Vec3) -> bool {
        (a - b).norm() < 1e-9
    }

    /// Equal as rotations: `q` and `−q` are one attitude.
    fn same_turn(a: Quat, b: Quat) -> bool {
        a.angle_to(b) < 1e-9
    }

    fn deg(d: f64) -> f64 {
        d.to_radians()
    }

    /// The right-hand rule, which every sign in a flight controller hangs
    /// from: a quarter turn about Z takes X to Y, about X takes Y to Z,
    /// about Y takes Z to X.
    #[test]
    fn a_quarter_turn_follows_the_right_hand_rule() {
        assert!(near(Quat::about_z(FRAC_PI_2).rotate(Vec3::X), Vec3::Y));
        assert!(near(Quat::about_x(FRAC_PI_2).rotate(Vec3::Y), Vec3::Z));
        assert!(near(Quat::about_y(FRAC_PI_2).rotate(Vec3::Z), Vec3::X));
        let q = Quat::from_axis_angle(Vec3::new(0.0, 0.0, 5.0), FRAC_PI_2).unwrap();
        assert!(same_turn(q, Quat::about_z(FRAC_PI_2)));
    }

    /// `a ⊗ b` turns by `b` first: the matrix product in the same order.
    #[test]
    fn a_product_turns_by_the_right_hand_factor_first() {
        let a = Quat::about_z(FRAC_PI_2);
        let b = Quat::about_x(FRAC_PI_2);
        let v = Vec3::Y;
        assert!(near((a * b).rotate(v), a.rotate(b.rotate(v))));
        assert!(near(
            (a * b).to_mat3().mul_vec(v),
            a.to_mat3().mul_vec(b.to_mat3().mul_vec(v))
        ));
        // And the order matters: the two answers differ.
        assert!(!near((a * b).rotate(v), (b * a).rotate(v)));
    }

    #[test]
    fn the_matrix_is_the_sandwich_and_its_columns_are_the_turned_axes() {
        let q = Quat::from_euler(Euler::new(deg(30.0), deg(-20.0), deg(135.0)));
        let r = q.to_mat3();
        assert!(near(r.col(0), q.rotate(Vec3::X)));
        assert!(near(r.col(1), q.rotate(Vec3::Y)));
        assert!(near(r.col(2), q.rotate(Vec3::Z)));
        assert!(r.orthonormal_error() < EPS * 10.0);
        assert!((r.det() - 1.0).abs() < 1e-12);
        let v = Vec3::new(0.3, -2.0, 1.5);
        assert!(near(r.mul_vec(v), q.rotate(v)));
    }

    /// Shepperd's method from every branch: a turn about each axis near a
    /// half turn puts the largest square on each diagonal entry in turn.
    #[test]
    fn a_matrix_comes_back_as_its_quaternion_from_every_branch() {
        for q in [
            Quat::IDENTITY,
            Quat::about_x(deg(179.0)),
            Quat::about_y(deg(179.0)),
            Quat::about_z(deg(179.0)),
            Quat::from_euler(Euler::new(deg(10.0), deg(20.0), deg(30.0))),
            Quat::from_axis_angle(Vec3::new(1.0, -2.0, 0.5), deg(250.0)).unwrap(),
        ] {
            assert!(same_turn(Quat::from_mat3(q.to_mat3()), q), "{q:?}");
        }
    }

    /// Z-Y-X intrinsic is the product of the three axis turns in that
    /// order, and the closed form is that product multiplied out.
    #[test]
    fn euler_angles_are_yaw_then_pitch_then_roll() {
        let e = Euler::new(deg(25.0), deg(-40.0), deg(100.0));
        let composed = Quat::about_z(e.yaw) * Quat::about_y(e.pitch) * Quat::about_x(e.roll);
        let q = Quat::from_euler(e);
        assert!((q - composed).norm() < EPS);
    }

    #[test]
    fn euler_angles_come_back_away_from_the_poles() {
        for roll in [-170.0, -90.0, -30.0, 0.0, 45.0, 120.0, 179.0] {
            for pitch in [-85.0, -45.0, 0.0, 10.0, 60.0, 89.0] {
                for yaw in [-179.0, -60.0, 0.0, 90.0, 180.0] {
                    let e = Euler::new(deg(roll), deg(pitch), deg(yaw));
                    let back = Quat::from_euler(e).to_euler();
                    assert!((wrap(back.roll - e.roll)).abs() < 1e-9, "{e:?} → {back:?}");
                    assert!((back.pitch - e.pitch).abs() < 1e-9, "{e:?} → {back:?}");
                    assert!((wrap(back.yaw - e.yaw)).abs() < 1e-9, "{e:?} → {back:?}");
                }
            }
        }
    }

    /// At a pole roll and yaw are one axis: the reading gives it all to yaw,
    /// and must name the *same attitude* — the seam cf-drone-rs found its
    /// heading flipping across.
    #[test]
    fn at_a_pole_the_reading_is_the_same_attitude_with_roll_given_to_yaw() {
        for pitch in [90.0, -90.0, 89.9999, -89.9999] {
            for (roll, yaw) in [(0.0, 0.0), (30.0, 10.0), (-120.0, 170.0), (90.0, -90.0)] {
                let q = Quat::from_euler(Euler::new(deg(roll), deg(pitch), deg(yaw)));
                let e = q.to_euler();
                assert!(e.at_pole(), "{pitch}");
                assert_eq!(e.roll, 0.0);
                let back = Quat::from_euler(e);
                assert!(back.angle_to(q) < 1e-3, "{roll} {pitch} {yaw} → {e:?}");
                assert!(e.yaw.abs() <= PI);
            }
        }
    }

    #[test]
    fn an_unnormalised_quaternion_reads_as_the_attitude_it_scales() {
        let e = Euler::new(deg(12.0), deg(-7.0), deg(33.0));
        let q = Quat::from_euler(e) * 1.37;
        let back = q.to_euler();
        assert!((back.roll - e.roll).abs() < 1e-12);
        assert!((back.pitch - e.pitch).abs() < 1e-12);
        assert!((back.yaw - e.yaw).abs() < 1e-12);
        // …while turning a vector scales it by |q|².
        let v = q.rotate(Vec3::X);
        assert!((v.norm() - 1.37 * 1.37).abs() < 1e-12);
    }

    #[test]
    fn a_rotation_vector_is_the_axis_times_the_angle_both_ways() {
        let v = Vec3::new(0.3, -0.4, 1.2);
        let q = Quat::from_rotvec(v);
        let (axis, angle) = q.axis_angle().unwrap();
        assert!((angle - v.norm()).abs() < EPS);
        assert!(near(axis, v.normalized().unwrap()));
        assert!(near(q.to_rotvec(), v));
        // The negative names the same turn and gives the same vector back.
        assert!(near((-q).to_rotvec(), v));
        // A turn past a half comes back as the shorter one the other way.
        let long = Quat::about_z(deg(270.0));
        assert!(near(long.to_rotvec(), Vec3::new(0.0, 0.0, deg(-90.0))));
        // And none has no axis.
        assert!(Quat::IDENTITY.axis_angle().is_none());
        assert!(near(
            Quat::from_rotvec(Vec3::new(1e-12, 0.0, 0.0)).to_rotvec(),
            Vec3::ZERO
        ));
    }

    /// The error between two attitudes, however either was signed.
    #[test]
    fn the_angle_between_attitudes_ignores_the_sign() {
        let a = Quat::about_x(deg(10.0));
        let b = Quat::about_x(deg(35.0));
        assert!((a.angle_to(b) - deg(25.0)).abs() < 1e-12);
        assert!((a.angle_to(-b) - deg(25.0)).abs() < 1e-12);
        assert!((a * a.delta_to(b) - b).norm() < EPS);
    }

    #[test]
    fn slerp_runs_the_short_way_at_an_even_rate() {
        let a = Quat::about_z(deg(10.0));
        let b = Quat::about_z(deg(130.0));
        assert!(same_turn(a.slerp(b, 0.0), a));
        assert!(same_turn(a.slerp(b, 1.0), b));
        for t in [0.1, 0.25, 0.5, 0.9] {
            let q = a.slerp(b, t);
            assert!((q.norm() - 1.0).abs() < EPS);
            assert!((a.angle_to(q) - t * deg(120.0)).abs() < 1e-9);
        }
        // Written with the other sign it still goes the short way.
        let q = a.slerp(-b, 0.5);
        assert!(same_turn(q, Quat::about_z(deg(70.0))));
        // And two all but equal ones do not divide by nothing.
        let c = Quat::about_z(deg(10.0 + 1e-9));
        assert!(a.slerp(c, 0.5).is_finite());
    }

    #[test]
    fn the_shortest_turn_between_two_directions_takes_one_onto_the_other() {
        let pairs = [
            (Vec3::X, Vec3::Y),
            (Vec3::new(1.0, 2.0, 3.0), Vec3::new(-2.0, 0.5, 1.0)),
            (Vec3::Z, Vec3::Z * 4.0),
            (Vec3::new(0.0, 0.0, 1.0), Vec3::new(0.0, 0.0, -1.0)),
            (Vec3::new(1.0, 1.0, 0.0), Vec3::new(-1.0, -1.0, 0.0)),
        ];
        for (a, b) in pairs {
            let q = Quat::from_to(a, b).unwrap();
            assert!(
                near(q.rotate(a.normalized().unwrap()), b.normalized().unwrap()),
                "{a:?} → {b:?}"
            );
        }
        assert_eq!(Quat::from_to(Vec3::ZERO, Vec3::X), None);
        // The shortest: the angle of the turn is the angle between them.
        let q = Quat::from_to(Vec3::X, Vec3::new(1.0, 1.0, 0.0)).unwrap();
        assert!((q.angle_to(Quat::IDENTITY) - deg(45.0)).abs() < 1e-12);
    }

    /// Ninety degrees a second about the body's Z for a second, in a
    /// thousand steps, is a quarter turn about Z — and to first order it
    /// walks off the unit sphere by what the second-order term leaves out.
    #[test]
    fn a_body_rate_integrates_to_the_turn_it_makes() {
        let rate = Vec3::new(0.0, 0.0, deg(90.0));
        let (mut exact, mut linear) = (Quat::IDENTITY, Quat::IDENTITY);
        for _ in 0..1000 {
            exact = exact.integrate(rate, 0.001);
            linear = linear.integrate_linear(rate, 0.001);
        }
        assert!(same_turn(exact, Quat::about_z(FRAC_PI_2)));
        assert!((exact.norm() - 1.0).abs() < 1e-12);
        assert!(linear.norm() > 1.0 + 1e-7, "{}", linear.norm());
        assert!(linear.normalized().unwrap().angle_to(exact) < 1e-6);
        // In the body's axes: after a quarter turn about Z, a roll rate
        // turns about the body's X — which is now the world's Y.
        let turned = Quat::about_z(FRAC_PI_2).integrate(Vec3::new(0.1, 0.0, 0.0), 1.0);
        let axis = Quat::about_z(FRAC_PI_2)
            .delta_to(turned)
            .axis_angle()
            .unwrap()
            .0;
        assert!(near(axis, Vec3::X));
        let world = turned * Quat::about_z(FRAC_PI_2).conj();
        assert!(near(world.axis_angle().unwrap().0, Vec3::Y));
    }

    #[test]
    fn wrap_lands_in_the_half_open_turn() {
        assert!((wrap(3.0 * PI) - PI).abs() < 1e-12);
        assert!((wrap(-PI) - PI).abs() < 1e-12);
        assert!((wrap(deg(370.0)) - deg(10.0)).abs() < 1e-12);
        assert!((wrap(deg(-190.0)) - deg(170.0)).abs() < 1e-12);
    }
}
