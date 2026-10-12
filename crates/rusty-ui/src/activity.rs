//! What the workbench is doing right now, and how the last thing it did
//! went — the first item in the status bar.
//!
//! Xcode's activity view is the shape. While something runs it says what,
//! and how far it has got (`Building · esp-hal · 12 s`); when it stops it
//! says how that went (`Build succeeded · 12.4 s · 85.3 KB flash`) and keeps
//! saying it until the next run replaces it. The bar used to say "Working"
//! for all of it, and a build's whole result was a scrollback somebody had
//! to open and read from the bottom up.
//!
//! Everything here is pure and tested: `controller::session` hands every
//! line a session prints to [`Activity::observe`] and the exit code to
//! [`Activity::finish`], and the status bar draws what comes back.

/// What kind of work a session is — decided by the output channel it
/// streams under (`state.dock.source`), which every runner already names.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Build,
    Test,
    Flash,
    Monitor,
    Simulate,
    /// A simulation booted frozen for the debugger.
    Debug,
    Install,
    /// The serial link the Plot panel's tunables ride on.
    Link,
    /// Anything else run through the dock — a git write, a typed command.
    Command,
}

impl Kind {
    pub fn of_channel(channel: &str) -> Kind {
        match channel {
            "build" => Kind::Build,
            "test" => Kind::Test,
            "flash" => Kind::Flash,
            "monitor" => Kind::Monitor,
            "simulate" => Kind::Simulate,
            "tools" => Kind::Install,
            "link" => Kind::Link,
            _ => Kind::Command,
        }
    }

    /// Runs until somebody stops it rather than until it is done. Its end is
    /// not a verdict — a monitor that was closed did not fail — so only a
    /// non-zero exit it reached by itself is worth reporting.
    pub fn open_ended(self) -> bool {
        matches!(
            self,
            Kind::Monitor | Kind::Simulate | Kind::Debug | Kind::Link
        )
    }
}

/// Something running now.
#[derive(Clone, PartialEq, Debug)]
pub struct Activity {
    pub kind: Kind,
    /// When it started, in milliseconds (`Date.now()`); the bar counts from
    /// it.
    pub started: f64,
    /// What it is on at the moment: the crate being compiled.
    pub step: Option<String>,
    /// Where it is going: the port a flash writes to, the command a
    /// `Command` runs.
    pub target: Option<String>,
    pub errors: u32,
    pub warnings: u32,
    /// Tests, summed over every test binary's `test result:` line.
    pub passed: u32,
    pub failed: u32,
    /// cargo's counts per unit, as `(unit, errors, warnings)`: a crate that
    /// fails says its warnings twice — ``generated 1 warning`` and then
    /// ``…; 1 warning emitted`` — so they are kept by unit and not summed
    /// line by line.
    counted: Vec<(String, u32, u32)>,
}

/// How the last one ended.
#[derive(Clone, PartialEq, Debug)]
pub struct Outcome {
    pub kind: Kind,
    pub ok: bool,
    pub took_ms: f64,
    /// The exit code, when a failure has one worth naming.
    pub code: Option<i32>,
    pub errors: u32,
    pub warnings: u32,
    pub passed: u32,
    pub failed: u32,
    pub target: Option<String>,
    /// What the image a successful build produced costs the chip, once the
    /// analysis of it has landed — PlatformIO's and Arduino's closing line,
    /// which is the one number people read after every build.
    pub size: Option<Size>,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Size {
    pub flash: u64,
    pub ram: u64,
    pub ram_capacity: Option<u32>,
}

impl Activity {
    pub fn new(kind: Kind, started: f64) -> Activity {
        Activity {
            kind,
            started,
            step: None,
            target: None,
            errors: 0,
            warnings: 0,
            passed: 0,
            failed: 0,
            counted: Vec::new(),
        }
    }

    /// Take in one line the session printed.
    ///
    /// Returns an outcome when the line itself is one: a flash that goes on
    /// monitoring has *finished flashing* long before its process exits, and
    /// the bar should say so then — the activity carries on as a monitor of
    /// the same port.
    pub fn observe(&mut self, line: &str, now: f64) -> Option<Outcome> {
        if let Some(name) = compiling(line) {
            self.step = Some(name.to_string());
        } else if line.trim_start().starts_with("Finished `") {
            // cargo is done; what runs next — the emulator, the flash — is
            // not a crate being compiled.
            self.step = None;
        } else if let Some((unit, errors, warnings)) = tally(line) {
            match self.counted.iter_mut().find(|(known, ..)| *known == unit) {
                Some((_, e, w)) => {
                    *e = (*e).max(errors);
                    *w = (*w).max(warnings);
                }
                None => self.counted.push((unit, errors, warnings)),
            }
            self.errors = self.counted.iter().map(|(_, e, _)| e).sum();
            self.warnings = self.counted.iter().map(|(_, _, w)| w).sum();
        } else if let Some((passed, failed)) = test_result(line) {
            self.passed += passed;
            self.failed += failed;
        } else if self.kind == Kind::Flash && flashed(line) {
            let done = Outcome {
                kind: Kind::Flash,
                ok: true,
                took_ms: now - self.started,
                code: None,
                errors: 0,
                warnings: 0,
                passed: 0,
                failed: 0,
                target: self.target.clone(),
                size: None,
            };
            self.kind = Kind::Monitor;
            self.started = now;
            self.step = None;
            return Some(done);
        }
        None
    }

    /// The verdict when the process exits, or `None` when its end says
    /// nothing: an open-ended session that exited cleanly, or a command
    /// that did what it was asked — a git write's success is the panel
    /// moving, not a line in the status bar.
    pub fn finish(self, code: Option<i32>, now: f64) -> Option<Outcome> {
        // `None` is how a runner reports a process it could not get a code
        // for; the dock calls that finished, and so does this.
        let ok = matches!(code, Some(0) | None);
        if (self.kind.open_ended() || self.kind == Kind::Command) && ok {
            return None;
        }
        Some(Outcome {
            kind: self.kind,
            ok,
            took_ms: now - self.started,
            code: code.filter(|c| *c != 0),
            errors: self.errors,
            warnings: self.warnings,
            passed: self.passed,
            failed: self.failed,
            target: self.target,
            size: None,
        })
    }
}

/// Whether a line could matter to [`Activity::observe`] at all — a cheap
/// look before anything is copied, since a simulation or a monitor prints
/// thousands of lines a second and none of them is about the activity.
pub fn relevant(line: &str) -> bool {
    let line = line.trim_start();
    [
        "Compiling ",
        "Finished ",
        "warning: ",
        "error: could not compile",
        "test result: ",
        "Flashing has completed",
    ]
    .iter()
    .any(|start| line.starts_with(start))
}

/// The crate a cargo line says it is compiling: `   Compiling esp-hal
/// v1.1.2` names `esp-hal`.
pub fn compiling(line: &str) -> Option<&str> {
    line.trim_start()
        .strip_prefix("Compiling ")?
        .split_whitespace()
        .next()
}

/// The counts cargo states itself, as `(unit, errors, warnings)`.
///
/// Read off its summary lines — ``warning: `app` (bin "app") generated 3
/// warnings`` and ``error: could not compile `app` (bin "app") due to 2
/// previous errors; 3 warnings emitted`` — rather than by counting lines
/// that start with `error`: one diagnostic spans a dozen lines, and a
/// `warning:` inside a note would be counted twice. The unit is the part
/// that names what was compiled (`` `app` (bin "app") ``), which both
/// lines about one crate share.
pub fn tally(line: &str) -> Option<(String, u32, u32)> {
    let line = line.trim();
    if let Some(rest) = line.strip_prefix("error: could not compile ") {
        let unit = rest.split(" due to ").next().unwrap_or(rest).to_string();
        // An old cargo says "due to previous error" with no number; it is
        // still at least one.
        let errors = number_before(rest, " previous error").unwrap_or(1);
        let warnings = number_before(rest, " warning").unwrap_or(0);
        return Some((unit, errors, warnings));
    }
    if let Some(rest) = line.strip_prefix("warning: ") {
        let (unit, after) = rest.split_once(" generated ")?;
        let count = after.split_whitespace().next()?.parse().ok()?;
        return Some((unit.to_string(), 0, count));
    }
    None
}

/// The number written just before `word`: `due to 2 previous errors` is 2.
fn number_before(text: &str, word: &str) -> Option<u32> {
    let at = text.find(word)?;
    text[..at].rsplit(' ').next()?.parse().ok()
}

/// A test binary's closing line, as `(passed, failed)`: `test result: ok.
/// 12 passed; 0 failed; …`.
pub fn test_result(line: &str) -> Option<(u32, u32)> {
    let rest = line.trim().strip_prefix("test result: ")?;
    let passed = number_before(rest, " passed")?;
    let failed = number_before(rest, " failed")?;
    Some((passed, failed))
}

/// The line after which the image is on the chip: espflash's own
/// announcement, or probe-rs finishing its download.
pub fn flashed(line: &str) -> bool {
    let line = line.trim();
    line.starts_with("Flashing has completed") || line.starts_with("Finished in ")
}

/// A duration the way the bar says it: tenths under a minute, whole
/// seconds after, as `(minutes, seconds)` for the caller to word.
pub fn clock(ms: f64) -> (u64, f64) {
    let seconds = (ms.max(0.0) / 1000.0).max(0.0);
    if seconds < 60.0 {
        (0, (seconds * 10.0).floor() / 10.0)
    } else {
        let whole = seconds.floor() as u64;
        (whole / 60, (whole % 60) as f64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The lines are cargo's own, copied from a real build; the crate name
    /// is what the bar shows, and nothing else in the line is.
    #[test]
    fn the_crate_being_compiled_is_read_off_cargo_s_line() {
        assert_eq!(compiling("   Compiling esp-hal v1.1.2"), Some("esp-hal"),);
        assert_eq!(
            compiling("   Compiling blinky v0.1.0 (E:\\work\\blinky)"),
            Some("blinky"),
        );
        assert_eq!(compiling("    Finished `release` profile"), None);
        assert_eq!(compiling("error: Compiling is not a thing"), None);
    }

    #[test]
    fn counts_come_from_cargo_s_own_summary_lines() {
        let unit = "`blinky` (bin \"blinky\")".to_string();
        assert_eq!(
            tally("warning: `blinky` (bin \"blinky\") generated 3 warnings"),
            Some((unit.clone(), 0, 3)),
        );
        assert_eq!(
            tally(
                "warning: `blinky` (bin \"blinky\") generated 1 warning (run `cargo fix \
                 --bin \"blinky\"` to apply 1 suggestion)"
            ),
            Some((unit.clone(), 0, 1)),
        );
        assert_eq!(
            tally(
                "error: could not compile `blinky` (bin \"blinky\") due to 2 previous \
                 errors; 3 warnings emitted"
            ),
            Some((unit.clone(), 2, 3)),
        );
        assert_eq!(
            tally("error: could not compile `blinky` (bin \"blinky\") due to 1 previous error"),
            Some((unit, 1, 0)),
        );
        // The diagnostics themselves are not counted: each would be counted
        // once per line it spans.
        assert_eq!(tally("warning: unused variable: `state`"), None);
        assert_eq!(
            tally("error[E0425]: cannot find value `x` in this scope"),
            None
        );
        assert_eq!(
            tally("warning: build failed, waiting for other jobs to finish..."),
            None
        );
    }

    /// A crate that fails says its warnings twice — the `generated` line and
    /// then again at the end of `could not compile` — and they are one set
    /// of warnings. Found by driving a failing build: the bar said two for
    /// one.
    #[test]
    fn a_failing_crate_s_warnings_are_counted_once() {
        let mut build = Activity::new(Kind::Build, 0.0);
        build.observe(
            "warning: `blinky` (bin \"blinky\") generated 1 warning",
            1.0,
        );
        build.observe(
            "error: could not compile `blinky` (bin \"blinky\") due to 1 previous error; \
             1 warning emitted",
            2.0,
        );
        assert_eq!((build.errors, build.warnings), (1, 1));
    }

    #[test]
    fn a_test_binary_s_closing_line_is_counted() {
        assert_eq!(
            test_result(
                "test result: ok. 12 passed; 0 failed; 0 ignored; 0 measured; 0 filtered \
                 out; finished in 0.21s"
            ),
            Some((12, 0)),
        );
        assert_eq!(
            test_result("test result: FAILED. 3 passed; 1 failed; 0 ignored"),
            Some((3, 1)),
        );
        assert_eq!(test_result("running 12 tests"), None);
    }

    /// A flash that goes on monitoring reports *flashed* at the line that
    /// says so, and carries on as a monitor of the same port — so the bar
    /// can say the image is on the board while the process is still up.
    #[test]
    fn a_flash_that_monitors_reports_at_the_line_and_carries_on() {
        let mut activity = Activity::new(Kind::Flash, 1_000.0);
        activity.target = Some("COM3".into());
        assert_eq!(activity.observe("[00:00:01] Writing", 2_000.0), None);
        let done = activity
            .observe("Flashing has completed!", 9_500.0)
            .expect("the line is the verdict");
        assert!(done.ok);
        assert_eq!(done.kind, Kind::Flash);
        assert_eq!(done.took_ms, 8_500.0);
        assert_eq!(done.target.as_deref(), Some("COM3"));
        assert_eq!(activity.kind, Kind::Monitor);
        assert_eq!(activity.started, 9_500.0);
        assert_eq!(activity.target.as_deref(), Some("COM3"));

        // Closing the monitor is not a failure, and says nothing.
        assert_eq!(activity.clone().finish(Some(0), 20_000.0), None);
        // A board unplugged mid-monitor is.
        let lost = activity.finish(Some(1), 20_000.0).expect("a verdict");
        assert!(!lost.ok);
        assert_eq!(lost.kind, Kind::Monitor);
        assert_eq!(lost.code, Some(1));
    }

    #[test]
    fn a_build_ends_with_what_it_counted() {
        let mut build = Activity::new(Kind::Build, 0.0);
        build.observe("   Compiling esp-hal v1.1.2", 100.0);
        assert_eq!(build.step.as_deref(), Some("esp-hal"));
        build.observe("warning: `esp-hal` (lib) generated 2 warnings", 200.0);
        build.observe(
            "error: could not compile `blinky` (bin \"blinky\") due to 1 previous error; \
             1 warning emitted",
            300.0,
        );
        let outcome = build.finish(Some(101), 12_400.0).expect("a verdict");
        assert!(!outcome.ok);
        assert_eq!((outcome.errors, outcome.warnings), (1, 3));
        assert_eq!(outcome.took_ms, 12_400.0);
        assert_eq!(outcome.code, Some(101));

        let clean = Activity::new(Kind::Build, 0.0)
            .finish(Some(0), 3_000.0)
            .expect("a build always has a verdict");
        assert!(clean.ok);
        assert_eq!(clean.code, None, "a clean exit names no code");
    }

    #[test]
    fn a_command_s_success_and_a_closed_monitor_say_nothing() {
        assert_eq!(Activity::new(Kind::Command, 0.0).finish(Some(0), 1.0), None);
        assert_eq!(Activity::new(Kind::Simulate, 0.0).finish(None, 1.0), None);
        let failed = Activity::new(Kind::Command, 0.0)
            .finish(Some(128), 1.0)
            .expect("a failed command is worth saying");
        assert!(!failed.ok);
        assert_eq!(failed.code, Some(128));
    }

    /// Every line `observe` acts on passes the cheap look, and the lines a
    /// firmware prints by the thousand do not.
    #[test]
    fn the_cheap_look_lets_through_exactly_what_observe_reads() {
        for line in [
            "   Compiling esp-hal v1.1.2",
            "    Finished `release` profile [optimized] target(s) in 12.34s",
            "warning: `app` (bin \"app\") generated 2 warnings",
            "error: could not compile `app` (bin \"app\") due to 1 previous error",
            "test result: ok. 3 passed; 0 failed",
            "Flashing has completed!",
            "Finished in 3.21s",
        ] {
            assert!(relevant(line), "{line}");
        }
        for line in [
            "[rusty:gpio@1234] 2=1",
            "I (952) boot: ESP-IDF",
            "hello from main",
        ] {
            assert!(!relevant(line), "{line}");
        }
        let mut run = Activity::new(Kind::Simulate, 0.0);
        run.observe("   Compiling blinky v0.1.0", 1.0);
        assert_eq!(run.step.as_deref(), Some("blinky"));
        run.observe(
            "    Finished `release` profile [optimized] target(s) in 1.0s",
            2.0,
        );
        assert_eq!(run.step, None, "the emulator is not a crate being compiled");
    }

    #[test]
    fn channels_name_their_kind() {
        assert_eq!(Kind::of_channel("build"), Kind::Build);
        assert_eq!(Kind::of_channel("flash"), Kind::Flash);
        assert_eq!(Kind::of_channel("tools"), Kind::Install);
        assert_eq!(Kind::of_channel("commands"), Kind::Command);
        assert!(Kind::Monitor.open_ended());
        assert!(!Kind::Build.open_ended());
    }

    #[test]
    fn the_clock_reads_in_tenths_then_minutes() {
        assert_eq!(clock(12_449.0), (0, 12.4));
        assert_eq!(clock(59_990.0), (0, 59.9));
        assert_eq!(clock(65_000.0), (1, 5.0));
        assert_eq!(clock(-5.0), (0, 0.0));
    }
}
