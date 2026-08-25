#[allow(dead_code)]
mod support;

use std::{
    fs,
    path::Path,
    process::Command,
    sync::{Arc, Mutex, mpsc},
    time::{Duration, Instant},
};

use piu_lib::{
    application::ApplicationCore,
    git_process::GitProcess,
    host_boundary::{HostRoundTripRequest, HostRoundTripResponse},
    project_commands::{
        CHAT_SETUP_CHANGED_EVENT, CHAT_TERMINAL_REQUESTED_EVENT, ChatIdRequest, CreateChatRequest,
        CreateChatResponse, OpenRepositoryRequest, OpenRepositoryResponse,
        PROJECT_INBOX_CHANGED_EVENT, ProjectInboxChangedEvent, RenameChatRequest,
        SaveProjectDraftRequest,
    },
    project_inbox::{
        DraftSummary, ProjectInbox, RepositoryIdentity, RepositoryInspectionError,
        RepositoryInspector,
    },
};
use support::TemporaryGitRemote;
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
    let core = ApplicationCore::open(
        &fixture.path().join("piu.sqlite3"),
        GitProcess::with_executable("/usr/bin/git".into()),
    )
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
                    attachments: vec![],
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

struct DelayedInspector {
    started: Mutex<Option<mpsc::SyncSender<()>>>,
    release: Mutex<mpsc::Receiver<()>>,
}

impl RepositoryInspector for DelayedInspector {
    fn inspect(
        &self,
        _selected_path: &Path,
    ) -> Result<RepositoryIdentity, RepositoryInspectionError> {
        if let Some(started) = self.started.lock().unwrap().take() {
            started.send(()).unwrap();
        }
        self.release.lock().unwrap().recv().unwrap();
        Err(RepositoryInspectionError::Missing)
    }
}

#[test]
fn delayed_git_inspection_does_not_block_another_ipc_request() {
    let fixture = tempfile::TempDir::new().unwrap();
    let repository_path = fixture.path().join("alpha");
    fs::create_dir(&repository_path).unwrap();
    assert!(
        Command::new("/usr/bin/git")
            .args(["init", "--quiet"])
            .arg(&repository_path)
            .status()
            .unwrap()
            .success()
    );
    let database_path = fixture.path().join("piu.sqlite3");
    let initial = ProjectInbox::with_git(
        &database_path,
        GitProcess::with_executable("/usr/bin/git".into()),
    )
    .unwrap();
    initial.open_repository(&repository_path).unwrap();
    drop(initial);

    let (started_sender, started_receiver) = mpsc::sync_channel(0);
    let (release_sender, release_receiver) = mpsc::sync_channel(0);
    let inbox = ProjectInbox::with_inspector(
        &database_path,
        Arc::new(DelayedInspector {
            started: Mutex::new(Some(started_sender)),
            release: Mutex::new(release_receiver),
        }),
    )
    .unwrap();
    let core = ApplicationCore::from_project_inbox(
        inbox,
        fixture.path(),
        GitProcess::with_executable("/usr/bin/git".into()),
    );
    let app = piu_lib::configure_builder(test::mock_builder().manage(core))
        .build(test::mock_context(test::noop_assets()))
        .unwrap();
    let webview = WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .unwrap();
    let (load_sender, load_receiver) = mpsc::channel();

    webview.clone().on_message(
        InvokeRequest {
            cmd: "load_project_inbox".into(),
            callback: CallbackFn(10),
            error: CallbackFn(11),
            url: "tauri://localhost".parse().unwrap(),
            body: InvokeBody::default(),
            headers: Default::default(),
            invoke_key: test::INVOKE_KEY.into(),
        },
        Box::new(move |_window, _cmd, response, _callback, _error| {
            load_sender.send(response).unwrap();
        }),
    );
    started_receiver
        .recv_timeout(Duration::from_secs(1))
        .expect("the slow Git inspection should start");
    let started = Instant::now();

    let response = test::get_ipc_response(
        &webview,
        InvokeRequest {
            cmd: "host_round_trip".into(),
            callback: CallbackFn(12),
            error: CallbackFn(13),
            url: "tauri://localhost".parse().unwrap(),
            body: InvokeBody::Json(serde_json::json!({
                "request": HostRoundTripRequest {
                    correlation_id: "while-git-is-slow".into(),
                    sent_at_ms: 7,
                }
            })),
            headers: Default::default(),
            invoke_key: test::INVOKE_KEY.into(),
        },
    )
    .expect("an independent IPC request should complete")
    .deserialize::<HostRoundTripResponse>()
    .unwrap();

    assert_eq!(response.correlation_id, "while-git-is-slow");
    assert!(started.elapsed() < Duration::from_millis(250));
    release_sender.send(()).unwrap();
    load_receiver
        .recv_timeout(Duration::from_secs(1))
        .expect("the deferred load should finish after inspection is released");
}

#[test]
fn first_send_crosses_the_typed_boundary_and_publishes_setup_and_terminal_actions() {
    let fixture = tempfile::TempDir::new().expect("fixture should be created");
    let repository = TemporaryGitRemote::new();
    fs::write(repository.working_path().join("README.md"), "fixture\n").unwrap();
    repository.git(["add", "README.md"]);
    repository.git(["commit", "-m", "fixture"]);
    repository.git(["push", "-u", "origin", "main"]);
    let core = ApplicationCore::open(
        &fixture.path().join("piu.sqlite3"),
        GitProcess::with_executable("/usr/bin/git".into()),
    )
    .unwrap();
    let project_id = core
        .project_inbox()
        .open_repository(repository.working_path())
        .unwrap()
        .project
        .id;
    let app = piu_lib::configure_builder(test::mock_builder().manage(core))
        .build(test::mock_context(test::noop_assets()))
        .unwrap();
    let webview = WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .unwrap();
    let (setup_sender, setup_receiver) = mpsc::channel();
    app.listen(CHAT_SETUP_CHANGED_EVENT, move |event| {
        setup_sender
            .send(
                serde_json::from_str::<piu_lib::chat_workspaces::ChatSetupChangedEvent>(
                    event.payload(),
                )
                .unwrap(),
            )
            .unwrap();
    });
    let (terminal_sender, terminal_receiver) = mpsc::channel();
    app.listen(CHAT_TERMINAL_REQUESTED_EVENT, move |event| {
        terminal_sender.send(event.payload().to_owned()).unwrap();
    });
    let (inbox_sender, inbox_receiver) = mpsc::channel();
    app.listen(PROJECT_INBOX_CHANGED_EVENT, move |event| {
        inbox_sender
            .send(
                serde_json::from_str::<ProjectInboxChangedEvent>(event.payload())
                    .expect("inbox event should be typed"),
            )
            .unwrap();
    });

    let response = test::get_ipc_response(
        &webview,
        InvokeRequest {
            cmd: "create_chat".into(),
            callback: CallbackFn(20),
            error: CallbackFn(21),
            url: "tauri://localhost".parse().unwrap(),
            body: InvokeBody::Json(serde_json::json!({
                "request": CreateChatRequest {
                    project_id,
                    prompt: "Build the parser boundary".into(),
                    attachments: vec![],
                }
            })),
            headers: Default::default(),
            invoke_key: test::INVOKE_KEY.into(),
        },
    )
    .unwrap()
    .deserialize::<CreateChatResponse>()
    .unwrap();

    assert_eq!(response.snapshot.chats.len(), 1);
    assert_eq!(response.chat.id, response.snapshot.chats[0].id);
    assert!(
        response
            .chat
            .branch_name
            .ends_with("-build-the-parser-boundary")
    );
    inbox_receiver
        .recv_timeout(Duration::from_secs(1))
        .expect("chat creation should publish its inbox change");

    let renamed = test::get_ipc_response(
        &webview,
        InvokeRequest {
            cmd: "rename_chat".into(),
            callback: CallbackFn(24),
            error: CallbackFn(25),
            url: "tauri://localhost".parse().unwrap(),
            body: InvokeBody::Json(serde_json::json!({
                "request": RenameChatRequest {
                    chat_id: response.chat.id.clone(),
                    title: "  Parser   boundary  ".into(),
                }
            })),
            headers: Default::default(),
            invoke_key: test::INVOKE_KEY.into(),
        },
    )
    .unwrap()
    .deserialize::<piu_lib::project_inbox::InboxSnapshot>()
    .unwrap();
    assert_eq!(renamed.chats[0].title, "Parser boundary");
    assert_eq!(renamed.chats[0].branch_name, response.chat.branch_name);
    let renamed_event = inbox_receiver
        .recv_timeout(Duration::from_secs(1))
        .expect("rename should publish one inbox change");
    assert_eq!(renamed_event.snapshot, renamed);
    assert_eq!(renamed_event.focused_project_id, None);
    assert!(inbox_receiver.try_recv().is_err());

    let setup = setup_receiver.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(
        setup.setup.phase,
        piu_lib::project_inbox::ChatSetupPhase::NotRequired
    );

    test::get_ipc_response(
        &webview,
        InvokeRequest {
            cmd: "open_chat_terminal".into(),
            callback: CallbackFn(22),
            error: CallbackFn(23),
            url: "tauri://localhost".parse().unwrap(),
            body: InvokeBody::Json(serde_json::json!({
                "request": ChatIdRequest {
                    chat_id: response.chat.id.clone(),
                }
            })),
            headers: Default::default(),
            invoke_key: test::INVOKE_KEY.into(),
        },
    )
    .unwrap();
    let terminal_event = terminal_receiver
        .recv_timeout(Duration::from_secs(1))
        .unwrap();
    assert!(!terminal_event.contains("worktrees"));
    assert!(terminal_event.contains(&response.chat.id));
}
