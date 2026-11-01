#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod mouse;
mod tray;

use tauri::{Emitter, Manager, State, WindowEvent};
use tauri_plugin_dialog::DialogExt;

#[tauri::command]
fn get_mouse_state(mouse: State<'_, mouse::MouseHandle>) -> mouse::MouseState {
    mouse.state()
}

#[tauri::command]
fn get_version(app: tauri::AppHandle) -> String {
    app.package_info().version.to_string()
}

const MACRO_FILE: &str = "macros.json";
const PROFILE_NAMES_FILE: &str = "profiles.json";
const START_IN_TRAY: &str = "--tray";

fn data_file(app: &tauri::AppHandle, name: &str) -> Result<std::path::PathBuf, String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join(name))
}

fn load_data(app: &tauri::AppHandle, name: &str) -> Result<Option<String>, String> {
    match std::fs::read_to_string(data_file(app, name)?) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.to_string()),
    }
}

#[tauri::command(async)]
fn load_macros(app: tauri::AppHandle) -> Result<Option<String>, String> {
    load_data(&app, MACRO_FILE)
}

#[tauri::command(async)]
fn save_macros(app: tauri::AppHandle, text: String) -> Result<(), String> {
    std::fs::write(data_file(&app, MACRO_FILE)?, text).map_err(|e| e.to_string())
}

#[tauri::command(async)]
fn load_profile_names(app: tauri::AppHandle) -> Result<Option<String>, String> {
    load_data(&app, PROFILE_NAMES_FILE)
}

#[tauri::command(async)]
fn save_profile_names(app: tauri::AppHandle, text: String) -> Result<(), String> {
    std::fs::write(data_file(&app, PROFILE_NAMES_FILE)?, &text).map_err(|e| e.to_string())?;
    if let Ok(labels) = serde_json::from_str(&text) {
        tray::update_labels(&app, labels);
    }
    Ok(())
}

/// Profile names are free text, but Windows forbids some characters in file names.
fn safe_file_name(name: &str) -> String {
    let cleaned: String = name.chars().map(|c| if c.is_control() || "<>:\"/\\|?*".contains(c) { '_' } else { c }).collect();
    let trimmed = cleaned.trim().trim_end_matches('.');
    format!("{}.bin", if trimmed.is_empty() { "hypace-konfiguration" } else { trimmed })
}

#[tauri::command(async)]
fn mouse_command(mouse: State<'_, mouse::MouseHandle>, command: mouse::MouseCommand) -> Result<(), String> {
    mouse.send(command)
}

#[tauri::command(async)]
fn export_config(app: tauri::AppHandle, mouse: State<'_, mouse::MouseHandle>, name: String) -> Result<Option<String>, String> {
    let bytes = mouse.export_bytes().ok_or("Noch keine Mausdaten gelesen.")?;
    let target = app
        .dialog()
        .file()
        .add_filter("Hypace-Konfiguration", &["bin"])
        .set_file_name(safe_file_name(&name))
        .blocking_save_file();
    let Some(target) = target else { return Ok(None) };
    let path = target.into_path().map_err(|e| e.to_string())?;
    std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
    Ok(Some(path.display().to_string()))
}

/// Colours the native title bar like the paper of the app. Windows 10 ignores this and keeps its own colours.
#[cfg(windows)]
fn paint_title_bar(app: &tauri::App) {
    use std::ffi::c_void;

    #[link(name = "dwmapi")]
    extern "system" {
        fn DwmSetWindowAttribute(window: *mut c_void, attribute: u32, value: *const c_void, size: u32) -> i32;
    }
    const BORDER_COLOR: u32 = 34;
    const CAPTION_COLOR: u32 = 35;
    const TEXT_COLOR: u32 = 36;
    // Windows expects 0x00BBGGRR.
    const PAPER: u32 = 0x00F3_EDE9;
    const INK: u32 = 0x00A8_2A1F;

    let Some(window) = app.get_webview_window("main") else { return };
    let Ok(handle) = window.hwnd() else { return };
    for (attribute, color) in [(CAPTION_COLOR, PAPER), (TEXT_COLOR, INK), (BORDER_COLOR, INK)] {
        unsafe {
            DwmSetWindowAttribute(handle.0 as *mut c_void, attribute, &color as *const u32 as *const c_void, 4);
        }
    }
}

#[tauri::command(async)]
fn import_config(app: tauri::AppHandle, mouse: State<'_, mouse::MouseHandle>) -> Result<Option<String>, String> {
    let source = app.dialog().file().add_filter("Hypace-Konfiguration", &["bin"]).blocking_pick_file();
    let Some(source) = source else { return Ok(None) };
    let path = source.into_path().map_err(|e| e.to_string())?;
    let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
    mouse.send(mouse::MouseCommand::ImportConfig { bytes })?;
    Ok(Some(path.display().to_string()))
}

fn main() {
    tauri::Builder::default()
        // Must come first: a second start only brings the running window to the front.
        .plugin(tauri_plugin_single_instance::init(|app, _, _| tray::show_window(app)))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, Some(vec![START_IN_TRAY])))
        .setup(|app| {
            tray::create(app.handle())?;
            tray::prepare_autostart(app.handle());
            let handle = app.handle().clone();
            let mouse = mouse::start(move |state| {
                tray::refresh(&handle, &state);
                let _ = handle.emit("mouse-state", state);
            });
            app.manage(mouse);
            #[cfg(windows)]
            paint_title_bar(app);
            if let Some(window) = app.get_webview_window("main") {
                let info = app.package_info();
                let _ = window.set_title(&format!("{} {}", info.name, info.version));
            }
            if !std::env::args().any(|arg| arg == START_IN_TRAY) {
                tray::show_window(app.handle());
            }
            Ok(())
        })
        // The app keeps living in the tray, leaving it is in the tray menu.
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![get_version, get_mouse_state,mouse_command, export_config, import_config, load_macros, save_macros, load_profile_names, save_profile_names, tray::get_tray_settings, tray::save_tray_settings, tray::get_autostart, tray::set_autostart, tray::get_language, tray::set_language])
        .run(tauri::generate_context!())
        .expect("failed to run H-Mouse Config");
}

#[cfg(test)]
mod tests {
    use super::safe_file_name;

    #[test]
    fn file_names_stay_valid_on_windows() {
        assert_eq!(safe_file_name("Profil 1 - Büro"), "Profil 1 - Büro.bin");
        assert_eq!(safe_file_name("a/b:c?"), "a_b_c_.bin");
        assert_eq!(safe_file_name(" .. "), "hypace-konfiguration.bin");
    }
}
