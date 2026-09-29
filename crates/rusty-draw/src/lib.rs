//! Draw from code into rusty's 3-D view.
//!
//! A program says what to draw as lines of text, one shape to a line, each
//! beginning `[rusty:draw]`, on a stream rusty already reads: a test or an
//! example run from the editor's lens, a simulated firmware's console, a
//! board's serial port. rusty draws the scene in the dock's **Draw** tab —
//! the vectors as labelled arrows in a space you can turn, a square where
//! two of them meet at a right angle, and the angles between them written
//! out, so "is `a × b` perpendicular to both?" is answered by looking.
//!
//! ```
//! use rusty_draw::Scene;
//!
//! fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
//!     [
//!         a[1] * b[2] - a[2] * b[1],
//!         a[2] * b[0] - a[0] * b[2],
//!         a[0] * b[1] - a[1] * b[0],
//!     ]
//! }
//!
//! let a = [1.0, 0.4, 0.0];
//! let b = [0.3, 1.0, 0.5];
//! Scene::new("cross product")
//!     .vector("a", a)
//!     .vector("b", b)
//!     .vector("a × b", cross(a, b));
//! ```
//!
//! Run that from a test with **▶ Run Test**, or from a file in `examples/`
//! with **▶ Run**, and the three arrows appear. A scene is printed whole
//! when it is dropped — at the end of that statement here — so two tests
//! drawing at once cannot tear each other's lines, and a scene drawn again
//! under the same title replaces the one before, which is what a loop
//! drawing an attitude fifty times a second wants.
//!
//! # Without `std`
//!
//! [`SceneOn`] writes each line as it is drawn, to anything that implements
//! [`core::fmt::Write`]: esp-println's `Printer` on a board or in rusty's
//! simulator, a UART, a buffer.
//!
//! ```
//! use rusty_draw::SceneOn;
//!
//! let mut out = String::new();
//! SceneOn::new(&mut out, "attitude").frame("q", 1.0, 0.0, 0.0, 0.0);
//! assert!(out.starts_with("[rusty:draw] scene attitude\n"));
//! ```
//!
//! # The lines
//!
//! Numbers first, then a label that is the rest of the line:
//!
//! ```text
//! [rusty:draw] scene <title>
//! [rusty:draw] vector <x> <y> <z> <label>
//! [rusty:draw] vector_at <ox> <oy> <oz> <x> <y> <z> <label>
//! [rusty:draw] point <x> <y> <z> <label>
//! [rusty:draw] line <ax> <ay> <az> <bx> <by> <bz> <label>
//! [rusty:draw] span <ax> <ay> <az> <bx> <by> <bz> <label>
//! [rusty:draw] span_at <ox> <oy> <oz> <ax> <ay> <az> <bx> <by> <bz> <label>
//! [rusty:draw] frame <w> <x> <y> <z> <label>
//! [rusty:draw] end
//! ```
//!
//! Any program can print them itself; this crate spells them so nothing
//! has to be looked up. Space is right-handed with Z up. A `frame` is a
//! Hamilton quaternion with `w` first, and the order is in the name of every
//! argument: `(w, x, y, z)` and `(x, y, z, w)` are both somebody's
//! convention, and a quaternion read in the other one is a plausible, wrong
//! attitude.
#![no_std]

#[cfg(feature = "std")]
extern crate std;

use core::fmt;

/// Three coordinates, in whatever type a program keeps them.
///
/// Arrays and tuples of `f64`, `f32` and `i32` come implemented; a vector
/// type of your own takes one impl:
///
/// ```
/// struct Vector {
///     x: f32,
///     y: f32,
///     z: f32,
/// }
///
/// impl rusty_draw::Xyz for Vector {
///     fn xyz(&self) -> [f64; 3] {
///         [self.x as f64, self.y as f64, self.z as f64]
///     }
/// }
/// ```
pub trait Xyz {
    fn xyz(&self) -> [f64; 3];
}

impl Xyz for [f64; 3] {
    fn xyz(&self) -> [f64; 3] {
        *self
    }
}

impl Xyz for [f32; 3] {
    fn xyz(&self) -> [f64; 3] {
        self.map(f64::from)
    }
}

impl Xyz for [i32; 3] {
    fn xyz(&self) -> [f64; 3] {
        self.map(f64::from)
    }
}

impl Xyz for (f64, f64, f64) {
    fn xyz(&self) -> [f64; 3] {
        [self.0, self.1, self.2]
    }
}

impl Xyz for (f32, f32, f32) {
    fn xyz(&self) -> [f64; 3] {
        [self.0.into(), self.1.into(), self.2.into()]
    }
}

impl Xyz for (i32, i32, i32) {
    fn xyz(&self) -> [f64; 3] {
        [self.0.into(), self.1.into(), self.2.into()]
    }
}

impl<T: Xyz + ?Sized> Xyz for &T {
    fn xyz(&self) -> [f64; 3] {
        (**self).xyz()
    }
}

/// A label or a title as the end of one line: a space before it when there
/// is one, and a line break inside it turned into a space — a break would
/// end the line early and begin one rusty reads as something else.
struct Label<'a>(&'a str);

impl fmt::Display for Label<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = self.0.trim();
        if text.is_empty() {
            return Ok(());
        }
        f.write_str(" ")?;
        for (i, part) in text.split(['\r', '\n']).enumerate() {
            if i > 0 {
                f.write_str(" ")?;
            }
            f.write_str(part)?;
        }
        Ok(())
    }
}

/// A number as the shortest text that reads back as it: an `f32`'s when an
/// `f32` holds it exactly — which every number an `f32` program drew does,
/// so its `0.4` is written `0.4` and not as the `0.4000000059604645` it
/// widens to — and an `f64`'s otherwise. A `NaN` is written `NaN`, so a
/// broken vector arrives as one rather than as a zero.
struct Number(f64);

impl fmt::Display for Number {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let narrow = self.0 as f32;
        if f64::from(narrow) == self.0 {
            write!(f, "{narrow}")
        } else {
            write!(f, "{}", self.0)
        }
    }
}

/// Three numbers, space-separated.
struct Three([f64; 3]);

impl fmt::Display for Three {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let [x, y, z] = self.0.map(Number);
        write!(f, "{x} {y} {z}")
    }
}

fn emit(out: &mut impl fmt::Write, what: fmt::Arguments<'_>) -> fmt::Result {
    writeln!(out, "[rusty:draw] {what}")
}

/// The shapes, one method each, the same on [`Scene`] and [`SceneOn`].
macro_rules! shapes {
    () => {
        /// An arrow from the origin to `v`.
        pub fn vector(&mut self, label: &str, v: impl Xyz) -> &mut Self {
            self.put(format_args!("vector {}{}", Three(v.xyz()), Label(label)));
            self
        }

        /// The arrow `v`, drawn from `origin` rather than from the origin
        /// of space: its tip is at `origin + v`.
        pub fn vector_at(&mut self, label: &str, origin: impl Xyz, v: impl Xyz) -> &mut Self {
            self.put(format_args!(
                "vector_at {} {}{}",
                Three(origin.xyz()),
                Three(v.xyz()),
                Label(label)
            ));
            self
        }

        /// A point.
        pub fn point(&mut self, label: &str, p: impl Xyz) -> &mut Self {
            self.put(format_args!("point {}{}", Three(p.xyz()), Label(label)));
            self
        }

        /// A straight line between two points.
        pub fn line(&mut self, label: &str, a: impl Xyz, b: impl Xyz) -> &mut Self {
            self.put(format_args!(
                "line {} {}{}",
                Three(a.xyz()),
                Three(b.xyz()),
                Label(label)
            ));
            self
        }

        /// The parallelogram `a` and `b` span from the origin — the area
        /// `|a × b|` is, and the plane `a × b` stands up out of.
        pub fn span(&mut self, label: &str, a: impl Xyz, b: impl Xyz) -> &mut Self {
            self.put(format_args!(
                "span {} {}{}",
                Three(a.xyz()),
                Three(b.xyz()),
                Label(label)
            ));
            self
        }

        /// The parallelogram `a` and `b` span from `origin`.
        pub fn span_at(
            &mut self,
            label: &str,
            origin: impl Xyz,
            a: impl Xyz,
            b: impl Xyz,
        ) -> &mut Self {
            self.put(format_args!(
                "span_at {} {} {}{}",
                Three(origin.xyz()),
                Three(a.xyz()),
                Three(b.xyz()),
                Label(label)
            ));
            self
        }

        /// A body's three axes at the attitude `w + xi + yj + zk`: where a
        /// Hamilton quaternion turns X, Y and Z.
        pub fn frame(&mut self, label: &str, w: f64, x: f64, y: f64, z: f64) -> &mut Self {
            let [w, x, y, z] = [w, x, y, z].map(Number);
            self.put(format_args!("frame {w} {x} {y} {z}{}", Label(label)));
            self
        }
    };
}

/// A scene, printed to stdout whole when it is dropped or shown.
///
/// Whole, and in one `print!`: two tests drawing at once each take the
/// output in turn rather than interleaving line by line, and a test run
/// with its output captured keeps the scene with the rest of what it
/// printed.
#[cfg(feature = "std")]
pub struct Scene {
    text: std::string::String,
}

#[cfg(feature = "std")]
impl Scene {
    /// A scene called `title`. Drawing it again under the same title
    /// replaces it in rusty's view.
    pub fn new(title: &str) -> Scene {
        let mut scene = Scene {
            text: std::string::String::new(),
        };
        scene.put(format_args!("scene{}", Label(title)));
        scene
    }

    shapes!();

    /// Print it now, rather than when it goes out of scope.
    pub fn show(self) {}

    fn put(&mut self, what: fmt::Arguments<'_>) {
        // Writing to a `String` does not fail.
        let _ = emit(&mut self.text, what);
    }
}

#[cfg(feature = "std")]
impl Drop for Scene {
    fn drop(&mut self) {
        // Dropped by a panic, the scene goes out without its `end`: rusty
        // shows what was drawn as unfinished rather than as a scene that
        // said everything it meant to.
        if !std::thread::panicking() {
            self.put(format_args!("end"));
        }
        std::print!("{}", self.text);
    }
}

/// A scene written a line at a time to any [`fmt::Write`], for firmware
/// with no `std`: esp-println's `Printer`, a UART, a buffer.
///
/// Each line goes out as it is drawn, so on a stream shared with other
/// output the scene's lines can have other lines between them; rusty keeps
/// reading the scene until its `end`, which is written when this is dropped.
pub struct SceneOn<W: fmt::Write> {
    out: W,
}

impl<W: fmt::Write> SceneOn<W> {
    /// A scene called `title`, written to `out`.
    pub fn new(out: W, title: &str) -> SceneOn<W> {
        let mut scene = SceneOn { out };
        scene.put(format_args!("scene{}", Label(title)));
        scene
    }

    shapes!();

    /// End it now, rather than when it goes out of scope.
    pub fn end(self) {}

    fn put(&mut self, what: fmt::Arguments<'_>) {
        // A sink that refuses a line has nowhere to say so but here, and a
        // drawing is never worth stopping a firmware for.
        let _ = emit(&mut self.out, what);
    }
}

impl<W: fmt::Write> Drop for SceneOn<W> {
    fn drop(&mut self) {
        self.put(format_args!("end"));
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use std::string::{String, ToString};
    use std::vec::Vec;

    fn lines(text: &str) -> Vec<String> {
        text.lines().map(ToString::to_string).collect()
    }

    #[test]
    fn a_scene_is_its_title_its_shapes_and_an_end() {
        let mut out = String::new();
        SceneOn::new(&mut out, "cross product")
            .vector("a", [1.0, 0.0, 0.0])
            .vector("b", [0.0, 1.0_f32, 0.0])
            .vector("a × b", (0, 0, 1))
            .vector_at("v", [1.0, 1.0, 1.0], [0.5, 0.0, -0.25])
            .point("P", [2.0, -1.0, 0.5])
            .line("edge", [0.0; 3], [1.0, 2.0, 3.0])
            .span("a, b", [1.0, 0.0, 0.0], [0.0, 1.0, 0.0])
            .span_at("face", [1.0, 1.0, 1.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0])
            .frame("q", 1.0, 0.0, 0.0, 0.0);
        assert_eq!(
            lines(&out),
            [
                "[rusty:draw] scene cross product",
                "[rusty:draw] vector 1 0 0 a",
                "[rusty:draw] vector 0 1 0 b",
                "[rusty:draw] vector 0 0 1 a × b",
                "[rusty:draw] vector_at 1 1 1 0.5 0 -0.25 v",
                "[rusty:draw] point 2 -1 0.5 P",
                "[rusty:draw] line 0 0 0 1 2 3 edge",
                "[rusty:draw] span 1 0 0 0 1 0 a, b",
                "[rusty:draw] span_at 1 1 1 1 0 0 0 0 1 face",
                "[rusty:draw] frame 1 0 0 0 q",
                "[rusty:draw] end",
            ]
        );
    }

    /// A line break in a label would end the line early and start one that
    /// reads as something else; an empty label leaves no trailing space.
    #[test]
    fn a_label_stays_on_its_line() {
        let mut out = String::new();
        SceneOn::new(&mut out, "two\nlines")
            .vector("first\r\nsecond", [1.0, 2.0, 3.0])
            .point("  ", [0.0, 0.0, 0.0]);
        assert_eq!(
            lines(&out),
            [
                "[rusty:draw] scene two lines",
                "[rusty:draw] vector 1 2 3 first  second",
                "[rusty:draw] point 0 0 0",
                "[rusty:draw] end",
            ]
        );
    }

    /// A broken number arrives broken, where rusty can say so — not as a
    /// zero, which would draw a confident wrong arrow — and a number an
    /// `f32` held is written as the `f32` wrote it.
    #[test]
    fn numbers_read_back_as_themselves() {
        let mut out = String::new();
        SceneOn::new(&mut out, "")
            .vector("n", [f64::NAN, f64::INFINITY, 0.1])
            .vector("f", [0.4_f32, 1e-7, 3.0e9]);
        let numbers = |line: usize| -> Vec<f64> {
            lines(&out)[line]
                .split_whitespace()
                .skip(2)
                .take(3)
                .map(|n| n.parse().unwrap())
                .collect()
        };
        let broken = numbers(1);
        assert!(broken[0].is_nan());
        assert_eq!(broken[1], f64::INFINITY);
        assert_eq!(broken[2], 0.1, "an f64 is written as the f64 it is");
        assert_eq!(
            lines(&out)[2],
            "[rusty:draw] vector 0.4 0.0000001 3000000000 f"
        );
        let narrow: Vec<f32> = numbers(2).into_iter().map(|n| n as f32).collect();
        assert_eq!(
            narrow,
            [0.4_f32, 1e-7, 3.0e9],
            "and reads back as the same f32"
        );
        assert_eq!(lines(&out)[0], "[rusty:draw] scene");
    }

    /// The printing scene holds its lines until it is dropped, and ends
    /// them exactly once.
    #[test]
    fn a_printed_scene_is_held_until_it_goes() {
        let mut scene = Scene::new("held");
        scene.vector("a", [1.0, 0.0, 0.0]);
        assert_eq!(
            lines(&scene.text),
            ["[rusty:draw] scene held", "[rusty:draw] vector 1 0 0 a"]
        );
        scene.show();
    }

    #[test]
    fn a_vector_type_of_ones_own_draws() {
        struct Vector {
            x: f32,
            y: f32,
            z: f32,
        }
        impl Xyz for Vector {
            fn xyz(&self) -> [f64; 3] {
                [self.x.into(), self.y.into(), self.z.into()]
            }
        }
        let v = Vector {
            x: 0.5,
            y: -2.0,
            z: 4.0,
        };
        let mut out = String::new();
        SceneOn::new(&mut out, "own").vector("v", &v);
        assert_eq!(lines(&out)[1], "[rusty:draw] vector 0.5 -2 4 v");
    }
}
