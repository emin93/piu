use piu_lib::database::Database;
use tempfile::TempDir;

#[test]
fn empty_database_is_initialized_with_the_current_schema() {
    let app_data = TempDir::new().expect("temporary application data");
    let database_path = app_data.path().join("piu.sqlite3");

    let database = Database::open(&database_path).expect("initialization succeeds");

    assert!(database.has_table("projects").unwrap());
    assert!(database.has_table("chat_drafts").unwrap());
    assert!(database.has_table("chats").unwrap());
    assert!(database.has_table("chat_messages").unwrap());
    assert!(database.has_table("chat_workspace_creations").unwrap());
}

#[test]
fn current_database_can_be_opened_repeatedly() {
    let app_data = TempDir::new().expect("temporary application data");
    let database_path = app_data.path().join("piu.sqlite3");

    drop(Database::open(&database_path).expect("first open succeeds"));
    let reopened = Database::open(&database_path).expect("second open succeeds");

    assert!(reopened.has_table("projects").unwrap());
    assert!(reopened.has_table("chat_drafts").unwrap());
    assert!(reopened.has_table("chats").unwrap());
    assert!(reopened.has_table("chat_messages").unwrap());
    assert!(reopened.has_table("chat_workspace_creations").unwrap());
}
