use std::{
    collections::BTreeMap,
    ffi::OsString,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

use piu_lib::pi_rpc::{PiRpcChild, PiRpcError, PiRpcPolicy, PiRpcProcessSpec};
use serde_json::json;
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

fn fixture_spec(fixture: &TempDir, mode: &str) -> PiRpcProcessSpec {
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pi-rpc-child.zsh");
    let mut environment = BTreeMap::new();
    environment.insert(OsString::from("PIU_RPC_FIXTURE_MODE"), OsString::from(mode));
    environment.insert(
        OsString::from("PIU_RPC_FIXTURE_RECORD_DIR"),
        fixture.path().as_os_str().to_owned(),
    );
    environment.insert(
        OsString::from("PIU_RPC_FIXTURE_EXPLICIT_ENV"),
        OsString::from("isolated"),
    );
    environment.insert(OsString::from("PATH"), OsString::from("/usr/bin:/bin"));
    PiRpcProcessSpec {
        executable: PathBuf::from("/bin/zsh"),
        arguments: vec![script.into_os_string()],
        working_directory: fixture.path().to_path_buf(),
        environment,
    }
}

fn test_policy() -> PiRpcPolicy {
    PiRpcPolicy {
        readiness_timeout: Duration::from_secs(1),
        request_timeout: Duration::from_secs(1),
        graceful_shutdown_timeout: Duration::from_millis(200),
        maximum_record_bytes: 64 * 1024,
        retained_stderr_bytes: 8 * 1024,
        write_queue_capacity: 2,
        event_queue_capacity: 8,
    }
}

fn process_exists(path: &Path) -> bool {
    let pid = std::fs::read_to_string(path).expect("fixture should record its process id");
    Command::new("/bin/kill")
        .args(["-0", pid.trim()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("kill should inspect fixture process")
        .success()
}

async fn wait_until(condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(1);
    while !condition() && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn readiness_command_and_events_cross_the_supervised_child_seam() {
    let fixture = TempDir::new().expect("fixture should be created");
    let child = PiRpcChild::launch(fixture_spec(&fixture, "normal"), test_policy())
        .await
        .expect("correlated get_state should establish readiness");
    let mut events = child.subscribe();

    let response = child
        .request(
            json!({ "type": "prompt", "message": "hello" }),
            CancellationToken::new(),
        )
        .await
        .expect("command should receive its correlated response");
    let event = events
        .recv()
        .await
        .expect("interleaved event should be emitted");

    assert_eq!(response.command, "prompt");
    assert_eq!(response.data, Some(json!({ "accepted": true })));
    assert_eq!(event.kind, "agent_start");
    assert_eq!(event.payload["fixture"], "event-before-response");
    assert_eq!(
        std::fs::read_to_string(fixture.path().join("cwd"))
            .unwrap()
            .trim(),
        fixture.path().canonicalize().unwrap().to_string_lossy()
    );
    assert_eq!(
        std::fs::read_to_string(fixture.path().join("environment"))
            .unwrap()
            .trim(),
        "isolated"
    );

    child
        .shutdown()
        .await
        .expect("child should stop gracefully");
}

#[tokio::test]
async fn strict_lf_framing_preserves_unicode_separators_and_final_unterminated_response() {
    let fixture = TempDir::new().unwrap();
    let child = PiRpcChild::launch(fixture_spec(&fixture, "framing"), test_policy())
        .await
        .unwrap();
    let mut events = child.subscribe();

    let response = child
        .request(json!({ "type": "frame_test" }), CancellationToken::new())
        .await
        .expect("final nonempty record should be emitted at EOF");
    let event = events.recv().await.unwrap();

    assert_eq!(response.data, Some(json!({ "framed": true })));
    assert_eq!(event.kind, "future_event");
    assert_eq!(event.payload["text"], "before\u{2028}middle\u{2029}after");
}

#[tokio::test]
async fn every_byte_split_of_a_unicode_crlf_event_preserves_one_jsonl_record() {
    let fixture = TempDir::new().unwrap();
    let frame = "{\"type\":\"future_event\",\"text\":\"Zürich \u{2028} café\"}\r\n";

    for split_at in 0..=frame.len() {
        let mut spec = fixture_spec(&fixture, "framing-split");
        spec.environment.insert(
            OsString::from("PIU_RPC_FIXTURE_SPLIT_AT"),
            OsString::from(split_at.to_string()),
        );
        let child = PiRpcChild::launch(spec, test_policy()).await.unwrap();
        let mut events = child.subscribe();

        let response = child
            .request(
                json!({ "type": "frame_property" }),
                CancellationToken::new(),
            )
            .await
            .unwrap();
        let event = events.recv().await.unwrap();

        assert_eq!(
            response.command, "frame_property",
            "split at byte {split_at}"
        );
        assert_eq!(event.kind, "future_event", "split at byte {split_at}");
        assert_eq!(
            event.payload["text"], "Zürich \u{2028} café",
            "split at byte {split_at}"
        );
        child.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn concurrent_requests_are_correlated_when_responses_arrive_out_of_order() {
    let fixture = TempDir::new().unwrap();
    let child = PiRpcChild::launch(fixture_spec(&fixture, "out-of-order"), test_policy())
        .await
        .unwrap();

    let (first, second) = tokio::join!(
        child.request(json!({ "type": "first" }), CancellationToken::new()),
        child.request(json!({ "type": "second" }), CancellationToken::new()),
    );

    assert_eq!(first.unwrap().data, Some(json!({ "slot": "first" })));
    assert_eq!(second.unwrap().data, Some(json!({ "slot": "second" })));
    child.shutdown().await.unwrap();
}

#[tokio::test]
async fn timed_out_request_is_rejected_and_its_late_response_cannot_complete_another_request() {
    let fixture = TempDir::new().unwrap();
    let mut policy = test_policy();
    policy.request_timeout = Duration::from_millis(40);
    let child = PiRpcChild::launch(fixture_spec(&fixture, "hold-then-late"), policy)
        .await
        .unwrap();

    let timed_out = child
        .request(json!({ "type": "held" }), CancellationToken::new())
        .await;
    assert_eq!(
        timed_out,
        Err(PiRpcError::RequestTimedOut {
            command: "held".into()
        })
    );

    let response = child
        .request(json!({ "type": "next" }), CancellationToken::new())
        .await
        .expect("late response should be recognized and ignored");
    assert_eq!(response.command, "next");
    assert_eq!(response.data, Some(json!({ "accepted": true })));
    child.shutdown().await.unwrap();
}

#[tokio::test]
async fn cancelled_request_is_rejected_without_poisoning_the_child() {
    let fixture = TempDir::new().unwrap();
    let child = PiRpcChild::launch(fixture_spec(&fixture, "hold-then-late"), test_policy())
        .await
        .unwrap();
    let cancellation = CancellationToken::new();
    cancellation.cancel();

    let cancelled = child.request(json!({ "type": "held" }), cancellation).await;
    assert_eq!(
        cancelled,
        Err(PiRpcError::RequestCancelled {
            command: "held".into()
        })
    );

    // A pre-cancelled request may never have reached the child, so establish a held request first.
    let held_cancellation = CancellationToken::new();
    let cancel_later = held_cancellation.clone();
    let held = child.request(json!({ "type": "held" }), held_cancellation);
    let cancel = async move {
        tokio::time::sleep(Duration::from_millis(20)).await;
        cancel_later.cancel();
    };
    let (held, ()) = tokio::join!(held, cancel);
    assert!(matches!(held, Err(PiRpcError::RequestCancelled { .. })));

    let response = child
        .request(json!({ "type": "next" }), CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(response.command, "next");
    child.shutdown().await.unwrap();
}

#[tokio::test]
async fn remote_preflight_failure_is_distinct_from_transport_failure() {
    let fixture = TempDir::new().unwrap();
    let child = PiRpcChild::launch(fixture_spec(&fixture, "remote-failure"), test_policy())
        .await
        .unwrap();

    let error = child
        .request(json!({ "type": "prompt" }), CancellationToken::new())
        .await
        .unwrap_err();

    assert_eq!(
        error,
        PiRpcError::Remote {
            command: "prompt".into(),
            message: "fixture rejection".into()
        }
    );
    child.shutdown().await.unwrap();
}

#[tokio::test]
async fn malformed_invalid_utf8_and_oversized_stdout_are_fatal() {
    for (mode, expected) in [
        ("malformed", "not a JSON object"),
        ("invalid-utf8", "not valid UTF-8"),
        ("oversized", "exceeded 1024 bytes"),
    ] {
        let fixture = TempDir::new().unwrap();
        let mut policy = test_policy();
        policy.maximum_record_bytes = 1024;
        let child = PiRpcChild::launch(fixture_spec(&fixture, mode), policy)
            .await
            .unwrap();

        let error = child
            .request(json!({ "type": "trigger" }), CancellationToken::new())
            .await
            .unwrap_err();

        assert!(
            matches!(&error, PiRpcError::Protocol(message) if message.contains(expected)),
            "{mode} returned {error:?}"
        );
        child.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn fatal_protocol_corruption_terminates_the_owned_process_group() {
    let fixture = TempDir::new().unwrap();
    let child = PiRpcChild::launch(
        fixture_spec(&fixture, "malformed-descendant"),
        test_policy(),
    )
    .await
    .unwrap();
    let parent = fixture.path().join("parent.pid");
    let descendant = fixture.path().join("descendant.pid");
    wait_until(|| parent.exists() && descendant.exists()).await;

    let error = child
        .request(json!({ "type": "trigger" }), CancellationToken::new())
        .await
        .unwrap_err();
    assert!(matches!(error, PiRpcError::Protocol(_)));
    wait_until(|| !process_exists(&parent) && !process_exists(&descendant)).await;

    assert!(!process_exists(&parent));
    assert!(!process_exists(&descendant));
    child.shutdown().await.unwrap();
}

#[tokio::test]
async fn unknown_duplicate_and_mismatched_responses_stop_the_transport() {
    let unsolicited_fixture = TempDir::new().unwrap();
    let unsolicited = PiRpcChild::launch(
        fixture_spec(&unsolicited_fixture, "unsolicited-response"),
        test_policy(),
    )
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_millis(20)).await;
    let error = unsolicited
        .request(json!({ "type": "after_unknown" }), CancellationToken::new())
        .await
        .unwrap_err();
    assert!(
        matches!(error, PiRpcError::Protocol(message) if message.contains("unknown or duplicate"))
    );
    unsolicited.shutdown().await.unwrap();

    for mode in ["duplicate-response", "mismatched-command"] {
        let fixture = TempDir::new().unwrap();
        let child = PiRpcChild::launch(fixture_spec(&fixture, mode), test_policy())
            .await
            .unwrap();
        let first = child
            .request(json!({ "type": "one" }), CancellationToken::new())
            .await;
        if mode == "duplicate-response" {
            assert!(first.is_ok());
            tokio::time::sleep(Duration::from_millis(20)).await;
            let next = child
                .request(json!({ "type": "two" }), CancellationToken::new())
                .await
                .unwrap_err();
            assert!(
                matches!(next, PiRpcError::Protocol(message) if message.contains("unknown or duplicate"))
            );
        } else {
            assert!(
                matches!(first, Err(PiRpcError::Protocol(message)) if message.contains("expected one"))
            );
        }
        child.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn readiness_timeout_and_exit_before_readiness_fail_launch() {
    let fixture = TempDir::new().unwrap();
    let mut policy = test_policy();
    policy.readiness_timeout = Duration::from_millis(40);
    assert!(matches!(
        PiRpcChild::launch(fixture_spec(&fixture, "never-ready"), policy).await,
        Err(PiRpcError::ReadinessTimedOut)
    ));

    let fixture = TempDir::new().unwrap();
    assert!(matches!(
        PiRpcChild::launch(fixture_spec(&fixture, "exit-before-readiness"), test_policy()).await,
        Err(PiRpcError::ReadinessFailed(message)) if message.contains("exited") || message.contains("EOF")
    ));

    let fixture = TempDir::new().unwrap();
    assert!(matches!(
        PiRpcChild::launch(fixture_spec(&fixture, "failed-readiness"), test_policy()).await,
        Err(PiRpcError::ReadinessFailed(message)) if message.contains("fixture refused readiness")
    ));
}

#[tokio::test]
async fn child_exit_rejects_pending_requests() {
    let fixture = TempDir::new().unwrap();
    let child = PiRpcChild::launch(fixture_spec(&fixture, "exit-pending"), test_policy())
        .await
        .unwrap();

    let error = child
        .request(json!({ "type": "wait" }), CancellationToken::new())
        .await
        .unwrap_err();

    assert!(matches!(
        error,
        PiRpcError::Exited { code: Some(23), .. } | PiRpcError::Protocol(_)
    ));
    child.shutdown().await.unwrap();
}

#[tokio::test]
async fn stderr_is_drained_continuously_and_only_a_bounded_tail_is_retained() {
    let fixture = TempDir::new().unwrap();
    let mut policy = test_policy();
    policy.retained_stderr_bytes = 1024;
    let child = PiRpcChild::launch(fixture_spec(&fixture, "stderr-burst"), policy)
        .await
        .unwrap();

    child
        .request(json!({ "type": "after_stderr" }), CancellationToken::new())
        .await
        .expect("stderr burst must not block protocol progress");
    wait_until(|| child.diagnostics().stderr_was_truncated).await;
    let diagnostics = child.diagnostics();

    assert!(diagnostics.stderr_was_truncated);
    assert_eq!(diagnostics.stderr.len(), 1024);
    assert!(diagnostics.stderr.bytes().all(|byte| byte == b'x'));
    child.shutdown().await.unwrap();
}

#[tokio::test]
async fn bounded_event_delivery_reports_backpressure_without_corrupting_responses() {
    let fixture = TempDir::new().unwrap();
    let mut policy = test_policy();
    policy.event_queue_capacity = 1;
    let child = PiRpcChild::launch(fixture_spec(&fixture, "event-backpressure"), policy)
        .await
        .unwrap();
    let mut events = child.subscribe();

    child
        .request(json!({ "type": "emit" }), CancellationToken::new())
        .await
        .expect("response should survive a lagging event consumer");

    assert!(matches!(
        events.recv().await,
        Err(PiRpcError::EventBackpressure { missed }) if missed >= 1
    ));
    child.shutdown().await.unwrap();
}

#[tokio::test]
async fn cancellation_remains_responsive_while_the_bounded_writer_is_backpressured() {
    let fixture = TempDir::new().unwrap();
    let mut policy = test_policy();
    policy.request_timeout = Duration::from_secs(5);
    policy.write_queue_capacity = 1;
    let child = PiRpcChild::launch(fixture_spec(&fixture, "write-backpressure"), policy)
        .await
        .unwrap();
    let payload = "x".repeat(1024 * 1024);
    let first_cancel = CancellationToken::new();
    let second_cancel = CancellationToken::new();
    let third_cancel = CancellationToken::new();
    let cancel_third = third_cancel.clone();
    let started = Instant::now();

    let first = child.request(
        json!({ "type": "large_one", "payload": payload }),
        first_cancel,
    );
    let second = child.request(
        json!({ "type": "large_two", "payload": "y".repeat(1024 * 1024) }),
        second_cancel,
    );
    let third = async {
        let started = Instant::now();
        let result = child
            .request(
                json!({ "type": "large_three", "payload": "z".repeat(1024 * 1024) }),
                third_cancel,
            )
            .await;
        (started.elapsed(), result)
    };
    let cancel = async move {
        tokio::time::sleep(Duration::from_millis(30)).await;
        cancel_third.cancel();
    };
    let (_, _, (third_elapsed, third), ()) = tokio::join!(first, second, third, cancel);

    assert!(matches!(third, Err(PiRpcError::RequestCancelled { .. })));
    assert!(third_elapsed < Duration::from_millis(250));
    assert!(started.elapsed() < Duration::from_secs(7));
    child.shutdown().await.unwrap();
}

#[tokio::test]
async fn graceful_and_forced_shutdown_both_terminate_the_owned_process_group() {
    for mode in ["graceful-descendant", "forced-shutdown"] {
        let fixture = TempDir::new().unwrap();
        let mut policy = test_policy();
        policy.graceful_shutdown_timeout = Duration::from_millis(50);
        let child = PiRpcChild::launch(fixture_spec(&fixture, mode), policy)
            .await
            .unwrap();
        let parent = fixture.path().join("parent.pid");
        let descendant = fixture.path().join("descendant.pid");
        wait_until(|| parent.exists() && descendant.exists()).await;
        assert!(process_exists(&parent));
        assert!(process_exists(&descendant));

        child.shutdown().await.unwrap();
        wait_until(|| !process_exists(&parent) && !process_exists(&descendant)).await;

        assert!(!process_exists(&parent), "{mode} parent survived shutdown");
        assert!(
            !process_exists(&descendant),
            "{mode} descendant survived shutdown"
        );
    }
}

#[tokio::test]
async fn caller_supplied_ids_and_non_absolute_process_paths_are_rejected() {
    let fixture = TempDir::new().unwrap();
    let child = PiRpcChild::launch(fixture_spec(&fixture, "normal"), test_policy())
        .await
        .unwrap();
    assert!(matches!(
        child
            .request(
                json!({ "id": "caller", "type": "prompt" }),
                CancellationToken::new()
            )
            .await,
        Err(PiRpcError::InvalidCommand(_))
    ));
    child.shutdown().await.unwrap();

    let mut invalid = fixture_spec(&fixture, "normal");
    invalid.executable = PathBuf::from("zsh");
    assert!(matches!(
        PiRpcChild::launch(invalid, test_policy()).await,
        Err(PiRpcError::NonAbsoluteProcessPath)
    ));
}
