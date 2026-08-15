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

/// What the workbench remembers, and where it keeps it.
pub mod workbench {
    pub const RECENTS: &str = "recent_projects";
    pub const FORGET_RECENT: &str = "forget_recent";
    pub const STORAGE_LOCATION: &str = "storage_location";
    pub const RELOCATE: &str = "relocate_storage";
    /// How much disk the data directory is using — what decides whether to
    /// move it.
    pub const FOOTPRINT: &str = "storage_footprint";
    pub const PROXY: &str = "proxy_setting";
    pub const SET_PROXY: &str = "set_proxy_setting";
    pub const KEYBINDS: &str = "keybinds";
    /// Editor tabs per project, and the assistant profile — both moved out of
    /// the WebView's storage, which no data-directory relocation carries.
    pub const RECORD_TABS: &str = "record_tabs";
    pub const PROJECT_TABS: &str = "project_tabs";
    pub const ASSISTANT: &str = "assistant_choice";
    pub const SET_ASSISTANT: &str = "set_assistant_choice";
    pub const SET_KEYBIND: &str = "set_keybind";
    /// Modal editing on or off, remembered across launches and shared by
    /// every window.
    pub const VIM: &str = "vim_enabled";
    pub const SET_VIM: &str = "set_vim";
    pub const UPDATE: &str = "check_update";
    pub const OPEN_URL: &str = "open_url";
}

/// Hardware the workbench knows about.
pub mod catalog {
    pub const CHIPS: &str = "chip_catalogue";
    pub const BOARDS: &str = "board_catalogue";
    pub const PROBLEMS: &str = "catalog_problems";
}

/// The part's pins, and what the source says about them.
pub mod pins {
    pub const REPORT: &str = "pin_report";
}

/// Moving a project from one chip to another.
pub mod migrate {
    pub const PLAN: &str = "plan_migration";
    pub const APPLY: &str = "apply_migration";
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
    /// Hold a serial port open in both directions, for tuning a running
    /// board. Streams like [`RUN`] and never resolves until the link closes.
    pub const LINK: &str = "serial_link";
}

/// Starting a new project.
pub mod wizard {
    pub const OPTIONS: &str = "wizard_options";
    pub const EXPLAIN: &str = "explain_choice";
    pub const PLAN: &str = "plan_new_project";
    pub const CREATE: &str = "create_project";
    /// Scaffold C interop into the open project, either direction.
    pub const C_INTEROP: &str = "scaffold_c_interop";
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
    pub const SHELL_INFO: &str = "terminal_shell_info";
    pub const SET_SHELL: &str = "set_terminal_shell";
    pub const SHELLS: &str = "terminal_shells";
}

/// Looking at and changing the project's files.
pub mod files {
    pub const TREE: &str = "file_tree";
    pub const OPEN: &str = "open_file";
    pub const SAVE: &str = "save_file";
    pub const CREATE: &str = "create_entry";
    pub const DETACH: &str = "open_editor_window";
    /// Bring a detached editor's file back into the main window.
    pub const REATTACH: &str = "reattach_editor_window";
    pub const HIGHLIGHT: &str = "highlight_text";
    pub const OPEN_EXTERNAL: &str = "open_external";
    pub const FORMAT: &str = "format_text";
    pub const SEARCH: &str = "search_project";
}

/// The language server behind the editor.
pub mod lsp {
    pub const START: &str = "lsp_start";
    pub const OPEN: &str = "lsp_open";
    pub const CHANGE: &str = "lsp_change";
    pub const SAVED: &str = "lsp_saved";
    pub const COMPLETE: &str = "lsp_complete";
    pub const HOVER: &str = "lsp_hover";
    pub const DEFINITION: &str = "lsp_definition";
    pub const SIGNATURE: &str = "lsp_signature";
    pub const SEMANTIC: &str = "lsp_semantic";
    pub const ACTIONS: &str = "lsp_code_actions";
    /// Rename a symbol everywhere it is used. Writes the files it changes.
    pub const RENAME: &str = "lsp_rename";
}

/// Breakpoints, stepping, and what the target is doing.
pub mod debug {
    pub const START: &str = "debug_start";
    pub const BREAKPOINT: &str = "debug_breakpoint";
    pub const CONTROL: &str = "debug_control";
    pub const FRAME: &str = "debug_frame";
    pub const STOP: &str = "debug_stop";
    pub const READ: &str = "debug_read_memory";
    pub const REGISTERS: &str = "register_map";
    pub const FETCH_SVD: &str = "fetch_svd";
}

/// The workspace's direct dependencies against crates.io.
pub mod crates {
    pub const REPORT: &str = "crate_report";
}

/// Running firmware without hardware: Espressif's QEMU.
pub mod sim {
    pub const PLAN: &str = "plan_simulation";
    pub const RUN: &str = "run_simulation";
    pub const SAVE_BOARD: &str = "save_sim_board";
    /// A line into whatever session is running — QEMU's stdin, or a serial
    /// port rusty holds open. Named for the simulator it was written for; it
    /// is the only write path either has.
    pub const SEND: &str = "sim_send";
    pub const SAVE_TRACE: &str = "save_sim_trace";
    pub const INSTALL: &str = "install_sim_tool";
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
    pub const SET_ZOOM: &str = "window_set_zoom";
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
