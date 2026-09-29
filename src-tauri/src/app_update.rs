//! Aktualizacje aplikacji desktopowej (Tauri Updater + GitHub Releases JSON).

use serde::Serialize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_updater::UpdaterExt;

const TOTAL_UNKNOWN: u64 = u64::MAX;

#[derive(Clone, Serialize)]
pub struct AppUpdateInfo {
    pub available: bool,
    pub current_version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
}

#[derive(Clone, Serialize)]
pub struct UpdateDownloadProgress {
    pub downloaded: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<u64>,
    pub percent: Option<f32>,
    pub phase: String,
}

pub struct PendingAppUpdate(pub Mutex<Option<tauri_plugin_updater::Update>>);

fn emit_progress(app: &AppHandle, phase: &str, downloaded: u64, total: Option<u64>) {
    let percent = total.filter(|t| *t > 0).map(|t| {
        ((downloaded as f64 / t as f64) * 100.0).min(100.0) as f32
    });
    let _ = app.emit(
        "update-download-progress",
        UpdateDownloadProgress {
            downloaded,
            total,
            percent,
            phase: phase.to_string(),
        },
    );
}

#[tauri::command]
pub fn get_app_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

#[tauri::command]
pub async fn check_app_update(
    app: AppHandle,
    pending: State<'_, PendingAppUpdate>,
) -> Result<AppUpdateInfo, String> {
    let current_version = app.package_info().version.to_string();

    #[cfg(any(target_os = "macos", windows, target_os = "linux"))]
    {
        let updater = app.updater().map_err(|e| e.to_string())?;
        match updater.check().await.map_err(|e| e.to_string())? {
            Some(update) => {
                let info = AppUpdateInfo {
                    available: true,
                    current_version,
                    latest_version: Some(update.version.clone()),
                    body: update.body.clone(),
                };
                *pending.0.lock().map_err(|e| e.to_string())? = Some(update);
                return Ok(info);
            }
            None => {
                *pending.0.lock().map_err(|e| e.to_string())? = None;
                return Ok(AppUpdateInfo {
                    available: false,
                    current_version,
                    latest_version: None,
                    body: None,
                });
            }
        }
    }

    #[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
    {
        let _ = (app, pending);
        Ok(AppUpdateInfo {
            available: false,
            current_version,
            latest_version: None,
            body: None,
        })
    }
}

#[tauri::command]
pub async fn install_app_update(
    app: AppHandle,
    pending: State<'_, PendingAppUpdate>,
) -> Result<(), String> {
    #[cfg(any(target_os = "macos", windows, target_os = "linux"))]
    {
        let update = pending
            .0
            .lock()
            .map_err(|e| e.to_string())?
            .take()
            .ok_or_else(|| {
                "Brak oczekującej aktualizacji — najpierw sprawdź aktualizacje.".to_string()
            })?;

        let app_emit = app.clone();
        let downloaded = Arc::new(AtomicU64::new(0));
        let total_bytes = Arc::new(AtomicU64::new(TOTAL_UNKNOWN));

        emit_progress(&app_emit, "started", 0, None);

        let dl = Arc::clone(&downloaded);
        let tot = Arc::clone(&total_bytes);
        let app_dl = app_emit.clone();
        let app_done = app_emit.clone();

        let bytes = update
            .download(
                move |chunk_len, content_length| {
                    if tot.load(Ordering::Relaxed) == TOTAL_UNKNOWN {
                        if let Some(n) = content_length {
                            tot.store(n as u64, Ordering::Relaxed);
                        }
                    }
                    let d = dl.fetch_add(chunk_len as u64, Ordering::Relaxed) + chunk_len as u64;
                    let total = match tot.load(Ordering::Relaxed) {
                        TOTAL_UNKNOWN => None,
                        t => Some(t),
                    };
                    emit_progress(&app_dl, "progress", d, total);
                },
                move || {
                    let d = downloaded.load(Ordering::Relaxed);
                    let total = match total_bytes.load(Ordering::Relaxed) {
                        TOTAL_UNKNOWN => None,
                        t => Some(t),
                    };
                    emit_progress(&app_done, "finished", d, total);
                },
            )
            .await
            .map_err(|e| e.to_string())?;

        update.install(&bytes).map_err(|e| e.to_string())?;
        app.restart();
        #[allow(unreachable_code)]
        Ok(())
    }

    #[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
    {
        let _ = (app, pending);
        Err("Updater niedostępny na tej platformie".to_string())
    }
}
