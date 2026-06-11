// Preclude for Tauri 2
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    idei_control::run()
}
