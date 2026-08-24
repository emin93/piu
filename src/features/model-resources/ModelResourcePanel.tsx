import { useCallback, useEffect, useId, useRef, useState } from "react";

import type { ModelAssetStatus } from "../../generated/ModelAssetStatus";
import {
  authorizeHuggingFace,
  cancelModelDownload,
  getModelAssetStatus,
  removeModelAssets,
  startModelDownload,
  subscribeToModelAssetStatus,
} from "../../platform/model-assets";

const phaseLabels: Record<ModelAssetStatus["phase"], string> = {
  missing: "Not downloaded",
  downloading: "Downloading",
  verifying: "Verifying",
  removing: "Removing",
  ready: "Ready",
  cancelled: "Paused",
  authenticationRequired: "Access required",
  failed: "Needs attention",
  revisionMismatch: "Revision mismatch",
};

function formatBytes(bytes: number): string {
  if (bytes === 0) return "0 GB";
  return `${new Intl.NumberFormat(undefined, { maximumFractionDigits: 1 }).format(bytes / 1_000_000_000)} GB`;
}

function progress(status: ModelAssetStatus): number {
  return status.totalBytes === 0
    ? 0
    : Math.min(100, Math.round((status.transferredBytes / status.totalBytes) * 100));
}

interface ModelResourcePanelProps {
  context: "onboarding" | "settings";
  /** Deterministic build-time QA input; production Settings never supplies it. */
  statusOverride?: ModelAssetStatus;
  dialogOpenForQa?: boolean;
}

export function ModelResourcePanel({
  context,
  statusOverride,
  dialogOpenForQa = false,
}: ModelResourcePanelProps) {
  const headingId = useId();
  const tokenId = useId();
  const dialogTitleId = useId();
  const [status, setStatus] = useState<ModelAssetStatus | undefined>(statusOverride);
  const [initialization, setInitialization] = useState<"loading" | "ready" | "failed">(
    statusOverride ? "ready" : "loading",
  );
  const [initializationAttempt, setInitializationAttempt] = useState(0);
  const [token, setToken] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const [confirmRemoval, setConfirmRemoval] = useState(dialogOpenForQa);
  const removeButtonRef = useRef<HTMLButtonElement>(null);
  const cancelRemovalRef = useRef<HTMLButtonElement>(null);
  const confirmRemovalRef = useRef<HTMLButtonElement>(null);
  const dialogWasOpen = useRef(dialogOpenForQa);

  useEffect(() => {
    if (statusOverride) return;
    let active = true;
    let stopListening: (() => void) | undefined;
    void subscribeToModelAssetStatus((next) => {
      if (active) setStatus(next);
    })
      .then((unlisten) => {
        if (!active) {
          unlisten();
          return undefined;
        }
        stopListening = unlisten;
        return getModelAssetStatus();
      })
      .then((current) => {
        if (active && current) {
          setStatus(current);
          setInitialization("ready");
        }
      })
      .catch((cause: unknown) => {
        if (active) {
          setInitialization("failed");
          setError(cause instanceof Error ? cause.message : String(cause));
        }
      });
    return () => {
      active = false;
      stopListening?.();
    };
  }, [initializationAttempt, statusOverride]);

  useEffect(() => {
    if (confirmRemoval) {
      dialogWasOpen.current = true;
      cancelRemovalRef.current?.focus();
    } else if (dialogWasOpen.current) {
      dialogWasOpen.current = false;
      removeButtonRef.current?.focus();
    }
  }, [confirmRemoval]);

  const perform = useCallback(async (action: () => Promise<unknown>) => {
    setBusy(true);
    setError(undefined);
    try {
      await action();
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setBusy(false);
    }
  }, []);

  if (initialization === "failed") {
    return (
      <section className="model-resource-card model-resource-card--failure">
        <p className="settings-eyebrow">Models & resources</p>
        <h2>Model resources unavailable</h2>
        <p className="resource-error" role="alert">
          {error ?? "Più couldn’t reach its model resource service."}
        </p>
        <button
          className="secondary-action"
          type="button"
          onClick={() => {
            setInitialization("loading");
            setStatus(undefined);
            setError(undefined);
            setInitializationAttempt((attempt) => attempt + 1);
          }}
        >
          Retry
        </button>
      </section>
    );
  }

  const currentStatus = statusOverride ?? status;
  if (initialization === "loading" || !currentStatus) {
    return (
      <section className="model-resource-card" aria-busy="true">
        <p className="model-resource-card__loading">Checking model resources…</p>
      </section>
    );
  }

  const downloading = currentStatus.phase === "downloading" || currentStatus.phase === "verifying";
  const removing = currentStatus.phase === "removing";
  const authenticationNeeded = currentStatus.phase === "authenticationRequired";
  const mismatch = currentStatus.phase === "revisionMismatch";
  const removable = currentStatus.phase === "ready" || mismatch;
  const transferRelevant =
    downloading || currentStatus.canResume || currentStatus.phase === "cancelled";
  const qaMode = statusOverride !== undefined;

  const closeRemoval = () => setConfirmRemoval(false);
  const keepFocusInDialog = (event: React.KeyboardEvent<HTMLDivElement>) => {
    if (event.key === "Escape") {
      event.preventDefault();
      closeRemoval();
      return;
    }
    if (event.key !== "Tab") return;
    if (event.shiftKey && document.activeElement === cancelRemovalRef.current) {
      event.preventDefault();
      confirmRemovalRef.current?.focus();
    } else if (!event.shiftKey && document.activeElement === confirmRemovalRef.current) {
      event.preventDefault();
      cancelRemovalRef.current?.focus();
    }
  };

  return (
    <section className="model-resource-card" aria-labelledby={headingId}>
      <header className="model-resource-card__header">
        <div>
          <p className="settings-eyebrow">
            {context === "onboarding" ? "Required resource" : "Models & resources"}
          </p>
          <h2 id={headingId}>Local model</h2>
        </div>
        <span
          className={`resource-status resource-status--${currentStatus.phase}`}
          role="status"
          aria-live="polite"
          aria-atomic="true"
        >
          {phaseLabels[currentStatus.phase]}
        </span>
      </header>

      <div className="model-resource-card__identity">
        <strong>Qwen 3.8 27B · 4-bit</strong>
        <span>MTP drafter</span>
      </div>
      <p className="model-resource-card__copy">
        Più installs this exact pinned model and drafter in managed application storage. Local
        inference stays off until it is needed.
      </p>

      <dl className="resource-metrics">
        <div>
          <dt>Download size</dt>
          <dd>{formatBytes(currentStatus.totalBytes)}</dd>
        </div>
        <div>
          <dt>Space needed</dt>
          <dd>{formatBytes(currentStatus.requiredFreeBytes)}</dd>
        </div>
        <div>
          <dt>Free now</dt>
          <dd>{formatBytes(currentStatus.currentFreeBytes)}</dd>
        </div>
      </dl>

      {transferRelevant ? (
        <div className="resource-progress">
          <div className="resource-progress__labels">
            <span>
              {currentStatus.phase === "verifying"
                ? "Checking downloaded files"
                : currentStatus.currentAsset === "drafter"
                  ? "MTP drafter"
                  : "Model"}
            </span>
            <span>
              {formatBytes(currentStatus.transferredBytes)} of{" "}
              {formatBytes(currentStatus.totalBytes)} · {progress(currentStatus)}%
            </span>
          </div>
          <progress
            max="100"
            value={progress(currentStatus)}
            aria-label="Model download progress"
          />
          <div className="resource-progress__detail">
            <span>{currentStatus.currentFile ?? "Ready to resume"}</span>
            <span>{formatBytes(currentStatus.remainingBytes)} remaining</span>
          </div>
        </div>
      ) : null}

      {authenticationNeeded ? (
        <form
          className="resource-auth"
          onSubmit={(event) => {
            event.preventDefault();
            void perform(async () => {
              await authorizeHuggingFace(token);
              setToken("");
            });
          }}
        >
          <label htmlFor={tokenId}>Hugging Face access token</label>
          <div>
            <input
              id={tokenId}
              type="password"
              autoComplete="off"
              value={token}
              disabled={qaMode}
              onChange={(event) => setToken(event.currentTarget.value)}
            />
            <button
              className="secondary-action"
              type="submit"
              disabled={busy || !token.trim() || qaMode}
            >
              Connect Hugging Face
            </button>
          </div>
          <p>The token is validated, then stored only in macOS Keychain.</p>
        </form>
      ) : null}

      {currentStatus.message ? (
        <p
          className={currentStatus.errorCode ? "resource-error" : "resource-message"}
          role={currentStatus.errorCode ? "alert" : undefined}
        >
          {currentStatus.message}
        </p>
      ) : null}
      {error ? (
        <p className="resource-error" role="alert">
          {error}
        </p>
      ) : null}

      <div className="resource-actions">
        {downloading || removing ? (
          <button
            className="secondary-action"
            type="button"
            disabled={(busy && !removing) || qaMode}
            onClick={() => {
              if (removing) {
                void cancelModelDownload().catch((cause: unknown) => {
                  setError(cause instanceof Error ? cause.message : String(cause));
                });
              } else {
                void perform(cancelModelDownload);
              }
            }}
          >
            {removing ? "Cancel removal" : "Cancel download"}
          </button>
        ) : removable ? (
          <button
            ref={removeButtonRef}
            className="danger-action"
            type="button"
            disabled={busy}
            onClick={() => setConfirmRemoval(true)}
          >
            {mismatch ? "Remove old model" : "Remove model"}
          </button>
        ) : (
          <button
            className="primary-action"
            type="button"
            disabled={busy || authenticationNeeded || qaMode}
            onClick={() => void perform(startModelDownload)}
          >
            {currentStatus.canResume ? "Resume download" : "Download model"}
          </button>
        )}
      </div>

      {confirmRemoval ? (
        <div className="dialog-backdrop">
          <div
            className="removal-confirmation"
            role="dialog"
            aria-modal="true"
            aria-labelledby={dialogTitleId}
            onKeyDown={keepFocusInDialog}
          >
            <h3 id={dialogTitleId}>{mismatch ? "Remove old model?" : "Remove local model?"}</h3>
            <p>
              Più verifies every file recorded by its ownership marker before removal. Unknown or
              changed files stay untouched.
            </p>
            <div>
              <button
                ref={cancelRemovalRef}
                className="secondary-action"
                type="button"
                onClick={closeRemoval}
              >
                Keep model
              </button>
              <button
                ref={confirmRemovalRef}
                className="danger-action"
                type="button"
                disabled={busy || qaMode}
                onClick={() => {
                  closeRemoval();
                  void perform(async () => {
                    const next = await removeModelAssets();
                    setStatus(next);
                  });
                }}
              >
                Confirm removal
              </button>
            </div>
          </div>
        </div>
      ) : null}
    </section>
  );
}
