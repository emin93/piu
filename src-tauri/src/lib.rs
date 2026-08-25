pub mod agent_environment;
pub mod agent_environment_commands;
pub mod application;
pub mod attachment_commands;
pub mod chat_runtime_commands;
pub mod chat_runtime_host;
pub mod chat_workspaces;
pub mod codex_auth;
pub mod codex_auth_boundary;
pub mod database;
pub mod git_process;
pub mod host_boundary;
pub mod model_asset_boundary;
pub mod model_assets;
mod owned_process;
pub mod pi_rpc;
pub mod project_commands;
pub mod project_inbox;
pub mod prompt_attachments;
pub mod runtime_lifecycle;
pub mod runtime_preferences;
pub mod system_appearance;

const TEST_APP_DATA_DIR_ENV: &str = "PIU_TEST_APP_DATA_DIR";

pub fn configure_builder<R: tauri::Runtime>(builder: tauri::Builder<R>) -> tauri::Builder<R> {
    builder
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            host_boundary::host_round_trip,
            codex_auth_boundary::codex_auth_status,
            codex_auth_boundary::start_codex_sign_in,
            codex_auth_boundary::answer_codex_auth_prompt,
            codex_auth_boundary::cancel_codex_sign_in,
            model_asset_boundary::model_asset_status,
            model_asset_boundary::start_model_download,
            model_asset_boundary::cancel_model_download,
            model_asset_boundary::authorize_hugging_face,
            model_asset_boundary::remove_model_assets,
            model_asset_boundary::retry_model_asset_recovery,
            attachment_commands::prepare_prompt_attachments,
            agent_environment_commands::get_project_agent_environment,
            agent_environment_commands::get_project_model_controls,
            agent_environment_commands::select_project_model_route,
            agent_environment_commands::select_project_reasoning_effort,
            agent_environment_commands::set_agent_resource_enabled,
            project_commands::load_project_inbox,
            project_commands::open_repository,
            project_commands::save_project_draft,
            project_commands::remove_project,
            project_commands::rename_chat,
            project_commands::create_chat,
            project_commands::retry_chat_setup,
            project_commands::cancel_chat_setup,
            project_commands::open_chat_terminal,
            chat_runtime_commands::open_chat_runtime,
            chat_runtime_commands::get_model_controls,
            chat_runtime_commands::select_model_route,
            chat_runtime_commands::select_reasoning_effort,
            chat_runtime_commands::send_chat_message,
            chat_runtime_commands::steer_chat,
            chat_runtime_commands::abort_chat_turn,
            chat_runtime_commands::answer_conversation_input,
            chat_runtime_commands::stop_chat_runtime,
            runtime_lifecycle::has_active_agent_turn,
            runtime_lifecycle::shutdown_runtime_processes,
            runtime_lifecycle::exit_application,
            system_appearance::system_appearance,
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
            let real_home = env::var_os("HOME")
                .map(PathBuf::from)
                .ok_or("Più requires the user's HOME directory")?;
            let git = git_process::GitProcess::from_bundled_runtime(&resource_dir.join("git"));
            let database_path = app_data.join("piu.sqlite3");
            let core = application::ApplicationCore::deferred(database_path.clone(), git);
            let agent_environment = agent_environment::AgentEnvironment::production(
                core.project_inbox(),
                &database_path,
                &app_data,
                &resource_dir,
                &real_home,
            )?;
            let chat_runtime = chat_runtime_host::ChatRuntimeHost::production(
                core.project_inbox(),
                core.chat_workspaces(),
                &app_data,
                &resource_dir,
            )?;
            chat_runtime_commands::forward_chat_runtime_events(app.handle().clone(), &chat_runtime);
            app.manage(core);
            app.manage(agent_environment);
            app.manage(chat_runtime);
            let model_assets =
                model_assets::ModelAssetManager::production_or_unavailable(&app_data);
            model_asset_boundary::forward_status_events(app.handle().clone(), &model_assets);
            app.manage(model_assets);
            let codex_auth = codex_auth::CodexAuthManager::from_bundled_runtime(
                &resource_dir,
                &app_data,
                &real_home,
            )?;
            codex_auth_boundary::forward_updates(app.handle().clone(), &codex_auth);
            app.manage(codex_auth);
            tracing::info!("application core configured");
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("Più application failed");
}
