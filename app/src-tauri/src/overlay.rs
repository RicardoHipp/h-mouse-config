//! Overlay: after a switch on the mouse, in the tray or by hotkey a small box shows the new DPI, polling
//! rate or profile, and the battery level, on top of everything for a few seconds. Changes made on the
//! page stay quiet.
//! It is a plain layered window instead of a second WebView, which would cost far more memory.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use ab_glyph::{point, Font, FontVec, PxScale, ScaleFont};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State};

use crate::bluetooth::BluetoothHandle;
use crate::mouse::{MouseHandle, MouseState};
use crate::tray::{profile_text, tray_font};

const SETTINGS_FILE: &str = "overlay.json";
const DURATIONS: [u8; 5] = [1, 2, 3, 5, 10];
const SAMPLE_DPI: u32 = 1600;
// The settings of a new profile arrive a moment after its number.
const PROFILE_SETTLE: Duration = Duration::from_secs(5);
const MAIN_PX: f32 = 34.0;
const SMALL_PX: f32 = 18.0;
const SMALL_GAP: f32 = 7.0;
// Space on both sides of the line before the battery, and between symbol and percent.
const BATTERY_GAP: f32 = 14.0;
const ICON_GAP: f32 = 6.0;
const ICON_RATIO: f32 = 1.9;
const PADDING_X: f32 = 18.0;
const PADDING_Y: f32 = 13.0;
const RADIUS: f32 = 10.0;
const MARGIN: f32 = 24.0;
// The ink of the app, nearly opaque so the digits stay readable over bright games.
const BACKGROUND: [u8; 3] = [0x1f, 0x2a, 0xa8];
const BACKGROUND_ALPHA: f32 = 0.92;

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Position {
    #[default]
    TopLeft,
    TopCenter,
    TopRight,
    BottomLeft,
    BottomCenter,
    BottomRight,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", default)]
pub struct OverlaySettings {
    pub enabled: bool,
    pub seconds: u8,
    pub position: Position,
    pub battery: bool,
}

impl Default for OverlaySettings {
    fn default() -> Self {
        Self { enabled: true, seconds: 5, position: Position::TopLeft, battery: true }
    }
}

/// What tells a press of the DPI or polling button apart from other changes.
#[derive(PartialEq)]
struct Seen {
    profile: Option<u8>,
    stages: Vec<u32>,
    active: u8,
    polling: u32,
}

#[derive(Debug, PartialEq)]
enum Shown {
    Value(u32, &'static str),
    Profile(u8),
}

struct Overlay {
    settings: OverlaySettings,
    last: Option<Seen>,
    settle_until: Option<Instant>,
    // A profile whose box is already on screen, so its switch does not show it twice.
    expected_profile: Option<u8>,
}

pub struct OverlayHandle {
    overlay: Mutex<Overlay>,
    // Commands of the page that are still running; their changes are already on screen.
    quiet: AtomicUsize,
}

fn validate_settings(settings: &OverlaySettings) -> Result<(), String> {
    if !DURATIONS.contains(&settings.seconds) {
        return Err(format!("Ungültige Anzeigedauer {} s.", settings.seconds));
    }
    Ok(())
}

/// What a change of the mouse should show. A freshly read mouse shows nothing, and after a profile switch
/// the new stages and polling rate belong to the profile, not to a press of the DPI or polling button.
fn switched(last: &mut Option<Seen>, settle_until: &mut Option<Instant>, state: &MouseState, now: Instant) -> Option<Shown> {
    let seen = (state.connected && state.has_data).then(|| Seen { profile: state.profile, stages: state.stages.clone(), active: state.active_stage, polling: state.polling });
    let settling = settle_until.is_some_and(|until| now < until);
    let shown = match (&*last, &seen) {
        (Some(old), Some(new)) if old.profile != new.profile => {
            *settle_until = Some(now + PROFILE_SETTLE);
            old.profile.and(new.profile).map(Shown::Profile)
        }
        (Some(old), Some(new)) if settling && (old.stages != new.stages || old.active != new.active || old.polling != new.polling) => {
            *settle_until = None;
            None
        }
        // An edited stage list is no press of the DPI button.
        (Some(old), Some(new)) if old.stages == new.stages && old.active != new.active => {
            state.stages.get(state.active_stage as usize).map(|&dpi| Shown::Value(dpi, "DPI"))
        }
        (Some(old), Some(new)) if old.polling != new.polling => Some(Shown::Value(new.polling, "Hz")),
        _ => None,
    };
    *last = seen;
    shown
}

/// Top left corner of the box inside the screen area given as left, top, right, bottom.
fn place(area: [i32; 4], size: (i32, i32), margin: i32, position: Position) -> (i32, i32) {
    use Position::*;
    let [left, top, right, bottom] = area;
    let x = match position {
        TopLeft | BottomLeft => left + margin,
        TopCenter | BottomCenter => left + (right - left - size.0) / 2,
        TopRight | BottomRight => right - margin - size.0,
    };
    let y = match position {
        TopLeft | TopCenter | TopRight => top + margin,
        BottomLeft | BottomCenter | BottomRight => bottom - margin - size.1,
    };
    (x, y)
}

/// White text over the premultiplied box. Returns where the pen ends.
fn draw_text(pixels: &mut [u8], size: (u32, u32), font: &FontVec, text: &str, scale: PxScale, mut pen: f32, baseline: f32) -> f32 {
    let scaled = font.as_scaled(scale);
    for c in text.chars() {
        let id = font.glyph_id(c);
        let glyph = id.with_scale_and_position(scale, point(pen, baseline));
        pen += scaled.h_advance(id);
        let Some(outline) = font.outline_glyph(glyph) else { continue };
        let bounds = outline.px_bounds();
        outline.draw(|x, y, coverage| brighten(pixels, size, bounds.min.x as i32 + x as i32, bounds.min.y as i32 + y as i32, coverage));
    }
    pen
}

/// Lightens one pixel towards white. It stays premultiplied, because white is full in every channel.
fn brighten(pixels: &mut [u8], size: (u32, u32), x: i32, y: i32, coverage: f32) {
    if x < 0 || y < 0 || x >= size.0 as i32 || y >= size.1 as i32 {
        return;
    }
    let at = (y as usize * size.0 as usize + x as usize) * 4;
    for channel in 0..4 {
        let old = pixels[at + channel] as f32;
        pixels[at + channel] = (old + (255.0 - old) * coverage.clamp(0.0, 1.0)) as u8;
    }
}

/// A white rectangle given as left, top, right, bottom, soft where it covers a pixel only in part.
fn fill_rect(pixels: &mut [u8], size: (u32, u32), [left, top, right, bottom]: [f32; 4], strength: f32) {
    for y in top.floor() as i32..bottom.ceil() as i32 {
        let cover_y = (bottom.min(y as f32 + 1.0) - top.max(y as f32)).max(0.0);
        for x in left.floor() as i32..right.ceil() as i32 {
            let cover_x = (right.min(x as f32 + 1.0) - left.max(x as f32)).max(0.0);
            brighten(pixels, size, x, y, cover_x * cover_y * strength);
        }
    }
}

/// A battery standing on the baseline, filled as far as the level. Returns its width with the pole.
fn draw_battery(pixels: &mut [u8], size: (u32, u32), left: f32, baseline: f32, height: f32, level: u8, scale: f32) -> f32 {
    let line = 1.6 * scale;
    let width = height * ICON_RATIO;
    let (top, right) = (baseline - height, left + width);
    for edge in [[left, top, right, top + line], [left, baseline - line, right, baseline], [left, top, left + line, baseline], [right - line, top, right, baseline]] {
        fill_rect(pixels, size, edge, 1.0);
    }
    let pole = height * 0.4;
    fill_rect(pixels, size, [right, top + (height - pole) / 2.0, right + 2.0 * scale, top + (height + pole) / 2.0], 1.0);
    let inset = line + 1.2 * scale;
    let full = width - 2.0 * inset;
    fill_rect(pixels, size, [left + inset, top + inset, left + inset + full * level.min(100) as f32 / 100.0, baseline - inset], 1.0);
    width + 2.0 * scale
}

/// The box as premultiplied BGRA rows from the top, which a layered window takes as they are. The main
/// text is large, the small one follows on the same baseline, like the unit after a number. The battery
/// level comes last, behind a thin line.
fn render(main: &str, small: &str, battery: Option<u8>, scale: f32) -> Option<(u32, u32, Vec<u8>)> {
    let font = tray_font()?;
    let main_scale = PxScale::from(MAIN_PX * scale);
    let small_scale = PxScale::from(SMALL_PX * scale);
    let width_of = |text: &str, scale: PxScale| {
        let scaled = font.as_scaled(scale);
        text.chars().map(|c| scaled.h_advance(font.glyph_id(c))).sum::<f32>()
    };
    // The box hugs the digits instead of the ascent of the font, which leaves room for accents.
    let digit = font.outline_glyph(font.glyph_id('0').with_scale_and_position(main_scale, point(0.0, 0.0)))?;
    let digit_height = -digit.px_bounds().min.y;
    let gap = if small.is_empty() { 0.0 } else { SMALL_GAP * scale };
    let level = battery.map(|level| format!("{level} %"));
    let icon_height = digit_height * SMALL_PX / MAIN_PX * 1.1;
    let battery_width = level.as_ref().map_or(0.0, |text| (2.0 * BATTERY_GAP + 1.0 + 2.0 + ICON_GAP) * scale + icon_height * ICON_RATIO + width_of(text, small_scale));
    let content = width_of(main, main_scale) + gap + width_of(small, small_scale) + battery_width;
    let width = (2.0 * PADDING_X * scale + content).ceil() as u32;
    let height = (2.0 * PADDING_Y * scale + digit_height).ceil() as u32;

    let mut pixels = vec![0u8; width as usize * height as usize * 4];
    let radius = RADIUS * scale;
    for y in 0..height as usize {
        for x in 0..width as usize {
            let dx = (radius - x as f32 - 0.5).max(x as f32 + 0.5 - (width as f32 - radius)).max(0.0);
            let dy = (radius - y as f32 - 0.5).max(y as f32 + 0.5 - (height as f32 - radius)).max(0.0);
            let alpha = BACKGROUND_ALPHA * (radius - (dx * dx + dy * dy).sqrt() + 0.5).clamp(0.0, 1.0);
            let at = (y * width as usize + x) * 4;
            let [red, green, blue] = BACKGROUND.map(|channel| (channel as f32 * alpha) as u8);
            pixels[at..at + 4].copy_from_slice(&[blue, green, red, (255.0 * alpha) as u8]);
        }
    }
    let baseline = PADDING_Y * scale + digit_height;
    let size = (width, height);
    let pen = draw_text(&mut pixels, size, font, main, main_scale, PADDING_X * scale, baseline);
    let pen = draw_text(&mut pixels, size, font, small, small_scale, pen + gap, baseline);
    if let (Some(battery), Some(text)) = (battery, &level) {
        let line = pen + BATTERY_GAP * scale;
        fill_rect(&mut pixels, size, [line, baseline - digit_height, line + scale, baseline], 0.35);
        let icon = line + (1.0 + BATTERY_GAP) * scale;
        let icon_width = draw_battery(&mut pixels, size, icon, baseline, icon_height, battery, scale);
        draw_text(&mut pixels, size, font, text, small_scale, icon + icon_width + ICON_GAP * scale, baseline);
    }
    Some((width, height, pixels))
}

fn show(main: String, small: String, battery: Option<u8>, settings: &OverlaySettings) {
    native::show(main, small, battery.filter(|_| settings.battery), settings.seconds, settings.position);
}

/// The battery level for the box. While Bluetooth carries the mouse, only Bluetooth knows it.
fn battery_level(app: &AppHandle) -> Option<u8> {
    if let Some(bluetooth) = app.try_state::<BluetoothHandle>().map(|handle| handle.state()).filter(|state| state.connected) {
        return bluetooth.battery;
    }
    let state = app.try_state::<MouseHandle>()?.state();
    state.battery.filter(|_| state.connected)
}

#[cfg(windows)]
mod native {
    use std::cell::Cell;
    use std::sync::{Mutex, OnceLock};
    use std::time::{Duration, Instant};

    use windows::core::w;
    use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, SIZE, WPARAM};
    use windows::Win32::Graphics::Gdi::{
        CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetMonitorInfoW, MonitorFromWindow, SelectObject, AC_SRC_ALPHA, AC_SRC_OVER,
        BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION, DIB_RGB_COLORS, MONITORINFO, MONITOR_DEFAULTTOPRIMARY,
    };
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DispatchMessageW, GetForegroundWindow, GetMessageW, KillTimer, PostMessageW, RegisterClassW, SetTimer, SetWindowPos,
        ShowWindow, TranslateMessage, UpdateLayeredWindow, HWND_TOPMOST, MSG, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW, SW_HIDE, ULW_ALPHA, WM_APP,
        WM_TIMER, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
    };

    use super::{place, render, Position, MARGIN};

    const WM_SHOW: u32 = WM_APP + 1;
    const FADE_TIMER: usize = 1;
    const TICK_MS: u32 = 30;
    const FADE: Duration = Duration::from_millis(300);

    struct Request {
        main: String,
        small: String,
        battery: Option<u8>,
        seconds: u8,
        position: Position,
    }

    static REQUEST: Mutex<Option<Request>> = Mutex::new(None);
    // The window handle as a number, because a handle may not cross threads. Zero if it failed.
    static WINDOW: OnceLock<isize> = OnceLock::new();

    thread_local! {
        static HIDE_AT: Cell<Option<Instant>> = const { Cell::new(None) };
    }

    pub fn show(main: String, small: String, battery: Option<u8>, seconds: u8, position: Position) {
        *REQUEST.lock().unwrap() = Some(Request { main, small, battery, seconds, position });
        let window = *WINDOW.get_or_init(start);
        if window != 0 {
            unsafe {
                let _ = PostMessageW(Some(HWND(window as *mut _)), WM_SHOW, WPARAM(0), LPARAM(0));
            }
        }
    }

    /// The window lives on its own thread, whose message loop also runs the fade.
    fn start() -> isize {
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || unsafe {
            let window = create().map_or(0, |window| window.0 as isize);
            let _ = sender.send(window);
            if window == 0 {
                return;
            }
            let mut message = MSG::default();
            while GetMessageW(&mut message, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        });
        receiver.recv().unwrap_or(0)
    }

    unsafe fn create() -> windows::core::Result<HWND> {
        let instance = GetModuleHandleW(None)?;
        let class = WNDCLASSW { lpfnWndProc: Some(window_proc), hInstance: instance.into(), lpszClassName: w!("HMouseConfigOverlay"), ..Default::default() };
        RegisterClassW(&class);
        // Layered for the see-through corners, transparent so that clicks reach the game, no-activate
        // so that the game keeps the focus, tool window so that it stays out of the taskbar and Alt+Tab.
        CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            w!("HMouseConfigOverlay"),
            w!("H-Mouse Config Overlay"),
            WS_POPUP,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance.into()),
            None,
        )
    }

    unsafe extern "system" fn window_proc(window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        match message {
            WM_SHOW => {
                let request = REQUEST.lock().unwrap().take();
                if let Some(request) = request {
                    present(window, &request);
                }
                LRESULT(0)
            }
            WM_TIMER if wparam.0 == FADE_TIMER => {
                tick(window);
                LRESULT(0)
            }
            _ => DefWindowProcW(window, message, wparam, lparam),
        }
    }

    unsafe fn present(window: HWND, request: &Request) {
        // The screen of the window in front, which is the game while one runs.
        let monitor = MonitorFromWindow(GetForegroundWindow(), MONITOR_DEFAULTTOPRIMARY);
        let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
        if !GetMonitorInfoW(monitor, &mut info).as_bool() {
            return;
        }
        let (mut dpi_x, mut dpi_y) = (96, 96);
        let _ = GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y);
        let scale = dpi_x as f32 / 96.0;
        let Some((width, height, pixels)) = render(&request.main, &request.small, request.battery, scale) else { return };
        let area = info.rcWork;
        let (x, y) = place([area.left, area.top, area.right, area.bottom], (width as i32, height as i32), (MARGIN * scale) as i32, request.position);
        if paint(window, (x, y), (width, height), &pixels).is_err() {
            return;
        }
        let _ = SetWindowPos(window, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW);
        HIDE_AT.set(Some(Instant::now() + Duration::from_secs(request.seconds as u64)));
        SetTimer(Some(window), FADE_TIMER, TICK_MS, None);
    }

    fn blend(alpha: u8) -> BLENDFUNCTION {
        BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, BlendFlags: 0, SourceConstantAlpha: alpha, AlphaFormat: AC_SRC_ALPHA as u8 }
    }

    unsafe fn paint(window: HWND, at: (i32, i32), size: (u32, u32), pixels: &[u8]) -> windows::core::Result<()> {
        let header = BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: size.0 as i32,
            // Negative: the rows run from the top.
            biHeight: -(size.1 as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        };
        let info = BITMAPINFO { bmiHeader: header, ..Default::default() };
        let memory = CreateCompatibleDC(None);
        let mut bits = std::ptr::null_mut();
        let result = CreateDIBSection(Some(memory), &info, DIB_RGB_COLORS, &mut bits, None, 0).and_then(|bitmap| {
            std::ptr::copy_nonoverlapping(pixels.as_ptr(), bits as *mut u8, pixels.len());
            let old = SelectObject(memory, bitmap.into());
            let position = POINT { x: at.0, y: at.1 };
            let extent = SIZE { cx: size.0 as i32, cy: size.1 as i32 };
            let result = UpdateLayeredWindow(window, None, Some(&position), Some(&extent), Some(memory), Some(&POINT::default()), COLORREF(0), Some(&blend(255)), ULW_ALPHA);
            SelectObject(memory, old);
            let _ = DeleteObject(bitmap.into());
            result
        });
        let _ = DeleteDC(memory);
        result
    }

    unsafe fn tick(window: HWND) {
        let Some(hide_at) = HIDE_AT.get() else { return };
        let left = hide_at.saturating_duration_since(Instant::now());
        if left.is_zero() {
            let _ = KillTimer(Some(window), FADE_TIMER);
            let _ = ShowWindow(window, SW_HIDE);
            HIDE_AT.set(None);
        } else if left < FADE {
            let alpha = (255.0 * left.as_secs_f32() / FADE.as_secs_f32()) as u8;
            let _ = UpdateLayeredWindow(window, None, None, None, None, None, COLORREF(0), Some(&blend(alpha)), ULW_ALPHA);
        }
    }
}

#[cfg(not(windows))]
mod native {
    pub fn show(_main: String, _small: String, _battery: Option<u8>, _seconds: u8, _position: super::Position) {}
}

fn load_settings(app: &AppHandle) -> Option<OverlaySettings> {
    let dir = app.path().app_data_dir().ok()?;
    let text = std::fs::read_to_string(dir.join(SETTINGS_FILE)).ok()?;
    serde_json::from_str(&text).ok().filter(|settings| validate_settings(settings).is_ok())
}

pub fn create(app: &AppHandle) {
    let settings = load_settings(app).unwrap_or_default();
    app.manage(OverlayHandle { overlay: Mutex::new(Overlay { settings, last: None, settle_until: None, expected_profile: None }), quiet: AtomicUsize::new(0) });
}

/// Runs a command of the page without the box: what the page changes, it shows itself.
pub fn quietly<T>(app: &AppHandle, command: impl FnOnce() -> T) -> T {
    let Some(handle) = app.try_state::<OverlayHandle>() else { return command() };
    handle.quiet.fetch_add(1, Ordering::SeqCst);
    let result = command();
    handle.quiet.fetch_sub(1, Ordering::SeqCst);
    result
}

/// Shows any two texts, for example why a hotkey could not switch the profile.
pub fn show_text(app: &AppHandle, main: String, small: String) {
    let Some(handle) = app.try_state::<OverlayHandle>() else { return };
    let settings = handle.overlay.lock().unwrap().settings.clone();
    if settings.enabled {
        show(main, small, battery_level(app), &settings);
    }
}

pub fn expect_profile(app: &AppHandle, profile: Option<u8>) {
    if let Some(handle) = app.try_state::<OverlayHandle>() {
        handle.overlay.lock().unwrap().expected_profile = profile;
    }
}

pub fn show_profile(app: &AppHandle, profile: u8) {
    show_text(app, profile_text(app, profile), String::new());
}

/// Called with every change of the mouse; shows the box when DPI, polling rate or profile switched.
pub fn mouse_changed(app: &AppHandle, state: &MouseState) {
    let Some(handle) = app.try_state::<OverlayHandle>() else { return };
    let shown = {
        let mut overlay = handle.overlay.lock().unwrap();
        let Overlay { last, settle_until, expected_profile, .. } = &mut *overlay;
        let shown = switched(last, settle_until, state, Instant::now());
        match shown {
            Some(Shown::Profile(profile)) if expected_profile.take() == Some(profile) => None,
            other => other,
        }
    };
    if handle.quiet.load(Ordering::SeqCst) > 0 {
        return;
    }
    match shown {
        Some(Shown::Value(value, unit)) => show_text(app, value.to_string(), unit.to_string()),
        Some(Shown::Profile(profile)) => show_profile(app, profile),
        None => {}
    }
}

#[tauri::command(async)]
pub fn get_overlay_settings(overlay: State<'_, OverlayHandle>) -> OverlaySettings {
    overlay.overlay.lock().unwrap().settings.clone()
}

#[tauri::command(async)]
pub fn save_overlay_settings(app: AppHandle, mouse: State<'_, MouseHandle>, settings: OverlaySettings) -> Result<(), String> {
    validate_settings(&settings)?;
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let text = serde_json::to_string(&settings).map_err(|e| e.to_string())?;
    std::fs::write(dir.join(SETTINGS_FILE), text).map_err(|e| e.to_string())?;
    app.state::<OverlayHandle>().overlay.lock().unwrap().settings = settings.clone();
    // A sample right away shows where the box appears and for how long.
    if settings.enabled {
        let state = mouse.state();
        let dpi = state.stages.get(state.active_stage as usize).copied().filter(|_| state.has_data).unwrap_or(SAMPLE_DPI);
        show(dpi.to_string(), "DPI".to_string(), battery_level(&app), &settings);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mouse(profile: u8, stages: &[u32], active: u8, polling: u32) -> MouseState {
        MouseState { connected: true, has_data: true, profile: Some(profile), stages: stages.to_vec(), active_stage: active, polling, ..Default::default() }
    }

    #[test]
    fn switches_show_their_value_and_reads_stay_quiet() {
        let start = Instant::now();
        let (mut last, mut settle_until) = (None, None);
        let mut check = |state: MouseState, seconds: u64| switched(&mut last, &mut settle_until, &state, start + Duration::from_secs(seconds));
        assert_eq!(check(mouse(0, &[800, 1600], 0, 1000), 0), None, "first read");
        assert_eq!(check(mouse(0, &[800, 1600], 1, 1000), 0), Some(Shown::Value(1600, "DPI")));
        assert_eq!(check(mouse(0, &[800, 1600], 1, 1000), 0), None, "nothing new");
        assert_eq!(check(mouse(0, &[800, 1600], 1, 4000), 0), Some(Shown::Value(4000, "Hz")));
        assert_eq!(check(mouse(0, &[800, 3200], 1, 4000), 0), None, "stage value edited");
        // A profile switch first reports the number, then the settings of the new profile.
        assert_eq!(check(mouse(1, &[800, 3200], 1, 4000), 10), Some(Shown::Profile(1)));
        assert_eq!(check(mouse(1, &[800, 3200], 0, 1000), 11), None, "settings of the new profile");
        assert_eq!(check(mouse(1, &[800, 3200], 1, 1000), 12), Some(Shown::Value(3200, "DPI")), "button after the switch");
        assert_eq!(check(mouse(2, &[800, 3200], 1, 1000), 20), Some(Shown::Profile(2)));
        assert_eq!(check(mouse(2, &[800, 3200], 0, 1000), 30), Some(Shown::Value(800, "DPI")), "same settings, later press");
        assert_eq!(check(MouseState::default(), 40), None, "mouse gone");
        assert_eq!(check(mouse(1, &[400, 1000], 0, 500), 41), None, "mouse back");
    }

    #[test]
    fn box_sits_in_the_chosen_corner() {
        let area = [0, 0, 1920, 1040];
        assert_eq!(place(area, (200, 60), 24, Position::TopLeft), (24, 24));
        assert_eq!(place(area, (200, 60), 24, Position::TopCenter), (860, 24));
        assert_eq!(place(area, (200, 60), 24, Position::BottomRight), (1696, 956));
        assert_eq!(place([1920, 0, 3840, 1080], (200, 60), 24, Position::BottomLeft), (1944, 996));
    }

    #[test]
    fn settings_need_an_offered_duration() {
        assert!(validate_settings(&OverlaySettings::default()).is_ok());
        assert!(validate_settings(&OverlaySettings { seconds: 4, ..Default::default() }).is_err());
        let saved: OverlaySettings = serde_json::from_str(r#"{"enabled":false,"seconds":2,"position":"bottom-right"}"#).unwrap();
        // Files from before the battery switch keep the battery on.
        assert_eq!(saved, OverlaySettings { enabled: false, seconds: 2, position: Position::BottomRight, battery: true });
    }

    #[test]
    fn box_has_round_corners_and_white_digits() {
        let Some((width, height, pixels)) = render("1600", "DPI", None, 1.0) else { return };
        assert_eq!(pixels.len(), (width * height * 4) as usize);
        assert_eq!(pixels[3], 0, "corner is see-through");
        let middle_left = ((height / 2 * width + 3) * 4) as usize;
        assert!(pixels[middle_left + 3] > 200, "edge is filled");
        assert!(pixels.chunks(4).any(|p| p == [255, 255, 255, 255]), "text is drawn");
        let (wide, _, _) = render("1600", "DPI", None, 1.5).unwrap();
        assert!(wide > width, "box grows with the screen scale");
        let (alone, _, _) = render("Profil 2", "", None, 1.0).unwrap();
        let (with_battery, _, _) = render("Profil 2", "", Some(80), 1.0).unwrap();
        assert!(with_battery > alone + 60, "the battery takes its own room");
    }
}
