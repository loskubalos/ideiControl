mod audio;
mod config;
mod media_keys;
mod serial;
mod telemetry;

use std::sync::atomic::Ordering;

use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Manager,
};

/// Monitor uśpienia/wybudzenia. Na Windows obsługa wybudzenia opiera się na auto-reconnect:
/// gdy port po uśpieniu przestanie odpowiadać, read_loop wyemituje device-disconnected,
/// a wątek monitorujący porty sam wykryje powrót urządzenia i zreconnectuje.
#[cfg(any(windows, not(windows)))]
mod power_events {
    use tauri::AppHandle;

    pub fn start_power_monitor(_app: AppHandle) {
        // Stub: wybudzenie obsługiwane przez utratę połączenia w read_loop + auto-reconnect.
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    telemetry::load_dotenv();
    tauri::Builder::default()
        .manage(serial::SerialState::new())
        .on_window_event(|window, event| {
            // Po utracie fokusu: jeśli okno jest zminimalizowane, chowamy je do tray (znika z paska zadań)
            if let tauri::WindowEvent::Focused(false) = event {
                if window.is_minimized().unwrap_or(false) {
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            serial::list_serial_ports,
            serial::get_port_names,
            serial::scan_idei_ports,
            serial::start_scan_idei_ports,
            serial::connect_serial,
            serial::disconnect_serial,
            serial::get_connection_status,
            serial::get_connected_devices,
            serial::get_slider_values,
            serial::get_volume_assignments,
            serial::set_volume_assignment,
            serial::get_app_config,
            serial::save_app_config,
            serial::save_autostart_preference,
            serial::notify_disconnected,
            serial::send_device_command,
            serial::set_button_media_keys,
            serial::get_button_media_keys,
            serial::get_button_bindings,
            serial::set_button_bindings,
            serial::get_button_hw_slider_mute,
            serial::set_button_hw_slider_mute,
            serial::get_shortcut_mute_led_map,
            serial::set_shortcut_mute_led_map,
            audio::get_system_volume,
            audio::set_system_volume,
            audio::get_audio_sessions,
            telemetry::send_telemetry,
        ])
        .setup(|app| {
            let config = config::load_config(app.handle());
            if let Some(state) = app.try_state::<serial::SerialState>() {
                if let Some(ref port) = config.last_port {
                    if let Ok(mut last) = state.last_connected_port.lock() {
                        *last = Some(port.clone());
                    }
                }
                state.button_media_keys.store(true, Ordering::SeqCst);
                serial::start_port_presence_monitor(app.handle().clone(), &state);
                // Nie wstrzykujemy przypisań ani map przycisków z globalnego configu
                // do domyślnego stanu. Każde urządzenie startuje „na pusto”, a użytkownik
                // świadomie ładuje preset lub konfiguruje suwak/przycisk samodzielnie.
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

            // Uruchom monitor uśpienia/wybudzenia (Windows)
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
