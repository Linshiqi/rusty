//! Two vectors and their cross product, drawn. ▶ Run above `main` in rusty
//! runs `cargo run --example cross`, and the Draw tab shows the scene: the
//! three arrows, the parallelogram `a` and `b` span, a right angle marked
//! where `a × b` meets each of them, and the angles written out.

use draw_vectors::Vector;
use rusty_draw::Scene;

fn main() {
    let a = Vector::new(1.0, 0.4, 0.0);
    let b = Vector::new(0.3, 1.0, 0.5);
    let c = a.cross(b);
    println!("a × b = {c:?}, |a × b| = {}", c.norm());
    Scene::new("cross product")
        .vector("a", a.xyz())
        .vector("b", b.xyz())
        .vector("a × b", c.xyz())
        .span("a, b", a.xyz(), b.xyz());
}
