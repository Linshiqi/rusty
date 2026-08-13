//! What a debug session looks like from the outside.
//!
//! Plain data, no IO — the frontend `use`s these directly, so what the
//! debugger reports and what the panel draws cannot drift.

use alloc::string::String;
use alloc::vec::Vec;
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
    pub line: u32,
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
}
