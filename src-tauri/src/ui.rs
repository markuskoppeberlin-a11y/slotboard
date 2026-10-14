//! The setup / help window and the state it shows.

use std::sync::Arc;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent};
use tauri_plugin_autostart::ManagerExt;

use crate::i18n::Lang;
use crate::state::{Core, SlotInfo, Status};

pub const SETUP_LABEL: &str = "setup";

#[derive(Clone, Serialize)]
pub struct AppState {
    pub platform: &'static str,
    pub lang: Lang,
    pub version: String,
    pub status: Status,
    pub slots: Vec<SlotInfo>,
    pub autostart: bool,
    pub onboarding_done: bool,
}

pub fn app_state(app: &AppHandle) -> AppState {
    let core = app.state::<Arc<Core>>();
    let onboarding_done = core.settings.lock().map(|s| s.onboarding_done).unwrap_or(false);
    AppState {
        platform: if cfg!(target_os = "macos") {
            "macos"
        } else if cfg!(target_os = "windows") {
            "windows"
        } else {
            "linux"
        },
        lang: core.lang(),
        version: app.package_info().version.to_string(),
        status: core.status(),
        slots: core.slot_infos(),
        autostart: app.autolaunch().is_enabled().unwrap_or(false),
        onboarding_done,
    }
}

pub fn emit_state(app: &AppHandle) {
    if app.get_webview_window(SETUP_LABEL).is_some() {
        let _ = app.emit_to(SETUP_LABEL, "state", app_state(app));
    }
}

/// Opens (or brings back) the setup / help window.
pub fn open_setup(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(SETUP_LABEL) {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
        return;
    }

    #[cfg(target_os = "macos")]
    let _ = app.set_activation_policy(tauri::ActivationPolicy::Regular);

    let builder = WebviewWindowBuilder::new(app, SETUP_LABEL, WebviewUrl::App("index.html".into()))
        .title("Slotboard")
        .inner_size(520.0, 600.0)
        .resizable(false)
        .maximizable(false)
        .minimizable(true)
        .center()
        .focused(true)
        .visible(true);

    #[cfg(target_os = "macos")]
    let builder = builder.title_bar_style(tauri::TitleBarStyle::Overlay).hidden_title(true);

    match builder.build() {
        Ok(window) => {
            let _ = window.set_focus();
            let handle = app.clone();
            window.on_window_event(move |event| {
                if let WindowEvent::Destroyed = event {
                    #[cfg(target_os = "macos")]
                    let _ = handle.set_activation_policy(tauri::ActivationPolicy::Accessory);
                    #[cfg(not(target_os = "macos"))]
                    let _ = &handle;
                }
            });
        }
        Err(err) => eprintln!("Slotboard: could not open window: {err}"),
    }
}

/// Brings the setup window to the front (after the permission was granted
/// in System Settings).
pub fn focus_setup(app: &AppHandle) {
    let app2 = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Some(window) = app2.get_webview_window(SETUP_LABEL) {
            let _ = window.show();
            let _ = window.set_focus();
        }
    });
}
