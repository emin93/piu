use std::fs;

use piu_lib::{application::ApplicationCore, git_process::GitProcess};

#[test]
fn failed_application_core_startup_can_be_retried() {
    let app_data = tempfile::tempdir().expect("temporary application data");
    let blocked_directory = app_data.path().join("blocked");
    fs::write(&blocked_directory, "not a directory").unwrap();
    let core = ApplicationCore::deferred(
        blocked_directory.join("piu.sqlite3"),
        GitProcess::with_executable("/usr/bin/git".into()),
    );

    assert!(core.ensure_storage_ready().is_err());

    fs::remove_file(&blocked_directory).unwrap();
    fs::create_dir(&blocked_directory).unwrap();
    core.ensure_storage_ready().unwrap();
}
