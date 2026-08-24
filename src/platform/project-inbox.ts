import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import type { ProjectCommandError } from "../generated/ProjectCommandError";
import type { ProjectCommandErrorCode } from "../generated/ProjectCommandErrorCode";
import type { ProjectInboxChangedEvent } from "../generated/ProjectInboxChangedEvent";

export type { ChatSummary } from "../generated/ChatSummary";
export type { DraftSummary } from "../generated/DraftSummary";
export type { InboxSnapshot } from "../generated/InboxSnapshot";
export type { OpenRepositoryResult } from "../generated/OpenRepositoryResult";
export type { ProjectSummary } from "../generated/ProjectSummary";

import type { InboxSnapshot } from "../generated/InboxSnapshot";
import type { DraftSummary } from "../generated/DraftSummary";
import type { OpenRepositoryResponse } from "../generated/OpenRepositoryResponse";

const PROJECT_INBOX_CHANGED_EVENT = "project-inbox://changed";
const PROJECT_COMMAND_ERROR_CODES = new Set<ProjectCommandErrorCode>([
  "invalidRepository",
  "repositoryInaccessible",
  "projectHasUnmergedChats",
  "projectNotFound",
  "repositoryInspectionFailed",
  "storageUnavailable",
]);

export function loadProjectInbox() {
  return invoke<InboxSnapshot>("load_project_inbox");
}

export function openRepository(path: string) {
  return invoke<OpenRepositoryResponse>("open_repository", { request: { path } });
}

export function saveProjectDraft(projectId: number, prompt: string) {
  return invoke<DraftSummary>("save_project_draft", {
    request: { projectId, prompt },
  });
}

export function removeProject(projectId: number) {
  return invoke<InboxSnapshot>("remove_project", { request: { projectId } });
}

export function listenToProjectInbox(onChange: (event: ProjectInboxChangedEvent) => void) {
  return listen<ProjectInboxChangedEvent>(PROJECT_INBOX_CHANGED_EVENT, ({ payload }) => {
    onChange(payload);
  });
}

export function projectErrorMessage(error: unknown, fallback: string) {
  if (
    typeof error === "object" &&
    error !== null &&
    "code" in error &&
    PROJECT_COMMAND_ERROR_CODES.has((error as ProjectCommandError).code) &&
    "message" in error &&
    typeof (error as ProjectCommandError).message === "string"
  ) {
    return (error as ProjectCommandError).message;
  }
  return fallback;
}
