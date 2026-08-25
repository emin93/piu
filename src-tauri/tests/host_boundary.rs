use std::{sync::mpsc, time::Duration};

use piu_lib::application::ApplicationCore;
use piu_lib::git_process::GitProcess;
use piu_lib::host_boundary::{HOST_ROUND_TRIP_EVENT, HostRoundTripRequest, HostRoundTripResponse};
use tauri::{
    Listener, WebviewWindowBuilder,
    ipc::{CallbackFn, InvokeBody},
    test,
    webview::InvokeRequest,
};

#[test]
fn typed_round_trip_crosses_the_command_and_event_boundary() {
    let app_data = tempfile::TempDir::new().expect("temporary application data");
    let core = ApplicationCore::open(
        &app_data.path().join("piu.sqlite3"),
        GitProcess::with_executable("/usr/bin/git".into()),
    )
    .expect("application core");
    let app = piu_lib::configure_builder(test::mock_builder().manage(core))
        .build(test::mock_context(test::noop_assets()))
        .expect("mock Più application");
    let webview = WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .expect("mock main webview");
    let (event_sender, event_receiver) = mpsc::channel();
    webview.listen(HOST_ROUND_TRIP_EVENT, move |event| {
        let response =
            serde_json::from_str::<HostRoundTripResponse>(event.payload()).expect("typed event");
        event_sender.send(response).expect("send observed event");
    });
    let request = HostRoundTripRequest {
        correlation_id: "boundary-7".into(),
        sent_at_ms: 42,
    };

    let response = test::get_ipc_response(
        &webview,
        InvokeRequest {
            cmd: "host_round_trip".into(),
            callback: CallbackFn(0),
            error: CallbackFn(1),
            url: "tauri://localhost".parse().unwrap(),
            body: InvokeBody::Json(serde_json::json!({ "request": request })),
            headers: Default::default(),
            invoke_key: test::INVOKE_KEY.into(),
        },
    )
    .expect("IPC command succeeds")
    .deserialize::<HostRoundTripResponse>()
    .expect("typed IPC response");
    let event = event_receiver
        .recv_timeout(Duration::from_secs(1))
        .expect("matching event arrives");

    assert_eq!(response, event);
    assert_eq!(response.correlation_id, "boundary-7");
}
