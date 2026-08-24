import { useCallback, useEffect, useRef, useState } from "react";

import { DeferredSurface, type DeferredSurfaceName } from "./features/deferred/DeferredSurface";
import { InboxWorkspace, type DraftPersistenceStatus } from "./features/inbox/InboxWorkspace";
import { useSystemAppearance } from "./hooks/use-system-appearance";
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
}

interface PendingDraft {
  prompt: string;
  timer: number;
  generation: number;
}

const EMPTY_INBOX: InboxSnapshot = { projects: [], drafts: [], chats: [] };
const DRAFT_SAVE_DELAY_MS = 250;

function StartupFailure({ onRetry }: { onRetry: () => void }) {
  return (
    <section className="startup-failure" aria-labelledby="startup-failure-title">
      <p className="startup-failure__eyebrow">Application unavailable</p>
      <h2 id="startup-failure-title">Più couldn't start</h2>
      <p>Something interrupted startup. Retry to continue without changing your work.</p>
      <button className="primary-action" type="button" onClick={onRetry}>
        Retry
      </button>
    </section>
  );
}

function StartupLoading() {
  return (
    <section className="startup-loading" aria-live="polite" role="status">
      <span aria-hidden="true" className="startup-loading__indicator" />
      <div>
        <p>Preparing Più</p>
        <h2>Opening your inbox…</h2>
      </div>
    </section>
  );
}

function optimisticDraft(
  snapshot: InboxSnapshot,
  projectId: number,
  prompt: string,
): InboxSnapshot {
  const drafts = snapshot.drafts.filter((draft) => draft.projectId !== projectId);
  if (prompt) drafts.push({ projectId, prompt, updatedAtMs: Date.now() });
  return { ...snapshot, drafts };
}

export function App({ onOpenRepository, surface = "inbox" }: AppProps) {
  useSystemAppearance();
  const [hostStatus, setHostStatus] = useState<"checking" | "ready" | "failed">("checking");
  const [snapshot, setSnapshot] = useState<InboxSnapshot>(EMPTY_INBOX);
  const [selectedProjectId, setSelectedProjectId] = useState<number | null>(null);
  const [query, setQuery] = useState("");
  const [repositoryActionError, setRepositoryActionError] = useState<string>();
  const [draftStatuses, setDraftStatuses] = useState<Record<number, DraftPersistenceStatus>>({});
  const verificationGeneration = useRef(0);
  const pendingDrafts = useRef(new Map<number, PendingDraft>());
  const draftSaveQueue = useRef(new Map<number, Promise<void>>());
  const draftGenerations = useRef(new Map<number, number>());

  const completeStartup = useCallback((generation: number) => {
    void Promise.all([verifyHostBoundary(), loadProjectInbox()]).then(
      ([, loadedSnapshot]) => {
        if (verificationGeneration.current !== generation) return;
        setSnapshot(loadedSnapshot);
        setHostStatus("ready");
      },
      () => {
        if (verificationGeneration.current === generation) setHostStatus("failed");
      },
    );
  }, []);

  const retryStartup = useCallback(() => {
    const generation = ++verificationGeneration.current;
    setHostStatus("checking");
    completeStartup(generation);
  }, [completeStartup]);

  const persistDraft = useCallback((projectId: number, prompt: string, generation: number) => {
    const previous = draftSaveQueue.current.get(projectId);
    const save = previous
      ? previous.catch(() => undefined).then(() => saveProjectDraft(projectId, prompt))
      : saveProjectDraft(projectId, prompt);
    const current = save.then(
      () => {
        if (draftGenerations.current.get(projectId) !== generation) return;
        setDraftStatuses((statuses) => ({ ...statuses, [projectId]: { state: "saved" } }));
      },
      (error: unknown) => {
        if (draftGenerations.current.get(projectId) !== generation) return;
        setDraftStatuses((statuses) => ({
          ...statuses,
          [projectId]: {
            state: "failed",
            message: projectErrorMessage(
              error,
              "Couldn't save this draft. Keep Più open and try again.",
            ),
          },
        }));
        throw error;
      },
    );
    draftSaveQueue.current.set(projectId, current);
    void current.catch(() => undefined);
    return current;
  }, []);

  const flushDraft = useCallback(
    (projectId: number) => {
      const pending = pendingDrafts.current.get(projectId);
      if (!pending) return;
      window.clearTimeout(pending.timer);
      pendingDrafts.current.delete(projectId);
      return persistDraft(projectId, pending.prompt, pending.generation);
    },
    [persistDraft],
  );

  const flushAllDrafts = useCallback(async () => {
    for (const projectId of [...pendingDrafts.current.keys()]) void flushDraft(projectId);
    await Promise.all(draftSaveQueue.current.values());
  }, [flushDraft]);

  const openSelectedRepository = useCallback(() => {
    if (selectedProjectId !== null) void flushDraft(selectedProjectId);
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
        setSnapshot(opened.snapshot);
        setSelectedProjectId(opened.focusedProjectId);
        setQuery("");
      } catch (error: unknown) {
        setRepositoryActionError(
          projectErrorMessage(error, "Couldn't open that repository. Try again."),
        );
      }
    })();
  }, [flushDraft, onOpenRepository, selectedProjectId]);

  const changeDraft = useCallback(
    (projectId: number, prompt: string) => {
      setSnapshot((current) => optimisticDraft(current, projectId, prompt));
      const generation = (draftGenerations.current.get(projectId) ?? 0) + 1;
      draftGenerations.current.set(projectId, generation);
      setDraftStatuses((statuses) => ({ ...statuses, [projectId]: { state: "saving" } }));
      const previous = pendingDrafts.current.get(projectId);
      if (previous) window.clearTimeout(previous.timer);
      const timer = window.setTimeout(() => void flushDraft(projectId), DRAFT_SAVE_DELAY_MS);
      pendingDrafts.current.set(projectId, { prompt, timer, generation });
    },
    [flushDraft],
  );

  const selectProject = useCallback(
    (projectId: number | null) => {
      if (selectedProjectId !== null) void flushDraft(selectedProjectId);
      setSelectedProjectId(projectId);
    },
    [flushDraft, selectedProjectId],
  );

  const changeQuery = useCallback(
    (nextQuery: string) => {
      if (selectedProjectId !== null && !query && nextQuery) void flushDraft(selectedProjectId);
      setQuery(nextQuery);
    },
    [flushDraft, query, selectedProjectId],
  );

  const removeSelectedProject = useCallback(
    async (projectId: number) => {
      const pending = pendingDrafts.current.get(projectId);
      if (pending) window.clearTimeout(pending.timer);
      pendingDrafts.current.delete(projectId);
      try {
        const nextSnapshot = await removeProject(projectId);
        setSnapshot(nextSnapshot);
        setDraftStatuses((statuses) => {
          const next = { ...statuses };
          delete next[projectId];
          return next;
        });
        if (selectedProjectId === projectId) setSelectedProjectId(null);
        return undefined;
      } catch (error: unknown) {
        if (pending) void persistDraft(projectId, pending.prompt, pending.generation);
        return projectErrorMessage(error, "Couldn't remove that project. Try again.");
      }
    },
    [persistDraft, selectedProjectId],
  );

  useEffect(() => {
    const generation = ++verificationGeneration.current;
    completeStartup(generation);
    return () => {
      verificationGeneration.current += 1;
    };
  }, [completeStartup]);

  useEffect(() => {
    let disposed = false;
    let stopListening: (() => void) | undefined;
    void listenToProjectInbox((event) => {
      if (disposed) return;
      setSnapshot(event.snapshot);
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
  }, []);

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
  const selectedDraft = snapshot.drafts.find(({ projectId }) => projectId === selectedProjectId);
  const selectedDraftStatus: DraftPersistenceStatus =
    selectedProjectId === null
      ? { state: "idle" }
      : (draftStatuses[selectedProjectId] ??
        (selectedDraft ? { state: "saved" } : { state: "idle" }));

  return (
    <div className="app-shell">
      <header className="titlebar" data-tauri-drag-region>
        <div className="wordmark" aria-label="Più">
          <span className="wordmark__symbol" aria-hidden="true">
            π
          </span>
          <span>Più</span>
        </div>
        <div className="titlebar__context">{selectedProject?.name ?? "All Projects"}</div>
      </header>
      {hostStatus === "checking" ? (
        <main className="startup-workspace">
          <StartupLoading />
        </main>
      ) : hostStatus === "failed" ? (
        <main className="startup-workspace">
          <StartupFailure onRetry={retryStartup} />
        </main>
      ) : surface === "inbox" ? (
        <InboxWorkspace
          actionError={repositoryActionError}
          draftStatus={selectedDraftStatus}
          onDraftChange={changeDraft}
          onOpenRepository={openSelectedRepository}
          onQueryChange={changeQuery}
          onRemoveProject={removeSelectedProject}
          onSelectProject={selectProject}
          query={query}
          selectedProjectId={selectedProjectId}
          snapshot={snapshot}
        />
      ) : (
        <main className="workspace workspace--deferred">
          <div className="conversation-stage">
            <DeferredSurface surface={surface} />
          </div>
        </main>
      )}
    </div>
  );
}
