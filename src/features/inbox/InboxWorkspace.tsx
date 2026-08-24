import {
  ChevronDownIcon,
  FolderPlusIcon,
  MoreHorizontalIcon,
  SearchIcon,
  Trash2Icon,
} from "lucide-react";
import { memo, useCallback, useMemo, useRef, useState, useSyncExternalStore } from "react";

import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Empty, EmptyDescription, EmptyHeader, EmptyTitle } from "@/components/ui/empty";
import { Input } from "@/components/ui/input";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Separator } from "@/components/ui/separator";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import type { ChatSummary, InboxSnapshot, ProjectSummary } from "@/platform/project-inbox";

import { ChatComposer } from "./ChatComposer";
import { EmptyInbox } from "./EmptyInbox";
import { type DraftPersistenceStatus, ProjectDraftController } from "./draft-controller";
import { composerProject, selectInbox } from "./inbox-model";
import { SidebarResizeHandle } from "./SidebarResizeHandle";

interface InboxWorkspaceProps {
  actionError: string | undefined;
  drafts: ProjectDraftController;
  onOpenRepository: () => void;
  onQueryChange: (query: string) => void;
  onRemoveProject: (projectId: number) => Promise<string | undefined>;
  onSelectProject: (projectId: number | null) => void;
  query: string;
  selectedProjectId: number | null;
  snapshot: InboxSnapshot;
}

export type { DraftPersistenceStatus };

const ProjectFilter = memo(function ProjectFilter({
  onRemove,
  onSelect,
  project,
  selected,
}: {
  onRemove: (trigger: HTMLButtonElement) => void;
  onSelect: () => void;
  project: ProjectSummary;
  selected: boolean;
}) {
  const actionsTriggerRef = useRef<HTMLButtonElement>(null);
  const removalBlocked = project.unmergedChatCount > 0;
  const activeChatLabel = `${project.unmergedChatCount} active ${project.unmergedChatCount === 1 ? "chat" : "chats"}`;
  const availabilityLabel =
    project.availability === "available" ? "available" : "repository unavailable";
  const removalReasonId = `project-${project.id}-removal-reason`;

  return (
    <li className="project-row">
      <Button
        aria-label={`${project.name}, ${availabilityLabel}, ${activeChatLabel}`}
        aria-pressed={selected}
        className="project-row-select"
        onClick={onSelect}
        type="button"
        variant="ghost"
      >
        <span
          aria-hidden="true"
          className="project-availability"
          data-availability={project.availability}
        />
        <span className="project-row-copy">
          <span className="project-row-name" title={project.name}>
            {project.name}
          </span>
          <span>{project.availability === "available" ? activeChatLabel : availabilityLabel}</span>
        </span>
      </Button>

      <DropdownMenu>
        <DropdownMenuTrigger
          render={
            <Button
              aria-label={`Project actions for ${project.name}`}
              className="project-actions-trigger"
              ref={actionsTriggerRef}
              size="icon-sm"
              type="button"
              variant="ghost"
            />
          }
        >
          <MoreHorizontalIcon aria-hidden="true" />
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end" className="w-44">
          <DropdownMenuItem
            aria-describedby={removalBlocked ? removalReasonId : undefined}
            disabled={removalBlocked}
            onClick={() => {
              if (actionsTriggerRef.current) onRemove(actionsTriggerRef.current);
            }}
            variant="destructive"
          >
            <Trash2Icon aria-hidden="true" />
            Remove project
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>

      {removalBlocked ? (
        <span className="sr-only" id={removalReasonId}>
          Merge active chats before removing {project.name}.
        </span>
      ) : null}
    </li>
  );
});

const ChatRow = memo(function ChatRow({ chat }: { chat: ChatSummary }) {
  return (
    <li className="chat-row" data-chat-id={chat.id}>
      <div className="chat-row-copy">
        <h3 title={chat.title}>{chat.title}</h3>
        <p>
          <span>{chat.projectName}</span>
          <span aria-hidden="true">/</span>
          <span className="font-mono" title={chat.branchName}>
            {chat.branchName}
          </span>
        </p>
      </div>
      {chat.pullRequestNumber !== null ? (
        <Badge className="font-mono" variant="outline">
          #{chat.pullRequestNumber}
        </Badge>
      ) : null}
    </li>
  );
});

function DraftList({
  drafts,
  onSelectProject,
  projects,
  query,
  selectedProjectId,
}: {
  drafts: ProjectDraftController;
  onSelectProject: (projectId: number | null) => void;
  projects: ProjectSummary[];
  query: string;
  selectedProjectId: number | null;
}) {
  useSyncExternalStore(drafts.subscribeAll, drafts.getRevision, drafts.getRevision);
  const normalizedQuery = query.trim().toLocaleLowerCase();
  const visibleDrafts = projects.flatMap((project) => {
    if (selectedProjectId !== null && project.id !== selectedProjectId) return [];
    const draft = drafts.get(project.id);
    if (!draft.prompt) return [];
    if (
      normalizedQuery &&
      !`${project.name} ${draft.prompt}`.toLocaleLowerCase().includes(normalizedQuery)
    ) {
      return [];
    }
    return [{ draft, project }];
  });

  if (visibleDrafts.length === 0) return null;

  return (
    <section aria-labelledby="retained-drafts-heading">
      <div className="sidebar-section-heading">
        <span id="retained-drafts-heading">Drafts</span>
        <span className="font-mono">{visibleDrafts.length}</span>
      </div>
      <ul aria-label="Unsent drafts" className="draft-list">
        {visibleDrafts.map(({ draft, project }) => (
          <li key={project.id}>
            <Button
              className="draft-row"
              onClick={() => onSelectProject(project.id)}
              type="button"
              variant="ghost"
            >
              <span className="draft-row-project">{project.name}</span>
              <span className="draft-row-prompt">{draft.prompt}</span>
              {draft.status.state === "failed" ? (
                <span className="draft-row-failure">Not saved</span>
              ) : null}
            </Button>
            {draft.status.state === "failed" ? (
              <Button
                className="draft-row-retry"
                onClick={() => void drafts.retry(project.id)}
                size="sm"
                type="button"
                variant="ghost"
              >
                Retry
              </Button>
            ) : null}
          </li>
        ))}
      </ul>
    </section>
  );
}

function SearchStage({ count }: { count: number }) {
  return (
    <Empty className="stage-empty">
      <EmptyHeader>
        <EmptyTitle>{count === 0 ? "No matching chats" : `${count} matching chats`}</EmptyTitle>
        <EmptyDescription>
          {count === 0
            ? "Try a title, project, branch, or pull-request number."
            : "Search results are shown in the inbox."}
        </EmptyDescription>
      </EmptyHeader>
    </Empty>
  );
}

export function InboxWorkspace({
  actionError,
  drafts,
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
  const removalCancelRef = useRef<HTMLButtonElement>(null);
  const removalTriggerRef = useRef<HTMLButtonElement>(null);
  const selection = useMemo(
    () => selectInbox(snapshot, { projectId: selectedProjectId, query }),
    [query, selectedProjectId, snapshot],
  );
  const targetProject = composerProject(snapshot, selectedProjectId);
  const visibleChatCount = selection.unmergedChats.length;

  const closeRemovalDialog = useCallback(() => {
    setProjectPendingRemoval(undefined);
    setRemovalError(undefined);
  }, []);

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
    ? Boolean(drafts.get(projectPendingRemoval.id).prompt)
    : false;
  const totalSearchResults = selection.unmergedChats.length + selection.mergedChats.length;

  return (
    <main className="workspace" aria-label="Più inbox">
      <aside
        aria-label="Chat inbox navigation"
        className="inbox-sidebar"
        inert={projectPendingRemoval ? true : undefined}
      >
        <div className="sidebar-header">
          <div className="sidebar-title-row">
            <h1>Inbox</h1>
            <div className="sidebar-title-actions">
              <Badge aria-label={`${visibleChatCount} active chats`} variant="secondary">
                {visibleChatCount}
              </Badge>
              {snapshot.projects.length > 0 ? (
                <Tooltip>
                  <TooltipTrigger
                    render={
                      <Button
                        aria-label="Open Repository"
                        onClick={onOpenRepository}
                        size="icon"
                        type="button"
                        variant="ghost"
                      />
                    }
                  >
                    <FolderPlusIcon aria-hidden="true" />
                  </TooltipTrigger>
                  <TooltipContent side="right">Open Repository</TooltipContent>
                </Tooltip>
              ) : null}
            </div>
          </div>

          <label className="search-field">
            <span className="sr-only">Search chats</span>
            <SearchIcon aria-hidden="true" />
            <Input
              aria-label="Search chats"
              disabled={snapshot.projects.length === 0}
              onChange={(event) => onQueryChange(event.target.value)}
              placeholder="Search chats"
              type="search"
              value={query}
            />
          </label>

          {actionError && snapshot.projects.length > 0 ? (
            <p className="sidebar-error" role="alert">
              {actionError}
            </p>
          ) : null}
        </div>

        <ScrollArea className="sidebar-scroll-area">
          <div className="sidebar-scroll-content">
            <nav aria-label="Project filters" className="project-filters">
              <div className="sidebar-section-heading">
                <span>Projects</span>
              </div>
              <Button
                aria-label={`All Projects, ${snapshot.projects.length} ${snapshot.projects.length === 1 ? "project" : "projects"}`}
                aria-pressed={selectedProjectId === null}
                className="all-projects-filter"
                disabled={snapshot.projects.length === 0}
                onClick={() => onSelectProject(null)}
                type="button"
                variant="ghost"
              >
                <span>All Projects</span>
                <span className="font-mono">{snapshot.projects.length}</span>
              </Button>
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

            <Separator className="sidebar-separator" />

            <DraftList
              drafts={drafts}
              onSelectProject={onSelectProject}
              projects={snapshot.projects}
              query={query}
              selectedProjectId={selectedProjectId}
            />

            <section aria-labelledby="active-chats-heading" className="chat-list-section">
              <div className="sidebar-section-heading">
                <span id="active-chats-heading">Chats</span>
                <span className="font-mono">{visibleChatCount}</span>
              </div>
              {selection.unmergedChats.length > 0 ? (
                <ul aria-label="Active chats" className="chat-list">
                  {selection.unmergedChats.map((chat) => (
                    <ChatRow chat={chat} key={chat.id} />
                  ))}
                </ul>
              ) : (
                <p className="sidebar-zero-copy">
                  {query.trim() ? "No matching chats" : "No active chats"}
                </p>
              )}
            </section>

            {selection.mergedChats.length > 0 ? (
              <Collapsible className="merged-history">
                <CollapsibleTrigger
                  render={
                    <Button className="merged-history-trigger" type="button" variant="ghost" />
                  }
                >
                  <ChevronDownIcon aria-hidden="true" />
                  Merged
                  <span className="font-mono">{selection.mergedChats.length}</span>
                </CollapsibleTrigger>
                <CollapsibleContent>
                  <ul aria-label="Merged chats" className="chat-list chat-list-merged">
                    {selection.mergedChats.map((chat) => (
                      <ChatRow chat={chat} key={chat.id} />
                    ))}
                  </ul>
                </CollapsibleContent>
              </Collapsible>
            ) : null}
          </div>
        </ScrollArea>
      </aside>

      <SidebarResizeHandle disabled={snapshot.projects.length === 0} />

      <section
        aria-label="Chat workspace"
        className="conversation-stage"
        inert={projectPendingRemoval ? true : undefined}
      >
        {snapshot.projects.length === 0 ? (
          <EmptyInbox actionError={actionError} onOpenRepository={onOpenRepository} />
        ) : query.trim() ? (
          <SearchStage count={totalSearchResults} />
        ) : targetProject ? (
          <ChatComposer drafts={drafts} project={targetProject} />
        ) : null}
      </section>

      <AlertDialog
        onOpenChange={(open) => {
          if (!open && !removing) closeRemovalDialog();
        }}
        open={Boolean(projectPendingRemoval)}
      >
        {projectPendingRemoval ? (
          <AlertDialogContent finalFocus={removalTriggerRef} initialFocus={removalCancelRef}>
            <AlertDialogHeader>
              <AlertDialogTitle>Remove {projectPendingRemoval.name}?</AlertDialogTitle>
              <AlertDialogDescription>
                Più will forget this project. The repository on disk won&apos;t be changed.
                {pendingRemovalHasDraft ? (
                  <span className="mt-2 block">Its unsent draft will be deleted from Più.</span>
                ) : null}
              </AlertDialogDescription>
            </AlertDialogHeader>
            {removalError ? (
              <p className="dialog-error" role="alert">
                {removalError}
              </p>
            ) : null}
            <AlertDialogFooter>
              <AlertDialogCancel disabled={removing} ref={removalCancelRef}>
                Cancel
              </AlertDialogCancel>
              <AlertDialogAction
                disabled={removing}
                onClick={() => void confirmRemoval()}
                variant="destructive"
              >
                {removing ? "Removing" : "Remove project"}
              </AlertDialogAction>
            </AlertDialogFooter>
          </AlertDialogContent>
        ) : null}
      </AlertDialog>
    </main>
  );
}
