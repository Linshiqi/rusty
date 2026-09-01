//! Prove the *inward* half of the protocol: that an injected sample reaches
//! a control loop and changes what it does.
//!
//! ```text
//! cargo run -p rusty-embed --example loop_probe -- examples/rate-loop
//! ```
//!
//! `sim_probe` proves the pipeline builds, boots and speaks. It cannot prove
//! injection, because it never writes — and a panel whose sliders silently
//! went nowhere would look exactly like firmware ignoring them, which is the
//! failure `tune_probe` exists for on the hardware side. This is that check
//! for the simulator.
//!
//! Four things have to be true, in order, or the loop is not closed:
//!
//! 1. the firmware **announces** a sensor, so a panel knows what it may feed;
//! 2. it **arms** on a button, proving `B` still reaches it;
//! 3. an injected `Igyro=…` **changes the motor duties**, proving the sample
//!    arrived, was used, and travelled through the controller;
//! 4. rolling one way and then the other moves the motors the **opposite**
//!    way, proving the sign is real rather than a number that happens to
//!    differ.
//!
//! Point 4 is the one worth the extra seconds. A loop that reacted to any
//! injection with the same asymmetry would pass 1–3 while being wired
//! backwards, and a backwards rate loop is a drone that flips on takeoff.

use std::time::{Duration, Instant};

use rusty_embed::{PwmReport, parse_pwm_report, parse_sensor_def, sensor_line};

/// How long to wait for any one thing to happen before calling it a failure.
const PATIENCE: Duration = Duration::from_secs(20);

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
    let mut steps = plan.steps.into_iter().enumerate();
    let mut boot = None;
    for (index, step) in &mut steps {
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

    // Every read is bounded: a firmware that never says the thing being
    // waited for must fail the probe rather than hang it.
    let wait_for = |what: &str, mut done: Box<dyn FnMut(&str) -> bool>| {
        let deadline = Instant::now() + PATIENCE;
        while Instant::now() < deadline {
            let Some(line) = session.recv() else {
                break;
            };
            if done(&line.text) {
                println!("  ✓ {what}");
                return;
            }
        }
        stopper.stop();
        eprintln!("✗ never saw {what}");
        std::process::exit(1);
    };

    // 1. The declaration. Without it a panel has nothing to offer, and
    //    inventing a sensor is the whole thing this protocol refuses to do.
    wait_for(
        "the firmware announce a sensor it wants fed",
        Box::new(|line| {
            parse_sensor_def(line).is_some_and(|def| def.name == "gyro" && def.components == 3)
        }),
    );

    // 2. Arm, and take throttle off the floor so a correction has something
    //    to be a correction *to*. At zero throttle every mixer output clamps
    //    at zero and the loop would look dead while working perfectly.
    input.send_line("B9=1");
    input.send_line("Sthrottle=0.5");
    wait_for(
        "the loop arm and spin all four motors",
        Box::new(|line| {
            parse_pwm_report(line).is_some_and(|r| r.pins.iter().all(|(_, duty)| *duty > 0.1))
        }),
    );

    // 3 and 4. Roll one way, then the other, and require the motors to move
    //    the opposite way each time.
    let lean = |input: &rusty_embed::process::Input, rate: f32| {
        input.send_line(&sensor_line("gyro", &[rate, 0.0, 0.0]));
    };
    let asymmetry = |report: &PwmReport| -> f32 {
        let duty = |pin: u8| {
            report
                .pins
                .iter()
                .find(|(p, _)| *p == pin)
                .map(|(_, d)| *d)
                .unwrap_or(0.0)
        };
        // Motors 0 and 3 are the left pair, 1 and 2 the right. Rolling right
        // has to push one pair up and the other down.
        (duty(1) + duty(2)) - (duty(0) + duty(3))
    };

    let settled = |rate: f32, what: &str| -> f32 {
        lean(&input, rate);
        let deadline = Instant::now() + PATIENCE;
        let mut seen = 0;
        while Instant::now() < deadline {
            let Some(line) = session.recv() else { break };
            if let Some(report) = parse_pwm_report(&line.text) {
                let last = asymmetry(&report);
                seen += 1;
                // A few reports in, the integral has had time to act.
                if seen > 40 {
                    println!("  ✓ {what}: asymmetry {last:+.3}");
                    return last;
                }
            }
        }
        stopper.stop();
        eprintln!("✗ no motor reports while {what}");
        std::process::exit(1);
    };

    let right = settled(6.0, "rolling right");
    let left = settled(-6.0, "rolling left");
    stopper.stop();

    if right.abs() < 0.01 {
        eprintln!(
            "✗ the injected sample changed nothing — the loop never saw it.\n  \
             Motor duties were symmetric at a gyro rate of 6 rad/s."
        );
        std::process::exit(1);
    }
    if right.signum() == left.signum() {
        eprintln!(
            "✗ rolling both ways moved the motors the same way ({right:+.3} then {left:+.3}).\n  \
             The sample arrives but the sign is wrong — a rate loop wired like this \n  \
             flips the aircraft on takeoff."
        );
        std::process::exit(1);
    }

    println!(
        "\nthe loop is closed: a declared sensor, an injected sample, and a\n\
         controller that answers it in the right direction ({right:+.3} / {left:+.3})"
    );
}
