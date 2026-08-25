import {
  ChevronDownIcon,
  FolderPlusIcon,
  MoreHorizontalIcon,
  PencilIcon,
  SearchIcon,
  SettingsIcon,
  Trash2Icon,
} from "lucide-react";
import {
  lazy,
  memo,
  Suspense,
  type RefObject,
  useCallback,
  useMemo,
  useRef,
  useState,
  useSyncExternalStore,
} from "react";

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
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuTrigger,
} from "@/components/ui/context-menu";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
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
import type { ConversationAdapter } from "@/platform/conversations";
import type { PromptAttachment } from "@/platform/prompt-attachments";

import { ChatComposer } from "./ChatComposer";
import type { TranscriptViewState } from "../conversation/ConversationSurface";
import { ChatActivityController } from "./chat-activity-controller";
import { EmptyInbox } from "./EmptyInbox";
import { type DraftPersistenceStatus, ProjectDraftController } from "./draft-controller";
import { composerProject, selectInbox } from "./inbox-model";
import { ChatSetupPanel } from "./ChatSetupPanel";
import { ChatSetupController } from "./setup-controller";
import { SidebarResizeHandle } from "./SidebarResizeHandle";

interface InboxWorkspaceProps {
  actionError: string | undefined;
  activities: ChatActivityController;
  conversationAdapter: ConversationAdapter;
  conversationRevision: number;
  drafts: ProjectDraftController;
  onCancelSetup: (chatId: string) => Promise<string | undefined>;
  onCreateChat: (
    projectId: number,
    prompt: string,
    attachments: readonly PromptAttachment[],
  ) => Promise<string | undefined>;
  onOpenRepository: () => void;
  onOpenTerminal: (chatId: string) => Promise<string | undefined>;
  onOpenSettings: () => void;
  onRequestCodexSignIn: () => void;
  onQueryChange: (query: string) => void;
  onRemoveProject: (projectId: number) => Promise<string | undefined>;
  onRenameChat: (chatId: string, title: string) => Promise<string | undefined>;
  onRetrySetup: (chatId: string) => Promise<string | undefined>;
  onSelectChat: (chatId: string) => void;
  onSelectProject: (projectId: number | null) => void;
  query: string;
  selectedProjectId: number | null;
  selectedChatId: string | null;
  settingsTriggerRef?: RefObject<HTMLButtonElement | null>;
  setups: ChatSetupController;
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

const ChatConversationPanel = lazy(() => import("../conversation/ChatConversationPanel"));
const MAX_CACHED_TRANSCRIPT_STATES = 32;

class TranscriptStateCache {
  readonly #states = new Map<string, TranscriptViewState>();

  get(chatId: string) {
    return this.#states.get(chatId);
  }

  remember(chatId: string, state: TranscriptViewState) {
    this.#states.delete(chatId);
    this.#states.set(chatId, state);
    if (this.#states.size <= MAX_CACHED_TRANSCRIPT_STATES) return;
    const oldestChatId = this.#states.keys().next().value;
    if (oldestChatId) this.#states.delete(oldestChatId);
  }
}

function SelectedChatStage({
  chat,
  conversationAdapter,
  conversationRevision,
  initialTranscriptState,
  onCancelSetup,
  onOpenTerminal,
  onRequestCodexSignIn,
  onRetrySetup,
  rememberTranscriptState,
  setups,
}: {
  chat: ChatSummary;
  conversationAdapter: ConversationAdapter;
  conversationRevision: number;
  initialTranscriptState?: TranscriptViewState;
  onCancelSetup: (chatId: string) => Promise<string | undefined>;
  onOpenTerminal: (chatId: string) => Promise<string | undefined>;
  onRequestCodexSignIn: () => void;
  onRetrySetup: (chatId: string) => Promise<string | undefined>;
  rememberTranscriptState: (chatId: string, state: TranscriptViewState) => void;
  setups: ChatSetupController;
}) {
  const subscribe = useCallback(
    (listener: () => void) => setups.subscribe(chat.id, listener),
    [chat.id, setups],
  );
  const getSnapshot = useCallback(() => setups.get(chat.id), [chat.id, setups]);
  const setup = useSyncExternalStore(subscribe, getSnapshot, getSnapshot);
  const ready = setup.phase === "notRequired" || setup.phase === "succeeded";
  const saveTranscriptState = useCallback(
    (state: TranscriptViewState) => rememberTranscriptState(chat.id, state),
    [chat.id, rememberTranscriptState],
  );

  if (!ready) {
    return (
      <ChatSetupPanel
        chat={chat}
        onCancel={onCancelSetup}
        onOpenTerminal={onOpenTerminal}
        onRetry={onRetrySetup}
        setups={setups}
      />
    );
  }

  return (
    <Suspense
      fallback={
        <div aria-busy="true" className="conversation-connection" role="status">
          Opening chat
        </div>
      }
    >
      <ChatConversationPanel
        adapter={conversationAdapter}
        chatId={chat.id}
        initialTranscriptState={initialTranscriptState}
        key={chat.id}
        onRequestCodexSignIn={onRequestCodexSignIn}
        onTranscriptStateChange={saveTranscriptState}
        revision={conversationRevision}
      />
    </Suspense>
  );
}

const ChatRow = memo(function ChatRow({
  activities,
  chat,
  onRename,
  onSelect,
  selected,
  setups,
}: {
  activities: ChatActivityController;
  chat: ChatSummary;
  onRename: (trigger: HTMLButtonElement) => void;
  onSelect: () => void;
  selected: boolean;
  setups: ChatSetupController;
}) {
  const selectTriggerRef = useRef<HTMLButtonElement>(null);
  const subscribe = useCallback(
    (listener: () => void) => setups.subscribe(chat.id, listener),
    [chat.id, setups],
  );
  const getSnapshot = useCallback(() => setups.get(chat.id), [chat.id, setups]);
  const setup = useSyncExternalStore(subscribe, getSnapshot, getSnapshot);
  const activityStore = activities.chat(chat.id);
  const activity = useSyncExternalStore(
    activityStore.subscribe,
    activityStore.getSnapshot,
    activityStore.getSnapshot,
  );
  const rowPhase =
    setup.phase === "succeeded" || setup.phase === "notRequired" ? activity.phase : setup.phase;
  return (
    <li
      className="chat-row"
      data-activity={activity.phase}
      data-chat-id={chat.id}
      data-unread={activity.unread || undefined}
    >
      <ContextMenu>
        <ContextMenuTrigger className="chat-row-context-trigger">
          <Button
            aria-label={`${chat.title}, ${rowPhase}${activity.unread ? ", unread" : ""}`}
            aria-pressed={selected}
            className="chat-row-select"
            onClick={onSelect}
            ref={selectTriggerRef}
            type="button"
            variant="ghost"
          >
            <span aria-hidden="true" className="chat-setup-indicator" data-phase={rowPhase} />
            <span className="chat-row-copy">
              <span className="chat-row-title" id={`chat-${chat.id}-title`} title={chat.title}>
                {chat.title}
              </span>
              <span className="chat-row-metadata">
                <span>{chat.projectName}</span>
                <span aria-hidden="true">/</span>
                <span className="font-mono" title={chat.branchName}>
                  {chat.branchName}
                </span>
              </span>
            </span>
            {chat.pullRequestNumber !== null ? (
              <Badge className="font-mono" variant="outline">
                #{chat.pullRequestNumber}
              </Badge>
            ) : null}
          </Button>
        </ContextMenuTrigger>
        <ContextMenuContent className="w-40">
          <ContextMenuItem
            onClick={() => {
              if (selectTriggerRef.current) onRename(selectTriggerRef.current);
            }}
          >
            <PencilIcon aria-hidden="true" />
            Rename chat
          </ContextMenuItem>
        </ContextMenuContent>
      </ContextMenu>

      <DropdownMenu>
        <DropdownMenuTrigger
          render={
            <Button
              aria-describedby={`chat-${chat.id}-title`}
              aria-label="More chat actions"
              className="chat-actions-trigger"
              size="icon-sm"
              type="button"
              variant="ghost"
            />
          }
        >
          <MoreHorizontalIcon aria-hidden="true" />
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end" className="w-40">
          <DropdownMenuItem
            onClick={() => {
              if (selectTriggerRef.current) onRename(selectTriggerRef.current);
            }}
          >
            <PencilIcon aria-hidden="true" />
            Rename chat
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
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
    if (!draft.prompt && draft.attachments.length === 0) return [];
    const attachmentNames = draft.attachments.map(({ name }) => name).join(" ");
    if (
      normalizedQuery &&
      !`${project.name} ${draft.prompt} ${attachmentNames}`
        .toLocaleLowerCase()
        .includes(normalizedQuery)
    ) {
      return [];
    }
    const summary =
      draft.prompt ||
      (draft.attachments.length === 1
        ? `Attached ${draft.attachments[0].name}`
        : `${draft.attachments.length} attached files`);
    return [{ draft, project, summary }];
  });

  if (visibleDrafts.length === 0) return null;

  return (
    <section aria-labelledby="retained-drafts-heading">
      <div className="sidebar-section-heading">
        <span id="retained-drafts-heading">Drafts</span>
        <span className="font-mono">{visibleDrafts.length}</span>
      </div>
      <ul aria-label="Unsent drafts" className="draft-list">
        {visibleDrafts.map(({ draft, project, summary }) => (
          <li key={project.id}>
            <Button
              className="draft-row"
              onClick={() => onSelectProject(project.id)}
              type="button"
              variant="ghost"
            >
              <span className="draft-row-project">{project.name}</span>
              <span className="draft-row-prompt">{summary}</span>
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
  activities,
  conversationAdapter,
  conversationRevision,
  drafts,
  onCancelSetup,
  onCreateChat,
  onOpenRepository,
  onOpenTerminal,
  onOpenSettings,
  onRequestCodexSignIn,
  onQueryChange,
  onRemoveProject,
  onRenameChat,
  onRetrySetup,
  onSelectChat,
  onSelectProject,
  query,
  selectedProjectId,
  selectedChatId,
  settingsTriggerRef,
  setups,
  snapshot,
}: InboxWorkspaceProps) {
  const [projectPendingRemoval, setProjectPendingRemoval] = useState<ProjectSummary>();
  const [removalError, setRemovalError] = useState<string>();
  const [removing, setRemoving] = useState(false);
  const removalCancelRef = useRef<HTMLButtonElement>(null);
  const removalTriggerRef = useRef<HTMLButtonElement>(null);
  const [chatPendingRename, setChatPendingRename] = useState<ChatSummary>();
  const [renameTitle, setRenameTitle] = useState("");
  const [renameError, setRenameError] = useState<string>();
  const [renaming, setRenaming] = useState(false);
  const renameInputRef = useRef<HTMLInputElement>(null);
  const renameTriggerRef = useRef<HTMLButtonElement>(null);
  const transcriptStates = useMemo(() => new TranscriptStateCache(), []);
  const selection = useMemo(
    () => selectInbox(snapshot, { projectId: selectedProjectId, query }),
    [query, selectedProjectId, snapshot],
  );
  const targetProject = composerProject(snapshot, selectedProjectId);
  const selectedChat = snapshot.chats.find(({ id }) => id === selectedChatId);
  const visibleChatCount = selection.unmergedChats.length;
  const rememberTranscriptState = useCallback(
    (chatId: string, state: TranscriptViewState) => {
      transcriptStates.remember(chatId, state);
    },
    [transcriptStates],
  );

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

  const closeRenameDialog = useCallback(() => {
    setChatPendingRename(undefined);
    setRenameError(undefined);
  }, []);

  const confirmRename = useCallback(async () => {
    if (!chatPendingRename || !renameTitle.trim()) return;
    setRenaming(true);
    setRenameError(undefined);
    const error = await onRenameChat(chatPendingRename.id, renameTitle);
    setRenaming(false);
    if (error) setRenameError(error);
    else closeRenameDialog();
  }, [chatPendingRename, closeRenameDialog, onRenameChat, renameTitle]);

  const pendingRemovalHasDraft = projectPendingRemoval
    ? Boolean(
        drafts.get(projectPendingRemoval.id).prompt ||
        drafts.get(projectPendingRemoval.id).attachments.length,
      )
    : false;
  const totalSearchResults = selection.unmergedChats.length + selection.mergedChats.length;

  return (
    <main className="workspace" aria-label="Più inbox">
      <aside
        aria-label="Chat inbox navigation"
        className="inbox-sidebar"
        inert={projectPendingRemoval || chatPendingRename ? true : undefined}
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
                    <ChatRow
                      activities={activities}
                      chat={chat}
                      key={chat.id}
                      onRename={(trigger) => {
                        renameTriggerRef.current = trigger;
                        setRenameTitle(chat.title);
                        setRenameError(undefined);
                        setChatPendingRename(chat);
                      }}
                      onSelect={() => onSelectChat(chat.id)}
                      selected={selectedChatId === chat.id}
                      setups={setups}
                    />
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
                      <ChatRow
                        activities={activities}
                        chat={chat}
                        key={chat.id}
                        onRename={(trigger) => {
                          renameTriggerRef.current = trigger;
                          setRenameTitle(chat.title);
                          setRenameError(undefined);
                          setChatPendingRename(chat);
                        }}
                        onSelect={() => onSelectChat(chat.id)}
                        selected={selectedChatId === chat.id}
                        setups={setups}
                      />
                    ))}
                  </ul>
                </CollapsibleContent>
              </Collapsible>
            ) : null}
          </div>
        </ScrollArea>
        <footer className="sidebar-footer">
          <Button
            className="sidebar-settings-action"
            onClick={onOpenSettings}
            ref={settingsTriggerRef}
            type="button"
            variant="ghost"
          >
            <SettingsIcon aria-hidden="true" data-icon="inline-start" />
            Settings
          </Button>
        </footer>
      </aside>

      <SidebarResizeHandle disabled={snapshot.projects.length === 0} />

      <section
        aria-label="Chat workspace"
        className="conversation-stage"
        inert={projectPendingRemoval || chatPendingRename ? true : undefined}
      >
        {snapshot.projects.length === 0 ? (
          <EmptyInbox actionError={actionError} onOpenRepository={onOpenRepository} />
        ) : query.trim() ? (
          <SearchStage count={totalSearchResults} />
        ) : selectedChat ? (
          <SelectedChatStage
            chat={selectedChat}
            conversationAdapter={conversationAdapter}
            conversationRevision={conversationRevision}
            initialTranscriptState={transcriptStates.get(selectedChat.id)}
            onCancelSetup={onCancelSetup}
            onOpenTerminal={onOpenTerminal}
            onRequestCodexSignIn={onRequestCodexSignIn}
            onRetrySetup={onRetrySetup}
            rememberTranscriptState={rememberTranscriptState}
            setups={setups}
          />
        ) : targetProject ? (
          <ChatComposer
            drafts={drafts}
            key={targetProject.id}
            onSubmit={onCreateChat}
            project={targetProject}
          />
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

      <Dialog
        onOpenChange={(open) => {
          if (!open && !renaming) closeRenameDialog();
        }}
        open={Boolean(chatPendingRename)}
      >
        {chatPendingRename ? (
          <DialogContent
            finalFocus={renameTriggerRef}
            initialFocus={renameInputRef}
            showCloseButton={false}
          >
            <form
              className="grid gap-4"
              onSubmit={(event) => {
                event.preventDefault();
                void confirmRename();
              }}
            >
              <DialogHeader>
                <DialogTitle>Rename chat</DialogTitle>
                <DialogDescription>
                  The branch and worktree keep their current names.
                </DialogDescription>
              </DialogHeader>
              <label className="grid gap-1.5">
                <span className="text-xs font-medium">Title</span>
                <Input
                  aria-invalid={Boolean(renameError)}
                  maxLength={72}
                  onChange={(event) => {
                    setRenameTitle(event.target.value);
                    setRenameError(undefined);
                  }}
                  onFocus={(event) => event.currentTarget.select()}
                  ref={renameInputRef}
                  value={renameTitle}
                />
              </label>
              {renameError ? (
                <p className="dialog-error" role="alert">
                  {renameError}
                </p>
              ) : null}
              <DialogFooter>
                <Button
                  disabled={renaming}
                  onClick={closeRenameDialog}
                  type="button"
                  variant="outline"
                >
                  Cancel
                </Button>
                <Button disabled={renaming || !renameTitle.trim()} type="submit">
                  {renaming ? "Saving" : "Save"}
                </Button>
              </DialogFooter>
            </form>
          </DialogContent>
        ) : null}
      </Dialog>
    </main>
  );
}
