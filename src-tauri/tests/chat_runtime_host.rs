#[allow(dead_code)]
mod support;

use std::{fs, os::unix::fs::symlink, sync::Arc};

use piu_lib::{
    chat_runtime_commands::{ConversationPromptRequest, ConversationStreamingBehavior},
    chat_runtime_host::{
        ChatRuntimeChangedEvent, ChatRuntimeHost, ChatRuntimeHostError, ConversationEvent,
        ConversationInputAnswer, ConversationInputKind, ConversationItem, ConversationPhase,
        ConversationSnapshot, ConversationToolStatus,
    },
    chat_workspaces::ChatWorkspaces,
    git_process::GitProcess,
    project_inbox::{ChatSetupPhase, ProjectInbox},
    prompt_attachments::{PromptAttachment, PromptAttachmentError, PromptAttachmentKind},
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
            .create_chat(project.id, "Inspect the runtime", &[])
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
        self.create_chat_with_attachments(prompt, &[])
    }

    fn create_chat_with_attachments(
        &self,
        prompt: &str,
        attachments: &[PromptAttachment],
    ) -> String {
        let project_id = self.inbox.snapshot().unwrap().projects.first().unwrap().id;
        let chat = self
            .workspaces
            .create_chat(project_id, prompt, attachments)
            .unwrap()
            .chat;
        let setup = self
            .workspaces
            .start_setup(&chat.id, Arc::new(|_| {}))
            .unwrap();
        assert_eq!(setup.phase, ChatSetupPhase::NotRequired);
        chat.id
    }

    fn fresh_host(&self) -> ChatRuntimeHost {
        ChatRuntimeHost::new(
            Arc::clone(&self.inbox),
            Arc::clone(&self.workspaces),
            self._app_data.path(),
            &self.resource_directory,
            &self._app_data.path().join("real-home"),
        )
        .expect("create fresh runtime host")
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

    async fn wait_for_record_contains(&self, name: &str, expected: &str) {
        tokio::time::timeout(FIXTURE_EVENT_TIMEOUT, async {
            loop {
                if fs::read_to_string(self._app_data.path().join("host-fixture").join(name))
                    .is_ok_and(|contents| contents.contains(expected))
                {
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("expected {name} to contain {expected}"));
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
        input_request: None,
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
            "inputRequest": null,
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

fn text_attachment() -> PromptAttachment {
    PromptAttachment {
        id: "attachment-notes".into(),
        name: "notes.txt".into(),
        kind: PromptAttachmentKind::Text,
        mime_type: "text/plain".into(),
        content: "Keep the public boundary".into(),
        size_bytes: 24,
    }
}

fn image_attachment() -> PromptAttachment {
    PromptAttachment {
        id: "attachment-view".into(),
        name: "view.png".into(),
        kind: PromptAttachmentKind::Image,
        mime_type: "image/png".into(),
        content: "iVBORw0KGgpmaXh0dXJl".into(),
        size_bytes: 15,
    }
}

#[tokio::test]
async fn first_prompt_attachments_are_restored_and_delivered_once() {
    let fixture = ChatFixture::new(true);
    let chat_id = fixture
        .create_chat_with_attachments("Inspect these", &[text_attachment(), image_attachment()]);

    fixture.host.open(&chat_id).await.unwrap();

    let commands = fixture.record(&format!("commands-pi-{chat_id}"));
    assert!(commands.contains("BEGIN ATTACHED TEXT FILE notes.txt [attachment-notes]"));
    assert!(commands.contains("\"images\":[{\"data\":\"iVBORw0KGgpmaXh0dXJl\",\"mimeType\":\"image/png\",\"type\":\"image\"}]"));
    assert_eq!(commands.matches("\"type\":\"prompt\"").count(), 1);
    fixture.host.stop_runtime(&chat_id).await.unwrap();
}

#[tokio::test]
async fn later_turn_attachments_use_delimited_text_and_pi_native_images() {
    let fixture = ChatFixture::new(true);
    fixture.host.open(&fixture.chat_id).await.unwrap();

    fixture
        .host
        .send_with_attachments(
            &fixture.chat_id,
            "Compare these",
            &[text_attachment(), image_attachment()],
        )
        .await
        .unwrap();

    let commands = fixture.record("commands");
    assert!(commands.contains("BEGIN ATTACHED TEXT FILE notes.txt [attachment-notes]"));
    assert!(commands.contains("\"images\":[{\"data\":\"iVBORw0KGgpmaXh0dXJl\",\"mimeType\":\"image/png\",\"type\":\"image\"}]"));
    assert_eq!(commands.matches("\"type\":\"prompt\"").count(), 2);
    fixture.host.stop_runtime(&fixture.chat_id).await.unwrap();
}

#[tokio::test]
async fn native_extension_ui_requests_pause_for_typed_user_input_and_can_be_cancelled() {
    let fixture = ChatFixture::with_options(true, true, "needs-input");
    let mut events = fixture.host.subscribe();
    fixture.host.open(&fixture.chat_id).await.unwrap();

    let request = tokio::time::timeout(FIXTURE_EVENT_TIMEOUT, async {
        loop {
            let changed = events.recv().await.unwrap();
            if let ConversationEvent::InputRequested { request } = changed.event {
                break request;
            }
        }
    })
    .await
    .expect("Pi's extension input should reach the host");
    assert_eq!(request.id, "extension-choice-1");
    assert_eq!(request.kind, ConversationInputKind::Select);
    assert_eq!(request.title, "Choose a strategy");
    assert_eq!(request.options, ["Keep both", "Replace"]);
    assert_eq!(
        fixture
            .host
            .snapshot(&fixture.chat_id)
            .unwrap()
            .input_request,
        Some(request.clone())
    );

    fixture
        .host
        .answer_input(
            &fixture.chat_id,
            &request.id,
            ConversationInputAnswer::Value {
                value: "Keep both".into(),
            },
        )
        .await
        .unwrap();
    assert!(matches!(
        events.recv().await.unwrap().event,
        ConversationEvent::InputResolved { ref request_id }
            if request_id == "extension-choice-1"
    ));
    assert!(
        fixture
            .host
            .snapshot(&fixture.chat_id)
            .unwrap()
            .input_request
            .is_none()
    );
    fixture
        .wait_for_record_contains(
            "extension-ui-responses",
            r#"{"id":"extension-choice-1","type":"extension_ui_response","value":"Keep both"}"#,
        )
        .await;

    let cancel_request = tokio::time::timeout(FIXTURE_EVENT_TIMEOUT, async {
        loop {
            let changed = events.recv().await.unwrap();
            if let ConversationEvent::InputRequested { request } = changed.event {
                break request;
            }
        }
    })
    .await
    .expect("the next extension input should reach the host");
    assert_eq!(cancel_request.kind, ConversationInputKind::Editor);
    fixture
        .host
        .answer_input(
            &fixture.chat_id,
            &cancel_request.id,
            ConversationInputAnswer::Cancelled,
        )
        .await
        .unwrap();
    fixture
        .wait_for_record_contains(
            "extension-ui-responses",
            r#"{"cancelled":true,"id":"extension-editor-2","type":"extension_ui_response"}"#,
        )
        .await;

    fixture.host.abort(&fixture.chat_id).await.unwrap();
}

#[tokio::test]
async fn image_send_is_typed_when_the_selected_model_has_no_image_input() {
    let fixture = ChatFixture::with_options(true, true, "text-only");
    fixture.host.open(&fixture.chat_id).await.unwrap();

    let error = fixture
        .host
        .send_with_attachments(&fixture.chat_id, "Inspect this", &[image_attachment()])
        .await
        .expect_err("text-only route must reject image input");

    assert!(matches!(
        error,
        ChatRuntimeHostError::Attachment(PromptAttachmentError::ModelMediaUnsupported)
    ));
    assert_eq!(
        fixture
            .record("commands")
            .matches("\"type\":\"prompt\"")
            .count(),
        1,
        "the unsupported turn must not reach Pi"
    );
    fixture.host.stop_runtime(&fixture.chat_id).await.unwrap();
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
async fn native_steering_queue_keeps_current_turn_output_before_the_accepted_message() {
    let fixture = ChatFixture::with_options(true, true, "steering-queue");
    let mut events = fixture.host.subscribe();
    fixture.host.open(&fixture.chat_id).await.unwrap();

    tokio::time::timeout(FIXTURE_EVENT_TIMEOUT, async {
        loop {
            let changed = events.recv().await.unwrap();
            if matches!(
                changed.event,
                ConversationEvent::TextDelta { ref delta, .. }
                    if delta == "Working on the current turn."
            ) {
                break;
            }
        }
    })
    .await
    .expect("the active turn should begin streaming");

    fixture
        .host
        .send(&fixture.chat_id, "Inspect the queued result")
        .await
        .unwrap();

    tokio::time::timeout(FIXTURE_EVENT_TIMEOUT, async {
        let mut saw_queue = false;
        let mut saw_tool = false;
        while !(saw_queue && saw_tool) {
            match events.recv().await.unwrap().event {
                ConversationEvent::MessageQueueChanged { queued: true, .. } => saw_queue = true,
                ConversationEvent::ItemAdded {
                    item: ConversationItem::Tool { ref id, .. },
                    ..
                } if id == "tool-call-queued" => saw_tool = true,
                _ => {}
            }
        }
    })
    .await
    .expect("the accepted steering message and later tool should both project");

    let snapshot = fixture.host.snapshot(&fixture.chat_id).unwrap();
    let assistant = snapshot
        .items
        .iter()
        .position(|item| matches!(item, ConversationItem::Message { text, .. } if text == "Working on the current turn."))
        .unwrap();
    let tool = snapshot
        .items
        .iter()
        .position(
            |item| matches!(item, ConversationItem::Tool { id, .. } if id == "tool-call-queued"),
        )
        .unwrap();
    let steering = snapshot
        .items
        .iter()
        .position(|item| matches!(item, ConversationItem::Message { text, queued: true, .. } if text == "Inspect the queued result"))
        .unwrap();
    assert!(assistant < tool && tool < steering);

    let commands = fixture.record("commands");
    assert!(commands.contains("\"type\":\"set_auto_retry\""));
    assert!(commands.contains("\"enabled\":false"));
    assert_eq!(commands.matches("\"type\":\"prompt\"").count(), 2);
    assert_eq!(
        commands.matches("\"streamingBehavior\":\"steer\"").count(),
        2
    );
    let session = fixture
        .inbox
        .chat_session(&fixture.chat_id)
        .unwrap()
        .expect("the active chat should keep its exact session binding");
    fixture.host.abort(&fixture.chat_id).await.unwrap();
    let mut tool_interrupted = false;
    let mut turn_stopped = false;
    while !(tool_interrupted && turn_stopped) {
        match events.recv().await.unwrap().event {
            ConversationEvent::ToolUpdate {
                status: ConversationToolStatus::Interrupted,
                ..
            } => tool_interrupted = true,
            ConversationEvent::TurnStopped => turn_stopped = true,
            _ => {}
        }
    }
    fixture.wait_for_live_children(0).await;
    assert_eq!(
        fixture.host.snapshot(&fixture.chat_id).unwrap().phase,
        ConversationPhase::Stopped
    );
    assert_eq!(
        fixture.inbox.chat_session(&fixture.chat_id).unwrap(),
        Some(session)
    );
    fixture.host.shutdown_all().await;
}

#[tokio::test]
async fn an_unexpected_pi_retry_attempt_becomes_one_terminal_failure_without_replay() {
    let fixture = ChatFixture::with_options(true, true, "retry-attempt");
    let mut events = fixture.host.subscribe();
    fixture.host.open(&fixture.chat_id).await.unwrap();

    let failure = tokio::time::timeout(FIXTURE_EVENT_TIMEOUT, async {
        loop {
            let changed = events.recv().await.unwrap();
            if let ConversationEvent::TurnFailed { ref message } = changed.event {
                break message.clone();
            }
        }
    })
    .await
    .expect("an unexpected automatic retry should become a terminal failure");

    assert!(failure.contains("provider unavailable"));
    fixture.wait_for_live_children(0).await;
    let snapshot = fixture.host.snapshot(&fixture.chat_id).unwrap();
    assert_eq!(snapshot.phase, ConversationPhase::Failed);
    assert!(snapshot.items.iter().any(|item| matches!(
        item,
        ConversationItem::Tool {
            id,
            status: ConversationToolStatus::Interrupted,
            ..
        } if id == "tool-call-retry"
    )));
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
async fn repeated_user_text_remains_two_distinct_turns() {
    let fixture = ChatFixture::with_options(true, true, "repeated-events");
    let mut events = fixture.host.subscribe();
    fixture.host.open(&fixture.chat_id).await.unwrap();
    for _ in 0..2 {
        assert!(matches!(
            events.recv().await.unwrap().event,
            ConversationEvent::ItemAdded {
                item: ConversationItem::Message { .. },
                ..
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
async fn supervised_process_loss_interrupts_the_turn_and_running_tool_without_replay() {
    let fixture = ChatFixture::with_options(true, true, "crash-after-prompt");
    let mut events = fixture.host.subscribe();

    fixture.host.open(&fixture.chat_id).await.unwrap();
    let interruption = tokio::time::timeout(FIXTURE_EVENT_TIMEOUT, async {
        loop {
            let changed = events.recv().await.unwrap();
            if matches!(changed.event, ConversationEvent::TurnInterrupted { .. }) {
                break changed;
            }
        }
    })
    .await
    .expect("the supervised child exit should become a typed interruption event");
    assert_eq!(interruption.chat_id, fixture.chat_id);
    assert_eq!(
        fixture.host.snapshot(&fixture.chat_id).unwrap().phase,
        ConversationPhase::Interrupted
    );
    assert!(
        fixture
            .host
            .snapshot(&fixture.chat_id)
            .unwrap()
            .items
            .iter()
            .any(|item| matches!(
                item,
                ConversationItem::Tool {
                    id,
                    status: ConversationToolStatus::Interrupted,
                    ..
                } if id == "tool-call-crashed"
            ))
    );
    assert_eq!(fixture.live_children(), 0);

    let cached = fixture.host.open(&fixture.chat_id).await.unwrap();
    assert_eq!(cached.phase, ConversationPhase::Interrupted);
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
async fn fresh_host_restores_an_unresolved_tool_as_interrupted_without_replay() {
    let fixture = ChatFixture::with_options(true, true, "persisted-unresolved-tool");
    let mut events = fixture.host.subscribe();
    fixture.host.open(&fixture.chat_id).await.unwrap();

    tokio::time::timeout(FIXTURE_EVENT_TIMEOUT, async {
        loop {
            if matches!(
                events.recv().await.unwrap().event,
                ConversationEvent::TurnInterrupted { .. }
            ) {
                break;
            }
        }
    })
    .await
    .expect("the first host should observe the interrupted owned process");
    fixture.wait_for_live_children(0).await;
    let stored_session = fixture
        .inbox
        .chat_session(&fixture.chat_id)
        .unwrap()
        .expect("the first host should bind the exact session before dispatch");
    assert!(fs::metadata(&stored_session.path).unwrap().len() > 0);

    let fresh_host = fixture.fresh_host();
    let restored = fresh_host.open(&fixture.chat_id).await.unwrap();

    assert_eq!(restored.phase, ConversationPhase::Interrupted);
    assert_eq!(
        restored.failure.as_deref(),
        Some("The agent turn was interrupted before Più reopened this chat.")
    );
    assert!(restored.items.iter().any(|item| matches!(
        item,
        ConversationItem::Tool {
            id,
            status: ConversationToolStatus::Interrupted,
            ..
        } if id == "tool-call-crashed"
    )));
    fixture.wait_for_live_children(0).await;
    assert_eq!(fixture.record("launches").lines().count(), 2);
    assert_eq!(
        fixture.inbox.chat_session(&fixture.chat_id).unwrap(),
        Some(stored_session)
    );
    assert_eq!(
        fixture
            .record("commands")
            .matches("\"type\":\"prompt\"")
            .count(),
        1,
        "restoration must inspect the exact session without replaying its first prompt"
    );
    fresh_host.shutdown_all().await;
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
