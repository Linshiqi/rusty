//! Fly the loop against the plant, and prove the simulator can tell a good
//! tune from a bad one.
//!
//! ```text
//! cargo run -p rusty-embed --example flight_probe -- examples/rate-loop
//! ```
//!
//! `loop_probe` proves an injected sample reaches the controller and moves the
//! motors the right way. That is the *open* loop: the rate it injects never
//! changes in answer to the motors, so it can catch a reversed axis and can
//! say nothing about whether the aircraft would stay in the air.
//!
//! This closes it. The same [`Plant`] the panel runs sits between the two:
//! motor duties out of QEMU, body rates back in, forty times a second. Then
//! the aircraft is kicked, and the question is whether the loop brings it
//! back.
//!
//! **The second half is the point.** A plant that showed every tune settling
//! would be decoration - it has to be able to show a bad one failing, or it
//! teaches nothing and quietly reassures. So the same gust is delivered
//! twice: once at the firmware's own gains, and once with `roll_p` at the top
//! of the range the firmware declared and `roll_d` at zero.
//!
//! What separates them is **overshoot and ringing, not peak rate**. The first
//! attempt here measured the peak and found the two identical, for a reason
//! worth keeping: the mixer clamps every motor to 0..1, and a 6 rad/s gust
//! saturates it at any gain at all. Saturated, a well-tuned loop and a wild
//! one command exactly the same thing - full one side, nothing the other. So
//! the gust is small enough to stay inside the mixer's headroom, and what is
//! counted is how far past zero the recovery swings and how many times it
//! changes its mind.

use std::time::{Duration, Instant};

use rusty_embed::{Plant, parse_pwm_report, parse_sensor_def, sensor_line};

/// One named channel out of a `[rusty:tel]` line.
fn channel(line: &str, want: &str) -> Option<f32> {
    rusty_embed::protocol::parse_telemetry(line)?
        .channels
        .into_iter()
        .find(|(name, _)| name == want)
        .map(|(_, value)| value)
}

/// Wall-clock ceiling on any one phase.
const PATIENCE: Duration = Duration::from_secs(25);
/// Seconds of simulated time per plant step, matching the panel's timer.
const DT: f32 = 0.02;

fn main() {
    let root = std::path::PathBuf::from(
        std::env::args()
            .nth(1)
            .unwrap_or_else(|| "examples/rate-loop".to_string()),
    );

    let project = rusty_embed::project::detect(&root).expect("detect");
    let plan = rusty_embed::simulate::plan(&project, false);
    if !plan.supported || !plan.missing.is_empty() {
        eprintln!("cannot simulate this project; run sim_probe for the reason");
        std::process::exit(2);
    }
    rusty_embed::simulate::prepare(&root).expect("prepare image dir");

    let total = plan.steps.len();
    let mut boot = None;
    for (index, step) in plan.steps.into_iter().enumerate() {
        println!("$ {}", step.display);
        let session = rusty_embed::process::spawn(&step, Some(&root)).expect("spawn");
        if index + 1 == total {
            boot = Some(session);
            break;
        }
        while let Some(line) = session.recv() {
            println!("  {}", line.text);
        }
        if session.wait() != Some(0) {
            eprintln!("build step failed");
            std::process::exit(1);
        }
    }
    let session = boot.expect("a boot step");
    let input = session.input();
    let stopper = session.stopper();

    let fail = |stopper: &rusty_embed::process::Stopper, why: &str| -> ! {
        stopper.stop();
        eprintln!("✗ {why}");
        std::process::exit(1);
    };

    // The sensor has to be declared before anything may be injected into it.
    let deadline = Instant::now() + PATIENCE;
    let mut sensor = None;
    while Instant::now() < deadline && sensor.is_none() {
        let Some(line) = session.recv() else { break };
        sensor = parse_sensor_def(&line.text).filter(|def| def.components == 3);
    }
    let Some(sensor) = sensor else {
        fail(&stopper, "the firmware never announced a three-axis sensor");
    };
    println!("  ✓ sensor announced: {}", sensor.name);

    input.send_line("B9=1");
    input.send_line("Sthrottle=0.5");

    /// How a recovery went.
    struct Recovery {
        /// How far past zero it swung on the way back, in rad/s. The number
        /// a tuner actually watches.
        overshoot: f32,
        /// How many times the rate changed sign. One is a clean recovery;
        /// several is the loop arguing with itself.
        crossings: u32,
        /// Where it ended up.
        settled: f32,
    }

    /// Fly the plant against the firmware for `seconds`, kicking it once at
    /// the start.
    fn fly(
        session: &rusty_embed::process::Session,
        input: &rusty_embed::process::Input,
        name: &str,
        gust: f32,
        seconds: f32,
    ) -> Option<Recovery> {
        let mut plant = Plant::default();
        plant.disturb([gust, 0.0, 0.0]);

        let mut overshoot: f32 = 0.0;
        let mut crossings = 0u32;
        let mut last: f32 = gust;
        let steps = (seconds / DT) as usize;
        let deadline = Instant::now() + PATIENCE;

        for _ in 0..steps {
            if Instant::now() > deadline {
                return None;
            }
            input.send_line(&sensor_line(name, &plant.rate()));
            input.send_line(&sensor_line("accel", &plant.accelerometer()));

            // Take the newest duties the firmware has produced. Draining to
            // the latest rather than using the first keeps the plant fed with
            // what the controller decided *now*, which is what the panel's
            // own timer sees too.
            let mut duties = None;
            let read_until = Instant::now() + Duration::from_millis(60);
            while Instant::now() < read_until {
                let Some(line) = session.recv() else { break };
                if let Some(report) = parse_pwm_report(&line.text)
                    && report.pins.len() >= 4
                {
                    let mut pins = report.pins.clone();
                    pins.sort_by_key(|(pin, _)| *pin);
                    duties = Some([pins[0].1, pins[1].1, pins[2].1, pins[3].1]);
                }
            }
            let Some(duties) = duties else { continue };

            let rate = plant.step(duties, DT)[0];
            // A sign change is the recovery passing through level. Anything
            // after the first one is overshoot; several is ringing.
            if rate.signum() != last.signum() && last.abs() > 0.005 {
                crossings += 1;
            }
            if crossings > 0 && rate.signum() != gust.signum() {
                overshoot = overshoot.max(rate.abs());
            }
            last = rate;
        }
        Some(Recovery {
            overshoot,
            crossings,
            settled: last.abs(),
        })
    }

    // Small enough to stay inside the mixer's headroom at throttle 0.5.
    // Saturated, every gain commands the same thing and the comparison below
    // would measure nothing at all.
    const GUST: f32 = 0.8;

    println!(
        "
- kicking it at the firmware's own gains"
    );
    let Some(calm) = fly(&session, &input, &sensor.name, GUST, 4.0) else {
        fail(&stopper, "the firmware stopped reporting motor duties");
    };
    println!(
        "  overshoot {:.3} rad/s, {} sign changes, ended at {:.3}",
        calm.overshoot, calm.crossings, calm.settled,
    );

    // The top of the range the firmware itself declared, with the derivative
    // term removed. Not numbers invented here: whatever it said it accepts is
    // the worst it can legally be asked to do.
    input.send_line("Sroll_p=0.6");
    input.send_line("Sroll_d=0");
    std::thread::sleep(Duration::from_millis(300));

    println!(
        "
- and again with roll_p at its ceiling and roll_d at zero"
    );
    let Some(hot) = fly(&session, &input, &sensor.name, GUST, 4.0) else {
        fail(&stopper, "the firmware stopped reporting motor duties");
    };
    println!(
        "  overshoot {:.3} rad/s, {} sign changes, ended at {:.3}",
        hot.overshoot, hot.crossings, hot.settled,
    );
    // The fusion filter, checked separately from the controller: hold the
    // aircraft at a fixed tilt and see whether the firmware's own attitude
    // estimate finds it. This is the half the accelerometer was added for,
    // and it fails in a way nothing else here would catch — a filter that
    // trusts gravity too little drifts, one that trusts it too much chases
    // the motors, and both fly until they do not.
    println!(
        "
- holding a 20 degree tilt and watching the estimate find it"
    );
    let tilt = 20.0f32.to_radians();
    let mut held = Plant::default();
    // Turn to the tilt, then stop: no motors, no rates, just an aircraft
    // sitting at an angle, which is the one case an accelerometer alone is
    // right about.
    for _ in 0..200 {
        held.disturb([tilt / (200.0 * DT) - held.rate()[0], 0.0, 0.0]);
        held.step([0.0; 4], DT);
    }
    let truth = held.attitude()[0].to_degrees();
    let mut estimate = None;
    let deadline = Instant::now() + PATIENCE;
    while Instant::now() < deadline {
        input.send_line(&sensor_line(&sensor.name, &[0.0, 0.0, 0.0]));
        input.send_line(&sensor_line("accel", &held.accelerometer()));
        let Some(line) = session.recv() else { break };
        if let Some(value) = channel(&line.text, "att_roll") {
            estimate = Some(value);
            if (value - truth).abs() < 2.0 {
                break;
            }
        }
    }
    stopper.stop();

    match estimate {
        Some(value) if (value - truth).abs() < 3.0 => {
            println!("  ✓ estimate {value:.1}° against a true {truth:.1}°");
        }
        Some(value) => {
            eprintln!(
                "x the attitude estimate settled at {value:.1}° for a true {truth:.1}°.
                   The accelerometer is arriving but the filter disagrees with it - check the
                   axis signs before the weights."
            );
            std::process::exit(1);
        }
        None => {
            eprintln!(
                "x the firmware never published att_roll, so there is no estimate to                  check.
  A fused attitude has to be reported for anything to read it."
            );
            std::process::exit(1);
        }
    }

    if calm.settled > GUST * 0.25 {
        eprintln!(
            "x the loop never recovered from a {GUST} rad/s gust at its own gains (ended              at {:.3}).
  Either the controller is not holding, or the plant is not being              fed.",
            calm.settled,
        );
        std::process::exit(1);
    }
    if hot.overshoot <= calm.overshoot && hot.crossings <= calm.crossings {
        eprintln!(
            "x a gain at the top of its range behaved no worse than the tuned one
               (calm: {:.3} overshoot, {} crossings; hot: {:.3}, {}).
               A plant that cannot show a bad tune failing is decoration - it would
               quietly reassure about every gain anybody tried.",
            calm.overshoot, calm.crossings, hot.overshoot, hot.crossings,
        );
        std::process::exit(1);
    }

    println!(
        "
the physical loop is closed. A gust at the tuned gains came back to {:.3} rad/s with {} sign change(s);
at the ceiling it overshot to {:.3} and changed its mind {} times.",
        calm.settled, calm.crossings, hot.overshoot, hot.crossings,
    );
}
