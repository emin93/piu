use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::Path,
    process::Command,
    time::{Duration, Instant},
};

use piu_lib::git_process::GitProcess;
use piu_lib::project_inbox::{
    ChatMergeState, OpenRepositoryOutcome, ProjectAvailability, ProjectInbox, ProjectInboxError,
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

#[test]
fn rejects_non_repositories_without_persisting_them() {
    let fixture = TempDir::new().expect("fixture should be created");
    let database_path = fixture.path().join("piu.sqlite3");
    let ordinary_directory = fixture.path().join("ordinary-directory");
    fs::create_dir(&ordinary_directory).expect("ordinary directory should be created");
    let inbox = ProjectInbox::open(&database_path).expect("inbox should open");

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
    let inbox = ProjectInbox::open(&database_path).expect("inbox should open");

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

    let reopened = ProjectInbox::open(&database_path).expect("inbox should reopen");
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
    let inbox = ProjectInbox::open(&database_path).expect("inbox should open");
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
fn reports_a_repository_that_becomes_inaccessible() {
    let fixture = TempDir::new().expect("fixture should be created");
    let database_path = fixture.path().join("piu.sqlite3");
    let repository_path = fixture.path().join("restricted");
    make_repository(&repository_path);
    let inbox = ProjectInbox::open(&database_path).expect("inbox should open");
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
    let inbox = ProjectInbox::open(&database_path).expect("inbox should open");
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
fn snapshot_does_not_hold_storage_behind_git_inspection() {
    let fixture = TempDir::new().expect("fixture should be created");
    let database_path = fixture.path().join("piu.sqlite3");
    let repository_path = fixture.path().join("alpha");
    make_repository(&repository_path);
    let inbox = ProjectInbox::open(&database_path).expect("inbox should open");
    inbox
        .open_repository(&repository_path)
        .expect("repository should open");
    drop(inbox);

    let slow_git = fixture.path().join("slow-git.zsh");
    fs::write(&slow_git, "#!/bin/zsh\nsleep 10\n").expect("fixture executable should be written");
    fs::set_permissions(&slow_git, fs::Permissions::from_mode(0o755))
        .expect("fixture executable should be runnable");
    let inbox = ProjectInbox::with_git(&database_path, GitProcess::with_executable(slow_git))
        .expect("inbox should open without inspecting Git");

    let started = Instant::now();
    let snapshot = inbox.snapshot().expect("snapshot should load");

    assert_eq!(snapshot.projects.len(), 1);
    assert!(
        started.elapsed() < Duration::from_millis(500),
        "snapshot availability must not invoke Git while storage is locked"
    );
}
