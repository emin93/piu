pub mod application;
pub mod database;
pub mod host_boundary;

const TEST_APP_DATA_DIR_ENV: &str = "PIU_TEST_APP_DATA_DIR";

pub fn configure_builder<R: tauri::Runtime>(builder: tauri::Builder<R>) -> tauri::Builder<R> {
    builder
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![host_boundary::host_round_trip])
}

pub fn run() {
    use std::{env, path::PathBuf};

    use tauri::Manager;
    use tracing_subscriber::EnvFilter;

    let _ = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("piu=info")),
        )
        .try_init();

    configure_builder(tauri::Builder::default())
        .setup(|app| {
            let default_app_data = app.path().app_data_dir()?;
            let app_data = env::var_os(TEST_APP_DATA_DIR_ENV)
                .map(PathBuf::from)
                .unwrap_or(default_app_data);
            let core = application::ApplicationCore::deferred(app_data.join("piu.sqlite3"));
            app.manage(core);
            tracing::info!("application core configured");
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("Più application failed");
}
