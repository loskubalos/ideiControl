mod app_update;
mod audio;
mod config;
mod error_reporting;
mod hid;
mod media_keys;

#[cfg(target_os = "linux")]
mod audio_linux;
#[cfg(target_os = "linux")]
mod media_keys_linux;

use std::sync::atomic::Ordering;

use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Manager,
};

/// Monitor uśpienia/wybudzenia. Po wybudzeniu hot-plug HID sam reconnectuje urządzenie.
#[cfg(any(windows, not(windows)))]
mod power_events {
    use tauri::AppHandle;

    pub fn start_power_monitor(_app: AppHandle) {
        // Stub: wybudzenie obsługiwane przez utratę połączenia w read_loop + hot-plug HID.
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    error_reporting::load_dotenv();
    error_reporting::install_panic_hook();
    tauri::Builder::default()
        .manage(hid::HidState::new())
        .manage(app_update::PendingAppUpdate(std::sync::Mutex::new(None)))
        .on_window_event(|window, event| {
            // Po utracie fokusu: jeśli okno jest zminimalizowane, chowamy je do tray (znika z paska zadań)
            if let tauri::WindowEvent::Focused(false) = event {
                if window.is_minimized().unwrap_or(false) {
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            hid::list_hid_devices,
            hid::get_device_paths,
            hid::scan_idei_devices,
            hid::start_scan_idei_devices,
            hid::start_scan_idei_ports,
            hid::connect_device,
            hid::connect_serial,
            hid::disconnect_device,
            hid::disconnect_serial,
            hid::get_connection_status,
            hid::get_connected_devices,
            hid::get_slider_values,
            hid::get_volume_assignments,
            hid::set_volume_assignment,
            hid::get_app_config,
            hid::save_app_config,
            hid::save_autostart_preference,
            hid::get_profiles,
            hid::set_active_profile,
            hid::save_current_profile,
            hid::create_profile,
            hid::rename_profile,
            hid::delete_profile,
            hid::notify_disconnected,
            hid::send_device_command,
            hid::set_led_color,
            hid::set_button_media_keys,
            hid::get_button_media_keys,
            hid::get_button_bindings,
            hid::set_button_bindings,
            hid::get_button_hw_slider_mute,
            hid::set_button_hw_slider_mute,
            hid::get_shortcut_mute_led_map,
            hid::set_shortcut_mute_led_map,
            hid::get_shortcut_led_mode,
            hid::set_shortcut_led_mode,
            hid::get_shortcut_led_latched,
            audio::get_system_volume,
            audio::set_system_volume,
            audio::get_audio_sessions,
            error_reporting::report_frontend_error,
            error_reporting::get_error_reporting_consent,
            error_reporting::set_error_reporting_consent,
            app_update::get_app_version,
            app_update::check_app_update,
            app_update::install_app_update,
        ])
        .setup(|app| {
            let config = config::load_config(app.handle());
            error_reporting::sync_consent_from_config(app.handle());

            #[cfg(desktop)]
            {
                let _ = app.handle().plugin(
                    tauri_plugin_updater::Builder::new()
                        .build(),
                );
            }

            if let Some(state) = app.try_state::<hid::HidState>() {
                if let Some(ref port) = config.last_port {
                    if let Ok(mut last) = state.last_connected_port.lock() {
                        *last = Some(port.clone());
                    }
                }
                state.button_media_keys.store(true, Ordering::SeqCst);
                hid::start_hid_presence_monitor(app.handle().clone(), &state);
            }

            // Zasobnik: ikona + menu (Pokaż okno, Zamknij)
            let show_i = MenuItem::with_id(app, "show", "Pokaż okno", true, None::<&str>)?;
            let quit_i = MenuItem::with_id(app, "quit", "Zamknij", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show_i, &quit_i])?;
            let mut builder = TrayIconBuilder::new().menu(&menu).show_menu_on_left_click(true);
            if let Some(icon) = app.default_window_icon() {
                builder = builder.icon(icon.clone());
            }
            let _tray = builder
                .on_menu_event(move |app, event| {
                    match event.id.as_ref() {
                        "show" => {
                            if let Some(w) = app.get_webview_window("main") {
                                let _ = w.show();
                                let _ = w.unminimize();
                                let _ = w.set_focus();
                            }
                        }
                        "quit" => {
                            app.exit(0);
                        }
                        _ => {}
                    }
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        let app = tray.app_handle();
                        if let Some(w) = app.get_webview_window("main") {
                            let _ = w.show();
                            let _ = w.unminimize();
                            let _ = w.set_focus();
                        }
                    }
                })
                .build(app)?;

            #[cfg(desktop)]
            let _ = app.handle().plugin(tauri_plugin_autostart::init(
                tauri_plugin_autostart::MacosLauncher::LaunchAgent,
                None,
            ));

            #[cfg(windows)]
            {
                let app_power = app.handle().clone();
                power_events::start_power_monitor(app_power);
            }

            #[cfg(debug_assertions)]
            if let Some(w) = app.get_webview_window("main") {
                w.open_devtools();
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
