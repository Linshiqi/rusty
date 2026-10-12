//! What the sheet's names are called on screen: every step, remark,
//! refusal, note and verdict the arithmetic reports by name, said in the
//! reader's language. The arithmetic (`rusty_embed::spatial`) says what
//! happened; this says it in words.

use rusty_embed::spatial::check::Relation;
use rusty_embed::spatial::sheet::{Kind, Note, Problem, Remark, What};
use rusty_i18n::t;

pub fn what(w: What) -> String {
    match w {
        What::Add => t!("math.what.add"),
        What::Sub => t!("math.what.sub"),
        What::Scale => t!("math.what.scale"),
        What::Negate => t!("math.what.negate"),
        What::Dot => t!("math.what.dot"),
        What::Cross => t!("math.what.cross"),
        What::Norm => t!("math.what.norm"),
        What::Normalize => t!("math.what.normalize"),
        What::Project => t!("math.what.project"),
        What::AngleBetween => t!("math.what.angle-between"),
        What::Turn2 => t!("math.what.turn2"),
        What::Quaternion => t!("math.what.quaternion"),
        What::AxisAngle => t!("math.what.axis-angle"),
        What::EulerYaw => t!("math.what.euler-yaw"),
        What::EulerPitch => t!("math.what.euler-pitch"),
        What::EulerRoll => t!("math.what.euler-roll"),
        What::ToEuler => t!("math.what.to-euler"),
        What::ToEulerPole => t!("math.what.to-euler-pole"),
        What::Matrix => t!("math.what.matrix"),
        What::FromMatrix => t!("math.what.from-matrix"),
        What::Compose => t!("math.what.compose"),
        What::Conjugate => t!("math.what.conjugate"),
        What::Inverse => t!("math.what.inverse"),
        What::ToWorld => t!("math.what.to-world"),
        What::ToBody => t!("math.what.to-body"),
        What::Rodrigues => t!("math.what.rodrigues"),
        What::Slerp => t!("math.what.slerp"),
        What::FromTo => t!("math.what.from-to"),
        What::Integrate => t!("math.what.integrate"),
        What::IntegrateLinear => t!("math.what.integrate-linear"),
        What::Delta => t!("math.what.delta"),
        What::AttitudeError => t!("math.what.attitude-error"),
        What::ToRotvec => t!("math.what.to-rotvec"),
        What::FromRotvec => t!("math.what.from-rotvec"),
        What::AtRest => t!("math.what.at-rest"),
        What::Gravity => t!("math.what.gravity"),
        What::Tilt => t!("math.what.tilt"),
        What::MatVec => t!("math.what.mat-vec"),
        What::MatMat => t!("math.what.mat-mat"),
        What::Transpose => t!("math.what.transpose"),
        What::Det => t!("math.what.det"),
        What::Check => t!("math.what.check"),
    }
}

pub fn remark(r: Remark) -> String {
    match r {
        Remark::Area => t!("math.remark.area"),
        Remark::ShortWay => t!("math.remark.short-way"),
        Remark::NoTurn => t!("math.remark.no-turn"),
        Remark::Flipped => t!("math.remark.flipped"),
        Remark::Columns => t!("math.remark.columns"),
        Remark::BodyRate => t!("math.remark.body-rate"),
        Remark::OffSphere => t!("math.remark.off-sphere"),
        Remark::OwnAxes => t!("math.remark.own-axes"),
        Remark::NoHeading => t!("math.remark.no-heading"),
        Remark::TransposeInverse => t!("math.remark.transpose-inverse"),
        Remark::DetSign => t!("math.remark.det-sign"),
        Remark::Pole => t!("math.remark.pole"),
        Remark::Antiparallel => t!("math.remark.antiparallel"),
        Remark::BodyAxes => t!("math.remark.body-axes"),
        Remark::SameAttitude => t!("math.remark.same-attitude"),
    }
}

pub fn kind(k: Kind) -> String {
    match k {
        Kind::Number => t!("math.kind.number"),
        Kind::Angle => t!("math.kind.angle"),
        Kind::Vec2 => t!("math.kind.vec2"),
        Kind::Vec3 => t!("math.kind.vec3"),
        Kind::Quat => t!("math.kind.quat"),
        Kind::Euler => t!("math.kind.euler"),
        Kind::Mat3 => t!("math.kind.mat3"),
        Kind::Verdict => t!("math.kind.verdict"),
        Kind::Text => t!("math.kind.text"),
    }
}

fn kinds(list: &[Kind]) -> String {
    if list.is_empty() {
        return t!("math.kind.nothing");
    }
    list.iter().map(|k| kind(*k)).collect::<Vec<_>>().join(", ")
}

pub fn problem(p: &Problem) -> String {
    match p {
        Problem::Unexpected { found, .. } => t!("math.problem.unexpected", found = found.clone()),
        Problem::Unclosed { .. } => t!("math.problem.unclosed"),
        Problem::Incomplete => t!("math.problem.incomplete"),
        Problem::BadNumber { text } => t!("math.problem.bad-number", text = text.clone()),
        Problem::Unknown { name } => t!("math.problem.unknown", name = name.clone()),
        Problem::Later { name } => t!("math.problem.later", name = name.clone()),
        Problem::Twice { name } => t!("math.problem.twice", name = name.clone()),
        Problem::Reserved { name } => t!("math.problem.reserved", name = name.clone()),
        Problem::Broken { name } => t!("math.problem.broken", name = name.clone()),
        Problem::NotCallable { name } => t!("math.problem.not-callable", name = name.clone()),
        Problem::Operands { op, left, right } => {
            // A vector times a vector is the one people reach for most, and
            // what they meant is one of two functions.
            if *op == '*'
                && matches!(left, Kind::Vec2 | Kind::Vec3)
                && matches!(right, Kind::Vec2 | Kind::Vec3)
            {
                return t!("math.problem.vector-product");
            }
            t!(
                "math.problem.operands",
                op = op.to_string(),
                left = kind(*left),
                right = kind(*right)
            )
        }
        Problem::Arguments { function, got } => t!(
            "math.problem.arguments",
            function = function.clone(),
            got = kinds(got)
        ),
        Problem::NoField { kind: k, field } => t!(
            "math.problem.no-field",
            kind = kind(*k),
            field = field.clone()
        ),
        Problem::FourTuple => t!("math.problem.four-tuple"),
        Problem::Tuple { len } => t!("math.problem.tuple", len = len.to_string()),
        Problem::NoDirection => t!("math.problem.no-direction"),
        Problem::DivideByZero => t!("math.problem.divide-by-zero"),
        Problem::NotFinite => t!("math.problem.not-finite"),
        Problem::NoChannel { name } => t!("math.problem.no-channel", name = name.clone()),
        Problem::NoPlant => t!("math.problem.no-plant"),
        Problem::StrayText => t!("math.problem.stray-text"),
    }
}

pub fn note(n: &Note) -> String {
    match n {
        Note::NotUnit { norm } => t!("math.note.not-unit", norm = format!("{norm:.6}")),
        Note::BareAngle { value } => t!(
            "math.note.bare-angle",
            value = format!("{value}"),
            degrees = format!("{:.1}", value.to_degrees())
        ),
        Note::Pole => t!("math.note.pole"),
    }
}

pub fn relation(r: Relation) -> String {
    match r {
        Relation::Same => t!("math.relation.same"),
        Relation::OtherSign => t!("math.relation.other-sign"),
        Relation::Inverse => t!("math.relation.inverse"),
        Relation::WLast => t!("math.relation.w-last"),
        Relation::OrderXyz => t!("math.relation.order-xyz"),
        Relation::OtherFrame => t!("math.relation.other-frame"),
        Relation::Opposite => t!("math.relation.opposite"),
        Relation::Degrees => t!("math.relation.degrees"),
        Relation::Radians => t!("math.relation.radians"),
        Relation::Unexplained => t!("math.relation.unexplained"),
    }
}

pub fn example(id: &str) -> String {
    match id {
        "euler" => t!("math.example.euler"),
        "frames" => t!("math.example.frames"),
        "gravity" => t!("math.example.gravity"),
        "gyro" => t!("math.example.gyro"),
        "compose" => t!("math.example.compose"),
        "error" => t!("math.example.error"),
        "slerp" => t!("math.example.slerp"),
        "check" => t!("math.example.check"),
        "live" => t!("math.example.live"),
        "rigid" => t!("math.example.rigid"),
        "plane" => t!("math.example.plane"),
        other => other.to_string(),
    }
}
