//! Commands the setup window can call.

use std::sync::Arc;

use tauri::{AppHandle, Manager, State};
use tauri_plugin_autostart::ManagerExt;

use crate::engine::Msg;
use crate::platform;
use crate::state::Core;
use crate::ui::{self, AppState};

const WEBSITE: &str = "https://github.com/markuskoppeberlin-a11y/slotboard";

#[tauri::command]
pub fn get_state(app: AppHandle) -> AppState {
    ui::app_state(&app)
}

#[tauri::command]
pub fn request_permission(core: State<'_, Arc<Core>>) {
    platform::request_permission();
    core.send(Msg::CheckPermission);
}

#[tauri::command]
pub fn open_permission_settings() {
    platform::open_permission_settings();
}

#[tauri::command]
pub fn restart_app(app: AppHandle) {
    app.restart();
}

#[tauri::command]
pub fn set_autostart(app: AppHandle, enabled: bool) -> Result<bool, String> {
    let manager = app.autolaunch();
    let result = if enabled { manager.enable() } else { manager.disable() };
    result.map_err(|e| e.to_string())?;
    crate::tray::refresh(&app);
    Ok(manager.is_enabled().unwrap_or(enabled))
}

#[tauri::command]
pub fn finish_setup(app: AppHandle, core: State<'_, Arc<Core>>) {
    if let Ok(mut settings) = core.settings.lock() {
        settings.onboarding_done = true;
        crate::settings::save(&app, &settings);
    }
    if let Some(window) = app.get_webview_window(ui::SETUP_LABEL) {
        let _ = window.close();
    }
}

#[tauri::command]
pub fn close_window(app: AppHandle) {
    if let Some(window) = app.get_webview_window(ui::SETUP_LABEL) {
        let _ = window.close();
    }
}

#[tauri::command]
pub fn open_website() {
    platform::open_url(WEBSITE);
}

#[tauri::command]
pub fn clear_slots(core: State<'_, Arc<Core>>) {
    core.send(Msg::ClearAll);
}
