pub mod cloud;
pub mod config;
pub mod i18n;
pub mod provision;
pub mod tunnel;
pub mod vpn;

use config::ConfigManager;
use i18n::I18n;
use std::sync::Mutex;
use tauri::Manager;

/// Shared application state held by Tauri.
pub struct AppState {
    pub config: Mutex<ConfigManager>,
    pub i18n: Mutex<I18n>,
}

#[tauri::command]
fn get_config(state: tauri::State<AppState>) -> Result<config::AppConfig, String> {
    let config = state.config.lock().map_err(|e| e.to_string())?;
    Ok(config.get().clone())
}

#[tauri::command]
fn update_config(
    state: tauri::State<AppState>,
    new_config: config::AppConfig,
) -> Result<(), String> {
    let mut config = state.config.lock().map_err(|e| e.to_string())?;
    config.update(new_config).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
fn get_translation(state: tauri::State<AppState>, key: String) -> Result<String, String> {
    let i18n = state.i18n.lock().map_err(|e| e.to_string())?;
    Ok(i18n.t(&key).to_string())
}

#[tauri::command]
fn set_language(state: tauri::State<AppState>, lang: String) -> Result<(), String> {
    let mut i18n = state.i18n.lock().map_err(|e| e.to_string())?;
    i18n.set_locale(&lang).map_err(|e| e.to_string())?;

    let mut config = state.config.lock().map_err(|e| e.to_string())?;
    let mut app_config = config.get().clone();
    app_config.language = lang;
    config.update(app_config).map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .setup(|app| {
            let app_data_dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
            std::fs::create_dir_all(&app_data_dir).map_err(|e| e.to_string())?;

            let config_mgr = ConfigManager::new(app_data_dir.clone());
            let i18n_instance = i18n::I18n::new(&config_mgr.get().language)
                .map_err(|e| e.to_string())?;

            app.manage(AppState {
                config: Mutex::new(config_mgr),
                i18n: Mutex::new(i18n_instance),
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_config,
            update_config,
            get_translation,
            set_language,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
