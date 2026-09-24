//! What a debug session looks like from the outside.
//!
//! Plain data, no IO — the frontend `use`s these directly, so what the
//! debugger reports and what the panel draws cannot drift.

use serde::{Deserialize, Serialize};

/// A breakpoint the user asked for, and what gdb made of it.
///
/// The request and the result are one type on purpose: a breakpoint that
/// gdb moved to the next executable line, or could not place at all, is
/// exactly what the gutter has to show. Two types would let the gutter keep
/// drawing the request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Breakpoint {
    /// gdb's own number, once it has one.
    pub number: Option<u32>,
    /// Project-relative, `/`-separated — the same identity the editor uses.
    pub file: String,
    /// Zero-based, like every other line number that crosses this boundary.
    /// gdb counts from one; the conversion happens at the edge.
    ///
    /// Where the breakpoint actually *is*, which is not always where it was
    /// asked for: an optimised build has no code on many lines, and gdb
    /// moves the breakpoint to the next line that does.
    pub line: u32,
    /// The line the user clicked, when gdb reported one — so the margin can
    /// move its dot to where execution will really stop instead of leaving
    /// it on a line the compiler deleted.
    #[serde(default)]
    pub requested: Option<u32>,
    /// False when gdb refused it — no code at that line, usually. The
    /// gutter draws it hollow and the reason says why.
    pub verified: bool,
    /// Why it is not verified, in gdb's words.
    pub reason: Option<String>,
    pub enabled: bool,
}

/// One frame of the call stack.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StackFrame {
    /// 0 is where execution stopped.
    pub level: u32,
    pub function: String,
    /// Absent for frames with no source — a HAL compiled without debug
    /// info, an interrupt vector. The panel lists them and refuses to
    /// pretend they can be opened.
    pub file: Option<String>,
    pub line: Option<u32>,
    /// Program counter, for the frames source cannot explain.
    pub address: String,
}

/// A local, an argument, or a field of one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Variable {
    pub name: String,
    pub value: String,
    /// What gdb calls it — `u32`, `&mut Output<'_>`.
    pub kind: Option<String>,
    /// gdb's handle for expanding a struct or an array, when it has parts.
    pub handle: Option<String>,
    pub children: u32,
}

/// Why the target is sitting still.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StopReason {
    Breakpoint,
    Step,
    /// Paused by the user.
    Pause,
    /// A watchpoint, a signal, a fault — anything the target did to itself.
    Signal,
    Exited,
    /// Stopped for a reason gdb named but this does not model.
    Other,
}

/// One span of target memory, as read.
///
/// A register view asks for a peripheral's whole block in one request
/// rather than a round trip per register: thirty round trips per stop over
/// a gdbstub is what makes a register panel feel broken.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryRead {
    pub begin: u64,
    /// Little-endian bytes, exactly as the target holds them.
    pub data: Vec<u8>,
}

/// Where a session is, as one value the frontend can render.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DebugState {
    /// True while the target is executing — the pause button's condition.
    pub running: bool,
    /// Set once the session is live: false while gdb is starting and
    /// attaching, which is a second or two on a cold QEMU.
    pub attached: bool,
    pub reason: Option<StopReason>,
    /// Empty while running: a stack read mid-flight is a lie.
    pub stack: Vec<StackFrame>,
    /// Locals and arguments of the selected frame.
    pub variables: Vec<Variable>,
    /// Which frame the variables belong to.
    pub frame: u32,
    pub breakpoints: Vec<Breakpoint>,
    /// The last thing that went wrong, in gdb's words.
    pub error: Option<String>,
    /// Set when the session ended, with the target's status if it had one.
    pub exited: Option<i32>,
    /// The last spans read. Replaced rather than accumulated: a register
    /// view wants what the target holds *now*, and keeping history would
    /// mean showing a value from before the last step.
    pub memory: Vec<MemoryRead>,
    /// What the session printed since the last state: the build that
    /// produced a host test binary, then the program's own stdout under
    /// gdb. Lines, not an accumulation — the frontend appends them to the
    /// dock and the next state starts empty. Empty on a remote target,
    /// whose console is the simulator's serial line and not gdb's.
    #[serde(default)]
    pub output: Vec<String>,
}

impl DebugState {
    /// Attached, not executing, and not over: where a step or a continue
    /// can start. The transport's buttons, the menu's rows and the keys all
    /// ask this, so a key cannot do what its button is greyed out for — and
    /// a step sent to a gdb still attaching is one it refuses.
    pub fn stopped(&self) -> bool {
        self.attached && !self.running && self.exited.is_none()
    }

    /// The target is executing again. A stack read while it runs is a lie, so
    /// what the last stop read is dropped rather than left on the panel as if
    /// it were current.
    pub fn resumed(&mut self) {
        self.running = true;
        self.stack.clear();
        self.variables.clear();
        self.reason = None;
    }

    /// The target has come to rest. Both debuggers read a stop at its
    /// innermost frame — gdb selects it, and the adapter's stack answer asks
    /// for its scopes — so the variables that follow are that frame's, and
    /// the selected frame goes back to it whichever was chosen before.
    pub fn halted(&mut self, reason: StopReason) {
        self.running = false;
        self.attached = true;
        self.reason = Some(reason);
        self.frame = 0;
    }

    /// A breakpoint as the debugger reported it: in place of the one carrying
    /// its number, or added. One the debugger has not numbered is always
    /// added — there is nothing to know it again by.
    pub fn record_breakpoint(&mut self, entry: Breakpoint) {
        match self
            .breakpoints
            .iter_mut()
            .find(|existing| existing.number == entry.number && entry.number.is_some())
        {
            Some(existing) => *existing = entry,
            None => self.breakpoints.push(entry),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_step_starts_only_from_a_stopped_target() {
        let live = DebugState {
            attached: true,
            ..DebugState::default()
        };
        assert!(live.stopped(), "attached and at rest");
        assert!(!DebugState::default().stopped(), "still attaching");
        assert!(
            !DebugState {
                running: true,
                ..live.clone()
            }
            .stopped(),
            "executing: that is Pause's turn"
        );
        assert!(
            !DebugState {
                exited: Some(0),
                ..live
            }
            .stopped(),
            "a program that has ended has nothing to step"
        );
    }

    /// Both debuggers say the target is going again in their own words, and
    /// both mean this: running, and nothing read at the last stop left over.
    #[test]
    fn resuming_forgets_what_the_last_stop_read() {
        let mut state = DebugState {
            attached: true,
            reason: Some(StopReason::Breakpoint),
            stack: vec![StackFrame {
                level: 0,
                function: "main".into(),
                file: None,
                line: Some(3),
                address: "0x0".into(),
            }],
            variables: vec![Variable {
                name: "n".into(),
                value: "3".into(),
                kind: None,
                handle: None,
                children: 0,
            }],
            frame: 1,
            ..DebugState::default()
        };
        state.resumed();
        assert!(state.running);
        assert!(state.stack.is_empty() && state.variables.is_empty());
        assert_eq!(state.reason, None);
        assert!(state.attached, "attached is not resuming's to change");
        assert_eq!(state.frame, 1, "nor is the selected frame");
    }

    /// A stop is read at the innermost frame, so the marker that says whose
    /// variables are shown goes back there — whichever frame was chosen at
    /// the last stop, whose variables are gone.
    #[test]
    fn halting_selects_the_innermost_frame() {
        let mut state = DebugState {
            attached: true,
            running: true,
            frame: 2,
            ..DebugState::default()
        };
        state.halted(StopReason::Step);
        assert!(state.stopped(), "at rest, attached, not over");
        assert_eq!(state.reason, Some(StopReason::Step));
        assert_eq!(state.frame, 0);
    }

    /// A breakpoint reported again under its number replaces the one it was;
    /// one the debugger has not numbered is another every time.
    #[test]
    fn a_breakpoint_is_known_again_by_its_number() {
        let at = |number: Option<u32>, line: u32| Breakpoint {
            number,
            file: "src/main.rs".into(),
            line,
            requested: None,
            verified: true,
            reason: None,
            enabled: true,
        };
        let mut state = DebugState::default();
        for (number, line) in [
            (Some(1), 10),
            (Some(2), 20),
            (Some(1), 12),
            (None, 30),
            (None, 30),
        ] {
            state.record_breakpoint(at(number, line));
        }
        let placed: Vec<(Option<u32>, u32)> = state
            .breakpoints
            .iter()
            .map(|b| (b.number, b.line))
            .collect();
        assert_eq!(
            placed,
            [(Some(1), 12), (Some(2), 20), (None, 30), (None, 30)]
        );
    }
}
