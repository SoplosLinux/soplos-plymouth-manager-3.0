mod commands;
mod system_theme;

pub use commands::privileged::run_ops_file;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // Must be the first plugin registered, same as the other Soplos
        // Tauri apps: it needs to intercept and exit a second launch before
        // anything else runs.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            use tauri::Manager;
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            system_theme::get_system_theme,
            commands::environment::get_environment_info,
            commands::themes::list_themes,
            commands::themes::get_current_theme,
            commands::themes::read_theme_preview,
            commands::jobs::poll_job,
            commands::actions::apply_theme,
            commands::actions::install_theme,
            commands::actions::remove_themes,
            commands::actions::preview_theme,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Soplos Plymouth Manager");
}
