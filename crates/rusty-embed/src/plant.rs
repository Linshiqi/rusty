//! A rigid body that answers the motors, so a rate loop can be flown at a desk.
//!
//! `[rusty:pwm]` carries what the firmware commanded and `Igyro=…` carries
//! what the gyro reads. Between them there was nothing, so injecting a rate
//! proved only that the controller *responded* — the injected rate never
//! changed because the motors spun. That is enough to catch a reversed axis
//! and useless for the question tuning actually asks, which is whether the
//! loop settles or rings.
//!
//! This is the integrator in the middle. Motor duties in, body rates out,
//! fed back as the sample the firmware reads.
//!
//! It carries an orientation as well as a rate, which is what lets it hand
//! the firmware a believable **accelerometer** — gravity projected into the
//! body frame. That makes a *fusion* filter testable, and fusion is where a
//! great many drone bugs live: a complementary filter weighted wrongly
//! drifts, and axes that disagree between gyro and accelerometer leave an
//! aircraft slowly leaning while every individual reading looks fine.
//!
//! **It is a model, and a deliberately small one.** Three things it does
//! *not* claim:
//!
//! - **No translation.** Gravity is the only thing the accelerometer sees,
//!   because nothing here moves through space. That makes it exactly a drone
//!   clamped to a test gimbal — which is the bench setup people actually use,
//!   and is honest as long as nobody reads a hover out of it.
//! - **No aerodynamics past a damping term.** Ground effect, prop wash and
//!   blade flapping are what separate a simulator from a wind tunnel.
//! - **No claim about *your* aircraft.** The constants below describe a small
//!   brushed quad in the same sense a stick figure describes a person. Gains
//!   found here are a starting point that will not fly untouched; what they
//!   *do* transfer is the sign of every axis, the order of the motors, and
//!   whether the loop is stable in shape.
//!
//! What it is good for is the class of bug that costs an afternoon and a set
//! of propellers: a mixer with two motors swapped, a D term that amplifies
//! instead of damping, an integral that winds up and never comes back, a
//! filter that trusts gravity so hard the aircraft chases its own throttle.
//!
//! Compiled unconditionally and free of IO, like `protocol` beside it: the
//! frontend runs this on a timer.

/// What the simulated aircraft is like.
///
/// Yours to set and yours to be right about — rusty cannot weigh your drone.
/// The defaults are a 45 g toy quad with 7 mm coreless motors, which is the
/// shape of the thing this was written against.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlantConfig {
    /// Torque at full throttle from one motor, relative to inertia. Bigger is
    /// a twitchier aircraft; this and [`Self::inertia`] only matter as a
    /// ratio, so one of them is the dial and the other is the unit.
    pub authority: f32,
    /// Seconds for a motor to reach a new commanded speed.
    ///
    /// The most important constant here after the sign of the mixer. A
    /// brushed motor is not instant, and that lag is exactly what a D term
    /// interacts with — a plant that spun up instantly would make almost any
    /// D look fine.
    pub spin_up: f32,
    /// Rotational inertia, roll/pitch/yaw. Yaw is larger on a quad and its
    /// authority is smaller, which together are why yaw is the slow axis.
    pub inertia: [f32; 3],
    /// How hard the air resists rotation, per axis, as a fraction of rate.
    /// Without it a step input integrates for ever and every gain looks
    /// unstable.
    pub damping: f32,
    /// How much of a motor's torque reaches yaw. A quad yaws by *drag*
    /// differential rather than thrust, which is a much weaker effect — and
    /// the reason a badly tuned yaw axis is usually just slow rather than
    /// dangerous.
    pub yaw_authority: f32,
}

impl Default for PlantConfig {
    fn default() -> Self {
        PlantConfig {
            authority: 26.0,
            spin_up: 0.035,
            inertia: [1.0, 1.0, 1.8],
            damping: 1.2,
            yaw_authority: 0.22,
        }
    }
}

/// Orientation, as a unit quaternion.
///
/// **Not three Euler angles accumulated.** Body rates are not Euler-angle
/// rates — the two agree only near level, and integrating rates straight into
/// angles drifts as soon as the aircraft is not, then fails outright at 90° of
/// pitch where roll and yaw become the same axis. A quaternion has neither
/// problem and costs four numbers instead of three.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Quat {
    pub w: f32,
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Default for Quat {
    /// Level and facing forward.
    fn default() -> Self {
        Quat {
            w: 1.0,
            x: 0.0,
            y: 0.0,
            z: 0.0,
        }
    }
}

impl Quat {
    /// Turn by a body-frame rate for `dt` seconds.
    ///
    /// Renormalised every step. Floating-point integration walks off the unit
    /// sphere within seconds, and an un-normalised quaternion quietly scales
    /// every vector it rotates — an attitude that slowly shrinks toward level
    /// while nothing in the loop asked it to.
    fn integrate(self, rate: [f32; 3], dt: f32) -> Self {
        let (p, q, r) = (rate[0] * 0.5, rate[1] * 0.5, rate[2] * 0.5);
        let next = Quat {
            w: self.w + (-self.x * p - self.y * q - self.z * r) * dt,
            x: self.x + (self.w * p + self.y * r - self.z * q) * dt,
            y: self.y + (self.w * q - self.x * r + self.z * p) * dt,
            z: self.z + (self.w * r + self.x * q - self.y * p) * dt,
        };
        next.normalised()
    }

    fn normalised(self) -> Self {
        let len = (self.w * self.w + self.x * self.x + self.y * self.y + self.z * self.z).sqrt();
        if len < 1e-9 {
            return Quat::default();
        }
        Quat {
            w: self.w / len,
            x: self.x / len,
            y: self.y / len,
            z: self.z / len,
        }
    }

    /// Roll, pitch and yaw in radians, for reading and for drawing.
    ///
    /// Euler angles are an *output* here and never a state, which is the whole
    /// point of the quaternion above. Pitch is clamped at the poles rather
    /// than allowed to produce a NaN out of a rounding error at exactly 90°.
    pub fn euler(self) -> [f32; 3] {
        let Quat { w, x, y, z } = self;
        let roll = (2.0 * (w * x + y * z)).atan2(1.0 - 2.0 * (x * x + y * y));
        let sin_pitch = (2.0 * (w * y - z * x)).clamp(-1.0, 1.0);
        let pitch = sin_pitch.asin();
        let yaw = (2.0 * (w * z + x * y)).atan2(1.0 - 2.0 * (y * y + z * z));
        [roll, pitch, yaw]
    }

    /// A world-frame vector, expressed in body coordinates.
    fn to_body(self, world: [f32; 3]) -> [f32; 3] {
        // v' = q* ⊗ v ⊗ q, written out rather than composed, because a
        // general quaternion multiply here would be three temporaries and one
        // more place to transpose a sign.
        let Quat { w, x, y, z } = self;
        let [vx, vy, vz] = world;
        let t = [
            2.0 * (y * vz - z * vy),
            2.0 * (z * vx - x * vz),
            2.0 * (x * vy - y * vx),
        ];
        [
            vx - w * t[0] + (y * t[2] - z * t[1]),
            vy - w * t[1] + (z * t[0] - x * t[2]),
            vz - w * t[2] + (x * t[1] - y * t[0]),
        ]
    }
}

/// The aircraft's state between steps, and what it is made of.
#[derive(Debug, Clone, Copy)]
pub struct Plant {
    pub config: PlantConfig,
    /// Where each motor actually is, chasing what it was told.
    spin: [f32; 4],
    /// Body rates in rad/s: roll, pitch, yaw.
    rate: [f32; 3],
    /// Which way it is pointing.
    attitude: Quat,
}

impl Default for Plant {
    fn default() -> Self {
        Plant {
            config: PlantConfig::default(),
            spin: [0.0; 4],
            rate: [0.0; 3],
            attitude: Quat::default(),
        }
    }
}

impl Plant {
    /// Advance by `dt` seconds under four commanded duties, and report what
    /// a gyro bolted to this body would read.
    ///
    /// Motor order is the quad-X convention `examples/rate-loop` mixes for:
    /// front-left, front-right, rear-right, rear-left. Diagonals spin the
    /// same way, which is what makes yaw work at all — and getting this order
    /// wrong is the single most common way a first flight ends, which is why
    /// it is worth being able to discover at a desk.
    pub fn step(&mut self, duties: [f32; 4], dt: f32) -> [f32; 3] {
        // Nothing sensible follows from a zero or backwards step, and a NaN
        // here would poison every later sample.
        if !(dt.is_finite() && dt > 0.0) {
            return self.rate;
        }

        // Motors chase their command rather than jumping to it. Exponential
        // approach, clamped so a huge `dt` cannot overshoot past the target
        // and oscillate — a numerical artefact would read as a real one.
        let alpha = (dt / self.spin_up_or_default()).clamp(0.0, 1.0);
        for (spin, duty) in self.spin.iter_mut().zip(duties) {
            let target = duty.clamp(0.0, 1.0);
            *spin += (target - *spin) * alpha;
        }

        let config = self.config;
        // Thrust goes as the square of speed. Not a detail: it is why a loop
        // tuned at hover throttle rings at full throttle, and a plant linear
        // in duty would hide the most common surprise in tuning.
        let thrust: [f32; 4] = [
            self.spin[0] * self.spin[0],
            self.spin[1] * self.spin[1],
            self.spin[2] * self.spin[2],
            self.spin[3] * self.spin[3],
        ];

        // The mixer, inverted. Right pair up and left pair down is a roll.
        let roll = (thrust[1] + thrust[2]) - (thrust[0] + thrust[3]);
        let pitch = (thrust[0] + thrust[1]) - (thrust[2] + thrust[3]);
        // Yaw comes from drag on the diagonals, not from thrust asymmetry.
        let yaw = ((thrust[0] + thrust[2]) - (thrust[1] + thrust[3])) * config.yaw_authority;

        let torque = [
            roll * config.authority,
            pitch * config.authority,
            yaw * config.authority,
        ];
        for ((rate, torque), inertia) in self.rate.iter_mut().zip(torque).zip(config.inertia) {
            let accel = torque / inertia - config.damping * *rate;
            *rate += accel * dt;
        }
        self.attitude = self.attitude.integrate(self.rate, dt);
        self.rate
    }

    /// Which way it is pointing: roll, pitch and yaw in radians.
    pub fn attitude(&self) -> [f32; 3] {
        self.attitude.euler()
    }

    /// The orientation itself, for anything that would rather not go through
    /// Euler angles — a drawing, most obviously.
    pub fn orientation(&self) -> Quat {
        self.attitude
    }

    /// What an accelerometer bolted to this body would read, in g.
    ///
    /// **This is why attitude was worth adding.** With it the plant can hand
    /// the firmware a believable accelerometer as well as a gyro, and a
    /// *fusion* filter becomes testable — which is where a great many drone
    /// bugs live: a complementary filter weighted wrongly drifts, and axes
    /// that disagree between the two sensors leave an aircraft slowly leaning
    /// while every individual reading looks fine.
    ///
    /// Gravity only. There is no translation in this model, so there is no
    /// linear acceleration to add — which makes it exactly a drone clamped to
    /// a test gimbal, and that is the bench setup people actually use.
    ///
    /// Convention: body X forward, Y right, Z up, and level reads
    /// `[0, 0, 1]` — a GY-521 flat on a desk. Rolling right puts gravity on
    /// +Y. Yours may differ, and finding out that it does is what the board
    /// drawing in the Flight panel is for.
    pub fn accelerometer(&self) -> [f32; 3] {
        self.attitude.to_body([0.0, 0.0, 1.0])
    }

    /// The rates as they stand, without advancing anything.
    pub fn rate(&self) -> [f32; 3] {
        self.rate
    }

    /// A gust: add to the body rates without the motors having done it.
    ///
    /// The only way to see whether a loop *recovers*. Left undisturbed with a
    /// zero setpoint, a stable loop and an unstable one both sit at zero and
    /// look identical — the difference only appears on the way back from
    /// somewhere.
    pub fn disturb(&mut self, rate: [f32; 3]) {
        for (axis, kick) in self.rate.iter_mut().zip(rate) {
            if kick.is_finite() {
                *axis += kick;
            }
        }
    }

    /// Put the aircraft back on the bench, keeping how it is built.
    ///
    /// What disarming should do, and what the panel does when the loop is
    /// switched off — a plant left spinning would hand the next run a rate
    /// nobody commanded. The config survives because it describes the
    /// aircraft rather than its motion.
    pub fn reset(&mut self) {
        self.spin = [0.0; 4];
        self.rate = [0.0; 3];
        self.attitude = Quat::default();
    }

    fn spin_up_or_default(&self) -> f32 {
        if self.config.spin_up > 0.0 {
            self.config.spin_up
        } else {
            0.001
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DT: f32 = 0.005;

    /// Run the plant for `seconds` holding one set of duties.
    fn hold(plant: &mut Plant, duties: [f32; 4], seconds: f32) -> [f32; 3] {
        let steps = (seconds / DT) as usize;
        let mut rate = [0.0; 3];
        for _ in 0..steps {
            rate = plant.step(duties, DT);
        }
        rate
    }

    /// Four equal motors are a hover, whatever the throttle. An aircraft that
    /// rotated under balanced power would make every tuning session chase a
    /// bug in the simulator.
    #[test]
    fn balanced_motors_do_not_rotate() {
        for throttle in [0.0, 0.3, 0.6, 1.0] {
            let mut plant = Plant::default();
            let rate = hold(&mut plant, [throttle; 4], 2.0);
            for axis in rate {
                assert!(axis.abs() < 1e-4, "throttle {throttle} rotated: {rate:?}");
            }
        }
    }

    /// The sign that matters most. Lifting the right pair rolls one way and
    /// lifting the left pair rolls the other — a plant that got this backwards
    /// would teach every user to reverse a mixer that was already correct.
    #[test]
    fn the_pairs_roll_in_opposite_directions() {
        let mut right = Plant::default();
        let mut left = Plant::default();
        // Motors 1 and 2 are the right pair, 0 and 3 the left.
        let leaning_right = hold(&mut right, [0.3, 0.7, 0.7, 0.3], 1.0)[0];
        let leaning_left = hold(&mut left, [0.7, 0.3, 0.3, 0.7], 1.0)[0];

        assert!(leaning_right.abs() > 0.1, "no roll at all: {leaning_right}");
        assert_eq!(
            leaning_right.signum(),
            -leaning_left.signum(),
            "the two pairs rolled the same way: {leaning_right} and {leaning_left}",
        );
    }

    #[test]
    fn the_front_and_rear_pairs_pitch_in_opposite_directions() {
        let mut nose_up = Plant::default();
        let mut nose_down = Plant::default();
        // 0 and 1 are the front pair, 2 and 3 the rear.
        let up = hold(&mut nose_up, [0.7, 0.7, 0.3, 0.3], 1.0)[1];
        let down = hold(&mut nose_down, [0.3, 0.3, 0.7, 0.7], 1.0)[1];
        assert!(up.abs() > 0.1);
        assert_eq!(up.signum(), -down.signum());
    }

    /// Yaw comes off the diagonals, and it is the slow axis. Both halves
    /// matter: a plant where yaw responded like roll would let somebody tune
    /// a yaw gain that is wildly too high for the real aircraft.
    #[test]
    fn yaw_comes_from_the_diagonals_and_is_the_weak_axis() {
        let mut yawing = Plant::default();
        // 0 and 2 are one diagonal, 1 and 3 the other.
        let yaw = hold(&mut yawing, [0.7, 0.3, 0.7, 0.3], 1.0)[2];
        assert!(yaw.abs() > 0.0, "the diagonals produced no yaw");

        let mut rolling = Plant::default();
        let roll = hold(&mut rolling, [0.3, 0.7, 0.7, 0.3], 1.0)[0];
        assert!(
            yaw.abs() < roll.abs(),
            "yaw ({yaw}) should be weaker than roll ({roll}) — a quad yaws by drag",
        );
    }

    /// Damping is what makes a rate settle instead of integrating for ever.
    /// Without it every gain looks unstable and the panel teaches nothing.
    /// Measured after it has had time to settle, not while it still is. The
    /// time constant is `1 / damping` — about 0.8s with the defaults — so a
    /// reading at one second is at 70% of steady state and comparing it to
    /// anything later shows a rise that is the model working correctly.
    #[test]
    fn a_held_input_settles_rather_than_growing_without_bound() {
        let held = [0.3, 0.7, 0.7, 0.3];
        let mut plant = Plant::default();
        let settled = hold(&mut plant, held, 4.0)[0];
        let still = hold(&mut plant, held, 4.0)[0];
        assert!(
            (still - settled).abs() < settled.abs() * 0.01,
            "still accelerating after eight seconds: {settled} then {still}",
        );

        // And it settles where the arithmetic says: torque balances damping,
        // so the steady rate is `torque / (inertia * damping)`. Checking the
        // value and not merely the flatness is what would catch damping
        // applied twice, which also flattens.
        let config = plant.config;
        let torque = (0.7 * 0.7 * 2.0 - 0.3 * 0.3 * 2.0) * config.authority;
        let expected = torque / (config.inertia[0] * config.damping);
        assert!(
            (settled - expected).abs() < expected.abs() * 0.02,
            "settled at {settled}, arithmetic says {expected}",
        );
    }

    /// Cutting the motors has to bring the rates back down, or a loop could
    /// never recover in the simulator however well it was tuned.
    #[test]
    fn rates_decay_when_the_motors_stop() {
        let mut plant = Plant::default();
        let spinning = hold(&mut plant, [0.3, 0.7, 0.7, 0.3], 2.0)[0];
        let coasting = hold(&mut plant, [0.0; 4], 3.0)[0];
        assert!(
            coasting.abs() < spinning.abs() * 0.2,
            "rate barely decayed: {spinning} then {coasting}",
        );
    }

    /// Thrust goes as the square of speed, so the same *difference* in duty
    /// produces more torque near full throttle than near idle. This is the
    /// reason a loop tuned at hover rings when the throttle comes up, and a
    /// plant linear in duty would hide it.
    #[test]
    fn the_same_duty_split_bites_harder_at_high_throttle() {
        let mut low = Plant::default();
        let mut high = Plant::default();
        let gentle = hold(&mut low, [0.1, 0.3, 0.3, 0.1], 1.0)[0].abs();
        let fierce = hold(&mut high, [0.7, 0.9, 0.9, 0.7], 1.0)[0].abs();
        assert!(
            fierce > gentle * 1.5,
            "the square is not being modelled: {gentle} then {fierce}",
        );
    }

    /// A motor takes time to reach its commanded speed. That lag is what a D
    /// term interacts with, so a plant without it would make almost any D
    /// look well behaved.
    #[test]
    fn a_motor_does_not_reach_its_command_instantly() {
        let mut plant = Plant::default();
        // One step, far shorter than the spin-up constant.
        plant.step([1.0, 0.0, 0.0, 0.0], 0.001);
        assert!(
            plant.spin[0] < 0.2,
            "the motor jumped straight to its command: {}",
            plant.spin[0],
        );
        hold(&mut plant, [1.0, 0.0, 0.0, 0.0], 0.5);
        assert!(
            plant.spin[0] > 0.9,
            "and it should get there eventually: {}",
            plant.spin[0],
        );
    }

    /// A step that is zero, negative or not a number must change nothing. The
    /// frontend's timer can hand over any of the three across a tab switch or
    /// a clock adjustment, and one NaN would poison every later sample.
    #[test]
    fn a_nonsense_step_changes_nothing() {
        let mut plant = Plant::default();
        hold(&mut plant, [0.3, 0.7, 0.7, 0.3], 0.5);
        let before = plant.rate();
        for bad in [0.0, -0.01, f32::NAN, f32::INFINITY] {
            assert_eq!(plant.step([1.0, 0.0, 0.0, 0.0], bad), before);
        }
        assert_eq!(plant.rate(), before);
    }

    /// The config has to be reachable, or it is documentation pretending to
    /// be a dial. A stiffer aircraft must turn more slowly under the same
    /// motors.
    #[test]
    fn a_heavier_aircraft_turns_more_slowly() {
        let held = [0.3, 0.7, 0.7, 0.3];
        let mut light = Plant::default();
        let mut heavy = Plant {
            config: PlantConfig {
                inertia: [4.0, 4.0, 6.0],
                ..PlantConfig::default()
            },
            ..Plant::default()
        };
        let quick = hold(&mut light, held, 0.3)[0].abs();
        let sluggish = hold(&mut heavy, held, 0.3)[0].abs();
        assert!(
            sluggish < quick * 0.5,
            "inertia changed nothing: {quick} then {sluggish}",
        );
    }

    /// A gust with the motors idle has to decay on its own, or nothing could
    /// be said about whether a *loop* brought it back.
    #[test]
    fn a_gust_decays_when_nothing_answers_it() {
        let mut plant = Plant::default();
        plant.disturb([5.0, 0.0, 0.0]);
        assert_eq!(plant.rate()[0], 5.0);
        let after = hold(&mut plant, [0.0; 4], 4.0)[0];
        assert!(after.abs() < 0.5, "the gust never decayed: {after}");

        // And a nonsense gust changes nothing rather than poisoning the state.
        let mut plant = Plant::default();
        plant.disturb([f32::NAN, f32::INFINITY, 1.0]);
        assert_eq!(plant.rate()[0], 0.0);
        assert_eq!(plant.rate()[2], 1.0);
    }

    /// Spin the body at a fixed rate about one axis and read the angle back.
    fn turn(rate: [f32; 3], seconds: f32) -> Quat {
        let mut q = Quat::default();
        let steps = (seconds / DT) as usize;
        for _ in 0..steps {
            q = q.integrate(rate, DT);
        }
        q
    }

    /// A constant rate for a known time is a known angle. The floor under
    /// everything else here.
    #[test]
    fn a_held_rate_integrates_to_the_angle_it_should() {
        let quarter = core::f32::consts::FRAC_PI_2;
        let [roll, _, _] = turn([quarter, 0.0, 0.0], 1.0).euler();
        assert!(
            (roll - quarter).abs() < 0.02,
            "a quarter turn a second for a second should be 90°, got {}°",
            roll.to_degrees(),
        );
    }

    /// The property naive Euler integration gets wrong, and the reason the
    /// state is a quaternion.
    ///
    /// Roll 90°, then yaw 90°, and the result is *not* the same as yawing
    /// then rolling. Adding rates into angle accumulators cannot express that
    /// at all — it would give the same answer both ways, and be wrong about
    /// an aircraft that had done anything but stay level.
    #[test]
    fn rotations_do_not_commute_and_the_model_knows_it() {
        let quarter = core::f32::consts::FRAC_PI_2;
        let mut roll_then_yaw = Quat::default();
        roll_then_yaw = roll_then_yaw.integrate([quarter, 0.0, 0.0], 1.0);
        roll_then_yaw = roll_then_yaw.integrate([0.0, 0.0, quarter], 1.0);

        let mut yaw_then_roll = Quat::default();
        yaw_then_roll = yaw_then_roll.integrate([0.0, 0.0, quarter], 1.0);
        yaw_then_roll = yaw_then_roll.integrate([quarter, 0.0, 0.0], 1.0);

        let a = roll_then_yaw.euler();
        let b = yaw_then_roll.euler();
        let apart: f32 = a.iter().zip(b).map(|(p, q)| (p - q).abs()).sum();
        assert!(
            apart > 0.5,
            "the two orders came out the same ({a:?} and {b:?}) — this is \
             accumulating angles, not rotating",
        );
    }

    /// Straight up is where Euler angles fail and a quaternion does not. A
    /// model that produced a NaN here would poison every later sample and
    /// take the whole panel with it.
    #[test]
    fn pointing_straight_up_produces_numbers_rather_than_nan() {
        let quarter = core::f32::consts::FRAC_PI_2;
        let q = turn([0.0, quarter, 0.0], 1.0);
        let angles = q.euler();
        assert!(
            angles.iter().all(|a| a.is_finite()),
            "gimbal lock produced {angles:?}",
        );
        assert!(
            (angles[1] - quarter).abs() < 0.02,
            "pitch should be 90°, got {}°",
            angles[1].to_degrees(),
        );
    }

    /// Integration walks off the unit sphere within seconds, and an
    /// un-normalised quaternion scales every vector it rotates — an attitude
    /// that shrinks toward level while nothing asked it to.
    #[test]
    fn the_orientation_stays_a_unit_quaternion() {
        let q = turn([2.1, -1.4, 0.9], 30.0);
        let len = (q.w * q.w + q.x * q.x + q.y * q.y + q.z * q.z).sqrt();
        assert!(
            (len - 1.0).abs() < 1e-3,
            "drifted off the unit sphere: {len}"
        );
    }

    /// Level is `[0, 0, 1]`, and tilting moves gravity onto the axis that
    /// went down. These signs are a convention, and one worth pinning: an
    /// accelerometer whose axes disagree with the gyro's leaves an aircraft
    /// slowly leaning while every individual reading looks fine.
    #[test]
    fn the_accelerometer_reads_gravity_where_the_body_is_pointing() {
        let plant = Plant::default();
        let level = plant.accelerometer();
        assert!(
            level[0].abs() < 1e-4 && level[1].abs() < 1e-4 && (level[2] - 1.0).abs() < 1e-4,
            "level should read [0, 0, 1], got {level:?}",
        );

        let quarter = core::f32::consts::FRAC_PI_2;
        let plant = Plant {
            attitude: turn([quarter, 0.0, 0.0], 1.0),
            ..Plant::default()
        };
        let rolled = plant.accelerometer();
        assert!(
            rolled[1] > 0.98 && rolled[2].abs() < 0.05,
            "rolled 90° right should put gravity on +Y, got {rolled:?}",
        );

        let plant = Plant {
            attitude: turn([0.0, quarter, 0.0], 1.0),
            ..Plant::default()
        };
        let pitched = plant.accelerometer();
        assert!(
            pitched[0] < -0.98 && pitched[2].abs() < 0.05,
            "pitched 90° up should put gravity on -X, got {pitched:?}",
        );
    }

    /// Yaw is the one rotation gravity cannot see, and a fusion filter that
    /// believed otherwise would correct heading from the accelerometer —
    /// which is exactly why a magnetometer exists.
    #[test]
    fn spinning_flat_does_not_move_gravity() {
        let plant = Plant {
            attitude: turn([0.0, 0.0, 2.0], 1.5),
            ..Plant::default()
        };
        let spun = plant.accelerometer();
        assert!(
            spun[0].abs() < 1e-3 && spun[1].abs() < 1e-3 && (spun[2] - 1.0).abs() < 1e-3,
            "yaw changed what gravity reads: {spun:?}",
        );
        // And the yaw itself did happen, so the reading above is gravity
        // being blind to it rather than nothing having turned.
        assert!(plant.attitude()[2].abs() > 1.0, "nothing yawed at all");
    }

    #[test]
    fn resetting_puts_it_back_on_the_bench() {
        let mut plant = Plant::default();
        hold(&mut plant, [0.3, 0.7, 0.7, 0.3], 1.0);
        assert!(plant.rate()[0].abs() > 0.0);
        plant.reset();
        assert_eq!(plant.rate(), [0.0; 3]);
        assert_eq!(plant.spin, [0.0; 4]);
    }
}
