//! Raportowanie błędów / crashy → PocketBase `ideiControl` (bez Sentry).
//!
//! POST `/api/collections/ideiControl/records` z nagłówkiem `X-App-Token`.
//! Wysyłka tylko gdy użytkownik wyraził zgodę (`error_reporting_consent == Some(true)`).
//!
//! URL i token: wyłącznie ze zmiennych środowiskowych (build-time przez `build.rs`
//! lub runtime z `.env`). Bez sekretów w źródłach — brak konfiguracji = brak wysyłki.

use crate::config::{load_config, save_config};
use serde_json::json;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Once;
use tauri::AppHandle;

static PANIC_HOOK: Once = Once::new();
/// Cache zgody — `true` tylko przy `error_reporting_consent == Some(true)`.
static CONSENT_GRANTED: AtomicBool = AtomicBool::new(false);

fn env_lookup(runtime_key: &str, compile_key: Option<&str>) -> Option<String> {
    std::env::var(runtime_key)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| {
            compile_key
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
        })
}

fn report_url() -> Option<String> {
    env_lookup(
        "IDEI_ERROR_REPORT_URL",
        option_env!("IDEI_ERROR_REPORT_URL"),
    )
}

fn app_token() -> Option<String> {
    env_lookup(
        "IDEI_ERROR_REPORT_TOKEN",
        option_env!("IDEI_ERROR_REPORT_TOKEN"),
    )
}

fn os_label() -> String {
    format!("{} ({})", std::env::consts::OS, std::env::consts::ARCH)
}

fn app_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &s[..end])
}

pub fn consent_granted() -> bool {
    CONSENT_GRANTED.load(Ordering::SeqCst)
}

/// Synchronizuje cache z pliku konfiguracyjnego (start aplikacji).
pub fn sync_consent_from_config(app: &AppHandle) {
    let cfg = load_config(app);
    CONSENT_GRANTED.store(cfg.error_reporting_consent == Some(true), Ordering::SeqCst);
}

fn post_report(message: String, details: String) {
    let (Some(url), Some(token)) = (report_url(), app_token()) else {
        return;
    };

    let app_version = app_version();
    let os = os_label();

    std::thread::spawn(move || {
        let payload = json!({
            "app_version": app_version,
            "os": os,
            "message": message,
            "details": details,
        });
        let _ = ureq::post(&url)
            .set("Content-Type", "application/json")
            .set("X-App-Token", &token)
            .set("User-Agent", "IDEI-Control/error-report")
            .timeout(std::time::Duration::from_secs(8))
            .send_json(payload);
    });
}

/// Wysyłka w osobnym wątku — tylko przy aktywnej zgodzie i skonfigurowanym endpointcie.
pub fn report_error(message: impl Into<String>, details: impl Into<String>) {
    if !consent_granted() {
        return;
    }
    let message = truncate(&message.into(), 500);
    let details = truncate(&details.into(), 12_000);
    if message.trim().is_empty() {
        return;
    }
    post_report(message, details);
}

fn send_registration_test() {
    post_report(
        "Testowe połączenie / Rejestracja klienta".to_string(),
        "Użytkownik wyraził zgodę na raportowanie błędów. Moduł działa poprawnie.".to_string(),
    );
}

/// Hook paniki Rust — raportuje crash przed abortem procesu (tylko ze zgodą).
pub fn install_panic_hook() {
    PANIC_HOOK.call_once(|| {
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if consent_granted() {
                let location = info
                    .location()
                    .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
                    .unwrap_or_else(|| "unknown".to_string());
                let payload = if let Some(s) = info.payload().downcast_ref::<&str>() {
                    (*s).to_string()
                } else if let Some(s) = info.payload().downcast_ref::<String>() {
                    s.clone()
                } else {
                    "non-string panic payload".to_string()
                };
                report_error(
                    format!("panic: {payload}"),
                    format!("location={location}\npayload={payload}"),
                );
                std::thread::sleep(std::time::Duration::from_millis(400));
            }
            prev(info);
        }));
    });
}

pub fn load_dotenv() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".env");
    let _ = dotenvy::from_path(path);
}

#[tauri::command]
pub fn get_error_reporting_consent(app: AppHandle) -> Option<bool> {
    load_config(&app).error_reporting_consent
}

#[tauri::command]
pub fn set_error_reporting_consent(app: AppHandle, enabled: bool) -> Result<(), String> {
    let mut cfg = load_config(&app);
    let previously_enabled = cfg.error_reporting_consent == Some(true);
    cfg.error_reporting_consent = Some(enabled);
    save_config(&app, &cfg)?;
    CONSENT_GRANTED.store(enabled, Ordering::SeqCst);

    // Pierwsze włączenie (z None/false → true) → strzał rejestracyjny.
    if enabled && !previously_enabled {
        send_registration_test();
    }
    Ok(())
}

#[tauri::command]
pub fn report_frontend_error(
    _app: AppHandle,
    message: String,
    details: Option<String>,
) -> Result<(), String> {
    report_error(message, details.unwrap_or_default());
    Ok(())
}
