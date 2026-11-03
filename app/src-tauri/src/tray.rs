//! Tray icon: shows the active DPI on a background that follows the battery, and offers the profiles.

use std::sync::{Mutex, OnceLock};

use ab_glyph::{point, Font, FontVec, PxScale, ScaleFont};
use serde::{Deserialize, Serialize};
use tauri::image::Image;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, State, Wry};
use tauri_plugin_autostart::ManagerExt;

use crate::bluetooth::{BluetoothHandle, BluetoothState};
use crate::mouse::{MouseCommand, MouseHandle, MouseState};

const TRAY_ID: &str = "main";
const SETTINGS_FILE: &str = "tray.json";
const AUTOSTART_MARKER: &str = "autostart-initialized.txt";
const LANGUAGE_FILE: &str = "language.txt";
const PROFILE_NAMES_FILE: &str = "profiles.json";
const PROFILE_COUNT: usize = 4;
const FONT_PATH: &str = r"C:\Windows\Fonts\segoeuib.ttf";
const NO_BATTERY_COLOR: [u8; 3] = [0x6b, 0x70, 0x80];
const REFERENCE_TEXT: &str = "8,8k";
const MAX_STRETCH: f32 = 2.2;
const EDGE_GAMMA: f32 = 0.7;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TraySettings {
    pub low_below: u8,
    pub mid_below: u8,
    pub low_color: String,
    pub mid_color: String,
    pub high_color: String,
}

impl Default for TraySettings {
    fn default() -> Self {
        Self {
            low_below: 20,
            mid_below: 40,
            low_color: "#d03b3b".to_string(),
            mid_color: "#c98500".to_string(),
            high_color: "#1f8a50".to_string(),
        }
    }
}

struct Tray {
    settings: TraySettings,
    labels: Vec<String>,
    english: bool,
    open_item: MenuItem<Wry>,
    quit_item: MenuItem<Wry>,
    profile_items: Vec<CheckMenuItem<Wry>>,
    last_look: Option<(String, [u8; 3])>,
}

pub struct TrayHandle(Mutex<Tray>);

fn parse_color(hex: &str) -> Option<[u8; 3]> {
    let digits = hex.strip_prefix('#')?;
    if digits.len() != 6 {
        return None;
    }
    let channel = |at: usize| u8::from_str_radix(digits.get(at..at + 2)?, 16).ok();
    Some([channel(0)?, channel(2)?, channel(4)?])
}

fn format_dpi(dpi: u32) -> String {
    let tenths = (dpi + 50) / 100;
    if tenths % 10 == 0 {
        format!("{}k", tenths / 10)
    } else {
        format!("{},{}k", tenths / 10, tenths % 10)
    }
}

fn battery_color(settings: &TraySettings, battery: Option<u8>) -> [u8; 3] {
    let Some(level) = battery else { return NO_BATTERY_COLOR };
    let color = if level < settings.low_below {
        &settings.low_color
    } else if level < settings.mid_below {
        &settings.mid_color
    } else {
        &settings.high_color
    };
    parse_color(color).unwrap_or(NO_BATTERY_COLOR)
}

fn validate_settings(settings: &TraySettings) -> Result<(), String> {
    if settings.low_below >= settings.mid_below || settings.mid_below > 100 {
        return Err("Die untere Schwelle muss kleiner sein als die obere.".to_string());
    }
    let colors = [&settings.low_color, &settings.mid_color, &settings.high_color];
    if colors.iter().any(|color| parse_color(color).is_none()) {
        return Err("Ungültige Farbe.".to_string());
    }
    Ok(())
}

fn active_dpi(state: &MouseState) -> Option<u32> {
    if !state.has_data {
        return None;
    }
    state.stages.get(state.active_stage as usize).copied()
}

fn profile_label(labels: &[String], profile: usize, english: bool) -> String {
    let word = if english { "Profile" } else { "Profil" };
    match labels.get(profile).map(|name| name.trim()) {
        Some(name) if !name.is_empty() => format!("{word} {} – {name}", profile + 1),
        _ => format!("{word} {}", profile + 1),
    }
}

fn tooltip(state: &MouseState, bluetooth: Option<&BluetoothState>, labels: &[String], english: bool) -> String {
    let say = |german: &'static str, translated: &'static str| if english { translated } else { german };
    if let Some(bluetooth) = bluetooth {
        let mut parts = vec!["H-Mouse Config".to_string(), "Bluetooth".to_string()];
        if let Some(level) = bluetooth.battery {
            parts.push(format!("{} {level} %", say("Akku", "Battery")));
        }
        return parts.join(", ");
    }
    if !state.connected {
        return format!("H-Mouse Config: {}", say("nicht verbunden", "not connected"));
    }
    let mut parts = vec!["H-Mouse Config".to_string()];
    if let Some(dpi) = active_dpi(state) {
        parts.push(format!("{dpi} DPI"));
    }
    if let Some(level) = state.battery {
        let charging = if state.charging { say(" (lädt)", " (charging)") } else { "" };
        parts.push(format!("{} {level} %{charging}", say("Akku", "Battery")));
    }
    if let Some(profile) = state.profile {
        parts.push(profile_label(labels, profile as usize, english));
    }
    if !state.online {
        parts.push(say("inaktiv", "inactive").to_string());
    }
    parts.join(", ")
}

fn tray_font() -> Option<&'static FontVec> {
    static FONT: OnceLock<Option<FontVec>> = OnceLock::new();
    FONT.get_or_init(|| FontVec::try_from_vec(std::fs::read(FONT_PATH).ok()?).ok()).as_ref()
}

/// The area the glyphs really cover, relative to pen start and baseline: left, top, right, bottom.
fn ink_box(font: &FontVec, text: &str, scale: PxScale) -> (f32, f32, f32, f32) {
    let scaled = font.as_scaled(scale);
    let mut pen = 0.0;
    let mut area = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for c in text.chars() {
        let id = font.glyph_id(c);
        if let Some(outline) = font.outline_glyph(id.with_scale_and_position(scale, point(pen, 0.0))) {
            let bounds = outline.px_bounds();
            area = (area.0.min(bounds.min.x), area.1.min(bounds.min.y), area.2.max(bounds.max.x), area.3.max(bounds.max.y));
        }
        pen += scaled.h_advance(id);
    }
    area
}

/// Every value gets the same glyph height. Only values wider than the reference are narrowed.
fn render_icon(text: &str, background: [u8; 3], size: u32) -> Vec<u8> {
    let side = size as usize;
    let mut pixels = vec![0u8; side * side * 4];
    let radius = size as f32 / 8.0;
    for y in 0..side {
        for x in 0..side {
            let dx = (radius - x as f32 - 0.5).max(x as f32 + 0.5 - (size as f32 - radius)).max(0.0);
            let dy = (radius - y as f32 - 0.5).max(y as f32 + 0.5 - (size as f32 - radius)).max(0.0);
            if dx * dx + dy * dy <= radius * radius {
                let at = (y * side + x) * 4;
                pixels[at..at + 4].copy_from_slice(&[background[0], background[1], background[2], 255]);
            }
        }
    }
    let Some(font) = tray_font() else { return pixels };
    let luminance = 0.299 * background[0] as f32 + 0.587 * background[1] as f32 + 0.114 * background[2] as f32;
    let ink: u8 = if luminance > 150.0 { 0 } else { 255 };

    // Same fit as the old tray app: the text fills the icon and may be taller than its natural shape.
    let inner = size as f32 - 2.0;
    let probe = PxScale::from(100.0);
    let reference = ink_box(font, REFERENCE_TEXT, probe);
    let shown = ink_box(font, text, probe);
    let reference_width = reference.2 - reference.0;
    let reference_height = reference.3 - reference.1;
    let shown_width = (shown.2 - shown.0).max(1.0);
    let factor_x = (inner / reference_width).min(inner / shown_width);
    let factor_y = (inner / reference_height).min(inner / reference_width * MAX_STRETCH);
    let scale = PxScale { x: 100.0 * factor_x, y: 100.0 * factor_y };
    let baseline = (size as f32 - reference_height * factor_y) / 2.0 - reference.1 * factor_y;
    let mut pen = (size as f32 - shown_width * factor_x) / 2.0 - shown.0 * factor_x;
    let scaled = font.as_scaled(scale);
    for c in text.chars() {
        let id = font.glyph_id(c);
        let glyph = id.with_scale_and_position(scale, point(pen, baseline));
        pen += scaled.h_advance(id);
        let Some(outline) = font.outline_glyph(glyph) else { continue };
        let bounds = outline.px_bounds();
        outline.draw(|x, y, coverage| {
            let px = bounds.min.x as i32 + x as i32;
            let py = bounds.min.y as i32 + y as i32;
            if px < 0 || py < 0 || px >= side as i32 || py >= side as i32 {
                return;
            }
            let at = (py as usize * side + px as usize) * 4;
            for channel in 0..3 {
                let old = pixels[at + channel] as f32;
                // Pulling faint edge pixels up keeps the small digits solid.
                pixels[at + channel] = (old + (ink as f32 - old) * coverage.min(1.0).powf(EDGE_GAMMA)) as u8;
            }
            pixels[at + 3] = 255;
        });
    }
    pixels
}

fn icon_size() -> u32 {
    #[cfg(windows)]
    {
        #[link(name = "user32")]
        extern "system" {
            fn GetSystemMetrics(index: i32) -> i32;
        }
        const SM_CXSMICON: i32 = 49;
        let size = unsafe { GetSystemMetrics(SM_CXSMICON) };
        if size > 0 {
            return size as u32;
        }
    }
    32
}

fn tray_image(text: &str, background: [u8; 3]) -> Image<'static> {
    let size = icon_size();
    Image::new_owned(render_icon(text, background, size), size, size)
}

fn load_file(app: &AppHandle, name: &str) -> Option<String> {
    let dir = app.path().app_data_dir().ok()?;
    std::fs::read_to_string(dir.join(name)).ok()
}

fn load_labels(app: &AppHandle) -> Vec<String> {
    load_file(app, PROFILE_NAMES_FILE).and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default()
}

pub fn show_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// Windows has no official way to show a tray icon outside the overflow area. This uses its
/// undocumented per-program entry, and only once per path, so that a later choice of the user stands.
#[cfg(windows)]
fn promote_once(app: &AppHandle) {
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_SET_VALUE};
    use winreg::RegKey;

    const SETTINGS_KEY: &str = r"Control Panel\NotifyIconSettings";
    const ATTEMPTS: usize = 30;
    const PAUSE: std::time::Duration = std::time::Duration::from_secs(2);

    let Ok(exe) = std::env::current_exe() else { return };
    let exe = exe.display().to_string();
    let Ok(dir) = app.path().app_data_dir() else { return };
    let marker = dir.join("tray-promoted.txt");
    let done = std::fs::read_to_string(&marker).unwrap_or_default();
    if done.lines().any(|line| same_path(line, &exe)) {
        return;
    }
    // Windows creates the entry a little after the icon first appears.
    std::thread::spawn(move || {
        let settings = RegKey::predef(HKEY_CURRENT_USER);
        for _ in 0..ATTEMPTS {
            std::thread::sleep(PAUSE);
            let Ok(list) = settings.open_subkey(SETTINGS_KEY) else { continue };
            for name in list.enum_keys().flatten() {
                let Ok(entry) = list.open_subkey_with_flags(&name, KEY_READ | KEY_SET_VALUE) else { continue };
                let path: String = entry.get_value("ExecutablePath").unwrap_or_default();
                if same_path(&path, &exe) && entry.set_value("IsPromoted", &1u32).is_ok() {
                    let _ = std::fs::create_dir_all(&dir);
                    let _ = std::fs::write(&marker, format!("{done}{exe}\n"));
                    return;
                }
            }
        }
    });
}

fn same_path(left: &str, right: &str) -> bool {
    left.trim().eq_ignore_ascii_case(right.trim())
}

pub fn create(app: &AppHandle) -> tauri::Result<()> {
    let settings = load_file(app, SETTINGS_FILE)
        .and_then(|text| serde_json::from_str::<TraySettings>(&text).ok())
        .filter(|settings| validate_settings(settings).is_ok())
        .unwrap_or_default();
    let labels = load_labels(app);
    let english = load_file(app, LANGUAGE_FILE).is_some_and(|text| text.trim() == "en");

    let open = MenuItem::with_id(app, "open", open_text(english), true, None::<&str>)?;
    let profile_items = (0..PROFILE_COUNT)
        .map(|profile| CheckMenuItem::with_id(app, format!("profile-{profile}"), profile_label(&labels, profile, english), true, false, None::<&str>))
        .collect::<tauri::Result<Vec<_>>>()?;
    let quit = MenuItem::with_id(app, "quit", quit_text(english), true, None::<&str>)?;

    let menu = Menu::new(app)?;
    menu.append(&open)?;
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    for item in &profile_items {
        menu.append(item)?;
    }
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&quit)?;

    TrayIconBuilder::with_id(TRAY_ID)
        .icon(tray_image("-", NO_BATTERY_COLOR))
        .tooltip("H-Mouse Config")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(move |app, event| match event.id().as_ref() {
            "open" => show_window(app),
            "quit" => app.exit(0),
            id => {
                let Some(profile) = id.strip_prefix("profile-").and_then(|n| n.parse::<u8>().ok()) else { return };
                let app = app.clone();
                // Switching reads all settings of the profile, which must not block the menu.
                std::thread::spawn(move || {
                    let mouse = app.state::<MouseHandle>();
                    let _ = mouse.send(MouseCommand::SetProfile { profile });
                    refresh(&app, &mouse.state());
                });
            }
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                show_window(tray.app_handle());
            }
        })
        .build(app)?;

    app.manage(TrayHandle(Mutex::new(Tray { settings, labels, english, open_item: open, quit_item: quit, profile_items, last_look: None })));
    #[cfg(windows)]
    promote_once(app);
    Ok(())
}

/// Autostart is on by default, so it is switched on once. After that the user's choice stands,
/// and only the stored path follows the EXE if it has moved.
pub fn prepare_autostart(app: &AppHandle) {
    let manager = app.autolaunch();
    let Ok(dir) = app.path().app_data_dir() else { return };
    let marker = dir.join(AUTOSTART_MARKER);
    if !marker.exists() {
        if manager.enable().is_ok() {
            let _ = std::fs::create_dir_all(&dir);
            let _ = std::fs::write(&marker, "");
        }
    } else if manager.is_enabled().unwrap_or(false) {
        let _ = manager.enable();
    }
}

#[tauri::command(async)]
pub fn get_autostart(app: AppHandle) -> Result<bool, String> {
    app.autolaunch().is_enabled().map_err(|e| e.to_string())
}

#[tauri::command(async)]
pub fn set_autostart(app: AppHandle, on: bool) -> Result<bool, String> {
    let manager = app.autolaunch();
    if on { manager.enable() } else { manager.disable() }.map_err(|e| e.to_string())?;
    manager.is_enabled().map_err(|e| e.to_string())
}

/// Brings icon, tooltip and profile ticks in line with the mouse.
pub fn refresh(app: &AppHandle, state: &MouseState) {
    let Some(handle) = app.try_state::<TrayHandle>() else { return };
    let Some(icon) = app.tray_by_id(TRAY_ID) else { return };
    // Over Bluetooth the mouse tells only its battery level, so the icon shows that instead of the DPI.
    // Bluetooth wins: after a switch the receiver still reports the mouse for about six seconds.
    let bluetooth = app.try_state::<BluetoothHandle>().map(|handle| handle.state()).filter(|bluetooth| bluetooth.connected);

    // The tray calls below wait for the main thread, and commands on the main thread may wait
    // for this lock. Holding it across those calls would freeze the whole app.
    let (new_look, tip, items) = {
        let mut tray = handle.0.lock().unwrap();
        let (text, battery) = match &bluetooth {
            Some(bluetooth) => (bluetooth.battery.map_or_else(|| "-".to_string(), |level| format!("{level}%")), bluetooth.battery),
            None => (active_dpi(state).map_or_else(|| "-".to_string(), format_dpi), state.battery),
        };
        let look = (text, battery_color(&tray.settings, battery));
        let new_look = (tray.last_look.as_ref() != Some(&look)).then(|| look.clone());
        tray.last_look = Some(look);
        (new_look, tooltip(state, bluetooth.as_ref(), &tray.labels, tray.english), tray.profile_items.clone())
    };
    if let Some((text, background)) = new_look {
        let _ = icon.set_icon(Some(tray_image(&text, background)));
    }
    let _ = icon.set_tooltip(Some(tip));
    for (profile, item) in items.iter().enumerate() {
        let _ = item.set_checked(state.profile == Some(profile as u8));
    }
}

/// Names are saved by the page, so the menu picks them up from there.
pub fn update_labels(app: &AppHandle, labels: Vec<String>) {
    let Some(handle) = app.try_state::<TrayHandle>() else { return };
    handle.0.lock().unwrap().labels = labels;
    apply_texts(&handle);
}

/// Like in refresh, the lock must be gone before the menu is touched.
fn apply_texts(handle: &TrayHandle) {
    let (labels, english, open, quit, items) = {
        let tray = handle.0.lock().unwrap();
        (tray.labels.clone(), tray.english, tray.open_item.clone(), tray.quit_item.clone(), tray.profile_items.clone())
    };
    let _ = open.set_text(open_text(english));
    let _ = quit.set_text(quit_text(english));
    for (profile, item) in items.iter().enumerate() {
        let _ = item.set_text(profile_label(&labels, profile, english));
    }
}

fn open_text(english: bool) -> &'static str {
    if english { "Open" } else { "Öffnen" }
}

fn quit_text(english: bool) -> &'static str {
    if english { "Quit" } else { "Beenden" }
}

#[tauri::command(async)]
pub fn get_language(app: AppHandle) -> Option<String> {
    load_file(&app, LANGUAGE_FILE).map(|text| text.trim().to_string())
}

#[tauri::command(async)]
pub fn set_language(app: AppHandle, mouse: State<'_, MouseHandle>, language: String) -> Result<(), String> {
    if language != "de" && language != "en" {
        return Err(format!("Unknown language {language}."));
    }
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    std::fs::write(dir.join(LANGUAGE_FILE), &language).map_err(|e| e.to_string())?;
    let handle = app.state::<TrayHandle>();
    handle.0.lock().unwrap().english = language == "en";
    apply_texts(&handle);
    refresh(&app, &mouse.state());
    Ok(())
}

#[tauri::command(async)]
pub fn get_tray_settings(tray: State<'_, TrayHandle>) -> TraySettings {
    tray.0.lock().unwrap().settings.clone()
}

#[tauri::command(async)]
pub fn save_tray_settings(app: AppHandle, mouse: State<'_, MouseHandle>, settings: TraySettings) -> Result<(), String> {
    validate_settings(&settings)?;
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let text = serde_json::to_string(&settings).map_err(|e| e.to_string())?;
    std::fs::write(dir.join(SETTINGS_FILE), text).map_err(|e| e.to_string())?;
    app.state::<TrayHandle>().0.lock().unwrap().settings = settings;
    refresh(&app, &mouse.state());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_compare_like_windows_does() {
        assert!(same_path(r"C:\Programmieren\HYPACE.exe", r"c:\programmieren\hypace.exe "));
        assert!(!same_path(r"C:\a\hypace.exe", r"C:\b\hypace.exe"));
    }

    #[test]
    fn dpi_text_stays_short() {
        assert_eq!(format_dpi(800), "0,8k");
        assert_eq!(format_dpi(6400), "6,4k");
        assert_eq!(format_dpi(12000), "12k");
        assert_eq!(format_dpi(40000), "40k");
    }

    #[test]
    fn battery_picks_the_color_by_threshold() {
        let settings = TraySettings::default();
        let color = |text: &str| parse_color(text).unwrap();
        assert_eq!(battery_color(&settings, Some(19)), color(&settings.low_color));
        assert_eq!(battery_color(&settings, Some(20)), color(&settings.mid_color));
        assert_eq!(battery_color(&settings, Some(39)), color(&settings.mid_color));
        assert_eq!(battery_color(&settings, Some(40)), color(&settings.high_color));
        assert_eq!(battery_color(&settings, None), NO_BATTERY_COLOR);
    }

    #[test]
    fn settings_need_ordered_thresholds_and_valid_colors() {
        assert!(validate_settings(&TraySettings::default()).is_ok());
        assert!(validate_settings(&TraySettings { low_below: 40, mid_below: 40, ..Default::default() }).is_err());
        assert!(validate_settings(&TraySettings { mid_below: 101, ..Default::default() }).is_err());
        assert!(validate_settings(&TraySettings { high_color: "grün".to_string(), ..Default::default() }).is_err());
    }

    #[test]
    fn tooltip_over_bluetooth_shows_only_the_battery() {
        let bluetooth = BluetoothState { connected: true, battery: Some(95) };
        assert_eq!(tooltip(&MouseState::default(), Some(&bluetooth), &[], false), "H-Mouse Config, Bluetooth, Akku 95 %");
        assert_eq!(tooltip(&MouseState::default(), Some(&bluetooth), &[], true), "H-Mouse Config, Bluetooth, Battery 95 %");
        assert_eq!(tooltip(&MouseState::default(), None, &[], false), "H-Mouse Config: nicht verbunden");
    }

    #[test]
    fn profile_label_keeps_the_number() {
        let labels = vec!["Büro".to_string(), " ".to_string()];
        assert_eq!(profile_label(&labels, 0, false), "Profil 1 – Büro");
        assert_eq!(profile_label(&labels, 1, false), "Profil 2");
        assert_eq!(profile_label(&labels, 3, false), "Profil 4");
        assert_eq!(profile_label(&labels, 0, true), "Profile 1 – Büro");
    }

    #[test]
    fn icon_shows_background_and_text_inside_its_bounds() {
        for text in ["-", "0,8k", "6,4k", "12k", "12,8k", "40k", "5%", "95%", "100%"] {
            let pixels = render_icon(text, [0x1f, 0x8a, 0x50], 32);
            assert_eq!(pixels.len(), 32 * 32 * 4);
            assert_eq!(&pixels[(32 + 16) * 4..(32 + 16) * 4 + 4], &[0x1f, 0x8a, 0x50, 255]);            if tray_font().is_some() {
                assert!(pixels.chunks(4).any(|p| p[..3] == [255, 255, 255]), "{text}");
            }
        }
    }
}
