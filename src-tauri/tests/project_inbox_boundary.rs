use std::{fs, process::Command, sync::mpsc, time::Duration};

use piu_lib::{
    application::ApplicationCore,
    project_commands::{
        OpenRepositoryRequest, OpenRepositoryResponse, PROJECT_INBOX_CHANGED_EVENT,
        ProjectInboxChangedEvent, SaveProjectDraftRequest,
    },
    project_inbox::DraftSummary,
};
use tauri::{
    Listener, WebviewWindowBuilder,
    ipc::{CallbackFn, InvokeBody},
    test,
    webview::InvokeRequest,
};

#[test]
fn opening_a_repository_crosses_the_typed_boundary_and_emits_one_coarse_change() {
    let fixture = tempfile::TempDir::new().expect("fixture should be created");
    let repository_path = fixture.path().join("alpha");
    fs::create_dir(&repository_path).expect("repository directory should be created");
    assert!(
        Command::new("git")
            .args(["init", "--quiet"])
            .arg(&repository_path)
            .status()
            .expect("git should create the fixture")
            .success()
    );
    let core = ApplicationCore::open(&fixture.path().join("piu.sqlite3"))
        .expect("application core should open");
    let app = piu_lib::configure_builder(test::mock_builder().manage(core))
        .build(test::mock_context(test::noop_assets()))
        .expect("mock Più application should build");
    let webview = WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .expect("mock main webview should build");
    let (event_sender, event_receiver) = mpsc::channel();
    app.listen(PROJECT_INBOX_CHANGED_EVENT, move |event| {
        let event = serde_json::from_str::<ProjectInboxChangedEvent>(event.payload())
            .expect("event should use the typed projection");
        event_sender.send(event).expect("event should be observed");
    });

    let response = test::get_ipc_response(
        &webview,
        InvokeRequest {
            cmd: "open_repository".into(),
            callback: CallbackFn(0),
            error: CallbackFn(1),
            url: "tauri://localhost".parse().unwrap(),
            body: InvokeBody::Json(serde_json::json!({
                "request": OpenRepositoryRequest {
                    path: repository_path.to_string_lossy().into_owned(),
                }
            })),
            headers: Default::default(),
            invoke_key: test::INVOKE_KEY.into(),
        },
    )
    .expect("IPC command should succeed")
    .deserialize::<OpenRepositoryResponse>()
    .expect("IPC response should be typed");
    let event = event_receiver
        .recv_timeout(Duration::from_secs(1))
        .expect("one matching change event should arrive");

    assert_eq!(
        response.focused_project_id,
        response.snapshot.projects[0].id
    );
    assert_eq!(event.snapshot, response.snapshot);
    assert_eq!(event.focused_project_id, Some(response.focused_project_id));
    assert!(event_receiver.try_recv().is_err());

    let saved_draft = test::get_ipc_response(
        &webview,
        InvokeRequest {
            cmd: "save_project_draft".into(),
            callback: CallbackFn(2),
            error: CallbackFn(3),
            url: "tauri://localhost".parse().unwrap(),
            body: InvokeBody::Json(serde_json::json!({
                "request": SaveProjectDraftRequest {
                    project_id: response.focused_project_id,
                    prompt: "Retained prompt".into(),
                }
            })),
            headers: Default::default(),
            invoke_key: test::INVOKE_KEY.into(),
        },
    )
    .expect("draft IPC command should succeed")
    .deserialize::<DraftSummary>()
    .expect("draft IPC response should match its generated contract");
    assert_eq!(saved_draft.project_id, response.focused_project_id);
    assert_eq!(saved_draft.prompt, "Retained prompt");
    assert!(
        event_receiver.try_recv().is_err(),
        "draft persistence must not emit a broad inbox refresh per save"
    );
}
