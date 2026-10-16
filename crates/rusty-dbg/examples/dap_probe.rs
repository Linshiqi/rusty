//! Drive a debug adapter against a real test binary, headless.
//!
//! `cargo run -p rusty-dbg --example dap_probe -- <project> <exe> <file> <line> [args…]`
//!
//! This is the check the unit tests cannot be: they pin the conversions, and
//! this pins that an adapter on *this* machine actually launches a program,
//! stops where it was told, can say what the locals hold, and goes on again
//! when told to. It is how "debugging does not work on Windows" gets split
//! into "no adapter answers" and "rusty is asking the wrong thing".
//!
//! It fails loudly rather than passing quietly: a session that never stops is
//! the failure worth catching, and a probe that shrugged at it would be the
//! test that reported a working debugger on a machine with none.
//!
//! **Every stop is answered, and the program is driven to its end.** What is
//! done at each stop is `DAP_PROBE_STEPS`, in order — `continue`, `over`,
//! `into`, `out`, and `break:<line>` or `break:<file>:<line>`, which places
//! a breakpoint at once and waits for no stop. Once the list runs out every
//! stop is continued. The first version resumed once and then waited, so a
//! breakpoint hit twice — in a loop, in a helper called twice — left the
//! program stopped at its second hit with nobody to say go, and that read as
//! Continue not working.
//!
//! `DAP_PROBE_EXPECT` lists the lines, one-based, the stops are to be at, and
//! `DAP_PROBE_EXIT` the program's exit code; either one missed fails the
//! probe. `DAP_PROBE_CWD` is the directory the program runs in, where it is
//! not the project — a member's test runs in its package's directory, as
//! `cargo test` runs it. `RUSTY_DAP_ADAPTER` names the one adapter to try,
//! and `RUSTY_DAP_LOG` prints every message on the wire.
//!
//! A failing test's panic is written to its stderr, which is what marks the
//! line it failed on; the probe says whether it reached the output.

use std::path::PathBuf;
use std::sync::mpsc::{RecvTimeoutError, channel};
use std::time::{Duration, Instant};

use rusty_dbg::{DapLaunch, DapSession, DebugState};

/// What to do at a stop.
#[derive(Debug, Clone)]
enum Verb {
    Continue,
    Over,
    Into,
    Out,
    /// Placed at once; the next verb follows without a stop.
    Break(String, u32),
}

fn verbs(text: &str, file: &str) -> Vec<Verb> {
    text.split(',')
        .map(str::trim)
        .filter(|word| !word.is_empty())
        .map(|word| match word {
            "continue" | "c" => Verb::Continue,
            "over" | "next" | "n" => Verb::Over,
            "into" | "in" | "s" => Verb::Into,
            "out" => Verb::Out,
            _ => {
                let place = word
                    .strip_prefix("break:")
                    .unwrap_or_else(|| panic!("not a verb: {word}"));
                let (at, line) = place.rsplit_once(':').unwrap_or((file, place));
                let line: u32 = line.parse().expect("a line number");
                Verb::Break(at.to_string(), line - 1)
            }
        })
        .collect()
}

fn main() {
    let mut args = std::env::args().skip(1);
    let root = PathBuf::from(args.next().expect("project directory"));
    let program = PathBuf::from(args.next().expect("test executable"));
    let file = args.next().expect("source file, project-relative");
    let line: u32 = args.next().expect("line").parse().expect("a line number");
    let rest: Vec<String> = args.collect();
    let mut plan = verbs(&std::env::var("DAP_PROBE_STEPS").unwrap_or_default(), &file).into_iter();
    let expect: Option<Vec<u32>> = std::env::var("DAP_PROBE_EXPECT").ok().map(|text| {
        text.split(',')
            .map(|line| line.trim().parse().expect("an expected line"))
            .collect()
    });
    let expect_exit: Option<i32> = std::env::var("DAP_PROBE_EXIT")
        .ok()
        .map(|code| code.parse().expect("an exit code"));

    // `RUSTY_DAP_ADAPTER` names one adapter and tries no other — which copy
    // was measured is then a fact, not the ladder's choice.
    let adapters = match std::env::var_os("RUSTY_DAP_ADAPTER") {
        Some(adapter) => vec![PathBuf::from(adapter)],
        None => rusty_embed::host_debug::host_adapters(),
    };
    if adapters.is_empty() {
        eprintln!("no debug adapter found: install lldb-dap, or CodeLLDB");
        std::process::exit(2);
    }
    eprintln!("adapters, best first:");
    for adapter in &adapters {
        eprintln!("  {}", adapter.display());
    }

    // One-based on the way in, because that is how a person reads a file.
    let breakpoints = vec![(file.clone(), line - 1)];

    let mut session = None;
    for adapter in adapters {
        eprintln!("trying {}", adapter.display());
        let launch = DapLaunch {
            adapter: adapter.clone(),
            program: program.clone(),
            args: rest.clone(),
            root: root.clone(),
            cwd: std::env::var_os("DAP_PROBE_CWD").map(PathBuf::from),
            breakpoints: breakpoints.clone(),
            board: None,
        };
        match DapSession::start(&launch) {
            Ok(pair) => {
                eprintln!("  started");
                session = Some(pair);
                break;
            }
            Err(e) => eprintln!("  refused: {e}"),
        }
    }
    let Some((session, events)) = session else {
        eprintln!("PROBE FAILED: no adapter would start");
        std::process::exit(1);
    };

    // The session's stream blocks; a deadline needs a stream that can be
    // waited on for a while and then given up on.
    let (forward, states) = channel::<DebugState>();
    std::thread::spawn(move || {
        while let Some(state) = events.next() {
            if forward.send(state).is_err() {
                break;
            }
        }
    });

    let started = Instant::now();
    let deadline = started + Duration::from_secs(90);
    let mut printed = Vec::new();
    // Where the first breakpoint was placed, which is not always the line
    // asked for: `rusty_dbg::placement`, or a line with no code the adapter
    // moved it off.
    let mut landed = line - 1;
    // Each stop as it was read: its line, one-based, and why.
    let mut stops: Vec<(Option<String>, Option<u32>, String)> = Vec::new();
    // The stop being looked at: when its stack arrived, and whether it has
    // been answered. A stop is answered once its locals are in, or a moment
    // after its stack if the frame has none.
    let mut at_stop: Option<Instant> = None;
    let mut answered = true;
    let mut have_locals = false;
    let mut exited = None;
    let mut ended = false;
    let mut last_error: Option<String> = None;

    loop {
        let now = Instant::now();
        if now >= deadline {
            eprintln!("deadline: 90 s and the program has not exited");
            break;
        }
        // Answer a stop whose locals never came.
        let wait = match at_stop {
            Some(since) if !answered => (since + Duration::from_millis(600))
                .saturating_duration_since(now)
                .min(deadline - now),
            _ => deadline - now,
        };
        let state = match states.recv_timeout(wait) {
            Ok(state) => Some(state),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => {
                ended = true;
                break;
            }
        };
        if let Some(state) = state {
            for text in &state.output {
                printed.push(text.clone());
                eprintln!("  [program] {text}");
            }
            if state.error != last_error {
                eprintln!("  [error] {:?}", state.error);
                last_error = state.error.clone();
            }
            if let Some(placed) = state
                .breakpoints
                .iter()
                .find(|b| b.file == file && b.requested == Some(line - 1))
                && placed.line != landed
            {
                landed = placed.line;
                eprintln!("  the breakpoint on {line} was placed on {}", landed + 1);
            }
            if let Some(code) = state.exited {
                exited = Some(code);
                break;
            }
            let resting = !state.running && !state.stack.is_empty();
            if resting && at_stop.is_none() {
                let frame = &state.stack[0];
                eprintln!(
                    "STOP {} after {:.1?} at {}:{} in {} ({:?})",
                    stops.len() + 1,
                    started.elapsed(),
                    frame.file.as_deref().unwrap_or("?"),
                    frame.line.map(|l| l + 1).unwrap_or(0),
                    frame.function,
                    state.reason,
                );
                for f in state.stack.iter().take(3) {
                    eprintln!(
                        "   #{} {} {}:{}",
                        f.level,
                        f.function,
                        f.file.as_deref().unwrap_or("?"),
                        f.line.map(|l| l + 1).unwrap_or(0),
                    );
                }
                stops.push((
                    frame.file.clone(),
                    frame.line.map(|l| l + 1),
                    format!("{:?}", state.reason),
                ));
                at_stop = Some(Instant::now());
                answered = false;
                have_locals = false;
            } else if !resting {
                // Running again, or a stop whose stack has not come yet.
                at_stop = None;
            }
            if at_stop.is_some() && !answered && !state.variables.is_empty() {
                let locals: Vec<String> = state
                    .variables
                    .iter()
                    .take(6)
                    .map(|v| format!("{} = {}", v.name, v.value))
                    .collect();
                eprintln!("   locals: {}", locals.join(", "));
                have_locals = true;
            }
        }
        let complete = match at_stop {
            Some(since) if !answered => {
                have_locals || since.elapsed() >= Duration::from_millis(600)
            }
            _ => false,
        };
        if !complete {
            continue;
        }

        // The stop is complete: what the plan says next, every `break` in a
        // row placed on the way.
        answered = true;
        loop {
            let verb = plan.next().unwrap_or(Verb::Continue);
            eprintln!("  -> {verb:?}");
            let sent = match &verb {
                Verb::Continue => session.resume(),
                Verb::Over => session.step_over(),
                Verb::Into => session.step_into(),
                Verb::Out => session.step_out(),
                Verb::Break(at, line) => session.add_breakpoint(at, *line),
            };
            if let Err(e) = sent {
                eprintln!("PROBE FAILED: {verb:?} could not be sent: {e}");
                session.stop();
                std::process::exit(1);
            }
            if !matches!(verb, Verb::Break(..)) {
                break;
            }
        }
    }

    session.stop();
    let mut failed = Vec::new();
    match printed.iter().find(|line| line.contains("panicked at")) {
        Some(panic) => eprintln!("the program's panic reached the output: {panic}"),
        None => eprintln!("no panic reached the output"),
    }
    eprintln!(
        "stops: {}",
        stops
            .iter()
            .map(|(_, line, reason)| format!("{}({reason})", line.unwrap_or(0)))
            .collect::<Vec<_>>()
            .join(" ")
    );
    match exited {
        Some(code) => eprintln!("exited with {code} after {:.1?}", started.elapsed()),
        None if ended => failed.push("the session ended without an exit".to_string()),
        None => failed.push("the program never exited".to_string()),
    }
    match stops.first() {
        Some((Some(at), Some(hit), _)) if *at == file && *hit == landed + 1 => {}
        Some((at, hit, _)) => failed.push(format!(
            "the first stop was at {at:?}:{hit:?}, expected {file}:{}",
            landed + 1
        )),
        None => failed.push("the breakpoint was never hit".to_string()),
    }
    if let Some(expect) = &expect {
        let got: Vec<u32> = stops.iter().map(|(_, line, _)| line.unwrap_or(0)).collect();
        if &got != expect {
            failed.push(format!("stopped at {got:?}, expected {expect:?}"));
        }
    }
    if let Some(code) = expect_exit
        && exited != Some(code)
    {
        failed.push(format!("exited with {exited:?}, expected {code}"));
    }
    if failed.is_empty() {
        eprintln!("PROBE OK");
    } else {
        for reason in &failed {
            eprintln!("PROBE FAILED: {reason}");
        }
        std::process::exit(1);
    }
}
