use crate::audio::{volume_targets_conflict, VolumeTarget};
use crate::config::{assignments_to_vec, load_config, save_config, AppConfig, DeviceRuntimeConfig};
use crate::media_keys::{default_button_bindings, send_button_binding, ButtonBinding, MEDIA_BUTTON_SLOTS};
use serde::Serialize;
use serialport::{ClearBuffer, SerialPort, SerialPortType};
use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};

/// Model strings from device IDENTIFY / ready JSON (`DEVICE_MODEL` in firmware).
const IDEI_MODELS: &[&str] = &["ideiMx", "ideiMx-max"];
const BAUDRATE: u32 = 115200;

/// Liczba suwaków z nazwy modelu (JSON `sliders` jest tylko podpowiedzią dla nieznanych modeli).
fn effective_slider_count(model: &str, json_sliders: Option<u64>) -> u32 {
    let max = crate::audio::MAX_SLIDERS as u32;
    let from_json = json_sliders
        .map(|u| u as u32)
        .filter(|&s| s >= 1 && s <= max);
    match model {
        "ideiMx-max" => 5,
        "ideiMx" => 3,
        _ => from_json.unwrap_or(3),
    }
}
/// Normalny odczyt w `read_loop` — krótki timeout, żeby nie blokować UI.
const READ_TIMEOUT_MS: u64 = 50;
/// Podczas handshake dłuższy timeout na pierwszy bajt (CDC / Windows bywa powolny).
const HANDSHAKE_READ_TIMEOUT_MS: u64 = 180;
/// Maks. czas czekania na JSON `ready` / `identify` po IDENTIFY.
const HANDSHAKE_DEADLINE_MS: u64 = 4500;
/// Ponowienia handshake przy błędzie (otwarcie portu od nowa).
const HANDSHAKE_MAX_ATTEMPTS: u32 = 3;
/// Ochrona przed zawieszeniem / OOM przy linii bez `\n` z urządzenia.
const MAX_CDC_LINE_BYTES: usize = 2048;
/// Pauza po zapisie linii CDC (Windows USB często zwraca 121 przy zbyt szybkich kolejnych WriteFile).
const CDC_WRITE_GAP_MS: u64 = 20;

/// Win32 ERROR_SEM_TIMEOUT (121) — „Przekroczono limit czasu semafora” przy zapisie na COM/CDC.
fn cdc_write_retryable_io_error(e: &std::io::Error) -> bool {
    e.raw_os_error() == Some(121)
        || e.to_string().to_lowercase().contains("semaphore")
        || e.to_string().to_lowercase().contains("semafor")
}

fn write_cdc_line(port: &mut dyn SerialPort, line: &str) -> std::io::Result<()> {
    const MAX_ATTEMPTS: u32 = 4;
    for attempt in 0..MAX_ATTEMPTS {
        match port.write_all(line.as_bytes()).and_then(|_| port.flush()) {
            Ok(()) => return Ok(()),
            Err(e) => {
                if attempt + 1 < MAX_ATTEMPTS && cdc_write_retryable_io_error(&e) {
                    thread::sleep(Duration::from_millis(30 + 25 * u64::from(attempt)));
                    continue;
                }
                return Err(e);
            }
        }
    }
    unreachable!()
}

#[derive(Clone, Serialize)]
pub struct SerialPortInfo {
    pub name: String,
    pub description: String,
}

#[derive(Clone, Serialize)]
pub struct DeviceInfo {
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

#[derive(Clone)]
struct SessionHandle {
    device: DeviceInfo,
    stop_requested: Arc<AtomicBool>,
    last_slider_values: Arc<Mutex<[u16; crate::audio::MAX_SLIDERS]>>,
    volume_assignments: Arc<Mutex<[Vec<VolumeTarget>; crate::audio::MAX_SLIDERS]>>,
    button_bindings: Arc<Mutex<[ButtonBinding; MEDIA_BUTTON_SLOTS]>>,
    button_hw_slider_mute: Arc<Mutex<[bool; MEDIA_BUTTON_SLOTS]>>,
    shortcut_mute_led_map: Arc<Mutex<[bool; MEDIA_BUTTON_SLOTS]>>,
    serial_cmd_tx: Arc<Mutex<Option<mpsc::Sender<String>>>>,
}

struct SessionRuntime {
    handle: SessionHandle,
    read_thread: Option<JoinHandle<()>>,
    apply_thread: Option<JoinHandle<()>>,
    port: Arc<Mutex<Option<Box<dyn SerialPort>>>>,
}

pub struct SerialState {
    sessions: Arc<Mutex<HashMap<String, SessionRuntime>>>,
    pub active_port: Arc<Mutex<Option<String>>>,
    pub last_connected_port: Arc<Mutex<Option<String>>>,
    pub default_assignments: Arc<Mutex<[Vec<VolumeTarget>; crate::audio::MAX_SLIDERS]>>,
    pub default_button_bindings: Arc<Mutex<[ButtonBinding; MEDIA_BUTTON_SLOTS]>>,
    pub default_button_hw_slider_mute: Arc<Mutex<[bool; MEDIA_BUTTON_SLOTS]>>,
    pub default_shortcut_mute_led_map: Arc<Mutex<[bool; MEDIA_BUTTON_SLOTS]>>,
    pub button_media_keys: Arc<AtomicBool>,
    /// Wyklucza wyścig dwóch równoległych `connect_serial` (oba widziałyby pustą listę sesji).
    connect_serial_mutex: Arc<Mutex<()>>,
    /// Kolejność podłączenia — pierwsze urządzenie zachowuje cele głośności przy konflikcie z kolejnymi.
    session_connect_order: Arc<Mutex<Vec<String>>>,
}

fn default_assignments() -> [Vec<VolumeTarget>; crate::audio::MAX_SLIDERS] {
    std::array::from_fn(|_| vec![])
}

fn enqueue_line_cmd(queue: &Arc<Mutex<Option<mpsc::Sender<String>>>>, line: &str) {
    if let Ok(g) = queue.lock() {
        if let Some(ref tx) = *g {
            let _ = tx.send(line.to_string());
        }
    }
}

/// Firmware ≥ 1.0.2: `SET_HW_MUTE_BTN_MAP 0,1,0,...` — per-przycisk przełączanie mute suwaka.
fn build_hw_mute_btn_map_cmd(map: &[bool; MEDIA_BUTTON_SLOTS]) -> String {
    let mut s = String::from("SET_HW_MUTE_BTN_MAP ");
    for i in 0..MEDIA_BUTTON_SLOTS {
        if i > 0 {
            s.push(',');
        }
        s.push(if map[i] { '1' } else { '0' });
    }
    s.push('\n');
    s
}

fn enqueue_hw_mute_btn_map(queue: &Arc<Mutex<Option<mpsc::Sender<String>>>>, map: &[bool; MEDIA_BUTTON_SLOTS]) {
    let line = build_hw_mute_btn_map_cmd(map);
    enqueue_line_cmd(queue, &line);
}

/// Firmware ≥ 1.0.7 / proto 3: `SET_SHORTCUT_MUTE_LED_MAP 0,1,0,...`
fn build_shortcut_mute_led_map_cmd(map: &[bool; MEDIA_BUTTON_SLOTS]) -> String {
    let mut s = String::from("SET_SHORTCUT_MUTE_LED_MAP ");
    for i in 0..MEDIA_BUTTON_SLOTS {
        if i > 0 {
            s.push(',');
        }
        s.push(if map[i] { '1' } else { '0' });
    }
    s.push('\n');
    s
}

fn enqueue_shortcut_mute_led_map(queue: &Arc<Mutex<Option<mpsc::Sender<String>>>>, map: &[bool; MEDIA_BUTTON_SLOTS]) {
    let line = build_shortcut_mute_led_map_cmd(map);
    enqueue_line_cmd(queue, &line);
}

/// Proto ≥ 3: pełna mapa; starsze FW — tylko global `SET_SHORTCUT_MUTE_LED` (dowolny slot ON = 1).
fn enqueue_shortcut_mute_led_from_state(
    map: &[bool; MEDIA_BUTTON_SLOTS],
    queue: &Arc<Mutex<Option<mpsc::Sender<String>>>>,
    device: &Arc<Mutex<Option<DeviceInfo>>>,
) {
    let proto = device
        .lock()
        .ok()
        .and_then(|g| g.clone())
        .and_then(|d| d.proto)
        .unwrap_or(0);
    if proto < 3 {
        let any = map.iter().any(|&x| x);
        let line = format!(
            "SET_SHORTCUT_MUTE_LED {}\n",
            if any { '1' } else { '0' }
        );
        enqueue_line_cmd(queue, &line);
    } else {
        enqueue_shortcut_mute_led_map(queue, map);
    }
}

impl SerialState {
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(Mutex::new(HashMap::new())),
            active_port: Arc::new(Mutex::new(None)),
            last_connected_port: Arc::new(Mutex::new(None)),
            // Domyślnie: brak przypisań suwaków.
            default_assignments: Arc::new(Mutex::new(default_assignments())),
            // Domyślnie: wszystkie przyciski = None (bez akcji).
            default_button_bindings: Arc::new(Mutex::new(std::array::from_fn(|_| ButtonBinding::None))),
            // Domyślnie: brak mute na suwaku, brak LED po skrócie.
            default_button_hw_slider_mute: Arc::new(Mutex::new([false; MEDIA_BUTTON_SLOTS])),
            default_shortcut_mute_led_map: Arc::new(Mutex::new([false; MEDIA_BUTTON_SLOTS])),
            button_media_keys: Arc::new(AtomicBool::new(false)),
            connect_serial_mutex: Arc::new(Mutex::new(())),
            session_connect_order: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

/// Zwraca tylko nazwy portów (bez otwierania). Szybkie — do wykrywania zmiany listy COM.
#[tauri::command]
pub fn get_port_names() -> Result<Vec<String>, String> {
    serialport::available_ports()
        .map_err(|e| e.to_string())
        .map(|ports| ports.into_iter().map(|p| p.port_name).collect())
}

/// List all available serial ports (COM on Windows, /dev/tty* on Linux).
#[tauri::command]
pub fn list_serial_ports() -> Result<Vec<SerialPortInfo>, String> {
    serialport::available_ports().map_err(|e| e.to_string()).map(|ports| {
        ports
            .into_iter()
            .map(|p| {
                let (name, description) = match &p.port_type {
                    SerialPortType::UsbPort(usb) => (
                        p.port_name.clone(),
                        format!(
                            "{} {} ({}:{})",
                            usb.manufacturer.as_deref().unwrap_or(""),
                            usb.product.as_deref().unwrap_or("USB Serial"),
                            usb.vid,
                            usb.pid
                        )
                        .trim()
                        .to_string(),
                    ),
                    _ => (p.port_name.clone(), "Serial port".to_string()),
                };
                SerialPortInfo { name, description }
            })
            .collect()
    })
}

/// Czyści bufor RX (stare linie po poprzednim otwarciu portu / skanie).
fn drain_serial_rx(port: &mut dyn SerialPort) {
    let prev = port.timeout();
    let _ = port.clear(ClearBuffer::Input);
    let mut buf = [0u8; 512];
    let _ = port.set_timeout(Duration::from_millis(15));
    for _ in 0..96 {
        match port.read(&mut buf) {
            Ok(n) => {
                if n == 0 {
                    break;
                }
            }
            Err(_) => break,
        }
    }
    let _ = port.set_timeout(prev);
}

/// DTR + RTS — wiele układów CDC używa obu do sygnalizacji „host gotowy”.
fn set_host_serial_signals(port: &mut dyn SerialPort) {
    let _ = port.write_data_terminal_ready(true);
    let _ = port.write_request_to_send(true);
}

fn do_scan_idei_ports() -> Vec<String> {
    let ports = match serialport::available_ports() {
        Ok(p) => p,
        Err(_) => return vec![],
    };
    // Skanuj głównie USB — unika wolnego otwierania portów BT / wirtualnych.
    let usb_only: Vec<_> = ports
        .iter()
        .filter(|p| matches!(p.port_type, SerialPortType::UsbPort(_)))
        .cloned()
        .collect();
    let to_scan = if usb_only.is_empty() { ports } else { usb_only };

    let mut found = Vec::new();
    for p in to_scan {
        let name = p.port_name.clone();
        let mut port = match serialport::new(name.clone(), BAUDRATE)
            .data_bits(serialport::DataBits::Eight)
            .stop_bits(serialport::StopBits::One)
            .parity(serialport::Parity::None)
            .timeout(Duration::from_millis(HANDSHAKE_READ_TIMEOUT_MS))
            .open()
        {
            Ok(p) => p,
            Err(_) => continue,
        };
        set_host_serial_signals(&mut *port);
        thread::sleep(Duration::from_millis(100));
        drain_serial_rx(&mut *port);
        let _ = port.set_timeout(Duration::from_millis(HANDSHAKE_READ_TIMEOUT_MS));
        let _ = port.write_all(b"IDENTIFY\n");
        let _ = port.flush();

        let mut line_buf = Vec::new();
        let mut byte_buf = [0u8; 256];
        let mut is_idei = false;
        let scan_deadline = Instant::now() + Duration::from_millis(1200);
        while Instant::now() < scan_deadline && !is_idei {
            match port.read(&mut byte_buf) {
                Ok(n) => {
                    if n == 0 {
                        continue;
                    }
                    for &b in &byte_buf[..n] {
                        if b == b'\n' || b == b'\r' {
                            if !line_buf.is_empty() {
                                let line = String::from_utf8_lossy(&line_buf);
                                if parse_ready_or_identify(line.trim()).is_some() {
                                    is_idei = true;
                                    break;
                                }
                                line_buf.clear();
                            }
                        } else if line_buf.len() >= MAX_CDC_LINE_BYTES {
                            line_buf.clear();
                        } else {
                            line_buf.push(b);
                        }
                    }
                }
                Err(_) => {}
            }
        }
        drop(port);
        if is_idei {
            found.push(name);
        }
    }
    found
}

/// Skanuje porty synchronicznie (do użycia gdy potrzebna od razu lista).
#[tauri::command]
pub fn scan_idei_ports() -> Result<Vec<String>, String> {
    Ok(do_scan_idei_ports())
}

/// Uruchamia skan w tle; wynik przychodzi zdarzeniem "scan-complete" (payload: string[]). UI nie wisi.
/// Krótkie opóźnienie przed emit daje czas na zwolnienie portu przez OS po skanie.
#[tauri::command]
pub fn start_scan_idei_ports(app: AppHandle) -> Result<(), String> {
    let app = app.clone();
    thread::spawn(move || {
        let list = do_scan_idei_ports();
        thread::sleep(Duration::from_millis(450));
        app.emit("scan-complete", list).ok();
    });
    Ok(())
}

/// Backend watchdog: jeśli port fizycznie znika z systemu (kabel out),
/// zamykamy sesję i emitujemy `device-disconnected*` bez czekania na read timeout.
pub fn start_port_presence_monitor(app: AppHandle, state: &SerialState) {
    let sessions = state.sessions.clone();
    let active_port = state.active_port.clone();
    let connect_order = state.session_connect_order.clone();
    thread::spawn(move || loop {
        thread::sleep(Duration::from_millis(450));

        let present_ports: Vec<String> = match serialport::available_ports() {
            Ok(ports) => ports.into_iter().map(|p| p.port_name).collect(),
            Err(_) => continue,
        };

        let candidates: Vec<(String, Arc<Mutex<Option<Box<dyn SerialPort>>>>)> = {
            let map = match sessions.lock() {
                Ok(m) => m,
                Err(_) => continue,
            };
            map.iter()
                .map(|(p, rt)| (p.clone(), rt.port.clone()))
                .collect()
        };

        let mut missing: Vec<String> = Vec::new();
        for (port_name, port_arc) in candidates {
            // 1) Fast path: port not listed by OS anymore.
            let listed = present_ports.iter().any(|p| p == &port_name);
            if !listed {
                missing.push(port_name);
                continue;
            }
            // 2) Liveness probe on already-open handle (some drivers keep COM listed after unplug).
            let dead = match port_arc.lock() {
                Ok(mut guard) => match guard.as_mut() {
                    Some(p) => {
                        let read_probe = p.bytes_to_read().is_err();
                        let clear_probe = p.clear(ClearBuffer::Input).is_err();
                        read_probe && clear_probe
                    }
                    None => true,
                },
                Err(_) => true,
            };
            if dead {
                missing.push(port_name);
            }
        }
        if missing.is_empty() {
            continue;
        }

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
            let _ = rt.handle.serial_cmd_tx.lock().ok().and_then(|mut g| g.take());
            if let Some(j) = rt.read_thread.take() {
                let _ = j.join();
            }
            if let Some(j) = rt.apply_thread.take() {
                let _ = j.join();
            }
            let _ = rt.port.lock().ok().and_then(|mut g| g.take());
            app.emit("device-disconnected-port", &port).ok();
            app.emit("device-disconnected", ()).ok();
        }

        let next_active = sessions
            .lock()
            .ok()
            .and_then(|m| m.keys().next().cloned());
        if let Ok(mut ap) = active_port.lock() {
            if ap.as_ref().map(|p| missing.iter().any(|m| m == p)).unwrap_or(false) {
                *ap = next_active;
            }
        }
    });
}

/// Monitoruje porty USB i automatycznie reconnectuje gdy urządzenie się pojawi lub zniknie.
fn get_session(state: &SerialState, port_name: Option<String>) -> Result<SessionHandle, String> {
    let port = if let Some(p) = port_name { p } else { state.active_port.lock().map_err(|e| e.to_string())?.clone().ok_or_else(|| "Brak aktywnego portu".to_string())? };
    let sessions = state.sessions.lock().map_err(|e| e.to_string())?;
    sessions.get(&port).map(|s| s.handle.clone()).ok_or_else(|| format!("Brak połączenia dla portu {}", port))
}

fn device_identity_key(device: &DeviceInfo) -> String {
    if let Some(uid) = &device.uid {
        let u = uid.trim();
        if !u.is_empty() {
            return format!("uid:{}", u);
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
    let mut out: [Vec<VolumeTarget>; crate::audio::MAX_SLIDERS] =
        std::array::from_fn(|_| vec![]);
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

fn ordered_session_ports(state: &SerialState) -> Vec<String> {
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

/// Jedna konfiguracja na cały `VolumeTarget` w całej aplikacji: pierwsze podłączone urządzenie wygrywa.
fn reconcile_global_volume_assignments(state: &SerialState) {
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

fn read_initial_slider_snapshot(
    port: &mut dyn SerialPort,
    slider_count: usize,
    timeout_ms: u64,
) -> Option<Vec<u16>> {
    if slider_count == 0 || slider_count > crate::audio::MAX_SLIDERS {
        return None;
    }
    let prev_timeout = port.timeout();
    let _ = port.set_timeout(Duration::from_millis(25));
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    let mut line_buf: Vec<u8> = Vec::new();
    let mut byte_buf = [0u8; 128];
    while Instant::now() < deadline {
        match port.read(&mut byte_buf) {
            Ok(n) if n > 0 => {
                for &b in &byte_buf[..n] {
                    if b == b'\n' || b == b'\r' {
                        if line_buf.is_empty() {
                            continue;
                        }
                        let line = String::from_utf8_lossy(&line_buf)
                            .trim()
                            .trim_end_matches('\r')
                            .to_string();
                        line_buf.clear();
                        if let Some(parsed) = parse_slider_line(&line, slider_count) {
                            let _ = port.set_timeout(prev_timeout);
                            return Some(parsed);
                        }
                    } else if line_buf.len() >= MAX_CDC_LINE_BYTES {
                        line_buf.clear();
                    } else {
                        line_buf.push(b);
                    }
                }
            }
            Ok(_) => {}
            Err(_) => {}
        }
    }
    let _ = port.set_timeout(prev_timeout);
    None
}

/// Connect to a serial port. Całe I/O w tle — komenda od razu wraca; wynik przez eventy "device-connected" / "connect-error".
#[tauri::command]
pub fn connect_serial(
    app: AppHandle,
    port_name: String,
    state: tauri::State<SerialState>,
) -> Result<(), String> {
    let _connect_guard = state.connect_serial_mutex.lock().map_err(|e| e.to_string())?;
    if state.sessions.lock().map_err(|e| e.to_string())?.contains_key(&port_name) {
        if let Ok(mut a) = state.active_port.lock() { *a = Some(port_name.clone()); }
        return Ok(());
    }
    let (mut device, mut port) = do_connect_handshake(port_name.clone())?;
    device.port = port_name.clone();

    let initial_snapshot = read_initial_slider_snapshot(&mut *port, device.sliders as usize, 260);
    let port_arc = Arc::new(Mutex::new(Some(port)));
    let last_values = Arc::new(Mutex::new([0; crate::audio::MAX_SLIDERS]));
    if let Some(vals) = initial_snapshot {
        if let Ok(mut g) = last_values.lock() {
            for i in 0..crate::audio::MAX_SLIDERS {
                g[i] = *vals.get(i).unwrap_or(&0u16);
            }
        }
    }
    let mut init_assignments = state.default_assignments.lock().map_err(|e| e.to_string())?.clone();
    let mut init_bindings = state.default_button_bindings.lock().map_err(|e| e.to_string())?.clone();
    let mut init_hw_mute = state.default_button_hw_slider_mute.lock().map_err(|e| e.to_string())?.clone();
    let mut init_shortcut_map = state.default_shortcut_mute_led_map.lock().map_err(|e| e.to_string())?.clone();
    let cfg = load_config(&app);
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
                init_shortcut_map[i] = *dc.shortcut_mute_led_map.get(i).unwrap_or(&false);
            }
        }
    }

    let assignments = Arc::new(Mutex::new(init_assignments));
    let button_bindings = Arc::new(Mutex::new(init_bindings));
    let button_hw_slider_mute = Arc::new(Mutex::new(init_hw_mute));
    let shortcut_mute_led_map = Arc::new(Mutex::new(init_shortcut_map));
    let serial_cmd_tx = Arc::new(Mutex::new(None));
    let stop_requested = Arc::new(AtomicBool::new(false));
    let (cmd_tx, cmd_rx) = mpsc::channel::<String>();
    if let Ok(mut g) = serial_cmd_tx.lock() { *g = Some(cmd_tx); }
    let app_read = app.clone();
    let port_for_read = port_arc.clone();
    let values_for_read = last_values.clone();
    let stop_for_read = stop_requested.clone();
    let bindings_for_read = button_bindings.clone();
    let button_media = state.button_media_keys.clone();
    let read_port_name = port_name.clone();
    let read_join = thread::spawn(move || {
        read_loop(port_for_read, app_read, read_port_name, values_for_read, device.sliders as usize, stop_for_read, button_media, bindings_for_read, cmd_rx);
    });

    let values_for_apply = last_values.clone();
    let assignments_for_apply = assignments.clone();
    let stop_for_apply = stop_requested.clone();
    let apply_join = thread::spawn(move || apply_volume_loop(values_for_apply, assignments_for_apply, device.sliders as usize, stop_for_apply));

    let handle = SessionHandle { device: device.clone(), stop_requested, last_slider_values: last_values, volume_assignments: assignments, button_bindings, button_hw_slider_mute, shortcut_mute_led_map, serial_cmd_tx };
    let runtime = SessionRuntime { handle: handle.clone(), read_thread: Some(read_join), apply_thread: Some(apply_join), port: port_arc };
    state.sessions.lock().map_err(|e| e.to_string())?.insert(port_name.clone(), runtime);
    if let Ok(mut ord) = state.session_connect_order.lock() {
        if !ord.iter().any(|p| p == &port_name) {
            ord.push(port_name.clone());
        }
    }
    reconcile_global_volume_assignments(&state);
    if let Ok(mut a) = state.active_port.lock() { *a = Some(port_name.clone()); }
    if let Ok(mut l) = state.last_connected_port.lock() { *l = Some(port_name.clone()); }
    // Reapply per-device firmware-side maps on reconnect.
    let hw_map = handle
        .button_hw_slider_mute
        .lock()
        .ok()
        .map(|g| *g)
        .unwrap_or([false; MEDIA_BUTTON_SLOTS]);
    enqueue_hw_mute_btn_map(&handle.serial_cmd_tx, &hw_map);
    let led_map = handle
        .shortcut_mute_led_map
        .lock()
        .ok()
        .map(|g| *g)
        .unwrap_or([false; MEDIA_BUTTON_SLOTS]);
    enqueue_shortcut_mute_led_from_state(
        &led_map,
        &handle.serial_cmd_tx,
        &Arc::new(Mutex::new(Some(handle.device.clone()))),
    );
    enqueue_line_cmd(&handle.serial_cmd_tx, "GET_STATE\n");
    let _ = save_config(&app, &build_config_from_state(&app, &state).unwrap_or_else(|_| load_config(&app)));

    app.emit("device-connected", &device).ok();
    app.emit("device-connected-port", &device).ok();
    if let Ok(g) = handle.last_slider_values.lock() {
        let payload = SliderValuesPayload {
            port: device.port.clone(),
            values: g.iter().take(device.sliders as usize).copied().collect(),
        };
        app.emit("slider-values", &payload).ok();
        app.emit("slider-values-port", &payload).ok();
    }
    let ports = ordered_session_ports(&state);
    let _ = app.emit("assignments-reconciled", &ports);
    Ok(())
}

/// Czyta linie do pierwszego JSON `ready` / `identify` z poprawnym `model`.
fn read_until_identify(port: &mut dyn SerialPort, deadline: Instant) -> Result<DeviceInfo, ()> {
    let mut line_buf = Vec::new();
    let mut byte_buf = [0u8; 256];
    let start = Instant::now();
    let mut resent_identify = false;

    while Instant::now() < deadline {
        if !resent_identify && start.elapsed() >= Duration::from_millis(900) {
            let _ = port.write_all(b"IDENTIFY\n");
            let _ = port.flush();
            resent_identify = true;
        }
        match port.read(&mut byte_buf) {
            Ok(n) => {
                if n == 0 {
                    continue;
                }
                for &b in &byte_buf[..n] {
                    if b == b'\n' || b == b'\r' {
                        if !line_buf.is_empty() {
                            let line = String::from_utf8_lossy(&line_buf);
                            let line = line.trim();
                            if let Some(dev) = parse_ready_or_identify(line) {
                                return Ok(dev);
                            }
                            line_buf.clear();
                        }
                    } else if line_buf.len() >= MAX_CDC_LINE_BYTES {
                        line_buf.clear();
                    } else {
                        line_buf.push(b);
                    }
                }
            }
            Err(_) => {}
        }
    }
    Err(())
}

fn do_connect_handshake_attempt(
    port_name: String,
) -> Result<(DeviceInfo, Box<dyn SerialPort>), String> {
    let mut port = serialport::new(port_name.clone(), BAUDRATE)
        .data_bits(serialport::DataBits::Eight)
        .stop_bits(serialport::StopBits::One)
        .parity(serialport::Parity::None)
        .timeout(Duration::from_millis(HANDSHAKE_READ_TIMEOUT_MS))
        .open()
        .map_err(|e| format!("Nie można otworzyć portu: {}", e))?;

    set_host_serial_signals(&mut *port);
    thread::sleep(Duration::from_millis(180));
    drain_serial_rx(&mut *port);
    let _ = port.set_timeout(Duration::from_millis(HANDSHAKE_READ_TIMEOUT_MS));
    thread::sleep(Duration::from_millis(40));
    let _ = port.write_all(b"IDENTIFY\n");
    let _ = port.flush();

    let deadline = Instant::now() + Duration::from_millis(HANDSHAKE_DEADLINE_MS);
    let mut device = read_until_identify(&mut *port, deadline).map_err(|()| {
        "Urządzenie nie odpowiedziało (ready/IDENTIFY). Sprawdź czy to IDEI Mix.".to_string()
    })?;
    device.port = port_name;
    let _ = port.set_timeout(Duration::from_millis(READ_TIMEOUT_MS));
    Ok((device, port))
}

/// Otwiera port, DTR+RTS, czyści RX, IDENTIFY (z powtórką), czyta JSON. Ponowienia przy błędzie.
pub(crate) fn do_connect_handshake(
    port_name: String,
) -> Result<(DeviceInfo, Box<dyn SerialPort>), String> {
    let mut last_err = String::from("Urządzenie nie odpowiedziało (ready/IDENTIFY). Sprawdź czy to IDEI Mix.");
    for attempt in 0..HANDSHAKE_MAX_ATTEMPTS {
        match do_connect_handshake_attempt(port_name.clone()) {
            Ok(ok) => return Ok(ok),
            Err(e) => {
                last_err = e;
                if attempt + 1 < HANDSHAKE_MAX_ATTEMPTS {
                    thread::sleep(Duration::from_millis(280 + 120 * u64::from(attempt)));
                }
            }
        }
    }
    Err(last_err)
}

/// Rozłączenie: zwraca od razu, cleanup w tle (żeby nie blokować ani nie crashować UI).
#[tauri::command]
pub fn disconnect_serial(app: AppHandle, state: tauri::State<SerialState>, port_name: Option<String>) -> Result<(), String> {
    if let Some(port) = port_name.or_else(|| state.active_port.lock().ok().and_then(|g| g.clone())) {
        let removed = state.sessions.lock().map_err(|e| e.to_string())?.remove(&port);
        if let Ok(mut ord) = state.session_connect_order.lock() {
            ord.retain(|p| p != &port);
        }
        if let Some(mut s) = removed {
            s.handle.stop_requested.store(true, Ordering::SeqCst);
            let _ = s.handle.serial_cmd_tx.lock().ok().and_then(|mut g| g.take());
            if let Some(j) = s.read_thread.take() { let _ = j.join(); }
            if let Some(j) = s.apply_thread.take() { let _ = j.join(); }
            let _ = s.port.lock().ok().and_then(|mut g| g.take());
            app.emit("device-disconnected-port", &port).ok();
            app.emit("device-disconnected", ()).ok();
        }
    } else {
        let ports: Vec<String> = state.sessions.lock().map_err(|e| e.to_string())?.keys().cloned().collect();
        for p in ports { let _ = disconnect_serial(app.clone(), state.clone(), Some(p)); }
    }
    Ok(())
}

/// Return current device if connected.
#[tauri::command]
pub fn get_connection_status(state: tauri::State<SerialState>) -> Option<DeviceInfo> {
    let port = state.active_port.lock().ok().and_then(|g| g.clone())?;
    state.sessions.lock().ok()?.get(&port).map(|s| s.handle.device.clone())
}

#[tauri::command]
pub fn get_connected_devices(state: tauri::State<SerialState>) -> Vec<DeviceInfo> {
    state.sessions.lock().ok().map(|m| m.values().map(|s| s.handle.device.clone()).collect()).unwrap_or_default()
}

/// Ostatnie wartości suwaków (0–1023). Frontend może odpytywać co ~150 ms gdy połączony.
#[tauri::command]
pub fn get_slider_values(state: tauri::State<SerialState>, port_name: Option<String>) -> Result<Vec<u16>, String> {
    let s = get_session(&state, port_name)?;
    let vals = s.last_slider_values.lock().map(|g| *g).unwrap_or([0; crate::audio::MAX_SLIDERS]);
    Ok(vals.iter().take(s.device.sliders as usize).copied().collect())
}

/// Przypisania suwaków: dla każdego suwaka lista celów (System i/lub App(pid, name)).
#[tauri::command]
pub fn get_volume_assignments(state: tauri::State<SerialState>, port_name: Option<String>) -> Result<[Vec<VolumeTarget>; crate::audio::MAX_SLIDERS], String> {
    let s = get_session(&state, port_name)?;
    Ok(s.volume_assignments.lock().map(|g| g.clone()).unwrap_or_else(|_| default_assignments()))
}

/// Ustawia listę celów dla suwaka (slider_index 0 .. MAX_SLIDERS-1).
#[tauri::command]
pub fn set_volume_assignment(
    app: AppHandle,
    state: tauri::State<SerialState>,
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
    if let Ok(cfg) = build_config_from_state(&app, &state) {
        let _ = save_config(&app, &cfg);
    }
    Ok(())
}

fn build_config_from_state(app: &AppHandle, state: &SerialState) -> Result<AppConfig, ()> {
    let mut cfg = load_config(app);
    let assignments = if let Some(ap) = state.active_port.lock().map_err(|_| ())?.clone() {
        state.sessions.lock().map_err(|_| ())?.get(&ap).map(|s| s.handle.volume_assignments.lock().ok().map(|x| x.clone()).unwrap_or_else(default_assignments)).unwrap_or_else(default_assignments)
    } else {
        state.default_assignments.lock().map_err(|_| ())?.clone()
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
            if let Ok(m) = sess.handle.shortcut_mute_led_map.lock() {
                cfg.shortcut_mute_led_map = Some(m.to_vec());
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
    if cfg.shortcut_mute_led_map.is_none() {
        if let Ok(m) = state.default_shortcut_mute_led_map.lock() {
            cfg.shortcut_mute_led_map = Some(m.to_vec());
        }
    }

    let mut per_device = cfg.device_configs.take().unwrap_or_default();
    if let Ok(sessions) = state.sessions.lock() {
        for (_port, sess) in sessions.iter() {
            let key = device_identity_key(&sess.handle.device);
            let entry = DeviceRuntimeConfig {
                volume_assignments: assignments_to_vec(
                    &sess.handle.volume_assignments.lock().ok().map(|g| g.clone()).unwrap_or_else(default_assignments),
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
                shortcut_mute_led_map: sess
                    .handle
                    .shortcut_mute_led_map
                    .lock()
                    .ok()
                    .map(|g| g.to_vec())
                    .unwrap_or_else(|| vec![false; MEDIA_BUTTON_SLOTS]),
            };
            per_device.insert(key, entry);
        }
    }
    cfg.device_configs = Some(per_device);
    Ok(cfg)
}

/// Zwraca konfigurację do UI: przypisania i ostatni port z bieżącego stanu, reszta z pliku.
#[tauri::command]
pub fn get_app_config(app: AppHandle, state: tauri::State<SerialState>) -> AppConfig {
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
                if let Ok(a) = sess.handle.button_bindings.lock() { cfg.button_bindings = Some(a.iter().cloned().collect()); }
                if let Ok(m) = sess.handle.button_hw_slider_mute.lock() { cfg.button_hw_slider_mute = Some(m.to_vec()); }
                if let Ok(m) = sess.handle.shortcut_mute_led_map.lock() { cfg.shortcut_mute_led_map = Some(m.to_vec()); }
            }
        }
    }
    cfg
}

/// Zapisuje bieżącą konfigurację (przypisania, ostatni port) do pliku.
#[tauri::command]
pub fn save_app_config(app: AppHandle, state: tauri::State<SerialState>) -> Result<(), String> {
    let cfg = build_config_from_state(&app, &state).map_err(|_| "Lock state".to_string())?;
    save_config(&app, &cfg)
}

/// Aktualizuje tylko preferencję autostartu w pliku konfiguracji.
#[tauri::command]
pub fn save_autostart_preference(app: AppHandle, enabled: bool) -> Result<(), String> {
    let mut cfg = load_config(&app);
    cfg.autostart = Some(enabled);
    save_config(&app, &cfg)
}

/// Wysyła jednoliniowe polecenie do urządzenia (np. `GET_STATE`, `RESET_DEFAULTS`), zakończone `\n`.
/// Kolejka jest obsługiwana w `read_loop` przy tym samym zamku co `read` — bez „serial busy”.
#[tauri::command]
pub fn send_device_command(state: tauri::State<SerialState>, port_name: Option<String>, cmd: String) -> Result<(), String> {
    let cmd = cmd.trim().to_string();
    if cmd.is_empty() {
        return Err("Puste polecenie".to_string());
    }
    let s = get_session(&state, port_name)?;
    let tx = s.serial_cmd_tx.lock().map_err(|e| e.to_string())?.clone().ok_or_else(|| "Brak połączenia".to_string())?;
    let line = format!("{}\n", cmd);
    tx.send(line).map_err(|_| "Nie można wysłać (rozłączono)".to_string())
}

#[tauri::command]
pub fn set_button_media_keys(app: AppHandle, state: tauri::State<SerialState>, enabled: bool) -> Result<(), String> {
    state.button_media_keys.store(enabled, Ordering::SeqCst);
    let mut cfg = load_config(&app);
    cfg.button_media_keys = Some(enabled);
    save_config(&app, &cfg)
}

#[tauri::command]
pub fn get_button_media_keys(state: tauri::State<SerialState>) -> bool {
    state.button_media_keys.load(Ordering::SeqCst)
}

#[tauri::command]
pub fn get_button_bindings(state: tauri::State<SerialState>, port_name: Option<String>) -> Result<Vec<ButtonBinding>, String> {
    let s = get_session(&state, port_name)?;
    Ok(s.button_bindings.lock()
        .map(|g| g.iter().cloned().collect())
        .unwrap_or_else(|_| default_button_bindings().iter().cloned().collect()))
}

#[tauri::command]
pub fn set_button_bindings(
    app: AppHandle,
    state: tauri::State<SerialState>,
    port_name: Option<String>,
    bindings: Vec<ButtonBinding>,
) -> Result<(), String> {
    if bindings.len() != MEDIA_BUTTON_SLOTS {
        return Err(format!("Wymagane dokładnie {} wpisów", MEDIA_BUTTON_SLOTS));
    }
    let norm: Vec<ButtonBinding> = bindings
        .iter()
        .map(|b| b.clone().normalized())
        .collect();
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
pub fn get_button_hw_slider_mute(state: tauri::State<SerialState>, port_name: Option<String>) -> Result<Vec<bool>, String> {
    let s = get_session(&state, port_name)?;
    Ok(s.button_hw_slider_mute
        .lock()
        .map(|g| g.to_vec())
        .unwrap_or_else(|_| vec![false; MEDIA_BUTTON_SLOTS]))
}

#[tauri::command]
pub fn set_button_hw_slider_mute(
    app: AppHandle,
    state: tauri::State<SerialState>,
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
    if let Ok(cfg) = build_config_from_state(&app, &state) {
        save_config(&app, &cfg)?;
    } else {
        let mut cfg = load_config(&app);
        cfg.button_hw_slider_mute = Some(enabled);
        save_config(&app, &cfg)?;
    }
    let map = s.button_hw_slider_mute.lock().map(|g| *g).unwrap_or([false; MEDIA_BUTTON_SLOTS]);
    enqueue_hw_mute_btn_map(&s.serial_cmd_tx, &map);
    Ok(())
}

#[tauri::command]
pub fn get_shortcut_mute_led_map(state: tauri::State<SerialState>, port_name: Option<String>) -> Result<Vec<bool>, String> {
    let s = get_session(&state, port_name)?;
    Ok(s.shortcut_mute_led_map
        .lock()
        .map(|g| g.to_vec())
        .unwrap_or_else(|_| vec![false; MEDIA_BUTTON_SLOTS]))
}

#[tauri::command]
pub fn set_shortcut_mute_led_map(
    app: AppHandle,
    state: tauri::State<SerialState>,
    port_name: Option<String>,
    enabled: Vec<bool>,
) -> Result<(), String> {
    if enabled.len() != MEDIA_BUTTON_SLOTS {
        return Err(format!("Wymagane {} wartości", MEDIA_BUTTON_SLOTS));
    }
    let s = get_session(&state, port_name)?;
    if let Ok(mut g) = s.shortcut_mute_led_map.lock() {
        for i in 0..MEDIA_BUTTON_SLOTS {
            g[i] = enabled[i];
        }
    }
    if let Ok(cfg) = build_config_from_state(&app, &state) {
        save_config(&app, &cfg)?;
    } else {
        let mut cfg = load_config(&app);
        cfg.shortcut_mute_led_map = Some(enabled);
        save_config(&app, &cfg)?;
    }
    let map = s.shortcut_mute_led_map.lock().map(|g| *g).unwrap_or([false; MEDIA_BUTTON_SLOTS]);
    enqueue_shortcut_mute_led_from_state(&map, &s.serial_cmd_tx, &Arc::new(Mutex::new(Some(s.device.clone()))));
    Ok(())
}

/// Wywoływane z frontendu po zdarzeniu device-disconnected (np. odłączenie kabla). Czyści stan. Bez panic przy już rozłączonym.
#[tauri::command]
pub fn notify_disconnected(state: tauri::State<SerialState>) -> Result<(), String> {
    let _ = state;
    Ok(())
}

/// Pętla w tle: co ~80 ms odczytuje wartości suwaków i przypisania, stosuje głośność (system + sesje).
pub(crate) fn apply_volume_loop(
    values_arc: Arc<Mutex<[u16; crate::audio::MAX_SLIDERS]>>,
    assignments_arc: Arc<Mutex<[Vec<VolumeTarget>; crate::audio::MAX_SLIDERS]>>,
    slider_count: usize,
    stop: Arc<AtomicBool>,
) {
    while !stop.load(Ordering::SeqCst) {
        let n = slider_count;
        let values = values_arc.lock().ok().map(|g| *g).unwrap_or([0; crate::audio::MAX_SLIDERS]);
        let assignments = assignments_arc
            .lock()
            .ok()
            .map(|g| g.clone())
            .unwrap_or_else(default_assignments);
        crate::audio::apply_volume_mapping(&values, &assignments, n);
        thread::sleep(Duration::from_millis(80));
    }
}

fn parse_ready_or_identify(line: &str) -> Option<DeviceInfo> {
    let line = line.trim();
    if !line.starts_with('{') {
        return None;
    }
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    let model = v.get("model").and_then(|m| m.as_str())?;
    if !IDEI_MODELS.iter().any(|&m| m == model) {
        return None;
    }
    let json_sliders = v.get("sliders").and_then(|s| s.as_u64());
    let sliders = effective_slider_count(model, json_sliders);
    let proto = v.get("proto").and_then(|p| p.as_u64()).map(|u| u as u32);
    let fw = v.get("fw").and_then(|s| s.as_str()).map(|s| s.to_string());
    let buttons = v.get("buttons").and_then(|b| b.as_u64()).map(|u| u as u32);
    let caps = v.get("caps").and_then(|c| c.as_array()).map(|arr| {
        arr.iter()
            .filter_map(|x| x.as_str().map(|s| s.to_string()))
            .collect()
    });
    Some(DeviceInfo {
        port: String::new(),
        model: model.to_string(),
        sliders,
        proto,
        fw,
        buttons,
        caps,
        uid: v
            .get("uid")
            .and_then(|x| x.as_str())
            .map(|s| s.to_string())
            .or_else(|| v.get("serial").and_then(|x| x.as_str()).map(|s| s.to_string())),
    })
}

/// Parsuje linię deej `v0|v1|…` (0–1023). Wymaga co najmniej jednego pola; brakujące uzupełnia zerami,
/// nadmiarowe pola ignoruje — żeby 3‑suwakowe FW działało przy błędnym `sliders` w JSON i odwrotnie.
fn parse_slider_line(line: &str, n: usize) -> Option<Vec<u16>> {
    if n == 0 || n > crate::audio::MAX_SLIDERS {
        return None;
    }
    let line = line.trim().trim_start_matches('\u{feff}').trim_end_matches('\r');
    if line.is_empty() || line.starts_with('{') {
        return None;
    }
    let parts: Vec<&str> = line.split('|').collect();
    let m = parts.len();
    if m == 0 || m > crate::audio::MAX_SLIDERS {
        return None;
    }
    let mut values = vec![0u16; n];
    for i in 0..n {
        if i < m {
            let v = parts[i].trim().parse::<u16>().ok()?;
            if v > 1023 {
                return None;
            }
            values[i] = v;
        }
    }
    Some(values)
}

pub(crate) fn read_loop(
    port_arc: Arc<Mutex<Option<Box<dyn SerialPort>>>>,
    app: AppHandle,
    port_name: String,
    last_values: Arc<Mutex<[u16; crate::audio::MAX_SLIDERS]>>,
    slider_count: usize,
    stop: Arc<AtomicBool>,
    button_media_keys: Arc<AtomicBool>,
    button_bindings: Arc<Mutex<[ButtonBinding; MEDIA_BUTTON_SLOTS]>>,
    outgoing_rx: mpsc::Receiver<String>,
) {
    let mut last_button_seq: Option<u64> = None;
    let mut line_buf = Vec::new();
    let mut byte_buf = [0u8; 128];
    let mut timeout_streak: u32 = 0;

    loop {
        if stop.load(Ordering::SeqCst) {
            break;
        }
        let n = {
            let mut guard = match port_arc.lock() {
                Ok(g) => g,
                Err(_) => break,
            };
            let port = match guard.as_mut() {
                Some(p) => p,
                None => break,
            };
            // Jedna linia CDC na iterację (read potem) — burst try_recv + write blokował USB CDC (błąd 121).
            if let Ok(line) = outgoing_rx.try_recv() {
                match write_cdc_line(port.as_mut(), &line) {
                    Ok(()) => {
                        let shown = line.trim_end_matches('\n').trim_end_matches('\r');
                        if !shown.is_empty() {
                            let _ = app.emit("log", format!("→ {}", shown));
                        }
                    }
                    Err(e) => {
                        let _ = app.emit("log", format!("→ (write error: {})", e));
                    }
                }
                thread::sleep(Duration::from_millis(CDC_WRITE_GAP_MS));
            }
            match port.read(&mut byte_buf) {
                Ok(n) => {
                    timeout_streak = 0;
                    n
                }
                Err(e) => {
                    let msg = e.to_string().to_lowercase();
                    let is_timeout = msg.contains("timeout") || msg.contains("timed out")
                        || msg.contains("would block");
                    if is_timeout {
                        timeout_streak = timeout_streak.saturating_add(1);
                        // On some Windows drivers unplug may look like endless timeouts.
                        // Every ~1s verify port health using a cheap driver call.
                        if timeout_streak >= 20 {
                            timeout_streak = 0;
                            if port.bytes_to_read().is_err() {
                                drop(guard);
                                let _ = app.emit("device-disconnected-port", &port_name);
                                let _ = app.emit("device-disconnected", ());
                                break;
                            }
                        }
                        continue;
                    }
                    drop(guard);
                    let _ = app.emit("device-disconnected-port", &port_name);
                    let _ = app.emit("device-disconnected", ());
                    break;
                }
            }
        };

        for &b in &byte_buf[..n] {
            if b == b'\n' || b == b'\r' {
                if line_buf.is_empty() {
                    continue;
                }
                let line = String::from_utf8_lossy(&line_buf)
                    .trim()
                    .trim_end_matches('\r')
                    .to_string();
                line_buf.clear();

                if line.starts_with('{') {
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) {
                        if v.get("type").and_then(|t| t.as_str()) == Some("btn") {
                            let idx = v.get("i").and_then(|x| x.as_u64()).unwrap_or(0) as u32;
                            let seq = v.get("seq").and_then(|x| x.as_u64()).unwrap_or(0);
                            let mut skip_dup = false;
                            if Some(seq) == last_button_seq {
                                    skip_dup = true;
                            } else {
                                last_button_seq = Some(seq);
                            }
                            if !skip_dup {
                                let payload = DeviceButtonPayload { port: port_name.clone(), index: idx, seq };
                                let _ = app.emit("device-button", &payload);
                                let _ = app.emit("device-button-port", &payload);
                                if button_media_keys.load(Ordering::SeqCst) {
                                    let bi = idx as usize;
                                    let binding = button_bindings
                                        .lock()
                                        .ok()
                                        .and_then(|g| g.get(bi).cloned())
                                        .unwrap_or(ButtonBinding::None);
                                    if !matches!(binding, ButtonBinding::None) {
                                        send_button_binding(&binding);
                                    }
                                }
                            }
                        } else if v.get("type").and_then(|t| t.as_str()) == Some("state") {
                            if let Some(vals) = v.get("values").and_then(|x| x.as_array()) {
                                let mut parsed = vec![0u16; slider_count];
                                for i in 0..slider_count {
                                    if let Some(u) = vals.get(i).and_then(|x| x.as_u64()) {
                                        parsed[i] = (u.min(1023)) as u16;
                                    }
                                }
                                if let Ok(mut g) = last_values.lock() {
                                    for i in 0..crate::audio::MAX_SLIDERS {
                                        g[i] = *parsed.get(i).unwrap_or(&0u16);
                                    }
                                }
                                let payload = SliderValuesPayload { port: port_name.clone(), values: parsed };
                                app.emit("slider-values", &payload).ok();
                                app.emit("slider-values-port", &payload).ok();
                            }
                        } else {
                            let short = if line.len() > 400 {
                                format!("{}…", &line[..400])
                            } else {
                                line.clone()
                            };
                            let _ = app.emit("log", format!("← {}", short));
                        }
                    }
                    continue;
                }
                let n = slider_count;
                if n == 0 {
                    continue;
                }
                if let Some(parsed) = parse_slider_line(&line, n) {
                    if let Ok(mut g) = last_values.lock() {
                        for i in 0..crate::audio::MAX_SLIDERS {
                            g[i] = *parsed.get(i).unwrap_or(&0u16);
                        }
                    }
                    let payload = SliderValuesPayload { port: port_name.clone(), values: parsed };
                    app.emit("slider-values", &payload).ok();
                    app.emit("slider-values-port", &payload).ok();
                }
            } else {
                if line_buf.len() >= MAX_CDC_LINE_BYTES {
                    line_buf.clear();
                } else {
                    line_buf.push(b);
                }
            }
        }
    }
}
