//! Command names, shared by both sides of the IPC boundary.
//!
//! A Tauri command's name is a string on the wire. Written twice — once in the
//! backend's handler list, once in the frontend's call site — the two drift,
//! and the failure is a runtime "command not found" rather than a build error.
//!
//! So the names live here and both sides `use` them. `rusty-app` additionally
//! has a test asserting each constant matches the function it names, which
//! turns the remaining risk (a typo in a constant) into a test failure.
//!
//! No dependencies, and nothing but `&'static str`, so this costs the wasm
//! bundle nothing.

#![no_std]

/// Opening and inspecting a project.
pub mod project {
    pub const OPEN: &str = "open_project";
    pub const STATUS: &str = "project_status";
    pub const PATH: &str = "project_path";
    pub const WORKSPACE_REPORT: &str = "workspace_report";
}

/// Hardware the workbench knows about.
pub mod catalog {
    pub const CHIPS: &str = "chip_catalogue";
    pub const BOARDS: &str = "board_catalogue";
    pub const PROBLEMS: &str = "catalog_problems";
}

pub mod toolchain {
    pub const REPORT: &str = "toolchain_report";
}

pub mod memory {
    pub const REPORT: &str = "memory_report";
}

/// Binaries the project has already built.
pub mod firmware {
    pub const LIST: &str = "firmware_list";
}

/// Devices, and getting a binary onto one.
pub mod flash {
    pub const SERIAL_PORTS: &str = "serial_ports";
    pub const DEBUG_PROBES: &str = "debug_probes";
    pub const PLAN: &str = "plan_flash";
    pub const RUN: &str = "run_flash";
    pub const STOP: &str = "stop_flash";
}

/// Starting a new project.
pub mod wizard {
    pub const OPTIONS: &str = "wizard_options";
    pub const EXPLAIN: &str = "explain_choice";
    pub const PLAN: &str = "plan_new_project";
    pub const CREATE: &str = "create_project";
}

/// The terminal: a real shell behind a pseudo-terminal, plus the one-shot
/// runner the panels use to launch a tool without a shell.
pub mod terminal {
    pub const RUN: &str = "run_command";
    pub const OPEN: &str = "terminal_open";
    pub const WRITE: &str = "terminal_write";
    pub const RESIZE: &str = "terminal_resize";
    pub const SCROLL: &str = "terminal_scroll";
    pub const CLOSE: &str = "terminal_close";
}

pub mod features {
    pub const ROWS: &str = "feature_rows";
    pub const IMPACT: &str = "feature_impact";
}

/// Window controls. The app draws its own title bar.
pub mod window {
    pub const MINIMIZE: &str = "window_minimize";
    pub const TOGGLE_MAXIMIZE: &str = "window_toggle_maximize";
    pub const CLOSE: &str = "window_close";
    pub const IS_MAXIMIZED: &str = "window_is_maximized";
}

/// The assistant and its configuration.
pub mod ai {
    pub const ASK: &str = "ai_ask";
    pub const PRESETS: &str = "ai_presets";
    pub const TOOLS: &str = "ai_tools";
    pub const KEY_CONFIGURED: &str = "ai_key_configured";
    pub const STORE_KEY: &str = "ai_store_key";
    pub const DELETE_KEY: &str = "ai_delete_key";
    pub const LIST_MODELS: &str = "ai_list_models";
    pub const CHECK_PROVIDER: &str = "ai_check_provider";
}
