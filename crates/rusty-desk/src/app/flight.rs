//! The Flight tab: what a control loop is doing, in the shape the loop has
//! — which way the craft leans and whether one motor does all the work,
//! both questions about several channels at one instant, where Plot
//! answers about one channel over many.
//!
//! **Nothing here is invented.** The attitude comes from channels the
//! firmware named, matched by convention; the motor bars come from
//! `[rusty:pwm]`, the firmware's own word about what it commanded. Where a
//! channel is missing the panel says so rather than drawing a level
//! horizon, because a level horizon is a claim.
//!
//! **The plant closes the loop**: a rigid body between the motors and the
//! gyro (`rusty_embed::Plant`), stepped at 50 Hz and fed to the sensors the
//! firmware declared, matched by name — a gyro gets rates, an accelerometer
//! gravity. Off by default and never self-starting: firmware that reads a
//! real IMU and accepts injection would otherwise see two sources disagree.

use std::{rc::Rc, time::Duration};

use rusty_embed::{
    Plant,
    plot::Plot,
    scene::{self, Camera, Look, Projector, Svg, model},
    spatial::{Euler, Frame, Quat, Vec3},
};
use rusty_i18n::t;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use super::App;
use crate::{DrawPiece, FlightAxis, FlightMotor, Shell};

/// Channel names used for the three rates without being told, the rate
/// loop's own word for an axis first.
const ROLL: [&str; 4] = ["gyro_r", "roll", "gyro_x", "rate_roll"];
const PITCH: [&str; 4] = ["gyro_p", "pitch", "gyro_y", "rate_pitch"];
const YAW: [&str; 4] = ["gyro_y", "yaw", "gyro_z", "rate_yaw"];

/// How often the plant steps and injects: 50 Hz, far slower than the loop
/// it feeds, which is fine because the aircraft is slower than either.
const PLANT_PERIOD: Duration = Duration::from_millis(20);
const PLANT_DT: f32 = 0.02;

/// The craft's view, in pixels.
const CRAFT: f64 = 132.0;

#[derive(Default)]
pub(super) struct Flight {
    plant: Plant,
    pub(super) closed: bool,
}

impl Flight {
    /// The plant's attitude and rates while the loop is closed — what a
    /// math sheet's `truth()` and `truth_rate()` read.
    pub(super) fn truth(&self) -> Option<(rusty_embed::spatial::Quat, rusty_embed::spatial::Vec3)> {
        if !self.closed {
            return None;
        }
        let q = self.plant.orientation();
        let r = self.plant.rate();
        Some((
            rusty_embed::spatial::Quat::new(
                f64::from(q.w),
                f64::from(q.x),
                f64::from(q.y),
                f64::from(q.z),
            ),
            rusty_embed::spatial::Vec3::new(f64::from(r[0]), f64::from(r[1]), f64::from(r[2])),
        ))
    }
}

/// What the plant has to offer a sensor.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Feed {
    Rates,
    Gravity,
}

fn latest(plot: &Plot, channel: &str) -> Option<f32> {
    plot.channels
        .iter()
        .find(|(name, _)| name == channel)
        .and_then(|(_, samples)| samples.last())
        .map(|(_, value)| *value)
}

/// The first of `wanted` the firmware is actually talking about.
fn preferred(plot: &Plot, wanted: &[&str]) -> Option<String> {
    wanted
        .iter()
        .find(|name| plot.channels.iter().any(|(known, _)| known == *name))
        .map(|name| (*name).to_string())
}

impl App {
    /// Which declared sensors the plant can fill, and with what — matched
    /// by name, three components only.
    fn plant_feeds(&self) -> Vec<(String, Feed)> {
        let state = self.state.borrow();
        state
            .sim
            .sensors
            .iter()
            .filter(|def| def.components == 3)
            .filter_map(|def| {
                let lower = def.name.to_lowercase();
                let feed = if lower.contains("gyro") || lower.contains("rate") {
                    Feed::Rates
                } else if lower.contains("acc") {
                    Feed::Gravity
                } else {
                    return None;
                };
                Some((def.name.clone(), feed))
            })
            .collect()
    }

    /// Close the physical loop, or open it again; the plant starts level.
    pub(super) fn flight_close(self: &Rc<Self>, closed: bool) {
        {
            let mut state = self.state.borrow_mut();
            state.flight.closed = closed;
            state.flight.plant.reset();
        }
        if closed {
            let weak = Rc::downgrade(self);
            self.plant_clock
                .start(slint::TimerMode::Repeated, PLANT_PERIOD, move || {
                    if let Some(app) = weak.upgrade() {
                        app.plant_step();
                    }
                });
        } else {
            self.plant_clock.stop();
        }
        self.sync_flight();
    }

    /// One step of the simulated aircraft, fed to the sensors.
    fn plant_step(self: &Rc<Self>) {
        if !self.sim_live() {
            return;
        }
        let feeds = self.plant_feeds();
        let lines = {
            let mut state = self.state.borrow_mut();
            let mut motors: Vec<(u8, f32)> =
                state.sim.duties.iter().map(|(p, d)| (*p, d.duty)).collect();
            motors.sort_by_key(|(pin, _)| *pin);
            // Fewer than four driven pins is not an aircraft: wait rather
            // than fly half its motors as dead.
            if motors.len() < 4 {
                return;
            }
            let duties = [motors[0].1, motors[1].1, motors[2].1, motors[3].1];
            let plant = &mut state.flight.plant;
            plant.step(duties, PLANT_DT);
            let (rate, gravity) = (plant.rate(), plant.accelerometer());
            let mut lines = Vec::new();
            for (name, feed) in feeds {
                let sample = match feed {
                    Feed::Rates => rate,
                    Feed::Gravity => gravity,
                };
                lines.push(rusty_embed::sensor_line(&name, &sample));
                state.sim.sensor_values.insert(name, sample.to_vec());
            }
            lines
        };
        for line in lines {
            self.sim_send_console(&line);
        }
        self.flight_moved();
    }

    /// Something the panel reads moved: redrawn a few times a second.
    pub(super) fn flight_moved(self: &Rc<Self>) {
        if self.flight_due.replace(true) {
            return;
        }
        let weak = Rc::downgrade(self);
        slint::Timer::single_shot(Duration::from_millis(80), move || {
            if let Some(app) = weak.upgrade() {
                app.flight_due.set(false);
                app.sync_flight();
            }
        });
    }

    pub(super) fn sync_flight(&self) {
        let Some(window) = self.window() else { return };
        let shell = window.global::<Shell>();
        let feeds = self.plant_feeds();
        let state = self.state.borrow();
        let plot = &state.sim.plot;
        let axis = |label: String, wanted: &[&str]| -> FlightAxis {
            match preferred(plot, wanted) {
                Some(channel) => {
                    let value = latest(plot, &channel);
                    FlightAxis {
                        label: label.into(),
                        channel: channel.into(),
                        value: value
                            .map_or_else(|| "—".to_string(), |v| format!("{v:+.2}"))
                            .into(),
                        known: true,
                    }
                }
                None => FlightAxis {
                    label: label.into(),
                    channel: t!("dock.flight.no-channel").into(),
                    value: "—".into(),
                    known: false,
                },
            }
        };
        let roll = axis(t!("dock.flight.roll"), &ROLL);
        let pitch = axis(t!("dock.flight.pitch"), &PITCH);
        let yaw = axis(t!("dock.flight.yaw"), &YAW);
        let value_of =
            |wanted: &[&str]| preferred(plot, wanted).and_then(|channel| latest(plot, &channel));
        // The horizon: rates scaled to read.
        let horizon_roll = value_of(&ROLL);
        let horizon_pitch = value_of(&PITCH).unwrap_or(0.0);
        let mut motors: Vec<FlightMotor> = state
            .sim
            .duties
            .iter()
            .map(|(pin, duty)| FlightMotor {
                pin: i32::from(*pin),
                duty: duty.duty.clamp(0.0, 1.0),
                text: format!("{:.0}%", duty.duty * 100.0).into(),
            })
            .collect();
        motors.sort_by_key(|m| m.pin);
        let nothing = !roll.known && !pitch.known && !yaw.known && motors.is_empty();

        // The craft: the plant's attitude while the loop is closed, else
        // the firmware's own angles where it prints all three.
        let flight = &state.flight;
        let attitude: Option<([f32; 3], String)> = if flight.closed {
            Some((flight.plant.attitude(), t!("dock.flight.from-plant")))
        } else {
            let pick = |names: &[&str]| {
                preferred(plot, names).and_then(|n| latest(plot, &n).map(|v| (n, v)))
            };
            match (
                pick(&["att_roll", "roll"]),
                pick(&["att_pitch", "pitch", "pit"]),
                pick(&["att_yaw", "yaw"]),
            ) {
                (Some((name, r)), Some((_, p)), Some((_, y))) => Some((
                    [r.to_radians(), p.to_radians(), y.to_radians()],
                    t!("dock.flight.from-channels", name = name),
                )),
                _ => None,
            }
        };
        let mut craft = Vec::new();
        let (craft_text, craft_source) = match &attitude {
            Some(([roll, pitch, yaw], source)) => {
                let q = Quat::from_euler(Euler {
                    roll: f64::from(*roll),
                    pitch: f64::from(*pitch),
                    yaw: f64::from(*yaw),
                });
                let quad = model::quad(q, Frame::ZUp, false);
                // Framed on the craft level, rotors and all, so a lean is
                // seen as a lean rather than as the view refitting.
                let level = model::quad(Quat::IDENTITY, Frame::ZUp, false);
                let r = model::extent(&level);
                let corners: Vec<Vec3> = [(r, r), (r, -r), (-r, r), (-r, -r)]
                    .into_iter()
                    .flat_map(|(x, y)| [Vec3::new(x, y, r * 0.5), Vec3::new(x, y, -r * 0.5)])
                    .collect();
                let camera =
                    Camera::look(Look::Iso).framing(&corners, Frame::ZUp, CRAFT, CRAFT, (6.0, 6.0));
                let items: Vec<_> = quad.into_iter().map(|item| (item, 1.0)).collect();
                for drawn in
                    scene::render(&items, &Projector::new(&camera, Frame::ZUp, CRAFT, CRAFT))
                {
                    let (ink, rgb) = super::draw::ink_of(drawn.ink);
                    let mut piece = DrawPiece {
                        ink,
                        rgb,
                        width: drawn.width as f32,
                        opacity: drawn.opacity as f32,
                        fill: drawn.fill as f32,
                        ..DrawPiece::default()
                    };
                    match drawn.svg {
                        Svg::Path(path) => piece.commands = path.into(),
                        Svg::Polygon(points) => {
                            piece.kind = 1;
                            piece.commands = super::draw::polygon(&points).into();
                        }
                        Svg::Text { .. } => continue,
                        Svg::Dot { x, y, r } => {
                            piece.kind = 3;
                            piece.x = x as f32;
                            piece.y = y as f32;
                            piece.r = r as f32;
                        }
                    }
                    craft.push(piece);
                }
                (
                    format!(
                        "{:+.0}° {:+.0}° {:+.0}°",
                        roll.to_degrees(),
                        pitch.to_degrees(),
                        yaw.to_degrees()
                    ),
                    source.clone(),
                )
            }
            None => (String::new(), String::new()),
        };
        let blocker = if feeds.is_empty() {
            Some(t!("dock.flight.no-sensor"))
        } else if motors.len() < 4 {
            Some(t!(
                "dock.flight.few-motors",
                motors = motors.len().to_string()
            ))
        } else {
            None
        };
        let feeding = feeds
            .iter()
            .map(|(name, feed)| {
                let what = match feed {
                    Feed::Rates => t!("dock.flight.feed-rates"),
                    Feed::Gravity => t!("dock.flight.feed-gravity"),
                };
                format!("{name}←{what}")
            })
            .collect::<Vec<_>>()
            .join("  ");
        shell.set_flight_nothing(nothing);
        shell.set_flight_axes(ModelRc::new(VecModel::from(vec![roll, pitch, yaw])));
        shell.set_flight_horizon_known(horizon_roll.is_some());
        shell.set_flight_horizon_roll(horizon_roll.unwrap_or(0.0) * 6.0);
        shell.set_flight_horizon_shift((horizon_pitch * -4.0).clamp(-40.0, 40.0));
        shell.set_flight_motors(ModelRc::new(VecModel::from(motors)));
        shell.set_flight_craft(ModelRc::new(VecModel::from(craft)));
        shell.set_flight_craft_known(attitude.is_some());
        shell.set_flight_craft_text(SharedString::from(craft_text));
        shell.set_flight_craft_source(SharedString::from(craft_source));
        shell.set_flight_closed(flight.closed);
        shell.set_flight_ready(blocker.is_none());
        shell.set_flight_feeds(SharedString::from(feeding));
        shell.set_flight_help(SharedString::from(
            blocker.unwrap_or_else(|| t!("dock.flight.plant-help")),
        ));
    }
}
