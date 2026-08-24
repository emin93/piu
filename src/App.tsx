import { useCallback, useEffect, useRef, useState } from "react";

import { useSystemAppearance } from "./hooks/use-system-appearance";
import { DeferredSurface, type DeferredSurfaceName } from "./features/deferred/DeferredSurface";
import { EmptyInbox } from "./features/inbox/EmptyInbox";
import { verifyHostBoundary } from "./platform/host-boundary";
import { selectRepositoryDirectory } from "./platform/repository-picker";

interface AppProps {
  onOpenRepository?: () => void;
  surface?: "inbox" | DeferredSurfaceName;
}

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

export function App({ onOpenRepository, surface = "inbox" }: AppProps) {
  useSystemAppearance();
  const [hostStatus, setHostStatus] = useState<"checking" | "ready" | "failed">("checking");
  const [repositoryActionError, setRepositoryActionError] = useState<string>();
  const verificationGeneration = useRef(0);

  const completeHostVerification = useCallback((generation: number) => {
    void verifyHostBoundary().then(
      () => {
        if (verificationGeneration.current === generation) setHostStatus("ready");
      },
      () => {
        if (verificationGeneration.current === generation) setHostStatus("failed");
      },
    );
  }, []);

  const retryHostVerification = useCallback(() => {
    const generation = ++verificationGeneration.current;
    setHostStatus("checking");
    completeHostVerification(generation);
  }, [completeHostVerification]);

  const openRepository = useCallback(() => {
    setRepositoryActionError(undefined);
    if (onOpenRepository) {
      onOpenRepository();
      return;
    }
    void selectRepositoryDirectory().catch(() => {
      setRepositoryActionError("Couldn't open the repository picker. Try again.");
    });
  }, [onOpenRepository]);

  useEffect(() => {
    const generation = ++verificationGeneration.current;
    completeHostVerification(generation);
    return () => {
      verificationGeneration.current += 1;
    };
  }, [completeHostVerification]);

  return (
    <div className="app-shell">
      <header className="titlebar" data-tauri-drag-region>
        <div className="wordmark" aria-label="Più">
          <span className="wordmark__symbol" aria-hidden="true">
            π
          </span>
          <span>Più</span>
        </div>
        <div className="titlebar__context">All projects</div>
      </header>
      <main className="workspace" aria-label="Più inbox">
        <aside className="inbox-rail" aria-label="Chat inbox">
          <div>
            <p className="inbox-rail__label">Workspace</p>
            <div className="inbox-rail__heading">
              <h1>Inbox</h1>
              <span aria-label="0 chats">0</span>
            </div>
          </div>
          <p className="inbox-rail__hint">Unmerged chats from every open project appear here.</p>
        </aside>
        <div className="conversation-stage">
          {hostStatus === "failed" ? (
            <StartupFailure onRetry={retryHostVerification} />
          ) : surface === "inbox" ? (
            <EmptyInbox actionError={repositoryActionError} onOpenRepository={openRepository} />
          ) : (
            <DeferredSurface surface={surface} />
          )}
        </div>
      </main>
    </div>
  );
}
