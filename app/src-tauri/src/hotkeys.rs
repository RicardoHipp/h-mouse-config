//! Profile hotkeys: key combinations that switch the mouse profile from anywhere, also while the app sits
//! in the tray. Windows hands a registered combination to this app alone, no other program sees it then.

use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State};

use crate::bluetooth::BluetoothHandle;
use crate::mouse::{MouseCommand, MouseHandle};
use crate::{overlay, tray};

const SETTINGS_FILE: &str = "hotkeys.json";
const SLOT_COUNT: usize = 6;
const SLOT_NAMES: [&str; SLOT_COUNT] = ["Nächstes Profil", "Vorheriges Profil", "Profil 1", "Profil 2", "Profil 3", "Profil 4"];
const PROFILE_COUNT: u8 = 4;
const OCCUPIED: &str = "Von einem anderen Programm belegt.";
const CYCLE_DELAYS: [u32; 4] = [500, 1000, 1500, 2000];
const DEFAULT_CYCLE_DELAY: u32 = 1000;
// A profile wanted while the mouse sleeps waits this long for it to wake up.
const WAKE_WAIT: Duration = Duration::from_secs(60);
const WAKE_POLL: Duration = Duration::from_millis(250);
const ASLEEP: &str = "ist inaktiv";

// Virtual key codes of Windows. The page gets the same numbers as keyCode.
const VK_TAB: u32 = 0x09;
const VK_RETURN: u32 = 0x0D;
const VK_ESCAPE: u32 = 0x1B;
const VK_SPACE: u32 = 0x20;
const VK_DELETE: u32 = 0x2E;
const VK_F1: u32 = 0x70;
const VK_F4: u32 = 0x73;
const VK_F9: u32 = 0x78;
const VK_F10: u32 = 0x79;
const VK_F12: u32 = 0x7B;
const MODIFIER_KEYS: [u32; 11] = [0x10, 0x11, 0x12, 0x5B, 0x5C, 0xA0, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5];
// Copy, paste, undo, save, find and the like.
const COMMON_CTRL_LETTERS: &str = "ACFNOPRSTVWXYZ";

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Hotkey {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub win: bool,
    pub key: u32,
    /// The key as the keyboard it was pressed on calls it, only for the page.
    pub name: String,
}

impl Hotkey {
    fn same_keys(&self, other: &Hotkey) -> bool {
        (self.ctrl, self.alt, self.shift, self.win, self.key) == (other.ctrl, other.alt, other.shift, other.win, other.key)
    }

    fn modifiers_are(&self, ctrl: bool, alt: bool, shift: bool, win: bool) -> bool {
        (self.ctrl, self.alt, self.shift, self.win) == (ctrl, alt, shift, win)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", default)]
struct HotkeyFile {
    slots: Vec<Option<Hotkey>>,
    /// How long cycling waits for another press before the mouse switches, in milliseconds.
    cycle_delay: u32,
}

impl Default for HotkeyFile {
    fn default() -> Self {
        Self { slots: Vec::new(), cycle_delay: DEFAULT_CYCLE_DELAY }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HotkeyView {
    hotkey: Option<Hotkey>,
    warnings: Vec<String>,
    error: Option<String>,
}

struct Hotkeys {
    slots: Vec<Option<Hotkey>>,
    errors: Vec<Option<String>>,
    cycle_delay: u32,
}

pub struct HotkeyHandle {
    hotkeys: Mutex<Hotkeys>,
    registrar: native::Registrar,
}

/// Combinations that Windows keeps for itself, or that make no hotkey at all.
fn rejection(hotkey: &Hotkey) -> Option<&'static str> {
    let key = hotkey.key;
    if key == 0 || MODIFIER_KEYS.contains(&key) {
        return Some("Bitte zusätzlich zu Strg, Alt, Umschalt oder Win eine Taste drücken.");
    }
    let reserved = (hotkey.ctrl && hotkey.alt && key == VK_DELETE)
        || (hotkey.ctrl && key == VK_ESCAPE)
        || (hotkey.alt && key == VK_TAB)
        || (hotkey.win && key == 'L' as u32)
        // Windows keeps F12 for debuggers.
        || (hotkey.modifiers_are(false, false, false, false) && key == VK_F12);
    reserved.then_some("Diese Kombination behält Windows sich vor.")
}

/// Combinations that work, but take something away elsewhere. `altgr` is the character that Ctrl+Alt
/// with this key types on the current keyboard, because Windows treats Ctrl+Alt like AltGr.
fn warnings(hotkey: &Hotkey, altgr: Option<char>) -> Vec<String> {
    let key = hotkey.key;
    let letter = char::from_u32(key).filter(char::is_ascii_uppercase);
    let only_alt = hotkey.modifiers_are(false, true, false, false);
    let mut found = Vec::new();
    if !(hotkey.ctrl || hotkey.alt || hotkey.win) {
        found.push("Ohne Strg, Alt oder Win fehlt diese Taste dann in allen Programmen.".to_string());
    }
    if let Some(character) = altgr {
        found.push(format!("Strg+Alt wirkt hier wie AltGr: „{character}“ lässt sich dann nicht mehr tippen."));
    }
    let common_ctrl = hotkey.ctrl && !hotkey.alt && !hotkey.win && (key == VK_TAB || letter.is_some_and(|l| COMMON_CTRL_LETTERS.contains(l)));
    if common_ctrl || (only_alt && [VK_F4, VK_SPACE, VK_ESCAPE, VK_RETURN].contains(&key)) {
        found.push("Windows und viele Programme nutzen diese Kombination selbst.".to_string());
    }
    if only_alt && (key == 'Z' as u32 || [VK_F1, VK_F9, VK_F10].contains(&key)) {
        found.push("Das NVIDIA-Overlay nutzt diese Kombination.".to_string());
    }
    if (only_alt && key == 'R' as u32) || (hotkey.modifiers_are(true, false, true, false) && key == 'O' as u32) {
        found.push("Die AMD-Software nutzt diese Kombination.".to_string());
    }
    if hotkey.modifiers_are(false, false, true, false) && key == VK_TAB {
        found.push("Das Steam-Overlay nutzt diese Kombination.".to_string());
    }
    if hotkey.win {
        found.push("Windows belegt viele Kombinationen mit der Win-Taste, etwa Win+G für die Xbox Game Bar.".to_string());
    }
    found
}

/// Slot 0 goes to the next profile, slot 1 to the previous one, the others straight to profile 1 to 4.
fn target_profile(slot: usize, current: Option<u8>) -> u8 {
    let base = current.unwrap_or(0);
    match slot {
        0 => (base + 1) % PROFILE_COUNT,
        1 => (base + PROFILE_COUNT - 1) % PROFILE_COUNT,
        direct => (direct - 2) as u8,
    }
}

fn views(hotkeys: &Hotkeys) -> Vec<HotkeyView> {
    hotkeys
        .slots
        .iter()
        .zip(&hotkeys.errors)
        .map(|(hotkey, error)| HotkeyView {
            warnings: hotkey.as_ref().map(|hotkey| warnings(hotkey, native::altgr_char(hotkey))).unwrap_or_default(),
            hotkey: hotkey.clone(),
            error: error.clone(),
        })
        .collect()
}

fn load_file(app: &AppHandle) -> HotkeyFile {
    let saved = app.path().app_data_dir().ok().and_then(|dir| std::fs::read_to_string(dir.join(SETTINGS_FILE)).ok());
    let mut file = saved.and_then(|text| serde_json::from_str::<HotkeyFile>(&text).ok()).unwrap_or_default();
    file.slots.resize(SLOT_COUNT, None);
    file.slots = file.slots.into_iter().map(|slot| slot.filter(|hotkey| rejection(hotkey).is_none())).collect();
    if !CYCLE_DELAYS.contains(&file.cycle_delay) {
        file.cycle_delay = DEFAULT_CYCLE_DELAY;
    }
    file
}

fn save_file(app: &AppHandle, slots: &[Option<Hotkey>], cycle_delay: u32) -> Result<(), String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let text = serde_json::to_string(&HotkeyFile { slots: slots.to_vec(), cycle_delay }).map_err(|e| e.to_string())?;
    std::fs::write(dir.join(SETTINGS_FILE), text).map_err(|e| e.to_string())
}

fn notice(app: &AppHandle, german: &str, english: &str) {
    let small = if tray::english(app) { english } else { german };
    let main = if tray::english(app) { "Profile switch" } else { "Profilwechsel" };
    overlay::show_text(app, main.to_string(), small.to_string());
}

/// The reason of a failed switch, short enough for the overlay, in German and English. Unknown
/// messages appear as they are, so that no failure stays without a reason.
fn failure_reason(error: &str) -> (String, String) {
    let known = [
        (ASLEEP, "Maus schläft – wechselt nach dem Bewegen", "mouse asleep – switches once moved"),
        ("Profilwechsel nicht bestätigt", "Maus hat nicht reagiert", "mouse did not respond"),
        ("Profil nicht gewechselt", "Maus hat nicht reagiert", "mouse did not respond"),
        ("konnte nicht gelesen", "Profil nicht gelesen, Maus bewegen", "profile not read, move the mouse"),
        ("nicht verbunden", "Maus nicht verbunden", "mouse not connected"),
        ("Mausverbindung", "Maus nicht verbunden", "mouse not connected"),
    ];
    match known.iter().find(|(part, ..)| error.contains(part)) {
        Some((_, german, english)) => (german.to_string(), english.to_string()),
        None => (error.to_string(), error.to_string()),
    }
}

/// Sends a switch. When the mouse sleeps, returns the profile to send again once it wakes up.
fn switch_to(app: &AppHandle, target: u8, shown: Option<u8>) -> Option<(u8, Instant)> {
    let mouse = app.state::<MouseHandle>();
    // The overlay shows the new profile as soon as the mouse reports it, unless cycling showed it already.
    overlay::expect_profile(app, shown.filter(|&profile| profile == target));
    let result = mouse.send(MouseCommand::SetProfile { profile: target });
    tray::refresh(app, &mouse.state());
    let error = result.err()?;
    overlay::expect_profile(app, None);
    let (german, english) = failure_reason(&error);
    notice(app, &german, &english);
    error.contains(ASLEEP).then(|| (target, Instant::now() + WAKE_WAIT))
}

/// Switches the mouse after the presses. Cycling first only shows each target and waits for the next
/// press, so the mouse reads just the profile it lands on. A direct key switches at once. A profile
/// wanted while the mouse sleeps follows as soon as it wakes up.
fn switch_profiles(app: AppHandle, presses: Receiver<usize>) {
    let mut waiting: Option<(u8, Instant)> = None;
    loop {
        let first = match waiting {
            None => match presses.recv() {
                Ok(slot) => slot,
                Err(_) => return,
            },
            Some((target, until)) => match presses.recv_timeout(WAKE_POLL) {
                Ok(slot) => slot,
                Err(RecvTimeoutError::Timeout) => {
                    let state = app.state::<MouseHandle>().state();
                    if state.online && state.has_data {
                        waiting = switch_to(&app, target, None);
                    } else if Instant::now() >= until {
                        waiting = None;
                    }
                    continue;
                }
                Err(RecvTimeoutError::Disconnected) => return,
            },
        };
        let mouse = app.state::<MouseHandle>();
        let state = mouse.state();
        let bluetooth = app.try_state::<BluetoothHandle>().is_some_and(|handle| handle.state().connected);
        if bluetooth || !state.has_data {
            waiting = None;
            while presses.try_recv().is_ok() {}
            notice(&app, "nur mit Empfänger oder Kabel", "needs receiver or cable");
            continue;
        }
        let delay = Duration::from_millis(app.state::<HotkeyHandle>().hotkeys.lock().unwrap().cycle_delay as u64);
        // Cycling goes on from a profile that still waits for the mouse.
        let current = waiting.take().map(|(target, _)| target).or(state.profile);
        let (mut slot, mut target) = (first, target_profile(first, current));
        let mut shown = None;
        while slot < 2 {
            overlay::show_profile(&app, target);
            shown = Some(target);
            match presses.recv_timeout(delay) {
                Ok(next) => (slot, target) = (next, target_profile(next, Some(target))),
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
        if mouse.state().profile == Some(target) {
            if shown != Some(target) {
                overlay::show_profile(&app, target);
            }
            continue;
        }
        waiting = switch_to(&app, target, shown);
    }
}

pub fn create(app: &AppHandle) {
    let HotkeyFile { slots, cycle_delay } = load_file(app);
    let (presses, receiver) = mpsc::channel();
    let registrar = native::Registrar::start(move |slot| {
        let _ = presses.send(slot);
    });
    let errors = registrar.apply(&slots);
    app.manage(HotkeyHandle { hotkeys: Mutex::new(Hotkeys { slots, errors, cycle_delay }), registrar });
    let app = app.clone();
    std::thread::spawn(move || switch_profiles(app, receiver));
}

#[tauri::command(async)]
pub fn get_hotkeys(handle: State<'_, HotkeyHandle>) -> Vec<HotkeyView> {
    views(&handle.hotkeys.lock().unwrap())
}

#[tauri::command(async)]
pub fn set_hotkey(app: AppHandle, handle: State<'_, HotkeyHandle>, slot: usize, hotkey: Option<Hotkey>) -> Result<Vec<HotkeyView>, String> {
    if slot >= SLOT_COUNT {
        return Err(format!("Unbekannter Platz {slot}."));
    }
    let mut hotkeys = handle.hotkeys.lock().unwrap();
    if let Some(hotkey) = &hotkey {
        if let Some(reason) = rejection(hotkey) {
            return Err(reason.to_string());
        }
        let taken = hotkeys.slots.iter().enumerate().find(|(other, held)| *other != slot && held.as_ref().is_some_and(|held| held.same_keys(hotkey)));
        if let Some((other, _)) = taken {
            return Err(format!("Schon bei „{}“ eingetragen.", SLOT_NAMES[other]));
        }
    }
    let mut slots = hotkeys.slots.clone();
    slots[slot] = hotkey;
    let errors = handle.registrar.apply(&slots);
    if slots[slot].is_some() && errors[slot].is_some() {
        // A combination that another program holds is not taken, the old one comes back.
        hotkeys.errors = handle.registrar.apply(&hotkeys.slots);
        return Err(OCCUPIED.to_string());
    }
    save_file(&app, &slots, hotkeys.cycle_delay)?;
    (hotkeys.slots, hotkeys.errors) = (slots, errors);
    Ok(views(&hotkeys))
}

#[tauri::command(async)]
pub fn get_cycle_delay(handle: State<'_, HotkeyHandle>) -> u32 {
    handle.hotkeys.lock().unwrap().cycle_delay
}

#[tauri::command(async)]
pub fn set_cycle_delay(app: AppHandle, handle: State<'_, HotkeyHandle>, delay: u32) -> Result<(), String> {
    if !CYCLE_DELAYS.contains(&delay) {
        return Err(format!("Ungültige Wartezeit {delay} ms."));
    }
    let mut hotkeys = handle.hotkeys.lock().unwrap();
    save_file(&app, &hotkeys.slots, delay)?;
    hotkeys.cycle_delay = delay;
    Ok(())
}

/// While the page records a combination, the registered ones would never reach it.
#[tauri::command(async)]
pub fn pause_hotkeys(handle: State<'_, HotkeyHandle>, paused: bool) {
    let errors = handle.registrar.pause(paused);
    if !paused {
        handle.hotkeys.lock().unwrap().errors = errors;
    }
}

#[cfg(windows)]
mod native {
    use std::sync::mpsc::{self, Sender};

    use windows::Win32::Foundation::{LPARAM, WPARAM};
    use windows::Win32::System::Threading::GetCurrentThreadId;
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        GetKeyboardLayout, MapVirtualKeyExW, RegisterHotKey, ToUnicodeEx, UnregisterHotKey, MAPVK_VK_TO_VSC, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, GetMessageW, GetWindowThreadProcessId, PeekMessageW, PostThreadMessageW, MSG, PM_NOREMOVE, WM_APP, WM_HOTKEY,
    };

    use super::{Hotkey, OCCUPIED};

    const WM_WAKE: u32 = WM_APP + 1;

    enum Request {
        Apply(Vec<Option<Hotkey>>),
        Pause(bool),
    }

    /// Hotkeys belong to the thread that registers them, so one thread does all of it and waits for them.
    pub struct Registrar {
        requests: Sender<(Request, Sender<Vec<Option<String>>>)>,
        thread: u32,
    }

    impl Registrar {
        pub fn start(on_press: impl Fn(usize) + Send + 'static) -> Self {
            let (requests, receiver) = mpsc::channel::<(Request, Sender<Vec<Option<String>>>)>();
            let (ready, thread) = mpsc::channel();
            std::thread::spawn(move || unsafe {
                let mut message = MSG::default();
                // Gives the thread its message queue before anyone posts to it.
                let _ = PeekMessageW(&mut message, None, 0, 0, PM_NOREMOVE);
                let _ = ready.send(GetCurrentThreadId());
                let mut active: Vec<Option<Hotkey>> = Vec::new();
                while GetMessageW(&mut message, None, 0, 0).as_bool() {
                    match message.message {
                        // The id is the slot plus one, because zero is no valid id.
                        WM_HOTKEY => on_press(message.wParam.0.saturating_sub(1)),
                        WM_WAKE => {
                            while let Ok((request, reply)) = receiver.try_recv() {
                                unregister(active.len());
                                let paused = match request {
                                    Request::Apply(slots) => {
                                        active = slots;
                                        false
                                    }
                                    Request::Pause(pause) => pause,
                                };
                                let errors = if paused { vec![None; active.len()] } else { register(&active) };
                                let _ = reply.send(errors);
                            }
                        }
                        _ => {}
                    }
                }
            });
            Self { requests, thread: thread.recv().unwrap_or(0) }
        }

        fn ask(&self, request: Request, slots: usize) -> Vec<Option<String>> {
            let (reply, answer) = mpsc::channel();
            let sent = self.requests.send((request, reply)).is_ok() && unsafe { PostThreadMessageW(self.thread, WM_WAKE, WPARAM(0), LPARAM(0)) }.is_ok();
            let errors = if sent { answer.recv().ok() } else { None };
            errors.unwrap_or_else(|| vec![None; slots])
        }

        pub fn apply(&self, slots: &[Option<Hotkey>]) -> Vec<Option<String>> {
            self.ask(Request::Apply(slots.to_vec()), slots.len())
        }

        pub fn pause(&self, paused: bool) -> Vec<Option<String>> {
            self.ask(Request::Pause(paused), super::SLOT_COUNT)
        }
    }

    unsafe fn register(slots: &[Option<Hotkey>]) -> Vec<Option<String>> {
        slots
            .iter()
            .enumerate()
            .map(|(slot, hotkey)| {
                let hotkey = hotkey.as_ref()?;
                // Holding the keys down switches only once.
                let mut modifiers = MOD_NOREPEAT;
                for (on, flag) in [(hotkey.ctrl, MOD_CONTROL), (hotkey.alt, MOD_ALT), (hotkey.shift, MOD_SHIFT), (hotkey.win, MOD_WIN)] {
                    if on {
                        modifiers = modifiers | flag;
                    }
                }
                RegisterHotKey(None, slot as i32 + 1, modifiers, hotkey.key).err().map(|_| OCCUPIED.to_string())
            })
            .collect()
    }

    unsafe fn unregister(count: usize) {
        for slot in 0..count {
            let _ = UnregisterHotKey(None, slot as i32 + 1);
        }
    }

    /// The character that Ctrl+Alt with this key types on the keyboard of the window in front.
    pub fn altgr_char(hotkey: &Hotkey) -> Option<char> {
        if !(hotkey.ctrl && hotkey.alt) || hotkey.win {
            return None;
        }
        unsafe {
            let layout = GetKeyboardLayout(GetWindowThreadProcessId(GetForegroundWindow(), None));
            let scan = MapVirtualKeyExW(hotkey.key, MAPVK_VK_TO_VSC, Some(layout));
            let mut keys = [0u8; 256];
            keys[0x11] = 0x80;
            keys[0x12] = 0x80;
            if hotkey.shift {
                keys[0x10] = 0x80;
            }
            let mut text = [0u16; 8];
            // Flag 4 leaves the dead-key state of the keyboard as it is.
            let count = ToUnicodeEx(hotkey.key, scan, &keys, &mut text, 4, Some(layout));
            if count < 1 {
                return None;
            }
            char::from_u32(text[0] as u32).filter(|c| !c.is_control())
        }
    }
}

#[cfg(not(windows))]
mod native {
    use super::Hotkey;

    pub struct Registrar;

    impl Registrar {
        pub fn start(_on_press: impl Fn(usize) + Send + 'static) -> Self {
            Self
        }

        pub fn apply(&self, slots: &[Option<Hotkey>]) -> Vec<Option<String>> {
            vec![None; slots.len()]
        }

        pub fn pause(&self, _paused: bool) -> Vec<Option<String>> {
            vec![None; super::SLOT_COUNT]
        }
    }

    pub fn altgr_char(_hotkey: &Hotkey) -> Option<char> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(ctrl: bool, alt: bool, shift: bool, win: bool, key: u32) -> Hotkey {
        Hotkey { ctrl, alt, shift, win, key, name: String::new() }
    }

    #[test]
    fn next_and_previous_wrap_around() {
        assert_eq!(target_profile(0, Some(0)), 1);
        assert_eq!(target_profile(0, Some(3)), 0);
        assert_eq!(target_profile(1, Some(0)), 3);
        assert_eq!(target_profile(1, None), 3);
        assert_eq!(target_profile(2, Some(3)), 0);
        assert_eq!(target_profile(5, Some(0)), 3);
    }

    #[test]
    fn failures_name_their_reason() {
        let reason = |error: &str| failure_reason(error).0;
        assert_eq!(reason("Die Maus ist inaktiv. Bitte kurz bewegen und erneut versuchen."), "Maus schläft – wechselt nach dem Bewegen");
        assert_eq!(reason("Die Maus hat den Profilwechsel nicht bestätigt."), "Maus hat nicht reagiert");
        assert_eq!(reason("Das Profil konnte nicht gelesen werden. Bitte die Maus kurz bewegen."), "Profil nicht gelesen, Maus bewegen");
        assert_eq!(reason("Die Mausverbindung wurde unterbrochen."), "Maus nicht verbunden");
        assert_eq!(reason("Unbekanntes Profil 7."), "Unbekanntes Profil 7.");
    }

    #[test]
    fn older_files_get_the_default_delay() {
        let file: HotkeyFile = serde_json::from_str(r#"{"slots":[null]}"#).unwrap();
        assert_eq!(file.cycle_delay, DEFAULT_CYCLE_DELAY);
    }

    #[test]
    fn windows_combinations_are_refused() {
        assert!(rejection(&keys(true, true, false, false, VK_DELETE)).is_some());
        assert!(rejection(&keys(true, false, true, false, VK_ESCAPE)).is_some());
        assert!(rejection(&keys(false, true, false, false, VK_TAB)).is_some());
        assert!(rejection(&keys(false, false, false, true, 'L' as u32)).is_some());
        assert!(rejection(&keys(false, false, false, false, VK_F12)).is_some());
        assert!(rejection(&keys(true, true, false, false, 0x11)).is_some(), "modifier alone");
        assert!(rejection(&keys(true, false, false, false, VK_F12)).is_none());
        assert!(rejection(&keys(true, true, false, false, 0x61)).is_none(), "numpad 1");
    }

    #[test]
    fn risky_combinations_are_named() {
        assert!(warnings(&keys(true, true, false, false, 0x61), None).is_empty());
        assert!(warnings(&keys(true, true, false, false, 'Q' as u32), Some('@'))[0].contains("„@“"));
        assert!(warnings(&keys(true, false, false, false, 'C' as u32), None)[0].starts_with("Windows und viele"));
        assert!(warnings(&keys(false, true, false, false, 'Z' as u32), None)[0].contains("NVIDIA"));
        assert!(warnings(&keys(false, false, true, false, VK_TAB), None).iter().any(|w| w.contains("Steam")));
        assert!(warnings(&keys(false, false, false, false, VK_F1 + 4), None)[0].starts_with("Ohne Strg"));
        assert!(warnings(&keys(true, true, false, false, VK_F9), None).is_empty());
    }
}
