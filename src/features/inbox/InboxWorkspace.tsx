import {
  ChevronDownIcon,
  FolderIcon,
  FolderPlusIcon,
  MoreHorizontalIcon,
  PencilIcon,
  SearchIcon,
  SettingsIcon,
  SquarePenIcon,
  Trash2Icon,
} from "lucide-react";
import {
  Fragment,
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
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Input } from "@/components/ui/input";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import type { ChatSummary, InboxSnapshot, ProjectSummary } from "@/platform/project-inbox";
import type { ConversationAdapter } from "@/platform/conversations";
import type { PromptAttachment } from "@/platform/prompt-attachments";
import type { ModelControlsAdapter } from "@/platform/model-controls";
import type { ModelRouteId } from "@/generated/ModelRouteId";
import type { ReasoningEffort } from "@/generated/ReasoningEffort";
import { recordInboxRender } from "#inbox-performance-review";

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
  chatModelControlsAdapter?: ModelControlsAdapter;
  conversationAdapter: ConversationAdapter;
  conversationRevision: number;
  drafts: ProjectDraftController;
  modelControlsAdapter?: ModelControlsAdapter<number>;
  onCancelSetup: (chatId: string) => Promise<string | undefined>;
  onCreateChat: (
    projectId: number,
    prompt: string,
    attachments: readonly PromptAttachment[],
    route: ModelRouteId,
    effort: ReasoningEffort,
  ) => Promise<string | undefined>;
  onDeleteChat: (chatId: string) => Promise<string | undefined>;
  onNewChat: () => void;
  onOpenRepository: () => void;
  onOpenTerminal: (chatId: string) => Promise<string | undefined>;
  onOpenSettings: () => void;
  onRequestCodexSignIn: () => void;
  onProjectScopeChange: (projectId: number | null) => void;
  onQueryChange: (query: string) => void;
  onRenameChat: (chatId: string, title: string) => Promise<string | undefined>;
  onRetrySetup: (chatId: string) => Promise<string | undefined>;
  onSelectChat: (chatId: string) => void;
  query: string;
  selectedProjectId: number | null;
  selectedChatId: string | null;
  settingsTriggerRef?: RefObject<HTMLButtonElement | null>;
  setups: ChatSetupController;
  snapshot: InboxSnapshot;
}

export type { DraftPersistenceStatus };

const ALL_PROJECTS_SCOPE = "all";

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
  cacheOwner,
  chat,
  chatModelControlsAdapter,
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
  cacheOwner: object;
  chat: ChatSummary;
  chatModelControlsAdapter?: ModelControlsAdapter;
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
        cacheOwner={cacheOwner}
        chatId={chat.id}
        initialTranscriptState={initialTranscriptState}
        key={chat.id}
        modelControlsAdapter={chatModelControlsAdapter}
        onRequestCodexSignIn={onRequestCodexSignIn}
        onTranscriptStateChange={saveTranscriptState}
        revision={conversationRevision}
      />
    </Suspense>
  );
}

const CHAT_ACTIONS = [
  {
    destructive: false,
    Icon: PencilIcon,
    id: "rename",
    label: "Rename chat",
    separatorBefore: false,
  },
  {
    destructive: true,
    Icon: Trash2Icon,
    id: "delete",
    label: "Delete chat",
    separatorBefore: true,
  },
] as const;

type ChatActionId = (typeof CHAT_ACTIONS)[number]["id"];
const MERGED_CHAT_ACTIONS = CHAT_ACTIONS.slice(0, 1);

function availableChatActions(chat: ChatSummary) {
  return chat.mergeState === "merged" ? MERGED_CHAT_ACTIONS : CHAT_ACTIONS;
}

const CHAT_PHASE_LABELS = {
  cancelled: "Cancelled",
  failed: "Failed",
  finished: "Finished",
  idle: "Idle",
  interrupted: "Interrupted",
  "needs-input": "Needs input",
  notRequired: "Idle",
  pending: "Preparing",
  running: "Running",
  succeeded: "Idle",
} as const;

const ChatRow = memo(function ChatRow({
  activities,
  chat,
  onAction,
  onSelect,
  selected,
  showProjectIdentity,
  setups,
}: {
  activities: ChatActivityController;
  chat: ChatSummary;
  onAction: (action: ChatActionId, chat: ChatSummary, trigger: HTMLButtonElement) => void;
  onSelect: (chatId: string) => void;
  selected: boolean;
  showProjectIdentity: boolean;
  setups: ChatSetupController;
}) {
  recordInboxRender?.({ id: chat.id, kind: "chat-row" });
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
  const transientStatus = rowPhase === "idle" ? null : CHAT_PHASE_LABELS[rowPhase];
  const actions = availableChatActions(chat);
  const compact = !showProjectIdentity && !transientStatus;
  const openNativeMenu = useCallback(
    async (position?: { x: number; y: number }) => {
      const trigger = selectTriggerRef.current;
      if (!trigger) return;
      trigger.focus({ preventScroll: true });
      const { popupNativeContextMenu } = await import("@/platform/native-context-menu");
      await popupNativeContextMenu({
        actions: availableChatActions(chat),
        onAction: (action) => onAction(action, chat, trigger),
        position,
      });
    },
    [chat, onAction],
  );
  const requestNativeMenu = useCallback(
    (position?: { x: number; y: number }) => {
      void openNativeMenu(position).catch(() => undefined);
    },
    [openNativeMenu],
  );

  return (
    <li
      className="chat-row"
      data-activity={rowPhase}
      data-chat-id={chat.id}
      data-compact={compact || undefined}
      data-unread={activity.unread || undefined}
    >
      <Button
        aria-label={`${chat.title}, ${rowPhase}${activity.unread ? ", unread" : ""}`}
        aria-pressed={selected}
        className="chat-row-select"
        onClick={() => onSelect(chat.id)}
        onContextMenu={(event) => {
          event.preventDefault();
          event.stopPropagation();
          requestNativeMenu();
        }}
        onKeyDown={(event) => {
          if (event.key !== "ContextMenu" && !(event.shiftKey && event.key === "F10")) return;
          event.preventDefault();
          event.stopPropagation();
          const bounds = event.currentTarget.getBoundingClientRect();
          requestNativeMenu({
            x: Math.round(bounds.left + Math.min(16, bounds.width / 2)),
            y: Math.round(bounds.top + Math.min(28, bounds.height)),
          });
        }}
        ref={selectTriggerRef}
        type="button"
        variant="ghost"
      >
        <span className="chat-row-copy">
          {showProjectIdentity || transientStatus ? (
            <span className="chat-row-eyebrow">
              {showProjectIdentity ? (
                <span className="chat-row-project">{chat.projectName}</span>
              ) : null}
              {transientStatus ? (
                <span className="chat-row-status" data-phase={rowPhase}>
                  {transientStatus}
                </span>
              ) : null}
            </span>
          ) : null}
          <span className="chat-row-title" id={`chat-${chat.id}-title`} title={chat.title}>
            {chat.title}
          </span>
          <span className="chat-row-metadata">
            <span className="chat-row-branch font-mono" title={chat.branchName}>
              {chat.branchName}
            </span>
            {chat.pullRequestNumber !== null ? (
              <Badge className="font-mono" variant="outline">
                #{chat.pullRequestNumber}
              </Badge>
            ) : null}
          </span>
        </span>
      </Button>

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
          {actions.map((action) => (
            <Fragment key={action.id}>
              {action.separatorBefore ? <DropdownMenuSeparator /> : null}
              <DropdownMenuItem
                onClick={() => {
                  if (selectTriggerRef.current) onAction(action.id, chat, selectTriggerRef.current);
                }}
                variant={action.destructive ? "destructive" : "default"}
              >
                <action.Icon aria-hidden="true" />
                {action.label}
              </DropdownMenuItem>
            </Fragment>
          ))}
        </DropdownMenuContent>
      </DropdownMenu>
    </li>
  );
});

const ProjectScopeControl = memo(function ProjectScopeControl({
  onOpenRepository,
  onProjectScopeChange,
  projects,
  scopeProject,
  selectedProjectId,
}: {
  onOpenRepository: () => void;
  onProjectScopeChange: (projectId: number | null) => void;
  projects: readonly ProjectSummary[];
  scopeProject: ProjectSummary | undefined;
  selectedProjectId: number | null;
}) {
  recordInboxRender?.({ kind: "scope-control" });

  return (
    <div className="sidebar-scope-row">
      <DropdownMenu>
        <DropdownMenuTrigger
          render={
            <Button
              aria-label={`Project scope: ${scopeProject?.name ?? "All Projects"}`}
              className="project-scope-trigger"
              type="button"
              variant="ghost"
            />
          }
        >
          <FolderIcon aria-hidden="true" />
          <span>{scopeProject?.name ?? "All Projects"}</span>
          <ChevronDownIcon aria-hidden="true" />
        </DropdownMenuTrigger>
        <DropdownMenuContent align="start" className="project-scope-menu">
          <DropdownMenuRadioGroup
            onValueChange={(value) =>
              onProjectScopeChange(value === ALL_PROJECTS_SCOPE ? null : Number(value))
            }
            value={selectedProjectId === null ? ALL_PROJECTS_SCOPE : String(selectedProjectId)}
          >
            <DropdownMenuRadioItem closeOnClick value={ALL_PROJECTS_SCOPE}>
              <FolderIcon aria-hidden="true" />
              <span>All Projects</span>
            </DropdownMenuRadioItem>
            {projects.map((project) => (
              <DropdownMenuRadioItem closeOnClick key={project.id} value={String(project.id)}>
                <FolderIcon aria-hidden="true" />
                <span>{project.name}</span>
              </DropdownMenuRadioItem>
            ))}
          </DropdownMenuRadioGroup>
        </DropdownMenuContent>
      </DropdownMenu>

      <Tooltip>
        <TooltipTrigger
          render={
            <Button
              aria-label="Open Repository"
              onClick={onOpenRepository}
              size="icon-lg"
              type="button"
              variant="ghost"
            />
          }
        >
          <FolderPlusIcon aria-hidden="true" />
        </TooltipTrigger>
        <TooltipContent side="right">Open Repository</TooltipContent>
      </Tooltip>
    </div>
  );
});

function ProjectDraftRow({
  drafts,
  onSelect,
  project,
}: {
  drafts: ProjectDraftController;
  onSelect: () => void;
  project: ProjectSummary;
}) {
  useSyncExternalStore(drafts.subscribeAll, drafts.getRevision, drafts.getRevision);
  const draft = drafts.get(project.id);
  if (!draft.prompt && draft.attachments.length === 0) return null;
  const summary =
    draft.prompt ||
    (draft.attachments.length === 1
      ? `Attached ${draft.attachments[0].name}`
      : `${draft.attachments.length} attached files`);

  return (
    <li className="draft-row-shell" data-draft-project-id={project.id}>
      <Button className="draft-row" onClick={onSelect} type="button" variant="ghost">
        <span className="draft-row-project">Draft</span>
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
  );
}

export function InboxWorkspace({
  actionError,
  activities,
  chatModelControlsAdapter,
  conversationAdapter,
  conversationRevision,
  drafts,
  modelControlsAdapter,
  onCancelSetup,
  onCreateChat,
  onDeleteChat,
  onNewChat,
  onOpenRepository,
  onOpenTerminal,
  onOpenSettings,
  onRequestCodexSignIn,
  onProjectScopeChange,
  onQueryChange,
  onRenameChat,
  onRetrySetup,
  onSelectChat,
  query,
  selectedProjectId,
  selectedChatId,
  settingsTriggerRef,
  setups,
  snapshot,
}: InboxWorkspaceProps) {
  const [chatPendingRename, setChatPendingRename] = useState<ChatSummary>();
  const [renameTitle, setRenameTitle] = useState("");
  const [renameError, setRenameError] = useState<string>();
  const [renaming, setRenaming] = useState(false);
  const renameInputRef = useRef<HTMLInputElement>(null);
  const renameTriggerRef = useRef<HTMLButtonElement>(null);
  const [chatPendingDeletion, setChatPendingDeletion] = useState<ChatSummary>();
  const [deletionError, setDeletionError] = useState<string>();
  const [deleting, setDeleting] = useState(false);
  const deletionCancelRef = useRef<HTMLButtonElement>(null);
  const deletionFinalFocusRef = useRef<HTMLButtonElement>(null);
  const deletionNeighborChatIdRef = useRef<string | undefined>(undefined);
  const newChatTriggerRef = useRef<HTMLButtonElement>(null);
  const transcriptStates = useMemo(() => new TranscriptStateCache(), []);
  const conversationCacheOwner = useMemo(
    () => ({ chatModelControlsAdapter, conversationAdapter }),
    [chatModelControlsAdapter, conversationAdapter],
  );
  const scopeProject = snapshot.projects.find(({ id }) => id === selectedProjectId);
  const selection = useMemo(
    () => selectInbox(snapshot, { projectId: selectedProjectId, query }),
    [query, selectedProjectId, snapshot],
  );
  const targetProject = composerProject(snapshot, selectedProjectId);
  const selectedChat = snapshot.chats.find(({ id }) => id === selectedChatId);

  const rememberTranscriptState = useCallback(
    (chatId: string, state: TranscriptViewState) => {
      transcriptStates.remember(chatId, state);
    },
    [transcriptStates],
  );

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

  const closeDeletionDialog = useCallback(() => {
    setChatPendingDeletion(undefined);
    setDeletionError(undefined);
  }, []);

  const handleChatAction = useCallback(
    (action: ChatActionId, chat: ChatSummary, trigger: HTMLButtonElement) => {
      if (action === "rename") {
        renameTriggerRef.current = trigger;
        setRenameTitle(chat.title);
        setRenameError(undefined);
        setChatPendingRename(chat);
        return;
      }

      const visibleChats =
        chat.mergeState === "merged" ? selection.mergedChats : selection.unmergedChats;
      const chatIndex = visibleChats.findIndex(({ id }) => id === chat.id);
      deletionNeighborChatIdRef.current =
        visibleChats[chatIndex + 1]?.id ?? visibleChats[chatIndex - 1]?.id;
      deletionFinalFocusRef.current = trigger;
      setDeletionError(undefined);
      setChatPendingDeletion(chat);
    },
    [selection.mergedChats, selection.unmergedChats],
  );

  const confirmDeletion = useCallback(async () => {
    if (!chatPendingDeletion) return;
    setDeleting(true);
    setDeletionError(undefined);
    const error = await onDeleteChat(chatPendingDeletion.id);
    setDeleting(false);
    if (error) {
      setDeletionError(error);
      return;
    }

    const { invalidateCachedChatConversationSession } =
      await import("../conversation/chat-conversation-session-cache");
    invalidateCachedChatConversationSession(conversationCacheOwner, chatPendingDeletion.id);

    const neighborChatId = deletionNeighborChatIdRef.current;
    const neighborRow = neighborChatId
      ? [...document.querySelectorAll<HTMLElement>("[data-chat-id]")].find(
          ({ dataset }) => dataset.chatId === neighborChatId,
        )
      : undefined;
    deletionFinalFocusRef.current =
      neighborRow?.querySelector<HTMLButtonElement>(".chat-row-select") ??
      newChatTriggerRef.current;
    closeDeletionDialog();
  }, [chatPendingDeletion, closeDeletionDialog, conversationCacheOwner, onDeleteChat]);

  return (
    <main className="workspace" aria-label="Più inbox">
      <aside
        aria-label="Chat inbox navigation"
        className="inbox-sidebar"
        inert={chatPendingRename || chatPendingDeletion ? true : undefined}
      >
        <div className="sidebar-header">
          <h1 className="sr-only">Inbox</h1>
          <div className="sidebar-search-row">
            <label className="search-field">
              <span className="sr-only">Search chats</span>
              <SearchIcon aria-hidden="true" />
              <Input
                aria-label="Search chats"
                disabled={snapshot.projects.length === 0}
                onChange={(event) => onQueryChange(event.target.value)}
                onKeyDown={(event) => {
                  if (event.key !== "Escape" || !query) return;
                  event.preventDefault();
                  onQueryChange("");
                  event.currentTarget.focus();
                }}
                placeholder="Search chats"
                type="search"
                value={query}
              />
            </label>
            <Button
              className="sidebar-new-chat-action"
              disabled={!targetProject}
              onClick={onNewChat}
              ref={newChatTriggerRef}
              size="icon-lg"
              type="button"
              variant="ghost"
              aria-label="New Chat"
            >
              <SquarePenIcon aria-hidden="true" />
            </Button>
          </div>

          {snapshot.projects.length > 0 ? (
            <ProjectScopeControl
              onOpenRepository={onOpenRepository}
              onProjectScopeChange={onProjectScopeChange}
              projects={snapshot.projects}
              scopeProject={scopeProject}
              selectedProjectId={selectedProjectId}
            />
          ) : null}

          {actionError && snapshot.projects.length > 0 ? (
            <p className="sidebar-error" role="alert">
              {actionError}
            </p>
          ) : null}
        </div>

        <ScrollArea className="sidebar-scroll-area">
          <div className="sidebar-scroll-content">
            {selection.unmergedChats.length > 0 || (scopeProject && !query.trim()) ? (
              <ul aria-label="Chat inbox" className="chat-list">
                {scopeProject && !query.trim() ? (
                  <ProjectDraftRow drafts={drafts} onSelect={onNewChat} project={scopeProject} />
                ) : null}
                {selection.unmergedChats.map((chat) => (
                  <ChatRow
                    activities={activities}
                    chat={chat}
                    key={chat.id}
                    onAction={handleChatAction}
                    onSelect={onSelectChat}
                    selected={selectedChatId === chat.id}
                    setups={setups}
                    showProjectIdentity={selectedProjectId === null}
                  />
                ))}
              </ul>
            ) : (
              <p className="sidebar-zero-copy">
                {query.trim() ? "No matching chats" : "No active chats"}
              </p>
            )}

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
                        onAction={handleChatAction}
                        onSelect={onSelectChat}
                        selected={selectedChatId === chat.id}
                        setups={setups}
                        showProjectIdentity={selectedProjectId === null}
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
        data-selected-chat-id={selectedChat?.id}
        inert={chatPendingRename || chatPendingDeletion ? true : undefined}
      >
        {snapshot.projects.length === 0 ? (
          <EmptyInbox actionError={actionError} onOpenRepository={onOpenRepository} />
        ) : selectedChat ? (
          <SelectedChatStage
            cacheOwner={conversationCacheOwner}
            chat={selectedChat}
            chatModelControlsAdapter={chatModelControlsAdapter}
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
            modelControlsAdapter={modelControlsAdapter}
            onRequestCodexSignIn={onRequestCodexSignIn}
            onSubmit={onCreateChat}
            project={targetProject}
            revision={conversationRevision}
          />
        ) : null}
      </section>

      <AlertDialog
        onOpenChange={(open) => {
          if (!open && !deleting) closeDeletionDialog();
        }}
        open={Boolean(chatPendingDeletion)}
      >
        {chatPendingDeletion ? (
          <AlertDialogContent
            finalFocus={() => deletionFinalFocusRef.current}
            initialFocus={deletionCancelRef}
          >
            <AlertDialogHeader>
              <AlertDialogTitle>Delete “{chatPendingDeletion.title}”?</AlertDialogTitle>
              <AlertDialogDescription>
                This permanently deletes the local conversation, managed worktree, and local branch.
                It won&apos;t close a pull request or delete a remote branch.
                <span className="mt-2 block">Any active agent will be stopped first.</span>
              </AlertDialogDescription>
            </AlertDialogHeader>
            {deletionError ? (
              <p className="dialog-error" role="alert">
                {deletionError}
              </p>
            ) : null}
            <AlertDialogFooter>
              <AlertDialogCancel disabled={deleting} ref={deletionCancelRef}>
                Cancel
              </AlertDialogCancel>
              <AlertDialogAction
                disabled={deleting}
                onClick={() => void confirmDeletion()}
                variant="destructive"
              >
                {deleting ? "Deleting" : "Delete chat"}
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
