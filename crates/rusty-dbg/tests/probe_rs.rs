//! Driven against a real `probe-rs dap-server`, when one is installed.
//!
//! What a desk with no board can prove: that probe-rs takes the handshake
//! and the launch this client sends — its own `SessionConfig`, read off its
//! source — and that a launch it cannot carry out comes back at once with
//! probe-rs's own reason, rather than as a wait out the minutes a flash is
//! allowed. The image named does not exist, so the launch is refused on a
//! desk with a probe plugged in as well: never a board written by a test.

use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

use rusty_dbg::{Board, DapLaunch, DapSession, Error};

fn probe_rs() -> Option<PathBuf> {
    let answered = Command::new("probe-rs").arg("--version").output().ok()?;
    answered.status.success().then(|| PathBuf::from("probe-rs"))
}

#[test]
fn a_launch_probe_rs_cannot_carry_out_is_refused_at_once_in_its_words() {
    let Some(adapter) = probe_rs() else {
        eprintln!("skipping: probe-rs is not installed on this machine");
        return;
    };
    let dir = std::env::temp_dir().join(format!("rusty-probe-rs-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    let launch = DapLaunch {
        adapter,
        program: dir.join("no-such-image.elf"),
        args: Vec::new(),
        root: dir.clone(),
        breakpoints: vec![("src/main.rs".to_string(), 10)],
        board: Some(Board {
            chip: "STM32F411CEUx".to_string(),
            probe: None,
        }),
    };
    let started = Instant::now();
    let error = match DapSession::start(&launch) {
        Ok(_) => panic!("a launch of an image that does not exist was accepted"),
        Err(error) => error,
    };
    assert!(
        started.elapsed() < Duration::from_secs(60),
        "refused only after {:?}",
        started.elapsed()
    );
    let Error::Refused(reason) = &error else {
        panic!("not probe-rs's refusal: {error}");
    };
    eprintln!("probe-rs said: {reason}");
    assert!(!reason.trim().is_empty());
}
