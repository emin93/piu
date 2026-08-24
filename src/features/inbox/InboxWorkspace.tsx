import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import type { ChatSummary, InboxSnapshot, ProjectSummary } from "../../platform/project-inbox";
import { EmptyInbox } from "./EmptyInbox";
import { projectDraft, selectInbox } from "./inbox-model";

interface InboxWorkspaceProps {
  actionError: string | undefined;
  draftStatus: DraftPersistenceStatus;
  onDraftChange: (projectId: number, prompt: string) => void;
  onOpenRepository: () => void;
  onQueryChange: (query: string) => void;
  onRemoveProject: (projectId: number) => Promise<string | undefined>;
  onSelectProject: (projectId: number | null) => void;
  query: string;
  selectedProjectId: number | null;
  snapshot: InboxSnapshot;
}

export type DraftPersistenceStatus =
  { state: "idle" | "saving" | "saved" } | { state: "failed"; message: string };

function RemoveIcon() {
  return (
    <svg aria-hidden="true" viewBox="0 0 16 16">
      <path d="M4.5 4.5h7M6.2 4.5V3.2h3.6v1.3m.9 0-.5 8.2H5.8l-.5-8.2M7 6.4v4.4m2-4.4v4.4" />
    </svg>
  );
}

function SearchIcon() {
  return (
    <svg aria-hidden="true" viewBox="0 0 16 16">
      <circle cx="6.8" cy="6.8" r="4.1" />
      <path d="m9.8 9.8 3.2 3.2" />
    </svg>
  );
}

function ProjectFilter({
  project,
  selected,
  onSelect,
  onRemove,
}: {
  project: ProjectSummary;
  selected: boolean;
  onSelect: () => void;
  onRemove: (trigger: HTMLButtonElement) => void;
}) {
  const removalBlocked = project.unmergedChatCount > 0;
  const activeChatLabel = `${project.unmergedChatCount} active ${project.unmergedChatCount === 1 ? "chat" : "chats"}`;
  const availabilityLabel =
    project.availability === "available" ? "available" : "repository unavailable";
  const removalReasonId = `project-${project.id}-removal-reason`;
  return (
    <li className="project-filter">
      <button
        aria-label={`${project.name}, ${availabilityLabel}, ${activeChatLabel}`}
        aria-pressed={selected}
        className="project-filter__select"
        onClick={onSelect}
        type="button"
      >
        <span
          aria-hidden="true"
          className={`project-filter__status project-filter__status--${project.availability}`}
        />
        <span className="project-filter__copy">
          <span className="project-filter__name" title={project.name}>
            {project.name}
          </span>
          {project.availability === "available" ? (
            <span>
              {project.unmergedChatCount} active{" "}
              {project.unmergedChatCount === 1 ? "chat" : "chats"}
            </span>
          ) : (
            <span>Repository unavailable</span>
          )}
        </span>
      </button>
      <button
        aria-describedby={removalBlocked ? removalReasonId : undefined}
        aria-disabled={removalBlocked}
        aria-label={`Remove ${project.name}`}
        className="project-filter__remove"
        onClick={(event) => {
          if (!removalBlocked) onRemove(event.currentTarget);
        }}
        type="button"
      >
        <RemoveIcon />
      </button>
      {removalBlocked && (
        <span className="visually-hidden" id={removalReasonId}>
          Merge active chats before removing {project.name}.
        </span>
      )}
    </li>
  );
}

function ChatRow({ chat }: { chat: ChatSummary }) {
  return (
    <li className="chat-row" data-chat-id={chat.id}>
      <div className="chat-row__main">
        <h3 title={chat.title}>{chat.title}</h3>
        <p>
          <span>{chat.projectName}</span>
          <span aria-hidden="true">/</span>
          <span className="chat-row__branch" title={chat.branchName}>
            {chat.branchName}
          </span>
        </p>
      </div>
      {chat.pullRequestNumber !== null && (
        <span className="chat-row__pr">PR #{chat.pullRequestNumber}</span>
      )}
    </li>
  );
}

export function InboxWorkspace({
  actionError,
  draftStatus,
  onDraftChange,
  onOpenRepository,
  onQueryChange,
  onRemoveProject,
  onSelectProject,
  query,
  selectedProjectId,
  snapshot,
}: InboxWorkspaceProps) {
  const [projectPendingRemoval, setProjectPendingRemoval] = useState<ProjectSummary>();
  const [removalError, setRemovalError] = useState<string>();
  const [removing, setRemoving] = useState(false);
  const dialogRef = useRef<HTMLElement>(null);
  const removalTriggerRef = useRef<HTMLButtonElement | undefined>(undefined);
  const dialogWasOpen = useRef(false);
  const selection = useMemo(
    () => selectInbox(snapshot, { projectId: selectedProjectId, query }),
    [query, selectedProjectId, snapshot],
  );
  const selectedProject = snapshot.projects.find(({ id }) => id === selectedProjectId);
  const selectedDraft = selectedProject ? projectDraft(snapshot, selectedProject.id) : undefined;
  const visibleChatCount = selection.unmergedChats.length;

  const closeRemovalDialog = useCallback(() => {
    setProjectPendingRemoval(undefined);
    setRemovalError(undefined);
  }, []);

  useEffect(() => {
    if (!projectPendingRemoval) {
      if (dialogWasOpen.current) removalTriggerRef.current?.focus();
      dialogWasOpen.current = false;
      return;
    }
    dialogWasOpen.current = true;
    dialogRef.current?.querySelector<HTMLElement>("[data-dialog-initial-focus]")?.focus();
    const containFocus = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !removing) {
        event.preventDefault();
        closeRemovalDialog();
        return;
      }
      if (event.key !== "Tab") return;
      const controls = [
        ...(dialogRef.current?.querySelectorAll<HTMLElement>("button") ?? []),
      ].filter((element) => !element.hasAttribute("disabled"));
      if (controls.length === 0) return;
      const first = controls[0];
      const last = controls[controls.length - 1];
      if (event.shiftKey && document.activeElement === first) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && document.activeElement === last) {
        event.preventDefault();
        first.focus();
      } else if (!dialogRef.current?.contains(document.activeElement)) {
        event.preventDefault();
        first.focus();
      }
    };
    window.addEventListener("keydown", containFocus);
    return () => window.removeEventListener("keydown", containFocus);
  }, [closeRemovalDialog, projectPendingRemoval, removing]);

  const confirmRemoval = useCallback(async () => {
    if (!projectPendingRemoval) return;
    setRemoving(true);
    setRemovalError(undefined);
    const error = await onRemoveProject(projectPendingRemoval.id);
    setRemoving(false);
    if (error) setRemovalError(error);
    else closeRemovalDialog();
  }, [closeRemovalDialog, onRemoveProject, projectPendingRemoval]);

  const pendingRemovalHasDraft = projectPendingRemoval
    ? Boolean(projectDraft(snapshot, projectPendingRemoval.id)?.prompt)
    : false;

  const draftStatusLabel =
    draftStatus.state === "saving"
      ? "Saving…"
      : draftStatus.state === "saved"
        ? "Saved locally"
        : draftStatus.state === "failed"
          ? "Not saved"
          : "Not saved yet";

  return (
    <main className="workspace" aria-label="Più inbox">
      <aside
        className="inbox-rail"
        aria-label="Chat inbox navigation"
        inert={projectPendingRemoval ? true : undefined}
      >
        <div className="inbox-rail__top">
          <div className="inbox-rail__heading">
            <div>
              <p className="inbox-rail__label">Workspace</p>
              <h1>Inbox</h1>
            </div>
            <span aria-label={`${visibleChatCount} active chats`}>{visibleChatCount}</span>
          </div>
          {snapshot.projects.length > 0 && (
            <>
              <div className="inbox-search">
                <SearchIcon />
                <input
                  aria-label="Search chats"
                  onChange={(event) => onQueryChange(event.target.value)}
                  placeholder="Search chats"
                  type="search"
                  value={query}
                />
              </div>
              <button className="rail-open-action" onClick={onOpenRepository} type="button">
                <span aria-hidden="true">+</span>
                Open Repository
              </button>
            </>
          )}
          {actionError && snapshot.projects.length > 0 && (
            <p className="inline-error" role="alert">
              {actionError}
            </p>
          )}
          {snapshot.projects.length > 0 && (
            <nav aria-label="Project filters" className="project-filters">
              <p className="project-filters__label">Projects</p>
              <button
                aria-label={`All Projects, ${snapshot.projects.length} ${snapshot.projects.length === 1 ? "project" : "projects"}`}
                aria-pressed={selectedProjectId === null}
                className="all-projects-filter"
                onClick={() => onSelectProject(null)}
                type="button"
              >
                <span>All Projects</span>
                <span>
                  {snapshot.projects.length}{" "}
                  {snapshot.projects.length === 1 ? "project" : "projects"}
                </span>
              </button>
              <ul>
                {snapshot.projects.map((project) => (
                  <ProjectFilter
                    key={project.id}
                    onRemove={(trigger) => {
                      removalTriggerRef.current = trigger;
                      setRemovalError(undefined);
                      setProjectPendingRemoval(project);
                    }}
                    onSelect={() => onSelectProject(project.id)}
                    project={project}
                    selected={selectedProjectId === project.id}
                  />
                ))}
              </ul>
            </nav>
          )}
        </div>
        <p className="inbox-rail__hint">
          Unmerged chats stay in the inbox until their pull request merges.
        </p>
      </aside>

      <section
        className={`conversation-stage${snapshot.projects.length > 0 ? " conversation-stage--populated" : ""}`}
        aria-label="Chat inbox"
        inert={projectPendingRemoval ? true : undefined}
      >
        {snapshot.projects.length === 0 ? (
          <EmptyInbox actionError={actionError} onOpenRepository={onOpenRepository} />
        ) : (
          <div className="inbox-content">
            <header className="inbox-content__header">
              <div>
                <p className="inbox-content__eyebrow">
                  {selectedProject ? selectedProject.name : "Every project"}
                </p>
                <h2>Chat inbox</h2>
              </div>
              <span>{visibleChatCount} active</span>
            </header>

            {selectedProject?.availability !== "available" && selectedProject && (
              <div className="repository-warning" role="alert">
                <strong>Repository unavailable</strong>
                <span>Move it back or restore access before starting new work.</span>
              </div>
            )}
            {selectedProject && !query.trim() && (
              <section className="draft-card" aria-labelledby="draft-heading">
                <div className="draft-card__header">
                  <div>
                    <p>Unsent draft</p>
                    <h3 id="draft-heading">Start something in {selectedProject.name}</h3>
                  </div>
                  <span
                    aria-live="polite"
                    className={`draft-status draft-status--${draftStatus.state}`}
                  >
                    {draftStatusLabel}
                  </span>
                </div>
                <textarea
                  aria-label={`Draft for ${selectedProject.name}`}
                  disabled={selectedProject.availability !== "available"}
                  onChange={(event) => onDraftChange(selectedProject.id, event.target.value)}
                  placeholder="Describe what you want to change…"
                  rows={4}
                  value={selectedDraft?.prompt ?? ""}
                />
                {draftStatus.state === "failed" && (
                  <p className="inline-error" role="alert">
                    {draftStatus.message}
                  </p>
                )}
              </section>
            )}

            {!selectedProject && !query.trim() && selection.drafts.length > 0 && (
              <section className="retained-drafts" aria-labelledby="retained-drafts-heading">
                <div className="section-heading">
                  <h3 id="retained-drafts-heading">Unsent drafts</h3>
                  <span>{selection.drafts.length}</span>
                </div>
                <div className="retained-drafts__grid">
                  {selection.drafts.map((draft) => {
                    const project = snapshot.projects.find(({ id }) => id === draft.projectId);
                    if (!project) return null;
                    return (
                      <button
                        className="retained-draft"
                        key={draft.projectId}
                        onClick={() => onSelectProject(draft.projectId)}
                        type="button"
                      >
                        <span>{project.name}</span>
                        <strong>{draft.prompt}</strong>
                      </button>
                    );
                  })}
                </div>
              </section>
            )}

            <section className="chat-list-section" aria-labelledby="active-chats-heading">
              <div className="section-heading">
                <h3 id="active-chats-heading">Active chats</h3>
                <span>{visibleChatCount}</span>
              </div>
              {selection.unmergedChats.length > 0 ? (
                <ul aria-label="Active chats" className="chat-list">
                  {selection.unmergedChats.map((chat) => (
                    <ChatRow chat={chat} key={chat.id} />
                  ))}
                </ul>
              ) : (
                <div className="inbox-zero-state">
                  <h3>{query.trim() ? "No matching chats" : "No active chats"}</h3>
                  <p>
                    {query.trim()
                      ? "Try a title, project, branch, or pull-request number."
                      : selectedProject
                        ? "Your draft is ready whenever you are."
                        : "Choose a project to begin a draft."}
                  </p>
                </div>
              )}
            </section>

            {selection.mergedChats.length > 0 && (
              <details className="merged-history">
                <summary>Merged history · {selection.mergedChats.length}</summary>
                <ul aria-label="Merged chats" className="chat-list">
                  {selection.mergedChats.map((chat) => (
                    <ChatRow chat={chat} key={chat.id} />
                  ))}
                </ul>
              </details>
            )}
          </div>
        )}
      </section>

      {projectPendingRemoval && (
        <div className="dialog-backdrop">
          <section
            aria-labelledby="remove-project-title"
            aria-modal="true"
            className="remove-project-dialog"
            ref={dialogRef}
            role="dialog"
          >
            <p className="inbox-content__eyebrow">Remove project</p>
            <h2 id="remove-project-title">Remove {projectPendingRemoval.name}?</h2>
            <p className="remove-project-dialog__description">
              Più will forget this project. The repository on disk won’t be changed.
            </p>
            {pendingRemovalHasDraft && (
              <p className="remove-project-dialog__draft-warning">
                Its unsent draft will be deleted from Più.
              </p>
            )}
            {removalError && (
              <p className="inline-error remove-project-dialog__error" role="alert">
                {removalError}
              </p>
            )}
            <div className="remove-project-dialog__actions">
              <button
                className="secondary-action"
                data-dialog-initial-focus
                disabled={removing}
                onClick={closeRemovalDialog}
                type="button"
              >
                Cancel
              </button>
              <button
                className="danger-action"
                disabled={removing}
                onClick={() => void confirmRemoval()}
                type="button"
              >
                {removing ? "Removing…" : "Remove project"}
              </button>
            </div>
          </section>
        </div>
      )}
    </main>
  );
}
