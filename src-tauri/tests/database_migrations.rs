use piu_lib::database::{CURRENT_SCHEMA_VERSION, Database, Migration};
use rusqlite::Connection;
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
fn version_two_project_data_migrates_without_being_recreated() {
    let app_data = TempDir::new().expect("temporary application data");
    let database_path = app_data.path().join("piu.sqlite3");
    let connection = Connection::open(&database_path).expect("legacy database should open");
    connection
        .execute_batch(
            r#"
            PRAGMA foreign_keys = ON;
            CREATE TABLE application_metadata (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                created_at TEXT NOT NULL
            );
            INSERT INTO application_metadata (id, created_at)
            VALUES (1, '2026-08-24T00:00:00.000Z');

            CREATE TABLE projects (
                id INTEGER PRIMARY KEY,
                canonical_path TEXT NOT NULL UNIQUE,
                name TEXT NOT NULL,
                created_at_ms INTEGER NOT NULL
            );
            CREATE TABLE chat_drafts (
                project_id INTEGER PRIMARY KEY REFERENCES projects(id) ON DELETE CASCADE,
                prompt TEXT NOT NULL,
                updated_at_ms INTEGER NOT NULL
            );
            CREATE TABLE chats (
                id TEXT PRIMARY KEY,
                project_id INTEGER REFERENCES projects(id) ON DELETE SET NULL,
                project_name TEXT NOT NULL,
                title TEXT NOT NULL,
                branch_name TEXT NOT NULL,
                pull_request_number INTEGER,
                created_at_ms INTEGER NOT NULL,
                merge_state TEXT NOT NULL CHECK (merge_state IN ('unmerged', 'merged'))
            );
            CREATE INDEX chats_created_at ON chats(created_at_ms DESC, id ASC);
            CREATE INDEX chats_project_id ON chats(project_id);

            INSERT INTO projects (id, canonical_path, name, created_at_ms)
            VALUES (7, '/tmp/legacy-project', 'legacy-project', 100);
            INSERT INTO chat_drafts (project_id, prompt, updated_at_ms)
            VALUES (7, 'Preserve this draft', 200);
            INSERT INTO chats (
                id, project_id, project_name, title, branch_name,
                pull_request_number, created_at_ms, merge_state
            ) VALUES (
                'legacy-chat', 7, 'legacy-project', 'Preserve this chat',
                'agent/legacy-chat', 42, 300, 'unmerged'
            );
            PRAGMA user_version = 2;
            "#,
        )
        .expect("legacy schema should be created");
    drop(connection);

    let migrated = Database::open(&database_path).expect("legacy database should migrate");
    assert_eq!(migrated.schema_version().unwrap(), CURRENT_SCHEMA_VERSION);
    drop(migrated);

    let connection = Connection::open(&database_path).expect("migrated database should reopen");
    let project: (String, Option<String>, Option<String>) = connection
        .query_row(
            "SELECT name, root_device, git_dir_path FROM projects WHERE id = 7",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("legacy project should remain");
    assert_eq!(project, ("legacy-project".into(), None, None));
    let draft: String = connection
        .query_row(
            "SELECT prompt FROM chat_drafts WHERE project_id = 7",
            [],
            |row| row.get(0),
        )
        .expect("legacy draft should remain");
    assert_eq!(draft, "Preserve this draft");
    let chat: (String, Option<i64>) = connection
        .query_row(
            "SELECT title, pull_request_number FROM chats WHERE id = 'legacy-chat'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("legacy chat should remain");
    assert_eq!(chat, ("Preserve this chat".into(), Some(42)));
}

#[test]
fn failed_migration_rolls_back_schema_and_version() {
    let app_data = TempDir::new().expect("temporary application data");
    let database_path = app_data.path().join("piu.sqlite3");
    drop(Database::open(&database_path).expect("initial migration succeeds"));
    let failing_migration = Migration::new(
        CURRENT_SCHEMA_VERSION + 1,
        "injected failure",
        "CREATE TABLE should_rollback (id INTEGER); SELECT missing_function();",
    );

    let result = Database::open_with_migrations(&database_path, &[failing_migration]);

    assert!(result.is_err());
    let recovered = Database::open(&database_path).expect("prior schema remains valid");
    assert_eq!(recovered.schema_version().unwrap(), CURRENT_SCHEMA_VERSION);
    assert!(!recovered.has_table("should_rollback").unwrap());
}
