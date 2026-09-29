//! A vector type of the kind a flight library keeps, drawn with
//! `rusty-draw`. Open `examples/cross.rs` in rusty and press ▶ Run above
//! `main`, or ▶ Run Test above the test below, and the Draw tab shows the
//! arrows — `a × b` standing at right angles to both, and saying so in
//! degrees beside them.

/// A vector in space, in `f32` as firmware keeps one.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vector {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Vector {
    pub const fn new(x: f32, y: f32, z: f32) -> Vector {
        Vector { x, y, z }
    }

    pub fn dot(self, other: Vector) -> f32 {
        self.x * other.x + self.y * other.y + self.z * other.z
    }

    /// Right-handed: `X × Y = Z`.
    pub fn cross(self, other: Vector) -> Vector {
        Vector::new(
            self.y * other.z - self.z * other.y,
            self.z * other.x - self.x * other.z,
            self.x * other.y - self.y * other.x,
        )
    }

    pub fn norm(self) -> f32 {
        self.dot(self).sqrt()
    }

    /// The three numbers, as `rusty-draw` takes them.
    pub fn xyz(self) -> [f32; 3] {
        [self.x, self.y, self.z]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_draw::Scene;

    /// `a × b` is at right angles to both and makes a right-handed set with
    /// them — asserted, and drawn, so a failure can be looked at.
    #[test]
    fn the_cross_product_is_square_to_both() {
        let a = Vector::new(1.0, 0.4, 0.0);
        let b = Vector::new(0.3, 1.0, 0.5);
        let c = a.cross(b);
        Scene::new("cross product, from a test")
            .vector("a", a.xyz())
            .vector("b", b.xyz())
            .vector("a × b", c.xyz());
        assert!(c.dot(a).abs() < 1e-6);
        assert!(c.dot(b).abs() < 1e-6);
        assert!(a.dot(b.cross(c)) > 0.0, "a, b and a × b are right-handed");
    }
}
