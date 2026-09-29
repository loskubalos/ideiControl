fn main() {
    // Opcjonalny lokalny `.env` przy kompilacji (nie commituj).
    let env_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(".env");
    let _ = dotenvy::from_path(&env_path);

    // Wstrzykuj sekrety PocketBase jako cargo env (CI Secrets / lokalny .env — nigdy w źródłach).
    for key in ["IDEI_ERROR_REPORT_URL", "IDEI_ERROR_REPORT_TOKEN"] {
        if let Ok(val) = std::env::var(key) {
            let trimmed = val.trim();
            if !trimmed.is_empty() {
                println!("cargo:rustc-env={key}={trimmed}");
            }
        }
        println!("cargo:rerun-if-env-changed={key}");
    }

    tauri_build::build()
}
