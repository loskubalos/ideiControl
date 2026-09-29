//! Warstwa komunikacji USB Custom HID (zastępuje CDC / serialport).
//!
//! Protokół:
//! - Report 0x01 Feature — handshake / Get Info
//! - Report 0x02 Input — suwaki + surowa maska wciśniętych przycisków (STM32 → PC)
//! - Report 0x03 Output — NeoPixel LED (PC → STM32)
//!
//! Przyciski (bajt 11): 1 = wciśnięty, 0 = puszczony. Firmware nie wycisza suwaków —
//! mute kanału i media/shortcut realizuje aplikacja według profilu.

use crate::audio::{volume_targets_conflict, VolumeTarget};
use crate::config::{
    apply_profile_to_flat_config, assignments_to_vec, default_shortcut_led_modes, ensure_profiles,
    find_profile, find_profile_mut, load_config, profile_from_config, profiles_state, save_config,
    AppConfig, DeviceRuntimeConfig, Profile, ProfileData, ProfilesState, ShortcutLedMode,
    DEFAULT_PROFILE_NAME,
};
use crate::media_keys::{default_button_bindings, send_button_binding, ButtonBinding, MEDIA_BUTTON_SLOTS};
use hidapi::{HidApi, HidDevice};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use tauri::{AppHandle, Emitter};

/// STMicroelectronics vendor + custom HID product (firmware migration).
pub const IDEI_VID: u16 = 0x0483;
pub const IDEI_PID: u16 = 0x5750;
const IDEI_USAGE_PAGE: u16 = 0xFF00;
const IDEI_USAGE: u16 = 0x01;

const REPORT_INFO: u8 = 0x01;
const REPORT_STATE: u8 = 0x02;
const REPORT_LED: u8 = 0x03;

const CMD_GET_INFO: u8 = 0x01;
const INPUT_REPORT_LEN: usize = 13;
const LED_REPORT_LEN: usize = 6;
const INFO_REPORT_MIN: usize = 7;

const READ_TIMEOUT_MS: i32 = 50;
const HOTPLUG_POLL_MS: u64 = 500;
/// Worker głośności Windows — poza wątkiem HID, żeby Core Audio nie blokowało odczytu.
const APPLY_LOOP_MS: u64 = 12;

const MUTE_LED_R: u8 = 255;
const MUTE_LED_G: u8 = 24;
const MUTE_LED_B: u8 = 24;

#[derive(Clone, Debug)]
enum HidOutCmd {
    /// Output report 0x03
    Led {
        index: u8,
        r: u8,
        g: u8,
        b: u8,
        flags: u8,
    },
    /// Ponów Feature Get Info (handshake)
    GetInfo,
    /// Wszystkie LED off (RESET_DEFAULTS po stronie HW)
    ResetLeds,
}

#[derive(Clone, Serialize)]
pub struct HidDeviceInfo {
    pub path: String,
    pub label: String,
}

#[derive(Clone, Serialize)]
pub struct DeviceInfo {
    /// Identyfikator sesji = ścieżka HID (pole historycznie nazywa się `port` dla UI).
    pub port: String,
    pub model: String,
    pub sliders: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proto: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fw: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub buttons: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub caps: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uid: Option<String>,
}

#[derive(Clone, Serialize)]
pub struct DeviceButtonPayload {
    pub port: String,
    pub index: u32,
    pub seq: u64,
}

#[derive(Clone, Serialize)]
pub struct SliderValuesPayload {
    pub port: String,
    pub values: Vec<u16>,
}

#[derive(Clone, Serialize)]
pub struct ShortcutLedLatchedPayload {
    pub port: String,
    pub states: Vec<bool>,
}

#[derive(Clone)]
struct SessionHandle {
    device: DeviceInfo,
    stop_requested: Arc<AtomicBool>,
    last_slider_values: Arc<Mutex<[u16; crate::audio::MAX_SLIDERS]>>,
    /// Surowa maska wciśniętych przycisków z raportu 0x02 (bit i = przycisk i wciśnięty).
    #[allow(dead_code)]
    button_mask: Arc<Mutex<u8>>,
    /// Programowy mute kanału (toggle przy `button_hw_slider_mute[i]`).
    channel_mute: Arc<Mutex<[bool; MEDIA_BUTTON_SLOTS]>>,
    volume_assignments: Arc<Mutex<[Vec<VolumeTarget>; crate::audio::MAX_SLIDERS]>>,
    button_bindings: Arc<Mutex<[ButtonBinding; MEDIA_BUTTON_SLOTS]>>,
    button_hw_slider_mute: Arc<Mutex<[bool; MEDIA_BUTTON_SLOTS]>>,
    /// Off / Momentary / Toggle dla Media/Shortcut LED.
    shortcut_led_mode: Arc<Mutex<[ShortcutLedMode; MEDIA_BUTTON_SLOTS]>>,
    /// Latched ON dla trybu Toggle (nie dotyczy mute kanału).
    shortcut_led_latched: Arc<Mutex<[bool; MEDIA_BUTTON_SLOTS]>>,
    hid_cmd_tx: Arc<Mutex<Option<mpsc::Sender<HidOutCmd>>>>,
    /// Ustawiane przez wątek HID; czyści worker apply (Core Audio poza kolejką HID).
    volume_dirty: Arc<AtomicBool>,
}

struct SessionRuntime {
    handle: SessionHandle,
    read_thread: Option<JoinHandle<()>>,
    apply_thread: Option<JoinHandle<()>>,
}

pub struct HidState {
    sessions: Arc<Mutex<HashMap<String, SessionRuntime>>>,
    pub active_port: Arc<Mutex<Option<String>>>,
    pub last_connected_port: Arc<Mutex<Option<String>>>,
    pub default_assignments: Arc<Mutex<[Vec<VolumeTarget>; crate::audio::MAX_SLIDERS]>>,
    pub default_button_bindings: Arc<Mutex<[ButtonBinding; MEDIA_BUTTON_SLOTS]>>,
    pub default_button_hw_slider_mute: Arc<Mutex<[bool; MEDIA_BUTTON_SLOTS]>>,
    pub default_shortcut_led_mode: Arc<Mutex<[ShortcutLedMode; MEDIA_BUTTON_SLOTS]>>,
    pub button_media_keys: Arc<AtomicBool>,
    connect_mutex: Arc<Mutex<()>>,
    session_connect_order: Arc<Mutex<Vec<String>>>,
    /// Shared HidApi — tworzony raz w monitorze / skanie.
    api: Arc<Mutex<Option<HidApi>>>,
}

fn default_assignments() -> [Vec<VolumeTarget>; crate::audio::MAX_SLIDERS] {
    std::array::from_fn(|_| vec![])
}

fn refresh_api(api: &mut HidApi) {
    let _ = api.refresh_devices();
}

fn is_idei_hid(info: &hidapi::DeviceInfo) -> bool {
    if info.vendor_id() != IDEI_VID || info.product_id() != IDEI_PID {
        return false;
    }
    // Prefer custom collection; allow match when usage is unset (some backends).
    let page = info.usage_page();
    let usage = info.usage();
    (page == IDEI_USAGE_PAGE && usage == IDEI_USAGE) || (page == 0 && usage == 0)
}

fn path_to_string(path: &std::ffi::CStr) -> String {
    path.to_string_lossy().into_owned()
}

fn short_hid_label(path: &str, product: Option<&str>) -> String {
    let prod = product.unwrap_or("ideiMx").trim();
    let prod = if prod.is_empty() { "ideiMx" } else { prod };
    // Windows paths are long — show a short suffix.
    let tail = path
        .rsplit(['\\', '/', '#'])
        .find(|s| !s.is_empty() && s.len() > 2)
        .unwrap_or(path);
    let tail = if tail.len() > 18 {
        &tail[tail.len().saturating_sub(18)..]
    } else {
        tail
    };
    format!("{prod} ({tail})")
}

fn enumerate_idei(api: &HidApi) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for info in api.device_list().filter(|d| is_idei_hid(d)) {
        let path = path_to_string(info.path());
        if out.iter().any(|(p, _)| p == &path) {
            continue;
        }
        let product = info.product_string();
        let label = short_hid_label(&path, product.as_deref());
        out.push((path, label));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

fn model_from_slider_count(n: u32) -> String {
    match n {
        5 => "ideiMx-max".to_string(),
        3 => "ideiMx".to_string(),
        _ => format!("ideiMx-{n}"),
    }
}

fn parse_info_report(buf: &[u8], path: &str) -> Option<DeviceInfo> {
    if buf.len() < INFO_REPORT_MIN || buf[0] != REPORT_INFO {
        return None;
    }
    // Bajt 1: ACK 0x01
    if buf[1] != 0x01 {
        return None;
    }
    let major = buf[2];
    let minor = buf[3];
    let patch = buf[4];
    let sliders = (buf[5] as u32).clamp(1, crate::audio::MAX_SLIDERS as u32);
    let _flags = buf[6];
    Some(DeviceInfo {
        port: path.to_string(),
        model: model_from_slider_count(sliders),
        sliders,
        proto: Some(4), // HID binary protocol
        fw: Some(format!("{major}.{minor}.{patch}")),
        buttons: Some(sliders),
        caps: Some(vec!["hid".into(), "neopixel".into()]),
        uid: None,
    })
}

fn do_handshake(dev: &HidDevice, path: &str) -> Result<DeviceInfo, String> {
    // Feature: CMD_GET_INFO
    let req = [REPORT_INFO, CMD_GET_INFO];
    dev.send_feature_report(&req)
        .map_err(|e| format!("HID Get Info (send): {e}"))?;

    let mut buf = [0u8; 64];
    buf[0] = REPORT_INFO;
    let n = dev
        .get_feature_report(&mut buf)
        .map_err(|e| format!("HID Get Info (recv): {e}"))?;
    if n < INFO_REPORT_MIN {
        return Err("Urządzenie nie zwróciło pełnego raportu Get Info".to_string());
    }
    parse_info_report(&buf[..n], path)
        .ok_or_else(|| "Nieprawidłowa odpowiedź Get Info (ACK)".to_string())
}

fn parse_state_report(buf: &[u8]) -> Option<(Vec<u16>, u8, u8)> {
    // Bajt 11 = surowa maska wciśniętych przycisków (nie sticky mute FW).
    if buf.len() < INPUT_REPORT_LEN || buf[0] != REPORT_STATE {
        return None;
    }
    let mut values = Vec::with_capacity(5);
    for i in 0..5 {
        let lo = buf[1 + i * 2] as u16;
        let hi = buf[2 + i * 2] as u16;
        values.push((lo | (hi << 8)).min(1023));
    }
    let buttons = buf[11];
    let status = buf[12];
    Some((values, buttons, status))
}

fn write_led_report(dev: &HidDevice, index: u8, r: u8, g: u8, b: u8, flags: u8) -> Result<(), String> {
    let report = [REPORT_LED, index, r, g, b, flags];
    debug_assert_eq!(report.len(), LED_REPORT_LEN);
    dev.write(&report)
        .map_err(|e| format!("HID LED write: {e}"))?;
    Ok(())
}

fn enqueue_cmd(queue: &Arc<Mutex<Option<mpsc::Sender<HidOutCmd>>>>, cmd: HidOutCmd) {
    if let Ok(g) = queue.lock() {
        if let Some(ref tx) = *g {
            let _ = tx.send(cmd);
        }
    }
}

fn sync_channel_mute_leds(
    queue: &Arc<Mutex<Option<mpsc::Sender<HidOutCmd>>>>,
    channel_mute: &[bool; MEDIA_BUTTON_SLOTS],
) {
    for i in 0..MEDIA_BUTTON_SLOTS {
        if channel_mute[i] {
            enqueue_cmd(
                queue,
                HidOutCmd::Led {
                    index: i as u8,
                    r: MUTE_LED_R,
                    g: MUTE_LED_G,
                    b: MUTE_LED_B,
                    flags: 0,
                },
            );
        } else {
            enqueue_cmd(
                queue,
                HidOutCmd::Led {
                    index: i as u8,
                    r: 0,
                    g: 0,
                    b: 0,
                    flags: 0,
                },
            );
        }
    }
}

fn set_slot_led(
    queue: &Arc<Mutex<Option<mpsc::Sender<HidOutCmd>>>>,
    index: usize,
    on: bool,
) {
    if index >= MEDIA_BUTTON_SLOTS {
        return;
    }
    if on {
        enqueue_cmd(
            queue,
            HidOutCmd::Led {
                index: index as u8,
                r: MUTE_LED_R,
                g: MUTE_LED_G,
                b: MUTE_LED_B,
                flags: 0,
            },
        );
    } else {
        enqueue_cmd(
            queue,
            HidOutCmd::Led {
                index: index as u8,
                r: 0,
                g: 0,
                b: 0,
                flags: 0,
            },
        );
    }
}

/// Natychmiastowe mapowanie głośności — wyłącznie z workera / komend UI (NIE z wątku HID).
fn apply_volumes_now(
    values: &[u16; crate::audio::MAX_SLIDERS],
    channel_mute: &[bool; MEDIA_BUTTON_SLOTS],
    assignments: &[Vec<VolumeTarget>; crate::audio::MAX_SLIDERS],
    slider_count: usize,
) {
    let mut effective = *values;
    let n = slider_count.min(crate::audio::MAX_SLIDERS);
    for i in 0..n.min(MEDIA_BUTTON_SLOTS) {
        if channel_mute[i] {
            effective[i] = 0;
        }
    }
    crate::audio::apply_volume_mapping(&effective, assignments, n);
}

#[inline]
fn emit_led_latched(app: &AppHandle, path: &str, latched: &[bool; MEDIA_BUTTON_SLOTS]) {
    let payload = ShortcutLedLatchedPayload {
        port: path.to_string(),
        states: latched.to_vec(),
    };
    let _ = app.emit("shortcut-led-latched", &payload);
    let _ = app.emit("shortcut-led-latched-port", &payload);
}

#[inline]
fn mark_volume_dirty(dirty: &AtomicBool) {
    dirty.store(true, Ordering::Release);
}

fn clear_shortcut_led_latched(
    app: Option<&AppHandle>,
    path: Option<&str>,
    latched: &Arc<Mutex<[bool; MEDIA_BUTTON_SLOTS]>>,
    cmd_tx: &Arc<Mutex<Option<mpsc::Sender<HidOutCmd>>>>,
    hw: &[bool; MEDIA_BUTTON_SLOTS],
    channel: &[bool; MEDIA_BUTTON_SLOTS],
) {
    if let Ok(mut g) = latched.lock() {
        *g = [false; MEDIA_BUTTON_SLOTS];
        if let (Some(app), Some(path)) = (app, path) {
            emit_led_latched(app, path, &*g);
        }
    }
    for i in 0..MEDIA_BUTTON_SLOTS {
        if hw[i] {
            set_slot_led(cmd_tx, i, channel[i]);
        } else {
            set_slot_led(cmd_tx, i, false);
        }
    }
}

impl HidState {
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(Mutex::new(HashMap::new())),
            active_port: Arc::new(Mutex::new(None)),
            last_connected_port: Arc::new(Mutex::new(None)),
            default_assignments: Arc::new(Mutex::new(default_assignments())),
            default_button_bindings: Arc::new(Mutex::new(std::array::from_fn(|_| ButtonBinding::None))),
            default_button_hw_slider_mute: Arc::new(Mutex::new([false; MEDIA_BUTTON_SLOTS])),
            default_shortcut_led_mode: Arc::new(Mutex::new(default_shortcut_led_modes())),
            button_media_keys: Arc::new(AtomicBool::new(false)),
            connect_mutex: Arc::new(Mutex::new(())),
            session_connect_order: Arc::new(Mutex::new(Vec::new())),
            api: Arc::new(Mutex::new(None)),
        }
    }
}

fn get_session(state: &HidState, port_name: Option<String>) -> Result<SessionHandle, String> {
    let port = if let Some(p) = port_name {
        p
    } else {
        state
            .active_port
            .lock()
            .map_err(|e| e.to_string())?
            .clone()
            .ok_or_else(|| "Brak aktywnego urządzenia".to_string())?
    };
    let sessions = state.sessions.lock().map_err(|e| e.to_string())?;
    sessions
        .get(&port)
        .map(|s| s.handle.clone())
        .ok_or_else(|| format!("Brak połączenia dla {port}"))
}

fn device_identity_key(device: &DeviceInfo) -> String {
    if let Some(uid) = &device.uid {
        let u = uid.trim();
        if !u.is_empty() {
            return format!("uid:{u}");
        }
    }
    format!(
        "fallback:{}|s{}|b{}",
        device.model,
        device.sliders,
        device.buttons.unwrap_or(0)
    )
}

fn sanitize_assignments_against(
    src: &[Vec<VolumeTarget>; crate::audio::MAX_SLIDERS],
    blocked: &[VolumeTarget],
) -> [Vec<VolumeTarget>; crate::audio::MAX_SLIDERS] {
    let mut out: [Vec<VolumeTarget>; crate::audio::MAX_SLIDERS] = std::array::from_fn(|_| vec![]);
    let mut used: Vec<VolumeTarget> = blocked.to_vec();
    for i in 0..crate::audio::MAX_SLIDERS {
        let mut row: Vec<VolumeTarget> = vec![];
        for t in &src[i] {
            if used.iter().any(|u| volume_targets_conflict(t, u)) {
                continue;
            }
            used.push(t.clone());
            row.push(t.clone());
        }
        out[i] = row;
    }
    out
}

fn flatten_assignment_targets(a: &[Vec<VolumeTarget>; crate::audio::MAX_SLIDERS]) -> Vec<VolumeTarget> {
    a.iter().flat_map(|r| r.iter().cloned()).collect()
}

fn ordered_session_ports(state: &HidState) -> Vec<String> {
    let sessions = match state.sessions.lock() {
        Ok(m) => m,
        Err(_) => return Vec::new(),
    };
    let order = match state.session_connect_order.lock() {
        Ok(g) => g,
        Err(_) => return Vec::new(),
    };
    let keys: HashSet<String> = sessions.keys().cloned().collect();
    let mut out: Vec<String> = Vec::new();
    for p in order.iter() {
        if keys.contains(p) && !out.contains(p) {
            out.push(p.clone());
        }
    }
    let mut rest: Vec<String> = sessions.keys().filter(|k| !out.contains(k)).cloned().collect();
    rest.sort();
    out.extend(rest);
    out
}

fn reconcile_global_volume_assignments(state: &HidState) {
    let ports = ordered_session_ports(state);
    if ports.is_empty() {
        return;
    }
    let mut used: Vec<VolumeTarget> = Vec::new();
    for port in &ports {
        let current = {
            let sessions = match state.sessions.lock() {
                Ok(m) => m,
                Err(_) => return,
            };
            let Some(rt) = sessions.get(port) else {
                continue;
            };
            rt.handle
                .volume_assignments
                .lock()
                .ok()
                .map(|g| g.clone())
                .unwrap_or_else(default_assignments)
        };
        let new_assignments = sanitize_assignments_against(&current, &used);
        used.extend(flatten_assignment_targets(&new_assignments));
        if let Ok(sessions) = state.sessions.lock() {
            if let Some(rt) = sessions.get(port) {
                if let Ok(mut g) = rt.handle.volume_assignments.lock() {
                    *g = new_assignments;
                }
            }
        }
    }
}

fn do_scan_idei_paths(state: &HidState) -> Vec<String> {
    let mut guard = match state.api.lock() {
        Ok(g) => g,
        Err(_) => return vec![],
    };
    let api = match guard.as_mut() {
        Some(a) => a,
        None => match HidApi::new() {
            Ok(a) => {
                *guard = Some(a);
                guard.as_mut().unwrap()
            }
            Err(_) => return vec![],
        },
    };
    refresh_api(api);
    enumerate_idei(api).into_iter().map(|(p, _)| p).collect()
}

/// Lista znalezionych urządzeń IDEI (ścieżki HID).
#[tauri::command]
pub fn list_hid_devices(state: tauri::State<HidState>) -> Result<Vec<HidDeviceInfo>, String> {
    let paths = {
        let mut guard = state.api.lock().map_err(|e| e.to_string())?;
        if guard.is_none() {
            *guard = Some(HidApi::new().map_err(|e| format!("HID init: {e}"))?);
        }
        let api = guard.as_mut().unwrap();
        refresh_api(api);
        enumerate_idei(api)
    };
    Ok(paths
        .into_iter()
        .map(|(path, label)| HidDeviceInfo { path, label })
        .collect())
}

/// Szybka lista ścieżek (hot-plug / UI).
#[tauri::command]
pub fn get_device_paths(state: tauri::State<HidState>) -> Result<Vec<String>, String> {
    Ok(do_scan_idei_paths(&state))
}

#[tauri::command]
pub fn scan_idei_devices(state: tauri::State<HidState>) -> Result<Vec<String>, String> {
    Ok(do_scan_idei_paths(&state))
}

#[tauri::command]
pub fn start_scan_idei_devices(app: AppHandle, state: tauri::State<HidState>) -> Result<(), String> {
    let list = do_scan_idei_paths(&state);
    let app = app.clone();
    thread::spawn(move || {
        thread::sleep(Duration::from_millis(80));
        app.emit("scan-complete", list).ok();
    });
    Ok(())
}

/// Alias kompatybilności — UI / stare wywołania.
#[tauri::command]
pub fn start_scan_idei_ports(app: AppHandle, state: tauri::State<HidState>) -> Result<(), String> {
    start_scan_idei_devices(app, state)
}

/// Hot-plug: odłączenie + auto-reconnect gdy pojawi się dokładnie jedno urządzenie / znana ścieżka.
pub fn start_hid_presence_monitor(app: AppHandle, state: &HidState) {
    let sessions = state.sessions.clone();
    let active_port = state.active_port.clone();
    let connect_order = state.session_connect_order.clone();
    let api_slot = state.api.clone();
    let last_connected = state.last_connected_port.clone();
    let state_for_connect = HidStateSnapshot {
        sessions: state.sessions.clone(),
        active_port: state.active_port.clone(),
        last_connected_port: state.last_connected_port.clone(),
        default_assignments: state.default_assignments.clone(),
        default_button_bindings: state.default_button_bindings.clone(),
        default_button_hw_slider_mute: state.default_button_hw_slider_mute.clone(),
        default_shortcut_led_mode: state.default_shortcut_led_mode.clone(),
        button_media_keys: state.button_media_keys.clone(),
        connect_mutex: state.connect_mutex.clone(),
        session_connect_order: state.session_connect_order.clone(),
        api: state.api.clone(),
    };

    thread::spawn(move || {
        let mut last_present: HashSet<String> = HashSet::new();
        loop {
            thread::sleep(Duration::from_millis(HOTPLUG_POLL_MS));

            let present: HashSet<String> = {
                let mut guard = match api_slot.lock() {
                    Ok(g) => g,
                    Err(_) => continue,
                };
                if guard.is_none() {
                    match HidApi::new() {
                        Ok(a) => *guard = Some(a),
                        Err(_) => continue,
                    }
                }
                let api = match guard.as_mut() {
                    Some(a) => a,
                    None => continue,
                };
                refresh_api(api);
                enumerate_idei(api).into_iter().map(|(p, _)| p).collect()
            };

            // Disconnect sessions whose path vanished.
            let connected: Vec<String> = sessions
                .lock()
                .ok()
                .map(|m| m.keys().cloned().collect())
                .unwrap_or_default();
            let missing: Vec<String> = connected
                .into_iter()
                .filter(|p| !present.contains(p))
                .collect();

            if !missing.is_empty() {
                let mut removed: Vec<(String, SessionRuntime)> = Vec::new();
                {
                    let mut map = match sessions.lock() {
                        Ok(m) => m,
                        Err(_) => continue,
                    };
                    for p in &missing {
                        if let Some(rt) = map.remove(p) {
                            removed.push((p.clone(), rt));
                        }
                    }
                }
                if let Ok(mut ord) = connect_order.lock() {
                    ord.retain(|p| !missing.contains(p));
                }
                for (port, mut rt) in removed {
                    rt.handle.stop_requested.store(true, Ordering::SeqCst);
                    let _ = rt.handle.hid_cmd_tx.lock().ok().and_then(|mut g| g.take());
                    if let Some(j) = rt.read_thread.take() {
                        let _ = j.join();
                    }
                    if let Some(j) = rt.apply_thread.take() {
                        let _ = j.join();
                    }
                    app.emit("device-disconnected-port", &port).ok();
                    app.emit("device-disconnected", ()).ok();
                }
                let next_active = sessions.lock().ok().and_then(|m| m.keys().next().cloned());
                if let Ok(mut ap) = active_port.lock() {
                    if ap
                        .as_ref()
                        .map(|p| missing.iter().any(|m| m == p))
                        .unwrap_or(false)
                    {
                        *ap = next_active;
                    }
                }
            }

            // Auto-connect: new device appeared and nothing connected, or preferred path returned.
            let has_session = sessions.lock().ok().map(|m| !m.is_empty()).unwrap_or(true);
            if !has_session {
                let preferred = last_connected.lock().ok().and_then(|g| g.clone());
                let candidate = if let Some(ref pref) = preferred {
                    if present.contains(pref) {
                        Some(pref.clone())
                    } else if present.len() == 1 {
                        present.iter().next().cloned()
                    } else {
                        None
                    }
                } else if present.len() == 1 {
                    present.iter().next().cloned()
                } else {
                    None
                };

                if let Some(path) = candidate {
                    // Tylko przy pojawieniu się urządzenia (unikamy spamowania connect przy błędzie).
                    if !last_present.contains(&path) {
                        let app2 = app.clone();
                        let snap = state_for_connect.clone();
                        let _ = connect_device_inner(&app2, &snap, path);
                    }
                }
            }

            // Emit scan-complete when the set of present devices changes (UI refresh).
            if present != last_present {
                let list: Vec<String> = {
                    let mut v: Vec<_> = present.iter().cloned().collect();
                    v.sort();
                    v
                };
                app.emit("scan-complete", list).ok();
                last_present = present;
            }
        }
    });
}

/// Lekki snapshot stanu do użycia z wątku hot-plug (bez tauri::State).
#[derive(Clone)]
struct HidStateSnapshot {
    sessions: Arc<Mutex<HashMap<String, SessionRuntime>>>,
    active_port: Arc<Mutex<Option<String>>>,
    last_connected_port: Arc<Mutex<Option<String>>>,
    default_assignments: Arc<Mutex<[Vec<VolumeTarget>; crate::audio::MAX_SLIDERS]>>,
    default_button_bindings: Arc<Mutex<[ButtonBinding; MEDIA_BUTTON_SLOTS]>>,
    default_button_hw_slider_mute: Arc<Mutex<[bool; MEDIA_BUTTON_SLOTS]>>,
    default_shortcut_led_mode: Arc<Mutex<[ShortcutLedMode; MEDIA_BUTTON_SLOTS]>>,
    button_media_keys: Arc<AtomicBool>,
    connect_mutex: Arc<Mutex<()>>,
    session_connect_order: Arc<Mutex<Vec<String>>>,
    api: Arc<Mutex<Option<HidApi>>>,
}

impl HidStateSnapshot {
    fn as_state_view(&self) -> HidState {
        HidState {
            sessions: self.sessions.clone(),
            active_port: self.active_port.clone(),
            last_connected_port: self.last_connected_port.clone(),
            default_assignments: self.default_assignments.clone(),
            default_button_bindings: self.default_button_bindings.clone(),
            default_button_hw_slider_mute: self.default_button_hw_slider_mute.clone(),
            default_shortcut_led_mode: self.default_shortcut_led_mode.clone(),
            button_media_keys: self.button_media_keys.clone(),
            connect_mutex: self.connect_mutex.clone(),
            session_connect_order: self.session_connect_order.clone(),
            api: self.api.clone(),
        }
    }
}

fn hid_state_snapshot(state: &HidState) -> HidStateSnapshot {
    HidStateSnapshot {
        sessions: state.sessions.clone(),
        active_port: state.active_port.clone(),
        last_connected_port: state.last_connected_port.clone(),
        default_assignments: state.default_assignments.clone(),
        default_button_bindings: state.default_button_bindings.clone(),
        default_button_hw_slider_mute: state.default_button_hw_slider_mute.clone(),
        default_shortcut_led_mode: state.default_shortcut_led_mode.clone(),
        button_media_keys: state.button_media_keys.clone(),
        connect_mutex: state.connect_mutex.clone(),
        session_connect_order: state.session_connect_order.clone(),
        api: state.api.clone(),
    }
}

fn connect_device_inner(app: &AppHandle, snap: &HidStateSnapshot, path: String) -> Result<(), String> {
    let state = snap.as_state_view();
    let _guard = state.connect_mutex.lock().map_err(|e| e.to_string())?;
    if state
        .sessions
        .lock()
        .map_err(|e| e.to_string())?
        .contains_key(&path)
    {
        if let Ok(mut a) = state.active_port.lock() {
            *a = Some(path);
        }
        return Ok(());
    }

    let (device, hid_dev) = {
        let mut guard = state.api.lock().map_err(|e| e.to_string())?;
        if guard.is_none() {
            *guard = Some(HidApi::new().map_err(|e| format!("HID init: {e}"))?);
        }
        let api = guard.as_mut().unwrap();
        refresh_api(api);
        let hid_dev = api
            .open_path(std::ffi::CString::new(path.as_str()).map_err(|e| e.to_string())?.as_c_str())
            .or_else(|_| api.open(IDEI_VID, IDEI_PID))
            .map_err(|e| format!("Nie można otworzyć HID: {e}"))?;
        let device = do_handshake(&hid_dev, &path)?;
        (device, hid_dev)
    };

    let mut device = device;
    device.port = path.clone();

    finish_connect(app, &state, path, device, hid_dev)
}

fn finish_connect(
    app: &AppHandle,
    state: &HidState,
    path: String,
    device: DeviceInfo,
    hid_dev: HidDevice,
) -> Result<(), String> {
    let last_values = Arc::new(Mutex::new([0u16; crate::audio::MAX_SLIDERS]));
    let button_mask = Arc::new(Mutex::new(0u8));
    let channel_mute = Arc::new(Mutex::new([false; MEDIA_BUTTON_SLOTS]));
    let volume_dirty = Arc::new(AtomicBool::new(false));
    let mut init_assignments = state
        .default_assignments
        .lock()
        .map_err(|e| e.to_string())?
        .clone();
    let mut init_bindings = state
        .default_button_bindings
        .lock()
        .map_err(|e| e.to_string())?
        .clone();
    let mut init_hw_mute = state
        .default_button_hw_slider_mute
        .lock()
        .map_err(|e| e.to_string())?
        .clone();
    let mut init_led_mode = state
        .default_shortcut_led_mode
        .lock()
        .map_err(|e| e.to_string())?
        .clone();

    let cfg = load_config(app);
    let id_key = device_identity_key(&device);
    if let Some(map) = cfg.device_configs.as_ref() {
        if let Some(dc) = map.get(&id_key) {
            for i in 0..crate::audio::MAX_SLIDERS {
                init_assignments[i] = dc.volume_assignments.get(i).cloned().unwrap_or_default();
            }
            for i in 0..MEDIA_BUTTON_SLOTS {
                init_bindings[i] = dc
                    .button_bindings
                    .get(i)
                    .cloned()
                    .unwrap_or(ButtonBinding::None)
                    .normalized();
                init_hw_mute[i] = *dc.button_hw_slider_mute.get(i).unwrap_or(&false);
                init_led_mode[i] = *dc
                    .shortcut_led_mode
                    .get(i)
                    .unwrap_or(&ShortcutLedMode::Off);
            }
        }
    }
    // Aktywny profil ma pierwszeństwo nad device_configs (źródło prawdy mapowań).
    if let Some(pid) = cfg.active_profile_id.as_ref() {
        if let Some(profile) = find_profile(&cfg, pid) {
            for i in 0..crate::audio::MAX_SLIDERS {
                init_assignments[i] = profile
                    .volume_assignments
                    .get(i)
                    .cloned()
                    .unwrap_or_default();
            }
            for i in 0..MEDIA_BUTTON_SLOTS {
                init_bindings[i] = profile
                    .button_bindings
                    .get(i)
                    .cloned()
                    .unwrap_or(ButtonBinding::None)
                    .normalized();
                init_hw_mute[i] = *profile.button_hw_slider_mute.get(i).unwrap_or(&false);
                init_led_mode[i] = *profile
                    .shortcut_led_mode
                    .get(i)
                    .unwrap_or(&ShortcutLedMode::Off);
            }
        }
    }

    let assignments = Arc::new(Mutex::new(init_assignments));
    let button_bindings = Arc::new(Mutex::new(init_bindings));
    let button_hw_slider_mute = Arc::new(Mutex::new(init_hw_mute));
    let shortcut_led_mode = Arc::new(Mutex::new(init_led_mode));
    let shortcut_led_latched = Arc::new(Mutex::new([false; MEDIA_BUTTON_SLOTS]));
    let hid_cmd_tx = Arc::new(Mutex::new(None));
    let stop_requested = Arc::new(AtomicBool::new(false));
    let (cmd_tx, cmd_rx) = mpsc::channel::<HidOutCmd>();
    if let Ok(mut g) = hid_cmd_tx.lock() {
        *g = Some(cmd_tx);
    }

    let app_read = app.clone();
    let values_for_read = last_values.clone();
    let button_mask_for_read = button_mask.clone();
    let channel_mute_for_read = channel_mute.clone();
    let dirty_for_read = volume_dirty.clone();
    let stop_for_read = stop_requested.clone();
    let bindings_for_read = button_bindings.clone();
    let hw_mute_for_read = button_hw_slider_mute.clone();
    let led_mode_for_read = shortcut_led_mode.clone();
    let led_latched_for_read = shortcut_led_latched.clone();
    let cmd_tx_for_read = hid_cmd_tx.clone();
    let button_media = state.button_media_keys.clone();
    let read_path = path.clone();
    let slider_count = device.sliders as usize;

    let read_join = thread::spawn(move || {
        read_loop(
            hid_dev,
            app_read,
            read_path,
            values_for_read,
            button_mask_for_read,
            channel_mute_for_read,
            dirty_for_read,
            slider_count,
            stop_for_read,
            button_media,
            bindings_for_read,
            hw_mute_for_read,
            led_mode_for_read,
            led_latched_for_read,
            cmd_tx_for_read,
            cmd_rx,
        );
    });

    let values_for_apply = last_values.clone();
    let channel_mute_for_apply = channel_mute.clone();
    let assignments_for_apply = assignments.clone();
    let dirty_for_apply = volume_dirty.clone();
    let stop_for_apply = stop_requested.clone();
    let apply_join = thread::spawn(move || {
        apply_volume_loop(
            values_for_apply,
            channel_mute_for_apply,
            assignments_for_apply,
            dirty_for_apply,
            slider_count,
            stop_for_apply,
        )
    });

    let handle = SessionHandle {
        device: device.clone(),
        stop_requested,
        last_slider_values: last_values,
        button_mask,
        channel_mute,
        volume_assignments: assignments,
        button_bindings,
        button_hw_slider_mute,
        shortcut_led_mode,
        shortcut_led_latched,
        hid_cmd_tx: hid_cmd_tx.clone(),
        volume_dirty,
    };
    let runtime = SessionRuntime {
        handle: handle.clone(),
        read_thread: Some(read_join),
        apply_thread: Some(apply_join),
    };
    state
        .sessions
        .lock()
        .map_err(|e| e.to_string())?
        .insert(path.clone(), runtime);
    if let Ok(mut ord) = state.session_connect_order.lock() {
        if !ord.iter().any(|p| p == &path) {
            ord.push(path.clone());
        }
    }
    reconcile_global_volume_assignments(state);
    if let Ok(mut a) = state.active_port.lock() {
        *a = Some(path.clone());
    }
    if let Ok(mut l) = state.last_connected_port.lock() {
        *l = Some(path.clone());
    }

    // LED start: wyłączone — latched/toggle zaczynają od OFF.
    sync_channel_mute_leds(&handle.hid_cmd_tx, &[false; MEDIA_BUTTON_SLOTS]);
    emit_led_latched(app, &path, &[false; MEDIA_BUTTON_SLOTS]);

    let _ = save_config(
        app,
        &build_config_from_state(app, state).unwrap_or_else(|_| load_config(app)),
    );

    app.emit("device-connected", &device).ok();
    app.emit("device-connected-port", &device).ok();
    // Wartości 0 do czasu pierwszego raportu 0x02 (snapshot FW) — UI dostanie prawdziwy stan zaraz po nim.
    if let Ok(g) = handle.last_slider_values.lock() {
        let payload = SliderValuesPayload {
            port: device.port.clone(),
            values: g.iter().take(device.sliders as usize).copied().collect(),
        };
        app.emit("slider-values", &payload).ok();
        app.emit("slider-values-port", &payload).ok();
    }
    let ports = ordered_session_ports(state);
    let _ = app.emit("assignments-reconciled", &ports);
    Ok(())
}

#[tauri::command]
pub fn connect_device(
    app: AppHandle,
    path: String,
    state: tauri::State<HidState>,
) -> Result<(), String> {
    let snap = hid_state_snapshot(&state);
    match connect_device_inner(&app, &snap, path) {
        Ok(()) => Ok(()),
        Err(e) => {
            app.emit("connect-error", &e).ok();
            Err(e)
        }
    }
}

/// Alias kompatybilności (stare `connect_serial` + `portName`).
#[tauri::command]
pub fn connect_serial(
    app: AppHandle,
    port_name: String,
    state: tauri::State<HidState>,
) -> Result<(), String> {
    connect_device(app, port_name, state)
}

#[tauri::command]
pub fn disconnect_device(
    app: AppHandle,
    state: tauri::State<HidState>,
    path: Option<String>,
) -> Result<(), String> {
    if let Some(port) = path.or_else(|| state.active_port.lock().ok().and_then(|g| g.clone())) {
        let removed = state
            .sessions
            .lock()
            .map_err(|e| e.to_string())?
            .remove(&port);
        if let Ok(mut ord) = state.session_connect_order.lock() {
            ord.retain(|p| p != &port);
        }
        if let Some(mut s) = removed {
            s.handle.stop_requested.store(true, Ordering::SeqCst);
            let _ = s.handle.hid_cmd_tx.lock().ok().and_then(|mut g| g.take());
            if let Some(j) = s.read_thread.take() {
                let _ = j.join();
            }
            if let Some(j) = s.apply_thread.take() {
                let _ = j.join();
            }
            app.emit("device-disconnected-port", &port).ok();
            app.emit("device-disconnected", ()).ok();
        }
        let next = state.sessions.lock().ok().and_then(|m| m.keys().next().cloned());
        if let Ok(mut ap) = state.active_port.lock() {
            if ap.as_ref() == Some(&port) {
                *ap = next;
            }
        }
    } else {
        let ports: Vec<String> = state
            .sessions
            .lock()
            .map_err(|e| e.to_string())?
            .keys()
            .cloned()
            .collect();
        for p in ports {
            let _ = disconnect_device(app.clone(), state.clone(), Some(p));
        }
    }
    Ok(())
}

#[tauri::command]
pub fn disconnect_serial(
    app: AppHandle,
    state: tauri::State<HidState>,
    port_name: Option<String>,
) -> Result<(), String> {
    disconnect_device(app, state, port_name)
}

#[tauri::command]
pub fn get_connection_status(state: tauri::State<HidState>) -> Option<DeviceInfo> {
    active_device_info(state.inner())
}

pub fn active_device_info(state: &HidState) -> Option<DeviceInfo> {
    let port = state.active_port.lock().ok().and_then(|g| g.clone())?;
    state
        .sessions
        .lock()
        .ok()?
        .get(&port)
        .map(|s| s.handle.device.clone())
}

#[tauri::command]
pub fn get_connected_devices(state: tauri::State<HidState>) -> Vec<DeviceInfo> {
    state
        .sessions
        .lock()
        .ok()
        .map(|m| m.values().map(|s| s.handle.device.clone()).collect())
        .unwrap_or_default()
}

#[tauri::command]
pub fn get_slider_values(
    state: tauri::State<HidState>,
    port_name: Option<String>,
) -> Result<Vec<u16>, String> {
    let s = get_session(&state, port_name)?;
    let vals = s
        .last_slider_values
        .lock()
        .map(|g| *g)
        .unwrap_or([0; crate::audio::MAX_SLIDERS]);
    Ok(vals
        .iter()
        .take(s.device.sliders as usize)
        .copied()
        .collect())
}

#[tauri::command]
pub fn get_volume_assignments(
    state: tauri::State<HidState>,
    port_name: Option<String>,
) -> Result<[Vec<VolumeTarget>; crate::audio::MAX_SLIDERS], String> {
    let s = get_session(&state, port_name)?;
    Ok(s.volume_assignments
        .lock()
        .map(|g| g.clone())
        .unwrap_or_else(|_| default_assignments()))
}

#[tauri::command]
pub fn set_volume_assignment(
    app: AppHandle,
    state: tauri::State<HidState>,
    port_name: Option<String>,
    slider_index: usize,
    targets: Vec<VolumeTarget>,
) -> Result<(), String> {
    if slider_index >= crate::audio::MAX_SLIDERS {
        return Err("Nieprawidłowy indeks suwaka".to_string());
    }
    let s = get_session(&state, port_name)?;
    let current_port = s.device.port.clone();
    let mut occupied: Vec<VolumeTarget> = vec![];
    if let Ok(sessions) = state.sessions.lock() {
        for (port, rt) in sessions.iter() {
            if port == &current_port {
                if let Ok(arr) = rt.handle.volume_assignments.lock() {
                    for i in 0..crate::audio::MAX_SLIDERS {
                        if i == slider_index {
                            continue;
                        }
                        occupied.extend(arr[i].clone());
                    }
                }
            } else if let Ok(arr) = rt.handle.volume_assignments.lock() {
                for i in 0..crate::audio::MAX_SLIDERS {
                    occupied.extend(arr[i].clone());
                }
            }
        }
    }
    if targets
        .iter()
        .any(|t| occupied.iter().any(|u| volume_targets_conflict(t, u)))
    {
        return Err("Target already assigned on another slider/device".to_string());
    }
    if let Ok(mut g) = s.volume_assignments.lock() {
        g[slider_index] = targets;
    }
    mark_volume_dirty(&s.volume_dirty);
    if let Ok(cfg) = build_config_from_state(&app, &state) {
        let _ = save_config(&app, &cfg);
    }
    Ok(())
}

fn build_config_from_state(app: &AppHandle, state: &HidState) -> Result<AppConfig, ()> {
    let mut cfg = load_config(app);
    let assignments = if let Some(ap) = state.active_port.lock().map_err(|_| ())?.clone() {
        state
            .sessions
            .lock()
            .map_err(|_| ())?
            .get(&ap)
            .map(|s| {
                s.handle
                    .volume_assignments
                    .lock()
                    .ok()
                    .map(|x| x.clone())
                    .unwrap_or_else(default_assignments)
            })
            .unwrap_or_else(default_assignments)
    } else {
        state
            .default_assignments
            .lock()
            .map_err(|_| ())?
            .clone()
    };
    let last_port = state.last_connected_port.lock().map_err(|_| ())?.clone();
    cfg.volume_assignments = Some(assignments_to_vec(&assignments));
    cfg.last_port = last_port.or(cfg.last_port);
    cfg.button_media_keys = Some(state.button_media_keys.load(Ordering::SeqCst));
    if let Some(ap) = state.active_port.lock().map_err(|_| ())?.clone() {
        if let Some(sess) = state.sessions.lock().map_err(|_| ())?.get(&ap) {
            if let Ok(a) = sess.handle.button_bindings.lock() {
                cfg.button_bindings = Some(a.iter().cloned().collect());
            }
            if let Ok(m) = sess.handle.button_hw_slider_mute.lock() {
                cfg.button_hw_slider_mute = Some(m.to_vec());
            }
            if let Ok(m) = sess.handle.shortcut_led_mode.lock() {
                cfg.shortcut_led_mode = Some(m.to_vec());
            }
        }
    } else if let Ok(a) = state.default_button_bindings.lock() {
        cfg.button_bindings = Some(a.iter().cloned().collect());
    }
    if cfg.button_hw_slider_mute.is_none() {
        if let Ok(m) = state.default_button_hw_slider_mute.lock() {
            cfg.button_hw_slider_mute = Some(m.to_vec());
        }
    }
    if cfg.shortcut_led_mode.is_none() {
        if let Ok(m) = state.default_shortcut_led_mode.lock() {
            cfg.shortcut_led_mode = Some(m.to_vec());
        }
    }

    let mut per_device = cfg.device_configs.take().unwrap_or_default();
    if let Ok(sessions) = state.sessions.lock() {
        for (_port, sess) in sessions.iter() {
            let key = device_identity_key(&sess.handle.device);
            let entry = DeviceRuntimeConfig {
                volume_assignments: assignments_to_vec(
                    &sess
                        .handle
                        .volume_assignments
                        .lock()
                        .ok()
                        .map(|g| g.clone())
                        .unwrap_or_else(default_assignments),
                ),
                button_bindings: sess
                    .handle
                    .button_bindings
                    .lock()
                    .ok()
                    .map(|g| g.iter().cloned().collect())
                    .unwrap_or_else(|| vec![ButtonBinding::None; MEDIA_BUTTON_SLOTS]),
                button_hw_slider_mute: sess
                    .handle
                    .button_hw_slider_mute
                    .lock()
                    .ok()
                    .map(|g| g.to_vec())
                    .unwrap_or_else(|| vec![false; MEDIA_BUTTON_SLOTS]),
                shortcut_led_mode: sess
                    .handle
                    .shortcut_led_mode
                    .lock()
                    .ok()
                    .map(|g| g.to_vec())
                    .unwrap_or_else(|| default_shortcut_led_modes().to_vec()),
                shortcut_mute_led_map: vec![],
            };
            per_device.insert(key, entry);
        }
    }
    cfg.device_configs = Some(per_device);
    Ok(cfg)
}

#[tauri::command]
pub fn get_app_config(app: AppHandle, state: tauri::State<HidState>) -> AppConfig {
    let mut cfg = load_config(&app);
    if let Some(ap) = state.active_port.lock().ok().and_then(|g| g.clone()) {
        if let Ok(sessions) = state.sessions.lock() {
            if let Some(sess) = sessions.get(&ap) {
                if let Ok(assignments) = sess.handle.volume_assignments.lock() {
                    cfg.volume_assignments = Some(assignments_to_vec(&*assignments));
                }
            }
        }
    } else if let Ok(assignments) = state.default_assignments.lock() {
        cfg.volume_assignments = Some(assignments_to_vec(&*assignments));
    }
    if let Ok(last) = state.last_connected_port.lock() {
        if last.is_some() {
            cfg.last_port = last.clone();
        }
    }
    cfg.button_media_keys = Some(state.button_media_keys.load(Ordering::SeqCst));
    if let Some(ap) = state.active_port.lock().ok().and_then(|g| g.clone()) {
        if let Ok(sessions) = state.sessions.lock() {
            if let Some(sess) = sessions.get(&ap) {
                if let Ok(a) = sess.handle.button_bindings.lock() {
                    cfg.button_bindings = Some(a.iter().cloned().collect());
                }
                if let Ok(m) = sess.handle.button_hw_slider_mute.lock() {
                    cfg.button_hw_slider_mute = Some(m.to_vec());
                }
                if let Ok(m) = sess.handle.shortcut_led_mode.lock() {
                    cfg.shortcut_led_mode = Some(m.to_vec());
                }
            }
        }
    }
    cfg
}

#[tauri::command]
pub fn save_app_config(app: AppHandle, state: tauri::State<HidState>) -> Result<(), String> {
    let cfg = build_config_from_state(&app, &state).map_err(|_| "Lock state".to_string())?;
    save_config(&app, &cfg)
}

#[tauri::command]
pub fn save_autostart_preference(app: AppHandle, enabled: bool) -> Result<(), String> {
    let mut cfg = load_config(&app);
    cfg.autostart = Some(enabled);
    save_config(&app, &cfg)
}

fn sync_leds_for_session(app: &AppHandle, s: &SessionHandle) {
    let hw = s
        .button_hw_slider_mute
        .lock()
        .ok()
        .map(|g| *g)
        .unwrap_or([false; MEDIA_BUTTON_SLOTS]);
    let channel = s
        .channel_mute
        .lock()
        .ok()
        .map(|g| *g)
        .unwrap_or([false; MEDIA_BUTTON_SLOTS]);
    clear_shortcut_led_latched(
        Some(app),
        Some(&s.device.port),
        &s.shortcut_led_latched,
        &s.hid_cmd_tx,
        &hw,
        &channel,
    );
    let modes = s
        .shortcut_led_mode
        .lock()
        .ok()
        .map(|g| *g)
        .unwrap_or_else(default_shortcut_led_modes);
    let latched = s
        .shortcut_led_latched
        .lock()
        .ok()
        .map(|g| *g)
        .unwrap_or([false; MEDIA_BUTTON_SLOTS]);
    for i in 0..MEDIA_BUTTON_SLOTS {
        if hw[i] {
            set_slot_led(&s.hid_cmd_tx, i, channel[i]);
        } else {
            let on = matches!(modes[i], ShortcutLedMode::Toggle) && latched[i];
            set_slot_led(&s.hid_cmd_tx, i, on);
        }
    }
}

fn apply_profile_data_to_session(app: &AppHandle, s: &SessionHandle, data: &ProfileData) {
    let mut assignments = default_assignments();
    for i in 0..crate::audio::MAX_SLIDERS {
        assignments[i] = data
            .volume_assignments
            .get(i)
            .cloned()
            .unwrap_or_default();
    }
    if let Ok(mut g) = s.volume_assignments.lock() {
        *g = assignments;
    }

    if let Ok(mut g) = s.button_bindings.lock() {
        for i in 0..MEDIA_BUTTON_SLOTS {
            g[i] = data
                .button_bindings
                .get(i)
                .cloned()
                .unwrap_or(ButtonBinding::None)
                .normalized();
        }
    }
    if let Ok(mut g) = s.button_hw_slider_mute.lock() {
        for i in 0..MEDIA_BUTTON_SLOTS {
            g[i] = *data.button_hw_slider_mute.get(i).unwrap_or(&false);
        }
    }
    if let Ok(mut g) = s.shortcut_led_mode.lock() {
        for i in 0..MEDIA_BUTTON_SLOTS {
            g[i] = *data
                .shortcut_led_mode
                .get(i)
                .unwrap_or(&ShortcutLedMode::Off);
        }
    }
    // Reset programowego mute kanału przy przełączeniu profilu.
    if let Ok(mut mute) = s.channel_mute.lock() {
        *mute = [false; MEDIA_BUTTON_SLOTS];
    }
    sync_leds_for_session(app, s);
    mark_volume_dirty(&s.volume_dirty);
}

fn apply_profile_to_runtime(app: &AppHandle, state: &HidState, profile: &Profile) -> Result<(), String> {
    let data = profile.to_data();
    // Aktualizuj domyślne wartości (gdy brak sesji / nowe połączenia).
    if let Ok(mut g) = state.default_assignments.lock() {
        for i in 0..crate::audio::MAX_SLIDERS {
            g[i] = data
                .volume_assignments
                .get(i)
                .cloned()
                .unwrap_or_default();
        }
    }
    if let Ok(mut g) = state.default_button_bindings.lock() {
        for i in 0..MEDIA_BUTTON_SLOTS {
            g[i] = data
                .button_bindings
                .get(i)
                .cloned()
                .unwrap_or(ButtonBinding::None)
                .normalized();
        }
    }
    if let Ok(mut g) = state.default_button_hw_slider_mute.lock() {
        for i in 0..MEDIA_BUTTON_SLOTS {
            g[i] = *data.button_hw_slider_mute.get(i).unwrap_or(&false);
        }
    }
    if let Ok(mut g) = state.default_shortcut_led_mode.lock() {
        for i in 0..MEDIA_BUTTON_SLOTS {
            g[i] = *data
                .shortcut_led_mode
                .get(i)
                .unwrap_or(&ShortcutLedMode::Off);
        }
    }

    // Zastosuj do wszystkich aktywnych sesji (routing + LED 0x03).
    if let Ok(sessions) = state.sessions.lock() {
        for (_port, sess) in sessions.iter() {
            apply_profile_data_to_session(app, &sess.handle, &data);
        }
    }
    Ok(())
}

fn runtime_snapshot_as_profile_data(state: &HidState) -> ProfileData {
    let mut data = ProfileData {
        volume_assignments: (0..crate::audio::MAX_SLIDERS).map(|_| vec![]).collect(),
        button_bindings: vec![ButtonBinding::None; MEDIA_BUTTON_SLOTS],
        button_hw_slider_mute: vec![false; MEDIA_BUTTON_SLOTS],
        shortcut_led_mode: default_shortcut_led_modes().to_vec(),
        button_media_actions: None,
        shortcut_mute_led_map: None,
        shortcut_mute_led: None,
    };

    let port = state.active_port.lock().ok().and_then(|g| g.clone());
    if let Some(ap) = port {
        if let Ok(sessions) = state.sessions.lock() {
            if let Some(sess) = sessions.get(&ap) {
                if let Ok(a) = sess.handle.volume_assignments.lock() {
                    data.volume_assignments = assignments_to_vec(&*a);
                }
                if let Ok(b) = sess.handle.button_bindings.lock() {
                    data.button_bindings = b.iter().cloned().collect();
                }
                if let Ok(m) = sess.handle.button_hw_slider_mute.lock() {
                    data.button_hw_slider_mute = m.to_vec();
                }
                if let Ok(m) = sess.handle.shortcut_led_mode.lock() {
                    data.shortcut_led_mode = m.to_vec();
                }
                return data;
            }
        }
    }

    if let Ok(a) = state.default_assignments.lock() {
        data.volume_assignments = assignments_to_vec(&*a);
    }
    if let Ok(b) = state.default_button_bindings.lock() {
        data.button_bindings = b.iter().cloned().collect();
    }
    if let Ok(m) = state.default_button_hw_slider_mute.lock() {
        data.button_hw_slider_mute = m.to_vec();
    }
    if let Ok(m) = state.default_shortcut_led_mode.lock() {
        data.shortcut_led_mode = m.to_vec();
    }
    data
}

#[tauri::command]
pub fn get_profiles(app: AppHandle) -> ProfilesState {
    let mut cfg = load_config(&app);
    ensure_profiles(&mut cfg);
    profiles_state(&cfg)
}

#[tauri::command]
pub fn set_active_profile(
    app: AppHandle,
    state: tauri::State<HidState>,
    profile_id: String,
) -> Result<ProfilesState, String> {
    let mut cfg = load_config(&app);
    ensure_profiles(&mut cfg);
    let profile = find_profile(&cfg, &profile_id)
        .cloned()
        .ok_or_else(|| format!("Nie znaleziono profilu: {profile_id}"))?;
    apply_profile_to_runtime(&app, &state, &profile)?;
    apply_profile_to_flat_config(&mut cfg, &profile);
    // Odśwież device_configs z runtime po przełączeniu.
    if let Ok(built) = build_config_from_state(&app, &state) {
        cfg.device_configs = built.device_configs;
        cfg.volume_assignments = built.volume_assignments;
        cfg.button_bindings = built.button_bindings;
        cfg.button_hw_slider_mute = built.button_hw_slider_mute;
        cfg.shortcut_led_mode = built.shortcut_led_mode;
        cfg.last_port = built.last_port.or(cfg.last_port);
    }
    cfg.active_profile_id = Some(profile.id.clone());
    save_config(&app, &cfg)?;
    Ok(profiles_state(&cfg))
}

#[tauri::command]
pub fn save_current_profile(
    app: AppHandle,
    state: tauri::State<HidState>,
    profile_data: ProfileData,
) -> Result<ProfilesState, String> {
    let mut cfg = load_config(&app);
    ensure_profiles(&mut cfg);
    let active_id = cfg
        .active_profile_id
        .clone()
        .ok_or_else(|| "Brak aktywnego profilu".to_string())?;
    let updated = {
        let profile = find_profile_mut(&mut cfg, &active_id)
            .ok_or_else(|| format!("Nie znaleziono profilu: {active_id}"))?;
        profile.apply_data(profile_data);
        profile.clone()
    };
    apply_profile_to_flat_config(&mut cfg, &updated);
    if let Ok(built) = build_config_from_state(&app, &state) {
        cfg.device_configs = built.device_configs;
        cfg.last_port = built.last_port.or(cfg.last_port);
    }
    save_config(&app, &cfg)?;
    Ok(profiles_state(&cfg))
}

#[tauri::command]
pub fn create_profile(
    app: AppHandle,
    state: tauri::State<HidState>,
    name: String,
    base_profile_id: Option<String>,
    seed: Option<ProfileData>,
) -> Result<ProfilesState, String> {
    let mut cfg = load_config(&app);
    ensure_profiles(&mut cfg);
    let trimmed = name.trim();
    let name = if trimmed.is_empty() {
        "New profile".to_string()
    } else {
        trimmed.to_string()
    };

    let data = if let Some(seed) = seed {
        seed
    } else if let Some(base_id) = base_profile_id.as_ref() {
        find_profile(&cfg, base_id)
            .map(|p| p.to_data())
            .ok_or_else(|| format!("Nie znaleziono profilu bazowego: {base_id}"))?
    } else {
        runtime_snapshot_as_profile_data(&state)
    };

    let mut profile = Profile::from_data(name, data, false);
    // Unikalna nazwa przy duplikacie.
    if base_profile_id.is_some() {
        let base_name = find_profile(&cfg, base_profile_id.as_ref().unwrap())
            .map(|p| p.name.clone())
            .unwrap_or_else(|| profile.name.clone());
        if profile.name == base_name || profile.name.is_empty() {
            profile.name = format!("{base_name} copy");
        }
    }

    let new_id = profile.id.clone();
    if let Some(ref mut list) = cfg.profiles {
        list.push(profile.clone());
    }
    apply_profile_to_runtime(&app, &state, &profile)?;
    apply_profile_to_flat_config(&mut cfg, &profile);
    cfg.active_profile_id = Some(new_id);
    if let Ok(built) = build_config_from_state(&app, &state) {
        cfg.device_configs = built.device_configs;
        cfg.last_port = built.last_port.or(cfg.last_port);
    }
    save_config(&app, &cfg)?;
    Ok(profiles_state(&cfg))
}

#[tauri::command]
pub fn rename_profile(
    app: AppHandle,
    profile_id: String,
    new_name: String,
) -> Result<ProfilesState, String> {
    let mut cfg = load_config(&app);
    ensure_profiles(&mut cfg);
    let trimmed = new_name.trim();
    if trimmed.is_empty() {
        return Err("Nazwa profilu nie może być pusta".to_string());
    }
    let profile = find_profile_mut(&mut cfg, &profile_id)
        .ok_or_else(|| format!("Nie znaleziono profilu: {profile_id}"))?;
    profile.name = trimmed.to_string();
    save_config(&app, &cfg)?;
    Ok(profiles_state(&cfg))
}

#[tauri::command]
pub fn delete_profile(
    app: AppHandle,
    state: tauri::State<HidState>,
    profile_id: String,
) -> Result<ProfilesState, String> {
    let mut cfg = load_config(&app);
    ensure_profiles(&mut cfg);
    let profiles = cfg.profiles.as_ref().cloned().unwrap_or_default();
    if profiles.len() <= 1 {
        return Err("Nie można usunąć ostatniego profilu".to_string());
    }
    let target = profiles
        .iter()
        .find(|p| p.id == profile_id)
        .ok_or_else(|| format!("Nie znaleziono profilu: {profile_id}"))?;
    if target.is_default {
        return Err("Nie można usunąć profilu domyślnego".to_string());
    }

    let was_active = cfg.active_profile_id.as_deref() == Some(profile_id.as_str());
    if let Some(ref mut list) = cfg.profiles {
        list.retain(|p| p.id != profile_id);
    }
    if was_active {
        let fallback = cfg
            .profiles
            .as_ref()
            .and_then(|list| {
                list.iter()
                    .find(|p| p.is_default)
                    .or_else(|| list.first())
                    .cloned()
            })
            .unwrap_or_else(|| profile_from_config(&cfg, DEFAULT_PROFILE_NAME, true));
        apply_profile_to_runtime(&app, &state, &fallback)?;
        apply_profile_to_flat_config(&mut cfg, &fallback);
        cfg.active_profile_id = Some(fallback.id);
        if let Ok(built) = build_config_from_state(&app, &state) {
            cfg.device_configs = built.device_configs;
            cfg.last_port = built.last_port.or(cfg.last_port);
        }
    }
    save_config(&app, &cfg)?;
    Ok(profiles_state(&cfg))
}

/// Komendy legacy tekstowe → mapowanie na HID.
#[tauri::command]
pub fn send_device_command(
    state: tauri::State<HidState>,
    port_name: Option<String>,
    cmd: String,
) -> Result<(), String> {
    let cmd = cmd.trim().to_uppercase();
    let s = get_session(&state, port_name)?;
    match cmd.as_str() {
        "GET_STATE" | "IDENTIFY" | "GET_INFO" => {
            enqueue_cmd(&s.hid_cmd_tx, HidOutCmd::GetInfo);
            Ok(())
        }
        "RESET_DEFAULTS" => {
            enqueue_cmd(&s.hid_cmd_tx, HidOutCmd::ResetLeds);
            Ok(())
        }
        _ => Err(format!("Nieobsługiwane polecenie HID: {cmd}")),
    }
}

/// Bezpośrednie sterowanie NeoPixel (Output Report 0x03).
#[tauri::command]
pub fn set_led_color(
    state: tauri::State<HidState>,
    port_name: Option<String>,
    index: u8,
    r: u8,
    g: u8,
    b: u8,
    flags: Option<u8>,
) -> Result<(), String> {
    let s = get_session(&state, port_name)?;
    if index != 0xFF && (index as usize) >= MEDIA_BUTTON_SLOTS {
        return Err("Indeks LED poza zakresem (0–4 lub 0xFF)".to_string());
    }
    enqueue_cmd(
        &s.hid_cmd_tx,
        HidOutCmd::Led {
            index,
            r,
            g,
            b,
            flags: flags.unwrap_or(0),
        },
    );
    Ok(())
}

#[tauri::command]
pub fn set_button_media_keys(
    app: AppHandle,
    state: tauri::State<HidState>,
    enabled: bool,
) -> Result<(), String> {
    state.button_media_keys.store(enabled, Ordering::SeqCst);
    let mut cfg = load_config(&app);
    cfg.button_media_keys = Some(enabled);
    save_config(&app, &cfg)
}

#[tauri::command]
pub fn get_button_media_keys(state: tauri::State<HidState>) -> bool {
    state.button_media_keys.load(Ordering::SeqCst)
}

#[tauri::command]
pub fn get_button_bindings(
    state: tauri::State<HidState>,
    port_name: Option<String>,
) -> Result<Vec<ButtonBinding>, String> {
    let s = get_session(&state, port_name)?;
    Ok(s.button_bindings
        .lock()
        .map(|g| g.iter().cloned().collect())
        .unwrap_or_else(|_| default_button_bindings().iter().cloned().collect()))
}

#[tauri::command]
pub fn set_button_bindings(
    app: AppHandle,
    state: tauri::State<HidState>,
    port_name: Option<String>,
    bindings: Vec<ButtonBinding>,
) -> Result<(), String> {
    if bindings.len() != MEDIA_BUTTON_SLOTS {
        return Err(format!("Wymagane dokładnie {} wpisów", MEDIA_BUTTON_SLOTS));
    }
    let norm: Vec<ButtonBinding> = bindings.iter().map(|b| b.clone().normalized()).collect();
    let s = get_session(&state, port_name)?;
    if let Ok(mut g) = s.button_bindings.lock() {
        for i in 0..MEDIA_BUTTON_SLOTS {
            g[i] = norm[i].clone();
        }
    }
    if let Ok(cfg) = build_config_from_state(&app, &state) {
        return save_config(&app, &cfg);
    }
    let mut cfg = load_config(&app);
    cfg.button_bindings = Some(norm);
    save_config(&app, &cfg)
}

#[tauri::command]
pub fn get_button_hw_slider_mute(
    state: tauri::State<HidState>,
    port_name: Option<String>,
) -> Result<Vec<bool>, String> {
    let s = get_session(&state, port_name)?;
    Ok(s.button_hw_slider_mute
        .lock()
        .map(|g| g.to_vec())
        .unwrap_or_else(|_| vec![false; MEDIA_BUTTON_SLOTS]))
}

#[tauri::command]
pub fn set_button_hw_slider_mute(
    app: AppHandle,
    state: tauri::State<HidState>,
    port_name: Option<String>,
    enabled: Vec<bool>,
) -> Result<(), String> {
    if enabled.len() != MEDIA_BUTTON_SLOTS {
        return Err(format!("Wymagane {} wartości", MEDIA_BUTTON_SLOTS));
    }
    let s = get_session(&state, port_name.clone())?;
    if let Ok(mut g) = s.button_hw_slider_mute.lock() {
        for i in 0..MEDIA_BUTTON_SLOTS {
            g[i] = enabled[i];
        }
    }
    // Włączając mute kanału — wyczyść latched LED skrótu dla tych slotów.
    // Wyłączając — wyczyść programowy mute kanału.
    if let Ok(mut mute) = s.channel_mute.lock() {
        for i in 0..MEDIA_BUTTON_SLOTS {
            if enabled[i] {
                if let Ok(mut latched) = s.shortcut_led_latched.lock() {
                    latched[i] = false;
                }
            } else if mute[i] {
                mute[i] = false;
            }
        }
    }
    let mut hw = [false; MEDIA_BUTTON_SLOTS];
    for i in 0..MEDIA_BUTTON_SLOTS {
        hw[i] = enabled[i];
    }
    let channel = s
        .channel_mute
        .lock()
        .ok()
        .map(|g| *g)
        .unwrap_or([false; MEDIA_BUTTON_SLOTS]);
    let latched = s
        .shortcut_led_latched
        .lock()
        .ok()
        .map(|g| *g)
        .unwrap_or([false; MEDIA_BUTTON_SLOTS]);
    emit_led_latched(&app, &s.device.port, &latched);
    for i in 0..MEDIA_BUTTON_SLOTS {
        if hw[i] {
            set_slot_led(&s.hid_cmd_tx, i, channel[i]);
        } else {
            let mode = s
                .shortcut_led_mode
                .lock()
                .ok()
                .map(|g| g[i])
                .unwrap_or(ShortcutLedMode::Off);
            let on = matches!(mode, ShortcutLedMode::Toggle) && latched[i];
            set_slot_led(&s.hid_cmd_tx, i, on);
        }
    }
    mark_volume_dirty(&s.volume_dirty);
    if let Ok(cfg) = build_config_from_state(&app, &state) {
        save_config(&app, &cfg)?;
    } else {
        let mut cfg = load_config(&app);
        cfg.button_hw_slider_mute = Some(enabled);
        save_config(&app, &cfg)?;
    }
    Ok(())
}

#[tauri::command]
pub fn get_shortcut_led_mode(
    state: tauri::State<HidState>,
    port_name: Option<String>,
) -> Result<Vec<ShortcutLedMode>, String> {
    let s = get_session(&state, port_name)?;
    Ok(s.shortcut_led_mode
        .lock()
        .map(|g| g.to_vec())
        .unwrap_or_else(|_| default_shortcut_led_modes().to_vec()))
}

#[tauri::command]
pub fn set_shortcut_led_mode(
    app: AppHandle,
    state: tauri::State<HidState>,
    port_name: Option<String>,
    modes: Vec<ShortcutLedMode>,
) -> Result<(), String> {
    if modes.len() != MEDIA_BUTTON_SLOTS {
        return Err(format!("Wymagane {} wartości", MEDIA_BUTTON_SLOTS));
    }
    let s = get_session(&state, port_name)?;
    if let Ok(mut g) = s.shortcut_led_mode.lock() {
        for i in 0..MEDIA_BUTTON_SLOTS {
            g[i] = modes[i];
        }
    }
    // Zmiana trybu — wyczyść latched i zsynchronizuj LED (nie gryź mute kanału).
    let hw = s
        .button_hw_slider_mute
        .lock()
        .ok()
        .map(|g| *g)
        .unwrap_or([false; MEDIA_BUTTON_SLOTS]);
    let channel = s
        .channel_mute
        .lock()
        .ok()
        .map(|g| *g)
        .unwrap_or([false; MEDIA_BUTTON_SLOTS]);
    clear_shortcut_led_latched(
        Some(&app),
        Some(&s.device.port),
        &s.shortcut_led_latched,
        &s.hid_cmd_tx,
        &hw,
        &channel,
    );
    if let Ok(cfg) = build_config_from_state(&app, &state) {
        save_config(&app, &cfg)?;
    } else {
        let mut cfg = load_config(&app);
        cfg.shortcut_led_mode = Some(modes);
        save_config(&app, &cfg)?;
    }
    Ok(())
}

/// Kompatybilność: true ≈ Momentary, false ≈ Off.
#[tauri::command]
pub fn get_shortcut_mute_led_map(
    state: tauri::State<HidState>,
    port_name: Option<String>,
) -> Result<Vec<bool>, String> {
    let modes = get_shortcut_led_mode(state, port_name)?;
    Ok(modes
        .into_iter()
        .map(|m| !matches!(m, ShortcutLedMode::Off))
        .collect())
}

#[tauri::command]
pub fn set_shortcut_mute_led_map(
    app: AppHandle,
    state: tauri::State<HidState>,
    port_name: Option<String>,
    enabled: Vec<bool>,
) -> Result<(), String> {
    let modes: Vec<ShortcutLedMode> = enabled
        .into_iter()
        .map(ShortcutLedMode::from_legacy_bool)
        .collect();
    set_shortcut_led_mode(app, state, port_name, modes)
}

#[tauri::command]
pub fn get_shortcut_led_latched(
    state: tauri::State<HidState>,
    port_name: Option<String>,
) -> Result<Vec<bool>, String> {
    let s = get_session(&state, port_name)?;
    Ok(s.shortcut_led_latched
        .lock()
        .map(|g| g.to_vec())
        .unwrap_or_else(|_| vec![false; MEDIA_BUTTON_SLOTS]))
}

#[tauri::command]
pub fn notify_disconnected(state: tauri::State<HidState>) -> Result<(), String> {
    let _ = state;
    Ok(())
}

pub(crate) fn apply_volume_loop(
    values_arc: Arc<Mutex<[u16; crate::audio::MAX_SLIDERS]>>,
    channel_mute_arc: Arc<Mutex<[bool; MEDIA_BUTTON_SLOTS]>>,
    assignments_arc: Arc<Mutex<[Vec<VolumeTarget>; crate::audio::MAX_SLIDERS]>>,
    volume_dirty: Arc<AtomicBool>,
    slider_count: usize,
    stop: Arc<AtomicBool>,
) {
    // Pierwszy przebieg po connect — wymuś apply gdy przyjdzie snapshot (dirty).
    while !stop.load(Ordering::SeqCst) {
        if volume_dirty.swap(false, Ordering::AcqRel) {
            let values = values_arc
                .lock()
                .ok()
                .map(|g| *g)
                .unwrap_or([0; crate::audio::MAX_SLIDERS]);
            let channel_mute = channel_mute_arc
                .lock()
                .ok()
                .map(|g| *g)
                .unwrap_or([false; MEDIA_BUTTON_SLOTS]);
            let assignments = assignments_arc
                .lock()
                .ok()
                .map(|g| g.clone())
                .unwrap_or_else(default_assignments);
            apply_volumes_now(&values, &channel_mute, &assignments, slider_count);
            // Kolejne raporty HID podczas apply — nie gubimy ich.
            if volume_dirty.load(Ordering::Acquire) {
                continue;
            }
        }
        thread::sleep(Duration::from_millis(APPLY_LOOP_MS));
    }
}

fn read_loop(
    device: HidDevice,
    app: AppHandle,
    path: String,
    last_values: Arc<Mutex<[u16; crate::audio::MAX_SLIDERS]>>,
    button_mask: Arc<Mutex<u8>>,
    channel_mute: Arc<Mutex<[bool; MEDIA_BUTTON_SLOTS]>>,
    volume_dirty: Arc<AtomicBool>,
    slider_count: usize,
    stop: Arc<AtomicBool>,
    button_media_keys: Arc<AtomicBool>,
    button_bindings: Arc<Mutex<[ButtonBinding; MEDIA_BUTTON_SLOTS]>>,
    hw_mute: Arc<Mutex<[bool; MEDIA_BUTTON_SLOTS]>>,
    led_mode: Arc<Mutex<[ShortcutLedMode; MEDIA_BUTTON_SLOTS]>>,
    led_latched: Arc<Mutex<[bool; MEDIA_BUTTON_SLOTS]>>,
    cmd_tx_arc: Arc<Mutex<Option<mpsc::Sender<HidOutCmd>>>>,
    outgoing_rx: mpsc::Receiver<HidOutCmd>,
) {
    let mut prev_buttons: u8 = 0;
    let mut have_snapshot = false;
    let mut button_seq: u64 = 0;
    let mut buf = [0u8; 64];

    loop {
        if stop.load(Ordering::SeqCst) {
            break;
        }

        while let Ok(cmd) = outgoing_rx.try_recv() {
            match cmd {
                HidOutCmd::Led {
                    index,
                    r,
                    g,
                    b,
                    flags,
                } => {
                    if let Err(e) = write_led_report(&device, index, r, g, b, flags) {
                        let _ = app.emit("log", format!("→ LED error: {e}"));
                    } else {
                        let _ = app.emit(
                            "log",
                            format!("→ LED[{index:#04x}] rgb({r},{g},{b}) flags={flags}"),
                        );
                    }
                }
                HidOutCmd::GetInfo => match do_handshake(&device, &path) {
                    Ok(info) => {
                        let _ = app.emit(
                            "log",
                            format!(
                                "← Get Info fw={} sliders={}",
                                info.fw.as_deref().unwrap_or("?"),
                                info.sliders
                            ),
                        );
                    }
                    Err(e) => {
                        let _ = app.emit("log", format!("← Get Info error: {e}"));
                    }
                },
                HidOutCmd::ResetLeds => {
                    if let Err(e) = write_led_report(&device, 0xFF, 0, 0, 0, 0) {
                        let _ = app.emit("log", format!("→ Reset LEDs error: {e}"));
                    } else {
                        let _ = app.emit("log", "→ Reset LEDs".to_string());
                    }
                }
            }
        }

        match device.read_timeout(&mut buf, READ_TIMEOUT_MS) {
            Ok(0) => continue,
            Ok(n) => {
                if n < 1 {
                    continue;
                }
                let parsed = if buf[0] == REPORT_STATE {
                    parse_state_report(&buf[..n])
                } else if n >= INPUT_REPORT_LEN - 1 {
                    let mut tmp = [0u8; INPUT_REPORT_LEN];
                    tmp[0] = REPORT_STATE;
                    let copy_n = (INPUT_REPORT_LEN - 1).min(n);
                    tmp[1..1 + copy_n].copy_from_slice(&buf[..copy_n]);
                    parse_state_report(&tmp)
                } else {
                    None
                };

                if let Some((values, buttons, _status)) = parsed {
                    handle_state_report(
                        &app,
                        &path,
                        &last_values,
                        &button_mask,
                        &channel_mute,
                        &volume_dirty,
                        &button_media_keys,
                        &button_bindings,
                        &hw_mute,
                        &led_mode,
                        &led_latched,
                        &cmd_tx_arc,
                        slider_count,
                        values,
                        buttons,
                        &mut prev_buttons,
                        &mut have_snapshot,
                        &mut button_seq,
                    );
                }
            }
            Err(e) => {
                let msg = e.to_string().to_lowercase();
                if msg.contains("timeout") || msg.contains("timed out") || msg.contains("would block")
                {
                    continue;
                }
                // Rozłączenie: wyczyść latched (sesja i tak zniknie).
                if let Ok(mut g) = led_latched.lock() {
                    *g = [false; MEDIA_BUTTON_SLOTS];
                    emit_led_latched(&app, &path, &*g);
                }
                let _ = app.emit("device-disconnected-port", &path);
                let _ = app.emit("device-disconnected", ());
                let _ = app.emit("log", format!("HID read error: {e}"));
                break;
            }
        }
    }
}

fn handle_state_report(
    app: &AppHandle,
    path: &str,
    last_values: &Arc<Mutex<[u16; crate::audio::MAX_SLIDERS]>>,
    button_mask: &Arc<Mutex<u8>>,
    channel_mute: &Arc<Mutex<[bool; MEDIA_BUTTON_SLOTS]>>,
    volume_dirty: &AtomicBool,
    button_media_keys: &Arc<AtomicBool>,
    button_bindings: &Arc<Mutex<[ButtonBinding; MEDIA_BUTTON_SLOTS]>>,
    hw_mute: &Arc<Mutex<[bool; MEDIA_BUTTON_SLOTS]>>,
    led_mode: &Arc<Mutex<[ShortcutLedMode; MEDIA_BUTTON_SLOTS]>>,
    led_latched: &Arc<Mutex<[bool; MEDIA_BUTTON_SLOTS]>>,
    cmd_tx_arc: &Arc<Mutex<Option<mpsc::Sender<HidOutCmd>>>>,
    slider_count: usize,
    values: Vec<u16>,
    buttons: u8,
    prev_buttons: &mut u8,
    have_snapshot: &mut bool,
    button_seq: &mut u64,
) {
    if let Ok(mut g) = last_values.lock() {
        for i in 0..crate::audio::MAX_SLIDERS {
            g[i] = *values.get(i).unwrap_or(&0u16);
        }
    }
    if let Ok(mut g) = button_mask.lock() {
        *g = buttons;
    }

    let n = slider_count.min(values.len());
    let payload = SliderValuesPayload {
        port: path.to_string(),
        values: values.iter().take(n).copied().collect(),
    };
    app.emit("slider-values", &payload).ok();
    app.emit("slider-values-port", &payload).ok();
    mark_volume_dirty(volume_dirty);

    if !*have_snapshot {
        *have_snapshot = true;
        *prev_buttons = buttons;
        let _ = app.emit(
            "log",
            format!("← snapshot 0x02: {n} sliders (queued for volume worker)"),
        );
        return;
    }

    let risen = buttons & !*prev_buttons;
    let fallen = (!buttons) & *prev_buttons;
    let hw = hw_mute
        .lock()
        .ok()
        .map(|g| *g)
        .unwrap_or([false; MEDIA_BUTTON_SLOTS]);
    let modes = led_mode
        .lock()
        .ok()
        .map(|g| *g)
        .unwrap_or(default_shortcut_led_modes());

    if risen != 0 {
        for i in 0..MEDIA_BUTTON_SLOTS {
            if (risen & (1 << i)) == 0 {
                continue;
            }
            *button_seq = button_seq.wrapping_add(1);
            let payload = DeviceButtonPayload {
                port: path.to_string(),
                index: i as u32,
                seq: *button_seq,
            };
            let _ = app.emit("device-button", &payload);
            let _ = app.emit("device-button-port", &payload);

            if hw[i] {
                // Mute kanału — własny sticky LED, niezależny od toggle skrótu.
                let muted = {
                    let mut g = match channel_mute.lock() {
                        Ok(g) => g,
                        Err(_) => continue,
                    };
                    g[i] = !g[i];
                    g[i]
                };
                set_slot_led(cmd_tx_arc, i, muted);
                mark_volume_dirty(volume_dirty);
                let _ = app.emit(
                    "log",
                    format!(
                        "← btn[{i}] channel mute {}",
                        if muted { "ON" } else { "OFF" }
                    ),
                );
            } else if button_media_keys.load(Ordering::SeqCst) {
                let binding = button_bindings
                    .lock()
                    .ok()
                    .and_then(|g| g.get(i).cloned())
                    .unwrap_or(ButtonBinding::None);
                if !matches!(binding, ButtonBinding::None) {
                    send_button_binding(&binding);
                    let _ = app.emit("log", format!("← btn[{i}] media/shortcut"));
                }
                match modes[i] {
                    ShortcutLedMode::Off => {}
                    ShortcutLedMode::Momentary => {
                        set_slot_led(cmd_tx_arc, i, true);
                    }
                    ShortcutLedMode::Toggle => {
                        let on = {
                            let mut g = match led_latched.lock() {
                                Ok(g) => g,
                                Err(_) => continue,
                            };
                            g[i] = !g[i];
                            let on = g[i];
                            emit_led_latched(app, path, &*g);
                            on
                        };
                        set_slot_led(cmd_tx_arc, i, on);
                        let _ = app.emit(
                            "log",
                            format!(
                                "← btn[{i}] LED toggle {}",
                                if on { "ON" } else { "OFF" }
                            ),
                        );
                    }
                }
            }
        }
    }

    if fallen != 0 {
        for i in 0..MEDIA_BUTTON_SLOTS {
            if (fallen & (1 << i)) == 0 {
                continue;
            }
            // Momentary: zgaś przy puszczeniu. Toggle / mute kanału — zostaw.
            if !hw[i] && matches!(modes[i], ShortcutLedMode::Momentary) {
                set_slot_led(cmd_tx_arc, i, false);
            }
        }
    }

    *prev_buttons = buttons;
}
