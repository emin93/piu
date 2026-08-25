use piu_lib::runtime_preferences::{
    ModelRoute, ResourceScope, RuntimePreferences, RuntimeResource,
};
use rusqlite::{Connection, params};
use tempfile::TempDir;

#[test]
fn selected_route_and_each_routes_effort_survive_relaunch() {
    let app_data = TempDir::new().expect("temporary application data");
    let database_path = app_data.path().join("piu.sqlite3");
    let codex = ModelRoute::new("openai-codex", "gpt-5.4").unwrap();
    let api = ModelRoute::new("openai", "gpt-5.4").unwrap();

    let preferences = RuntimePreferences::open(&database_path).unwrap();
    assert_eq!(preferences.select_route(&codex).unwrap().effort, None);
    preferences.remember_effort(&codex, "high").unwrap();
    preferences.remember_effort(&api, "low").unwrap();
    assert_eq!(
        preferences.select_route(&api).unwrap().effort.as_deref(),
        Some("low")
    );
    drop(preferences);

    let relaunched = RuntimePreferences::open(&database_path).unwrap();
    assert_eq!(
        relaunched.current_selection().unwrap(),
        Some(api.selection(Some("low")))
    );
    assert_eq!(
        relaunched.select_route(&codex).unwrap(),
        codex.selection(Some("high"))
    );
}

#[test]
fn resource_overrides_are_isolated_by_kind_source_and_scope() {
    let app_data = TempDir::new().expect("temporary application data");
    let database_path = app_data.path().join("piu.sqlite3");
    let preferences = RuntimePreferences::open(&database_path).unwrap();
    let project_id = insert_project(&database_path);
    let global = ResourceScope::Global;
    let project = ResourceScope::Project(project_id);
    let skill = RuntimeResource::skill("piu://skills/review");
    let extension = RuntimeResource::extension("piu://extensions/review");
    let package = RuntimeResource::package("npm:@piu/review@1.2.3");
    let route = RuntimeResource::model_route(ModelRoute::new("openai-codex", "gpt-5.4").unwrap());

    preferences
        .set_resource_enabled(global, &skill, false)
        .unwrap();
    preferences
        .set_resource_enabled(project, &skill, true)
        .unwrap();
    preferences
        .set_resource_enabled(global, &extension, false)
        .unwrap();
    preferences
        .set_resource_enabled(global, &package, true)
        .unwrap();
    preferences
        .set_resource_enabled(global, &route, false)
        .unwrap();

    assert_eq!(
        preferences.resource_enabled(global, &skill).unwrap(),
        Some(false)
    );
    assert_eq!(
        preferences.resource_enabled(project, &skill).unwrap(),
        Some(true)
    );
    assert_eq!(
        preferences.resource_enabled(project, &extension).unwrap(),
        None
    );
    assert_eq!(
        preferences.resource_enabled(global, &extension).unwrap(),
        Some(false)
    );
    assert_eq!(
        preferences.resource_enabled(global, &package).unwrap(),
        Some(true)
    );
    assert_eq!(
        preferences.resource_enabled(global, &route).unwrap(),
        Some(false)
    );

    preferences
        .clear_resource_override(project, &skill)
        .unwrap();
    assert_eq!(preferences.resource_enabled(project, &skill).unwrap(), None);
    assert_eq!(
        preferences.resource_enabled(global, &skill).unwrap(),
        Some(false)
    );

    drop(preferences);
    let relaunched = RuntimePreferences::open(&database_path).unwrap();
    assert_eq!(
        relaunched.resource_enabled(global, &skill).unwrap(),
        Some(false)
    );
    assert_eq!(
        relaunched.resource_enabled(global, &route).unwrap(),
        Some(false)
    );
}

#[test]
fn invalid_or_unknown_project_identity_cannot_create_an_override() {
    let app_data = TempDir::new().expect("temporary application data");
    let database_path = app_data.path().join("piu.sqlite3");
    let preferences = RuntimePreferences::open(&database_path).unwrap();
    let skill = RuntimeResource::skill("piu://skills/review");

    assert!(
        preferences
            .set_resource_enabled(ResourceScope::Project(0), &skill, true)
            .is_err()
    );
    assert!(
        preferences
            .set_resource_enabled(ResourceScope::Project(404), &skill, true)
            .is_err()
    );
}

#[test]
fn removing_a_project_removes_its_resource_overrides() {
    let app_data = TempDir::new().expect("temporary application data");
    let database_path = app_data.path().join("piu.sqlite3");
    let preferences = RuntimePreferences::open(&database_path).unwrap();
    let project_id = insert_project(&database_path);
    let project = ResourceScope::Project(project_id);
    let skill = RuntimeResource::skill("piu://skills/review");
    preferences
        .set_resource_enabled(project, &skill, false)
        .unwrap();

    let connection = Connection::open(&database_path).unwrap();
    connection
        .pragma_update(None, "foreign_keys", "ON")
        .unwrap();
    connection
        .execute("DELETE FROM projects WHERE id = ?1", [project_id])
        .unwrap();

    assert_eq!(preferences.resource_enabled(project, &skill).unwrap(), None);
}

fn insert_project(database_path: &std::path::Path) -> i64 {
    let connection = Connection::open(database_path).unwrap();
    connection
        .execute(
            "INSERT INTO projects (
               canonical_path, root_device, root_inode, git_dir_path, git_dir_device,
               git_dir_inode, name, created_at_ms
             ) VALUES (?1, '1', '2', ?2, '1', '3', 'Project', 1)",
            params!["/tmp/project", "/tmp/project/.git"],
        )
        .unwrap();
    connection.last_insert_rowid()
}
