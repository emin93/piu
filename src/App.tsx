import { lazy, Suspense, useCallback, useEffect, useRef, useState } from "react";
import { LoaderCircleIcon } from "lucide-react";

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
import { Button } from "@/components/ui/button";
import {
  Empty,
  EmptyContent,
  EmptyDescription,
  EmptyHeader,
  EmptyTitle,
} from "@/components/ui/empty";
import { Skeleton } from "@/components/ui/skeleton";
import { TooltipProvider } from "@/components/ui/tooltip";
import type { ModelRouteId } from "@/generated/ModelRouteId";
import type { ReasoningEffort } from "@/generated/ReasoningEffort";
import { DeferredSurface, type DeferredSurfaceName } from "./features/deferred/DeferredSurface";
import { ProjectDraftController } from "./features/inbox/draft-controller";
import { ChatActivityController } from "./features/inbox/chat-activity-controller";
import { selectInbox } from "./features/inbox/inbox-model";
import { readRememberedProjectScope, rememberProjectScope } from "./features/inbox/inbox-scope";
import { InboxWorkspace } from "./features/inbox/InboxWorkspace";
import { ChatSetupController } from "./features/inbox/setup-controller";
import { useSystemAppearance } from "./hooks/use-system-appearance";
import {
  cancelChatSetup,
  chatWorkspaceErrorMessage,
  createChat,
  listenToChatSetup,
  openChatTerminal,
  retryChatSetup,
} from "./platform/chat-workspaces";
import { verifyHostBoundary } from "./platform/host-boundary";
import {
  type InboxSnapshot,
  deleteChat,
  listenToProjectInbox,
  loadProjectInbox,
  openRepository,
  projectErrorMessage,
  renameChat,
  saveProjectDraft,
} from "./platform/project-inbox";
import { selectRepositoryDirectory } from "./platform/repository-picker";
import {
  exitApplication,
  hasActiveAgentTurn,
  shutdownRuntimeProcesses,
} from "./platform/runtime-lifecycle";
import { listenToWindowClose } from "./platform/window-lifecycle";
import {
  type ConversationAdapter,
  listenToConversationEvents,
  tauriConversationAdapter,
} from "./platform/conversations";
import type { PromptAttachment } from "./platform/prompt-attachments";

interface AppProps {
  conversationAdapter?: ConversationAdapter;
  onOpenRepository?: () => void;
  surface?: "inbox" | DeferredSurfaceName;
  visualReviewState?:
    "closeConfirmation" | "connectionRecovery" | "conversation" | "loading" | "sendRecovery";
}

const EMPTY_INBOX: InboxSnapshot = { projects: [], drafts: [], chats: [] };
const CodexSignInDialog = lazy(() => import("./features/auth/CodexSignInDialog"));

function validRememberedProjectScope(snapshot: InboxSnapshot) {
  const rememberedProjectId = readRememberedProjectScope();
  if (
    rememberedProjectId === null ||
    snapshot.projects.some((project) => project.id === rememberedProjectId)
  ) {
    return rememberedProjectId;
  }
  rememberProjectScope(null);
  return null;
}

function StartupFailure({ onRetry }: { onRetry: () => void }) {
  return (
    <Empty aria-labelledby="startup-failure-title" className="startup-state">
      <EmptyHeader>
        <EmptyTitle id="startup-failure-title">Più couldn&apos;t start</EmptyTitle>
        <EmptyDescription>
          Something interrupted startup. Retry to continue without changing your work.
        </EmptyDescription>
      </EmptyHeader>
      <EmptyContent>
        <Button onClick={onRetry} type="button">
          Retry
        </Button>
      </EmptyContent>
    </Empty>
  );
}

function StartupLoading() {
  return (
    <section className="startup-loading" aria-live="polite" role="status">
      <div className="startup-loading-copy">
        <Skeleton className="h-3 w-20" />
        <Skeleton className="h-5 w-32" />
      </div>
      <span className="sr-only">Opening your inbox</span>
    </section>
  );
}

export function App({
  conversationAdapter = tauriConversationAdapter,
  onOpenRepository,
  surface = "inbox",
  visualReviewState,
}: AppProps) {
  const appearance = useSystemAppearance();
  const [activeSurface, setActiveSurface] = useState<"inbox" | DeferredSurfaceName>(surface);
  const [hostStatus, setHostStatus] = useState<"checking" | "ready" | "failed">("checking");
  const [snapshot, setSnapshot] = useState<InboxSnapshot>(EMPTY_INBOX);
  const [selectedProjectId, setSelectedProjectId] = useState<number | null>(
    readRememberedProjectScope,
  );
  const [selectedChatId, setSelectedChatId] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [repositoryActionError, setRepositoryActionError] = useState<string>();
  const [codexSignInOpen, setCodexSignInOpen] = useState(false);
  const [closeConfirmationOpen, setCloseConfirmationOpen] = useState(false);
  const [applicationClosing, setApplicationClosing] = useState(false);
  const [conversationRevision, setConversationRevision] = useState(0);
  const verificationGeneration = useRef(0);
  const settingsTriggerRef = useRef<HTMLButtonElement>(null);
  const keepWorkingRef = useRef<HTMLButtonElement>(null);
  const restoreSettingsFocus = useRef(false);
  const [setups] = useState(() => new ChatSetupController());
  const [activities] = useState(() => new ChatActivityController());
  const [drafts] = useState(() => {
    const controller = new ProjectDraftController(
      async (projectId, prompt, attachments) => {
        await saveProjectDraft(projectId, prompt, attachments);
        setSnapshot((current) => controller.overlay(current));
      },
      {
        toFailureMessage: (error) =>
          projectErrorMessage(error, "Couldn't save this draft. Keep Più open and try again."),
      },
    );
    return controller;
  });

  const completeStartup = useCallback(
    (generation: number) => {
      void Promise.all([verifyHostBoundary(), loadProjectInbox()]).then(
        ([, loadedSnapshot]) => {
          if (verificationGeneration.current !== generation) return;
          drafts.reconcile(loadedSnapshot);
          setups.reconcile(loadedSnapshot);
          setSnapshot(drafts.overlay(loadedSnapshot));
          setSelectedProjectId(validRememberedProjectScope(loadedSnapshot));
          if (
            visualReviewState === "connectionRecovery" ||
            visualReviewState === "conversation" ||
            visualReviewState === "sendRecovery"
          ) {
            setSelectedChatId(loadedSnapshot.chats[0]?.id ?? null);
          }
          setHostStatus("ready");
        },
        () => {
          if (verificationGeneration.current === generation) setHostStatus("failed");
        },
      );
    },
    [drafts, setups, visualReviewState],
  );

  const retryStartup = useCallback(() => {
    const generation = ++verificationGeneration.current;
    setHostStatus("checking");
    completeStartup(generation);
  }, [completeStartup]);

  const flushAllDrafts = useCallback(() => drafts.flushAll(), [drafts]);
  const closeApplication = useCallback(async () => {
    await flushAllDrafts();
    await shutdownRuntimeProcesses();
  }, [flushAllDrafts]);

  const requestApplicationClose = useCallback(async () => {
    if (await hasActiveAgentTurn()) {
      setCloseConfirmationOpen(true);
      return;
    }
    await closeApplication();
    await exitApplication();
  }, [closeApplication]);

  const confirmApplicationClose = useCallback(async () => {
    setApplicationClosing(true);
    try {
      await closeApplication();
      await exitApplication();
    } catch {
      // Draft persistence explains its own failure; keep Più open so the user can recover.
    } finally {
      setApplicationClosing(false);
    }
  }, [closeApplication]);

  const openSettings = useCallback(() => {
    void flushAllDrafts().catch(() => undefined);
    restoreSettingsFocus.current = true;
    setActiveSurface("settings");
  }, [flushAllDrafts]);

  const closeDeferredSurface = useCallback(() => setActiveSurface("inbox"), []);
  const openCodexSignIn = useCallback(() => setCodexSignInOpen(true), []);
  const completeCodexSignIn = useCallback(() => {
    setCodexSignInOpen(false);
    setConversationRevision((current) => current + 1);
  }, []);

  useEffect(() => {
    if (activeSurface !== "inbox" || !restoreSettingsFocus.current) return;
    restoreSettingsFocus.current = false;
    settingsTriggerRef.current?.focus();
  }, [activeSurface]);

  const openSelectedRepository = useCallback(() => {
    void flushAllDrafts().catch(() => undefined);
    setRepositoryActionError(undefined);
    if (onOpenRepository) {
      onOpenRepository();
      return;
    }
    void (async () => {
      let path: string | null;
      try {
        path = await selectRepositoryDirectory();
      } catch {
        setRepositoryActionError("Couldn't open the repository picker. Try again.");
        return;
      }
      if (!path) return;
      try {
        const opened = await openRepository(path);
        drafts.reconcile(opened.snapshot);
        setups.reconcile(opened.snapshot);
        setSnapshot(drafts.overlay(opened.snapshot));
        rememberProjectScope(opened.focusedProjectId);
        setSelectedProjectId(opened.focusedProjectId);
        setSelectedChatId(null);
        setQuery("");
      } catch (error: unknown) {
        setRepositoryActionError(
          projectErrorMessage(error, "Couldn't open that repository. Try again."),
        );
      }
    })();
  }, [drafts, flushAllDrafts, onOpenRepository, setups]);

  const changeProjectScope = useCallback(
    (projectId: number | null) => {
      void flushAllDrafts().catch(() => undefined);
      rememberProjectScope(projectId);
      setSelectedProjectId(projectId);
    },
    [flushAllDrafts],
  );

  const startNewChat = useCallback(() => {
    void flushAllDrafts().catch(() => undefined);
    setSelectedChatId(null);
    setQuery("");
  }, [flushAllDrafts]);

  const selectChat = useCallback(
    (chatId: string) => {
      void flushAllDrafts().catch(() => undefined);
      setSelectedChatId(chatId);
      setQuery("");
    },
    [flushAllDrafts],
  );

  const createProjectChat = useCallback(
    async (
      projectId: number,
      prompt: string,
      attachments: readonly PromptAttachment[],
      route: ModelRouteId,
      effort: ReasoningEffort,
    ) => {
      await drafts.flush(projectId);
      try {
        const created = await createChat(projectId, prompt, attachments, route, effort);
        drafts.forget(projectId);
        setups.reconcile(created.snapshot);
        setSnapshot(created.snapshot);
        setSelectedChatId(created.chat.id);
        setQuery("");
        return undefined;
      } catch (error: unknown) {
        return chatWorkspaceErrorMessage(
          error,
          "Più couldn’t prepare this chat. Check the repository and try again.",
        );
      }
    },
    [drafts, setups],
  );

  const retrySetup = useCallback(
    async (chatId: string) => {
      try {
        const setup = await retryChatSetup(chatId);
        setups.apply({ chatId, setup });
        return undefined;
      } catch (error: unknown) {
        return chatWorkspaceErrorMessage(error, "Più couldn’t retry setup. Try again.");
      }
    },
    [setups],
  );

  const cancelSetup = useCallback(async (chatId: string) => {
    try {
      await cancelChatSetup(chatId);
      return undefined;
    } catch (error: unknown) {
      return chatWorkspaceErrorMessage(error, "Più couldn’t cancel setup. Try again.");
    }
  }, []);

  const openTerminal = useCallback(async (chatId: string) => {
    try {
      await openChatTerminal(chatId);
      return undefined;
    } catch (error: unknown) {
      return chatWorkspaceErrorMessage(error, "Più couldn’t open the chat terminal. Try again.");
    }
  }, []);

  const changeQuery = useCallback(
    (nextQuery: string) => {
      if (!query && nextQuery) void flushAllDrafts().catch(() => undefined);
      setQuery(nextQuery);
    },
    [flushAllDrafts, query],
  );

  const renameSelectedChat = useCallback(async (chatId: string, title: string) => {
    try {
      const nextSnapshot = await renameChat(chatId, title);
      setSnapshot(nextSnapshot);
      return undefined;
    } catch (error: unknown) {
      return projectErrorMessage(error, "Couldn't rename that chat. Try again.");
    }
  }, []);

  const deleteSelectedChat = useCallback(
    async (chatId: string) => {
      const previousChats = selectInbox(snapshot, {
        projectId: selectedProjectId,
        query,
      });
      const previousOrder = [...previousChats.unmergedChats, ...previousChats.mergedChats];
      const deletedIndex = previousOrder.findIndex((chat) => chat.id === chatId);
      try {
        const nextSnapshot = await deleteChat(chatId);
        drafts.reconcile(nextSnapshot);
        setups.reconcile(nextSnapshot);
        const nextWithDrafts = drafts.overlay(nextSnapshot);
        setSnapshot(nextWithDrafts);
        setSelectedChatId((current) => {
          if (current !== chatId) return current;
          const nextChats = selectInbox(nextWithDrafts, {
            projectId: selectedProjectId,
            query,
          });
          const nextOrder = [...nextChats.unmergedChats, ...nextChats.mergedChats];
          if (nextOrder.length === 0) return null;
          const neighborIndex = deletedIndex < 0 ? 0 : Math.min(deletedIndex, nextOrder.length - 1);
          return nextOrder[neighborIndex]?.id ?? null;
        });
        return undefined;
      } catch (error: unknown) {
        return projectErrorMessage(error, "Couldn't delete that chat. Try again.");
      }
    },
    [drafts, query, selectedProjectId, setups, snapshot],
  );

  useEffect(() => {
    if (visualReviewState === "loading") return;
    const generation = ++verificationGeneration.current;
    completeStartup(generation);
    return () => {
      verificationGeneration.current += 1;
    };
  }, [completeStartup, visualReviewState]);

  useEffect(() => {
    if (visualReviewState !== "closeConfirmation" || hostStatus !== "ready") return;
    const timeout = window.setTimeout(() => setCloseConfirmationOpen(true), 0);
    return () => window.clearTimeout(timeout);
  }, [hostStatus, visualReviewState]);

  useEffect(() => {
    let disposed = false;
    let stopListening: (() => void) | undefined;
    void listenToProjectInbox((event) => {
      if (disposed) return;
      drafts.reconcile(event.snapshot);
      setups.reconcile(event.snapshot);
      setSnapshot(drafts.overlay(event.snapshot));
      if (event.focusedProjectId !== null) {
        rememberProjectScope(event.focusedProjectId);
        setSelectedProjectId(event.focusedProjectId);
      } else {
        setSelectedProjectId(validRememberedProjectScope(event.snapshot));
      }
    })
      .then((unlisten) => {
        if (disposed) unlisten();
        else stopListening = unlisten;
      })
      .catch(() => undefined);
    return () => {
      disposed = true;
      stopListening?.();
    };
  }, [drafts, setups]);

  useEffect(() => {
    activities.reconcile(snapshot.chats.map((chat) => chat.id));
  }, [activities, snapshot.chats]);

  useEffect(() => {
    const visibleChatId = activeSurface === "inbox" ? selectedChatId : null;
    activities.select(visibleChatId);
  }, [activeSurface, activities, selectedChatId]);

  useEffect(() => {
    let disposed = false;
    let stopListening: (() => void) | undefined;
    void listenToConversationEvents((chatId, event) => {
      if (disposed) return;
      switch (event.type) {
        case "turn-started":
        case "turn-completed":
        case "turn-failed":
        case "turn-interrupted":
        case "turn-stopped":
          activities.apply(chatId, event);
          break;
        case "input-requested":
          activities.apply(chatId, { type: "needs-input" });
          break;
        case "input-resolved":
          activities.apply(chatId, { type: "input-resolved" });
          break;
      }
    })
      .then((unlisten) => {
        if (disposed) unlisten();
        else stopListening = unlisten;
      })
      .catch(() => undefined);
    return () => {
      disposed = true;
      stopListening?.();
    };
  }, [activities]);

  useEffect(() => {
    let disposed = false;
    let stopListening: (() => void) | undefined;
    void listenToChatSetup((event) => {
      if (!disposed) setups.apply(event);
    })
      .then((unlisten) => {
        if (disposed) unlisten();
        else stopListening = unlisten;
      })
      .catch(() => undefined);
    return () => {
      disposed = true;
      stopListening?.();
    };
  }, [setups]);

  useEffect(() => {
    const flush = () => void flushAllDrafts().catch(() => undefined);
    window.addEventListener("blur", flush);
    window.addEventListener("pagehide", flush);
    return () => {
      window.removeEventListener("blur", flush);
      window.removeEventListener("pagehide", flush);
      void flushAllDrafts().catch(() => undefined);
    };
  }, [flushAllDrafts]);

  useEffect(() => {
    let disposed = false;
    let stopListening: (() => void) | undefined;
    void listenToWindowClose(requestApplicationClose).then((unlisten) => {
      if (disposed) unlisten();
      else stopListening = unlisten;
    });
    return () => {
      disposed = true;
      stopListening?.();
    };
  }, [requestApplicationClose]);

  const selectedProject = snapshot.projects.find(({ id }) => id === selectedProjectId);
  const settingsProject =
    selectedProject ?? snapshot.projects.find(({ availability }) => availability === "available");
  const selectedChat = snapshot.chats.find(({ id }) => id === selectedChatId);
  const titlebarContext =
    activeSurface === "settings"
      ? "Models & Resources"
      : (selectedChat?.title ?? selectedProject?.name ?? "All Projects");

  return (
    <TooltipProvider>
      <div className="app-shell" data-appearance={appearance}>
        <header className="titlebar" data-tauri-drag-region="deep">
          <div aria-label="Più" className="wordmark">
            <span aria-hidden="true" className="wordmark-symbol">
              π
            </span>
            <span>Più</span>
          </div>
          <div className="titlebar-context">{titlebarContext}</div>
        </header>
        {hostStatus === "checking" ? (
          <main className="startup-workspace">
            <StartupLoading />
          </main>
        ) : hostStatus === "failed" ? (
          <main className="startup-workspace">
            <StartupFailure onRetry={retryStartup} />
          </main>
        ) : activeSurface === "inbox" ? (
          <InboxWorkspace
            activities={activities}
            actionError={repositoryActionError}
            conversationAdapter={conversationAdapter}
            conversationRevision={conversationRevision}
            drafts={drafts}
            onCancelSetup={cancelSetup}
            onCreateChat={createProjectChat}
            onDeleteChat={deleteSelectedChat}
            onNewChat={startNewChat}
            onOpenRepository={openSelectedRepository}
            onOpenTerminal={openTerminal}
            onOpenSettings={openSettings}
            onRequestCodexSignIn={openCodexSignIn}
            onProjectScopeChange={changeProjectScope}
            onQueryChange={changeQuery}
            onRenameChat={renameSelectedChat}
            onRetrySetup={retrySetup}
            onSelectChat={selectChat}
            query={query}
            selectedChatId={selectedChatId}
            selectedProjectId={selectedProjectId}
            settingsTriggerRef={settingsTriggerRef}
            setups={setups}
            snapshot={snapshot}
          />
        ) : (
          <main className="deferred-workspace">
            <div className="conversation-stage">
              <DeferredSurface
                onClose={closeDeferredSurface}
                project={settingsProject}
                surface={activeSurface}
              />
            </div>
          </main>
        )}
        {codexSignInOpen ? (
          <Suspense
            fallback={
              <div className="lazy-dialog-backdrop">
                <div aria-live="polite" className="lazy-dialog-card" role="status">
                  <LoaderCircleIcon aria-hidden="true" />
                  Opening Codex sign-in
                </div>
              </div>
            }
          >
            <CodexSignInDialog
              onComplete={completeCodexSignIn}
              onOpenChange={setCodexSignInOpen}
              open={codexSignInOpen}
            />
          </Suspense>
        ) : null}
        <AlertDialog
          onOpenChange={(open) => {
            if (!applicationClosing) setCloseConfirmationOpen(open);
          }}
          open={closeConfirmationOpen}
        >
          <AlertDialogContent initialFocus={keepWorkingRef}>
            <AlertDialogHeader>
              <AlertDialogTitle>Stop active work and quit?</AlertDialogTitle>
              <AlertDialogDescription>
                Active agent work will be stopped. Your conversation and draft will remain available
                when you reopen Più.
              </AlertDialogDescription>
            </AlertDialogHeader>
            <AlertDialogFooter>
              <AlertDialogCancel disabled={applicationClosing} ref={keepWorkingRef}>
                Keep working
              </AlertDialogCancel>
              <AlertDialogAction
                disabled={applicationClosing}
                onClick={() => void confirmApplicationClose()}
                variant="destructive"
              >
                {applicationClosing ? "Quitting…" : "Stop and quit"}
              </AlertDialogAction>
            </AlertDialogFooter>
          </AlertDialogContent>
        </AlertDialog>
      </div>
    </TooltipProvider>
  );
}
