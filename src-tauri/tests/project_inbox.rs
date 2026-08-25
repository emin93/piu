use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::Path,
    process::Command,
    sync::{Arc, Mutex, mpsc},
    thread,
    time::Duration,
};

use piu_lib::{
    git_process::GitProcess,
    project_inbox::{
        ChatMergeState, ChatSessionReference, OpenRepositoryOutcome, ProjectAvailability,
        ProjectInbox, ProjectInboxError, RepositoryIdentity, RepositoryInspectionError,
        RepositoryInspector,
    },
};
use rusqlite::{Connection, params};
use tempfile::TempDir;

fn make_repository(path: &Path) {
    fs::create_dir_all(path).expect("repository directory should be created");
    let status = Command::new("git")
        .args(["init", "--quiet"])
        .arg(path)
        .status()
        .expect("git should be available to create a real repository fixture");
    assert!(status.success(), "git init should succeed");
}

fn open_inbox(database_path: &Path) -> ProjectInbox {
    ProjectInbox::with_git(
        database_path,
        GitProcess::with_executable("/usr/bin/git".into()),
    )
    .expect("inbox should open")
}

fn seed_chat(
    database_path: &Path,
    id: &str,
    project_id: i64,
    title: &str,
    created_at_ms: i64,
    merge_state: &str,
) {
    let connection = Connection::open(database_path).expect("fixture database should open");
    let project_name: String = connection
        .query_row(
            "SELECT name FROM projects WHERE id = ?1",
            [project_id],
            |row| row.get(0),
        )
        .expect("fixture project should exist");
    connection
        .execute(
            "INSERT INTO chats (
                id, project_id, project_name, title, branch_name, worktree_path,
                worktree_root_path, worktree_root_device, worktree_root_inode,
                worktree_git_dir_path, worktree_git_dir_device, worktree_git_dir_inode,
                base_commit, pull_request_number, created_at_ms, merge_state,
                setup_phase, setup_attempt, setup_log
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6, 'fixture-device', ?7,
                       ?8, 'fixture-git-device', ?9, 'fixture-base', ?10, ?11, ?12,
                       'succeeded', 1, '')",
            params![
                id,
                project_id,
                project_name,
                title,
                format!("feature/{id}"),
                format!("/private/tmp/piu-fixture-{id}"),
                format!("fixture-root-inode-{id}"),
                format!("/private/tmp/piu-fixture-git-{id}"),
                format!("fixture-git-inode-{id}"),
                73,
                created_at_ms,
                merge_state,
            ],
        )
        .expect("fixture chat should be inserted");
}

#[test]
fn rejects_non_repositories_without_persisting_them() {
    let fixture = TempDir::new().expect("fixture should be created");
    let database_path = fixture.path().join("piu.sqlite3");
    let ordinary_directory = fixture.path().join("ordinary-directory");
    fs::create_dir(&ordinary_directory).expect("ordinary directory should be created");
    let inbox = open_inbox(&database_path);

    let error = inbox
        .open_repository(&ordinary_directory)
        .expect_err("a non-repository should be rejected");

    assert!(matches!(error, ProjectInboxError::InvalidRepository));
    assert!(
        inbox
            .snapshot()
            .expect("snapshot should load")
            .projects
            .is_empty()
    );
}

#[test]
fn opens_real_repositories_once_and_retains_one_draft_per_project() {
    let fixture = TempDir::new().expect("fixture should be created");
    let database_path = fixture.path().join("piu.sqlite3");
    let repository_path = fixture.path().join("alpha");
    make_repository(&repository_path);
    let inbox = open_inbox(&database_path);

    let opened = inbox
        .open_repository(&repository_path)
        .expect("repository should open");
    assert_eq!(opened.outcome, OpenRepositoryOutcome::Added);
    assert_eq!(opened.project.name, "alpha");

    let duplicate = inbox
        .open_repository(&repository_path.join("."))
        .expect("the same repository should focus its existing project");
    assert_eq!(duplicate.outcome, OpenRepositoryOutcome::FocusedExisting);
    assert_eq!(duplicate.project.id, opened.project.id);

    inbox
        .save_draft(opened.project.id, "first prompt")
        .expect("draft should save");
    inbox
        .save_draft(opened.project.id, "replacement prompt")
        .expect("draft should update");
    drop(inbox);

    let reopened = open_inbox(&database_path);
    let snapshot = reopened.snapshot().expect("snapshot should load");
    assert_eq!(snapshot.projects.len(), 1);
    assert_eq!(snapshot.drafts.len(), 1);
    assert_eq!(snapshot.drafts[0].prompt, "replacement prompt");
}

#[test]
fn reports_a_repository_that_moves_after_it_was_opened() {
    let fixture = TempDir::new().expect("fixture should be created");
    let database_path = fixture.path().join("piu.sqlite3");
    let original_path = fixture.path().join("before");
    let moved_path = fixture.path().join("after");
    make_repository(&original_path);
    let inbox = open_inbox(&database_path);
    let opened = inbox
        .open_repository(&original_path)
        .expect("repository should open");

    fs::rename(&original_path, &moved_path).expect("repository should move");
    let snapshot = inbox.snapshot().expect("snapshot should still load");

    let project = snapshot
        .projects
        .iter()
        .find(|project| project.id == opened.project.id)
        .expect("project should remain in the inbox");
    assert_eq!(project.availability, ProjectAvailability::Missing);
}

#[test]
fn relaunch_revalidates_the_original_git_worktree_identity() {
    let fixture = TempDir::new().expect("fixture should be created");
    let database_path = fixture.path().join("piu.sqlite3");
    let repository_path = fixture.path().join("alpha");
    make_repository(&repository_path);
    let inbox = open_inbox(&database_path);
    let project = inbox
        .open_repository(&repository_path)
        .expect("repository should open")
        .project;
    drop(inbox);

    fs::remove_dir_all(repository_path.join(".git"))
        .expect("the admitted repository metadata should be removable");
    let without_git = open_inbox(&database_path);
    let availability = without_git
        .snapshot()
        .expect("snapshot should load")
        .projects
        .into_iter()
        .find(|candidate| candidate.id == project.id)
        .expect("project should remain remembered")
        .availability;
    assert_eq!(availability, ProjectAvailability::Missing);
    drop(without_git);

    make_repository(&repository_path);
    let replacement = open_inbox(&database_path);
    let availability = replacement
        .snapshot()
        .expect("snapshot should load")
        .projects
        .into_iter()
        .find(|candidate| candidate.id == project.id)
        .expect("project should remain remembered")
        .availability;
    assert_eq!(availability, ProjectAvailability::Missing);
}

#[test]
fn reports_a_repository_that_becomes_inaccessible() {
    let fixture = TempDir::new().expect("fixture should be created");
    let database_path = fixture.path().join("piu.sqlite3");
    let repository_path = fixture.path().join("restricted");
    make_repository(&repository_path);
    let inbox = open_inbox(&database_path);
    let opened = inbox
        .open_repository(&repository_path)
        .expect("repository should open");

    fs::set_permissions(&repository_path, fs::Permissions::from_mode(0o000))
        .expect("repository permissions should change");
    let availability = inbox
        .snapshot()
        .expect("snapshot should still load")
        .projects
        .into_iter()
        .find(|project| project.id == opened.project.id)
        .expect("project should remain in the inbox")
        .availability;
    fs::set_permissions(&repository_path, fs::Permissions::from_mode(0o755))
        .expect("repository permissions should be restored");

    assert_eq!(availability, ProjectAvailability::Inaccessible);
}

#[test]
fn removal_is_transactional_and_preserves_merged_history() {
    let fixture = TempDir::new().expect("fixture should be created");
    let database_path = fixture.path().join("piu.sqlite3");
    let repository_path = fixture.path().join("alpha");
    make_repository(&repository_path);
    let inbox = open_inbox(&database_path);
    let project = inbox
        .open_repository(&repository_path)
        .expect("repository should open")
        .project;
    inbox
        .save_draft(project.id, "keep until removal succeeds")
        .expect("draft should save");
    seed_chat(
        &database_path,
        "unmerged",
        project.id,
        "Unmerged work",
        200,
        "unmerged",
    );

    let blocked = inbox
        .remove_project(project.id)
        .expect_err("unmerged work should block project removal");
    assert!(matches!(
        blocked,
        ProjectInboxError::ProjectHasUnmergedChats { count: 1 }
    ));
    let unchanged = inbox.snapshot().expect("snapshot should still load");
    assert_eq!(unchanged.projects.len(), 1);
    assert_eq!(unchanged.drafts.len(), 1);

    let connection = Connection::open(&database_path).expect("fixture database should open");
    connection
        .execute(
            "UPDATE chats SET merge_state = 'merged' WHERE id = 'unmerged'",
            [],
        )
        .expect("fixture chat should merge");
    drop(connection);

    let removed = inbox
        .remove_project(project.id)
        .expect("a project with only merged history should be removable");
    assert!(removed.projects.is_empty());
    assert!(removed.drafts.is_empty());
    assert_eq!(removed.chats.len(), 1);
    assert_eq!(removed.chats[0].merge_state, ChatMergeState::Merged);
    assert_eq!(removed.chats[0].project_id, None);
    assert_eq!(removed.chats[0].project_name, "alpha");
}

#[test]
fn chat_session_binding_is_exact_idempotent_and_durable() {
    let fixture = TempDir::new().expect("fixture should be created");
    let database_path = fixture.path().join("piu.sqlite3");
    let repository_path = fixture.path().join("alpha");
    make_repository(&repository_path);
    let inbox = open_inbox(&database_path);
    let project = inbox
        .open_repository(&repository_path)
        .expect("repository should open")
        .project;
    seed_chat(
        &database_path,
        "chat-with-session",
        project.id,
        "Persistent conversation",
        300,
        "unmerged",
    );
    let expected = ChatSessionReference {
        id: "pi-session-1".into(),
        path: fixture.path().join("sessions/pi-session-1.jsonl"),
    };

    assert_eq!(
        inbox
            .chat_session("chat-with-session")
            .expect("chat should exist"),
        None
    );
    assert_eq!(
        inbox
            .bind_chat_session("chat-with-session", &expected.id, &expected.path)
            .expect("first binding should persist"),
        expected
    );
    assert_eq!(
        inbox
            .bind_chat_session("chat-with-session", &expected.id, &expected.path)
            .expect("the exact binding should be idempotent"),
        expected
    );
    drop(inbox);

    let reopened = open_inbox(&database_path);
    assert_eq!(
        reopened
            .chat_session("chat-with-session")
            .expect("chat should reopen"),
        Some(expected.clone())
    );
    let conflict = reopened
        .bind_chat_session(
            "chat-with-session",
            "pi-session-2",
            &fixture.path().join("sessions/pi-session-2.jsonl"),
        )
        .expect_err("a chat cannot drift to another Pi session");
    assert!(matches!(
        conflict,
        ProjectInboxError::ChatSessionAlreadyBound { chat_id }
            if chat_id == "chat-with-session"
    ));
    assert_eq!(
        reopened
            .chat_session("chat-with-session")
            .expect("original session should remain readable"),
        Some(expected)
    );
}

struct BlockingRepositoryInspector {
    started: Mutex<Option<mpsc::SyncSender<()>>>,
    release: Mutex<mpsc::Receiver<()>>,
}

impl RepositoryInspector for BlockingRepositoryInspector {
    fn inspect(
        &self,
        _selected_path: &Path,
    ) -> Result<RepositoryIdentity, RepositoryInspectionError> {
        if let Some(started) = self
            .started
            .lock()
            .expect("start signal lock should remain healthy")
            .take()
        {
            started
                .send(())
                .expect("snapshot should announce repository inspection");
        }
        self.release
            .lock()
            .expect("release signal lock should remain healthy")
            .recv()
            .expect("test should release repository inspection");
        Err(RepositoryInspectionError::Missing)
    }
}

#[test]
fn slow_repository_revalidation_does_not_block_draft_storage() {
    let fixture = TempDir::new().expect("fixture should be created");
    let database_path = fixture.path().join("piu.sqlite3");
    let repository_path = fixture.path().join("alpha");
    make_repository(&repository_path);
    let initial = open_inbox(&database_path);
    let project = initial
        .open_repository(&repository_path)
        .expect("repository should open")
        .project;
    drop(initial);

    let (started_sender, started_receiver) = mpsc::sync_channel(0);
    let (release_sender, release_receiver) = mpsc::sync_channel(0);
    let inspector = Arc::new(BlockingRepositoryInspector {
        started: Mutex::new(Some(started_sender)),
        release: Mutex::new(release_receiver),
    });
    let inbox = Arc::new(
        ProjectInbox::with_inspector(&database_path, inspector)
            .expect("inbox should reopen with a controlled repository inspector"),
    );
    let snapshot_inbox = Arc::clone(&inbox);
    let snapshot = thread::spawn(move || snapshot_inbox.snapshot());
    started_receiver
        .recv_timeout(Duration::from_secs(1))
        .expect("snapshot should reach repository inspection");

    let (saved_sender, saved_receiver) = mpsc::channel();
    let draft_inbox = Arc::clone(&inbox);
    thread::spawn(move || {
        saved_sender
            .send(draft_inbox.save_draft(project.id, "Store while probing"))
            .expect("draft result should be observed");
    });
    let saved = saved_receiver
        .recv_timeout(Duration::from_millis(250))
        .expect("repository inspection must not hold the database mutex");
    assert_eq!(
        saved.expect("draft should save").prompt,
        "Store while probing"
    );

    release_sender
        .send(())
        .expect("snapshot repository inspection should be released");
    assert_eq!(
        snapshot
            .join()
            .expect("snapshot thread should finish")
            .expect("snapshot should load")
            .projects[0]
            .availability,
        ProjectAvailability::Missing
    );
}
