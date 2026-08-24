import { useCallback, useEffect, useRef, useState } from "react";

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
import { DeferredSurface, type DeferredSurfaceName } from "./features/deferred/DeferredSurface";
import { ProjectDraftController } from "./features/inbox/draft-controller";
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
  listenToProjectInbox,
  loadProjectInbox,
  openRepository,
  projectErrorMessage,
  removeProject,
  saveProjectDraft,
} from "./platform/project-inbox";
import { selectRepositoryDirectory } from "./platform/repository-picker";
import { listenToWindowClose } from "./platform/window-lifecycle";

interface AppProps {
  onOpenRepository?: () => void;
  surface?: "inbox" | DeferredSurfaceName;
  visualReviewStartup?: "loading";
}

const EMPTY_INBOX: InboxSnapshot = { projects: [], drafts: [], chats: [] };

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

export function App({ onOpenRepository, surface = "inbox", visualReviewStartup }: AppProps) {
  useSystemAppearance();
  const [activeSurface, setActiveSurface] = useState<"inbox" | DeferredSurfaceName>(surface);
  const [hostStatus, setHostStatus] = useState<"checking" | "ready" | "failed">("checking");
  const [snapshot, setSnapshot] = useState<InboxSnapshot>(EMPTY_INBOX);
  const [selectedProjectId, setSelectedProjectId] = useState<number | null>(null);
  const [selectedChatId, setSelectedChatId] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [repositoryActionError, setRepositoryActionError] = useState<string>();
  const verificationGeneration = useRef(0);
  const [setups] = useState(() => new ChatSetupController());
  const [drafts] = useState(() => {
    const controller = new ProjectDraftController(
      async (projectId, prompt) => {
        await saveProjectDraft(projectId, prompt);
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
          setHostStatus("ready");
        },
        () => {
          if (verificationGeneration.current === generation) setHostStatus("failed");
        },
      );
    },
    [drafts, setups],
  );

  const retryStartup = useCallback(() => {
    const generation = ++verificationGeneration.current;
    setHostStatus("checking");
    completeStartup(generation);
  }, [completeStartup]);

  const flushAllDrafts = useCallback(() => drafts.flushAll(), [drafts]);

  const openSettings = useCallback(() => {
    void flushAllDrafts().catch(() => undefined);
    setActiveSurface("settings");
  }, [flushAllDrafts]);

  const closeDeferredSurface = useCallback(() => setActiveSurface("inbox"), []);

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

  const selectProject = useCallback(
    (projectId: number | null) => {
      void flushAllDrafts().catch(() => undefined);
      setSelectedProjectId(projectId);
      setSelectedChatId(null);
    },
    [flushAllDrafts],
  );

  const createProjectChat = useCallback(
    async (projectId: number, prompt: string) => {
      await drafts.flush(projectId);
      try {
        const created = await createChat(projectId, prompt);
        drafts.forget(projectId);
        setups.reconcile(created.snapshot);
        setSnapshot(created.snapshot);
        setSelectedProjectId(created.chat.projectId);
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

  const removeSelectedProject = useCallback(
    async (projectId: number) => {
      const draftBeforeRemoval = drafts.get(projectId);
      drafts.cancel(projectId);
      try {
        const nextSnapshot = await removeProject(projectId);
        drafts.forget(projectId);
        drafts.reconcile(nextSnapshot);
        setSnapshot(drafts.overlay(nextSnapshot));
        if (selectedProjectId === projectId) setSelectedProjectId(null);
        return undefined;
      } catch (error: unknown) {
        if (draftBeforeRemoval.status.state === "saving") {
          drafts.change(projectId, draftBeforeRemoval.prompt);
        }
        return projectErrorMessage(error, "Couldn't remove that project. Try again.");
      }
    },
    [drafts, selectedProjectId],
  );

  useEffect(() => {
    if (visualReviewStartup === "loading") return;
    const generation = ++verificationGeneration.current;
    completeStartup(generation);
    return () => {
      verificationGeneration.current += 1;
    };
  }, [completeStartup, visualReviewStartup]);

  useEffect(() => {
    let disposed = false;
    let stopListening: (() => void) | undefined;
    void listenToProjectInbox((event) => {
      if (disposed) return;
      drafts.reconcile(event.snapshot);
      setups.reconcile(event.snapshot);
      setSnapshot(drafts.overlay(event.snapshot));
      if (event.focusedProjectId !== null) setSelectedProjectId(event.focusedProjectId);
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
    void listenToWindowClose(flushAllDrafts).then((unlisten) => {
      if (disposed) unlisten();
      else stopListening = unlisten;
    });
    return () => {
      disposed = true;
      stopListening?.();
    };
  }, [flushAllDrafts]);

  const selectedProject = snapshot.projects.find(({ id }) => id === selectedProjectId);
  const titlebarContext =
    activeSurface === "settings" ? "Settings" : (selectedProject?.name ?? "All Projects");

  return (
    <TooltipProvider>
      <div className="app-shell">
        <header className="titlebar" data-tauri-drag-region>
          <div className="wordmark" aria-label="Più">
            <span className="wordmark-symbol" aria-hidden="true">
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
            actionError={repositoryActionError}
            drafts={drafts}
            onCancelSetup={cancelSetup}
            onCreateChat={createProjectChat}
            onOpenRepository={openSelectedRepository}
            onOpenTerminal={openTerminal}
            onOpenSettings={openSettings}
            onQueryChange={changeQuery}
            onRemoveProject={removeSelectedProject}
            onRetrySetup={retrySetup}
            onSelectChat={setSelectedChatId}
            onSelectProject={selectProject}
            query={query}
            selectedChatId={selectedChatId}
            selectedProjectId={selectedProjectId}
            setups={setups}
            snapshot={snapshot}
          />
        ) : (
          <main className="deferred-workspace">
            <div className="conversation-stage">
              <DeferredSurface onClose={closeDeferredSurface} surface={activeSurface} />
            </div>
          </main>
        )}
      </div>
    </TooltipProvider>
  );
}
