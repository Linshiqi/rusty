# rusty-draw

Draw vectors, points and frames from Rust code into
[rusty](https://github.com/Linshiqi/rusty)'s 3-D view.

```rust
use rusty_draw::Scene;

Scene::new("cross product")
    .vector("a", a)
    .vector("b", b)
    .vector("a × b", a.cross(b));
```

Run that from a test (**▶ Run Test**) or from `main` in an example
(**▶ Run**) in rusty, and the dock's **Draw** tab shows the arrows in a space
you can turn, a right angle marked wherever two of them meet at one, and the
angles between them written out — so "is `a × b` perpendicular to both, and
the right way round?" is answered by looking.

Each shape is one line of text — `[rusty:draw] vector 1 0.4 0 a` — so a scene
printed by a simulated firmware or a board on the serial port is drawn the
same way. A `Scene` is printed whole when it is dropped, so tests drawing at
once cannot tear each other's lines; drawn again under the same title, it
replaces itself, which is what a loop drawing an attitude every frame wants.

## Adding it

```toml
[dev-dependencies]
rusty-draw = { git = "https://github.com/Linshiqi/rusty" }
```

No dependencies of its own. Without `std` (`default-features = false`),
`SceneOn` writes each line to any `core::fmt::Write` — esp-println's
`Printer`, a UART, a buffer.

## Shapes

| Method | Draws |
|---|---|
| `vector(label, v)` | an arrow from the origin |
| `vector_at(label, origin, v)` | the arrow `v` from `origin` |
| `point(label, p)` | a point |
| `line(label, a, b)` | a straight line between two points |
| `span(label, a, b)` | the parallelogram `a` and `b` span |
| `span_at(label, origin, a, b)` | the same, from `origin` |
| `frame(label, w, x, y, z)` | a body's axes at a quaternion, `w` first |

Anything with three coordinates draws: arrays and tuples of `f64`, `f32` and
`i32`, and a type of your own through a one-line `impl Xyz`. Space is
right-handed with Z up.
