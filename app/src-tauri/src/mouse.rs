//! Connection to the Lofree Hypace.
//! Every write is validated first, needs an active mouse and is read back afterwards.

use std::ffi::CString;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use hidapi::{HidApi, HidDevice};
use serde::{Deserialize, Serialize};

const VENDOR_ID: u16 = 0x3554;
const PRODUCT_IDS: [u16; 2] = [0xFB16, 0xFB14];
const REPORT_ID: u8 = 8;
const PACKET_LEN: usize = 16;
const REPORT_LEN: usize = PACKET_LEN + 1;
const CHUNK: usize = 10;

const CMD_ENCRYPTION: u8 = 1;
const CMD_ONLINE: u8 = 3;
const CMD_BATTERY: u8 = 4;
const CMD_WRITE_FLASH: u8 = 7;
const CMD_READ_FLASH: u8 = 8;
const CMD_STATUS_CHANGED: u8 = 10;
const CMD_GET_PROFILE: u8 = 14;
const CMD_SET_PROFILE: u8 = 15;
const PROFILE_COUNT: u8 = 4;
const CMD_READ_VERSION: u8 = 18;
const CMD_SET_LONG_RANGE: u8 = 22;
const CMD_GET_LONG_RANGE: u8 = 23;
const CMD_SET_DONGLE_LIGHT: u8 = 24;
const CMD_GET_DONGLE_LIGHT: u8 = 25;
const CMD_GET_DONGLE_VERSION: u8 = 29;

const STATUS_DPI: u8 = 1;
const STATUS_POLLING: u8 = 2;
const STATUS_PROFILE: u8 = 4;
const STATUS_DPI_EFFECT: u8 = 8;
const STATUS_LIGHT: u8 = 32;
const STATUS_BATTERY: u8 = 64;

const FLASH_SIZE: usize = 8192;
const SETTINGS_READ_END: usize = 256;
const ADDR_POLLING: usize = 0;
const ADDR_STAGE_COUNT: usize = 2;
const ADDR_ACTIVE_STAGE: usize = 4;
const ADDR_LOD: usize = 10;
const ADDR_DPI: usize = 12;
const ADDR_DPI_EFFECT: usize = 76;
const ADDR_KEYS: usize = 96;
const ADDR_LIGHT: usize = 160;
const ADDR_DEBOUNCE: usize = 169;
const ADDR_MOTION_SYNC: usize = 171;
const ADDR_SLEEP: usize = 173;
const ADDR_ANGLE: usize = 175;
const ADDR_RIPPLE: usize = 177;
const ADDR_PERFORMANCE_ON: usize = 181;
const ADDR_PERFORMANCE_TIME: usize = 183;
const ADDR_SHORTCUTS: usize = 256;
const SHORTCUT_SIZE: usize = 32;
const ADDR_MACROS: usize = 768;
const MACRO_SIZE: usize = 384;
const MACRO_EVENTS_OFFSET: usize = 31;
const MAX_MACRO_NAME: usize = 30;
const MAX_MACRO_EVENTS: usize = 70;
const KEY_COUNT: usize = 5;
const MAX_STAGES: usize = 8;
const DPI_STEP: u32 = 50;
const DPI_MIN: u32 = 100;
const DPI_MAX: u32 = 40000;
const DPI_DOUBLED_FROM: u32 = 30100;
const DPI_DOUBLED_STEP: u32 = 100;
const DPI_DOUBLED_FLAGS: u8 = 17;
const DPI_ENTRY_LEN: usize = 4;
const MAX_DEBOUNCE: u8 = 15;
const MAX_LIGHT_MODE: u8 = 5;
const LIGHT_MODE_OFF: u8 = 0;
const LIGHT_REFRESH_PAUSE: Duration = Duration::from_millis(40);
// The receiver can report the old values for a while after a change.
const LIGHT_READ_RETRIES: usize = 20;
const MAX_LIGHT_LEVEL: u8 = 9;
const POLLING_RATES: [u32; 7] = [125, 250, 500, 1000, 2000, 4000, 8000];
const LOD_VALUES: [u8; 3] = [1, 2, 3];
const TIME_VALUES: [u8; 7] = [1, 3, 6, 12, 30, 60, 90];
const KEY_TYPE_DISABLED: u8 = 0;
const KEY_TYPE_MOUSE: u8 = 1;
const KEY_TYPE_DPI: u8 = 2;
const KEY_TYPE_SCROLL: u8 = 3;
const KEY_TYPE_FIRE: u8 = 4;
const KEY_TYPE_SHORTCUT: u8 = 5;
const KEY_TYPE_MACRO: u8 = 6;
const KEY_TYPE_POLLING: u8 = 7;
const LEFT_CLICK: u16 = 0x0100;
const DEFAULT_KEYS: [u16; KEY_COUNT] = [LEFT_CLICK, 0x0200, 0x0400, 0x0800, 0x1000];
const MEDIA_CODES: [u16; 18] = [
    0x0183, 0x00CD, 0x00B5, 0x00B6, 0x00B7, 0x00E2, 0x00E9, 0x00EA, 0x018A, 0x0192, 0x0194, 0x0223, 0x0221, 0x0225,
    0x0224, 0x0226, 0x0227, 0x022A,
];
const HID_KIND_MEDIA: u8 = 2;
const MAX_COMBO_KEYS: usize = 5;
const SHORTCUT_PRESS: u8 = 128;
const SHORTCUT_RELEASE: u8 = 64;
const MACRO_PRESS: u8 = 2;
const MACRO_RELEASE: u8 = 1;
const MAX_FIRE_TIMES: u16 = 3;
const FIRE_INTERVAL: std::ops::RangeInclusive<u16> = 10..=255;
const CYCLE_TIMES: std::ops::RangeInclusive<u8> = 1..=250;
const CYCLE_UNTIL_RELEASED: u8 = 254;
const CYCLE_UNTIL_OTHER: u8 = 255;

const ATTEMPTS: usize = 5;
const REPLY_TIMEOUT: Duration = Duration::from_millis(200);
const IDLE_READ_MS: i32 = 200;
const RECONNECT_DELAY: Duration = Duration::from_secs(3);
const ONLINE_CHECK: Duration = Duration::from_secs(30);
const OFFLINE_CHECK: Duration = Duration::from_secs(3);
const BATTERY_CHECK: Duration = Duration::from_secs(60);

/// Discharge curve as (millivolts, percent): the median voltage per level byte of the mouse over one discharge.
/// That measurement ended at 20 %, so the step down to 3300 mV is an assumption.
const BATTERY_CURVE: [(u32, u32); 10] = [(3300, 0), (3612, 20), (3708, 30), (3808, 40), (3877, 50), (3946, 60), (3984, 70), (4030, 80), (4063, 90), (4164, 100)];
const BATTERY_CATCH_UP_GAP: u8 = 10;

const FOOTER_LEN: usize = 64;
const FOOTER_VENDOR: &[u8] = b"Compx Inc";
const FOOTER_TYPE: &[u8] = b"mouse";
const FOOTER_SENSOR: &[u8] = b"3950";

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MacroEvent {
    pub down: bool,
    pub kind: u8,
    pub value: u16,
    pub delay: u16,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyBinding {
    pub kind: u8,
    pub param: u16,
    pub media: Option<u16>,
    pub combo: Vec<HidKey>,
    pub macro_name: Option<String>,
    pub macro_events: Vec<MacroEvent>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LightState {
    pub mode: u8,
    pub color: [u8; 3],
    pub speed: u8,
    pub brightness: u8,
    pub time: u8,
}

#[derive(Clone, Copy, Debug, Deserialize)]
pub enum Field {
    Lod,
    Debounce,
    Ripple,
    Angle,
    MotionSync,
    Sleep,
    Performance,
    PerformanceTime,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum MouseCommand {
    SetField { field: Field, value: u8 },
    SetPolling { hz: u32 },
    SetStages { stages: Vec<u32>, active: u8 },
    SetLongDistance { on: bool },
    SetLight { mode: u8, color: [u8; 3], speed: u8, brightness: u8 },
    SetKey { key: u8, kind: u8, param: u16 },
    SetMedia { key: u8, code: u16 },
    SetCombo { key: u8, keys: Vec<HidKey> },
    SetMacro { key: u8, name: String, cycle: u8, events: Vec<MacroEvent> },
    ClearMacro { key: u8 },
    ImportConfig { bytes: Vec<u8> },
    SetProfile { profile: u8 },
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct HidKey {
    pub kind: u8,
    pub value: u16,
}

struct Job {
    command: MouseCommand,
    reply: Sender<Result<(), String>>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MouseState {
    pub connected: bool,
    pub online: bool,
    pub has_data: bool,
    pub cid: u8,
    pub mid: u8,
    pub connection: u8,
    pub profile: Option<u8>,
    pub battery: Option<u8>,
    pub charging: bool,
    pub battery_voltage: Option<u32>,
    pub firmware: Option<String>,
    pub dongle_firmware: Option<String>,
    pub stages: Vec<u32>,
    pub active_stage: u8,
    pub polling: u32,
    pub lod: u8,
    pub debounce: u8,
    pub ripple: bool,
    pub angle: bool,
    pub motion_sync: bool,
    pub long_distance: Option<bool>,
    pub performance: bool,
    pub performance_time: u8,
    pub sleep_time: u8,
    pub keys: Vec<KeyBinding>,
    pub light: Option<LightState>,
}

struct Shared {
    state: MouseState,
    flash: Vec<u8>,
}

#[derive(Clone)]
pub struct MouseHandle {
    shared: Arc<Mutex<Shared>>,
    jobs: Sender<Job>,
}

impl MouseHandle {
    pub fn state(&self) -> MouseState {
        self.shared.lock().unwrap().state.clone()
    }

    /// Runs one command on the worker thread and waits for its verified result.
    pub fn send(&self, command: MouseCommand) -> Result<(), String> {
        let (reply, result) = mpsc::channel();
        self.jobs.send(Job { command, reply }).map_err(|_| "Die Mausverbindung läuft nicht.".to_string())?;
        result.recv().map_err(|_| "Die Mausverbindung wurde unterbrochen.".to_string())?
    }

    /// Export layout: flash copy plus a 64 byte footer.
    pub fn export_bytes(&self) -> Option<Vec<u8>> {
        let shared = self.shared.lock().unwrap();
        if !shared.state.has_data {
            return None;
        }
        let mut bytes = shared.flash.clone();
        bytes.resize(FLASH_SIZE + FOOTER_LEN, 0);
        bytes[FLASH_SIZE..FLASH_SIZE + FOOTER_VENDOR.len()].copy_from_slice(FOOTER_VENDOR);
        bytes[FLASH_SIZE + 32..FLASH_SIZE + 32 + FOOTER_TYPE.len()].copy_from_slice(FOOTER_TYPE);
        bytes[FLASH_SIZE + 48..FLASH_SIZE + 48 + FOOTER_SENSOR.len()].copy_from_slice(FOOTER_SENSOR);
        Some(bytes)
    }
}

pub fn start(on_change: impl Fn(MouseState) + Send + 'static) -> MouseHandle {
    let shared = Arc::new(Mutex::new(Shared {
        state: MouseState::default(),
        flash: vec![0xFF; FLASH_SIZE],
    }));
    let (jobs, job_queue) = mpsc::channel();
    let handle = MouseHandle { shared: shared.clone(), jobs };
    thread::spawn(move || {
        let mut worker = Worker { shared, on_change: Box::new(on_change), status_flags: 0, job_queue, battery_display: None };
        worker.run();
    });
    handle
}

/// The device stopped responding at the HID level.
struct DeviceGone;

type Packet = [u8; PACKET_LEN];

struct Worker {
    shared: Arc<Mutex<Shared>>,
    on_change: Box<dyn Fn(MouseState) + Send>,
    status_flags: u8,
    job_queue: Receiver<Job>,
    battery_display: Option<u8>,
}

fn build_packet(command: u8, address: usize, length: u8, payload: &[u8]) -> Packet {
    let mut packet = [0u8; PACKET_LEN];
    packet[0] = command;
    packet[2] = (address >> 8) as u8;
    packet[3] = address as u8;
    packet[4] = length;
    packet[5..5 + payload.len()].copy_from_slice(payload);
    let sum: u32 = packet[..PACKET_LEN - 1].iter().map(|&b| b as u32).sum();
    packet[PACKET_LEN - 1] = 85u8.wrapping_sub(sum as u8).wrapping_sub(REPORT_ID);
    packet
}

fn is_reply(reply: &[u8], request: &Packet) -> bool {
    if reply[0] != request[0] {
        return false;
    }
    let addressed = request[0] == CMD_READ_FLASH || request[0] == CMD_WRITE_FLASH;
    !addressed || (reply[2] == request[2] && reply[3] == request[3])
}

fn validate_field(field: Field, value: u8) -> Result<usize, String> {
    let (address, valid) = match field {
        Field::Lod => (ADDR_LOD, LOD_VALUES.contains(&value)),
        Field::Debounce => (ADDR_DEBOUNCE, value <= MAX_DEBOUNCE),
        Field::Ripple => (ADDR_RIPPLE, value <= 1),
        Field::Angle => (ADDR_ANGLE, value <= 1),
        Field::MotionSync => (ADDR_MOTION_SYNC, value <= 1),
        Field::Sleep => (ADDR_SLEEP, TIME_VALUES.contains(&value)),
        Field::Performance => (ADDR_PERFORMANCE_ON, value <= 1),
        Field::PerformanceTime => (ADDR_PERFORMANCE_TIME, TIME_VALUES.contains(&value)),
    };
    if valid {
        Ok(address)
    } else {
        Err(format!("Ungültiger Wert {value}."))
    }
}

/// The level byte of the mouse is coarse, so the display uses the voltage.
fn voltage_to_level(voltage: u32, charging: bool) -> u8 {
    if voltage >= BATTERY_CURVE[BATTERY_CURVE.len() - 1].0 {
        return if charging { 99 } else { 100 };
    }
    let Some(index) = BATTERY_CURVE.iter().position(|&(step, _)| voltage < step) else {
        return 0;
    };
    if index == 0 {
        return 0;
    }
    let (low_voltage, low_level) = BATTERY_CURVE[index - 1];
    let (high_voltage, high_level) = BATTERY_CURVE[index];
    let share = (voltage - low_voltage) as f64 / (high_voltage - low_voltage) as f64;
    (low_level as f64 + share * (high_level - low_level) as f64).round() as u8
}

/// Voltage sags under load, so the shown value only moves in one step per reading:
/// down while discharging, and up while charging once it lags clearly behind.
fn next_battery_display(previous: Option<u8>, target: u8, raw_level: u8, charging: bool) -> u8 {
    let Some(shown) = previous else { return target };
    if charging {
        if raw_level == 100 {
            100
        } else if target > shown && target - shown > BATTERY_CATCH_UP_GAP {
            shown + 1
        } else {
            shown
        }
    } else if raw_level == 0 {
        0
    } else if target < shown {
        shown - 1
    } else {
        shown
    }
}

fn checksum(bytes: &[u8]) -> u8 {
    let sum: u32 = bytes.iter().map(|&b| b as u32).sum();
    85u8.wrapping_sub(sum as u8)
}

fn key_entry(kind: u8, param: u16) -> [u8; 4] {
    let bytes = [kind, (param >> 8) as u8, param as u8];
    [bytes[0], bytes[1], bytes[2], checksum(&bytes)]
}

fn validate_key(kind: u8, param: u16) -> Result<(), String> {
    let valid = match kind {
        KEY_TYPE_DISABLED | KEY_TYPE_POLLING => param == 0,
        KEY_TYPE_MOUSE => DEFAULT_KEYS.contains(&param),
        KEY_TYPE_DPI => [0x0100, 0x0200, 0x0300].contains(&param),
        KEY_TYPE_SCROLL => [0x0100, 0x0200].contains(&param),
        KEY_TYPE_FIRE => (param & 255) <= MAX_FIRE_TIMES && FIRE_INTERVAL.contains(&(param >> 8)),
        _ => false,
    };
    if valid {
        Ok(())
    } else {
        Err(format!("Ungültige Tastenfunktion {kind}/{param:#06x}."))
    }
}

fn shortcut_bytes_media(code: u16) -> [u8; 8] {
    let (low, high) = (code as u8, (code >> 8) as u8);
    let bytes = [2, SHORTCUT_PRESS | HID_KIND_MEDIA, low, high, SHORTCUT_RELEASE | HID_KIND_MEDIA, low, high];
    [bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], checksum(&bytes)]
}

/// Keys go down in order and come back up in reverse.
fn shortcut_bytes_combo(keys: &[HidKey]) -> Vec<u8> {
    let mut bytes = vec![(2 * keys.len()) as u8];
    for key in keys {
        bytes.extend([SHORTCUT_PRESS | key.kind, key.value as u8, (key.value >> 8) as u8]);
    }
    for key in keys.iter().rev() {
        bytes.extend([SHORTCUT_RELEASE | key.kind, key.value as u8, (key.value >> 8) as u8]);
    }
    bytes.push(checksum(&bytes));
    bytes
}

fn macro_bytes(name: &str, events: &[MacroEvent]) -> Vec<u8> {
    let mut bytes = vec![0xFF; MACRO_EVENTS_OFFSET];
    let name = name.as_bytes();
    bytes[0] = name.len() as u8;
    bytes[1..1 + name.len()].copy_from_slice(name);
    bytes.push(events.len() as u8);
    let mut body = Vec::with_capacity(5 * events.len() + 1);
    for event in events {
        let phase = if event.down { MACRO_PRESS } else { MACRO_RELEASE };
        body.extend([
            (phase << 6) + event.kind,
            event.value as u8,
            (event.value >> 8) as u8,
            (event.delay >> 8) as u8,
            event.delay as u8,
        ]);
    }
    body.push(checksum(&body).wrapping_sub(events.len() as u8));
    bytes.extend(body);
    bytes
}

fn validate_macro(name: &str, cycle: u8, events: &[MacroEvent]) -> Result<(), String> {
    if name.is_empty() || name.len() > MAX_MACRO_NAME {
        return Err(format!("Der Makroname darf 1 bis {MAX_MACRO_NAME} Bytes lang sein."));
    }
    if events.len() > MAX_MACRO_EVENTS {
        return Err(format!("Maximal {MAX_MACRO_EVENTS} Ereignisse pro Makro."));
    }
    if !CYCLE_TIMES.contains(&cycle) && cycle != CYCLE_UNTIL_RELEASED && cycle != CYCLE_UNTIL_OTHER {
        return Err(format!("Ungültige Wiederholung {cycle}."));
    }
    if events.iter().any(|event| event.kind > 15) {
        return Err("Ungültiges Makro-Ereignis.".to_string());
    }
    Ok(())
}

fn validate_combo(keys: &[HidKey]) -> Result<(), String> {
    if keys.is_empty() || keys.len() > MAX_COMBO_KEYS {
        return Err(format!("Eine Kombination braucht 1 bis {MAX_COMBO_KEYS} Tasten."));
    }
    if keys.iter().any(|key| key.kind > 15) {
        return Err("Ungültige Taste in der Kombination.".to_string());
    }
    Ok(())
}

/// Checks an exported file and returns its flash copy, limited to what this mouse supports.
fn prepare_import(bytes: &[u8], connection: u8) -> Result<Vec<u8>, String> {
    let footer_text = |offset: usize, length: usize| {
        let field = &bytes[FLASH_SIZE + offset..FLASH_SIZE + offset + length];
        field.iter().rposition(|&b| b != 0).map_or(&field[..0], |last| &field[..=last]).to_vec()
    };
    if bytes.len() != FLASH_SIZE + FOOTER_LEN || footer_text(0, 32) != FOOTER_VENDOR {
        return Err("Keine gültige Konfigurationsdatei.".to_string());
    }
    if footer_text(32, 16) != FOOTER_TYPE || footer_text(48, 16) != FOOTER_SENSOR {
        return Err("Die Datei passt nicht zu dieser Maus.".to_string());
    }
    let mut flash = bytes[..FLASH_SIZE].to_vec();
    let limit = max_polling(connection);
    if polling_from_flash(flash[ADDR_POLLING]) > limit {
        flash[ADDR_POLLING] = polling_to_flash(limit);
        flash[ADDR_POLLING + 1] = 85u8.wrapping_sub(flash[ADDR_POLLING]);
    }
    if flash[ADDR_STAGE_COUNT] == 0 {
        return Err("Die Datei enthält keine DPI-Stufe.".to_string());
    }
    if flash[ADDR_STAGE_COUNT] as usize > MAX_STAGES {
        flash[ADDR_STAGE_COUNT] = MAX_STAGES as u8;
        flash[ADDR_STAGE_COUNT + 1] = 85u8.wrapping_sub(MAX_STAGES as u8);
    }
    if flash[ADDR_ACTIVE_STAGE] >= flash[ADDR_STAGE_COUNT] {
        flash[ADDR_ACTIVE_STAGE] = flash[ADDR_STAGE_COUNT] - 1;
        flash[ADDR_ACTIVE_STAGE + 1] = 85u8.wrapping_sub(flash[ADDR_ACTIVE_STAGE]);
    }
    let has_left_click = (0..KEY_COUNT).any(|key| {
        let address = ADDR_KEYS + 4 * key;
        let param = ((flash[address + 1] as u16) << 8) + flash[address + 2] as u16;
        flash[address] == KEY_TYPE_MOUSE && param == LEFT_CLICK
    });
    if !has_left_click {
        return Err("In der Datei hat keine Taste Linksklick.".to_string());
    }
    Ok(flash)
}

fn polling_to_flash(hz: u32) -> u8 {
    if hz <= 1000 {
        (1000 / hz) as u8
    } else {
        (hz / 2000 * 16) as u8
    }
}

fn max_polling(connection: u8) -> u32 {
    match connection {
        1 => 4000,
        3 | 5 => 8000,
        4 => 2000,
        _ => 1000,
    }
}

fn valid_dpi(dpi: u32) -> bool {
    if !(DPI_MIN..=DPI_MAX).contains(&dpi) {
        return false;
    }
    let step = if dpi >= DPI_DOUBLED_FROM { DPI_DOUBLED_STEP } else { DPI_STEP };
    dpi % step == 0
}

/// One stage entry: low byte twice, high bits and scale flags, then the checksum.
fn encode_dpi(dpi: u32) -> [u8; DPI_ENTRY_LEN] {
    let (scaled, flags) = if dpi >= DPI_DOUBLED_FROM { (dpi / 2, DPI_DOUBLED_FLAGS) } else { (dpi, 0) };
    let raw = scaled / DPI_STEP - 1;
    let low = raw as u8;
    let high = (raw >> 8) as u8;
    let third = (high << 2) | (high << 6) | flags;
    let sum = low as u32 + low as u32 + third as u32;
    [low, low, third, 85u8.wrapping_sub(sum as u8)]
}

impl Worker {
    fn run(&mut self) {
        loop {
            if let Some(device) = self.open_device() {
                let _ = self.session(&device);
            }
            self.update(|state| *state = MouseState::default());
            let wait_until = Instant::now() + RECONNECT_DELAY;
            while Instant::now() < wait_until {
                while let Ok(job) = self.job_queue.try_recv() {
                    let _ = job.reply.send(Err("Maus nicht verbunden.".to_string()));
                }
                thread::sleep(Duration::from_millis(100));
            }
        }
    }

    fn write_flash(&mut self, device: &HidDevice, address: usize, data: &[u8]) -> Result<Result<(), String>, DeviceGone> {
        let mut offset = 0;
        while offset < data.len() {
            let end = (offset + CHUNK).min(data.len());
            let chunk = &data[offset..end];
            let packet = build_packet(CMD_WRITE_FLASH, address + offset, chunk.len() as u8, chunk);
            if self.request(device, &packet)?.is_none() {
                return Ok(Err("Die Maus hat das Schreiben nicht bestätigt.".to_string()));
            }
            offset = end;
        }
        if !self.read_flash(device, address, data.len())? {
            return Ok(Err("Der Wert konnte nicht zurückgelesen werden.".to_string()));
        }
        let matches = self.shared.lock().unwrap().flash[address..address + data.len()] == *data;
        Ok(if matches { Ok(()) } else { Err("Der zurückgelesene Wert weicht ab.".to_string()) })
    }

    /// Single settings are stored with their complement behind them.
    fn write_byte(&mut self, device: &HidDevice, address: usize, value: u8) -> Result<Result<(), String>, DeviceGone> {
        self.write_flash(device, address, &[value, 85u8.wrapping_sub(value)])
    }

    fn set_stages(&mut self, device: &HidDevice, stages: &[u32], active: u8) -> Result<Result<(), String>, DeviceGone> {
        if stages.is_empty() || stages.len() > MAX_STAGES || active as usize >= stages.len() {
            return Ok(Err("Ungültige DPI-Stufen.".to_string()));
        }
        if let Some(bad) = stages.iter().find(|&&dpi| !valid_dpi(dpi)) {
            return Ok(Err(format!("Ungültiger DPI-Wert {bad}.")));
        }
        for (index, &dpi) in stages.iter().enumerate() {
            let address = ADDR_DPI + DPI_ENTRY_LEN * index;
            let entry = encode_dpi(dpi);
            let unchanged = self.shared.lock().unwrap().flash[address..address + DPI_ENTRY_LEN] == entry;
            if !unchanged {
                if let Err(message) = self.write_flash(device, address, &entry)? {
                    return Ok(Err(message));
                }
            }
        }
        // The active stage must exist at every moment, so shrinking moves it first.
        let count = stages.len() as u8;
        let shrinking = count < self.flash_byte(ADDR_STAGE_COUNT);
        let steps = if shrinking {
            [(ADDR_ACTIVE_STAGE, active), (ADDR_STAGE_COUNT, count)]
        } else {
            [(ADDR_STAGE_COUNT, count), (ADDR_ACTIVE_STAGE, active)]
        };
        for (address, value) in steps {
            if self.flash_byte(address) != value {
                if let Err(message) = self.write_byte(device, address, value)? {
                    return Ok(Err(message));
                }
            }
        }
        Ok(Ok(()))
    }

    fn set_long_distance(&mut self, device: &HidDevice, on: bool) -> Result<Result<(), String>, DeviceGone> {
        let mut payload = [0u8; 10];
        payload[0] = on as u8;
        if self.request(device, &build_packet(CMD_SET_LONG_RANGE, 0, 10, &payload))?.is_none() {
            return Ok(Err("Die Maus hat das Schreiben nicht bestätigt.".to_string()));
        }
        let current = self.query(device, CMD_GET_LONG_RANGE)?.map(|reply| reply[5] == 1);
        self.update(|state| state.long_distance = current);
        Ok(if current == Some(on) { Ok(()) } else { Err("Der zurückgelesene Wert weicht ab.".to_string()) })
    }

    fn set_light(&mut self, device: &HidDevice, mode: u8, color: [u8; 3], speed: u8, brightness: u8) -> Result<Result<(), String>, DeviceGone> {
        if mode > MAX_LIGHT_MODE || speed > MAX_LIGHT_LEVEL || brightness > MAX_LIGHT_LEVEL {
            return Ok(Err("Ungültige Lichtwerte.".to_string()));
        }
        // With a sleeping mouse at app start, nothing has read the light yet.
        let known = self.shared.lock().unwrap().state.light.clone();
        let previous = match known {
            Some(light) => Some(light),
            None => self.read_light(device)?,
        };
        let Some(previous) = previous else {
            return Ok(Err("Das Empfänger-Licht wurde noch nicht gelesen.".to_string()));
        };
        let time = previous.time;
        let mut payload = [mode, color[0], color[1], color[2], speed, brightness, time, 0, 0, 0];
        // The receiver only shows new values after a mode change, so pass through "off" briefly.
        if previous.mode == mode && mode != LIGHT_MODE_OFF {
            payload[0] = LIGHT_MODE_OFF;
            if self.request(device, &build_packet(CMD_SET_DONGLE_LIGHT, 0, 10, &payload))?.is_none() {
                return Ok(Err("Der Empfänger hat das Schreiben nicht bestätigt.".to_string()));
            }
            thread::sleep(LIGHT_REFRESH_PAUSE);
            payload[0] = mode;
        }
        if self.request(device, &build_packet(CMD_SET_DONGLE_LIGHT, 0, 10, &payload))?.is_none() {
            return Ok(Err("Der Empfänger hat das Schreiben nicht bestätigt.".to_string()));
        }
        let wanted = LightState { mode, color, speed, brightness, time };
        let mut current = self.read_light(device)?;
        for _ in 0..LIGHT_READ_RETRIES {
            if current.as_ref() == Some(&wanted) {
                break;
            }
            thread::sleep(LIGHT_REFRESH_PAUSE);
            current = self.read_light(device)?;
        }
        let confirmed = current.as_ref() == Some(&wanted);
        self.update(|state| state.light = current);
        Ok(if confirmed { Ok(()) } else { Err("Der zurückgelesene Wert weicht ab.".to_string()) })
    }

    fn read_light(&mut self, device: &HidDevice) -> Result<Option<LightState>, DeviceGone> {
        Ok(self.query(device, CMD_GET_DONGLE_LIGHT)?.map(|reply| LightState {
            mode: reply[5],
            color: [reply[6], reply[7], reply[8]],
            speed: reply[9],
            brightness: reply[10],
            time: reply[11],
        }))
    }

    /// One key has to keep left click, otherwise the mouse can no longer be operated.
    fn keeps_left_click(&self, key: usize, kind: u8, param: u16) -> bool {
        if kind == KEY_TYPE_MOUSE && param == LEFT_CLICK {
            return true;
        }
        let shared = self.shared.lock().unwrap();
        (0..KEY_COUNT).filter(|&other| other != key).any(|other| {
            let address = ADDR_KEYS + 4 * other;
            let other_param = ((shared.flash[address + 1] as u16) << 8) + shared.flash[address + 2] as u16;
            shared.flash[address] == KEY_TYPE_MOUSE && other_param == LEFT_CLICK
        })
    }

    fn write_key(&mut self, device: &HidDevice, key: usize, kind: u8, param: u16) -> Result<Result<(), String>, DeviceGone> {
        if key >= KEY_COUNT {
            return Ok(Err("Unbekannte Taste.".to_string()));
        }
        if !self.keeps_left_click(key, kind, param) {
            return Ok(Err("Mindestens eine Taste muss Linksklick bleiben.".to_string()));
        }
        self.write_flash(device, ADDR_KEYS + 4 * key, &key_entry(kind, param))
    }

    fn write_shortcut_key(&mut self, device: &HidDevice, key: usize, data: &[u8]) -> Result<Result<(), String>, DeviceGone> {
        if key >= KEY_COUNT {
            return Ok(Err("Unbekannte Taste.".to_string()));
        }
        if !self.keeps_left_click(key, KEY_TYPE_SHORTCUT, 0) {
            return Ok(Err("Mindestens eine Taste muss Linksklick bleiben.".to_string()));
        }
        if let Err(message) = self.write_flash(device, ADDR_SHORTCUTS + SHORTCUT_SIZE * key, data)? {
            return Ok(Err(message));
        }
        self.write_key(device, key, KEY_TYPE_SHORTCUT, 0)
    }

    fn set_macro(&mut self, device: &HidDevice, key: usize, name: &str, cycle: u8, events: &[MacroEvent]) -> Result<Result<(), String>, DeviceGone> {
        if key >= KEY_COUNT {
            return Ok(Err("Unbekannte Taste.".to_string()));
        }
        if let Err(message) = validate_macro(name, cycle, events) {
            return Ok(Err(message));
        }
        let param = ((key as u16) << 8) | cycle as u16;
        if !self.keeps_left_click(key, KEY_TYPE_MACRO, param) {
            return Ok(Err("Mindestens eine Taste muss Linksklick bleiben.".to_string()));
        }
        // The data comes first, so that the key never points at an unfinished macro.
        if let Err(message) = self.write_flash(device, ADDR_MACROS + MACRO_SIZE * key, &macro_bytes(name, events))? {
            return Ok(Err(message));
        }
        self.write_flash(device, ADDR_KEYS + 4 * key, &key_entry(KEY_TYPE_MACRO, param))
    }

    fn clear_macro(&mut self, device: &HidDevice, key: usize) -> Result<Result<(), String>, DeviceGone> {
        if key >= KEY_COUNT {
            return Ok(Err("Unbekannte Taste.".to_string()));
        }
        if let Err(message) = self.write_flash(device, ADDR_KEYS + 4 * key, &key_entry(KEY_TYPE_MOUSE, DEFAULT_KEYS[key]))? {
            return Ok(Err(message));
        }
        self.write_flash(device, ADDR_MACROS + MACRO_SIZE * key, &[0u8; MACRO_SIZE])
    }

    fn write_imported(&mut self, device: &HidDevice, flash: &[u8]) -> Result<Result<(), String>, DeviceGone> {
        // The data comes first, so that the keys never point at unfinished shortcuts or macros.
        for key in 0..KEY_COUNT {
            let slots = [(ADDR_SHORTCUTS + SHORTCUT_SIZE * key, SHORTCUT_SIZE), (ADDR_MACROS + MACRO_SIZE * key, MACRO_SIZE)];
            for (address, length) in slots {
                let wanted = &flash[address..address + length];
                let unchanged = self.shared.lock().unwrap().flash[address..address + length] == *wanted;
                if !unchanged {
                    if let Err(message) = self.write_flash(device, address, wanted)? {
                        return Ok(Err(message));
                    }
                }
            }
        }
        self.write_flash(device, 0, &flash[..SETTINGS_READ_END])
    }

    fn import_config(&mut self, device: &HidDevice, bytes: &[u8]) -> Result<Result<(), String>, DeviceGone> {
        let connection = self.shared.lock().unwrap().state.connection;
        let flash = match prepare_import(bytes, connection) {
            Ok(flash) => flash,
            Err(message) => return Ok(Err(message)),
        };
        let result = self.write_imported(device, &flash)?;
        if result.is_err() {
            // A partial import may leave anything behind, so show what the mouse really holds.
            self.read_all(device)?;
        }
        Ok(result)
    }

    fn set_profile(&mut self, device: &HidDevice, profile: u8) -> Result<Result<(), String>, DeviceGone> {
        if profile >= PROFILE_COUNT {
            return Ok(Err(format!("Unbekanntes Profil {profile}.")));
        }
        if self.request(device, &build_packet(CMD_SET_PROFILE, 0, 1, &[profile]))?.is_none() {
            return Ok(Err("Die Maus hat den Profilwechsel nicht bestätigt.".to_string()));
        }
        let current = self.read_profile(device)?;
        self.update(|state| state.profile = current);
        if current != Some(profile) {
            return Ok(Err("Die Maus hat das Profil nicht gewechselt.".to_string()));
        }
        // Every profile has its own settings. Without them the page must not show the old ones.
        if !self.read_all(device)? {
            self.update(|state| state.has_data = false);
            return Ok(Err("Das Profil konnte nicht gelesen werden. Bitte die Maus kurz bewegen.".to_string()));
        }
        self.apply_flash();
        Ok(Ok(()))
    }

    fn execute(&mut self, device: &HidDevice, command: MouseCommand) -> Result<Result<(), String>, DeviceGone> {
        // The light lives in the receiver, so a sleeping mouse does not matter here.
        if let MouseCommand::SetLight { mode, color, speed, brightness } = command {
            return self.set_light(device, mode, color, speed, brightness);
        }
        let online = self.query(device, CMD_ONLINE)?.map_or(false, |reply| reply[5] == 1);
        let has_data = self.shared.lock().unwrap().state.has_data;
        if !online || !has_data {
            return Ok(Err("Die Maus ist inaktiv. Bitte kurz bewegen und erneut versuchen.".to_string()));
        }
        if let MouseCommand::SetProfile { profile } = command {
            return self.set_profile(device, profile);
        }
        let result = match command {
            MouseCommand::SetField { field, value } => match validate_field(field, value) {
                Ok(address) => self.write_byte(device, address, value)?,
                Err(message) => Err(message),
            },
            MouseCommand::SetPolling { hz } => {
                let limit = max_polling(self.shared.lock().unwrap().state.connection);
                if POLLING_RATES.contains(&hz) && hz <= limit {
                    self.write_byte(device, ADDR_POLLING, polling_to_flash(hz))?
                } else {
                    Err(format!("Polling Rate {hz} Hz wird hier nicht unterstützt."))
                }
            }
            MouseCommand::SetStages { stages, active } => self.set_stages(device, &stages, active)?,
            MouseCommand::SetLongDistance { on } => self.set_long_distance(device, on)?,
            MouseCommand::SetLight { .. } | MouseCommand::SetProfile { .. } => unreachable!("handled above"),
            MouseCommand::SetKey { key, kind, param } => match validate_key(kind, param) {
                Ok(()) => self.write_key(device, key as usize, kind, param)?,
                Err(message) => Err(message),
            },
            MouseCommand::SetMedia { key, code } => {
                if MEDIA_CODES.contains(&code) {
                    self.write_shortcut_key(device, key as usize, &shortcut_bytes_media(code))?
                } else {
                    Err(format!("Unbekannte Multimedia-Funktion {code:#06x}."))
                }
            }
            MouseCommand::SetCombo { key, keys } => match validate_combo(&keys) {
                Ok(()) => self.write_shortcut_key(device, key as usize, &shortcut_bytes_combo(&keys))?,
                Err(message) => Err(message),
            },
            MouseCommand::SetMacro { key, name, cycle, events } => self.set_macro(device, key as usize, &name, cycle, &events)?,
            MouseCommand::ClearMacro { key } => self.clear_macro(device, key as usize)?,
            MouseCommand::ImportConfig { bytes } => self.import_config(device, &bytes)?,
        };
        // On failure too: the screen must show what the mouse really holds.
        self.apply_flash();
        Ok(result)
    }

    fn run_jobs(&mut self, device: &HidDevice) -> Result<(), DeviceGone> {
        while let Ok(job) = self.job_queue.try_recv() {
            match self.execute(device, job.command) {
                Ok(result) => {
                    let _ = job.reply.send(result);
                }
                Err(gone) => {
                    let _ = job.reply.send(Err("Maus nicht verbunden.".to_string()));
                    return Err(gone);
                }
            }
        }
        Ok(())
    }

    fn update(&self, change: impl FnOnce(&mut MouseState)) {
        let snapshot = {
            let mut shared = self.shared.lock().unwrap();
            let before = shared.state.clone();
            change(&mut shared.state);
            if shared.state == before {
                return;
            }
            shared.state.clone()
        };
        (self.on_change)(snapshot);
    }

    fn open_device(&mut self) -> Option<HidDevice> {
        let api = HidApi::new().ok()?;
        let paths: Vec<CString> = api
            .device_list()
            .filter(|d| d.vendor_id() == VENDOR_ID && PRODUCT_IDS.contains(&d.product_id()))
            .map(|d| d.path().to_owned())
            .collect();
        for path in paths {
            let Ok(device) = api.open_path(&path) else { continue };
            // Only the vendor collection answers on report 8, the others reject the write.
            let probe = build_packet(CMD_ONLINE, 0, 0, &[]);
            if let Ok(Some(_)) = self.request(&device, &probe) {
                return Some(device);
            }
        }
        None
    }

    fn request(&mut self, device: &HidDevice, packet: &Packet) -> Result<Option<Packet>, DeviceGone> {
        let mut report = [0u8; REPORT_LEN];
        report[0] = REPORT_ID;
        report[1..].copy_from_slice(packet);
        for _ in 0..ATTEMPTS {
            device.write(&report).map_err(|_| DeviceGone)?;
            let deadline = Instant::now() + REPLY_TIMEOUT;
            loop {
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    break;
                }
                let Some(reply) = self.read_packet(device, left.as_millis() as i32)? else { continue };
                if !is_reply(&reply, packet) {
                    continue;
                }
                return Ok(if reply[1] == 0 { Some(reply) } else { None });
            }
        }
        Ok(None)
    }

    fn read_packet(&mut self, device: &HidDevice, timeout_ms: i32) -> Result<Option<Packet>, DeviceGone> {
        let mut buffer = [0u8; REPORT_LEN];
        let count = device.read_timeout(&mut buffer, timeout_ms).map_err(|_| DeviceGone)?;
        if count < REPORT_LEN || buffer[0] != REPORT_ID {
            return Ok(None);
        }
        let mut packet = [0u8; PACKET_LEN];
        packet.copy_from_slice(&buffer[1..]);
        if packet[0] == CMD_STATUS_CHANGED {
            self.status_flags |= packet[5];
            return Ok(None);
        }
        Ok(Some(packet))
    }

    fn read_flash(&mut self, device: &HidDevice, address: usize, length: usize) -> Result<bool, DeviceGone> {
        let mut offset = 0;
        while offset < length {
            let chunk = CHUNK.min(length - offset);
            let packet = build_packet(CMD_READ_FLASH, address + offset, chunk as u8, &[]);
            let Some(reply) = self.request(device, &packet)? else { return Ok(false) };
            let mut shared = self.shared.lock().unwrap();
            shared.flash[address + offset..address + offset + chunk].copy_from_slice(&reply[5..5 + chunk]);
            offset += CHUNK;
        }
        Ok(true)
    }

    fn flash_byte(&self, address: usize) -> u8 {
        self.shared.lock().unwrap().flash[address]
    }

    /// Reads everything an export contains, so that it stays complete:
    /// the settings block, and shortcut and macro data only for keys that use them.
    fn read_all(&mut self, device: &HidDevice) -> Result<bool, DeviceGone> {
        self.shared.lock().unwrap().flash.fill(0xFF);
        let mut address = 0;
        while address < SETTINGS_READ_END {
            if !self.read_flash(device, address, CHUNK)? {
                return Ok(false);
            }
            address += CHUNK;
        }
        for key in 0..KEY_COUNT {
            match self.flash_byte(ADDR_KEYS + 4 * key) {
                KEY_TYPE_SHORTCUT => {
                    if !self.read_shortcut(device, key)? {
                        return Ok(false);
                    }
                }
                KEY_TYPE_MACRO => {
                    if !self.read_macro(device, key)? {
                        return Ok(false);
                    }
                }
                _ => {}
            }
        }
        Ok(true)
    }

    fn read_shortcut(&mut self, device: &HidDevice, key: usize) -> Result<bool, DeviceGone> {
        let start = ADDR_SHORTCUTS + SHORTCUT_SIZE * key;
        if !self.read_flash(device, start, CHUNK)? {
            return Ok(false);
        }
        let count = self.flash_byte(start) as usize;
        if count > 2 {
            let total = (3 * count + 2).min(SHORTCUT_SIZE);
            if total > CHUNK && !self.read_flash(device, start + CHUNK, total - CHUNK)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn read_macro(&mut self, device: &HidDevice, key: usize) -> Result<bool, DeviceGone> {
        let start = ADDR_MACROS + MACRO_SIZE * key;
        if !self.read_flash(device, start, CHUNK)? {
            return Ok(false);
        }
        let name_total = (self.flash_byte(start) as usize + 1).min(MACRO_EVENTS_OFFSET);
        if name_total > CHUNK && !self.read_flash(device, start + CHUNK, name_total - CHUNK)? {
            return Ok(false);
        }
        let events = start + MACRO_EVENTS_OFFSET;
        if !self.read_flash(device, events, CHUNK)? {
            return Ok(false);
        }
        let count = self.flash_byte(events) as usize;
        if count > 2 {
            let total = (5 * count + 2).min(MACRO_SIZE - MACRO_EVENTS_OFFSET);
            if total > CHUNK && !self.read_flash(device, events + CHUNK, total - CHUNK)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn apply_flash(&self) {
        let parsed = {
            let shared = self.shared.lock().unwrap();
            parse_flash(&shared.flash)
        };
        self.update(|state| {
            state.has_data = true;
            state.stages = parsed.stages;
            state.active_stage = parsed.active_stage;
            state.polling = parsed.polling;
            state.lod = parsed.lod;
            state.debounce = parsed.debounce;
            state.ripple = parsed.ripple;
            state.angle = parsed.angle;
            state.motion_sync = parsed.motion_sync;
            state.performance = parsed.performance;
            state.performance_time = parsed.performance_time;
            state.sleep_time = parsed.sleep_time;
            state.keys = parsed.keys;
        });
    }

    fn query(&mut self, device: &HidDevice, command: u8) -> Result<Option<Packet>, DeviceGone> {
        self.request(device, &build_packet(command, 0, 0, &[]))
    }

    fn read_battery(&mut self, device: &HidDevice) -> Result<(), DeviceGone> {
        if let Some(reply) = self.query(device, CMD_BATTERY)? {
            let charging = reply[6] == 1;
            let voltage = ((reply[7] as u32) << 8) + reply[8] as u32;
            let target = if voltage > 0 { voltage_to_level(voltage, charging) } else { reply[5] };
            let shown = next_battery_display(self.battery_display, target, reply[5], charging);
            self.battery_display = Some(shown);
            self.update(|state| {
                state.battery = Some(shown);
                state.charging = charging;
                state.battery_voltage = (voltage > 0).then_some(voltage);
            });
        }
        Ok(())
    }

    fn read_profile(&mut self, device: &HidDevice) -> Result<Option<u8>, DeviceGone> {
        Ok(self.query(device, CMD_GET_PROFILE)?.map(|reply| reply[5]))
    }

    fn read_extras(&mut self, device: &HidDevice) -> Result<(), DeviceGone> {
        let profile = self.read_profile(device)?;
        let firmware = self.query(device, CMD_READ_VERSION)?.map(version_text);
        let dongle = self.query(device, CMD_GET_DONGLE_VERSION)?.map(version_text);
        let long_range = self.query(device, CMD_GET_LONG_RANGE)?.map(|reply| reply[5] == 1);
        let light = self.read_light(device)?;
        self.update(|state| {
            state.profile = profile;
            state.firmware = firmware;
            state.dongle_firmware = dongle;
            state.long_distance = long_range;
            state.light = light;
        });
        self.read_battery(device)
    }

    fn handle_status(&mut self, device: &HidDevice) -> Result<(), DeviceGone> {
        let flags = std::mem::take(&mut self.status_flags);
        if flags == 0 {
            return Ok(());
        }
        if flags & STATUS_PROFILE != 0 {
            // Every profile has its own settings, so a switch needs a complete read.
            let profile = self.read_profile(device)?;
            self.update(|state| state.profile = profile);
            if self.read_all(device)? {
                self.apply_flash();
            }
        }
        if flags & STATUS_DPI != 0 && self.read_flash(device, ADDR_ACTIVE_STAGE, 2)? {
            self.apply_flash();
        }
        if flags & STATUS_POLLING != 0 && self.read_flash(device, ADDR_POLLING, 2)? {
            self.apply_flash();
        }
        if flags & STATUS_DPI_EFFECT != 0 {
            self.read_flash(device, ADDR_DPI_EFFECT, 8)?;
        }
        if flags & STATUS_LIGHT != 0 {
            self.read_flash(device, ADDR_LIGHT, 7)?;
        }
        if flags & STATUS_BATTERY != 0 {
            self.read_battery(device)?;
        }
        Ok(())
    }

    fn session(&mut self, device: &HidDevice) -> Result<(), DeviceGone> {
        self.update(|state| state.connected = true);

        // This handshake comes before anything else and returns the device ids.
        let seed = (Instant::now().elapsed().subsec_nanos() ^ std::process::id()).to_le_bytes();
        let handshake = [seed[0], seed[1], seed[2], seed[3], 0, 0, 0, 0];
        if let Some(reply) = self.request(device, &build_packet(CMD_ENCRYPTION, 0, 8, &handshake))? {
            self.update(|state| {
                state.cid = reply[9];
                state.mid = reply[10];
                state.connection = reply[11];
            });
        }

        let mut was_online = false;
        let mut needs_read = true;
        let mut next_online_check = Instant::now();
        let mut next_battery = Instant::now() + BATTERY_CHECK;
        loop {
            let now = Instant::now();
            if now >= next_online_check {
                let online = self.query(device, CMD_ONLINE)?.map_or(false, |reply| reply[5] == 1);
                if online && !was_online {
                    needs_read = true;
                }
                was_online = online;
                self.update(|state| state.online = online);
                next_online_check = now + if online { ONLINE_CHECK } else { OFFLINE_CHECK };
            }
            if was_online && needs_read && self.read_all(device)? {
                self.apply_flash();
                self.read_extras(device)?;
                needs_read = false;
            }
            if was_online && now >= next_battery {
                self.read_battery(device)?;
                next_battery = now + BATTERY_CHECK;
            }
            self.read_packet(device, IDLE_READ_MS)?;
            self.run_jobs(device)?;
            if self.status_flags != 0 && !was_online {
                // A status event while marked offline means the mouse has woken up.
                next_online_check = Instant::now();
            }
            if was_online {
                self.handle_status(device)?;
            }
        }
    }
}

fn version_text(reply: Packet) -> String {
    format!("v{}.{:02x}", reply[5], reply[6])
}

fn polling_from_flash(value: u8) -> u32 {
    if value >= 16 {
        value as u32 / 16 * 2000
    } else if value > 0 {
        1000 / value as u32
    } else {
        1000
    }
}

fn decode_dpi(low: u8, flags: u8) -> u32 {
    let raw = low as u32 + (((flags as u32 & 12) >> 2) << 8);
    let mut dpi = (raw + 1) * DPI_STEP;
    if flags & 1 != 0 {
        dpi *= 2;
    }
    if flags & 2 != 0 {
        dpi *= 2;
    }
    dpi
}

fn parse_media(flash: &[u8], key: usize) -> Option<u16> {
    let start = ADDR_SHORTCUTS + SHORTCUT_SIZE * key;
    let entries = flash[start] as usize / 2;
    if entries != 1 || flash[start + 1] & 15 != 2 {
        return None;
    }
    Some(flash[start + 2] as u16 + ((flash[start + 3] as u16) << 8))
}

fn parse_combo(flash: &[u8], key: usize) -> Vec<HidKey> {
    let start = ADDR_SHORTCUTS + SHORTCUT_SIZE * key;
    let entries = (flash[start] as usize / 2).min(MAX_COMBO_KEYS);
    (0..entries)
        .map(|i| {
            let entry = start + 1 + 3 * i;
            HidKey { kind: flash[entry] & 15, value: flash[entry + 1] as u16 + ((flash[entry + 2] as u16) << 8) }
        })
        .collect()
}

fn parse_macro(flash: &[u8], key: usize) -> Option<(String, Vec<MacroEvent>)> {
    let start = ADDR_MACROS + MACRO_SIZE * key;
    let name_len = flash[start] as usize;
    let count = flash[start + MACRO_EVENTS_OFFSET] as usize;
    if name_len == 0 || name_len > MAX_MACRO_NAME || count > MAX_MACRO_EVENTS {
        return None;
    }
    let name = String::from_utf8_lossy(&flash[start + 1..start + 1 + name_len]).into_owned();
    let events = (0..count)
        .map(|i| {
            let e = start + 32 + 5 * i;
            MacroEvent {
                down: flash[e] >> 6 == 2,
                kind: flash[e] & 15,
                value: flash[e + 1] as u16 + ((flash[e + 2] as u16) << 8),
                delay: ((flash[e + 3] as u16) << 8) + flash[e + 4] as u16,
            }
        })
        .collect();
    Some((name, events))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_entries_match_bytes_read_from_the_mouse() {
        assert_eq!(key_entry(KEY_TYPE_MOUSE, 0x0100), [0x01, 0x01, 0x00, 0x53]);
        assert_eq!(key_entry(KEY_TYPE_MOUSE, 0x0800), [0x01, 0x08, 0x00, 0x4C]);
        assert_eq!(key_entry(KEY_TYPE_DPI, 0x0100), [0x02, 0x01, 0x00, 0x52]);
    }

    #[test]
    fn media_shortcut_has_the_expected_layout() {
        assert_eq!(shortcut_bytes_media(0x00CD), [2, 130, 0xCD, 0, 66, 0xCD, 0, 245]);
    }

    #[test]
    fn combo_releases_in_reverse_order() {
        let ctrl = HidKey { kind: 0, value: 1 };
        let c = HidKey { kind: 1, value: 6 };
        // 4 + 128 + 1 + 0 + 129 + 6 + 0 + 65 + 6 + 0 + 64 + 1 + 0 = 404, low byte 148, 85 - 148 wraps to 193.
        assert_eq!(shortcut_bytes_combo(&[ctrl, c]), vec![4, 128, 1, 0, 129, 6, 0, 65, 6, 0, 64, 1, 0, 193]);
    }

    #[test]
    fn macro_bytes_round_trip_through_the_parser() {
        let events = vec![
            MacroEvent { down: true, kind: 1, value: 6, delay: 300 },
            MacroEvent { down: false, kind: 1, value: 6, delay: 0 },
        ];
        let bytes = macro_bytes("Test", &events);
        assert_eq!(bytes.len(), 33 + 5 * events.len());
        assert_eq!(&bytes[..5], &[4, b'T', b'e', b's', b't']);
        assert_eq!(bytes[5], 0xFF);
        assert_eq!(bytes[31], 2);
        assert_eq!(&bytes[32..37], &[0x81, 6, 0, 1, 44]);
        // 129 + 6 + 1 + 44 + 65 + 6 = 251, 85 - 251 wraps to 90, minus the two events.
        assert_eq!(bytes[42], 88);

        let mut flash = vec![0xFF; FLASH_SIZE];
        flash[ADDR_MACROS..ADDR_MACROS + bytes.len()].copy_from_slice(&bytes);
        assert_eq!(parse_macro(&flash, 0), Some(("Test".to_string(), events)));
    }

    #[test]
    fn macro_bytes_have_the_expected_format() {
        let events = vec![
            MacroEvent { down: true, kind: 0, value: 1, delay: 168 },
            MacroEvent { down: true, kind: 1, value: 6, delay: 75 },
            MacroEvent { down: false, kind: 0, value: 1, delay: 29 },
            MacroEvent { down: false, kind: 1, value: 6, delay: 0 },
        ];
        let mut expected = vec![0x05, 0x74, 0x65, 0x73, 0x74, 0x31];
        expected.resize(31, 0xFF);
        expected.extend([
            0x04, 0x80, 0x01, 0x00, 0x00, 0xA8, 0x81, 0x06, 0x00, 0x00, 0x4B, 0x40, 0x01, 0x00, 0x00, 0x1D, 0x41, 0x06,
            0x00, 0x00, 0x00, 0xB1,
        ]);
        assert_eq!(macro_bytes("test1", &events), expected);
        assert_eq!(key_entry(KEY_TYPE_MACRO, (4 << 8) | 1), [0x06, 0x04, 0x01, 0x4A]);
    }

    fn import_file() -> Vec<u8> {
        let mut bytes = vec![0xFF; FLASH_SIZE + FOOTER_LEN];
        bytes[FLASH_SIZE..].fill(0);
        bytes[FLASH_SIZE..FLASH_SIZE + FOOTER_VENDOR.len()].copy_from_slice(FOOTER_VENDOR);
        bytes[FLASH_SIZE + 32..FLASH_SIZE + 32 + FOOTER_TYPE.len()].copy_from_slice(FOOTER_TYPE);
        bytes[FLASH_SIZE + 48..FLASH_SIZE + 48 + FOOTER_SENSOR.len()].copy_from_slice(FOOTER_SENSOR);
        bytes[ADDR_POLLING] = polling_to_flash(1000);
        bytes[ADDR_STAGE_COUNT] = 4;
        bytes[ADDR_ACTIVE_STAGE] = 1;
        bytes[ADDR_KEYS..ADDR_KEYS + 4].copy_from_slice(&key_entry(KEY_TYPE_MOUSE, LEFT_CLICK));
        bytes
    }

    #[test]
    fn import_keeps_a_valid_file_unchanged() {
        let bytes = import_file();
        assert_eq!(prepare_import(&bytes, 1).unwrap(), bytes[..FLASH_SIZE]);
    }

    #[test]
    fn import_rejects_foreign_files() {
        assert!(prepare_import(&[0u8; 100], 1).is_err());
        let mut vendor = import_file();
        vendor[FLASH_SIZE] = b'X';
        assert!(prepare_import(&vendor, 1).is_err());
        let mut sensor = import_file();
        sensor[FLASH_SIZE + 48] = b'X';
        assert!(prepare_import(&sensor, 1).is_err());
    }

    #[test]
    fn import_needs_left_click_and_a_stage() {
        let mut keys = import_file();
        keys[ADDR_KEYS..ADDR_KEYS + 4].copy_from_slice(&key_entry(KEY_TYPE_DISABLED, 0));
        assert!(prepare_import(&keys, 1).is_err());
        let mut stages = import_file();
        stages[ADDR_STAGE_COUNT] = 0;
        assert!(prepare_import(&stages, 1).is_err());
    }

    #[test]
    fn import_limits_polling_and_stages() {
        let mut bytes = import_file();
        bytes[ADDR_POLLING] = polling_to_flash(8000);
        bytes[ADDR_STAGE_COUNT] = 12;
        bytes[ADDR_ACTIVE_STAGE] = 11;
        let flash = prepare_import(&bytes, 4).unwrap();
        assert_eq!(polling_from_flash(flash[ADDR_POLLING]), 2000);
        assert_eq!(flash[ADDR_POLLING + 1], 85u8.wrapping_sub(flash[ADDR_POLLING]));
        assert_eq!(flash[ADDR_STAGE_COUNT], 8);
        assert_eq!(flash[ADDR_STAGE_COUNT + 1], 85u8.wrapping_sub(8));
        assert_eq!(flash[ADDR_ACTIVE_STAGE], 7);
        assert_eq!(flash[ADDR_ACTIVE_STAGE + 1], 85u8.wrapping_sub(7));
    }

    #[test]
    fn battery_level_follows_the_voltage_curve() {
        assert_eq!(voltage_to_level(3000, false), 0);
        assert_eq!(voltage_to_level(3300, false), 0);
        assert_eq!(voltage_to_level(3456, false), 10);
        assert_eq!(voltage_to_level(3612, false), 20);
        assert_eq!(voltage_to_level(3800, false), 39);
        assert_eq!(voltage_to_level(3875, false), 50);
        assert_eq!(voltage_to_level(4100, false), 94);
        assert_eq!(voltage_to_level(4164, false), 100);
        assert_eq!(voltage_to_level(4200, false), 100);
        assert_eq!(voltage_to_level(4200, true), 99);
    }

    #[test]
    fn battery_display_moves_in_single_steps() {
        assert_eq!(next_battery_display(None, 60, 50, false), 60);
        assert_eq!(next_battery_display(Some(60), 50, 50, false), 59);
        assert_eq!(next_battery_display(Some(60), 70, 50, false), 60);
        assert_eq!(next_battery_display(Some(60), 65, 50, true), 60);
        assert_eq!(next_battery_display(Some(60), 80, 50, true), 61);
        assert_eq!(next_battery_display(Some(90), 95, 100, true), 100);
    }

    #[test]
    fn invalid_macros_and_keys_are_rejected() {
        assert!(validate_macro("", 1, &[]).is_err());
        assert!(validate_macro(&"x".repeat(31), 1, &[]).is_err());
        assert!(validate_macro("ok", 0, &[]).is_err());
        assert!(validate_macro("ok", 251, &[]).is_err());
        assert!(validate_macro("ok", 254, &[]).is_ok());
        assert!(validate_macro("ok", 1, &vec![MacroEvent::default(); 71]).is_err());
        assert!(validate_key(9, 0).is_err());
        assert!(validate_key(KEY_TYPE_MACRO, 0).is_err());
        assert!(validate_key(KEY_TYPE_FIRE, (10 << 8) | 4).is_err());
        assert!(validate_key(KEY_TYPE_FIRE, (9 << 8) | 2).is_err());
        assert!(validate_key(KEY_TYPE_FIRE, (10 << 8) | 2).is_ok());
    }

    #[test]
    fn dpi_entries_match_bytes_read_from_the_mouse() {
        assert_eq!(encode_dpi(800), [0x0F, 0x0F, 0x00, 0x37]);
        assert_eq!(encode_dpi(1200), [0x17, 0x17, 0x00, 0x27]);
        assert_eq!(encode_dpi(2400), [0x2F, 0x2F, 0x00, 0xF7]);
        assert_eq!(encode_dpi(3200), [0x3F, 0x3F, 0x00, 0xD7]);
        assert_eq!(encode_dpi(4000), [0x4F, 0x4F, 0x00, 0xB7]);
    }

    #[test]
    fn dpi_encoding_round_trips_over_the_whole_range() {
        let mut dpi = DPI_MIN;
        while dpi <= DPI_MAX {
            if valid_dpi(dpi) {
                let entry = encode_dpi(dpi);
                assert_eq!(decode_dpi(entry[0], entry[2]), dpi, "dpi {dpi}");
            }
            dpi += DPI_STEP;
        }
    }

    #[test]
    fn dpi_validation_follows_the_sensor_steps() {
        assert!(valid_dpi(100));
        assert!(valid_dpi(30000));
        assert!(!valid_dpi(50));
        assert!(!valid_dpi(125));
        assert!(!valid_dpi(30150));
        assert!(valid_dpi(30200));
        assert!(!valid_dpi(40100));
    }

    #[test]
    fn write_packet_has_the_expected_format() {
        // Motion Sync on.
        let packet = build_packet(CMD_WRITE_FLASH, ADDR_MOTION_SYNC, 2, &[1, 84]);
        assert_eq!(packet, [0x07, 0, 0, 0xAB, 2, 1, 0x54, 0, 0, 0, 0, 0, 0, 0, 0, 0x44]);
    }

    #[test]
    fn polling_rates_round_trip() {
        for hz in POLLING_RATES {
            assert_eq!(polling_from_flash(polling_to_flash(hz)), hz);
        }
    }

    #[test]
    fn fields_reject_values_outside_their_options() {
        assert_eq!(validate_field(Field::Sleep, 6), Ok(ADDR_SLEEP));
        assert!(validate_field(Field::Sleep, 7).is_err());
        assert!(validate_field(Field::Lod, 0).is_err());
        assert!(validate_field(Field::Debounce, 16).is_err());
        assert!(validate_field(Field::MotionSync, 2).is_err());
    }
}

fn parse_flash(flash: &[u8]) -> MouseState {
    let stage_count = (flash[ADDR_STAGE_COUNT] as usize).clamp(1, MAX_STAGES);
    let stages = (0..stage_count)
        .map(|i| decode_dpi(flash[ADDR_DPI + 4 * i], flash[ADDR_DPI + 4 * i + 2]))
        .collect();
    let keys = (0..KEY_COUNT)
        .map(|key| {
            let address = ADDR_KEYS + 4 * key;
            let kind = flash[address];
            let mut binding = KeyBinding {
                kind,
                param: ((flash[address + 1] as u16) << 8) + flash[address + 2] as u16,
                ..KeyBinding::default()
            };
            if kind == KEY_TYPE_SHORTCUT {
                binding.media = parse_media(flash, key);
                if binding.media.is_none() {
                    binding.combo = parse_combo(flash, key);
                }
            } else if kind == KEY_TYPE_MACRO {
                if let Some((name, events)) = parse_macro(flash, key) {
                    binding.macro_name = Some(name);
                    binding.macro_events = events;
                }
            }
            binding
        })
        .collect();
    MouseState {
        stages,
        active_stage: flash[ADDR_ACTIVE_STAGE],
        polling: polling_from_flash(flash[ADDR_POLLING]),
        lod: flash[ADDR_LOD],
        debounce: flash[ADDR_DEBOUNCE],
        ripple: flash[ADDR_RIPPLE] == 1,
        angle: flash[ADDR_ANGLE] == 1,
        motion_sync: flash[ADDR_MOTION_SYNC] == 1,
        performance: flash[ADDR_PERFORMANCE_ON] == 1,
        performance_time: flash[ADDR_PERFORMANCE_TIME],
        sleep_time: flash[ADDR_SLEEP],
        keys,
        ..MouseState::default()
    }
}
