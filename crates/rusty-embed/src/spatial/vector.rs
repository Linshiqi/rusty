//! Vectors in the plane and in space.

use std::ops::{Add, Div, Mul, Neg, Sub};

/// Shorter than this and a vector has a length but no usable direction:
/// dividing by it turns rounding noise into an arbitrary unit vector.
pub const TINY: f64 = 1e-12;

/// A vector in the plane.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Vec2 {
    pub x: f64,
    pub y: f64,
}

impl Vec2 {
    pub const ZERO: Vec2 = Vec2::new(0.0, 0.0);

    pub const fn new(x: f64, y: f64) -> Self {
        Vec2 { x, y }
    }

    pub fn dot(self, other: Vec2) -> f64 {
        self.x * other.x + self.y * other.y
    }

    /// The `z` of the cross product in space: the signed area of the
    /// parallelogram the two span, positive when `other` is anticlockwise
    /// of `self`.
    pub fn cross(self, other: Vec2) -> f64 {
        self.x * other.y - self.y * other.x
    }

    pub fn norm(self) -> f64 {
        self.x.hypot(self.y)
    }

    /// A unit vector the same way, or `None` for one too short to have a
    /// way.
    pub fn normalized(self) -> Option<Vec2> {
        let n = self.norm();
        (n > TINY && n.is_finite()).then(|| self / n)
    }

    /// Turned anticlockwise by `angle` radians.
    pub fn rotated(self, angle: f64) -> Vec2 {
        let (s, c) = angle.sin_cos();
        Vec2::new(c * self.x - s * self.y, s * self.x + c * self.y)
    }

    /// The angle from `self` round to `other`, anticlockwise positive, in
    /// `(−π, π]`; `None` when either has no direction.
    pub fn angle_to(self, other: Vec2) -> Option<f64> {
        (self.norm() > TINY && other.norm() > TINY)
            .then(|| self.cross(other).atan2(self.dot(other)))
    }

    /// The part of `self` along `onto`.
    pub fn project_onto(self, onto: Vec2) -> Option<Vec2> {
        let n2 = onto.dot(onto);
        (n2 > TINY * TINY).then(|| onto * (self.dot(onto) / n2))
    }

    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite()
    }
}

/// A vector in space.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Vec3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl Vec3 {
    pub const ZERO: Vec3 = Vec3::new(0.0, 0.0, 0.0);
    pub const X: Vec3 = Vec3::new(1.0, 0.0, 0.0);
    pub const Y: Vec3 = Vec3::new(0.0, 1.0, 0.0);
    pub const Z: Vec3 = Vec3::new(0.0, 0.0, 1.0);

    pub const fn new(x: f64, y: f64, z: f64) -> Self {
        Vec3 { x, y, z }
    }

    pub fn dot(self, other: Vec3) -> f64 {
        self.x * other.x + self.y * other.y + self.z * other.z
    }

    /// Right-handed: `X × Y = Z`.
    pub fn cross(self, other: Vec3) -> Vec3 {
        Vec3::new(
            self.y * other.z - self.z * other.y,
            self.z * other.x - self.x * other.z,
            self.x * other.y - self.y * other.x,
        )
    }

    pub fn norm(self) -> f64 {
        self.dot(self).sqrt()
    }

    /// A unit vector the same way, or `None` for one too short to have a
    /// way.
    pub fn normalized(self) -> Option<Vec3> {
        let n = self.norm();
        (n > TINY && n.is_finite()).then(|| self / n)
    }

    /// The angle between the two, in `[0, π]`; `None` when either has no
    /// direction.
    ///
    /// `atan2` of the cross product's length against the dot product rather
    /// than `acos` of the dot: `acos` near 1 throws away half the digits,
    /// and the angles a flight controller cares about most — the small
    /// error between two attitudes — are exactly the ones near 1.
    pub fn angle_to(self, other: Vec3) -> Option<f64> {
        (self.norm() > TINY && other.norm() > TINY)
            .then(|| self.cross(other).norm().atan2(self.dot(other)))
    }

    /// The part of `self` along `onto`.
    pub fn project_onto(self, onto: Vec3) -> Option<Vec3> {
        let n2 = onto.dot(onto);
        (n2 > TINY * TINY).then(|| onto * (self.dot(onto) / n2))
    }

    /// Some unit vector at right angles to this one — the axis a half turn
    /// takes when the two ends of it give none.
    pub fn any_perpendicular(self) -> Vec3 {
        // Cross with the axis this is least along, so the result is never
        // the short leftover of two nearly parallel vectors.
        let (ax, ay, az) = (self.x.abs(), self.y.abs(), self.z.abs());
        let other = if ax <= ay && ax <= az {
            Vec3::X
        } else if ay <= az {
            Vec3::Y
        } else {
            Vec3::Z
        };
        self.cross(other).normalized().unwrap_or(Vec3::X)
    }

    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.z.is_finite()
    }

    pub fn to_array(self) -> [f64; 3] {
        [self.x, self.y, self.z]
    }
}

macro_rules! arithmetic {
    ($t:ident { $($f:ident),+ }) => {
        impl Add for $t {
            type Output = $t;
            fn add(self, o: $t) -> $t {
                $t { $($f: self.$f + o.$f),+ }
            }
        }
        impl Sub for $t {
            type Output = $t;
            fn sub(self, o: $t) -> $t {
                $t { $($f: self.$f - o.$f),+ }
            }
        }
        impl Neg for $t {
            type Output = $t;
            fn neg(self) -> $t {
                $t { $($f: -self.$f),+ }
            }
        }
        impl Mul<f64> for $t {
            type Output = $t;
            fn mul(self, k: f64) -> $t {
                $t { $($f: self.$f * k),+ }
            }
        }
        impl Mul<$t> for f64 {
            type Output = $t;
            fn mul(self, v: $t) -> $t {
                v * self
            }
        }
        impl Div<f64> for $t {
            type Output = $t;
            fn div(self, k: f64) -> $t {
                $t { $($f: self.$f / k),+ }
            }
        }
    };
}

arithmetic!(Vec2 { x, y });
arithmetic!(Vec3 { x, y, z });

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::{FRAC_PI_2, PI};

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-12
    }

    #[test]
    fn the_cross_product_is_right_handed() {
        assert_eq!(Vec3::X.cross(Vec3::Y), Vec3::Z);
        assert_eq!(Vec3::Y.cross(Vec3::Z), Vec3::X);
        assert_eq!(Vec3::Z.cross(Vec3::X), Vec3::Y);
        let (a, b) = (Vec3::new(1.0, 2.0, 3.0), Vec3::new(-2.0, 0.5, 4.0));
        let c = a.cross(b);
        assert!(close(c.dot(a), 0.0) && close(c.dot(b), 0.0));
        // Its length is the parallelogram's area, |a||b|sin θ.
        let theta = a.angle_to(b).unwrap();
        assert!(close(c.norm(), a.norm() * b.norm() * theta.sin()));
    }

    #[test]
    fn a_plane_vector_turns_anticlockwise_and_measures_signed_angles() {
        let v = Vec2::new(1.0, 0.0).rotated(FRAC_PI_2);
        assert!(close(v.x, 0.0) && close(v.y, 1.0));
        let a = Vec2::new(1.0, 0.0);
        assert!(close(a.angle_to(Vec2::new(0.0, 1.0)).unwrap(), FRAC_PI_2));
        assert!(close(a.angle_to(Vec2::new(0.0, -1.0)).unwrap(), -FRAC_PI_2));
        assert!(close(a.angle_to(Vec2::new(-1.0, 0.0)).unwrap(), PI));
        assert!(close(a.cross(Vec2::new(0.0, 2.0)), 2.0));
    }

    /// Two directions a thousandth of a degree apart: `acos` of their dot
    /// product loses most of that; the cross-and-dot form does not.
    #[test]
    fn a_small_angle_between_vectors_keeps_its_digits() {
        let tiny = 1e-5_f64.to_radians();
        let a = Vec3::X;
        let b = Vec3::new(tiny.cos(), tiny.sin(), 0.0);
        assert!((a.angle_to(b).unwrap() - tiny).abs() < 1e-15);
    }

    #[test]
    fn a_vector_too_short_to_point_has_no_direction() {
        assert_eq!(Vec3::ZERO.normalized(), None);
        assert_eq!(Vec3::ZERO.angle_to(Vec3::X), None);
        assert_eq!(Vec2::ZERO.angle_to(Vec2::new(1.0, 0.0)), None);
        assert_eq!(Vec3::X.project_onto(Vec3::ZERO), None);
        let n = Vec3::new(3.0, 4.0, 12.0).normalized().unwrap();
        assert!(close(n.norm(), 1.0));
    }

    #[test]
    fn a_projection_is_the_part_along_and_leaves_the_rest_perpendicular() {
        let v = Vec3::new(2.0, 3.0, -1.0);
        let onto = Vec3::new(0.0, 2.0, 0.0);
        let along = v.project_onto(onto).unwrap();
        assert_eq!(along, Vec3::new(0.0, 3.0, 0.0));
        assert!(close((v - along).dot(onto), 0.0));
    }

    #[test]
    fn a_perpendicular_is_found_for_every_direction() {
        for v in [
            Vec3::X,
            Vec3::Y,
            Vec3::Z,
            Vec3::new(1.0, 1.0, 1.0),
            Vec3::new(-0.2, 5.0, 0.1),
        ] {
            let p = v.any_perpendicular();
            assert!(close(p.norm(), 1.0));
            assert!(p.dot(v).abs() < 1e-12, "{v:?} → {p:?}");
        }
    }
}
