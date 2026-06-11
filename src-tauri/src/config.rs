//! Zapis i odczyt konfiguracji aplikacji (assignments, ostatni port, ustawienia).

use crate::audio::VolumeTarget;
use crate::media_keys::{bindings_from_legacy_media, ButtonBinding, MediaKeyAction, MEDIA_BUTTON_SLOTS};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

const CONFIG_FILENAME: &str = "config.json";

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DeviceRuntimeConfig {
    #[serde(default)]
    pub volume_assignments: Vec<Vec<VolumeTarget>>,
    #[serde(default)]
    pub button_bindings: Vec<ButtonBinding>,
    #[serde(default)]
    pub button_hw_slider_mute: Vec<bool>,
    #[serde(default)]
    pub shortcut_mute_led_map: Vec<bool>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AppConfig {
    /// Przypisania suwaków (cel głośności dla każdego).
    #[serde(default)]
    pub volume_assignments: Option<Vec<Vec<VolumeTarget>>>,
    /// Ostatnio połączony port (np. COM3) — do prewyboru i auto-reconnect.
    #[serde(default)]
    pub last_port: Option<String>,
    /// Uruchamiać aplikację przy starcie systemu.
    #[serde(default)]
    pub autostart: Option<bool>,
    /// Minimalizować do zasobnika zamiast zamykać okno.
    #[serde(default)]
    pub minimize_to_tray: Option<bool>,
    /// Zdarzenia `btn` z urządzenia (proto>=1) -> domyślne klawisze mediów (Windows).
    #[serde(default)]
    pub button_media_keys: Option<bool>,
    /// Mapowanie przycisków: media lub skrót klawiszowy (Windows).
    #[serde(default)]
    pub button_bindings: Option<Vec<ButtonBinding>>,
    /// Stary format — wczytywany tylko gdy brak `button_bindings`; nie zapisujemy z powrotem.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub button_media_actions: Option<Vec<MediaKeyAction>>,
    /// Przycisk i: true = firmware przełącza mute suwaka i; false = tylko JSON btn + klawisze PC.
    #[serde(default)]
    pub button_hw_slider_mute: Option<Vec<bool>>,
    /// Per przycisk: Neo przy skrócie (`SET_SHORTCUT_MUTE_LED_MAP`). Domyślnie wyłączone; mute na suwaku steruje Neo jak wcześniej.
    #[serde(default)]
    pub shortcut_mute_led_map: Option<Vec<bool>>,
    /// Stary klucz globalny — przy wczytaniu migrujemy do `shortcut_mute_led_map`, potem nie zapisujemy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shortcut_mute_led: Option<bool>,
    /// Konfiguracja per konkretne urządzenie (klucz = UID urządzenia).
    #[serde(default)]
    pub device_configs: Option<HashMap<String, DeviceRuntimeConfig>>,
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
    // Normalizacja: liczba suwaków zgodna z MAX_SLIDERS
    if let Some(ref mut a) = config.volume_assignments {
        if a.len() != crate::audio::MAX_SLIDERS {
            *a = (0..crate::audio::MAX_SLIDERS)
                .map(|_| vec![])
                .collect();
        }
    }
    normalize_button_bindings(&mut config);
    normalize_hw_slider_mute(&mut config);
    normalize_shortcut_mute_led_map(&mut config);
    normalize_device_configs(&mut config);
    config
}

fn normalize_hw_slider_mute(cfg: &mut AppConfig) {
    match &mut cfg.button_hw_slider_mute {
        None => {
            cfg.button_hw_slider_mute = Some(vec![false; MEDIA_BUTTON_SLOTS]);
        }
        Some(v) if v.len() != MEDIA_BUTTON_SLOTS => {
            let mut base = vec![false; MEDIA_BUTTON_SLOTS];
            for i in 0..v.len().min(MEDIA_BUTTON_SLOTS) {
                base[i] = v[i];
            }
            *v = base;
        }
        _ => {}
    }
}

fn normalize_shortcut_mute_led_map(cfg: &mut AppConfig) {
    let legacy_global_on = cfg.shortcut_mute_led == Some(true);
    cfg.shortcut_mute_led = None;
    match &mut cfg.shortcut_mute_led_map {
        None => {
            cfg.shortcut_mute_led_map = Some(if legacy_global_on {
                vec![true; MEDIA_BUTTON_SLOTS]
            } else {
                vec![false; MEDIA_BUTTON_SLOTS]
            });
        }
        Some(v) if v.len() != MEDIA_BUTTON_SLOTS => {
            let mut base = vec![false; MEDIA_BUTTON_SLOTS];
            for i in 0..v.len().min(MEDIA_BUTTON_SLOTS) {
                base[i] = v[i];
            }
            *v = base;
        }
        _ => {}
    }
}

fn normalize_button_bindings(cfg: &mut AppConfig) {
    // Domyślnie: wszystkie przyciski = None; nie narzucamy klawiszy multimedialnych.
    let mut out: Vec<ButtonBinding> = vec![ButtonBinding::None; MEDIA_BUTTON_SLOTS];

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
            let mut fixed = (0..crate::audio::MAX_SLIDERS)
                .map(|_| vec![])
                .collect::<Vec<Vec<VolumeTarget>>>();
            for i in 0..dc.volume_assignments.len().min(crate::audio::MAX_SLIDERS) {
                fixed[i] = dc.volume_assignments[i].clone();
            }
            dc.volume_assignments = fixed;
        }
        if dc.button_bindings.len() != MEDIA_BUTTON_SLOTS {
            let mut out = vec![ButtonBinding::None; MEDIA_BUTTON_SLOTS];
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
            let mut out = vec![false; MEDIA_BUTTON_SLOTS];
            for i in 0..dc.button_hw_slider_mute.len().min(MEDIA_BUTTON_SLOTS) {
                out[i] = dc.button_hw_slider_mute[i];
            }
            dc.button_hw_slider_mute = out;
        }
        if dc.shortcut_mute_led_map.len() != MEDIA_BUTTON_SLOTS {
            let mut out = vec![false; MEDIA_BUTTON_SLOTS];
            for i in 0..dc.shortcut_mute_led_map.len().min(MEDIA_BUTTON_SLOTS) {
                out[i] = dc.shortcut_mute_led_map[i];
            }
            dc.shortcut_mute_led_map = out;
        }
    }
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
    let json = serde_json::to_string_pretty(&cfg).map_err(|e| e.to_string())?;
    fs::write(&path, json).map_err(|e| format!("write config: {}", e))?;
    Ok(())
}
