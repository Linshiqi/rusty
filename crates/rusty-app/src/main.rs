// Release builds must not spawn a console window behind the app; debug builds
// keep it so panics and logs are visible.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod ai;
mod commands;
mod debug;
mod error;
mod files;
mod flash;
mod lsp;
mod simulate;
mod state;
mod stream;
mod terminal;
mod window;

fn main() {
    // The built-in shell: this same executable, asked to be the terminal's
    // process. Checked before Tauri exists so a shell start costs nothing
    // but the OS exec — which is the point of building it in.
    if std::env::args().nth(1).as_deref() == Some("--builtin-shell") {
        rusty_term::builtin::run();
    }

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
            commands::crate_report,
            commands::chip_catalogue,
            commands::board_catalogue,
            commands::pin_report,
            commands::record_tabs,
            commands::project_tabs,
            commands::assistant_choice,
            commands::set_assistant_choice,
            commands::plan_migration,
            commands::apply_migration,
            commands::catalog_problems,
            commands::toolchain_report,
            commands::firmware_list,
            commands::recent_projects,
            commands::forget_recent,
            commands::storage_location,
            commands::relocate_storage,
            commands::storage_footprint,
            commands::proxy_setting,
            commands::set_proxy_setting,
            commands::check_update,
            commands::open_url,
            commands::keybinds,
            commands::set_keybind,
            commands::vim_enabled,
            commands::set_vim,
            commands::display_locale,
            commands::set_display_locale,
            files::file_tree,
            files::open_file,
            files::save_file,
            files::create_entry,
            files::open_editor_window,
            files::reattach_editor_window,
            files::highlight_text,
            files::open_external,
            files::format_text,
            files::search_project,
            files::replace_in_project,
            files::watch_project,
            lsp::lsp_start,
            lsp::lsp_open,
            lsp::lsp_change,
            lsp::lsp_saved,
            lsp::lsp_complete,
            lsp::lsp_hover,
            lsp::lsp_definition,
            lsp::lsp_signature,
            lsp::lsp_semantic,
            lsp::lsp_code_actions,
            lsp::lsp_rename,
            simulate::plan_simulation,
            simulate::run_simulation,
            simulate::install_sim_tool,
            simulate::save_sim_board,
            simulate::sim_send,
            simulate::save_sim_trace,
            commands::memory_report,
            commands::wizard_options,
            commands::explain_choice,
            commands::plan_new_project,
            commands::serial_ports,
            commands::debug_probes,
            commands::plan_flash,
            debug::debug_start,
            debug::debug_breakpoint,
            debug::debug_control,
            debug::debug_frame,
            debug::debug_stop,
            debug::debug_read_memory,
            debug::register_map,
            debug::fetch_svd,
            flash::run_flash,
            flash::stop_flash,
            flash::serial_link,
            flash::create_project,
            commands::scaffold_c_interop,
            flash::run_command,
            terminal::terminal_open,
            terminal::terminal_write,
            terminal::terminal_resize,
            terminal::terminal_scroll,
            terminal::terminal_close,
            terminal::terminal_shell_info,
            terminal::set_terminal_shell,
            terminal::terminal_shells,
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
            ai::ai_cancel,
            window::window_minimize,
            window::window_toggle_maximize,
            window::window_close,
            window::window_set_zoom,
        ])
        .build(tauri::generate_context!())
        .expect("rusty failed to start")
        .run(|app, event| {
            // Kill the children on the way out. rust-analyzer does not reliably
            // exit when its stdin closes — its proc-macro server keeps the pipe
            // alive — so every exit that skipped this leaked a server holding
            // hundreds of megabytes and the project's target directory. Three
            // of them were found grazing after a day of dev restarts.
            if let tauri::RunEvent::Exit = event {
                use tauri::Manager;
                let state = app.state::<state::AppState>();
                tauri::async_runtime::block_on(async {
                    state.cancel_ask().await;
                    state.stop_watch().await;
                    state.set_lsp(None).await;
                    state.set_terminal(None).await;
                    state.set_debugger(None).await;
                    state.stop_session().await;
                });
            }
        });
}

/// The frontend calls these commands by the names in `rusty-ipc`; this file
/// defines them by Rust function name. Nothing in the language ties the two
/// together, so a rename here is a runtime "command not found" over there —
/// found by whoever next opens that panel, not by CI.
///
/// Naming each handler proves it exists and `stringify!` proves the constant
/// agrees with how it is spelled. The second test below covers the other half:
/// that every handler named here is also in `generate_handler!` above, and
/// vice versa. That used to be left to the first call of the command, which is
/// the same as being left to whoever opens that panel.
#[cfg(test)]
mod wire_names {
    use std::collections::BTreeSet;

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
        use crate::{ai, commands, debug, files, flash, lsp, simulate, terminal, window};

        assert_named! {
            cmd::project::OPEN => commands::open_project,
            cmd::project::STATUS => commands::project_status,
            cmd::project::PATH => commands::project_path,
            cmd::project::WORKSPACE_REPORT => commands::workspace_report,
            cmd::crates::REPORT => commands::crate_report,

            cmd::catalog::CHIPS => commands::chip_catalogue,
            cmd::catalog::BOARDS => commands::board_catalogue,
            cmd::pins::REPORT => commands::pin_report,
            cmd::workbench::RECORD_TABS => commands::record_tabs,
            cmd::workbench::PROJECT_TABS => commands::project_tabs,
            cmd::workbench::VIM => commands::vim_enabled,
            cmd::workbench::SET_VIM => commands::set_vim,
            cmd::workbench::LOCALE => commands::display_locale,
            cmd::workbench::SET_LOCALE => commands::set_display_locale,
            cmd::workbench::ASSISTANT => commands::assistant_choice,
            cmd::workbench::SET_ASSISTANT => commands::set_assistant_choice,
            cmd::migrate::PLAN => commands::plan_migration,
            cmd::migrate::APPLY => commands::apply_migration,
            cmd::catalog::PROBLEMS => commands::catalog_problems,

            cmd::toolchain::REPORT => commands::toolchain_report,
            cmd::firmware::LIST => commands::firmware_list,
            cmd::workbench::RECENTS => commands::recent_projects,
            cmd::workbench::FORGET_RECENT => commands::forget_recent,
            cmd::workbench::STORAGE_LOCATION => commands::storage_location,
            cmd::workbench::RELOCATE => commands::relocate_storage,
            cmd::workbench::FOOTPRINT => commands::storage_footprint,
            cmd::workbench::PROXY => commands::proxy_setting,
            cmd::workbench::SET_PROXY => commands::set_proxy_setting,
            cmd::workbench::UPDATE => commands::check_update,
            cmd::workbench::OPEN_URL => commands::open_url,
            cmd::workbench::KEYBINDS => commands::keybinds,
            cmd::workbench::SET_KEYBIND => commands::set_keybind,
            cmd::files::TREE => files::file_tree,
            cmd::files::OPEN => files::open_file,
            cmd::files::SAVE => files::save_file,
            cmd::files::CREATE => files::create_entry,
            cmd::files::DETACH => files::open_editor_window,
            cmd::files::REATTACH => files::reattach_editor_window,
            cmd::files::HIGHLIGHT => files::highlight_text,
            cmd::files::OPEN_EXTERNAL => files::open_external,
            cmd::files::FORMAT => files::format_text,
            cmd::files::SEARCH => files::search_project,
            cmd::files::REPLACE => files::replace_in_project,
            cmd::files::WATCH => files::watch_project,

            cmd::lsp::START => lsp::lsp_start,
            cmd::lsp::OPEN => lsp::lsp_open,
            cmd::lsp::CHANGE => lsp::lsp_change,
            cmd::lsp::SAVED => lsp::lsp_saved,
            cmd::lsp::COMPLETE => lsp::lsp_complete,
            cmd::lsp::HOVER => lsp::lsp_hover,
            cmd::lsp::DEFINITION => lsp::lsp_definition,
            cmd::lsp::SIGNATURE => lsp::lsp_signature,
            cmd::lsp::SEMANTIC => lsp::lsp_semantic,
            cmd::lsp::ACTIONS => lsp::lsp_code_actions,
            cmd::lsp::RENAME => lsp::lsp_rename,
            cmd::sim::PLAN => simulate::plan_simulation,
            cmd::sim::RUN => simulate::run_simulation,
            cmd::sim::INSTALL => simulate::install_sim_tool,
            cmd::sim::SAVE_BOARD => simulate::save_sim_board,
            cmd::sim::SEND => simulate::sim_send,
            cmd::sim::SAVE_TRACE => simulate::save_sim_trace,
            cmd::memory::REPORT => commands::memory_report,

            cmd::flash::SERIAL_PORTS => commands::serial_ports,
            cmd::flash::DEBUG_PROBES => commands::debug_probes,
            cmd::flash::PLAN => commands::plan_flash,
            cmd::debug::START => debug::debug_start,
            cmd::debug::BREAKPOINT => debug::debug_breakpoint,
            cmd::debug::CONTROL => debug::debug_control,
            cmd::debug::FRAME => debug::debug_frame,
            cmd::debug::STOP => debug::debug_stop,
            cmd::debug::READ => debug::debug_read_memory,
            cmd::debug::REGISTERS => debug::register_map,
            cmd::debug::FETCH_SVD => debug::fetch_svd,
            cmd::flash::RUN => flash::run_flash,
            cmd::flash::STOP => flash::stop_flash,
            cmd::flash::LINK => flash::serial_link,

            cmd::wizard::OPTIONS => commands::wizard_options,
            cmd::wizard::EXPLAIN => commands::explain_choice,
            cmd::wizard::PLAN => commands::plan_new_project,
            cmd::wizard::CREATE => flash::create_project,
            cmd::wizard::C_INTEROP => commands::scaffold_c_interop,
            cmd::terminal::RUN => flash::run_command,
            cmd::terminal::OPEN => terminal::terminal_open,
            cmd::terminal::WRITE => terminal::terminal_write,
            cmd::terminal::RESIZE => terminal::terminal_resize,
            cmd::terminal::SCROLL => terminal::terminal_scroll,
            cmd::terminal::CLOSE => terminal::terminal_close,
            cmd::terminal::SHELL_INFO => terminal::terminal_shell_info,
            cmd::terminal::SET_SHELL => terminal::set_terminal_shell,
            cmd::terminal::SHELLS => terminal::terminal_shells,

            cmd::features::ROWS => commands::feature_rows,
            cmd::features::IMPACT => commands::feature_impact,

            cmd::ai::ASK => ai::ai_ask,
            cmd::ai::CANCEL => ai::ai_cancel,
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
            cmd::window::SET_ZOOM => window::window_set_zoom,
        }
    }

    /// The `module::handler` entries of `generate_handler![ … ]`, off this
    /// file's own source. Read rather than reflected on because the macro
    /// exposes nothing, and a list that is only checked by calling each
    /// command is a list nobody checks.
    fn registered_handlers(source: &str) -> Vec<String> {
        let open = "generate_handler![";
        let start = source.find(open).expect("the handler list") + open.len();
        let end = start + source[start..].find("])").expect("the end of the list");
        source[start..end]
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with("//"))
            .map(|line| line.trim_end_matches(',').to_string())
            .collect()
    }

    /// The `=> module::handler` entries of `assert_named! { … }`, the same way.
    fn named_handlers(source: &str) -> Vec<String> {
        let open = "assert_named! {";
        let start = source.find(open).expect("the wire names") + open.len();
        let end = start
            + source[start..]
                .find("\n        }")
                .expect("the end of the wire names");
        source[start..end]
            .lines()
            .filter_map(|line| line.split_once("=>"))
            .map(|(_, handler)| handler.trim().trim_end_matches(',').to_string())
            .collect()
    }

    /// Two lists of the same handlers, and this is what ties them together.
    /// A handler in one and not the other is either a command the frontend
    /// can never call or a constant with no handler behind it — both found at
    /// runtime by whoever opens that panel, unless found here.
    #[test]
    fn the_handler_list_and_the_wire_names_are_the_same_set() {
        let source = include_str!("main.rs");
        let registered = registered_handlers(source);
        let named = named_handlers(source);

        let registered_set: BTreeSet<&str> = registered.iter().map(String::as_str).collect();
        let named_set: BTreeSet<&str> = named.iter().map(String::as_str).collect();
        assert_eq!(
            registered_set.len(),
            registered.len(),
            "a handler is registered twice: {registered:?}",
        );
        assert_eq!(
            named_set.len(),
            named.len(),
            "a handler is named twice: {named:?}"
        );

        let unnamed: Vec<_> = registered_set.difference(&named_set).collect();
        let unregistered: Vec<_> = named_set.difference(&registered_set).collect();
        assert!(
            unnamed.is_empty() && unregistered.is_empty(),
            "registered but not in rusty-ipc: {unnamed:?}; in rusty-ipc but not registered: \
             {unregistered:?}",
        );
        assert!(
            registered.len() > 80,
            "the scan found the list, not a fragment of it"
        );
    }
}
