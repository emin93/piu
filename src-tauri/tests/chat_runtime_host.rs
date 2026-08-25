#[allow(dead_code)]
mod support;

use std::{fs, os::unix::fs::symlink, sync::Arc};

use piu_lib::{
    agent_environment::{
        AgentEnvironment, AgentEnvironmentPolicy, AgentEnvironmentProcessSpec, AgentResourceId,
        AgentResourcePreferenceScope, AgentResourceRefreshStatus,
    },
    chat_runtime_commands::{ConversationPromptRequest, ConversationStreamingBehavior},
    chat_runtime_host::{
        ChatRuntimeChangedEvent, ChatRuntimeHost, ChatRuntimeHostError, ConversationEvent,
        ConversationInputAnswer, ConversationInputKind, ConversationItem, ConversationPhase,
        ConversationSnapshot, ConversationToolStatus, ModelRouteId, ReasoningEffort,
    },
    chat_workspaces::ChatWorkspaces,
    git_process::GitProcess,
    project_inbox::{ChatSetupPhase, ProjectInbox},
    prompt_attachments::{PromptAttachment, PromptAttachmentError, PromptAttachmentKind},
    runtime_preferences::RuntimePreferences,
};
use support::{TemporaryAppData, TemporaryGitRemote};

const FIXTURE_EVENT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

fn project_skill() -> AgentResourceId {
    AgentResourceId::Skill {
        id: "project://skills/check".into(),
    }
}

struct ChatFixture {
    _app_data: TemporaryAppData,
    _remote: TemporaryGitRemote,
    host: ChatRuntimeHost,
    environment: Arc<AgentEnvironment>,
    inbox: Arc<ProjectInbox>,
    workspaces: Arc<ChatWorkspaces>,
    resource_directory: std::path::PathBuf,
    chat_id: String,
    worktree: std::path::PathBuf,
}

#[tokio::test]
async fn model_controls_do_not_turn_a_completed_chat_into_a_stopped_chat() {
    let fixture = ChatFixture::with_options(true, true, "streaming");
    let opened = fixture.host.open(&fixture.chat_id).await.unwrap();
    assert_eq!(opened.phase, ConversationPhase::Running);
    fixture.wait_for_live_children(0).await;
    assert_eq!(
        fixture.host.snapshot(&fixture.chat_id).unwrap().phase,
        ConversationPhase::Idle
    );

    fixture.host.model_controls(&fixture.chat_id).await.unwrap();

    assert_eq!(
        fixture.host.snapshot(&fixture.chat_id).unwrap().phase,
        ConversationPhase::Idle
    );
    fixture.host.shutdown_all().await;
}

#[tokio::test]
async fn an_idle_affected_child_restarts_on_the_exact_session_and_worktree() {
    let fixture = ChatFixture::with_options(true, true, "streaming");
    fixture.host.open(&fixture.chat_id).await.unwrap();
    fixture.wait_for_live_children(0).await;
    fixture.host.model_controls(&fixture.chat_id).await.unwrap();
    fixture.wait_for_live_children(1).await;
    let session = fixture
        .inbox
        .chat_session(&fixture.chat_id)
        .unwrap()
        .expect("completed chat should keep its Pi session");
    let project_id = fixture
        .inbox
        .snapshot()
        .unwrap()
        .chats
        .iter()
        .find(|chat| chat.id == fixture.chat_id)
        .and_then(|chat| chat.project_id)
        .unwrap();
    let launches_before = fixture.session_launch_count(&fixture.chat_id);

    let change = fixture
        .environment
        .set_resource_enabled(
            project_id,
            AgentResourcePreferenceScope::Project,
            project_skill(),
            false,
        )
        .await
        .unwrap();
    let applied = fixture
        .host
        .refresh_resources(project_id, change)
        .await
        .unwrap();

    assert_eq!(applied.status, AgentResourceRefreshStatus::Applied);
    assert_eq!(applied.deferred_chat_count, 0);
    assert_eq!(
        fixture.session_launch_count(&fixture.chat_id),
        launches_before + 1
    );
    assert_eq!(
        fixture.inbox.chat_session(&fixture.chat_id).unwrap(),
        Some(session)
    );
    assert_eq!(fixture.live_children(), 1);
    let arguments = fixture.record(&format!(
        "arguments-{}",
        fixture.session_id(&fixture.chat_id)
    ));
    assert_eq!(arguments.matches("--skill").count(), 1);
    assert!(
        arguments.contains(
            fixture
                .resource_directory
                .join("agent-runtime/skills")
                .to_string_lossy()
                .as_ref()
        )
    );
    assert!(!arguments.contains(".pi/skills/check/SKILL.md"));
    fixture.host.shutdown_all().await;
}

#[tokio::test]
async fn an_active_affected_child_keeps_running_and_reports_the_deferred_refresh() {
    let fixture = ChatFixture::new(true);
    fixture.host.open(&fixture.chat_id).await.unwrap();
    fixture.wait_for_live_children(1).await;
    let project_id = fixture
        .inbox
        .snapshot()
        .unwrap()
        .chats
        .iter()
        .find(|chat| chat.id == fixture.chat_id)
        .and_then(|chat| chat.project_id)
        .unwrap();
    let launches_before = fixture.session_launch_count(&fixture.chat_id);
    let session = fixture
        .inbox
        .chat_session(&fixture.chat_id)
        .unwrap()
        .expect("active chat should already be bound");

    let change = fixture
        .environment
        .set_resource_enabled(
            project_id,
            AgentResourcePreferenceScope::Project,
            project_skill(),
            false,
        )
        .await
        .unwrap();
    let applied = fixture
        .host
        .refresh_resources(project_id, change)
        .await
        .unwrap();

    assert_eq!(applied.status, AgentResourceRefreshStatus::Deferred);
    assert_eq!(applied.deferred_chat_count, 1);
    assert_eq!(
        fixture.session_launch_count(&fixture.chat_id),
        launches_before
    );
    assert_eq!(fixture.live_children(), 1);
    assert_eq!(
        fixture.inbox.chat_session(&fixture.chat_id).unwrap(),
        Some(session)
    );
    assert_eq!(
        fixture.host.snapshot(&fixture.chat_id).unwrap().phase,
        ConversationPhase::Running
    );
    fixture.host.shutdown_all().await;
}

#[tokio::test]
async fn a_deferred_resource_refresh_reopens_the_exact_session_at_turn_completion() {
    let fixture = ChatFixture::with_options(true, true, "controlled-completion");
    fixture.host.open(&fixture.chat_id).await.unwrap();
    fixture.wait_for_live_children(1).await;
    let project_id = fixture
        .inbox
        .snapshot()
        .unwrap()
        .chats
        .iter()
        .find(|chat| chat.id == fixture.chat_id)
        .and_then(|chat| chat.project_id)
        .unwrap();
    let session = fixture
        .inbox
        .chat_session(&fixture.chat_id)
        .unwrap()
        .expect("active chat should already be bound");
    let launches_before = fixture.session_launch_count(&fixture.chat_id);
    let change = fixture
        .environment
        .set_resource_enabled(
            project_id,
            AgentResourcePreferenceScope::Project,
            project_skill(),
            false,
        )
        .await
        .unwrap();
    let applied = fixture
        .host
        .refresh_resources(project_id, change)
        .await
        .unwrap();
    assert_eq!(applied.status, AgentResourceRefreshStatus::Deferred);
    assert_eq!(fixture.live_children(), 1);

    fs::write(
        fixture
            ._app_data
            .path()
            .join("host-fixture")
            .join(format!("complete-{}", fixture.session_id(&fixture.chat_id))),
        "complete\n",
    )
    .unwrap();
    tokio::time::timeout(FIXTURE_EVENT_TIMEOUT, async {
        while fixture.session_launch_count(&fixture.chat_id) != launches_before + 1 {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the exact session should reopen after the safe turn boundary");

    assert_eq!(fixture.live_children(), 1);
    assert_eq!(
        fixture.inbox.chat_session(&fixture.chat_id).unwrap(),
        Some(session)
    );
    let arguments = fixture.record(&format!(
        "arguments-{}",
        fixture.session_id(&fixture.chat_id)
    ));
    assert!(!arguments.contains(".pi/skills/check/SKILL.md"));
    fixture.host.shutdown_all().await;
}

#[tokio::test]
async fn a_resource_refresh_does_not_restart_a_project_with_an_effective_override() {
    let fixture = ChatFixture::with_options(true, true, "streaming");
    let second_remote = TemporaryGitRemote::new();
    fs::write(second_remote.working_path().join("README.md"), "second\n").unwrap();
    fs::create_dir_all(second_remote.working_path().join(".pi/skills/check")).unwrap();
    fs::write(
        second_remote
            .working_path()
            .join(".pi/skills/check/SKILL.md"),
        "# Check\n",
    )
    .unwrap();
    fs::create_dir_all(second_remote.working_path().join(".pi/extensions")).unwrap();
    fs::write(
        second_remote
            .working_path()
            .join(".pi/extensions/review.mjs"),
        "export default function review() {}\n",
    )
    .unwrap();
    second_remote.git(["add", "."]);
    second_remote.git(["commit", "-m", "second fixture"]);
    second_remote.git(["push", "-u", "origin", "main"]);
    let second_project = fixture
        .inbox
        .open_repository(second_remote.working_path())
        .unwrap()
        .project;
    let second_chat = fixture
        .workspaces
        .create_chat(second_project.id, "Inspect the runtime", &[])
        .unwrap()
        .chat;
    fixture
        .workspaces
        .start_setup(&second_chat.id, Arc::new(|_| {}))
        .unwrap();
    let first_project_id = fixture
        .inbox
        .snapshot()
        .unwrap()
        .chats
        .iter()
        .find(|chat| chat.id == fixture.chat_id)
        .and_then(|chat| chat.project_id)
        .unwrap();
    fixture.host.open(&fixture.chat_id).await.unwrap();
    fixture.host.open(&second_chat.id).await.unwrap();
    fixture.wait_for_live_children(0).await;
    fixture.host.model_controls(&fixture.chat_id).await.unwrap();
    fixture.host.model_controls(&second_chat.id).await.unwrap();
    fixture.wait_for_live_children(2).await;
    let first_launches = fixture.session_launch_count(&fixture.chat_id);
    let second_launches = fixture.session_launch_count(&second_chat.id);

    fixture
        .environment
        .set_resource_enabled(
            second_project.id,
            AgentResourcePreferenceScope::Project,
            project_skill(),
            true,
        )
        .await
        .unwrap();
    let change = fixture
        .environment
        .set_resource_enabled(
            first_project_id,
            AgentResourcePreferenceScope::Global,
            project_skill(),
            false,
        )
        .await
        .unwrap();
    let applied = fixture
        .host
        .refresh_resources(first_project_id, change)
        .await
        .unwrap();

    assert_eq!(applied.status, AgentResourceRefreshStatus::Applied);
    assert_eq!(applied.deferred_chat_count, 0);
    assert_eq!(
        fixture.session_launch_count(&fixture.chat_id),
        first_launches + 1
    );
    assert_eq!(
        fixture.session_launch_count(&second_chat.id),
        second_launches,
        "the second project's effective override should leave its idle child untouched"
    );
    assert_eq!(fixture.live_children(), 2);
    fixture.host.shutdown_all().await;
}

#[tokio::test]
async fn model_and_effort_controls_follow_the_native_pi_rpc_contract_without_restarting() {
    let fixture = ChatFixture::new(true);
    fixture.host.open(&fixture.chat_id).await.unwrap();

    let initial = fixture.host.model_controls(&fixture.chat_id).await.unwrap();
    assert_eq!(
        initial.selected_route,
        ModelRouteId {
            provider: "openai-codex".into(),
            model_id: "gpt-5.6-sol".into(),
        }
    );
    assert_eq!(initial.selected_effort, ReasoningEffort::ExtraHigh);
    assert!(initial.applies_after_current_step);
    assert_eq!(
        initial
            .routes
            .iter()
            .map(|route| (route.name.as_str(), route.accepts_images))
            .collect::<Vec<_>>(),
        vec![("GPT-5.6 Sol", true), ("Qwen 3.8 27B", false)]
    );
    assert_eq!(
        initial.efforts,
        vec![
            ReasoningEffort::Off,
            ReasoningEffort::Minimal,
            ReasoningEffort::Low,
            ReasoningEffort::Medium,
            ReasoningEffort::High,
            ReasoningEffort::ExtraHigh,
            ReasoningEffort::Maximum,
        ]
    );

    let switched = fixture
        .host
        .select_model_route(
            &fixture.chat_id,
            ModelRouteId {
                provider: "local-mlx".into(),
                model_id: "qwen3.8-27b".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(switched.selected_route.provider, "local-mlx");
    assert_eq!(
        switched.efforts,
        vec![
            ReasoningEffort::Low,
            ReasoningEffort::Medium,
            ReasoningEffort::ExtraHigh,
        ]
    );

    let adjusted = fixture
        .host
        .select_reasoning_effort(&fixture.chat_id, ReasoningEffort::Medium)
        .await
        .unwrap();
    assert_eq!(adjusted.selected_effort, ReasoningEffort::Medium);
    fixture
        .host
        .select_model_route(
            &fixture.chat_id,
            ModelRouteId {
                provider: "openai-codex".into(),
                model_id: "gpt-5.6-sol".into(),
            },
        )
        .await
        .unwrap();
    fixture
        .host
        .select_reasoning_effort(&fixture.chat_id, ReasoningEffort::High)
        .await
        .unwrap();
    let restored = fixture
        .host
        .select_model_route(
            &fixture.chat_id,
            ModelRouteId {
                provider: "local-mlx".into(),
                model_id: "qwen3.8-27b".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(restored.selected_effort, ReasoningEffort::Medium);
    assert_eq!(fixture.record("launches").lines().count(), 1);
    let commands = fixture.record("commands");
    assert!(commands.contains("\"type\":\"set_model\""));
    assert!(commands.contains("\"modelId\":\"qwen3.8-27b\""));
    assert!(commands.contains("\"type\":\"set_thinking_level\""));
    assert!(!commands.contains("\"type\":\"abort\""));
}

#[tokio::test]
async fn a_partial_model_change_rolls_back_the_previous_route_and_effort() {
    let fixture = ChatFixture::with_options(true, true, "reject-thinking-levels");
    fixture.host.open(&fixture.chat_id).await.unwrap();

    fixture
        .host
        .select_model_route(
            &fixture.chat_id,
            ModelRouteId {
                provider: "local-mlx".into(),
                model_id: "qwen3.8-27b".into(),
            },
        )
        .await
        .unwrap_err();

    let commands = fixture.record("commands");
    let selected = commands
        .find("\"modelId\":\"qwen3.8-27b\"")
        .expect("requested route should reach Pi");
    let restored = commands
        .rfind("\"modelId\":\"gpt-5.6-sol\"")
        .expect("the previous route should be restored");
    assert!(restored > selected);
    assert!(commands[restored..].contains("\"level\":\"xhigh\""));
    assert_eq!(fixture.record("launches").lines().count(), 1);
    assert!(!commands.contains("\"type\":\"abort\""));
}

#[tokio::test]
async fn a_new_chat_launches_with_its_captured_route_across_host_relaunch() {
    let fixture = ChatFixture::new(true);
    fixture.host.open(&fixture.chat_id).await.unwrap();
    fixture
        .host
        .select_model_route(
            &fixture.chat_id,
            ModelRouteId {
                provider: "local-mlx".into(),
                model_id: "qwen3.8-27b".into(),
            },
        )
        .await
        .unwrap();
    fixture
        .host
        .select_reasoning_effort(&fixture.chat_id, ReasoningEffort::Medium)
        .await
        .unwrap();

    let captured_chat = fixture.create_chat("Use the remembered local route");
    fixture
        .host
        .select_model_route(
            &fixture.chat_id,
            ModelRouteId {
                provider: "openai-codex".into(),
                model_id: "gpt-5.6-sol".into(),
            },
        )
        .await
        .unwrap();

    let preferences = RuntimePreferences::open(&fixture._app_data.database_path()).unwrap();
    let captured = preferences
        .initial_chat_selection(&captured_chat)
        .unwrap()
        .expect("new chat should journal its inference selection");
    assert_eq!(captured.route.provider_id(), "local-mlx");
    assert_eq!(captured.route.model_id(), "qwen3.8-27b");
    assert_eq!(captured.effort.as_deref(), Some("medium"));

    let relaunched_host = fixture.fresh_host();
    relaunched_host.open(&captured_chat).await.unwrap();
    let arguments = fixture.record("arguments");
    assert!(arguments.contains("--model-provider\nlocal-mlx\n"));
    assert!(arguments.contains("--model-id\nqwen3.8-27b\n"));
    assert!(arguments.contains("--thinking-level\nmedium\n"));
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
        if install_skills {
            fs::create_dir_all(remote.working_path().join(".pi/skills/check")).unwrap();
            fs::write(
                remote.working_path().join(".pi/skills/check/SKILL.md"),
                "# Check\n",
            )
            .unwrap();
            fs::create_dir_all(remote.working_path().join(".pi/extensions")).unwrap();
            fs::write(
                remote.working_path().join(".pi/extensions/review.mjs"),
                "export default function review() {}\n",
            )
            .unwrap();
        }
        remote.git(["add", "."]);
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
        let mut environment_variables = std::collections::BTreeMap::new();
        environment_variables.insert(
            std::ffi::OsString::from("HOME"),
            home.as_os_str().to_owned(),
        );
        environment_variables.insert(
            std::ffi::OsString::from("PATH"),
            std::ffi::OsString::from("/usr/bin:/bin"),
        );
        environment_variables.insert(
            std::ffi::OsString::from("PIU_ENVIRONMENT_FIXTURE_MODE"),
            std::ffi::OsString::from("chat-runtime"),
        );
        environment_variables.insert(
            std::ffi::OsString::from("PIU_ENVIRONMENT_FIXTURE_RECORD_DIR"),
            app_data.path().join("environment-fixture").into_os_string(),
        );
        let environment = Arc::new(
            AgentEnvironment::new(
                Arc::clone(&inbox),
                Arc::new(RuntimePreferences::open(&app_data.database_path()).unwrap()),
                AgentEnvironmentProcessSpec {
                    executable: std::path::PathBuf::from("/bin/zsh"),
                    launcher: std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                        .join("tests/fixtures/agent-environment-child.zsh"),
                    agent_directory: app_data.path().join("agent"),
                    credential_lock_directory: app_data.path().join("credential-locks"),
                    environment: environment_variables,
                },
                AgentEnvironmentPolicy {
                    inspection_timeout: std::time::Duration::from_secs(2),
                    maximum_stdout_bytes: 64 * 1024,
                    maximum_stderr_bytes: 16 * 1024,
                },
            )
            .unwrap(),
        );
        let host = ChatRuntimeHost::new(
            Arc::clone(&inbox),
            Arc::clone(&workspaces),
            Arc::clone(&environment),
            app_data.path(),
            &resource_directory,
            &home,
        )
        .expect("create runtime host");
        Self {
            _app_data: app_data,
            _remote: remote,
            host,
            environment,
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
            Arc::clone(&self.environment),
            self._app_data.path(),
            &self.resource_directory,
            &self._app_data.path().join("real-home"),
        )
        .expect("create fresh runtime host")
    }

    fn session_id(&self, chat_id: &str) -> String {
        format!("pi-{chat_id}")
    }

    fn session_launch_count(&self, chat_id: &str) -> usize {
        fs::read_to_string(
            self._app_data
                .path()
                .join("host-fixture")
                .join(format!("launches-{}", self.session_id(chat_id))),
        )
        .map(|launches| launches.lines().count())
        .unwrap_or_default()
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

    let opening_snapshot = fixture.host.open(&fixture.chat_id).await.unwrap();
    let mut received = Vec::new();
    let mut revisions = Vec::new();
    loop {
        let changed = tokio::time::timeout(FIXTURE_EVENT_TIMEOUT, events.recv())
            .await
            .expect("conversation event should arrive")
            .expect("event subscription should remain live");
        assert_eq!(changed.chat_id, fixture.chat_id);
        let completed = matches!(changed.event, ConversationEvent::TurnCompleted);
        revisions.push(changed.revision);
        received.push(changed.event);
        if completed {
            break;
        }
    }

    assert_eq!(revisions.first(), Some(&1));
    assert!(
        revisions
            .windows(2)
            .all(|window| window[1] == window[0] + 1)
    );
    assert!(opening_snapshot.revision <= *revisions.last().unwrap());

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
        revision: 4,
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
            "phase": "running",
            "revision": 4
        })
    );

    let changed = ChatRuntimeChangedEvent {
        chat_id: "chat-1".into(),
        event: ConversationEvent::ToolUpdate {
            detail: "README.md".into(),
            item_id: "tool-call-1".into(),
            status: ConversationToolStatus::Succeeded,
        },
        revision: 5,
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
            },
            "revision": 5
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
async fn select_input_rejects_an_unknown_option_without_consuming_the_request() {
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
    .expect("Pi's select input should reach the host");

    let error = fixture
        .host
        .answer_input(
            &fixture.chat_id,
            &request.id,
            ConversationInputAnswer::Value {
                value: "Forged choice".into(),
            },
        )
        .await
        .expect_err("a select answer must match one of Pi's pending options");
    assert!(matches!(error, ChatRuntimeHostError::InvalidInputAnswer));
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
    fixture
        .wait_for_record_contains(
            "extension-ui-responses",
            r#"{"id":"extension-choice-1","type":"extension_ui_response","value":"Keep both"}"#,
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
async fn rejected_image_send_retires_a_send_only_restored_text_runtime() {
    let fixture = ChatFixture::with_options(true, true, "text-only");
    fixture.host.open(&fixture.chat_id).await.unwrap();
    fixture.host.abort(&fixture.chat_id).await.unwrap();
    fixture.wait_for_live_children(0).await;

    let fresh_host = fixture.fresh_host();
    let restored = fresh_host.open(&fixture.chat_id).await.unwrap();
    let stored_session = fixture.inbox.chat_session(&fixture.chat_id).unwrap();
    assert_eq!(restored.phase, ConversationPhase::Stopped);
    fixture.wait_for_live_children(0).await;

    let error = fresh_host
        .send_with_attachments(&fixture.chat_id, "Inspect this", &[image_attachment()])
        .await
        .expect_err("text-only route must reject image input");

    assert!(matches!(
        error,
        ChatRuntimeHostError::Attachment(PromptAttachmentError::ModelMediaUnsupported)
    ));
    fixture.wait_for_live_children(0).await;
    assert_eq!(fresh_host.snapshot(&fixture.chat_id).unwrap(), restored);
    assert_eq!(
        fixture.inbox.chat_session(&fixture.chat_id).unwrap(),
        stored_session
    );
    fresh_host.shutdown_all().await;
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

    let host = fixture.host.clone();
    let chat_id = fixture.chat_id.clone();
    let pending_send =
        tokio::spawn(async move { host.send(&chat_id, "Inspect the queued result").await });
    tokio::time::timeout(std::time::Duration::from_millis(300), async {
        loop {
            if matches!(
                events.recv().await.unwrap().event,
                ConversationEvent::ItemAdded {
                    item: ConversationItem::Message {
                        ref text,
                        queued: true,
                        ..
                    },
                    ..
                } if text == "Inspect the queued result"
            ) {
                break;
            }
        }
    })
    .await
    .expect("the accepted steering message should project before Pi responds");
    assert!(!pending_send.is_finished());
    pending_send.await.unwrap().unwrap();

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
async fn rejected_steering_is_visible_while_pending_then_rolls_back_without_replay() {
    let fixture = ChatFixture::with_options(true, true, "reject-steer");
    fixture.host.open(&fixture.chat_id).await.unwrap();
    let mut events = fixture.host.subscribe();
    let host = fixture.host.clone();
    let chat_id = fixture.chat_id.clone();
    let pending_send = tokio::spawn(async move { host.send(&chat_id, "Reject this steer").await });

    let optimistic_item_id = tokio::time::timeout(std::time::Duration::from_millis(300), async {
        loop {
            if let ConversationEvent::ItemAdded {
                item:
                    ConversationItem::Message {
                        id,
                        ref text,
                        queued: true,
                        ..
                    },
                ..
            } = events.recv().await.unwrap().event
                && text == "Reject this steer"
            {
                break id;
            }
        }
    })
    .await
    .expect("the pending steer should be visible before Pi responds");
    assert!(!pending_send.is_finished());
    assert!(pending_send.await.unwrap().is_err());

    tokio::time::timeout(FIXTURE_EVENT_TIMEOUT, async {
        loop {
            if matches!(
                events.recv().await.unwrap().event,
                ConversationEvent::ItemRemoved { ref item_id } if item_id == &optimistic_item_id
            ) {
                break;
            }
        }
    })
    .await
    .expect("a rejected steer should remove only its optimistic transcript item");
    assert!(!fixture
        .host
        .snapshot(&fixture.chat_id)
        .unwrap()
        .items
        .iter()
        .any(|item| matches!(item, ConversationItem::Message { text, .. } if text == "Reject this steer")));
    assert_eq!(
        fixture
            .record("commands")
            .matches("\"type\":\"prompt\"")
            .count(),
        2
    );
    assert_eq!(
        fixture.live_children(),
        1,
        "rejecting a steer must preserve the child running the underlying turn"
    );
    fixture.host.shutdown_all().await;
}

#[tokio::test]
async fn rejected_restored_prompt_retires_its_child_and_preserves_the_session() {
    let fixture = ChatFixture::new(true);
    fixture.host.open(&fixture.chat_id).await.unwrap();
    fixture.host.abort(&fixture.chat_id).await.unwrap();
    fixture.wait_for_live_children(0).await;
    let stored_session = fixture
        .inbox
        .chat_session(&fixture.chat_id)
        .unwrap()
        .expect("the stopped chat should keep its exact session");
    fs::write(
        fixture._app_data.path().join("host-fixture/mode"),
        "reject-prompt",
    )
    .unwrap();

    let fresh_host = fixture.fresh_host();
    let restored = fresh_host.open(&fixture.chat_id).await.unwrap();
    assert_eq!(restored.phase, ConversationPhase::Stopped);
    fixture.wait_for_live_children(0).await;
    let mut events = fresh_host.subscribe();

    fresh_host
        .send(&fixture.chat_id, "Reject this restored prompt")
        .await
        .expect_err("Pi's prompt rejection should reach the host");

    let failed = fresh_host.snapshot(&fixture.chat_id).unwrap();
    assert_eq!(failed.phase, ConversationPhase::Failed);
    assert!(failed.failure.is_some());
    fixture.wait_for_live_children(0).await;
    assert_eq!(
        fixture.inbox.chat_session(&fixture.chat_id).unwrap(),
        Some(stored_session)
    );
    tokio::time::timeout(FIXTURE_EVENT_TIMEOUT, async {
        loop {
            if matches!(
                events.recv().await.unwrap().event,
                ConversationEvent::TurnFailed { .. }
            ) {
                break;
            }
        }
    })
    .await
    .expect("a rejected normal prompt should emit one terminal failure");
    fresh_host.shutdown_all().await;
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
    assert!(matches!(
        interruption.event,
        ConversationEvent::TurnInterrupted { ref message }
            if message == "The agent runtime stopped before the turn finished. Send another message to continue."
    ));
    assert_eq!(
        fixture.host.snapshot(&fixture.chat_id).unwrap().phase,
        ConversationPhase::Interrupted
    );
    assert_eq!(
        fixture
            .host
            .snapshot(&fixture.chat_id)
            .unwrap()
            .failure
            .as_deref(),
        Some(
            "The agent runtime stopped before the turn finished. Send another message to continue."
        )
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
            .canonicalize()
            .unwrap()
            .display()
    )));
    assert!(arguments.contains(&format!(
        "--skill\n{}\n",
        fixture
            .worktree
            .join(".pi/skills/check/SKILL.md")
            .canonicalize()
            .unwrap()
            .display()
    )));
    assert!(arguments.contains(&format!(
        "--extension\n{}\n",
        fixture
            .worktree
            .join(".pi/extensions/review.mjs")
            .canonicalize()
            .unwrap()
            .display()
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
