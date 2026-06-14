pub mod commands;
pub mod core;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            commands::is_elevated,
            commands::restart_as_admin,
            commands::preview_move,
            commands::start_move,
            commands::cancel_operation,
            commands::list_operations,
            commands::read_operation_log,
            commands::rollback_operation,
            commands::open_path_in_explorer
        ])
        .run(tauri::generate_context!())
        .expect("failed to run Robit Link Mover");
}
