//! Slotboard – 10 clipboard slots.
//! ⌘C + digit stores, ⌘V + digit pastes (Ctrl on Windows and Linux).

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod engine;
mod filter;
mod i18n;
mod platform;
mod settings;
mod state;
mod tray;
mod ui;

use std::sync::mpsc;
use std::sync::Arc;

use tauri::RunEvent;

use crate::engine::Msg;
use crate::platform::Permission;
use crate::state::Core;

fn main() {
    let core = Arc::new(Core::new());
    let (tx, rx) = mpsc::channel::<Msg>();
    if let Ok(mut guard) = core.tx.lock() {
        *guard = Some(tx.clone());
    }

    let builder = tauri::Builder::default();

    // A second start (e.g. double-clicking the app again) opens the help window.
    #[cfg(not(target_os = "macos"))]
    let builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
        ui::open_setup(app);
    }));

    let setup_core = core.clone();
    let app = builder
        .plugin(tauri_plugin_autostart::Builder::new().args(["--autostart"]).build())
        .manage(core)
        .invoke_handler(tauri::generate_handler![
            commands::get_state,
            commands::request_permission,
            commands::open_permission_settings,
            commands::restart_app,
            commands::set_autostart,
            commands::finish_setup,
            commands::close_window,
            commands::open_website,
            commands::clear_slots,
        ])
        .setup(move |app| {
            let handle = app.handle().clone();
            let core = setup_core;

            let settings = settings::load(&handle);
            platform::set_paused(settings.paused);
            if let Ok(mut status) = core.status.lock() {
                status.paused = settings.paused;
                status.permission = platform::permission();
                status.notice = platform::notice().map(str::to_string);
            }
            let onboarding_done = settings.onboarding_done;
            if let Ok(mut guard) = core.settings.lock() {
                *guard = settings;
            }

            let started_at_login = std::env::args().any(|a| a == "--autostart");
            let permission_ok = platform::permission() == Permission::Granted;
            let show_window = !onboarding_done || !permission_ok || !started_at_login;

            // Slotboard lives in the menu bar: no Dock icon unless a window is open.
            #[cfg(target_os = "macos")]
            app.set_activation_policy(if show_window {
                tauri::ActivationPolicy::Regular
            } else {
                tauri::ActivationPolicy::Accessory
            });

            tray::create(&handle)?;
            engine::spawn(handle.clone(), core.clone(), rx, tx);

            if show_window {
                ui::open_setup(&handle);
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building Slotboard");

    app.run(|_app, event| match event {
        // Closing the window must not quit the app – it keeps running in the menu bar.
        RunEvent::ExitRequested { api, code, .. } => {
            if code.is_none() {
                api.prevent_exit();
            }
        }
        #[cfg(target_os = "macos")]
        RunEvent::Reopen { .. } => ui::open_setup(_app),
        _ => {}
    });
}
