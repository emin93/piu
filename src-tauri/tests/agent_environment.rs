use std::{
    collections::BTreeMap,
    ffi::{OsStr, OsString},
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
    time::{Duration, Instant},
};

use piu_lib::{
    agent_environment::{
        AgentEnvironment, AgentEnvironmentError, AgentEnvironmentPolicy,
        AgentEnvironmentProcessSpec, AgentResourceId, AgentResourcePreferenceScope,
        AgentResourceSource,
    },
    chat_runtime_host::{ModelRouteId, ReasoningEffort},
    git_process::GitProcess,
    project_inbox::ProjectInbox,
    runtime_preferences::RuntimePreferences,
};
use tauri::{
    WebviewWindowBuilder,
    ipc::{CallbackFn, InvokeBody},
    test,
    webview::InvokeRequest,
};
use tempfile::TempDir;

fn make_repository(path: &Path) {
    fs::create_dir_all(path).unwrap();
    assert!(
        Command::new("/usr/bin/git")
            .args(["init", "--quiet"])
            .arg(path)
            .status()
            .unwrap()
            .success()
    );
}

struct Fixture {
    _root: TempDir,
    app_data: PathBuf,
    project_id: i64,
    repository: PathBuf,
    environment: AgentEnvironment,
}

impl Fixture {
    fn new(mode: &str, policy: AgentEnvironmentPolicy) -> Self {
        let root = TempDir::new().unwrap();
        let app_data = root.path().join("app-data");
        let repository = root.path().join("repository");
        fs::create_dir_all(&app_data).unwrap();
        make_repository(&repository);
        let database_path = app_data.join("piu.sqlite3");
        let inbox = Arc::new(
            ProjectInbox::with_git(
                &database_path,
                GitProcess::with_executable("/usr/bin/git".into()),
            )
            .unwrap(),
        );
        let project_id = inbox.open_repository(&repository).unwrap().project.id;
        let preferences = Arc::new(RuntimePreferences::open(&database_path).unwrap());
        let launcher = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/agent-environment-child.zsh");
        let mut environment = BTreeMap::new();
        environment.insert(OsString::from("HOME"), OsString::from("/Users/piu-test"));
        environment.insert(OsString::from("PATH"), OsString::from("/usr/bin:/bin"));
        environment.insert(
            OsString::from("PIU_ENVIRONMENT_FIXTURE_MODE"),
            OsString::from(mode),
        );
        environment.insert(
            OsString::from("PIU_ENVIRONMENT_FIXTURE_RECORD_DIR"),
            root.path().as_os_str().to_owned(),
        );
        let process = AgentEnvironmentProcessSpec {
            executable: PathBuf::from("/bin/zsh"),
            launcher,
            agent_directory: app_data.join("agent"),
            credential_lock_directory: app_data.join("credential-locks"),
            environment,
        };
        let environment = AgentEnvironment::new(inbox, preferences, process, policy).unwrap();
        Self {
            _root: root,
            app_data,
            project_id,
            repository,
            environment,
        }
    }
}

fn test_policy() -> AgentEnvironmentPolicy {
    AgentEnvironmentPolicy {
        inspection_timeout: Duration::from_secs(2),
        maximum_stdout_bytes: 64 * 1024,
        maximum_stderr_bytes: 16 * 1024,
    }
}

#[test]
fn bundled_process_spec_uses_the_app_owned_agent_and_exact_git_environment() {
    let spec = AgentEnvironmentProcessSpec::from_bundled_runtime(
        Path::new("/Applications/Più.app/Contents/Resources"),
        Path::new("/Users/test/Library/Application Support/ch.emin.piu"),
        Path::new("/Users/test"),
    );

    assert_eq!(
        spec.executable,
        PathBuf::from("/Applications/Più.app/Contents/Resources/agent-runtime/node/bin/node")
    );
    assert_eq!(
        spec.launcher,
        PathBuf::from(
            "/Applications/Più.app/Contents/Resources/agent-runtime/pi/launcher/environment-launcher.mjs"
        )
    );
    assert_eq!(
        spec.agent_directory,
        PathBuf::from("/Users/test/Library/Application Support/ch.emin.piu/agent")
    );
    assert_eq!(
        spec.environment.get(OsStr::new("HOME")).unwrap(),
        "/Users/test"
    );
    assert_eq!(
        spec.environment.get(OsStr::new("PATH")).unwrap(),
        "/Applications/Più.app/Contents/Resources/git/bin:/usr/bin:/bin"
    );
    assert_eq!(
        spec.environment.get(OsStr::new("GIT_EXEC_PATH")).unwrap(),
        "/Applications/Più.app/Contents/Resources/git/libexec/git-core"
    );
    assert_eq!(
        spec.environment
            .get(OsStr::new("GIT_TEMPLATE_DIR"))
            .unwrap(),
        "/Applications/Più.app/Contents/Resources/git/share/git-core/templates"
    );
    assert_eq!(spec.environment.len(), 7);
}

#[tokio::test]
async fn project_snapshot_uses_the_verified_repository_and_returns_deep_typed_controls() {
    let fixture = Fixture::new("snapshot", test_policy());

    let snapshot = fixture
        .environment
        .snapshot(fixture.project_id)
        .await
        .unwrap();

    assert_eq!(snapshot.model_routes.len(), 2);
    assert_eq!(snapshot.model_routes[0].name, "GPT 5.6");
    assert!(snapshot.model_routes[0].enabled);
    assert_eq!(snapshot.model_controls.routes.len(), 2);
    assert_eq!(
        snapshot.model_controls.selected_route,
        ModelRouteId {
            provider: "openai-codex".into(),
            model_id: "gpt-5.6-sol".into(),
        }
    );
    assert_eq!(snapshot.model_controls.efforts[3], ReasoningEffort::Maximum);
    assert_eq!(
        snapshot.model_controls.selected_effort,
        ReasoningEffort::Off
    );
    assert!(!snapshot.model_controls.applies_after_current_step);
    assert_eq!(snapshot.resources.skills[0].id, "project://skills/check");
    assert_eq!(snapshot.resources.skills[0].name, "Check");
    assert_eq!(
        snapshot.resources.skills[0].source,
        AgentResourceSource::Project
    );
    assert_eq!(snapshot.diagnostics[0].message, "Fixture warning");
    assert_eq!(
        fs::read_to_string(fixture._root.path().join("cwd"))
            .unwrap()
            .trim(),
        fixture.repository.canonicalize().unwrap().to_string_lossy()
    );
    let arguments = fs::read_to_string(fixture._root.path().join("arguments")).unwrap();
    assert!(arguments.contains(fixture.repository.to_string_lossy().as_ref()));
    assert!(arguments.contains(fixture.app_data.join("agent").to_string_lossy().as_ref()));
    assert!(!fixture.app_data.join("agent").exists());
    assert!(fixture.app_data.join("credential-locks").is_dir());
}

#[tokio::test]
async fn route_changes_use_only_inspected_levels_and_revalidate_remembered_effort() {
    let fixture = Fixture::new("snapshot", test_policy());
    let local = ModelRouteId {
        provider: "local".into(),
        model_id: "qwen".into(),
    };

    let selected = fixture
        .environment
        .select_model_route(fixture.project_id, local.clone())
        .await
        .unwrap();
    assert_eq!(selected.selected_route, local);
    assert_eq!(
        selected.efforts,
        vec![
            ReasoningEffort::Low,
            ReasoningEffort::Medium,
            ReasoningEffort::ExtraHigh
        ]
    );
    assert_eq!(selected.selected_effort, ReasoningEffort::Low);
    let selected = fixture
        .environment
        .select_reasoning_effort(fixture.project_id, ReasoningEffort::ExtraHigh)
        .await
        .unwrap();
    assert_eq!(selected.selected_effort, ReasoningEffort::ExtraHigh);
    assert!(matches!(
        fixture
            .environment
            .select_reasoning_effort(fixture.project_id, ReasoningEffort::Maximum)
            .await,
        Err(AgentEnvironmentError::EffortUnavailable { .. })
    ));

    // The persistence seam has its own relaunch contract; this verifies that the stored value is
    // revalidated against the current Pi catalog before it reaches the controls.
    let controls = fixture
        .environment
        .model_controls(fixture.project_id)
        .await
        .unwrap();
    assert_eq!(controls.selected_route.provider, "local");
    assert_eq!(controls.selected_effort, ReasoningEffort::ExtraHigh);
}

#[tokio::test]
async fn resource_preferences_are_validated_against_the_project_inventory() {
    let fixture = Fixture::new("snapshot", test_policy());
    let resource = AgentResourceId::Skill {
        id: "project://skills/check".into(),
    };
    let change = fixture
        .environment
        .set_resource_enabled(
            fixture.project_id,
            AgentResourcePreferenceScope::Project,
            resource.clone(),
            false,
        )
        .await
        .unwrap();
    assert_eq!(change.resource, resource);
    assert_eq!(change.scope, AgentResourcePreferenceScope::Project);
    assert!(!change.enabled);
    let snapshot = fixture
        .environment
        .snapshot(fixture.project_id)
        .await
        .unwrap();
    assert!(!snapshot.resources.skills[0].enabled);

    assert!(matches!(
        fixture
            .environment
            .set_resource_enabled(
                fixture.project_id,
                AgentResourcePreferenceScope::Global,
                AgentResourceId::Extension {
                    id: "unknown".into()
                },
                false,
            )
            .await,
        Err(AgentEnvironmentError::ResourceUnavailable)
    ));
}

#[tokio::test]
async fn disabling_the_selected_route_atomically_persists_a_valid_fallback() {
    let fixture = Fixture::new("snapshot", test_policy());
    fixture
        .environment
        .set_resource_enabled(
            fixture.project_id,
            AgentResourcePreferenceScope::Global,
            AgentResourceId::ModelRoute {
                route: ModelRouteId {
                    provider: "openai-codex".into(),
                    model_id: "gpt-5.6-sol".into(),
                },
            },
            false,
        )
        .await
        .unwrap();

    let snapshot = fixture
        .environment
        .snapshot(fixture.project_id)
        .await
        .unwrap();
    assert_eq!(snapshot.model_controls.selected_route.provider, "local");
    assert_eq!(
        snapshot.model_controls.selected_effort,
        ReasoningEffort::Low
    );
    let persisted = RuntimePreferences::open(&fixture.app_data.join("piu.sqlite3")).unwrap();
    let selection = persisted.current_selection().unwrap().unwrap();
    assert_eq!(selection.route.provider_id(), "local");
    assert_eq!(selection.effort.as_deref(), Some("low"));

    assert!(matches!(
        fixture
            .environment
            .set_resource_enabled(
                fixture.project_id,
                AgentResourcePreferenceScope::Global,
                AgentResourceId::ModelRoute {
                    route: ModelRouteId {
                        provider: "local".into(),
                        model_id: "qwen".into(),
                    },
                },
                false,
            )
            .await,
        Err(AgentEnvironmentError::CannotDisableLastModelRoute)
    ));
    assert!(
        fixture
            .environment
            .snapshot(fixture.project_id)
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn package_preferences_cascade_to_launch_paths_with_project_precedence() {
    let fixture = Fixture::new("snapshot", test_policy());
    let package = AgentResourceId::Package {
        id: "npm:@piu/review@1.0.0".into(),
    };
    let initial = fixture
        .environment
        .launch_resources(fixture.project_id)
        .await
        .unwrap();
    assert_eq!(initial.extension_paths.len(), 2);
    assert_eq!(initial.skill_paths.len(), 2);

    fixture
        .environment
        .set_resource_enabled(
            fixture.project_id,
            AgentResourcePreferenceScope::Global,
            package.clone(),
            false,
        )
        .await
        .unwrap();
    let globally_disabled = fixture
        .environment
        .launch_resources(fixture.project_id)
        .await
        .unwrap();
    assert_eq!(
        globally_disabled.extension_paths,
        vec![PathBuf::from("/private/tmp/piu/extensions/review.mjs")]
    );
    assert_eq!(
        globally_disabled.skill_paths,
        vec![PathBuf::from(
            "/private/tmp/project/.pi/skills/check/SKILL.md"
        )]
    );

    fixture
        .environment
        .set_resource_enabled(
            fixture.project_id,
            AgentResourcePreferenceScope::Project,
            package,
            true,
        )
        .await
        .unwrap();
    let project_enabled = fixture
        .environment
        .launch_resources(fixture.project_id)
        .await
        .unwrap();
    assert_eq!(project_enabled.extension_paths.len(), 2);
    assert_eq!(project_enabled.skill_paths.len(), 2);

    fixture
        .environment
        .set_resource_enabled(
            fixture.project_id,
            AgentResourcePreferenceScope::Project,
            AgentResourceId::Extension {
                id: "package://extensions/review".into(),
            },
            false,
        )
        .await
        .unwrap();
    let specifically_disabled = fixture
        .environment
        .launch_resources(fixture.project_id)
        .await
        .unwrap();
    assert_eq!(specifically_disabled.extension_paths.len(), 1);
    assert_eq!(specifically_disabled.skill_paths.len(), 2);
}

#[tokio::test]
async fn inspector_output_and_duration_are_bounded_and_children_are_reaped() {
    let oversize = Fixture::new(
        "oversize",
        AgentEnvironmentPolicy {
            maximum_stdout_bytes: 1024,
            ..test_policy()
        },
    );
    assert!(matches!(
        oversize.environment.snapshot(oversize.project_id).await,
        Err(AgentEnvironmentError::OutputLimitExceeded)
    ));

    let sleeping = Fixture::new(
        "sleep",
        AgentEnvironmentPolicy {
            inspection_timeout: Duration::from_millis(100),
            ..test_policy()
        },
    );
    let started = Instant::now();
    assert!(matches!(
        sleeping.environment.snapshot(sleeping.project_id).await,
        Err(AgentEnvironmentError::TimedOut)
    ));
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[test]
fn snapshot_and_resource_preference_cross_the_typed_tauri_boundary() {
    let Fixture {
        _root,
        project_id,
        environment,
        ..
    } = Fixture::new("snapshot", test_policy());
    let app = piu_lib::configure_builder(test::mock_builder().manage(environment))
        .build(test::mock_context(test::noop_assets()))
        .unwrap();
    let webview = WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .unwrap();
    let snapshot = test::get_ipc_response(
        &webview,
        InvokeRequest {
            cmd: "get_project_agent_environment".into(),
            callback: CallbackFn(0),
            error: CallbackFn(1),
            url: "tauri://localhost".parse().unwrap(),
            body: InvokeBody::Json(serde_json::json!({
                "request": { "projectId": project_id }
            })),
            headers: Default::default(),
            invoke_key: test::INVOKE_KEY.into(),
        },
    )
    .unwrap()
    .deserialize::<piu_lib::agent_environment::AgentEnvironmentSnapshot>()
    .unwrap();
    assert_eq!(
        snapshot.model_controls.selected_effort,
        ReasoningEffort::Off
    );

    let change = test::get_ipc_response(
        &webview,
        InvokeRequest {
            cmd: "set_agent_resource_enabled".into(),
            callback: CallbackFn(2),
            error: CallbackFn(3),
            url: "tauri://localhost".parse().unwrap(),
            body: InvokeBody::Json(serde_json::json!({
                "request": {
                    "projectId": project_id,
                    "scope": "project",
                    "resource": { "kind": "skill", "id": "project://skills/check" },
                    "enabled": false
                }
            })),
            headers: Default::default(),
            invoke_key: test::INVOKE_KEY.into(),
        },
    )
    .unwrap()
    .deserialize::<piu_lib::agent_environment::AgentResourcePreferenceChange>()
    .unwrap();
    assert!(!change.enabled);
}
