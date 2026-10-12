//! The two-way serial link, proven against a real board without the window.
//!
//! `sim_probe`'s sibling: that one proves the simulation pipeline, this one
//! proves the half a simulator cannot — that a port rusty holds open reaches
//! a board that is actually running, and that the board answers.
//!
//! ```text
//! cargo run -p rusty-embed --example tune_probe -- COM7 [set...]
//! cargo run -p rusty-embed --example tune_probe -- COM7 setpoint=80 kp=500
//! ```
//!
//! Each `name=value` is sent as the Plot panel's slider would send it, and
//! what comes back is printed. A clamp is the interesting case: the answer
//! carries what the firmware *took*, which is not necessarily what was sent.

use std::{
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use rusty_embed::{protocol, serial};

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(port) = args.next() else {
        eprintln!("usage: tune_probe <port> [name=value ...]");
        std::process::exit(2);
    };
    let sets: Vec<String> = args.collect();

    let link = match serial::open(&port, 115_200) {
        Ok(link) => link,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };
    let input = link.input();
    let stopper = link.stopper();

    // Everything the board says, kept so a "did it change" question can be
    // answered against what arrived before the write rather than after.
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let collector = Arc::clone(&seen);
    thread::spawn(move || {
        while let Some(line) = link.recv() {
            collector.lock().expect("seen").push(line.text);
        }
    });

    let settle = |for_: Duration| thread::sleep(for_);
    let count = || seen.lock().expect("seen").len();
    let since = |mark: usize| seen.lock().expect("seen")[mark..].to_vec();

    settle(Duration::from_secs(3));
    let opening = since(0);
    println!("--- {} lines in the first 3s", opening.len());
    if opening.is_empty() {
        println!("    nothing at all. Is the board running, and is this its port?");
        stopper.stop();
        return;
    }
    for line in opening.iter().take(3) {
        println!("    {line}");
    }

    let params: Vec<_> = opening
        .iter()
        .filter_map(|l| protocol::parse_param(l))
        .collect();
    println!("--- tunables announced: {}", params.len());
    for param in &params {
        println!(
            "    {} = {} ({})",
            param.name,
            param.value,
            match (param.min, param.max) {
                (Some(low), Some(high)) => format!("{low}..{high}"),
                _ => "no range — the panel will not draw a slider".into(),
            }
        );
    }
    let telemetry = opening
        .iter()
        .filter(|l| l.starts_with("[rusty:tel"))
        .count();
    println!("--- telemetry lines: {telemetry}");
    if let Some(last) = opening
        .iter()
        .rev()
        .find_map(|l| protocol::parse_telemetry(l))
    {
        println!(
            "    latest sample: {}",
            last.channels
                .iter()
                .map(|(name, value)| format!("{name}={value}"))
                .collect::<Vec<_>>()
                .join(" ")
        );
    }

    for set in sets {
        let Some((name, value)) = set.split_once('=') else {
            eprintln!("--- skipping `{set}`: expected name=value");
            continue;
        };
        let Ok(value) = value.parse::<f32>() else {
            eprintln!("--- skipping `{set}`: `{value}` is not a number");
            continue;
        };

        // What it held before the write. Firmware that re-announces on a timer
        // — which it should, so a panel connecting to an already-flying board
        // sees the tunables at all — means "a [rusty:param] line arrived" is
        // NOT evidence the write landed. Only a *change* is. Getting this
        // wrong reported a board that heard nothing as one that clamped.
        let before = since(0)
            .iter()
            .filter_map(|l| protocol::parse_param(l))
            .rfind(|p| p.name == name)
            .map(|p| p.value);

        let mark = count();
        let line = protocol::set_param_line(name, value);
        println!("--- sending {line} (it held {before:?})");
        input.send_line(&line);

        // Wait for the change rather than for a fixed time: a board that
        // answers in 20ms and one that answers in a second are both fine, and
        // a board that never answers is the finding.
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut answer = None;
        while Instant::now() < deadline && answer.is_none() {
            answer = since(mark)
                .iter()
                .filter_map(|l| protocol::parse_param(l))
                .rfind(|p| p.name == name && Some(p.value) != before);
            if answer.is_none() {
                settle(Duration::from_millis(50));
            }
        }
        match (answer, before) {
            (Some(param), _) if param.value == value => {
                println!("    took {} — as sent", param.value)
            }
            (Some(param), _) => println!("    took {} — clamped from {value}", param.value),
            // A set that asks for what it already holds cannot be told from
            // one that was never heard, and saying "confirmed" would be a
            // guess. Say which it is.
            (None, Some(held)) if held == value => {
                println!("    it already held {held}; this set proves nothing either way")
            }
            (None, _) => println!(
                "    unchanged after 3s. The board did not hear it, or does not know that name."
            ),
        }
    }

    settle(Duration::from_millis(500));
    if let Some(last) = since(0)
        .iter()
        .rev()
        .find_map(|l| protocol::parse_telemetry(l))
    {
        println!(
            "--- final sample: {}",
            last.channels
                .iter()
                .map(|(name, value)| format!("{name}={value}"))
                .collect::<Vec<_>>()
                .join(" ")
        );
    }
    stopper.stop();
}
