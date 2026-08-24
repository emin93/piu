use piu_lib::database::{CURRENT_SCHEMA_VERSION, Database, Migration};
use tempfile::TempDir;

#[test]
fn empty_database_migrates_to_the_current_schema() {
    let app_data = TempDir::new().expect("temporary application data");
    let database_path = app_data.path().join("piu.sqlite3");

    let database = Database::open(&database_path).expect("first migration succeeds");

    assert_eq!(database.schema_version().unwrap(), CURRENT_SCHEMA_VERSION);
    assert!(database.has_table("application_metadata").unwrap());
}

#[test]
fn current_database_can_be_opened_repeatedly() {
    let app_data = TempDir::new().expect("temporary application data");
    let database_path = app_data.path().join("piu.sqlite3");

    drop(Database::open(&database_path).expect("first open succeeds"));
    let reopened = Database::open(&database_path).expect("repeat migration succeeds");

    assert_eq!(reopened.schema_version().unwrap(), CURRENT_SCHEMA_VERSION);
    assert!(reopened.has_table("application_metadata").unwrap());
}

#[test]
fn failed_migration_rolls_back_schema_and_version() {
    let app_data = TempDir::new().expect("temporary application data");
    let database_path = app_data.path().join("piu.sqlite3");
    drop(Database::open(&database_path).expect("initial migration succeeds"));
    let failing_migration = Migration::new(
        2,
        "injected failure",
        "CREATE TABLE should_rollback (id INTEGER); SELECT missing_function();",
    );

    let result = Database::open_with_migrations(&database_path, &[failing_migration]);

    assert!(result.is_err());
    let recovered = Database::open(&database_path).expect("prior schema remains valid");
    assert_eq!(recovered.schema_version().unwrap(), CURRENT_SCHEMA_VERSION);
    assert!(!recovered.has_table("should_rollback").unwrap());
}
