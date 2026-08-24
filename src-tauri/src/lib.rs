pub mod application;
pub mod chat_workspaces;
pub mod database;
pub mod git_process;
pub mod host_boundary;
pub mod model_asset_boundary;
pub mod model_assets;
pub mod project_commands;
pub mod project_inbox;

const TEST_APP_DATA_DIR_ENV: &str = "PIU_TEST_APP_DATA_DIR";

pub fn configure_builder<R: tauri::Runtime>(builder: tauri::Builder<R>) -> tauri::Builder<R> {
    builder
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            host_boundary::host_round_trip,
            model_asset_boundary::model_asset_status,
            model_asset_boundary::start_model_download,
            model_asset_boundary::cancel_model_download,
            model_asset_boundary::authorize_hugging_face,
            model_asset_boundary::remove_model_assets,
            model_asset_boundary::retry_model_asset_recovery,
            project_commands::load_project_inbox,
            project_commands::open_repository,
            project_commands::save_project_draft,
            project_commands::remove_project,
            project_commands::create_chat,
            project_commands::retry_chat_setup,
            project_commands::cancel_chat_setup,
            project_commands::open_chat_terminal,
        ])
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
            let resource_dir = app.path().resource_dir()?;
            let app_data = env::var_os(TEST_APP_DATA_DIR_ENV)
                .map(PathBuf::from)
                .unwrap_or(default_app_data);
            let git = git_process::GitProcess::from_bundled_runtime(&resource_dir.join("git"));
            let core = application::ApplicationCore::deferred(app_data.join("piu.sqlite3"), git);
            app.manage(core);
            let model_assets =
                model_assets::ModelAssetManager::production_or_unavailable(&app_data);
            model_asset_boundary::forward_status_events(app.handle().clone(), &model_assets);
            app.manage(model_assets);
            tracing::info!("application core configured");
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("Più application failed");
}
