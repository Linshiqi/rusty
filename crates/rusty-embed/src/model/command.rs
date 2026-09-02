//! A command about to run, and what it said.

use serde::{Deserialize, Serialize};

/// A command that is about to be run, in full.
///
/// Produced without spawning anything so it can be tested without hardware —
/// and shown to the user verbatim before it runs. Embedded developers reach for
/// the terminal constantly; hiding the command behind a button is how a tool
/// becomes something to work around rather than with.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandPlan {
    pub program: String,
    pub args: Vec<String>,
    /// The whole thing as one copy-pasteable line.
    pub display: String,
    /// Why this tool and these flags, in one sentence.
    pub rationale: String,
    /// Read this before running it. Absent for the ordinary case; present
    /// when the plan is defensible but something about the situation says
    /// it will not do what the user expects — a device that cannot be the
    /// chip this project builds for, for instance.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LogStream {
    Stdout,
    Stderr,
}

/// One line of output from a flash or monitor session.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogLine {
    pub stream: LogStream,
    pub text: String,
    /// Severity parsed out of a defmt or ESP-IDF log line, when present.
    pub level: Option<LogLevel>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}
