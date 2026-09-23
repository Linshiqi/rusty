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

impl CommandPlan {
    /// A step shown exactly as it runs: the program and its arguments
    /// joined by spaces. A plan that has to quote an argument for a shell,
    /// or shows a tool by a shorter name than the path it runs, builds its
    /// own `display`.
    pub fn new(
        program: impl Into<String>,
        args: Vec<String>,
        rationale: impl Into<String>,
    ) -> Self {
        let program = program.into();
        let display = std::iter::once(program.as_str())
            .chain(args.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join(" ");
        CommandPlan {
            program,
            args,
            display,
            rationale: rationale.into(),
            warning: None,
        }
    }

    /// More arguments on the end of the step — the pin channel's socket,
    /// the monitor, the gdbstub — and on the end of the line it shows, so
    /// the dock never shows a command other than the one that ran.
    pub fn extend_args(&mut self, extra: Vec<String>) {
        self.display = format!("{} {}", self.display, extra.join(" "));
        self.args.extend(extra);
    }
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
