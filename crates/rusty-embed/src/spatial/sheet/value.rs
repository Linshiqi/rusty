//! What a row holds, what can go wrong with one, and what is worth saying
//! about one that went right.

use crate::spatial::check::Verdict;
use crate::spatial::{Euler, Mat3, Quat, Vec2, Vec3};

/// The value of a row, or of any expression in one.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Number(f64),
    /// In radians. Kept apart from a plain number so it can be shown in
    /// degrees, and so a number where an angle was meant can be noticed.
    Angle(f64),
    Vec2(Vec2),
    Vec3(Vec3),
    Quat(Quat),
    Euler(Euler),
    Mat3(Mat3),
    Verdict(Verdict),
}

/// A value's kind, for saying what an operation was handed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Number,
    Angle,
    Vec2,
    Vec3,
    Quat,
    Euler,
    Mat3,
    Verdict,
    /// A quoted name, which only `tel` takes.
    Text,
}

impl Value {
    pub fn kind(&self) -> Kind {
        match self {
            Value::Number(_) => Kind::Number,
            Value::Angle(_) => Kind::Angle,
            Value::Vec2(_) => Kind::Vec2,
            Value::Vec3(_) => Kind::Vec3,
            Value::Quat(_) => Kind::Quat,
            Value::Euler(_) => Kind::Euler,
            Value::Mat3(_) => Kind::Mat3,
            Value::Verdict(_) => Kind::Verdict,
        }
    }

    /// A scalar, angle or not, in radians when it is one.
    pub fn scalar(&self) -> Option<f64> {
        match self {
            Value::Number(v) | Value::Angle(v) => Some(*v),
            _ => None,
        }
    }

    pub fn is_finite(&self) -> bool {
        match self {
            Value::Number(v) | Value::Angle(v) => v.is_finite(),
            Value::Vec2(v) => v.is_finite(),
            Value::Vec3(v) => v.is_finite(),
            Value::Quat(q) => q.is_finite(),
            Value::Euler(e) => e.roll.is_finite() && e.pitch.is_finite() && e.yaw.is_finite(),
            Value::Mat3(m) => m.is_finite(),
            Value::Verdict(v) => v.error.is_finite(),
        }
    }
}

/// Why a row has no value. Each names what the writer can change.
#[derive(Debug, Clone, PartialEq)]
pub enum Problem {
    /// A character or token that has no place there.
    Unexpected {
        found: String,
        at: usize,
    },
    /// A bracket or a quote opened and never closed.
    Unclosed {
        at: usize,
    },
    /// The row ended where more was needed.
    Incomplete,
    BadNumber {
        text: String,
    },
    /// Not a row above, a constant or a function.
    Unknown {
        name: String,
    },
    /// A row below this one: the sheet is read top to bottom.
    Later {
        name: String,
    },
    /// The name a row above already has.
    Twice {
        name: String,
    },
    /// A row named after a function or a constant.
    Reserved {
        name: String,
    },
    /// A row above this one that has no value itself.
    Broken {
        name: String,
    },
    /// A function used as a value, or a value called like a function.
    NotCallable {
        name: String,
    },
    /// An operator between two kinds it has no meaning for.
    Operands {
        op: char,
        left: Kind,
        right: Kind,
    },
    /// A function handed kinds none of its forms take.
    Arguments {
        function: String,
        got: Vec<Kind>,
    },
    NoField {
        kind: Kind,
        field: String,
    },
    /// Four numbers in brackets could be `(w, x, y, z)` or `(x, y, z, w)`;
    /// `quat(w, x, y, z)` says which.
    FourTuple,
    /// Brackets around more than four, or around something that is not a
    /// number.
    Tuple {
        len: usize,
    },
    /// A direction asked of something with none: normalising a zero vector,
    /// an axis of no length, a turn from nowhere.
    NoDirection,
    DivideByZero,
    /// A result that is not a number.
    NotFinite,
    /// `tel` named a channel the firmware has not printed.
    NoChannel {
        name: String,
    },
    /// `truth` with no plant running.
    NoPlant,
    /// A quoted name anywhere but inside `tel("…")`.
    StrayText,
}

/// Something true about a row that has a value, worth saying beside it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Note {
    /// A quaternion used as a rotation, off the unit sphere: it scales what
    /// it turns by `|q|²`.
    NotUnit { norm: f64 },
    /// A plain number more than a turn, where an angle was taken: radians
    /// are assumed, and a number that size was probably meant in degrees.
    BareAngle { value: f64 },
    /// Euler angles at a pole, where roll and yaw turn about one axis.
    Pole,
}
