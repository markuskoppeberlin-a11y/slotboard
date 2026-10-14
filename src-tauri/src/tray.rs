//! Menu bar (macOS) / tray (Windows, Linux) icon with the slot list.

use std::sync::Arc;

use tauri::image::Image;
use tauri::menu::{CheckMenuItem, IsMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager, Wry};
use tauri_plugin_autostart::ManagerExt;

use crate::engine::Msg;
use crate::filter::label_for_slot;
use crate::i18n;
use crate::platform::{ContentKind, Permission};
use crate::state::{one_line, Core};

const TRAY_ID: &str = "slotboard";

fn icon() -> tauri::Result<Image<'static>> {
    #[cfg(target_os = "macos")]
    let bytes: &[u8] = include_bytes!("../icons/tray-template.png");
    #[cfg(not(target_os = "macos"))]
    let bytes: &[u8] = include_bytes!("../icons/tray-color.png");
    Image::from_bytes(bytes)
}

pub fn create(app: &AppHandle) -> tauri::Result<()> {
    let menu = build_menu(app)?;
    TrayIconBuilder::with_id(TRAY_ID)
        .icon(icon()?)
        .icon_as_template(cfg!(target_os = "macos"))
        .tooltip("Slotboard")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| on_menu(app, event.id().as_ref()))
        .build(app)?;
    Ok(())
}

pub fn refresh(app: &AppHandle) {
    let app2 = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Some(tray) = app2.tray_by_id(TRAY_ID) {
            if let Ok(menu) = build_menu(&app2) {
                let _ = tray.set_menu(Some(menu));
            }
        }
    });
}

fn slot_label(texts: &i18n::MenuTexts, kind: Option<&ContentKind>) -> String {
    match kind {
        None => texts.empty.to_string(),
        Some(ContentKind::Text(t)) => one_line(t, 42),
        Some(ContentKind::Image) => texts.image.to_string(),
        Some(ContentKind::Files(files)) if files.len() == 1 => one_line(&files[0], 42),
        Some(ContentKind::Files(files)) => format!("{} {}", files.len(), texts.files),
        Some(ContentKind::Other) => texts.content.to_string(),
    }
}

fn build_menu(app: &AppHandle) -> tauri::Result<Menu<Wry>> {
    let core = app.state::<Arc<Core>>();
    let texts = i18n::menu(core.lang());
    let status = core.status();

    let mut items: Vec<Box<dyn IsMenuItem<Wry>>> = Vec::new();

    match status.permission {
        Permission::Unsupported => {
            items.push(Box::new(MenuItem::with_id(app, "setup", texts.unsupported, true, None::<&str>)?));
            items.push(Box::new(PredefinedMenuItem::separator(app)?));
        }
        _ if !status.active => {
            items.push(Box::new(MenuItem::with_id(app, "setup", texts.finish_setup, true, None::<&str>)?));
            items.push(Box::new(PredefinedMenuItem::separator(app)?));
        }
        _ => {}
    }

    let mut any_filled = false;
    if let Ok(slots) = core.slots.lock() {
        for (i, slot) in slots.iter().enumerate() {
            let kind = slot.as_ref().map(|s| &s.kind);
            any_filled |= kind.is_some();
            let text = format!("{}    {}", label_for_slot(i as u8), slot_label(&texts, kind));
            items.push(Box::new(MenuItem::with_id(app, format!("slot-{i}"), text, kind.is_some(), None::<&str>)?));
        }
    }
    items.push(Box::new(PredefinedMenuItem::separator(app)?));
    items.push(Box::new(MenuItem::with_id(app, "clear", texts.clear_all, any_filled, None::<&str>)?));
    items.push(Box::new(PredefinedMenuItem::separator(app)?));

    let autostart = app.autolaunch().is_enabled().unwrap_or(false);
    items.push(Box::new(CheckMenuItem::with_id(app, "pause", texts.paused, true, status.paused, None::<&str>)?));
    items.push(Box::new(CheckMenuItem::with_id(app, "autostart", texts.autostart, true, autostart, None::<&str>)?));
    items.push(Box::new(MenuItem::with_id(app, "help", texts.setup, true, None::<&str>)?));
    items.push(Box::new(PredefinedMenuItem::separator(app)?));
    items.push(Box::new(MenuItem::with_id(app, "quit", texts.quit, true, None::<&str>)?));

    let refs: Vec<&dyn IsMenuItem<Wry>> = items.iter().map(|b| b.as_ref()).collect();
    Menu::with_items(app, &refs)
}

fn on_menu(app: &AppHandle, id: &str) {
    let core = app.state::<Arc<Core>>().inner().clone();
    match id {
        "setup" | "help" => crate::ui::open_setup(app),
        "clear" => core.send(Msg::ClearAll),
        "pause" => {
            let paused = !core.status().paused;
            if let Ok(mut settings) = core.settings.lock() {
                settings.paused = paused;
                crate::settings::save(app, &settings);
            }
            core.send(Msg::SetPaused(paused));
        }
        "autostart" => {
            let manager = app.autolaunch();
            let enabled = manager.is_enabled().unwrap_or(false);
            let _ = if enabled { manager.disable() } else { manager.enable() };
            refresh(app);
            crate::ui::emit_state(app);
        }
        "quit" => app.exit(0),
        other => {
            if let Some(index) = other.strip_prefix("slot-").and_then(|n| n.parse::<u8>().ok()) {
                core.send(Msg::CopySlotToClipboard(index));
            }
        }
    }
}
