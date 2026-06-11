//! Telemetria → Umami `POST /api/send` (tylko z Rusta, bez klucza w bundlu frontu).
//! Uwaga: publiczny collect może zbierać też śmieci z internetu — to ograniczenie Umami, nie aplikacji.

use serde_json::{json, Map, Value};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::AppHandle;
use tauri::Manager;

const INSTALLATION_ID_FILE: &str = "telemetry_installation_id.txt";

static APP_STARTED_SENT: AtomicBool = AtomicBool::new(false);

/// Bazowy URL Twojego Umami, np. `https://api.idei.cc` (bez `/api/send`).
fn telemetry_base_url() -> Option<String> {
    std::env::var("IDEI_TELEMETRY_BASE_URL")
        .ok()
        .map(|s| s.trim().trim_end_matches('/').to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| {
            option_env!("IDEI_TELEMETRY_BASE_URL")
                .map(str::trim)
                .map(|s| s.trim_end_matches('/').to_string())
                .filter(|s| !s.is_empty())
        })
}

fn telemetry_website_id() -> Option<String> {
    std::env::var("IDEI_TELEMETRY_WEBSITE_ID")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| {
            option_env!("IDEI_TELEMETRY_WEBSITE_ID")
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
        })
}

fn telemetry_hostname() -> String {
    std::env::var("IDEI_TELEMETRY_HOSTNAME")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| {
            option_env!("IDEI_TELEMETRY_HOSTNAME")
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
        })
        .unwrap_or_else(|| "idei-control.local".to_string())
}

fn installation_id_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("app_data_dir: {}", e))?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("create_dir_all: {}", e))?;
    Ok(dir.join(INSTALLATION_ID_FILE))
}

fn get_or_create_installation_id(app: &AppHandle) -> String {
    match installation_id_path(app) {
        Ok(path) => {
            if let Ok(existing) = std::fs::read_to_string(&path) {
                let t = existing.trim();
                if !t.is_empty() {
                    return t.to_string();
                }
            }
            let id = uuid::Uuid::new_v4().to_string();
            let _ = std::fs::write(&path, &id);
            id
        }
        Err(_) => "unknown-installation".to_string(),
    }
}

pub fn load_dotenv() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".env");
    let _ = dotenvy::from_path(path);
}

#[tauri::command]
pub fn send_telemetry(app: AppHandle, event_name: String, payload: Option<Value>) {
    let Some(base) = telemetry_base_url() else {
        return;
    };
    let Some(website) = telemetry_website_id() else {
        return;
    };

    if event_name == "app_started" {
        if APP_STARTED_SENT.swap(true, Ordering::SeqCst) {
            return;
        }
    }

    let send_url = format!("{}/api/send", base);
    let installation_id = get_or_create_installation_id(&app);
    let hostname = telemetry_hostname();

    let mut data_map: Map<String, Value> = match payload.unwrap_or(Value::Null) {
        Value::Object(m) => m,
        Value::Null => Map::new(),
        other => {
            let mut m = Map::new();
            m.insert("payload".to_string(), other);
            m
        }
    };
    data_map.insert(
        "installation_id".to_string(),
        Value::String(installation_id),
    );

    let body = json!({
        "type": "event",
        "payload": {
            "website": website,
            "hostname": hostname,
            "language": "en",
            "screen": "tauri",
            "title": "IDEI Control",
            "url": "app://idei-control",
            "name": event_name,
            "data": Value::Object(data_map),
        }
    });

    std::thread::spawn(move || {
        let _ = ureq::post(&send_url)
            .set("Content-Type", "application/json")
            .set("User-Agent", "IDEI-Control/telemetry")
            .send_json(body);
    });
}
