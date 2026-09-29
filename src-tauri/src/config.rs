//! Zapis i odczyt konfiguracji aplikacji (assignments, ostatnie urządzenie HID, profile, ustawienia).

use crate::audio::VolumeTarget;
use crate::media_keys::{bindings_from_legacy_media, ButtonBinding, MediaKeyAction, MEDIA_BUTTON_SLOTS};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

const CONFIG_FILENAME: &str = "config.json";
pub const DEFAULT_PROFILE_NAME: &str = "Default";

/// Zachowanie LED dla przycisku Media / Shortcut (gdy nie ma mute kanału).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShortcutLedMode {
    /// Brak LED.
    #[default]
    Off,
    /// Świeci tylko podczas trzymania przycisku.
    Momentary,
    /// Toggle ON/OFF przy każdym wciśnięciu (np. wskaźnik mute Discord).
    Toggle,
}

impl ShortcutLedMode {
    pub fn from_legacy_bool(on: bool) -> Self {
        if on {
            Self::Momentary
        } else {
            Self::Off
        }
    }
}

pub fn default_shortcut_led_modes() -> [ShortcutLedMode; MEDIA_BUTTON_SLOTS] {
    [ShortcutLedMode::Off; MEDIA_BUTTON_SLOTS]
}

fn empty_volume_assignments() -> Vec<Vec<VolumeTarget>> {
    (0..crate::audio::MAX_SLIDERS).map(|_| vec![]).collect()
}

fn default_hw_mute_vec() -> Vec<bool> {
    vec![false; MEDIA_BUTTON_SLOTS]
}

fn default_bindings_vec() -> Vec<ButtonBinding> {
    vec![ButtonBinding::None; MEDIA_BUTTON_SLOTS]
}

fn default_led_modes_vec() -> Vec<ShortcutLedMode> {
    default_shortcut_led_modes().to_vec()
}

/// Snapshot danych profilu (bez id/nazwy) — zapis / seed.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ProfileData {
    #[serde(default)]
    pub volume_assignments: Vec<Vec<VolumeTarget>>,
    #[serde(default)]
    pub button_bindings: Vec<ButtonBinding>,
    #[serde(default)]
    pub button_hw_slider_mute: Vec<bool>,
    #[serde(default)]
    pub shortcut_led_mode: Vec<ShortcutLedMode>,
    /// Legacy — migracja przy normalizacji.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub button_media_actions: Option<Vec<MediaKeyAction>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shortcut_mute_led_map: Option<Vec<bool>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shortcut_mute_led: Option<bool>,
}

/// Profil użytkownika: pełna mapa suwaków, przycisków i trybów LED.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Profile {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub volume_assignments: Vec<Vec<VolumeTarget>>,
    #[serde(default)]
    pub button_bindings: Vec<ButtonBinding>,
    #[serde(default)]
    pub button_hw_slider_mute: Vec<bool>,
    #[serde(default)]
    pub shortcut_led_mode: Vec<ShortcutLedMode>,
    /// Chroniony przed usunięciem (razem z regułą „ostatni profil”).
    #[serde(default)]
    pub is_default: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub button_media_actions: Option<Vec<MediaKeyAction>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shortcut_mute_led_map: Option<Vec<bool>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shortcut_mute_led: Option<bool>,
}

impl Profile {
    pub fn new_id() -> String {
        uuid::Uuid::new_v4().to_string()
    }

    pub fn from_data(name: impl Into<String>, data: ProfileData, is_default: bool) -> Self {
        let mut p = Self {
            id: Self::new_id(),
            name: name.into(),
            volume_assignments: data.volume_assignments,
            button_bindings: data.button_bindings,
            button_hw_slider_mute: data.button_hw_slider_mute,
            shortcut_led_mode: data.shortcut_led_mode,
            is_default,
            button_media_actions: data.button_media_actions,
            shortcut_mute_led_map: data.shortcut_mute_led_map,
            shortcut_mute_led: data.shortcut_mute_led,
        };
        normalize_profile(&mut p);
        p
    }

    pub fn to_data(&self) -> ProfileData {
        ProfileData {
            volume_assignments: self.volume_assignments.clone(),
            button_bindings: self.button_bindings.clone(),
            button_hw_slider_mute: self.button_hw_slider_mute.clone(),
            shortcut_led_mode: self.shortcut_led_mode.clone(),
            button_media_actions: None,
            shortcut_mute_led_map: None,
            shortcut_mute_led: None,
        }
    }

    pub fn apply_data(&mut self, data: ProfileData) {
        self.volume_assignments = data.volume_assignments;
        self.button_bindings = data.button_bindings;
        self.button_hw_slider_mute = data.button_hw_slider_mute;
        self.shortcut_led_mode = data.shortcut_led_mode;
        self.button_media_actions = data.button_media_actions;
        self.shortcut_mute_led_map = data.shortcut_mute_led_map;
        self.shortcut_mute_led = data.shortcut_mute_led;
        normalize_profile(self);
    }
}

/// Odpowiedź API profili.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProfilesState {
    pub profiles: Vec<Profile>,
    pub active_profile_id: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DeviceRuntimeConfig {
    #[serde(default)]
    pub volume_assignments: Vec<Vec<VolumeTarget>>,
    #[serde(default)]
    pub button_bindings: Vec<ButtonBinding>,
    #[serde(default)]
    pub button_hw_slider_mute: Vec<bool>,
    /// Preferowane: tryb LED per przycisk.
    #[serde(default)]
    pub shortcut_led_mode: Vec<ShortcutLedMode>,
    /// Legacy bool (true = momentary) — migracja przy load.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shortcut_mute_led_map: Vec<bool>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AppConfig {
    /// Przypisania suwaków (cel głośności dla każdego).
    #[serde(default)]
    pub volume_assignments: Option<Vec<Vec<VolumeTarget>>>,
    /// Ostatnio połączone urządzenie — ścieżka HID (historycznie `last_port` / COM).
    #[serde(default)]
    pub last_port: Option<String>,
    /// Uruchamiać aplikację przy starcie systemu.
    #[serde(default)]
    pub autostart: Option<bool>,
    /// Zgoda na anonimowe raportowanie błędów do PocketBase (`None` = jeszcze nie pytano).
    #[serde(default)]
    pub error_reporting_consent: Option<bool>,
    /// Minimalizować do zasobnika zamiast zamykać okno.
    #[serde(default)]
    pub minimize_to_tray: Option<bool>,
    /// Zdarzenia przycisków (mute rising-edge) -> klawisze mediów.
    #[serde(default)]
    pub button_media_keys: Option<bool>,
    /// Mapowanie przycisków: media lub skrót klawiszowy (Windows).
    #[serde(default)]
    pub button_bindings: Option<Vec<ButtonBinding>>,
    /// Stary format — wczytywany tylko gdy brak `button_bindings`; nie zapisujemy z powrotem.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub button_media_actions: Option<Vec<MediaKeyAction>>,
    /// Gdy true: wciśnięcie przycisku przełącza programowy mute kanału (sesje Windows + LED 0x03).
    /// Gdy false: przycisk wywołuje Media / Shortcut z `button_bindings`.
    #[serde(default)]
    pub button_hw_slider_mute: Option<Vec<bool>>,
    /// Per przycisk (tryb media/shortcut): Off / Momentary / Toggle.
    #[serde(default)]
    pub shortcut_led_mode: Option<Vec<ShortcutLedMode>>,
    /// Legacy: true = Momentary. Migracja → `shortcut_led_mode`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shortcut_mute_led_map: Option<Vec<bool>>,
    /// Stary klucz globalny — przy wczytaniu migrujemy, potem nie zapisujemy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shortcut_mute_led: Option<bool>,
    /// Konfiguracja per konkretne urządzenie (klucz = UID / model).
    #[serde(default)]
    pub device_configs: Option<HashMap<String, DeviceRuntimeConfig>>,
    /// Lista profili (presety) — pełne snapshoty mapowań.
    #[serde(default)]
    pub profiles: Option<Vec<Profile>>,
    /// ID aktywnego profilu.
    #[serde(default)]
    pub active_profile_id: Option<String>,
}

fn config_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("app_data_dir: {}", e))?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("create_dir_all: {}", e))?;
    Ok(dir.join(CONFIG_FILENAME))
}

/// Ładuje konfigurację z pliku. Zwraca domyślną, jeśli plik nie istnieje lub jest nieprawidłowy.
pub fn load_config(app: &AppHandle) -> AppConfig {
    let path = match config_path(app) {
        Ok(p) => p,
        Err(_) => return AppConfig::default(),
    };
    let bytes = match fs::read(&path) {
        Ok(b) => b,
        Err(_) => return AppConfig::default(),
    };
    let mut config: AppConfig = match serde_json::from_slice(&bytes) {
        Ok(c) => c,
        Err(_) => return AppConfig::default(),
    };
    if let Some(ref mut a) = config.volume_assignments {
        if a.len() != crate::audio::MAX_SLIDERS {
            *a = empty_volume_assignments();
        }
    }
    normalize_button_bindings(&mut config);
    normalize_hw_slider_mute(&mut config);
    normalize_shortcut_led_mode(&mut config);
    normalize_device_configs(&mut config);
    ensure_profiles(&mut config);
    config
}

fn normalize_hw_slider_mute(cfg: &mut AppConfig) {
    match &mut cfg.button_hw_slider_mute {
        None => {
            cfg.button_hw_slider_mute = Some(default_hw_mute_vec());
        }
        Some(v) if v.len() != MEDIA_BUTTON_SLOTS => {
            let mut base = default_hw_mute_vec();
            for i in 0..v.len().min(MEDIA_BUTTON_SLOTS) {
                base[i] = v[i];
            }
            *v = base;
        }
        _ => {}
    }
}

fn modes_from_legacy_bools(legacy: &[bool], global_on: bool) -> Vec<ShortcutLedMode> {
    let mut out = vec![ShortcutLedMode::Off; MEDIA_BUTTON_SLOTS];
    if legacy.is_empty() && global_on {
        return vec![ShortcutLedMode::Momentary; MEDIA_BUTTON_SLOTS];
    }
    for i in 0..legacy.len().min(MEDIA_BUTTON_SLOTS) {
        out[i] = ShortcutLedMode::from_legacy_bool(legacy[i]);
    }
    out
}

fn normalize_shortcut_led_mode(cfg: &mut AppConfig) {
    let legacy_global_on = cfg.shortcut_mute_led == Some(true);
    cfg.shortcut_mute_led = None;

    if cfg.shortcut_led_mode.as_ref().map(|v| v.len()) != Some(MEDIA_BUTTON_SLOTS) {
        let from_bool = cfg.shortcut_mute_led_map.as_deref().unwrap_or(&[]);
        let mut modes = modes_from_legacy_bools(from_bool, legacy_global_on);
        if let Some(ref existing) = cfg.shortcut_led_mode {
            for i in 0..existing.len().min(MEDIA_BUTTON_SLOTS) {
                modes[i] = existing[i];
            }
        }
        cfg.shortcut_led_mode = Some(modes);
    }
    cfg.shortcut_mute_led_map = None;
}

fn normalize_button_bindings(cfg: &mut AppConfig) {
    let mut out: Vec<ButtonBinding> = default_bindings_vec();

    if let Some(ref b) = cfg.button_bindings {
        if b.len() == MEDIA_BUTTON_SLOTS {
            for i in 0..MEDIA_BUTTON_SLOTS {
                out[i] = b[i].clone().normalized();
            }
        } else {
            for i in 0..b.len().min(MEDIA_BUTTON_SLOTS) {
                out[i] = b[i].clone().normalized();
            }
        }
    } else if let Some(ref legacy) = cfg.button_media_actions {
        if legacy.len() == MEDIA_BUTTON_SLOTS {
            let arr = bindings_from_legacy_media(legacy.as_slice());
            for i in 0..MEDIA_BUTTON_SLOTS {
                out[i] = arr[i].clone().normalized();
            }
        }
    }

    cfg.button_bindings = Some(out);
    cfg.button_media_actions = None;
}

fn normalize_device_configs(cfg: &mut AppConfig) {
    let Some(map) = cfg.device_configs.as_mut() else {
        return;
    };
    for (_k, dc) in map.iter_mut() {
        if dc.volume_assignments.len() != crate::audio::MAX_SLIDERS {
            let mut fixed = empty_volume_assignments();
            for i in 0..dc.volume_assignments.len().min(crate::audio::MAX_SLIDERS) {
                fixed[i] = dc.volume_assignments[i].clone();
            }
            dc.volume_assignments = fixed;
        }
        if dc.button_bindings.len() != MEDIA_BUTTON_SLOTS {
            let mut out = default_bindings_vec();
            for i in 0..dc.button_bindings.len().min(MEDIA_BUTTON_SLOTS) {
                out[i] = dc.button_bindings[i].clone().normalized();
            }
            dc.button_bindings = out;
        } else {
            for i in 0..MEDIA_BUTTON_SLOTS {
                dc.button_bindings[i] = dc.button_bindings[i].clone().normalized();
            }
        }
        if dc.button_hw_slider_mute.len() != MEDIA_BUTTON_SLOTS {
            let mut out = default_hw_mute_vec();
            for i in 0..dc.button_hw_slider_mute.len().min(MEDIA_BUTTON_SLOTS) {
                out[i] = dc.button_hw_slider_mute[i];
            }
            dc.button_hw_slider_mute = out;
        }
        if dc.shortcut_led_mode.len() != MEDIA_BUTTON_SLOTS {
            let mut modes = modes_from_legacy_bools(&dc.shortcut_mute_led_map, false);
            for i in 0..dc.shortcut_led_mode.len().min(MEDIA_BUTTON_SLOTS) {
                modes[i] = dc.shortcut_led_mode[i];
            }
            dc.shortcut_led_mode = modes;
        }
        dc.shortcut_mute_led_map.clear();
    }
}

fn pad_volume_assignments(src: &[Vec<VolumeTarget>]) -> Vec<Vec<VolumeTarget>> {
    let mut out = empty_volume_assignments();
    for i in 0..src.len().min(crate::audio::MAX_SLIDERS) {
        out[i] = src[i].clone();
    }
    out
}

fn pad_bindings(src: &[ButtonBinding]) -> Vec<ButtonBinding> {
    let mut out = default_bindings_vec();
    for i in 0..src.len().min(MEDIA_BUTTON_SLOTS) {
        out[i] = src[i].clone().normalized();
    }
    out
}

fn pad_bools(src: &[bool]) -> Vec<bool> {
    let mut out = default_hw_mute_vec();
    for i in 0..src.len().min(MEDIA_BUTTON_SLOTS) {
        out[i] = src[i];
    }
    out
}

fn pad_led_modes(src: &[ShortcutLedMode]) -> Vec<ShortcutLedMode> {
    let mut out = default_led_modes_vec();
    for i in 0..src.len().min(MEDIA_BUTTON_SLOTS) {
        out[i] = src[i];
    }
    out
}

fn normalize_profile(p: &mut Profile) {
    p.volume_assignments = pad_volume_assignments(&p.volume_assignments);

    if p.button_bindings.len() != MEDIA_BUTTON_SLOTS {
        if let Some(ref legacy) = p.button_media_actions {
            if legacy.len() == MEDIA_BUTTON_SLOTS {
                let arr = bindings_from_legacy_media(legacy.as_slice());
                p.button_bindings = arr.iter().cloned().map(|b| b.normalized()).collect();
            } else {
                p.button_bindings = pad_bindings(&p.button_bindings);
            }
        } else {
            p.button_bindings = pad_bindings(&p.button_bindings);
        }
    } else {
        p.button_bindings = pad_bindings(&p.button_bindings);
    }
    p.button_media_actions = None;

    p.button_hw_slider_mute = pad_bools(&p.button_hw_slider_mute);

    if p.shortcut_led_mode.len() != MEDIA_BUTTON_SLOTS {
        let legacy_global = p.shortcut_mute_led == Some(true);
        let from_bool = p.shortcut_mute_led_map.as_deref().unwrap_or(&[]);
        let mut modes = modes_from_legacy_bools(from_bool, legacy_global);
        for i in 0..p.shortcut_led_mode.len().min(MEDIA_BUTTON_SLOTS) {
            modes[i] = p.shortcut_led_mode[i];
        }
        p.shortcut_led_mode = modes;
    } else {
        p.shortcut_led_mode = pad_led_modes(&p.shortcut_led_mode);
    }
    p.shortcut_mute_led_map = None;
    p.shortcut_mute_led = None;

    let name = p.name.trim();
    if name.is_empty() {
        p.name = DEFAULT_PROFILE_NAME.to_string();
    } else {
        p.name = name.to_string();
    }
    if p.id.trim().is_empty() {
        p.id = Profile::new_id();
    }
}

fn snapshot_data_from_config(cfg: &AppConfig) -> ProfileData {
    ProfileData {
        volume_assignments: cfg
            .volume_assignments
            .clone()
            .unwrap_or_else(empty_volume_assignments),
        button_bindings: cfg
            .button_bindings
            .clone()
            .unwrap_or_else(default_bindings_vec),
        button_hw_slider_mute: cfg
            .button_hw_slider_mute
            .clone()
            .unwrap_or_else(default_hw_mute_vec),
        shortcut_led_mode: cfg
            .shortcut_led_mode
            .clone()
            .unwrap_or_else(default_led_modes_vec),
        button_media_actions: None,
        shortcut_mute_led_map: None,
        shortcut_mute_led: None,
    }
}

/// Tworzy domyślny profil z bieżącego snapshotu AppConfig (flat fields).
pub fn profile_from_config(cfg: &AppConfig, name: &str, is_default: bool) -> Profile {
    Profile::from_data(name, snapshot_data_from_config(cfg), is_default)
}

/// Gwarantuje niepustą listę profili + poprawne `active_profile_id`.
pub fn ensure_profiles(cfg: &mut AppConfig) {
    let mut profiles = cfg.profiles.take().unwrap_or_default();
    for p in profiles.iter_mut() {
        normalize_profile(p);
    }

    if profiles.is_empty() {
        profiles.push(profile_from_config(cfg, DEFAULT_PROFILE_NAME, true));
    } else if !profiles.iter().any(|p| p.is_default) {
        profiles[0].is_default = true;
    }

    let active = cfg
        .active_profile_id
        .as_ref()
        .filter(|id| profiles.iter().any(|p| &p.id == *id))
        .cloned()
        .unwrap_or_else(|| profiles[0].id.clone());

    cfg.profiles = Some(profiles);
    cfg.active_profile_id = Some(active);
}

pub fn profiles_state(cfg: &AppConfig) -> ProfilesState {
    let profiles = cfg.profiles.clone().unwrap_or_default();
    let active_profile_id = cfg
        .active_profile_id
        .clone()
        .or_else(|| profiles.first().map(|p| p.id.clone()))
        .unwrap_or_default();
    ProfilesState {
        profiles,
        active_profile_id,
    }
}

pub fn find_profile<'a>(cfg: &'a AppConfig, id: &str) -> Option<&'a Profile> {
    cfg.profiles
        .as_ref()?
        .iter()
        .find(|p| p.id == id)
}

pub fn find_profile_mut<'a>(cfg: &'a mut AppConfig, id: &str) -> Option<&'a mut Profile> {
    cfg.profiles
        .as_mut()?
        .iter_mut()
        .find(|p| p.id == id)
}

/// Kopiuje pola profilu do flat fields AppConfig (runtime / zapis).
pub fn apply_profile_to_flat_config(cfg: &mut AppConfig, profile: &Profile) {
    cfg.volume_assignments = Some(profile.volume_assignments.clone());
    cfg.button_bindings = Some(profile.button_bindings.clone());
    cfg.button_hw_slider_mute = Some(profile.button_hw_slider_mute.clone());
    cfg.shortcut_led_mode = Some(profile.shortcut_led_mode.clone());
    cfg.active_profile_id = Some(profile.id.clone());
}

/// Konwersja ze stanu na zapis do pliku.
pub fn assignments_to_vec(arr: &[Vec<VolumeTarget>; crate::audio::MAX_SLIDERS]) -> Vec<Vec<VolumeTarget>> {
    arr.iter().cloned().collect()
}

/// Zapisuje konfigurację do pliku.
pub fn save_config(app: &AppHandle, config: &AppConfig) -> Result<(), String> {
    let path = config_path(app)?;
    let mut cfg = config.clone();
    cfg.button_media_actions = None;
    cfg.shortcut_mute_led = None;
    cfg.shortcut_mute_led_map = None;
    if let Some(ref mut profiles) = cfg.profiles {
        for p in profiles.iter_mut() {
            p.button_media_actions = None;
            p.shortcut_mute_led = None;
            p.shortcut_mute_led_map = None;
        }
    }
    let json = serde_json::to_string_pretty(&cfg).map_err(|e| e.to_string())?;
    fs::write(&path, json).map_err(|e| format!("write config: {}", e))?;
    Ok(())
}
