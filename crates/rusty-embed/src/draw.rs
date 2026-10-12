//! What a program asked to be drawn: the `[rusty:draw]` lines the
//! `rusty-draw` crate writes, read into sketches, and what a sketch says
//! about itself — the angle between two arrows that share a tail, whether
//! three of them make a right-handed set, a number that is not one.
//!
//! The drawing is the program's; the reading is rusty's. A program that
//! computes `a × b` and draws the three arrows is asking "is this at right
//! angles to both?", and a picture answers that only roughly — a turned
//! view foreshortens every angle. So the angles are worked out here, from
//! the numbers the program printed, and the view marks a right angle only
//! where the arithmetic found one.
//!
//! Wasm-safe and unconditional, like `protocol`: the frontend reads the
//! stream as it passes.

use crate::spatial::{Quat, TINY, Vec3};

/// What begins every line of the drawing protocol.
pub const MARKER: &str = "[rusty:draw]";

/// One thing to draw, in world axes: right-handed, Z up.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Shape {
    /// The arrow `v`, drawn from `at` — the origin for a plain `vector`.
    /// The components are kept as the program wrote them rather than as a
    /// tip, so the numbers read back are its numbers and not `(at + v) - at`.
    Vector {
        at: Vec3,
        v: Vec3,
    },
    Point {
        at: Vec3,
    },
    /// A straight line between two points.
    Line {
        a: Vec3,
        b: Vec3,
    },
    /// The parallelogram `a` and `b` span from `at`.
    Span {
        at: Vec3,
        a: Vec3,
        b: Vec3,
    },
    /// A body's three axes at the attitude `q`, as written: a quaternion
    /// off the unit sphere is drawn as the turn it stands for and said to
    /// be off it, never quietly mended.
    Frame {
        q: Quat,
    },
}

/// A shape, what the program called it, and the colour it chose for it —
/// `None` leaves the colour to the view's palette.
#[derive(Debug, Clone, PartialEq)]
pub struct Mark {
    pub shape: Shape,
    pub label: String,
    pub color: Option<[u8; 3]>,
}

/// One line of the protocol.
#[derive(Debug, Clone, PartialEq)]
pub enum DrawLine {
    /// A scene begins, under this title.
    Scene(String),
    Mark(Mark),
    /// The scene is complete.
    End,
}

/// A line of the protocol, and the text before its marker.
///
/// The marker may come after something else on the line: a test harness
/// running one test at a time writes `test name ... ` and then whatever the
/// test prints, on the same line. That text is the caller's to keep.
///
/// A line that names no verb this reads, or carries the wrong count of
/// numbers for its verb, is `None` — it is not a drawing, and the caller
/// shows it as the text it is rather than drawing a guess at it.
pub fn split_draw(line: &str) -> Option<(&str, DrawLine)> {
    let at = line.find(MARKER)?;
    let drawn = read(&line[at + MARKER.len()..])?;
    Some((&line[..at], drawn))
}

/// [`split_draw`] for a line that starts with the marker.
pub fn parse_draw(line: &str) -> Option<DrawLine> {
    match split_draw(line)? {
        (before, drawn) if before.trim().is_empty() => Some(drawn),
        _ => None,
    }
}

fn read(rest: &str) -> Option<DrawLine> {
    let (verb, mut tail) = word(rest);
    // A chosen colour rides on the verb: `vector#e5484d`. One that is not
    // six hex digits makes the line no drawing, rather than a guess at it.
    let (verb, color) = match verb.split_once('#') {
        Some((verb, hex)) => (verb, Some(rgb(hex)?)),
        None => (verb, None),
    };
    if color.is_some() && matches!(verb, "scene" | "end") {
        return None;
    }
    let (count, make): (usize, fn(&[f64; 9]) -> Shape) = match verb {
        "scene" => return Some(DrawLine::Scene(tail.trim().to_string())),
        "end" => return tail.trim().is_empty().then_some(DrawLine::End),
        "vector" => (3, |n| Shape::Vector {
            at: Vec3::ZERO,
            v: xyz(n, 0),
        }),
        "vector_at" => (6, |n| Shape::Vector {
            at: xyz(n, 0),
            v: xyz(n, 3),
        }),
        "point" => (3, |n| Shape::Point { at: xyz(n, 0) }),
        "line" => (6, |n| Shape::Line {
            a: xyz(n, 0),
            b: xyz(n, 3),
        }),
        "span" => (6, |n| Shape::Span {
            at: Vec3::ZERO,
            a: xyz(n, 0),
            b: xyz(n, 3),
        }),
        "span_at" => (9, |n| Shape::Span {
            at: xyz(n, 0),
            a: xyz(n, 3),
            b: xyz(n, 6),
        }),
        "frame" => (4, |n| Shape::Frame {
            q: Quat::new(n[0], n[1], n[2], n[3]),
        }),
        _ => return None,
    };
    let mut numbers = [0.0; 9];
    for slot in numbers.iter_mut().take(count) {
        let (number, next) = word(tail);
        // `NaN` and `inf` read as themselves: a vector that came out of the
        // arithmetic broken is drawn as broken, not dropped.
        *slot = number.parse().ok()?;
        tail = next;
    }
    Some(DrawLine::Mark(Mark {
        shape: make(&numbers),
        label: tail.trim().to_string(),
        color,
    }))
}

/// `rrggbb` as three bytes.
fn rgb(hex: &str) -> Option<[u8; 3]> {
    if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let byte = |at: usize| u8::from_str_radix(&hex[at..at + 2], 16).ok();
    Some([byte(0)?, byte(2)?, byte(4)?])
}

fn xyz(n: &[f64; 9], from: usize) -> Vec3 {
    Vec3::new(n[from], n[from + 1], n[from + 2])
}

/// The first word, and what follows it.
fn word(text: &str) -> (&str, &str) {
    let text = text.trim_start();
    let end = text.find(char::is_whitespace).unwrap_or(text.len());
    (&text[..end], &text[end..])
}

/// A scene as it arrived.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Sketch {
    pub title: String,
    pub marks: Vec<Mark>,
    /// Ended by its own `end`. A sketch filed without one is what a program
    /// had drawn when it stopped — it panicked, was stopped, or began
    /// another scene — and is shown as that.
    pub finished: bool,
    /// Marks past [`MAX_MARKS`] were left out.
    pub truncated: bool,
}

/// Marks kept in one sketch. A firmware drawing a point per loop into one
/// scene that never ends would otherwise grow it for as long as it ran.
pub const MAX_MARKS: usize = 2_000;

/// Sketches kept. A title drawn again replaces its sketch, so this is how
/// many *different* scenes stay to be looked back at.
pub const MAX_SKETCHES: usize = 16;

impl Sketch {
    /// The furthest any drawn point reaches from the origin, a frame's axes
    /// counting as their unit length; zero for nothing drawable. What a view
    /// fits itself to.
    pub fn reach(&self) -> f64 {
        self.marks
            .iter()
            .flat_map(|mark| match mark.shape {
                Shape::Vector { at, v } => vec![at, at + v],
                Shape::Point { at } => vec![at],
                Shape::Line { a, b } => vec![a, b],
                Shape::Span { at, a, b } => vec![at, at + a, at + a + b, at + b],
                Shape::Frame { .. } => vec![Vec3::X],
            })
            .filter(|p| p.is_finite())
            .map(Vec3::norm)
            .fold(0.0, f64::max)
    }
}

/// The scene being received, between its `scene` and its `end`.
#[derive(Debug, Default)]
pub struct Sketchbook {
    pending: Option<Sketch>,
}

impl Sketchbook {
    /// One line read. What it answers is a sketch to file: the one its
    /// `end` finished, or an unfinished one a new `scene` broke off.
    ///
    /// Marks with no `scene` before them begin an untitled one, so a program
    /// printing bare `vector` lines draws them.
    pub fn read(&mut self, line: DrawLine) -> Option<Sketch> {
        match line {
            DrawLine::Scene(title) => {
                let broken = self.pending.replace(Sketch {
                    title,
                    ..Sketch::default()
                });
                broken.filter(|sketch| !sketch.marks.is_empty())
            }
            DrawLine::Mark(mark) => {
                let sketch = self.pending.get_or_insert_with(Sketch::default);
                if sketch.marks.len() < MAX_MARKS {
                    sketch.marks.push(mark);
                } else {
                    sketch.truncated = true;
                }
                None
            }
            DrawLine::End => self.pending.take().map(|mut sketch| {
                sketch.finished = true;
                sketch
            }),
        }
    }

    /// The stream ended: what was being drawn, unfinished — or nothing, if
    /// nothing had been drawn yet.
    pub fn close(&mut self) -> Option<Sketch> {
        self.pending
            .take()
            .filter(|sketch| !sketch.marks.is_empty())
    }
}

/// Put a sketch among the others, and answer where it went. In place of
/// one with the same title — a scene drawn again is the same scene, newer,
/// and a loop drawing an attitude fifty times a second is one scene — or
/// else at the end, the oldest going past [`MAX_SKETCHES`].
pub fn file(book: &mut Vec<Sketch>, sketch: Sketch) -> usize {
    if let Some(at) = book.iter().position(|s| s.title == sketch.title) {
        book[at] = sketch;
        return at;
    }
    book.push(sketch);
    if book.len() > MAX_SKETCHES {
        book.remove(0);
    }
    book.len() - 1
}

/// How near a right angle has to be to be one: the cosine of the angle,
/// against the lengths. A ten-thousandth is about a two-hundredth of a
/// degree — rounding in an `f32` cross product of ordinary vectors is a
/// thousand times smaller, and a cross product with a component in the
/// wrong place is thousands of times larger.
pub const SQUARE: f64 = 1e-4;

/// Two arrows from one tail, and the angle between them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Angle {
    /// Where the two are among the sketch's marks.
    pub a: usize,
    pub b: usize,
    /// In `[0, π]`.
    pub radians: f64,
    /// At right angles, to within [`SQUARE`].
    pub square: bool,
}

/// The arrows that can be measured: finite, with a length, and their
/// places among the marks.
fn arrows(marks: &[Mark]) -> Vec<(usize, Vec3, Vec3)> {
    marks
        .iter()
        .enumerate()
        .filter_map(|(i, mark)| match mark.shape {
            Shape::Vector { at, v } if at.is_finite() && v.is_finite() && v.norm() > TINY => {
                Some((i, at, v))
            }
            _ => None,
        })
        .collect()
}

/// Whether two tails are one point, to within the numbers' own rounding.
fn same_tail(p: Vec3, q: Vec3) -> bool {
    (p - q).norm() <= 1e-9 * (1.0 + p.norm().max(q.norm()))
}

/// Every pair of arrows drawn from one tail, in the order they were drawn.
/// Arrows from different points are not paired: the angle between them is
/// defined, but nothing on the page stands for it.
pub fn angles(marks: &[Mark]) -> Vec<Angle> {
    let arrows = arrows(marks);
    let mut out = Vec::new();
    for (k, &(a, at, v)) in arrows.iter().enumerate() {
        for &(b, other, w) in &arrows[k + 1..] {
            if !same_tail(at, other) {
                continue;
            }
            let Some(radians) = v.angle_to(w) else {
                continue;
            };
            let cosine = v.dot(w) / (v.norm() * w.norm());
            out.push(Angle {
                a,
                b,
                radians,
                square: cosine.abs() < SQUARE,
            });
        }
    }
    out
}

/// Which hand three arrows make, taken in the order they were drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hand {
    /// `a · (b × c) > 0`: X, Y, Z.
    Right,
    Left,
    /// In one plane: no hand at all.
    Flat,
}

/// Three arrows from one tail and the hand they make.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Triple {
    pub marks: [usize; 3],
    pub hand: Hand,
}

/// The hand a sketch's arrows make, when it has exactly three and they
/// share a tail — the question a cross product raises that a right angle
/// cannot settle: `b × a` is at right angles to both as well, and is the
/// wrong answer. `a`, `b` and `a × b`, drawn in that order, are
/// right-handed exactly when the product was taken the right way round.
pub fn handedness(marks: &[Mark]) -> Option<Triple> {
    let arrows = arrows(marks);
    let drawn = marks
        .iter()
        .filter(|mark| matches!(mark.shape, Shape::Vector { .. }))
        .count();
    let [(i, at, a), (j, at_b, b), (k, at_c, c)] = arrows[..] else {
        return None;
    };
    if drawn != 3 || !same_tail(at, at_b) || !same_tail(at, at_c) {
        return None;
    }
    let volume = a.dot(b.cross(c));
    let scale = a.norm() * b.norm() * c.norm();
    let hand = if volume.abs() <= 1e-9 * scale {
        Hand::Flat
    } else if volume > 0.0 {
        Hand::Right
    } else {
        Hand::Left
    };
    Some(Triple {
        marks: [i, j, k],
        hand,
    })
}

/// What is wrong with a mark's numbers, for the view to say beside it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Flaw {
    /// A `NaN` or an infinity among them: nothing can be drawn.
    NotFinite,
    /// A vector, or a quaternion, with no length: a vector is drawn as a
    /// point, a frame not at all.
    Zero,
    /// A quaternion off the unit sphere, by this length: drawn as the turn
    /// it stands for, and said.
    NotUnit(f64),
}

pub fn flaw(mark: &Mark) -> Option<Flaw> {
    match mark.shape {
        Shape::Vector { at, v } => {
            if !(at.is_finite() && v.is_finite()) {
                Some(Flaw::NotFinite)
            } else if v.norm() <= TINY {
                Some(Flaw::Zero)
            } else {
                None
            }
        }
        Shape::Point { at } => (!at.is_finite()).then_some(Flaw::NotFinite),
        Shape::Line { a, b } => (!(a.is_finite() && b.is_finite())).then_some(Flaw::NotFinite),
        Shape::Span { at, a, b } => {
            (!(at.is_finite() && a.is_finite() && b.is_finite())).then_some(Flaw::NotFinite)
        }
        Shape::Frame { q } => {
            let n = q.norm();
            if !n.is_finite() {
                Some(Flaw::NotFinite)
            } else if n <= TINY {
                Some(Flaw::Zero)
            } else if (n - 1.0).abs() > 1e-6 {
                Some(Flaw::NotUnit(n))
            } else {
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mark(line: &str) -> Mark {
        match parse_draw(line) {
            Some(DrawLine::Mark(mark)) => mark,
            other => panic!("{line}: {other:?}"),
        }
    }

    fn vector(v: [f64; 3], label: &str) -> Mark {
        Mark {
            shape: Shape::Vector {
                at: Vec3::ZERO,
                v: Vec3::new(v[0], v[1], v[2]),
            },
            label: label.to_string(),
            color: None,
        }
    }

    /// A colour on the verb is the mark's; one that is not six hex digits,
    /// or one on a scene's title, makes the line no drawing.
    #[test]
    fn a_colour_rides_on_the_verb() {
        assert_eq!(
            mark("[rusty:draw] vector#e5484d 1 0 0 a").color,
            Some([0xe5, 0x48, 0x4d])
        );
        assert_eq!(
            mark("[rusty:draw] frame#0080FF 1 0 0 0 q").color,
            Some([0, 0x80, 0xff])
        );
        assert_eq!(mark("[rusty:draw] vector 1 0 0 a").color, None);
        assert_eq!(parse_draw("[rusty:draw] vector#e548 1 0 0 a"), None);
        assert_eq!(parse_draw("[rusty:draw] vector#gggggg 1 0 0 a"), None);
        assert_eq!(parse_draw("[rusty:draw] scene#e5484d title"), None);
    }

    #[test]
    fn every_verb_reads_its_numbers_and_the_rest_is_the_label() {
        assert_eq!(
            parse_draw("[rusty:draw] scene cross product"),
            Some(DrawLine::Scene("cross product".into()))
        );
        assert_eq!(parse_draw("[rusty:draw] end"), Some(DrawLine::End));
        let a = mark("[rusty:draw] vector 1 0.5 -2 a × b");
        assert_eq!(
            a.shape,
            Shape::Vector {
                at: Vec3::ZERO,
                v: Vec3::new(1.0, 0.5, -2.0)
            }
        );
        assert_eq!(a.label, "a × b");
        assert_eq!(
            mark("[rusty:draw] vector_at 1 1 1 0 0 2 up").shape,
            Shape::Vector {
                at: Vec3::new(1.0, 1.0, 1.0),
                v: Vec3::new(0.0, 0.0, 2.0)
            }
        );
        assert_eq!(
            mark("[rusty:draw] point 3 2 1 P").shape,
            Shape::Point {
                at: Vec3::new(3.0, 2.0, 1.0)
            }
        );
        assert_eq!(
            mark("[rusty:draw] line 0 0 0 1 2 3").shape,
            Shape::Line {
                a: Vec3::ZERO,
                b: Vec3::new(1.0, 2.0, 3.0)
            }
        );
        assert_eq!(
            mark("[rusty:draw] span_at 1 0 0 0 1 0 0 0 1 face").shape,
            Shape::Span {
                at: Vec3::X,
                a: Vec3::Y,
                b: Vec3::Z
            }
        );
        assert_eq!(
            mark("[rusty:draw] frame 1 0 0 0 q").shape,
            Shape::Frame { q: Quat::IDENTITY }
        );
        // A label that starts with a digit is still a label: the numbers
        // are counted, not guessed at.
        assert_eq!(mark("[rusty:draw] point 0 0 0 2a").label, "2a");
    }

    /// Not a drawing is not drawn: the line stays text, where whoever wrote
    /// it can see what they wrote.
    #[test]
    fn a_line_that_is_not_quite_a_drawing_is_left_alone() {
        for line in [
            "[rusty:draw] vector 1 2",
            "[rusty:draw] vector 1 2 x y",
            "[rusty:draw] arrow 0 0 0 1 1 1",
            "[rusty:draw] end now",
            "[rusty:draw]",
            "[rusty:tel] x=1",
            "vector 1 2 3",
        ] {
            assert_eq!(parse_draw(line), None, "{line}");
        }
    }

    #[test]
    fn a_broken_number_arrives_broken() {
        let Shape::Vector { v, .. } = mark("[rusty:draw] vector NaN inf -inf n").shape else {
            panic!("a vector");
        };
        assert!(v.x.is_nan() && v.y == f64::INFINITY && v.z == f64::NEG_INFINITY);
    }

    /// A harness running one test at a time writes the test's name and then
    /// what it prints, on one line: the marker is found where it is, and
    /// what came before it goes back to the caller.
    #[test]
    fn the_marker_is_found_after_other_text_on_the_line() {
        let (before, drawn) =
            split_draw("test tests::cross ... [rusty:draw] scene cross product").unwrap();
        assert_eq!(before, "test tests::cross ... ");
        assert_eq!(drawn, DrawLine::Scene("cross product".into()));
        assert_eq!(
            parse_draw("test x ... [rusty:draw] end"),
            None,
            "not at the start"
        );
        assert_eq!(parse_draw("  [rusty:draw] end  "), Some(DrawLine::End));
    }

    fn feed(book: &mut Sketchbook, lines: &[&str]) -> Vec<Sketch> {
        lines
            .iter()
            .filter_map(|line| book.read(parse_draw(line).unwrap()))
            .collect()
    }

    #[test]
    fn a_scene_is_filed_when_it_ends() {
        let mut book = Sketchbook::default();
        let filed = feed(
            &mut book,
            &[
                "[rusty:draw] scene one",
                "[rusty:draw] vector 1 0 0 a",
                "[rusty:draw] vector 0 1 0 b",
            ],
        );
        assert!(filed.is_empty(), "nothing until the end");
        let filed = feed(&mut book, &["[rusty:draw] end"]);
        assert_eq!(filed.len(), 1);
        assert_eq!(filed[0].title, "one");
        assert_eq!(filed[0].marks.len(), 2);
        assert!(filed[0].finished);
        assert_eq!(book.close(), None, "nothing left over");
    }

    /// A program that stopped mid-scene, or began another, left something
    /// worth seeing — shown as unfinished rather than thrown away.
    #[test]
    fn an_unfinished_scene_is_filed_as_unfinished() {
        let mut book = Sketchbook::default();
        let broken = feed(
            &mut book,
            &[
                "[rusty:draw] scene first",
                "[rusty:draw] point 1 1 1",
                "[rusty:draw] scene second",
                "[rusty:draw] point 2 2 2",
            ],
        );
        assert_eq!(broken.len(), 1);
        assert_eq!(broken[0].title, "first");
        assert!(!broken[0].finished);
        let closed = book.close().unwrap();
        assert_eq!(closed.title, "second");
        assert!(!closed.finished);
        // A scene that drew nothing before it was broken off is nothing.
        let mut book = Sketchbook::default();
        assert!(feed(&mut book, &["[rusty:draw] scene a", "[rusty:draw] scene b"]).is_empty());
        assert_eq!(book.close(), None);
    }

    #[test]
    fn marks_without_a_scene_begin_an_untitled_one() {
        let mut book = Sketchbook::default();
        let filed = feed(
            &mut book,
            &["[rusty:draw] vector 1 2 3 v", "[rusty:draw] end"],
        );
        assert_eq!(filed[0].title, "");
        assert_eq!(filed[0].marks.len(), 1);
    }

    #[test]
    fn a_scene_that_never_ends_stops_growing() {
        let mut book = Sketchbook::default();
        book.read(DrawLine::Scene("trail".into()));
        for _ in 0..MAX_MARKS + 5 {
            book.read(DrawLine::Mark(vector([1.0, 0.0, 0.0], "")));
        }
        let sketch = book.close().unwrap();
        assert_eq!(sketch.marks.len(), MAX_MARKS);
        assert!(sketch.truncated);
    }

    /// A title drawn again is the same scene, newer, in the same place; a
    /// new title goes on the end, and the oldest goes past the cap.
    #[test]
    fn a_scene_drawn_again_replaces_itself() {
        let sketch = |title: &str, n: usize| Sketch {
            title: title.into(),
            marks: vec![vector([1.0, 0.0, 0.0], ""); n],
            finished: true,
            truncated: false,
        };
        let mut book = Vec::new();
        assert_eq!(file(&mut book, sketch("a", 1)), 0);
        assert_eq!(file(&mut book, sketch("b", 1)), 1);
        assert_eq!(file(&mut book, sketch("a", 3)), 0);
        assert_eq!(book.len(), 2);
        assert_eq!(book[0].marks.len(), 3);
        for i in 0..MAX_SKETCHES {
            file(&mut book, sketch(&i.to_string(), 1));
        }
        assert_eq!(book.len(), MAX_SKETCHES);
        assert!(book.iter().all(|s| s.title != "a" && s.title != "b"));
    }

    fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
        Vec3::new(a[0], a[1], a[2])
            .cross(Vec3::new(b[0], b[1], b[2]))
            .to_array()
    }

    /// The report: two vectors and their cross product. The product is at
    /// right angles to both, the two are not to each other, and the view
    /// may only mark what the numbers say.
    #[test]
    fn a_cross_product_is_square_to_both_and_nothing_else_is() {
        let (a, b) = ([1.0, 0.4, 0.0], [0.3, 1.0, 0.5]);
        let marks = [vector(a, "a"), vector(b, "b"), vector(cross(a, b), "a × b")];
        let found = angles(&marks);
        assert_eq!(found.len(), 3);
        let pair = |i, j| found.iter().find(|x| (x.a, x.b) == (i, j)).unwrap();
        assert!(!pair(0, 1).square);
        assert!(pair(0, 2).square && pair(1, 2).square);
        assert!((pair(0, 2).radians - std::f64::consts::FRAC_PI_2).abs() < 1e-12);
        // A component in the wrong place is not a right angle.
        let [x, y, z] = cross(a, b);
        let wrong = [vector(a, "a"), vector(b, "b"), vector([y, x, z], "a × b")];
        assert!(angles(&wrong).iter().all(|x| !x.square));
        // And an `f32` product is still square: rounding is not a mistake.
        let a32 = [1.0_f32, 0.4, 0.0];
        let b32 = [0.3_f32, 1.0, 0.5];
        let c32 = [
            a32[1] * b32[2] - a32[2] * b32[1],
            a32[2] * b32[0] - a32[0] * b32[2],
            a32[0] * b32[1] - a32[1] * b32[0],
        ];
        let wide = |v: [f32; 3]| v.map(f64::from);
        let single = [
            vector(wide(a32), "a"),
            vector(wide(b32), "b"),
            vector(wide(c32), "c"),
        ];
        let found = angles(&single);
        assert!(found.iter().filter(|x| x.square).count() == 2);
    }

    #[test]
    fn only_arrows_from_one_tail_with_a_length_are_measured() {
        let apart = Mark {
            shape: Shape::Vector {
                at: Vec3::new(5.0, 0.0, 0.0),
                v: Vec3::Y,
            },
            label: String::new(),
            color: None,
        };
        let marks = [
            vector([1.0, 0.0, 0.0], "a"),
            apart,
            vector([0.0, 0.0, 0.0], "zero"),
            vector([f64::NAN, 0.0, 0.0], "nan"),
            vector([0.0, 0.0, 3.0], "z"),
        ];
        let found = angles(&marks);
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].a, found[0].b), (0, 4));
        assert!(found[0].square);
    }

    /// `b × a` is at right angles to both as well — the one wrong answer a
    /// right angle cannot show. The hand shows it.
    #[test]
    fn the_hand_tells_a_times_b_from_b_times_a() {
        let (a, b) = ([1.0, 0.4, 0.0], [0.3, 1.0, 0.5]);
        let right = [vector(a, "a"), vector(b, "b"), vector(cross(a, b), "a × b")];
        assert_eq!(handedness(&right).unwrap().hand, Hand::Right);
        let left = [vector(a, "a"), vector(b, "b"), vector(cross(b, a), "b × a")];
        assert_eq!(handedness(&left).unwrap().hand, Hand::Left);
        let flat = [
            vector([1.0, 0.0, 0.0], ""),
            vector([0.0, 1.0, 0.0], ""),
            vector([1.0, 1.0, 0.0], ""),
        ];
        assert_eq!(handedness(&flat).unwrap().hand, Hand::Flat);
        let four = [
            vector(a, ""),
            vector(b, ""),
            vector(cross(a, b), ""),
            vector([1.0, 1.0, 1.0], ""),
        ];
        assert_eq!(handedness(&four), None, "which three is not a question");
    }

    #[test]
    fn a_mark_with_numbers_that_cannot_be_drawn_says_so() {
        assert_eq!(
            flaw(&vector([f64::NAN, 0.0, 0.0], "")),
            Some(Flaw::NotFinite)
        );
        assert_eq!(flaw(&vector([0.0, 0.0, 0.0], "")), Some(Flaw::Zero));
        assert_eq!(flaw(&vector([0.0, 0.0, 1.0], "")), None);
        let frame = |q: Quat| Mark {
            shape: Shape::Frame { q },
            label: String::new(),
            color: None,
        };
        assert_eq!(flaw(&frame(Quat::IDENTITY)), None);
        assert_eq!(
            flaw(&frame(Quat::new(2.0, 0.0, 0.0, 0.0))),
            Some(Flaw::NotUnit(2.0))
        );
        assert_eq!(
            flaw(&frame(Quat::new(0.0, 0.0, 0.0, 0.0))),
            Some(Flaw::Zero)
        );
    }

    #[test]
    fn a_sketch_reaches_as_far_as_its_furthest_point() {
        let sketch = Sketch {
            marks: vec![
                vector([3.0, 4.0, 0.0], ""),
                vector([f64::INFINITY, 0.0, 0.0], ""),
                Mark {
                    shape: Shape::Frame { q: Quat::IDENTITY },
                    label: String::new(),
                    color: None,
                },
            ],
            ..Sketch::default()
        };
        assert!((sketch.reach() - 5.0).abs() < 1e-12);
        assert_eq!(Sketch::default().reach(), 0.0);
    }

    /// The crate that writes the lines and this reader of them are two
    /// spellings of one format: a scene written by the one reads back, mark
    /// for mark, through the other.
    #[test]
    fn what_the_crate_writes_this_reads() {
        let mut out = String::new();
        rusty_draw::SceneOn::new(&mut out, "round trip")
            .vector("a", [1.0, 0.4, 0.0])
            .vector_at("v", [1.0, 1.0, 1.0], [0.5_f32, 0.0, -0.25])
            .point("P", (2, -1, 0))
            .line("edge", [0.0; 3], [1.0, 2.0, 3.0])
            .span("a, b", [1.0, 0.0, 0.0], [0.0, 1.0, 0.0])
            .span_at("face", [1.0; 3], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0])
            .frame("q", 0.5, 0.5, 0.5, 0.5);
        let mut book = Sketchbook::default();
        let filed: Vec<Sketch> = out
            .lines()
            .filter_map(|line| book.read(parse_draw(line).expect(line)))
            .collect();
        let [sketch] = &filed[..] else {
            panic!("one scene: {filed:?}");
        };
        assert_eq!(sketch.title, "round trip");
        assert!(sketch.finished);
        let shapes: Vec<Shape> = sketch.marks.iter().map(|m| m.shape).collect();
        assert_eq!(
            shapes,
            [
                Shape::Vector {
                    at: Vec3::ZERO,
                    v: Vec3::new(1.0, 0.4, 0.0)
                },
                Shape::Vector {
                    at: Vec3::new(1.0, 1.0, 1.0),
                    v: Vec3::new(0.5, 0.0, -0.25)
                },
                Shape::Point {
                    at: Vec3::new(2.0, -1.0, 0.0)
                },
                Shape::Line {
                    a: Vec3::ZERO,
                    b: Vec3::new(1.0, 2.0, 3.0)
                },
                Shape::Span {
                    at: Vec3::ZERO,
                    a: Vec3::X,
                    b: Vec3::Y
                },
                Shape::Span {
                    at: Vec3::new(1.0, 1.0, 1.0),
                    a: Vec3::X,
                    b: Vec3::Z
                },
                Shape::Frame {
                    q: Quat::new(0.5, 0.5, 0.5, 0.5)
                },
            ]
        );
        let labels: Vec<&str> = sketch.marks.iter().map(|m| m.label.as_str()).collect();
        assert_eq!(labels, ["a", "v", "P", "edge", "a, b", "face", "q"]);
    }
}
