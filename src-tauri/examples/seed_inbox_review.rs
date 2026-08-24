use std::{env, fs, path::Path, process::Command};

use piu_lib::project_inbox::ProjectInbox;
use rusqlite::{Connection, params};

struct ReviewChatFixture<'a> {
    id: &'a str,
    project_id: i64,
    project_name: &'a str,
    title: &'a str,
    branch_name: &'a str,
    pull_request_number: Option<u32>,
    created_at_ms: i64,
    merge_state: &'a str,
}

fn initialize_repository(path: &Path) {
    fs::create_dir_all(path).expect("review repository directory should be created");
    let status = Command::new("git")
        .args(["init", "--quiet"])
        .arg(path)
        .status()
        .expect("Git should create the review repository");
    assert!(
        status.success(),
        "Git should initialize the review repository"
    );
}

fn main() {
    let app_data = env::args_os()
        .nth(1)
        .map(std::path::PathBuf::from)
        .expect("usage: seed_inbox_review <temporary-app-data-directory>");
    assert!(
        !app_data.join("piu.sqlite3").exists(),
        "review app data must start empty"
    );
    let repository_root = app_data.join("review-repositories");
    let atlas_path = repository_root.join("atlas-desktop-with-a-deliberately-long-project-name");
    let beacon_path = repository_root.join("beacon-indexer");
    initialize_repository(&atlas_path);
    initialize_repository(&beacon_path);

    let database_path = app_data.join("piu.sqlite3");
    let inbox = ProjectInbox::open(&database_path).expect("review inbox should open");
    let atlas = inbox
        .open_repository(&atlas_path)
        .expect("Atlas should open")
        .project;
    let beacon = inbox
        .open_repository(&beacon_path)
        .expect("Beacon should open")
        .project;
    inbox
        .save_draft(
            atlas.id,
            "Trace the importer boundary and explain why the fallback order changes.",
        )
        .expect("review draft should save");

    let chats = [
        ReviewChatFixture {
            id: "review-01",
            project_id: beacon.id,
            project_name: &beacon.name,
            title: "Repair repository indexing after an interrupted refresh",
            branch_name: "agent/review-01-repair-repository-indexing",
            pull_request_number: Some(73),
            created_at_ms: 1_730_000_000_400,
            merge_state: "unmerged",
        },
        ReviewChatFixture {
            id: "review-02",
            project_id: atlas.id,
            project_name: &atlas.name,
            title: "Keep this deliberately long chat title stable while transient details change",
            branch_name: "agent/review-02-stable-inbox-order",
            pull_request_number: Some(62),
            created_at_ms: 1_730_000_000_300,
            merge_state: "unmerged",
        },
        ReviewChatFixture {
            id: "review-03",
            project_id: atlas.id,
            project_name: &atlas.name,
            title: "Document the importer contract",
            branch_name: "docs/importer-contract",
            pull_request_number: None,
            created_at_ms: 1_730_000_000_200,
            merge_state: "unmerged",
        },
        ReviewChatFixture {
            id: "review-04",
            project_id: beacon.id,
            project_name: &beacon.name,
            title: "Preserve merged search history",
            branch_name: "agent/review-04-merged-history",
            pull_request_number: Some(41),
            created_at_ms: 1_730_000_000_100,
            merge_state: "merged",
        },
    ];
    drop(inbox);
    let connection = Connection::open(database_path).expect("review database should reopen");
    for fixture in chats {
        connection
            .execute(
                "INSERT INTO chats (
                    id, project_id, project_name, title, branch_name,
                    pull_request_number, created_at_ms, merge_state
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    fixture.id,
                    fixture.project_id,
                    fixture.project_name,
                    fixture.title,
                    fixture.branch_name,
                    fixture.pull_request_number,
                    fixture.created_at_ms,
                    fixture.merge_state,
                ],
            )
            .expect("review chat should seed");
    }

    fs::rename(&beacon_path, repository_root.join("beacon-indexer-moved"))
        .expect("review repository should move after admission");
    println!("{}", app_data.display());
}
