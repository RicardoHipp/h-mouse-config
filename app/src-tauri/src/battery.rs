//! Battery history: records the battery of the mouse into a CSV file when switched on, estimates the time
//! left from the drain of the running discharge and of earlier ones, and warns through Windows.

use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State};

use crate::bluetooth::BluetoothHandle;
use crate::mouse::MouseHandle;
use crate::tray;

const SETTINGS_FILE: &str = "battery.json";
const LOG_FILE: &str = "battery.csv";
const LOG_HEADER: &str = "Zeit (UTC);Unix;Verbindung;Polling (Hz);Akku (%);Maus (%);Spannung (mV);Lädt";
/// Days to keep, 0 keeps everything.
const KEEP_CHOICES: [u32; 4] = [30, 90, 180, 0];
const LAST_WARNING: u8 = 5;
const HISTORY_DAYS: u64 = 7;
const MAX_SESSION_POINTS: usize = 20_000;
// A discharge counts for the history from 2 hours and 2 %, older ones count half after 30 days.
const MIN_HISTORY_HOURS: f64 = 2.0;
const MIN_HISTORY_DROP: u8 = 2;
const HALF_LIFE_DAYS: f64 = 30.0;
// After this long the running discharge alone decides the estimate. A whole day, so that one evening of
// gaming or one night does not tip it.
const FULL_TRUST_HOURS: f64 = 24.0;
const DAY: u64 = 86_400;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", default)]
pub struct BatterySettings {
    pub record: bool,
    pub keep_days: u32,
    pub warn_low: bool,
    pub notify_full: bool,
}

impl Default for BatterySettings {
    fn default() -> Self {
        Self { record: false, keep_days: 90, warn_low: true, notify_full: true }
    }
}

/// One reading; `key` is the polling rate, 0 for Bluetooth, because the drain depends on it.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Point {
    at: u64,
    level: u8,
    charging: bool,
    key: u32,
}

#[derive(Clone, Debug, PartialEq)]
struct Reading {
    point: Point,
    raw: Option<u8>,
    voltage: Option<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Estimate {
    hours_left: f64,
    cycles: usize,
    current_hours: f64,
}

#[derive(Debug, PartialEq)]
enum Notice {
    Low(u8),
    Full,
}

#[derive(Default)]
struct Warnings {
    low: bool,
    last: bool,
    full: bool,
}

struct Battery {
    settings: BatterySettings,
    // Readings of this run, so that an estimate works without the file too.
    session: Vec<Reading>,
    last: Option<Reading>,
    warnings: Warnings,
    pruned_at: u64,
}

pub struct BatteryHandle(Mutex<Battery>);

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatteryOverview {
    level: Option<u8>,
    charging: bool,
    bluetooth: bool,
    hours_left: Option<f64>,
    cycles: usize,
    current_hours: f64,
    /// Unix time, level and charging of the last days, for the chart.
    history: Vec<(u64, u8, bool)>,
    low_below: u8,
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |time| time.as_secs())
}

/// Unix time as "2026-11-04 09:15:00", in UTC, readable for spreadsheets without a date library.
fn utc_text(unix: u64) -> String {
    let days = (unix / DAY) as i64;
    let seconds = unix % DAY;
    // Civil date from days since 1970, after Howard Hinnant.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02}", seconds / 3600, seconds / 60 % 60, seconds % 60)
}

fn csv_line(reading: &Reading) -> String {
    let point = reading.point;
    let connection = if point.key == 0 { "Bluetooth" } else { "Empfänger/Kabel" };
    let optional = |value: Option<u32>| value.map_or(String::new(), |value| value.to_string());
    format!(
        "{};{};{connection};{};{};{};{};{}",
        utc_text(point.at),
        point.at,
        if point.key == 0 { String::new() } else { point.key.to_string() },
        point.level,
        optional(reading.raw.map(u32::from)),
        optional(reading.voltage),
        if point.charging { "ja" } else { "nein" }
    )
}

fn parse_line(line: &str) -> Option<Point> {
    let fields: Vec<&str> = line.split(';').collect();
    if fields.len() < 8 {
        return None;
    }
    Some(Point { at: fields[1].parse().ok()?, key: fields[3].parse().unwrap_or(0), level: fields[4].parse().ok()?, charging: fields[7] == "ja" })
}

/// Discharges between two charges. A level that rises a little after a pause, when the voltage recovers,
/// and a change of the polling rate stay within the same discharge.
fn discharges(points: &[Point]) -> Vec<&[Point]> {
    points.split(|point| point.charging).filter(|part| !part.is_empty()).collect()
}

/// Percent per hour from a straight line through all readings, so that a reading that rises after a pause or
/// one fast evening counts little. None for less than an hour or without any fall.
fn drain(points: &[Point]) -> Option<f64> {
    let first = points.first()?.at;
    let span = (points.last()?.at - first) as f64 / 3600.0;
    if points.len() < 2 || span < 1.0 {
        return None;
    }
    let hours: Vec<f64> = points.iter().map(|point| (point.at - first) as f64 / 3600.0).collect();
    let count = points.len() as f64;
    let mean_hours = hours.iter().sum::<f64>() / count;
    let mean_level = points.iter().map(|point| point.level as f64).sum::<f64>() / count;
    let (mut covariance, mut variance) = (0.0, 0.0);
    for (hour, point) in hours.iter().zip(points) {
        covariance += (hour - mean_hours) * (point.level as f64 - mean_level);
        variance += (hour - mean_hours).powi(2);
    }
    let rate = -covariance / variance;
    (variance > 0.0 && rate > 0.0).then_some(rate)
}

/// The polling rate that ran the longest in a discharge, because it decides the drain.
fn main_key(points: &[Point]) -> u32 {
    let mut time = std::collections::HashMap::<u32, u64>::new();
    for pair in points.windows(2) {
        *time.entry(pair[0].key).or_default() += pair[1].at - pair[0].at;
    }
    time.into_iter().max_by_key(|&(_, seconds)| seconds).map_or_else(|| points.last().map_or(0, |point| point.key), |(key, _)| key)
}

/// Earlier discharges at the same polling rate give a start value, the running one takes over the longer it
/// lasts. The time counts in real hours, nights and pauses included, so the estimate fits everyday use.
fn estimate(points: &[Point], now: u64) -> Option<Estimate> {
    let last = *points.last()?;
    if last.charging || last.level == 0 {
        return None;
    }
    let all = discharges(points);
    let (current, earlier) = all.split_last()?;
    // No reading since the last one means the level held until now, which tells as much as a fall.
    let mut running = current.to_vec();
    running.push(Point { at: now.max(last.at), ..last });
    let current_hours = now.saturating_sub(current[0].at) as f64 / 3600.0;
    let rate_now = drain(&running);
    let key = main_key(&running);

    let (mut weighted, mut weights, mut cycles) = (0.0, 0.0, 0);
    for discharge in earlier {
        let (first, end) = (discharge[0], discharge[discharge.len() - 1]);
        let hours = (end.at - first.at) as f64 / 3600.0;
        if main_key(discharge) != key || hours < MIN_HISTORY_HOURS || first.level.saturating_sub(end.level) < MIN_HISTORY_DROP {
            continue;
        }
        let Some(rate) = drain(discharge) else { continue };
        let age_days = now.saturating_sub(end.at) as f64 / DAY as f64;
        let weight = hours * 0.5f64.powf(age_days / HALF_LIFE_DAYS);
        weighted += weight * rate;
        weights += weight;
        cycles += 1;
    }
    let history = (weights > 0.0).then(|| weighted / weights);
    let rate = match (rate_now, history) {
        (Some(now_rate), Some(earlier)) => {
            let trust = (current_hours / FULL_TRUST_HOURS).min(1.0);
            trust * now_rate + (1.0 - trust) * earlier
        }
        (Some(now_rate), None) if current_hours >= 1.0 => now_rate,
        (None, Some(earlier)) => earlier,
        _ => return None,
    };
    (rate > 0.0).then(|| Estimate { hours_left: last.level as f64 / rate, cycles, current_hours })
}

/// Each warning once: at the red tray threshold, again at the last few percent, and once full while charging.
fn notice(warnings: &mut Warnings, settings: &BatterySettings, level: u8, charging: bool, low_below: u8) -> Option<Notice> {
    if charging {
        warnings.low = false;
        warnings.last = false;
        if level >= 100 && !warnings.full {
            warnings.full = true;
            return settings.notify_full.then_some(Notice::Full);
        }
        return None;
    }
    warnings.full = false;
    if level < LAST_WARNING && !warnings.last {
        (warnings.last, warnings.low) = (true, true);
        return settings.warn_low.then_some(Notice::Low(level));
    }
    if level < low_below && !warnings.low {
        warnings.low = true;
        return settings.warn_low.then_some(Notice::Low(level));
    }
    None
}

fn data_dir(app: &AppHandle) -> Option<std::path::PathBuf> {
    let dir = app.path().app_data_dir().ok()?;
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

fn read_log(app: &AppHandle) -> Vec<Point> {
    let Some(text) = data_dir(app).and_then(|dir| std::fs::read_to_string(dir.join(LOG_FILE)).ok()) else { return Vec::new() };
    text.lines().skip(1).filter_map(parse_line).collect()
}

fn append_log(app: &AppHandle, lines: &[String]) {
    use std::io::Write;
    let Some(path) = data_dir(app).map(|dir| dir.join(LOG_FILE)) else { return };
    let new = !path.exists();
    let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(&path) else { return };
    if new {
        let _ = writeln!(file, "{LOG_HEADER}");
    }
    for line in lines {
        let _ = writeln!(file, "{line}");
    }
}

/// Drops the rows older than the chosen days, the rest of the file stays as it is.
fn prune_log(app: &AppHandle, keep_days: u32) {
    if keep_days == 0 {
        return;
    }
    let Some(path) = data_dir(app).map(|dir| dir.join(LOG_FILE)) else { return };
    let Ok(text) = std::fs::read_to_string(&path) else { return };
    let oldest = now().saturating_sub(keep_days as u64 * DAY);
    let kept: Vec<&str> = text.lines().skip(1).filter(|line| parse_line(line).is_some_and(|point| point.at >= oldest)).collect();
    if kept.len() + 1 < text.lines().count() {
        let _ = std::fs::write(&path, format!("{LOG_HEADER}\n{}{}", kept.join("\n"), if kept.is_empty() { "" } else { "\n" }));
    }
}

fn load_settings(app: &AppHandle) -> BatterySettings {
    let text = data_dir(app).and_then(|dir| std::fs::read_to_string(dir.join(SETTINGS_FILE)).ok());
    text.and_then(|text| serde_json::from_str::<BatterySettings>(&text).ok()).filter(|settings| KEEP_CHOICES.contains(&settings.keep_days)).unwrap_or_default()
}

pub fn create(app: &AppHandle) {
    let settings = load_settings(app);
    if settings.record {
        prune_log(app, settings.keep_days);
    }
    app.manage(BatteryHandle(Mutex::new(Battery { settings, session: Vec::new(), last: None, warnings: Warnings::default(), pruned_at: now() })));
}

/// The reading now: Bluetooth wins like everywhere else, it carries the mouse while it is connected.
fn reading(app: &AppHandle) -> Option<Reading> {
    let at = now();
    if let Some(level) = app.try_state::<BluetoothHandle>().map(|handle| handle.state()).filter(|state| state.connected).and_then(|state| state.battery) {
        return Some(Reading { point: Point { at, level, charging: false, key: 0 }, raw: None, voltage: None });
    }
    let state = app.try_state::<MouseHandle>()?.state();
    let level = state.battery.filter(|_| state.connected)?;
    Some(Reading { point: Point { at, level, charging: state.charging, key: state.polling }, raw: state.battery_raw, voltage: state.battery_voltage })
}

/// Called with every change of the mouse or of Bluetooth. Only a new level, charging state or polling rate
/// makes a new row, the time between rows tells the drain.
pub fn observe(app: &AppHandle) {
    let Some(handle) = app.try_state::<BatteryHandle>() else { return };
    let Some(reading) = reading(app) else { return };
    let low_below = tray::low_threshold(app);
    let (line, notice, prune) = {
        let mut battery = handle.0.lock().unwrap();
        let same = |last: &Reading| {
            let (a, b) = (last.point, reading.point);
            (a.level, a.charging, a.key) == (b.level, b.charging, b.key)
        };
        if battery.last.as_ref().is_some_and(same) {
            return;
        }
        battery.session.push(reading.clone());
        if battery.session.len() > MAX_SESSION_POINTS {
            battery.session.remove(0);
        }
        let Battery { settings, warnings, .. } = &mut *battery;
        let notice = notice(warnings, settings, reading.point.level, reading.point.charging, low_below);
        let line = battery.settings.record.then(|| csv_line(&reading));
        let prune = battery.settings.record && reading.point.at >= battery.pruned_at + DAY;
        if prune {
            battery.pruned_at = reading.point.at;
        }
        battery.last = Some(reading);
        (line, notice, prune.then_some(battery.settings.keep_days))
    };
    if let Some(line) = line {
        append_log(app, &[line]);
    }
    if let Some(keep_days) = prune {
        prune_log(app, keep_days);
    }
    if let Some(notice) = notice {
        let english = tray::english(app);
        let (title, text) = match (notice, english) {
            (Notice::Low(level), false) => ("Akku fast leer".to_string(), format!("Die Hypace hat noch {level} %. Bitte bald laden.")),
            (Notice::Low(level), true) => ("Battery low".to_string(), format!("The Hypace has {level} % left. Please charge it soon.")),
            (Notice::Full, false) => ("Akku voll geladen".to_string(), "Die Hypace ist auf 100 % geladen.".to_string()),
            (Notice::Full, true) => ("Battery full".to_string(), "The Hypace is charged to 100 %.".to_string()),
        };
        toast::show(title, text);
    }
}

#[cfg(windows)]
mod toast {
    use windows::core::HSTRING;
    use windows::Data::Xml::Dom::XmlDocument;
    use windows::UI::Notifications::{ToastNotification, ToastNotificationManager};
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    const APP_ID: &str = "io.github.ricardohipp.h-mouse-config";
    const ICON: &[u8] = include_bytes!("../icons/icon.ico");

    fn escape(text: &str) -> String {
        text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
    }

    /// The PNG inside the icon file, the first image of the ICO directory.
    fn icon_png() -> Option<&'static [u8]> {
        let size = u32::from_le_bytes(ICON.get(14..18)?.try_into().ok()?) as usize;
        let offset = u32::from_le_bytes(ICON.get(18..22)?.try_into().ok()?) as usize;
        ICON.get(offset..offset + size).filter(|png| png.starts_with(b"\x89PNG"))
    }

    /// Windows shows toasts of an app without installer only with a name registered for its id.
    fn register() {
        let Ok((key, _)) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(format!(r"Software\Classes\AppUserModelId\{APP_ID}")) else { return };
        let _ = key.set_value("DisplayName", &"H-Mouse Config");
        if let Some(png) = icon_png() {
            let path = std::env::temp_dir().join("h-mouse-config-icon.png");
            if std::fs::write(&path, png).is_ok() {
                let _ = key.set_value("IconUri", &path.display().to_string());
            }
        }
    }

    pub fn show(title: String, text: String) {
        std::thread::spawn(move || unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            register();
            let xml = format!(r#"<toast><visual><binding template="ToastGeneric"><text>{}</text><text>{}</text></binding></visual></toast>"#, escape(&title), escape(&text));
            let shown = (|| -> windows::core::Result<()> {
                let document = XmlDocument::new()?;
                document.LoadXml(&HSTRING::from(xml))?;
                let toast = ToastNotification::CreateToastNotification(&document)?;
                ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(APP_ID))?.Show(&toast)
            })();
            let _ = shown;
        });
    }
}

#[cfg(not(windows))]
mod toast {
    pub fn show(_title: String, _text: String) {}
}

#[tauri::command(async)]
pub fn get_battery_settings(handle: State<'_, BatteryHandle>) -> BatterySettings {
    handle.0.lock().unwrap().settings.clone()
}

#[tauri::command(async)]
pub fn save_battery_settings(app: AppHandle, handle: State<'_, BatteryHandle>, settings: BatterySettings) -> Result<(), String> {
    if !KEEP_CHOICES.contains(&settings.keep_days) {
        return Err(format!("Ungültige Aufbewahrung {} Tage.", settings.keep_days));
    }
    let dir = data_dir(&app).ok_or("Kein Datenordner.")?;
    std::fs::write(dir.join(SETTINGS_FILE), serde_json::to_string(&settings).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let started = {
        let mut battery = handle.0.lock().unwrap();
        let started = settings.record && !battery.settings.record;
        battery.settings = settings.clone();
        // What this run has seen so far goes into the file too, so that switching on loses nothing.
        started.then(|| battery.session.iter().map(csv_line).collect::<Vec<_>>())
    };
    if let Some(lines) = started {
        append_log(&app, &lines);
    }
    if settings.record {
        prune_log(&app, settings.keep_days);
    }
    Ok(())
}

#[tauri::command(async)]
pub fn get_battery_overview(app: AppHandle, handle: State<'_, BatteryHandle>) -> BatteryOverview {
    let (record, session) = {
        let battery = handle.0.lock().unwrap();
        (battery.settings.record, battery.session.iter().map(|reading| reading.point).collect::<Vec<_>>())
    };
    let points = if record { read_log(&app) } else { session };
    let at = now();
    let estimate = estimate(&points, at);
    let current = reading(&app);
    BatteryOverview {
        level: current.as_ref().map(|reading| reading.point.level),
        charging: current.as_ref().is_some_and(|reading| reading.point.charging),
        bluetooth: current.as_ref().is_some_and(|reading| reading.point.key == 0),
        hours_left: estimate.map(|estimate| estimate.hours_left),
        cycles: estimate.map_or(0, |estimate| estimate.cycles),
        current_hours: estimate.map_or(0.0, |estimate| estimate.current_hours),
        history: points.iter().filter(|point| point.at + HISTORY_DAYS * DAY >= at).map(|point| (point.at, point.level, point.charging)).collect(),
        low_below: tray::low_threshold(&app),
    }
}

#[tauri::command(async)]
pub fn open_battery_folder(app: AppHandle) -> Result<(), String> {
    let dir = data_dir(&app).ok_or("Kein Datenordner.")?;
    std::process::Command::new("explorer").arg(dir).spawn().map(|_| ()).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOUR: u64 = 3600;

    fn point(hours: u64, level: u8, charging: bool, key: u32) -> Point {
        Point { at: hours * HOUR, level, charging, key }
    }

    #[test]
    fn utc_text_reads_like_a_date() {
        assert_eq!(utc_text(0), "1970-01-01 00:00:00");
        assert_eq!(utc_text(1_793_772_900), "2026-11-04 06:15:00");
    }

    #[test]
    fn rows_survive_the_file() {
        let reading = Reading { point: Point { at: 1_793_772_900, level: 87, charging: false, key: 4000 }, raw: Some(90), voltage: Some(4012) };
        assert_eq!(parse_line(&csv_line(&reading)), Some(reading.point));
        let bluetooth = Reading { point: Point { at: 5, level: 40, charging: true, key: 0 }, raw: None, voltage: None };
        assert_eq!(parse_line(&csv_line(&bluetooth)), Some(bluetooth.point));
    }

    #[test]
    fn only_charging_ends_a_discharge() {
        let points = [point(0, 100, false, 1000), point(10, 90, false, 1000), point(11, 90, true, 1000), point(12, 100, false, 1000), point(13, 99, false, 4000), point(14, 100, false, 4000)];
        let found = discharges(&points);
        assert_eq!(found.len(), 2);
        assert_eq!(found[1].len(), 3, "a rise and a new polling rate stay in the discharge");
        assert_eq!(main_key(&[point(0, 90, false, 250), point(5, 85, false, 1000), point(6, 84, false, 250)]), 250);
    }

    #[test]
    fn a_level_that_rises_after_a_pause_does_not_reset_the_estimate() {
        // Like the real recording: 40 % in four days, with small rises after pauses and fast evenings.
        let points = [point(0, 100, false, 250), point(24, 90, false, 250), point(25, 92, false, 250), point(48, 80, false, 250), point(70, 69, false, 250), point(72, 70, false, 250), point(96, 60, false, 250)];
        let found = estimate(&points, 96 * HOUR).unwrap();
        assert!((120.0..170.0).contains(&found.hours_left), "{found:?}");
        assert_eq!(found.cycles, 0);
    }

    #[test]
    fn estimate_starts_from_history_and_follows_the_running_discharge() {
        // Earlier: 1 % per hour at 4000 Hz. Now: unplugged 1 hour ago at 80 %, nothing seen yet.
        let mut points = vec![point(0, 100, false, 4000), point(20, 80, false, 4000), point(21, 80, true, 4000), point(100, 80, false, 4000)];
        let early = estimate(&points, 101 * HOUR).unwrap();
        assert!((early.hours_left - 80.0).abs() < 1.0, "{early:?}");
        assert_eq!(early.cycles, 1);
        // Six hours later it fell 12 %: twice as fast, but after a quarter day the history still counts most.
        points.push(point(106, 68, false, 4000));
        let later = estimate(&points, 106 * HOUR).unwrap();
        assert!((later.hours_left - 54.4).abs() < 1.0, "{later:?}");
        // While charging there is nothing to estimate.
        points.push(point(107, 68, true, 4000));
        assert_eq!(estimate(&points, 107 * HOUR), None);
    }

    #[test]
    fn estimate_needs_some_drain_without_history() {
        let points = [point(0, 100, false, 1000), point(0, 99, false, 1000)];
        assert_eq!(estimate(&points, HOUR / 2), None);
        assert!(estimate(&points, 2 * HOUR).is_some());
    }

    #[test]
    fn warnings_come_once_each() {
        let settings = BatterySettings::default();
        let mut warnings = Warnings::default();
        assert_eq!(notice(&mut warnings, &settings, 25, false, 20), None);
        assert_eq!(notice(&mut warnings, &settings, 19, false, 20), Some(Notice::Low(19)));
        assert_eq!(notice(&mut warnings, &settings, 18, false, 20), None);
        assert_eq!(notice(&mut warnings, &settings, 4, false, 20), Some(Notice::Low(4)));
        assert_eq!(notice(&mut warnings, &settings, 3, false, 20), None);
        assert_eq!(notice(&mut warnings, &settings, 50, true, 20), None);
        assert_eq!(notice(&mut warnings, &settings, 100, true, 20), Some(Notice::Full));
        assert_eq!(notice(&mut warnings, &settings, 100, true, 20), None);
        assert_eq!(notice(&mut warnings, &settings, 15, false, 20), Some(Notice::Low(15)), "armed again after charging");
    }
}
