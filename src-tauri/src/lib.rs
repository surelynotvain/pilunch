//! PiLunch — an AI code editor. Rust core (this crate) + TypeScript UI (../src).

mod agent;
mod commands;
mod conversations;
mod error;
mod ext;
mod ext_commands;
mod fs_ops;
mod browser;
mod computer;
mod git;
mod media;
mod process;
mod records;
mod search;
mod settings;
mod state;
mod terminal;
mod util;
mod watcher;
mod workspace;

use tauri::{Manager, RunEvent};

pub fn run() {
    // Resolve the login-shell PATH early, off the main thread (GUI launchers often start
    // apps with a minimal PATH).
    std::thread::spawn(process::resolve_login_path);

    let (config_dir, data_dir) = state::app_dirs();
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        // Reopen the window at its last size and position.
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .manage(state::AppState::new(config_dir, data_dir))
        .invoke_handler(tauri::generate_handler![
            commands::get_settings,
            commands::update_settings,
            commands::set_api_key,
            commands::list_models,
            commands::set_provider_key,
            commands::open_workspace,
            commands::close_workspace,
            commands::current_workspace,
            commands::list_dir,
            commands::read_file,
            commands::write_file,
            commands::create_file,
            commands::create_dir,
            commands::rename_path,
            commands::delete_path,
            commands::quick_open,
            commands::search_text,
            commands::git_status,
            commands::terminal_spawn,
            commands::terminal_write,
            commands::terminal_resize,
            commands::terminal_kill,
            commands::list_conversations,
            commands::create_conversation,
            commands::get_conversation,
            commands::rename_conversation,
            commands::delete_conversation,
            commands::agent_send,
            commands::agent_cancel,
            commands::agent_cancel_all,
            commands::agent_respond,
            commands::agent_running,
            ext_commands::usage_summary,
            ext_commands::clear_usage,
            ext_commands::trace_stats,
            ext_commands::export_traces,
            ext_commands::clear_traces,
            ext_commands::list_skills,
            ext_commands::read_skill,
            ext_commands::save_skill,
            ext_commands::delete_skill,
            ext_commands::list_custom_tools,
            ext_commands::save_custom_tool,
            ext_commands::delete_custom_tool,
            ext_commands::mcp_config,
            ext_commands::save_mcp_config,
            ext_commands::mcp_status,
            ext_commands::list_plugins,
            ext_commands::install_plugin_folder,
            ext_commands::install_plugin_git,
            ext_commands::remove_plugin,
            ext_commands::set_plugin_enabled,
            ext_commands::build_rust_extension,
            ext_commands::browser_action,
            ext_commands::browser_running,
            ext_commands::open_folder,
        ])
        .build(tauri::generate_context!())
        .expect("error while building PiLunch");

    app.run(|handle, event| {
        if let RunEvent::Exit = event {
            let state = handle.state::<state::AppState>();
            state.agent.cancel_all();
            state.terminals.kill_all();
            // Close Firefox cleanly (killing only geckodriver can leave it running).
            tauri::async_runtime::block_on(async {
                let _ = tokio::time::timeout(std::time::Duration::from_secs(3), state.browser.close(&state.http)).await;
                state.mcp.shutdown().await;
            });
        }
    });
}
