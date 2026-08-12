// Release builds must not spawn a console window behind the app; debug builds
// keep it so panics and logs are visible.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod ai;
mod commands;
mod error;
mod flash;
mod state;
mod terminal;
mod window;

fn main() {
    tauri::Builder::default()
        // The only OS capability the app asks for: picking a workspace folder.
        // Scoped in capabilities/default.json to opening directories, nothing
        // more — the app has no reason to read or write arbitrary paths.
        .plugin(tauri_plugin_dialog::init())
        .manage(state::AppState::default())
        .invoke_handler(tauri::generate_handler![
            commands::open_project,
            commands::project_status,
            commands::project_path,
            commands::workspace_report,
            commands::chip_catalogue,
            commands::board_catalogue,
            commands::catalog_problems,
            commands::toolchain_report,
            commands::firmware_list,
            commands::memory_report,
            commands::wizard_options,
            commands::explain_choice,
            commands::plan_new_project,
            commands::serial_ports,
            commands::debug_probes,
            commands::plan_flash,
            flash::run_flash,
            flash::stop_flash,
            flash::create_project,
            flash::run_command,
            terminal::terminal_open,
            terminal::terminal_write,
            terminal::terminal_resize,
            terminal::terminal_scroll,
            terminal::terminal_close,
            commands::feature_rows,
            commands::feature_impact,
            commands::ai_presets,
            commands::ai_tools,
            commands::ai_key_configured,
            commands::ai_store_key,
            commands::ai_delete_key,
            commands::ai_list_models,
            commands::ai_check_provider,
            ai::ai_ask,
            window::window_minimize,
            window::window_toggle_maximize,
            window::window_close,
            window::window_is_maximized,
        ])
        .run(tauri::generate_context!())
        .expect("rusty failed to start");
}

/// The frontend calls these commands by the names in `rusty-ipc`; this file
/// defines them by Rust function name. Nothing in the language ties the two
/// together, so a rename here is a runtime "command not found" over there —
/// found by whoever next opens that panel, not by CI.
///
/// Naming each handler proves it exists and `stringify!` proves the constant
/// agrees with how it is spelled. What this deliberately does not cover is
/// whether the handler was added to `generate_handler!` above; that failure at
/// least announces itself clearly the first time the command is called.
#[cfg(test)]
mod wire_names {
    use rusty_ipc as cmd;

    macro_rules! assert_named {
        ($($constant:expr => $module:ident::$handler:ident),* $(,)?) => {
            $(
                let _ = $module::$handler;
                assert_eq!(
                    $constant,
                    stringify!($handler),
                    concat!("the wire name and ", stringify!($handler), " have diverged"),
                );
            )*
        };
    }

    #[test]
    fn every_constant_names_a_real_handler() {
        use crate::{ai, commands, flash, terminal, window};

        assert_named! {
            cmd::project::OPEN => commands::open_project,
            cmd::project::STATUS => commands::project_status,
            cmd::project::PATH => commands::project_path,
            cmd::project::WORKSPACE_REPORT => commands::workspace_report,

            cmd::catalog::CHIPS => commands::chip_catalogue,
            cmd::catalog::BOARDS => commands::board_catalogue,
            cmd::catalog::PROBLEMS => commands::catalog_problems,

            cmd::toolchain::REPORT => commands::toolchain_report,
            cmd::firmware::LIST => commands::firmware_list,
            cmd::memory::REPORT => commands::memory_report,

            cmd::flash::SERIAL_PORTS => commands::serial_ports,
            cmd::flash::DEBUG_PROBES => commands::debug_probes,
            cmd::flash::PLAN => commands::plan_flash,
            cmd::flash::RUN => flash::run_flash,
            cmd::flash::STOP => flash::stop_flash,

            cmd::wizard::OPTIONS => commands::wizard_options,
            cmd::wizard::EXPLAIN => commands::explain_choice,
            cmd::wizard::PLAN => commands::plan_new_project,
            cmd::wizard::CREATE => flash::create_project,
            cmd::terminal::RUN => flash::run_command,
            cmd::terminal::OPEN => terminal::terminal_open,
            cmd::terminal::WRITE => terminal::terminal_write,
            cmd::terminal::RESIZE => terminal::terminal_resize,
            cmd::terminal::SCROLL => terminal::terminal_scroll,
            cmd::terminal::CLOSE => terminal::terminal_close,

            cmd::features::ROWS => commands::feature_rows,
            cmd::features::IMPACT => commands::feature_impact,

            cmd::ai::ASK => ai::ai_ask,
            cmd::ai::PRESETS => commands::ai_presets,
            cmd::ai::TOOLS => commands::ai_tools,
            cmd::ai::KEY_CONFIGURED => commands::ai_key_configured,
            cmd::ai::STORE_KEY => commands::ai_store_key,
            cmd::ai::DELETE_KEY => commands::ai_delete_key,
            cmd::ai::LIST_MODELS => commands::ai_list_models,
            cmd::ai::CHECK_PROVIDER => commands::ai_check_provider,

            cmd::window::MINIMIZE => window::window_minimize,
            cmd::window::TOGGLE_MAXIMIZE => window::window_toggle_maximize,
            cmd::window::CLOSE => window::window_close,
            cmd::window::IS_MAXIMIZED => window::window_is_maximized,
        }
    }
}
