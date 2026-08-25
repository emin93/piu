use piu_lib::database::Database;
use rusqlite::Connection;
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
    assert!(database.has_table("runtime_model_selection").unwrap());
    assert!(database.has_table("model_route_efforts").unwrap());
    assert!(
        database
            .has_table("global_resource_enable_overrides")
            .unwrap()
    );
    assert!(
        database
            .has_table("project_resource_enable_overrides")
            .unwrap()
    );
    assert!(!database.has_table("schema_migrations").unwrap());
    drop(database);

    let connection = Connection::open(&database_path).expect("current database should reopen");
    let mut columns = connection
        .prepare("SELECT name FROM pragma_table_info('chats')")
        .expect("chat columns should be readable");
    let columns = columns
        .query_map([], |row| row.get::<_, String>(0))
        .expect("chat columns should query")
        .collect::<Result<Vec<_>, _>>()
        .expect("chat columns should decode");
    assert!(columns.iter().any(|column| column == "pi_session_id"));
    assert!(columns.iter().any(|column| column == "pi_session_path"));
    assert!(
        columns
            .iter()
            .any(|column| column == "initial_model_provider")
    );
    assert!(columns.iter().any(|column| column == "initial_model_id"));
    assert!(
        columns
            .iter()
            .any(|column| column == "initial_reasoning_effort")
    );
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
    assert!(reopened.has_table("runtime_model_selection").unwrap());
    assert!(reopened.has_table("model_route_efforts").unwrap());
    assert!(
        reopened
            .has_table("global_resource_enable_overrides")
            .unwrap()
    );
    assert!(
        reopened
            .has_table("project_resource_enable_overrides")
            .unwrap()
    );
}
