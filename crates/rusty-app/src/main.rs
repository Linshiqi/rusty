// Release builds must not spawn a console window behind the app; debug builds
// keep it so panics and logs are visible.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod ai;
mod commands;
mod error;
mod flash;
mod state;

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
            commands::memory_report,
            commands::wizard_options,
            commands::explain_choice,
            commands::plan_new_project,
            commands::serial_ports,
            commands::debug_probes,
            commands::plan_flash,
            flash::run_flash,
            flash::stop_flash,
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
        ])
        .run(tauri::generate_context!())
        .expect("rusty failed to start");
}
