//! Sterowanie głośnością systemu i sesji aplikacji (Windows: Core Audio, Linux: PulseAudio).

#[cfg(target_os = "linux")]
mod audio_linux;

use serde::{Deserialize, Serialize};

/// Maksymalna liczba suwaków (wszystkie modele); aktywna liczba zależy od `DeviceInfo.sliders`.
pub const MAX_SLIDERS: usize = 5;

/// Pojedynczy cel głośności: system (master), aplikacja (PID) lub kategoria (np. "gry").
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum VolumeTarget {
    System,
    Mic,
    App {
        pid: u32,
        #[serde(default)]
        name: String,
    },
    /// Kategoria: np. "gry" — steruje wszystkimi wykrytymi grami (Steam, Epic, Minecraft itd.).
    Category {
        #[serde(rename = "id")]
        category_id: String,
    },
}

/// Czy dwa cele nie mogą być sterowane jednocześnie z różnych suwaków/urządzeń (globalny konflikt).
/// Dla aplikacji: ten sam PID **albo** ta sama sensowna nazwa (PID zmienia się po restarcie procesu).
pub fn volume_targets_conflict(a: &VolumeTarget, b: &VolumeTarget) -> bool {
    match (a, b) {
        (VolumeTarget::System, VolumeTarget::System) => true,
        (VolumeTarget::Mic, VolumeTarget::Mic) => true,
        (
            VolumeTarget::App {
                pid: pa,
                name: na,
            },
            VolumeTarget::App {
                pid: pb,
                name: nb,
            },
        ) => {
            if pa == pb {
                return true;
            }
            let na = normalize_app_name(na);
            let nb = normalize_app_name(nb);
            !na.is_empty() && na == nb
        }
        (
            VolumeTarget::Category { category_id: ca },
            VolumeTarget::Category { category_id: cb },
        ) => ca == cb,
        _ => false,
    }
}

/// Jedna sesja audio (aplikacja) — do wyświetlenia w UI.
#[derive(Clone, Serialize, Deserialize)]
pub struct AudioSessionInfo {
    pub pid: u32,
    pub name: String,
    /// Czy sesja ma aktualnie odtwarzany dźwięk (stan Active).
    #[serde(default)]
    pub is_active: bool,
    /// Czy proces uznany za grę (Steam, Epic, Minecraft, Origin itd.).
    #[serde(default)]
    pub is_game: bool,
}

/// Zwraca aktualną głośność master (0.0 ..= 1.0).
#[tauri::command]
pub fn get_system_volume() -> Result<f32, String> {
    get_system_volume_impl()
}

/// Ustawia głośność master (0.0 ..= 1.0).
#[tauri::command]
pub fn set_system_volume(level: f32) -> Result<(), String> {
    let level = level.clamp(0.0, 1.0);
    set_system_volume_impl(level)
}

/// Lista sesji audio (aplikacji z głośnością) — do menu przypisań.
#[tauri::command]
pub fn get_audio_sessions() -> Result<Vec<AudioSessionInfo>, String> {
    get_audio_sessions_impl()
}

/// Ustawia głośność sesji (aplikacji) po PID (0.0 ..= 1.0).
#[allow(dead_code)]
pub fn set_session_volume(pid: u32, level: f32) -> Result<(), String> {
    set_session_volume_impl(pid, level.clamp(0.0, 1.0))
}

/// Stosuje mapowanie: dla każdego suwaka i każdego przypisanego celu ustawia głośność.
/// assignments[i] = lista celów (System i/lub App(pid)) dla suwaka i.
pub fn apply_volume_mapping(
    values: &[u16; MAX_SLIDERS],
    assignments: &[Vec<VolumeTarget>; MAX_SLIDERS],
    active_sliders: usize,
) {
    let n = active_sliders.min(MAX_SLIDERS);
    for i in 0..n {
        let level = (values[i] as f32 / 1023.0).clamp(0.0, 1.0);
        if let Some(targets) = assignments.get(i) {
            for t in targets {
                match t {
                    VolumeTarget::System => {
                        let _ = set_system_volume_impl(level);
                    }
                    VolumeTarget::Mic => {
                        let _ = set_microphone_volume_impl(level);
                    }
                    VolumeTarget::App { pid, name } => {
                        // Prefer stable app name matching; PID stays as fallback.
                        let applied_by_name = if !normalize_app_name(name).is_empty() {
                            set_session_volume_by_name_impl(name, level).is_ok()
                        } else {
                            false
                        };
                        if !applied_by_name {
                            let _ = set_session_volume_impl(*pid, level);
                        }
                    }
                    VolumeTarget::Category { category_id } if category_id == "gry" => {
                        if let Ok(pids) = get_game_pids_impl() {
                            for pid in pids {
                                let _ = set_session_volume_impl(pid, level);
                            }
                        }
                    }
                    VolumeTarget::Category { .. } => {}
                }
            }
        }
    }
}

pub(crate) fn normalize_app_name(name: &str) -> String {
    let s = name.trim().to_lowercase();
    if s.is_empty() || s.starts_with("pid ") {
        return String::new();
    }
    s.trim_end_matches(".exe").to_string()
}

// CLSID MMDeviceEnumerator: BCDE0395-E52F-467C-8E3D-C4579291692E (mmdeviceapi.h)
#[cfg(windows)]
const CLSID_MMDEVICE_ENUMERATOR: windows::core::GUID = windows::core::GUID::from_values(
    0xBCDE0395,
    0xE52F,
    0x467C,
    [0x8E, 0x3D, 0xC4, 0x57, 0x92, 0x91, 0x69, 0x2E],
);

/// Ścieżki/ słowa kluczowe wskazujące na grę (Steam, Epic, Minecraft, Origin, GOG, itd.).
#[cfg(windows)]
fn is_game_path(path: &str) -> bool {
    let lower = path.to_lowercase();
    let keywords = [
        "steam",
        "steamapps",
        "epic games",
        "epicgames",
        "origin",
        "minecraft",
        "gog galaxy",
        "gog.com",
        "ubisoft",
        "ubisoft game launcher",
        "battle.net",
        "battlenet",
        "ea app",
        "ea desktop",
        "xbox game",
        "game bar",
        "rpg",
        "\\games\\",
        "\\game\\",
    ];
    keywords.iter().any(|k| lower.contains(k))
}

#[cfg(windows)]
fn get_process_path_impl(pid: u32) -> Option<String> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 260];
        let mut size = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(
            handle,
            windows::Win32::System::Threading::PROCESS_NAME_WIN32,
            windows::core::PWSTR(buf.as_mut_ptr()),
            &mut size,
        )
        .is_ok();
        let _ = CloseHandle(handle);
        if !ok {
            return None;
        }
        let path = String::from_utf16(&buf[..size as usize]).ok()?;
        Some(path.trim_end_matches('\0').to_string())
    }
}

#[cfg(windows)]
fn get_game_pids_impl() -> Result<Vec<u32>, String> {
    use std::mem;
    use windows::core::Interface;
    use windows::Win32::Media::Audio::{EDataFlow, ERole};
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, COINIT_APARTMENTTHREADED, CLSCTX_ALL,
    };

    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok();
        let enumerator: windows::Win32::Media::Audio::IMMDeviceEnumerator =
            CoCreateInstance(&CLSID_MMDEVICE_ENUMERATOR, None, CLSCTX_ALL)
                .map_err(|e| format!("CoCreateInstance: {}", e))?;
        let data_flow: EDataFlow = mem::transmute(0i32);
        let role: ERole = mem::transmute(0i32);
        let device = enumerator
            .GetDefaultAudioEndpoint(data_flow, role)
            .map_err(|e| format!("GetDefaultAudioEndpoint: {}", e))?;
        let session_manager: windows::Win32::Media::Audio::IAudioSessionManager2 = device
            .Activate(CLSCTX_ALL, None)
            .map_err(|e| format!("Activate SessionManager: {}", e))?;
        let enumerator_sessions = session_manager
            .GetSessionEnumerator()
            .map_err(|e| format!("GetSessionEnumerator: {}", e))?;
        let count = enumerator_sessions
            .GetCount()
            .map_err(|e| format!("GetCount: {}", e))?;
        let mut pids = Vec::new();
        for idx in 0..count {
            let session: windows::Win32::Media::Audio::IAudioSessionControl = enumerator_sessions
                .GetSession(idx)
                .map_err(|e| format!("GetSession: {}", e))?;
            let session2: windows::Win32::Media::Audio::IAudioSessionControl2 = session
                .cast()
                .map_err(|e| format!("IAudioSessionControl2: {}", e))?;
            let pid = session2.GetProcessId().unwrap_or(0);
            if pid != 0 {
                if let Some(path) = get_process_path_impl(pid) {
                    if is_game_path(&path) {
                        pids.push(pid);
                    }
                }
            }
        }
        Ok(pids)
    }
}

#[cfg(windows)]
fn get_system_volume_impl() -> Result<f32, String> {
    use std::mem;
    use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
    use windows::Win32::Media::Audio::{EDataFlow, ERole};
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, COINIT_APARTMENTTHREADED, CLSCTX_ALL,
    };

    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok();
        let enumerator: windows::Win32::Media::Audio::IMMDeviceEnumerator =
            CoCreateInstance(&CLSID_MMDEVICE_ENUMERATOR, None, CLSCTX_ALL)
                .map_err(|e| format!("CoCreateInstance: {}", e))?;
        let data_flow: EDataFlow = mem::transmute(0i32);
        let role: ERole = mem::transmute(0i32);
        let device = enumerator
            .GetDefaultAudioEndpoint(data_flow, role)
            .map_err(|e| format!("GetDefaultAudioEndpoint: {}", e))?;
        let volume: IAudioEndpointVolume = device
            .Activate(CLSCTX_ALL, None)
            .map_err(|e| format!("Activate: {}", e))?;
        let level = volume
            .GetMasterVolumeLevelScalar()
            .map_err(|e| format!("GetMasterVolumeLevelScalar: {}", e))?;
        Ok(level)
    }
}

#[cfg(windows)]
fn set_system_volume_impl(level: f32) -> Result<(), String> {
    use std::mem;
    use std::ptr;
    use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
    use windows::Win32::Media::Audio::{EDataFlow, ERole};
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, COINIT_APARTMENTTHREADED, CLSCTX_ALL,
    };

    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok();
        let enumerator: windows::Win32::Media::Audio::IMMDeviceEnumerator =
            CoCreateInstance(&CLSID_MMDEVICE_ENUMERATOR, None, CLSCTX_ALL)
                .map_err(|e| format!("CoCreateInstance: {}", e))?;
        let data_flow: EDataFlow = mem::transmute(0i32);
        let role: ERole = mem::transmute(0i32);
        let device = enumerator
            .GetDefaultAudioEndpoint(data_flow, role)
            .map_err(|e| format!("GetDefaultAudioEndpoint: {}", e))?;
        let volume: IAudioEndpointVolume = device
            .Activate(CLSCTX_ALL, None)
            .map_err(|e| format!("Activate: {}", e))?;
        volume
            .SetMasterVolumeLevelScalar(level, ptr::null())
            .map_err(|e| format!("SetMasterVolumeLevelScalar: {}", e))?;
        Ok(())
    }
}

#[cfg(windows)]
fn set_microphone_volume_impl(level: f32) -> Result<(), String> {
    use std::mem;
    use std::ptr;
    use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
    use windows::Win32::Media::Audio::{EDataFlow, ERole};
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, COINIT_APARTMENTTHREADED, CLSCTX_ALL,
    };

    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok();
        let enumerator: windows::Win32::Media::Audio::IMMDeviceEnumerator =
            CoCreateInstance(&CLSID_MMDEVICE_ENUMERATOR, None, CLSCTX_ALL)
                .map_err(|e| format!("CoCreateInstance: {}", e))?;
        // 1 = eCapture (domyślne urządzenie mikrofonu).
        let data_flow: EDataFlow = mem::transmute(1i32);
        let role: ERole = mem::transmute(0i32);
        let device = enumerator
            .GetDefaultAudioEndpoint(data_flow, role)
            .map_err(|e| format!("GetDefaultAudioEndpoint (mic): {}", e))?;
        let volume: IAudioEndpointVolume = device
            .Activate(CLSCTX_ALL, None)
            .map_err(|e| format!("Activate (mic): {}", e))?;
        volume
            .SetMasterVolumeLevelScalar(level, ptr::null())
            .map_err(|e| format!("SetMasterVolumeLevelScalar (mic): {}", e))?;
        Ok(())
    }
}

#[cfg(windows)]
fn get_audio_sessions_impl() -> Result<Vec<AudioSessionInfo>, String> {
    use std::mem;
    use windows::core::Interface;
    use windows::Win32::Media::Audio::{AudioSessionStateActive, EDataFlow, ERole};
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, COINIT_APARTMENTTHREADED, CLSCTX_ALL,
    };

    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok();
        let enumerator: windows::Win32::Media::Audio::IMMDeviceEnumerator =
            CoCreateInstance(&CLSID_MMDEVICE_ENUMERATOR, None, CLSCTX_ALL)
                .map_err(|e| format!("CoCreateInstance: {}", e))?;
        let data_flow: EDataFlow = mem::transmute(0i32);
        let role: ERole = mem::transmute(0i32);
        let device = enumerator
            .GetDefaultAudioEndpoint(data_flow, role)
            .map_err(|e| format!("GetDefaultAudioEndpoint: {}", e))?;
        let session_manager: windows::Win32::Media::Audio::IAudioSessionManager2 = device
            .Activate(CLSCTX_ALL, None)
            .map_err(|e| format!("Activate SessionManager: {}", e))?;
        let enumerator_sessions = session_manager
            .GetSessionEnumerator()
            .map_err(|e| format!("GetSessionEnumerator: {}", e))?;
        let count = enumerator_sessions
            .GetCount()
            .map_err(|e| format!("GetCount: {}", e))?;
        let mut out = Vec::new();
        for idx in 0..count {
            let session: windows::Win32::Media::Audio::IAudioSessionControl = enumerator_sessions
                .GetSession(idx)
                .map_err(|e| format!("GetSession: {}", e))?;
            let session2: windows::Win32::Media::Audio::IAudioSessionControl2 = session
                .cast()
                .map_err(|e| format!("IAudioSessionControl2: {}", e))?;
            let pid = session2.GetProcessId().unwrap_or(0);
            let display_name = session
                .GetDisplayName()
                .ok()
                .and_then(|pwstr| pwstr.to_string().ok())
                .unwrap_or_else(String::new);
            let display_name = display_name.trim().to_string();
            let process_name = get_process_path_impl(pid)
                .and_then(|p| {
                    std::path::Path::new(&p)
                        .file_stem()
                        .map(|s| s.to_string_lossy().to_string())
                })
                .unwrap_or_default();
            let resolved_name = if !process_name.trim().is_empty() {
                process_name
            } else if !display_name.is_empty() {
                display_name
            } else {
                format!("PID {}", pid)
            };
            let is_active = session
                .GetState()
                .map(|s| s == AudioSessionStateActive)
                .unwrap_or(false);
            let is_game = pid != 0
                && get_process_path_impl(pid)
                    .as_deref()
                    .map_or(false, is_game_path);
            if pid != 0 {
                out.push(AudioSessionInfo {
                    pid,
                    name: resolved_name,
                    is_active,
                    is_game,
                });
            }
        }
        Ok(out)
    }
}

#[cfg(windows)]
fn set_session_volume_impl(pid: u32, level: f32) -> Result<(), String> {
    use std::mem;
    use windows::core::Interface;
    use windows::Win32::Media::Audio::{EDataFlow, ERole};
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, COINIT_APARTMENTTHREADED, CLSCTX_ALL,
    };

    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok();
        let enumerator: windows::Win32::Media::Audio::IMMDeviceEnumerator =
            CoCreateInstance(&CLSID_MMDEVICE_ENUMERATOR, None, CLSCTX_ALL)
                .map_err(|e| format!("CoCreateInstance: {}", e))?;
        let data_flow: EDataFlow = mem::transmute(0i32);
        let role: ERole = mem::transmute(0i32);
        let device = enumerator
            .GetDefaultAudioEndpoint(data_flow, role)
            .map_err(|e| format!("GetDefaultAudioEndpoint: {}", e))?;
        let session_manager: windows::Win32::Media::Audio::IAudioSessionManager2 = device
            .Activate(CLSCTX_ALL, None)
            .map_err(|e| format!("Activate: {}", e))?;
        let enumerator_sessions = session_manager
            .GetSessionEnumerator()
            .map_err(|e| format!("GetSessionEnumerator: {}", e))?;
        let count = enumerator_sessions
            .GetCount()
            .map_err(|e| format!("GetCount: {}", e))?;
        for idx in 0..count {
            let session: windows::Win32::Media::Audio::IAudioSessionControl = enumerator_sessions
                .GetSession(idx)
                .map_err(|e| format!("GetSession: {}", e))?;
            let session2: windows::Win32::Media::Audio::IAudioSessionControl2 = session
                .cast()
                .map_err(|_| format!("cast"))?;
            if session2.GetProcessId().unwrap_or(0) == pid {
                let simple: windows::Win32::Media::Audio::ISimpleAudioVolume = session
                    .cast()
                    .map_err(|e| format!("ISimpleAudioVolume: {}", e))?;
                simple
                    .SetMasterVolume(level, std::ptr::null())
                    .map_err(|e| format!("SetMasterVolume: {}", e))?;
                return Ok(());
            }
        }
        Err(format!("Sesja dla PID {} nie znaleziona", pid))
    }
}

#[cfg(windows)]
fn set_session_volume_by_name_impl(app_name: &str, level: f32) -> Result<(), String> {
    use std::mem;
    use windows::core::Interface;
    use windows::Win32::Media::Audio::{EDataFlow, ERole};
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, COINIT_APARTMENTTHREADED, CLSCTX_ALL,
    };

    let wanted = normalize_app_name(app_name);
    if wanted.is_empty() {
        return Err("Pusta nazwa aplikacji".to_string());
    }

    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok();
        let enumerator: windows::Win32::Media::Audio::IMMDeviceEnumerator =
            CoCreateInstance(&CLSID_MMDEVICE_ENUMERATOR, None, CLSCTX_ALL)
                .map_err(|e| format!("CoCreateInstance: {}", e))?;
        let data_flow: EDataFlow = mem::transmute(0i32);
        let role: ERole = mem::transmute(0i32);
        let device = enumerator
            .GetDefaultAudioEndpoint(data_flow, role)
            .map_err(|e| format!("GetDefaultAudioEndpoint: {}", e))?;
        let session_manager: windows::Win32::Media::Audio::IAudioSessionManager2 = device
            .Activate(CLSCTX_ALL, None)
            .map_err(|e| format!("Activate: {}", e))?;
        let enumerator_sessions = session_manager
            .GetSessionEnumerator()
            .map_err(|e| format!("GetSessionEnumerator: {}", e))?;
        let count = enumerator_sessions
            .GetCount()
            .map_err(|e| format!("GetCount: {}", e))?;

        let mut matched = false;
        for idx in 0..count {
            let session: windows::Win32::Media::Audio::IAudioSessionControl = enumerator_sessions
                .GetSession(idx)
                .map_err(|e| format!("GetSession: {}", e))?;
            let session2: windows::Win32::Media::Audio::IAudioSessionControl2 = session
                .cast()
                .map_err(|_| "cast".to_string())?;
            let pid = session2.GetProcessId().unwrap_or(0);
            if pid == 0 {
                continue;
            }

            let display_name = session
                .GetDisplayName()
                .ok()
                .and_then(|pwstr| pwstr.to_string().ok())
                .unwrap_or_default()
                .trim()
                .to_string();
            let process_name = get_process_path_impl(pid)
                .and_then(|p| {
                    std::path::Path::new(&p)
                        .file_stem()
                        .map(|s| s.to_string_lossy().to_string())
                })
                .unwrap_or_default();
            let session_name = if !process_name.is_empty() {
                process_name
            } else {
                display_name
            };

            if normalize_app_name(&session_name) == wanted {
                let simple: windows::Win32::Media::Audio::ISimpleAudioVolume = session
                    .cast()
                    .map_err(|e| format!("ISimpleAudioVolume: {}", e))?;
                simple
                    .SetMasterVolume(level, std::ptr::null())
                    .map_err(|e| format!("SetMasterVolume: {}", e))?;
                matched = true;
            }
        }

        if matched {
            Ok(())
        } else {
            Err(format!("Sesja dla aplikacji '{}' nie znaleziona", app_name))
        }
    }
}

#[cfg(target_os = "linux")]
fn get_game_pids_impl() -> Result<Vec<u32>, String> {
    audio_linux::get_game_pids_impl()
}

#[cfg(target_os = "linux")]
fn get_audio_sessions_impl() -> Result<Vec<AudioSessionInfo>, String> {
    audio_linux::get_audio_sessions_impl()
}

#[cfg(target_os = "linux")]
fn get_system_volume_impl() -> Result<f32, String> {
    audio_linux::get_system_volume_impl()
}

#[cfg(target_os = "linux")]
fn set_system_volume_impl(level: f32) -> Result<(), String> {
    audio_linux::set_system_volume_impl(level)
}

#[cfg(target_os = "linux")]
fn set_microphone_volume_impl(level: f32) -> Result<(), String> {
    audio_linux::set_microphone_volume_impl(level)
}

#[cfg(target_os = "linux")]
fn set_session_volume_impl(pid: u32, level: f32) -> Result<(), String> {
    audio_linux::set_session_volume_impl(pid, level)
}

#[cfg(target_os = "linux")]
fn set_session_volume_by_name_impl(app_name: &str, level: f32) -> Result<(), String> {
    audio_linux::set_session_volume_by_name_impl(app_name, level)
}

#[cfg(not(any(windows, target_os = "linux")))]
fn get_game_pids_impl() -> Result<Vec<u32>, String> {
    Ok(vec![])
}

#[cfg(not(any(windows, target_os = "linux")))]
fn get_audio_sessions_impl() -> Result<Vec<AudioSessionInfo>, String> {
    Err("Audio nieobsługiwane na tej platformie".to_string())
}

#[cfg(not(any(windows, target_os = "linux")))]
fn set_session_volume_impl(_pid: u32, _level: f32) -> Result<(), String> {
    Err("Audio nieobsługiwane na tej platformie".to_string())
}

#[cfg(not(any(windows, target_os = "linux")))]
fn get_system_volume_impl() -> Result<f32, String> {
    Err("Audio nieobsługiwane na tej platformie".to_string())
}

#[cfg(not(any(windows, target_os = "linux")))]
fn set_system_volume_impl(_level: f32) -> Result<(), String> {
    Err("Audio nieobsługiwane na tej platformie".to_string())
}

#[cfg(not(any(windows, target_os = "linux")))]
fn set_microphone_volume_impl(_level: f32) -> Result<(), String> {
    Err("Audio nieobsługiwane na tej platformie".to_string())
}

#[cfg(not(any(windows, target_os = "linux")))]
fn set_session_volume_by_name_impl(_app_name: &str, _level: f32) -> Result<(), String> {
    Err("Audio nieobsługiwane na tej platformie".to_string())
}
