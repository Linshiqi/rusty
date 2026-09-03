//! Drive a debug adapter against a real test binary, headless.
//!
//! `cargo run -p rusty-dbg --example dap_probe -- <project> <exe> <file> <line> [args…]`
//!
//! This is the check the unit tests cannot be: they pin the conversions, and
//! this pins that an adapter on *this* machine actually launches a program,
//! stops where it was told, and can say what the locals hold. It is how
//! "debugging does not work on Windows" gets split into "no adapter answers"
//! and "rusty is asking the wrong thing".
//!
//! It fails loudly rather than passing quietly: a session that never stops is
//! the failure worth catching, and a probe that shrugged at it would be the
//! test that reported a working debugger on a machine with none.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use rusty_dbg::{DapLaunch, DapSession};

fn main() {
    let mut args = std::env::args().skip(1);
    let root = PathBuf::from(args.next().expect("project directory"));
    let program = PathBuf::from(args.next().expect("test executable"));
    let file = args.next().expect("source file, project-relative");
    let line: u32 = args.next().expect("line").parse().expect("a line number");
    let rest: Vec<String> = args.collect();

    let adapters = rusty_embed::host_debug::host_adapters();
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
            breakpoints: breakpoints.clone(),
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

    let started = Instant::now();
    let deadline = started + Duration::from_secs(90);
    let mut stopped_where = None;
    let mut printed = Vec::new();

    while Instant::now() < deadline {
        let Some(state) = events.next() else { break };
        for text in &state.output {
            printed.push(text.clone());
            eprintln!("  [program] {text}");
        }
        if let Some(error) = &state.error {
            eprintln!("  [error] {error}");
        }
        if !state.running
            && stopped_where.is_none()
            && let Some(frame) = state.stack.first()
        {
            eprintln!(
                "STOPPED after {:.1?} at {}:{} in {}",
                started.elapsed(),
                frame.file.as_deref().unwrap_or("?"),
                frame.line.map(|l| l + 1).unwrap_or(0),
                frame.function,
            );
            for f in state.stack.iter().take(4) {
                eprintln!(
                    "   #{} {} {}:{}",
                    f.level,
                    f.function,
                    f.file.as_deref().unwrap_or("?"),
                    f.line.map(|l| l + 1).unwrap_or(0),
                );
            }
            stopped_where = Some((frame.file.clone(), frame.line));
        }
        if stopped_where.is_some() && !state.variables.is_empty() {
            eprintln!("locals:");
            for v in state.variables.iter().take(8) {
                eprintln!("   {} = {}", v.name, v.value);
            }
            // Everything the panel needs has arrived; let the program finish
            // so the adapter and the debuggee are not left running.
            session.resume().expect("resume");
            break;
        }
        if state.exited.is_some() {
            break;
        }
    }

    session.stop();

    match stopped_where {
        Some((Some(at), Some(hit))) if at == file && hit + 1 == line => {
            eprintln!("PROBE OK: stopped at {at}:{}", hit + 1);
        }
        Some((at, hit)) => {
            eprintln!(
                "PROBE FAILED: stopped at {:?}:{:?}, expected {file}:{line}",
                at,
                hit.map(|l| l + 1),
            );
            std::process::exit(1);
        }
        None => {
            eprintln!("PROBE FAILED: the breakpoint was never hit");
            std::process::exit(1);
        }
    }
}
