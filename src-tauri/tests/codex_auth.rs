use std::{
    collections::BTreeMap,
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

use piu_lib::codex_auth::{
    CodexAuthError, CodexAuthEvent, CodexAuthManager, CodexAuthPolicy, CodexAuthProcessSpec,
    CodexAuthPrompt, CodexAuthStatus, CodexAuthUpdate,
};
use tempfile::TempDir;

fn fixture_manager(fixture: &TempDir, mode: &str) -> CodexAuthManager {
    fixture_manager_with_policy(fixture, mode, test_policy())
}

fn test_policy() -> CodexAuthPolicy {
    CodexAuthPolicy {
        operation_timeout: Duration::from_secs(2),
        graceful_shutdown_timeout: Duration::from_millis(200),
        maximum_record_bytes: 64 * 1024,
        maximum_response_bytes: 16 * 1024,
        write_queue_capacity: 4,
        update_queue_capacity: 32,
    }
}

fn fixture_manager_with_policy(
    fixture: &TempDir,
    mode: &str,
    policy: CodexAuthPolicy,
) -> CodexAuthManager {
    let script =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/codex-auth-child.zsh");
    let mut environment = BTreeMap::new();
    environment.insert(
        OsString::from("HOME"),
        OsString::from("/Users/piu-auth-test"),
    );
    environment.insert(OsString::from("PATH"), OsString::from("/usr/bin:/bin"));
    environment.insert(
        OsString::from("PIU_AUTH_FIXTURE_MODE"),
        OsString::from(mode),
    );
    environment.insert(
        OsString::from("PIU_AUTH_FIXTURE_RECORD_DIR"),
        fixture.path().as_os_str().to_owned(),
    );
    let spec = CodexAuthProcessSpec {
        executable: PathBuf::from("/bin/zsh"),
        arguments: vec![
            script.into_os_string(),
            OsString::from("--credential-lock-dir"),
            fixture.path().join("credential-locks").into_os_string(),
        ],
        working_directory: fixture.path().to_path_buf(),
        environment,
    };
    CodexAuthManager::new(spec, policy).expect("fixture policy should be valid")
}

#[tokio::test]
async fn graphical_sign_in_relays_typed_updates_and_correlates_the_prompt_answer() {
    let fixture = TempDir::new().expect("fixture directory");
    let manager = fixture_manager(&fixture, "browser-success");
    let mut updates = manager.subscribe();

    manager.start().await.expect("helper should start");
    assert_eq!(manager.status(), CodexAuthStatus::SigningIn);

    assert_eq!(
        updates.recv().await.unwrap(),
        CodexAuthUpdate::Event {
            event: CodexAuthEvent::Info {
                message: "Choose a sign-in method".into(),
                links: vec![],
            },
        }
    );
    assert_eq!(
        updates.recv().await.unwrap(),
        CodexAuthUpdate::Prompt {
            id: "auth-1".into(),
            prompt: CodexAuthPrompt::Select {
                message: "Sign in using".into(),
                options: vec![piu_lib::codex_auth::CodexAuthOption {
                    id: "browser".into(),
                    label: "Browser".into(),
                    description: Some("Recommended".into()),
                }],
            },
        }
    );
    assert_eq!(
        manager.status(),
        CodexAuthStatus::WaitingForInput {
            prompt_id: "auth-1".into()
        }
    );

    manager
        .answer("auth-1", "browser")
        .await
        .expect("matching answer should be accepted");
    assert_eq!(updates.recv().await.unwrap(), CodexAuthUpdate::Complete);
    manager
        .wait_until_idle()
        .await
        .expect("completed helper should exit");
    assert_eq!(manager.status(), CodexAuthStatus::SignedIn);

    assert_eq!(
        std::fs::read_to_string(fixture.path().join("home")).unwrap(),
        "/Users/piu-auth-test\n"
    );
    assert_eq!(
        std::fs::read_to_string(fixture.path().join("cwd"))
            .unwrap()
            .trim(),
        fixture.path().canonicalize().unwrap().to_string_lossy()
    );
    let command = std::fs::read_to_string(fixture.path().join("command")).unwrap();
    assert!(command.contains(r#""type":"auth_prompt_response""#));
    assert!(command.contains(r#""id":"auth-1""#));
    assert!(command.contains(r#""value":"browser""#));
}

#[tokio::test]
async fn every_public_event_and_prompt_variant_crosses_the_typed_seam() {
    let fixture = TempDir::new().unwrap();
    let manager = fixture_manager(&fixture, "all-variants");
    let mut updates = manager.subscribe();
    manager.start().await.unwrap();

    assert_eq!(
        updates.recv().await.unwrap(),
        CodexAuthUpdate::Event {
            event: CodexAuthEvent::Info {
                message: "Read the provider help".into(),
                links: vec![piu_lib::codex_auth::CodexAuthLink {
                    url: "https://example.test/help".into(),
                    label: Some("Help".into()),
                }],
            }
        }
    );
    assert_eq!(
        updates.recv().await.unwrap(),
        CodexAuthUpdate::Event {
            event: CodexAuthEvent::AuthUrl {
                url: "https://example.test/auth".into(),
                instructions: Some("Continue in the browser".into()),
            }
        }
    );
    assert_eq!(
        updates.recv().await.unwrap(),
        CodexAuthUpdate::Event {
            event: CodexAuthEvent::DeviceCode {
                user_code: "ABCD-EFGH".into(),
                verification_uri: "https://example.test/device".into(),
                interval_seconds: Some(5.0),
                expires_in_seconds: Some(900.0),
            }
        }
    );
    assert_eq!(
        updates.recv().await.unwrap(),
        CodexAuthUpdate::Event {
            event: CodexAuthEvent::Progress {
                message: "Waiting for authorization".into(),
            }
        }
    );

    assert_eq!(
        updates.recv().await.unwrap(),
        CodexAuthUpdate::Prompt {
            id: "auth-text".into(),
            prompt: CodexAuthPrompt::Text {
                message: "Organization".into(),
                placeholder: Some("Example, Inc.".into()),
            }
        }
    );
    manager.answer("auth-text", "piu").await.unwrap();
    assert_eq!(
        updates.recv().await.unwrap(),
        CodexAuthUpdate::Prompt {
            id: "auth-secret".into(),
            prompt: CodexAuthPrompt::Secret {
                message: "One-time secret".into(),
                placeholder: None,
            }
        }
    );
    manager.answer("auth-secret", "one-time").await.unwrap();
    assert_eq!(
        updates.recv().await.unwrap(),
        CodexAuthUpdate::Prompt {
            id: "auth-manual".into(),
            prompt: CodexAuthPrompt::ManualCode {
                message: "Paste the callback code".into(),
                placeholder: Some("code".into()),
            }
        }
    );
    manager
        .answer("auth-manual", "callback-code")
        .await
        .unwrap();

    assert_eq!(updates.recv().await.unwrap(), CodexAuthUpdate::Complete);
    manager.wait_until_idle().await.unwrap();
}

#[tokio::test]
async fn provider_cancelled_prompt_retires_its_id_before_a_raced_ui_answer() {
    let fixture = TempDir::new().unwrap();
    let manager = fixture_manager(&fixture, "provider-cancelled-prompt");
    let mut updates = manager.subscribe();
    manager.start().await.unwrap();

    assert!(matches!(
        updates.recv().await.unwrap(),
        CodexAuthUpdate::Prompt { ref id, .. } if id == "auth-race"
    ));
    assert_eq!(
        updates.recv().await.unwrap(),
        CodexAuthUpdate::PromptCancelled {
            id: "auth-race".into()
        }
    );
    assert_eq!(
        manager.answer("auth-race", "late-code").await,
        Err(CodexAuthError::PromptNotPending)
    );
    assert_eq!(updates.recv().await.unwrap(), CodexAuthUpdate::Complete);
    manager.wait_until_idle().await.unwrap();
}

#[tokio::test]
async fn user_cancellation_is_correlated_and_finishes_the_short_lived_helper() {
    let fixture = TempDir::new().unwrap();
    let manager = fixture_manager(&fixture, "user-cancel");
    let mut updates = manager.subscribe();
    manager.start().await.unwrap();
    assert!(matches!(
        updates.recv().await.unwrap(),
        CodexAuthUpdate::Prompt { .. }
    ));

    manager.cancel().await.unwrap();
    assert_eq!(manager.status(), CodexAuthStatus::Cancelling);
    assert_eq!(
        updates.recv().await.unwrap(),
        CodexAuthUpdate::PromptCancelled {
            id: "auth-cancel".into()
        }
    );
    assert_eq!(updates.recv().await.unwrap(), CodexAuthUpdate::Cancelled);
    manager.wait_until_idle().await.unwrap();
    assert_eq!(manager.status(), CodexAuthStatus::Cancelled);
    assert_eq!(
        std::fs::read_to_string(fixture.path().join("command"))
            .unwrap()
            .trim(),
        r#"{"type":"auth_cancel"}"#
    );
}

#[tokio::test]
async fn provider_failures_and_stderr_are_replaced_with_a_fixed_safe_error() {
    let fixture = TempDir::new().unwrap();
    let manager = fixture_manager(&fixture, "provider-failure");
    let mut updates = manager.subscribe();
    manager.start().await.unwrap();

    let update = updates.recv().await.unwrap();
    assert_eq!(
        update,
        CodexAuthUpdate::Failed {
            code: "sign_in_failed".into(),
            message: "Sign-in failed. Try again.".into(),
        }
    );
    manager.wait_until_idle().await.unwrap();
    let serialized = serde_json::to_string(&(update, manager.status())).unwrap();
    assert!(!serialized.contains("sensitive"));
    assert!(!serialized.contains("refresh-token"));
}

#[tokio::test]
async fn malformed_unknown_extra_and_oversized_records_fail_closed() {
    for mode in ["malformed", "unknown", "extra-field", "oversized"] {
        let fixture = TempDir::new().unwrap();
        let manager = fixture_manager(&fixture, mode);
        let mut updates = manager.subscribe();
        manager.start().await.unwrap();

        assert_eq!(
            updates.recv().await.unwrap(),
            CodexAuthUpdate::Failed {
                code: "sign_in_failed".into(),
                message: "Sign-in failed. Try again.".into(),
            },
            "fixture mode {mode}"
        );
        manager.wait_until_idle().await.unwrap();
    }
}

fn process_exists(path: &Path) -> bool {
    let pid = std::fs::read_to_string(path).expect("fixture should record process id");
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
async fn protocol_failure_kills_the_helper_and_its_descendants() {
    let fixture = TempDir::new().unwrap();
    let manager = fixture_manager(&fixture, "malformed-descendant");
    let mut updates = manager.subscribe();
    manager.start().await.unwrap();
    let parent = fixture.path().join("parent.pid");
    let descendant = fixture.path().join("descendant.pid");
    wait_until(|| parent.exists() && descendant.exists()).await;

    assert!(matches!(
        updates.recv().await.unwrap(),
        CodexAuthUpdate::Failed { .. }
    ));
    manager.wait_until_idle().await.unwrap();
    wait_until(|| !process_exists(&parent) && !process_exists(&descendant)).await;
    assert!(!process_exists(&parent));
    assert!(!process_exists(&descendant));
}

#[tokio::test]
async fn incremental_utf8_crlf_and_a_final_unterminated_record_are_framed_correctly() {
    let fixture = TempDir::new().unwrap();
    let manager = fixture_manager(&fixture, "framing");
    let mut updates = manager.subscribe();
    manager.start().await.unwrap();

    assert_eq!(
        updates.recv().await.unwrap(),
        CodexAuthUpdate::Event {
            event: CodexAuthEvent::Progress {
                message: "Waiting in Zürich".into(),
            }
        }
    );
    assert_eq!(updates.recv().await.unwrap(), CodexAuthUpdate::Complete);
    manager.wait_until_idle().await.unwrap();
    assert_eq!(manager.status(), CodexAuthStatus::SignedIn);
}

#[test]
fn production_spec_uses_only_absolute_bundled_paths_and_the_real_home() {
    let spec = CodexAuthProcessSpec::from_bundled_runtime(
        Path::new("/Applications/Più.app/Contents/Resources"),
        Path::new("/Users/emin/Library/Application Support/ch.emin.piu"),
        Path::new("/Users/emin"),
    );

    assert_eq!(
        spec.executable,
        PathBuf::from("/Applications/Più.app/Contents/Resources/agent-runtime/node/bin/node")
    );
    assert_eq!(
        spec.arguments,
        vec![
            OsString::from(
                "/Applications/Più.app/Contents/Resources/agent-runtime/pi/launcher/auth-launcher.mjs"
            ),
            OsString::from("--credential-lock-dir"),
            OsString::from("/Users/emin/Library/Application Support/ch.emin.piu/credential-locks"),
        ]
    );
    assert_eq!(
        spec.working_directory,
        PathBuf::from("/Applications/Più.app/Contents/Resources/agent-runtime/pi")
    );
    assert_eq!(spec.environment.len(), 4);
    assert_eq!(
        spec.environment.get(OsStr::new("HOME")),
        Some(&OsString::from("/Users/emin"))
    );
    assert_eq!(
        spec.environment.get(OsStr::new("PATH")),
        Some(&OsString::from("/usr/bin:/bin"))
    );
}

#[tokio::test]
async fn only_one_authentication_helper_can_run_at_a_time() {
    let fixture = TempDir::new().unwrap();
    let manager = fixture_manager(&fixture, "user-cancel");
    let other = manager.clone();
    let (first, second) = tokio::join!(manager.start(), other.start());
    assert!(
        matches!(
            (first, second),
            (Ok(()), Err(CodexAuthError::AlreadyRunning))
                | (Err(CodexAuthError::AlreadyRunning), Ok(()))
        ),
        "exactly one concurrent start should win"
    );
    manager.cancel().await.unwrap();
    manager.wait_until_idle().await.unwrap();
}

#[tokio::test]
async fn shutdown_forces_down_an_unresponsive_helper_and_its_descendants() {
    let fixture = TempDir::new().unwrap();
    let manager = fixture_manager(&fixture, "ignore-cancel-descendant");
    let mut updates = manager.subscribe();
    manager.start().await.unwrap();
    let parent = fixture.path().join("parent.pid");
    let descendant = fixture.path().join("descendant.pid");
    wait_until(|| parent.exists() && descendant.exists()).await;

    manager.shutdown().await.unwrap();
    assert_eq!(updates.recv().await.unwrap(), CodexAuthUpdate::Cancelled);
    manager.wait_until_idle().await.unwrap();
    wait_until(|| !process_exists(&parent) && !process_exists(&descendant)).await;

    assert_eq!(manager.status(), CodexAuthStatus::Cancelled);
    assert!(!process_exists(&parent));
    assert!(!process_exists(&descendant));
}

#[tokio::test]
async fn operation_timeout_is_bounded_and_sanitized() {
    let fixture = TempDir::new().unwrap();
    let mut policy = test_policy();
    policy.operation_timeout = Duration::from_millis(50);
    let manager = fixture_manager_with_policy(&fixture, "hang", policy);
    let mut updates = manager.subscribe();
    manager.start().await.unwrap();

    assert_eq!(
        updates.recv().await.unwrap(),
        CodexAuthUpdate::Failed {
            code: "sign_in_timed_out".into(),
            message: "Sign-in timed out. Try again.".into(),
        }
    );
    manager.wait_until_idle().await.unwrap();
}

#[tokio::test]
async fn completion_is_published_only_after_a_clean_helper_exit() {
    for mode in [
        "record-after-complete",
        "complete-with-failed-exit",
        "complete-without-exit",
    ] {
        let fixture = TempDir::new().unwrap();
        let manager = fixture_manager(&fixture, mode);
        let mut updates = manager.subscribe();
        manager.start().await.unwrap();

        assert_eq!(
            updates.recv().await.unwrap(),
            CodexAuthUpdate::Failed {
                code: "sign_in_failed".into(),
                message: "Sign-in failed. Try again.".into(),
            },
            "fixture mode {mode}"
        );
        manager.wait_until_idle().await.unwrap();
        assert!(matches!(manager.status(), CodexAuthStatus::Failed { .. }));
    }
}
