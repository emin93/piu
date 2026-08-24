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
        ChatMergeState, OpenRepositoryOutcome, ProjectAvailability, ProjectInbox,
        ProjectInboxError, RepositoryIdentity, RepositoryInspectionError, RepositoryInspector,
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
                id, project_id, project_name, title, branch_name, pull_request_number,
                created_at_ms, merge_state
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                id,
                project_id,
                project_name,
                title,
                format!("feature/{id}"),
                73,
                created_at_ms,
                merge_state,
            ],
        )
        .expect("fixture chat should be inserted");
}

fn seed_version_two_project_inbox(database_path: &Path, repository_path: &Path) {
    let canonical_path = repository_path
        .canonicalize()
        .expect("legacy repository path should canonicalize");
    let connection = Connection::open(database_path).expect("legacy database should open");
    connection
        .execute_batch(
            r#"
            PRAGMA foreign_keys = ON;
            CREATE TABLE application_metadata (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                created_at TEXT NOT NULL
            );
            INSERT INTO application_metadata (id, created_at)
            VALUES (1, '2026-08-24T00:00:00.000Z');
            CREATE TABLE projects (
                id INTEGER PRIMARY KEY,
                canonical_path TEXT NOT NULL UNIQUE,
                name TEXT NOT NULL,
                created_at_ms INTEGER NOT NULL
            );
            CREATE TABLE chat_drafts (
                project_id INTEGER PRIMARY KEY REFERENCES projects(id) ON DELETE CASCADE,
                prompt TEXT NOT NULL,
                updated_at_ms INTEGER NOT NULL
            );
            CREATE TABLE chats (
                id TEXT PRIMARY KEY,
                project_id INTEGER REFERENCES projects(id) ON DELETE SET NULL,
                project_name TEXT NOT NULL,
                title TEXT NOT NULL,
                branch_name TEXT NOT NULL,
                pull_request_number INTEGER,
                created_at_ms INTEGER NOT NULL,
                merge_state TEXT NOT NULL CHECK (merge_state IN ('unmerged', 'merged'))
            );
            CREATE INDEX chats_created_at ON chats(created_at_ms DESC, id ASC);
            CREATE INDEX chats_project_id ON chats(project_id);
            PRAGMA user_version = 2;
            "#,
        )
        .expect("legacy schema should be created");
    connection
        .execute(
            "INSERT INTO projects (id, canonical_path, name, created_at_ms)
             VALUES (7, ?1, 'legacy-project', 100)",
            [canonical_path.to_string_lossy().as_ref()],
        )
        .expect("legacy project should be inserted");
    connection
        .execute(
            "INSERT INTO chat_drafts (project_id, prompt, updated_at_ms)
             VALUES (7, 'Preserve this draft', 200)",
            [],
        )
        .expect("legacy draft should be inserted");
    connection
        .execute(
            "INSERT INTO chats (
                 id, project_id, project_name, title, branch_name,
                 pull_request_number, created_at_ms, merge_state
             ) VALUES (
                 'legacy-chat', 7, 'legacy-project', 'Preserve this chat',
                 'agent/legacy-chat', 42, 300, 'unmerged'
             )",
            [],
        )
        .expect("legacy chat should be inserted");
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
fn version_two_projects_are_revalidated_backfilled_and_preserved() {
    let fixture = TempDir::new().expect("fixture should be created");
    let database_path = fixture.path().join("piu.sqlite3");
    let repository_path = fixture.path().join("legacy-project");
    make_repository(&repository_path);
    seed_version_two_project_inbox(&database_path, &repository_path);

    let inbox = open_inbox(&database_path);
    let snapshot = inbox.snapshot().expect("legacy inbox should load");
    assert_eq!(snapshot.projects.len(), 1);
    assert_eq!(
        snapshot.projects[0].availability,
        ProjectAvailability::Available
    );
    assert_eq!(snapshot.drafts[0].prompt, "Preserve this draft");
    assert_eq!(snapshot.chats[0].title, "Preserve this chat");
    drop(inbox);

    let connection = Connection::open(&database_path).expect("migrated database should open");
    let identity: (Option<String>, Option<String>, Option<String>) = connection
        .query_row(
            "SELECT root_device, root_inode, git_dir_path FROM projects WHERE id = 7",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("legacy identity should remain addressable");
    assert!(identity.0.is_some());
    assert!(identity.1.is_some());
    assert!(identity.2.is_some());
    drop(connection);

    fs::remove_dir_all(repository_path.join(".git"))
        .expect("the admitted repository metadata should be removable");
    make_repository(&repository_path);
    let replacement = open_inbox(&database_path);
    assert_eq!(
        replacement
            .snapshot()
            .expect("replacement should still yield an inbox")
            .projects[0]
            .availability,
        ProjectAvailability::Missing
    );
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

    let reopened = replacement
        .open_repository(&repository_path)
        .expect("explicitly reopening a valid replacement should refresh its identity");
    assert_eq!(reopened.outcome, OpenRepositoryOutcome::FocusedExisting);
    assert_eq!(reopened.project.availability, ProjectAvailability::Available);
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
