#[allow(dead_code)]
mod support;

use std::{fs, os::unix::fs::symlink, sync::Arc};

use piu_lib::{
    chat_runtime_commands::{ConversationPromptRequest, ConversationStreamingBehavior},
    chat_runtime_host::{
        ChatRuntimeChangedEvent, ChatRuntimeHost, ChatRuntimeHostError, ConversationEvent,
        ConversationItem, ConversationPhase, ConversationSnapshot, ConversationToolStatus,
    },
    chat_workspaces::ChatWorkspaces,
    git_process::GitProcess,
    project_inbox::{ChatSetupPhase, ProjectInbox},
};
use support::{TemporaryAppData, TemporaryGitRemote};

const FIXTURE_EVENT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

struct ChatFixture {
    _app_data: TemporaryAppData,
    _remote: TemporaryGitRemote,
    host: ChatRuntimeHost,
    inbox: Arc<ProjectInbox>,
    workspaces: Arc<ChatWorkspaces>,
    resource_directory: std::path::PathBuf,
    chat_id: String,
    worktree: std::path::PathBuf,
}

impl ChatFixture {
    fn new(run_setup: bool) -> Self {
        Self::with_options(run_setup, true, "quiet")
    }

    fn with_skills(run_setup: bool, install_skills: bool) -> Self {
        Self::with_options(run_setup, install_skills, "quiet")
    }

    fn with_options(run_setup: bool, install_skills: bool, mode: &str) -> Self {
        let app_data = TemporaryAppData::new();
        let remote = TemporaryGitRemote::new();
        fs::write(remote.working_path().join("README.md"), "fixture\n").unwrap();
        remote.git(["add", "README.md"]);
        remote.git(["commit", "-m", "fixture"]);
        remote.git(["push", "-u", "origin", "main"]);

        let git = GitProcess::with_executable("/usr/bin/git".into());
        let inbox = Arc::new(
            ProjectInbox::with_git(&app_data.database_path(), git.clone()).expect("open inbox"),
        );
        let project = inbox
            .open_repository(remote.working_path())
            .expect("open repository")
            .project;
        let workspaces = Arc::new(ChatWorkspaces::new(
            Arc::clone(&inbox),
            git,
            app_data.path().join("worktrees"),
        ));
        let chat = workspaces
            .create_chat(project.id, "Inspect the runtime")
            .expect("create chat")
            .chat;
        if run_setup {
            let setup = workspaces
                .start_setup(&chat.id, Arc::new(|_| {}))
                .expect("finish missing setup");
            assert_eq!(setup.phase, ChatSetupPhase::NotRequired);
        }

        let resource_directory = app_data.path().join("resources");
        fs::create_dir_all(app_data.path().join("host-fixture")).unwrap();
        fs::write(app_data.path().join("host-fixture/mode"), mode).unwrap();
        let node = resource_directory.join("agent-runtime/node/bin/node");
        let launcher = resource_directory.join("agent-runtime/pi/launcher/chat-launcher.mjs");
        fs::create_dir_all(node.parent().unwrap()).unwrap();
        fs::create_dir_all(launcher.parent().unwrap()).unwrap();
        symlink("/bin/zsh", &node).unwrap();
        fs::copy(
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/chat-runtime-child.zsh"),
            &launcher,
        )
        .unwrap();
        let worktree = app_data.path().join("worktrees").join(&chat.id);
        if install_skills {
            fs::create_dir_all(resource_directory.join("agent-runtime/skills")).unwrap();
            fs::create_dir_all(worktree.join(".pi/skills")).unwrap();
        }
        let home = app_data.path().join("real-home");
        fs::create_dir(&home).unwrap();
        let host = ChatRuntimeHost::new(
            Arc::clone(&inbox),
            Arc::clone(&workspaces),
            app_data.path(),
            &resource_directory,
            &home,
        )
        .expect("create runtime host");
        Self {
            _app_data: app_data,
            _remote: remote,
            host,
            inbox,
            workspaces,
            resource_directory,
            worktree,
            chat_id: chat.id,
        }
    }

    fn record(&self, name: &str) -> String {
        fs::read_to_string(self._app_data.path().join("host-fixture").join(name)).unwrap()
    }

    fn create_chat(&self, prompt: &str) -> String {
        let project_id = self.inbox.snapshot().unwrap().projects.first().unwrap().id;
        let chat = self
            .workspaces
            .create_chat(project_id, prompt)
            .unwrap()
            .chat;
        let setup = self
            .workspaces
            .start_setup(&chat.id, Arc::new(|_| {}))
            .unwrap();
        assert_eq!(setup.phase, ChatSetupPhase::NotRequired);
        chat.id
    }

    fn live_children(&self) -> usize {
        fs::read_dir(self._app_data.path().join("host-fixture/live"))
            .map(|entries| entries.count())
            .unwrap_or_default()
    }

    async fn wait_for_live_children(&self, expected: usize) {
        tokio::time::timeout(FIXTURE_EVENT_TIMEOUT, async {
            while self.live_children() != expected {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap_or_else(|_| {
            panic!(
                "expected {expected} live Pi children, found {}",
                self.live_children()
            )
        });
    }
}

#[tokio::test]
async fn native_pi_events_project_to_one_authoritative_typed_conversation() {
    let fixture = ChatFixture::with_options(true, true, "streaming");
    let mut events = fixture.host.subscribe();

    fixture.host.open(&fixture.chat_id).await.unwrap();
    let mut received = Vec::new();
    loop {
        let changed = tokio::time::timeout(FIXTURE_EVENT_TIMEOUT, events.recv())
            .await
            .expect("conversation event should arrive")
            .expect("event subscription should remain live");
        assert_eq!(changed.chat_id, fixture.chat_id);
        let completed = matches!(changed.event, ConversationEvent::TurnCompleted);
        received.push(changed.event);
        if completed {
            break;
        }
    }

    assert!(received.iter().any(|event| matches!(
        event,
        ConversationEvent::ReasoningDelta { delta, .. } if delta == "I should inspect."
    )));
    assert!(received.iter().any(|event| matches!(
        event,
        ConversationEvent::TextDelta { delta, .. } if delta == "Done."
    )));
    assert!(received.iter().any(|event| matches!(
        event,
        ConversationEvent::ToolUpdate {
            status: ConversationToolStatus::Succeeded,
            detail,
            ..
        } if detail == "fixture contents"
    )));
    assert!(received.iter().any(|event| matches!(
        event,
        ConversationEvent::UsageUpdate {
            input_tokens: 12,
            output_tokens: 7,
            cache_read_tokens: Some(3),
            ..
        }
    )));

    fixture.wait_for_live_children(0).await;
    let launches_after_completion = fixture.record("launches").lines().count();
    let snapshot = fixture.host.open(&fixture.chat_id).await.unwrap();
    assert_eq!(snapshot.phase, ConversationPhase::Idle);
    assert_eq!(fixture.live_children(), 0);
    assert_eq!(
        fixture.record("launches").lines().count(),
        launches_after_completion
    );
    assert!(snapshot.items.iter().any(|item| matches!(
        item,
        ConversationItem::Message { text, .. } if text == "Done."
    )));
    assert!(snapshot.items.iter().any(|item| matches!(
        item,
        ConversationItem::Reasoning { text, .. } if text == "I should inspect."
    )));
    assert!(snapshot.items.iter().any(|item| matches!(
        item,
        ConversationItem::Tool {
            status: ConversationToolStatus::Succeeded,
            detail,
            ..
        } if detail == "fixture contents"
    )));

    let session = fixture
        .inbox
        .chat_session(&fixture.chat_id)
        .unwrap()
        .expect("completed chat keeps its exact session binding");
    fixture
        .host
        .send(&fixture.chat_id, "Continue after completion")
        .await
        .unwrap();
    assert_eq!(
        fixture
            .record("commands")
            .matches("\"type\":\"prompt\"")
            .count(),
        2
    );
    assert_eq!(fixture.record("launches").lines().count(), 2);
    assert_eq!(
        fixture.inbox.chat_session(&fixture.chat_id).unwrap(),
        Some(session)
    );
    loop {
        let changed = tokio::time::timeout(FIXTURE_EVENT_TIMEOUT, events.recv())
            .await
            .expect("resumed turn should complete")
            .expect("event subscription should remain live");
        if matches!(changed.event, ConversationEvent::TurnCompleted) {
            break;
        }
    }
    fixture.wait_for_live_children(0).await;
    fixture.host.shutdown_all().await;
}

#[test]
fn conversation_wire_types_match_the_frontend_contract() {
    let snapshot = ConversationSnapshot {
        failure: None,
        items: vec![ConversationItem::Usage {
            cache_read_tokens: Some(3),
            id: "message-1-usage".into(),
            input_tokens: 12,
            output_tokens: 7,
        }],
        phase: ConversationPhase::Running,
    };
    assert_eq!(
        serde_json::to_value(snapshot).unwrap(),
        serde_json::json!({
            "failure": null,
            "items": [{
                "kind": "usage",
                "cacheReadTokens": 3,
                "id": "message-1-usage",
                "inputTokens": 12,
                "outputTokens": 7
            }],
            "phase": "running"
        })
    );

    let changed = ChatRuntimeChangedEvent {
        chat_id: "chat-1".into(),
        event: ConversationEvent::ToolUpdate {
            detail: "README.md".into(),
            item_id: "tool-call-1".into(),
            status: ConversationToolStatus::Succeeded,
        },
    };
    assert_eq!(
        serde_json::to_value(changed).unwrap(),
        serde_json::json!({
            "chatId": "chat-1",
            "event": {
                "type": "tool-update",
                "detail": "README.md",
                "itemId": "tool-call-1",
                "status": "succeeded"
            }
        })
    );

    let prompt: ConversationPromptRequest = serde_json::from_value(serde_json::json!({
        "chatId": "chat-1",
        "streamingBehavior": "steer",
        "text": "Continue"
    }))
    .unwrap();
    assert_eq!(prompt.chat_id, "chat-1");
    assert_eq!(
        prompt.streaming_behavior,
        ConversationStreamingBehavior::Steer
    );
    assert_eq!(prompt.text, "Continue");
}

#[tokio::test]
async fn nonexistent_skill_directories_are_not_passed_to_pi() {
    let fixture = ChatFixture::with_skills(true, false);

    fixture.host.open(&fixture.chat_id).await.unwrap();

    assert!(!fixture.record("arguments").contains("--skill"));
    fixture.host.shutdown_all().await;
}

#[tokio::test]
async fn terminal_chat_retirement_preserves_a_running_background_chat() {
    let fixture = ChatFixture::new(true);
    let second_chat = fixture.create_chat("Inspect the second runtime");

    fixture.host.open(&fixture.chat_id).await.unwrap();
    assert_eq!(fixture.live_children(), 1);

    fixture.host.open(&second_chat).await.unwrap();
    assert_eq!(fixture.live_children(), 2);
    assert_eq!(
        fixture.host.snapshot(&fixture.chat_id).unwrap().phase,
        ConversationPhase::Running
    );
    assert_eq!(
        fixture.host.snapshot(&second_chat).unwrap().phase,
        ConversationPhase::Running
    );
    let first_session = fixture
        .inbox
        .chat_session(&fixture.chat_id)
        .unwrap()
        .expect("first chat should have an exact session");

    fixture.host.abort(&fixture.chat_id).await.unwrap();
    fixture
        .host
        .steer(&second_chat, "Keep working independently")
        .await
        .unwrap();
    fixture.wait_for_live_children(1).await;
    assert_eq!(
        fixture.host.snapshot(&fixture.chat_id).unwrap().phase,
        ConversationPhase::Stopped
    );
    assert_eq!(
        fixture.host.snapshot(&second_chat).unwrap().phase,
        ConversationPhase::Running
    );
    assert!(
        fixture
            .record(&format!("commands-pi-{}", fixture.chat_id))
            .contains("\"type\":\"abort\"")
    );
    assert!(
        !fixture
            .record(&format!("commands-pi-{second_chat}"))
            .contains("\"type\":\"abort\"")
    );
    assert!(
        fixture
            .record(&format!("commands-pi-{second_chat}"))
            .contains("\"type\":\"steer\"")
    );

    let stopped = fixture.host.open(&fixture.chat_id).await.unwrap();
    assert_eq!(stopped.phase, ConversationPhase::Stopped);
    assert_eq!(fixture.live_children(), 1);
    assert_eq!(fixture.record("launches").lines().count(), 2);

    fixture
        .host
        .send(&fixture.chat_id, "Resume the first runtime")
        .await
        .unwrap();
    assert_eq!(fixture.live_children(), 2);
    assert_eq!(fixture.record("launches").lines().count(), 3);
    assert_eq!(
        fixture.inbox.chat_session(&fixture.chat_id).unwrap(),
        Some(first_session)
    );
    fixture.host.abort(&fixture.chat_id).await.unwrap();
    fixture.wait_for_live_children(1).await;
    assert_eq!(
        fixture.host.snapshot(&fixture.chat_id).unwrap().phase,
        ConversationPhase::Stopped
    );
    assert_eq!(
        fixture.host.snapshot(&second_chat).unwrap().phase,
        ConversationPhase::Running
    );

    fixture
        .host
        .send(&second_chat, "Finish the independent check")
        .await
        .unwrap();
    assert_eq!(fixture.live_children(), 1);
    assert_eq!(fixture.record("launches").lines().count(), 3);
    assert_eq!(
        fixture
            .record("commands")
            .matches("\"type\":\"prompt\"")
            .count(),
        4
    );
    fixture.host.shutdown_all().await;
}

#[tokio::test]
async fn send_steer_abort_and_stop_preserve_the_resumable_chat() {
    let fixture = ChatFixture::new(true);
    let mut events = fixture.host.subscribe();
    assert!(!fixture.host.has_active_turn().unwrap());
    fixture.host.open(&fixture.chat_id).await.unwrap();
    assert!(fixture.host.has_active_turn().unwrap());

    fixture.host.abort(&fixture.chat_id).await.unwrap();
    fixture.wait_for_live_children(0).await;
    assert!(!fixture.host.has_active_turn().unwrap());
    assert_eq!(
        fixture.host.snapshot(&fixture.chat_id).unwrap().phase,
        ConversationPhase::Stopped
    );
    assert!(matches!(
        events.recv().await.unwrap().event,
        ConversationEvent::TurnStopped
    ));

    fixture
        .host
        .send(&fixture.chat_id, "Continue from here")
        .await
        .unwrap();
    assert!(fixture.host.has_active_turn().unwrap());
    fixture
        .host
        .steer(&fixture.chat_id, "Check the tests too")
        .await
        .unwrap();
    let active = fixture.host.snapshot(&fixture.chat_id).unwrap();
    assert_eq!(active.phase, ConversationPhase::Running);
    assert!(active.items.iter().any(|item| matches!(
        item,
        ConversationItem::Message {
            role: piu_lib::chat_runtime_host::ConversationRole::User,
            text,
            ..
        } if text == "Continue from here"
    )));

    let commands = fixture.record("commands");
    assert_eq!(commands.matches("\"type\":\"abort\"").count(), 1);
    assert_eq!(commands.matches("\"type\":\"steer\"").count(), 1);
    assert_eq!(commands.matches("\"type\":\"prompt\"").count(), 2);
    assert!(
        commands.contains("\"message\":\"Continue from here\",\"streamingBehavior\":\"steer\"")
    );
    assert_eq!(fixture.record("launches").lines().count(), 2);

    fixture.host.stop_runtime(&fixture.chat_id).await.unwrap();
    assert!(!fixture.host.has_active_turn().unwrap());
    assert_eq!(fixture.live_children(), 0);
    assert_eq!(
        fixture.host.snapshot(&fixture.chat_id).unwrap().phase,
        ConversationPhase::Stopped
    );
}

#[tokio::test]
async fn repeated_user_text_remains_two_distinct_turns() {
    let fixture = ChatFixture::with_options(true, true, "repeated-events");
    let mut events = fixture.host.subscribe();
    fixture.host.open(&fixture.chat_id).await.unwrap();
    for _ in 0..2 {
        assert!(matches!(
            events.recv().await.unwrap().event,
            ConversationEvent::ItemAdded {
                item: ConversationItem::Message { .. }
            }
        ));
    }

    let repeated = fixture
        .host
        .snapshot(&fixture.chat_id)
        .unwrap()
        .items
        .into_iter()
        .filter(|item| {
            matches!(
                item,
                ConversationItem::Message {
                    role: piu_lib::chat_runtime_host::ConversationRole::User,
                    text,
                    ..
                } if text == "Repeat this"
            )
        })
        .count();
    assert_eq!(repeated, 2);
    fixture.host.shutdown_all().await;
}

#[tokio::test]
async fn durable_empty_session_binding_prevents_initial_prompt_replay_after_child_crash() {
    let fixture = ChatFixture::with_options(true, true, "crash-before-prompt");

    let first_error = fixture.host.open(&fixture.chat_id).await.unwrap_err();
    assert!(
        matches!(&first_error, ChatRuntimeHostError::Rpc(_)),
        "unexpected error: {first_error:?}"
    );
    assert_eq!(
        fixture.host.snapshot(&fixture.chat_id).unwrap().phase,
        ConversationPhase::Failed
    );
    let bound_before_retry = fixture
        .inbox
        .chat_session(&fixture.chat_id)
        .unwrap()
        .expect("readiness binds the empty exact session before prompt acceptance");
    assert!(bound_before_retry.path.exists());

    let cached = fixture.host.open(&fixture.chat_id).await.unwrap();
    assert_eq!(cached.phase, ConversationPhase::Failed);
    assert!(cached.items.iter().any(|item| matches!(
        item,
        ConversationItem::Message { text, .. } if text == "Inspect the runtime"
    )));
    assert_eq!(
        fixture.host.snapshot(&fixture.chat_id).unwrap().phase,
        ConversationPhase::Failed,
        "viewing the chat must retain the authoritative failure projection"
    );
    assert_eq!(fixture.live_children(), 0);
    assert_eq!(fixture.record("launches").lines().count(), 1);
    assert_eq!(
        fixture
            .record("commands")
            .matches("\"type\":\"prompt\"")
            .count(),
        1,
        "the durable session binding is the at-most-once initial dispatch marker"
    );
    assert_eq!(
        fixture.inbox.chat_session(&fixture.chat_id).unwrap(),
        Some(bound_before_retry.clone())
    );

    fixture
        .host
        .send(&fixture.chat_id, "Continue after the failed launch")
        .await
        .unwrap();
    assert_eq!(
        fixture.host.snapshot(&fixture.chat_id).unwrap().phase,
        ConversationPhase::Running
    );
    assert_eq!(fixture.live_children(), 1);
    assert_eq!(fixture.record("launches").lines().count(), 2);
    assert_eq!(
        fixture.inbox.chat_session(&fixture.chat_id).unwrap(),
        Some(bound_before_retry)
    );
    assert_eq!(
        fixture
            .record("commands")
            .matches("\"type\":\"prompt\"")
            .count(),
        2,
        "a normal send resumes the exact session without replaying the initial prompt"
    );
    fixture.host.shutdown_all().await;
}

#[tokio::test]
async fn accepted_prompt_failure_is_cached_without_an_idle_relaunch() {
    let fixture = ChatFixture::with_options(true, true, "crash-after-prompt");
    let mut events = fixture.host.subscribe();

    fixture.host.open(&fixture.chat_id).await.unwrap();
    let failure = tokio::time::timeout(FIXTURE_EVENT_TIMEOUT, async {
        loop {
            let changed = events.recv().await.unwrap();
            if matches!(changed.event, ConversationEvent::TurnFailed { .. }) {
                break changed;
            }
        }
    })
    .await
    .expect("the supervised child exit should become a typed failure event");
    assert_eq!(failure.chat_id, fixture.chat_id);
    assert_eq!(
        fixture.host.snapshot(&fixture.chat_id).unwrap().phase,
        ConversationPhase::Failed
    );
    assert_eq!(fixture.live_children(), 0);

    let cached = fixture.host.open(&fixture.chat_id).await.unwrap();
    assert_eq!(cached.phase, ConversationPhase::Failed);
    assert_eq!(fixture.live_children(), 0);
    assert_eq!(fixture.record("launches").lines().count(), 1);
    assert_eq!(
        fixture
            .record("commands")
            .matches("\"type\":\"prompt\"")
            .count(),
        1
    );
    fixture.host.shutdown_all().await;
}

#[tokio::test]
async fn chat_launch_waits_for_setup_and_owns_one_exact_isolated_child() {
    let pending = ChatFixture::new(false);
    let setup_error = pending.host.open(&pending.chat_id).await.unwrap_err();
    assert!(
        matches!(
            &setup_error,
            ChatRuntimeHostError::SetupIncomplete {
                phase: ChatSetupPhase::Pending,
                ..
            }
        ),
        "unexpected setup error: {setup_error:?}"
    );

    let fixture = ChatFixture::new(true);
    let first = fixture.host.open(&fixture.chat_id).await.unwrap();
    let second = fixture.host.open(&fixture.chat_id).await.unwrap();

    assert_eq!(first.phase, ConversationPhase::Running);
    assert_eq!(second, first);
    assert_eq!(fixture.record("launches").lines().count(), 1);
    assert_eq!(
        fixture
            .record("commands")
            .matches("\"type\":\"prompt\"")
            .count(),
        1
    );
    assert!(
        fixture
            .record("commands")
            .contains("\"streamingBehavior\":\"steer\"")
    );
    assert_eq!(
        fixture.record("cwd").trim(),
        fixture.worktree.canonicalize().unwrap().to_string_lossy()
    );
    assert_eq!(
        fixture.record("home").trim(),
        fixture._app_data.path().join("real-home").to_string_lossy()
    );
    assert_eq!(
        fixture.record("path").trim(),
        format!(
            "{}:/usr/bin:/bin",
            fixture.resource_directory.join("git/bin").display()
        )
    );
    assert_eq!(
        fixture.record("git-exec-path").trim(),
        fixture
            .resource_directory
            .join("git/libexec/git-core")
            .to_string_lossy()
    );
    assert_eq!(
        fixture.record("git-template-dir").trim(),
        fixture
            .resource_directory
            .join("git/share/git-core/templates")
            .to_string_lossy()
    );

    let arguments = fixture.record("arguments");
    let expected_node = fixture
        .resource_directory
        .join("agent-runtime/node/bin/node");
    let expected_launcher = fixture
        .resource_directory
        .join("agent-runtime/pi/launcher/chat-launcher.mjs");
    assert!(expected_node.is_absolute());
    assert_eq!(
        fixture.record("launcher").trim(),
        expected_launcher.to_string_lossy()
    );
    assert!(arguments.contains(&format!("--cwd\n{}\n", fixture.worktree.display())));
    assert!(arguments.contains(&format!(
        "--agent-dir\n{}\n",
        fixture._app_data.path().join("agent").display()
    )));
    assert!(arguments.contains(&format!(
        "--session-dir\n{}\n",
        fixture._app_data.path().join("sessions").display()
    )));
    assert!(arguments.contains(&format!(
        "--credential-lock-dir\n{}\n",
        fixture._app_data.path().join("credential-locks").display()
    )));
    assert!(arguments.contains("--model-provider\nopenai-codex\n"));
    assert!(arguments.contains("--model-id\ngpt-5.6-sol\n"));
    assert!(arguments.contains("--thinking-level\nxhigh\n"));
    assert!(arguments.contains(&format!(
        "--skill\n{}\n",
        fixture
            .resource_directory
            .join("agent-runtime/skills")
            .display()
    )));
    assert!(arguments.contains(&format!(
        "--skill\n{}\n",
        fixture.worktree.join(".pi/skills").display()
    )));

    let session = fixture
        .inbox
        .chat_session(&fixture.chat_id)
        .expect("read bound session")
        .expect("session should bind after readiness");
    assert_eq!(session.id, format!("pi-{}", fixture.chat_id));
    assert_eq!(
        session.path,
        fixture
            ._app_data
            .path()
            .join("sessions")
            .join(format!("pi-{}.jsonl", fixture.chat_id))
    );

    fixture.host.stop_runtime(&fixture.chat_id).await.unwrap();
    let resumed = fixture.host.open(&fixture.chat_id).await.unwrap();
    assert_eq!(resumed.phase, ConversationPhase::Stopped);
    assert_eq!(fixture.live_children(), 0);
    assert_eq!(fixture.record("launches").lines().count(), 1);
    assert_eq!(
        fixture
            .record("commands")
            .matches("\"type\":\"prompt\"")
            .count(),
        1
    );

    fixture.host.shutdown_all().await;
}
