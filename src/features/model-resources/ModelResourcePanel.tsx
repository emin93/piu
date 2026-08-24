import { useCallback, useEffect, useId, useState } from "react";

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

export function ModelResourcePanel({ context }: { context: "onboarding" | "settings" }) {
  const tokenId = useId();
  const [status, setStatus] = useState<ModelAssetStatus>();
  const [token, setToken] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const [confirmRemoval, setConfirmRemoval] = useState(false);

  useEffect(() => {
    let active = true;
    let stopListening: (() => void) | undefined;
    void subscribeToModelAssetStatus((next) => {
      if (active) setStatus(next);
    })
      .then((unlisten) => {
        if (!active) unlisten();
        else stopListening = unlisten;
        return getModelAssetStatus();
      })
      .then((current) => {
        if (active) setStatus(current);
      })
      .catch((cause: unknown) => {
        if (active) setError(cause instanceof Error ? cause.message : String(cause));
      });
    return () => {
      active = false;
      stopListening?.();
    };
  }, []);

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

  if (!status) {
    return (
      <section className="model-resource-card" aria-busy="true">
        <p className="model-resource-card__loading">Checking model resources…</p>
        {error ? <p role="alert">{error}</p> : null}
      </section>
    );
  }

  const downloading = status.phase === "downloading" || status.phase === "verifying";
  const authenticationNeeded = status.phase === "authenticationRequired";

  return (
    <section className="model-resource-card" aria-labelledby="local-model-heading">
      <header className="model-resource-card__header">
        <div>
          <p className="settings-eyebrow">
            {context === "onboarding" ? "Required resource" : "Models & resources"}
          </p>
          <h2 id="local-model-heading">Local model</h2>
        </div>
        <span className={`resource-status resource-status--${status.phase}`}>
          {phaseLabels[status.phase]}
        </span>
      </header>

      <div className="model-resource-card__identity">
        <strong>Qwen 3.8 27B · 4-bit</strong>
        <span>MTP drafter · block 4</span>
      </div>
      <p className="model-resource-card__copy">
        Più installs this exact pinned model and drafter in its managed application storage. It
        won’t start local inference as part of this download.
      </p>

      <dl className="resource-metrics">
        <div>
          <dt>Download size</dt>
          <dd>{formatBytes(status.totalBytes)}</dd>
        </div>
        <div>
          <dt>Space needed</dt>
          <dd>{formatBytes(status.requiredFreeBytes)}</dd>
        </div>
        <div>
          <dt>Free now</dt>
          <dd>{formatBytes(status.currentFreeBytes)}</dd>
        </div>
        <div>
          <dt>Transferred</dt>
          <dd>{formatBytes(status.transferredBytes)}</dd>
        </div>
        <div>
          <dt>Remaining</dt>
          <dd>{formatBytes(status.remainingBytes)}</dd>
        </div>
      </dl>

      {downloading || status.canResume ? (
        <div className="resource-progress">
          <div className="resource-progress__labels">
            <span>{status.currentAsset === "drafter" ? "MTP drafter" : "Model"}</span>
            <span>{progress(status)}%</span>
          </div>
          <progress max="100" value={progress(status)} aria-label="Model download progress" />
          {status.currentFile ? <p>{status.currentFile}</p> : null}
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
              onChange={(event) => setToken(event.currentTarget.value)}
            />
            <button className="secondary-action" type="submit" disabled={busy || !token.trim()}>
              Connect Hugging Face
            </button>
          </div>
          <p>The token is validated graphically, then stored only in macOS Keychain.</p>
        </form>
      ) : null}

      {status.message ? <p className="resource-message">{status.message}</p> : null}
      {error ? (
        <p className="resource-error" role="alert">
          {error}
        </p>
      ) : null}

      <div className="resource-actions">
        {downloading ? (
          <button
            className="secondary-action"
            type="button"
            disabled={busy}
            onClick={() => void perform(cancelModelDownload)}
          >
            Cancel download
          </button>
        ) : status.phase === "ready" ? (
          <button
            className="danger-action"
            type="button"
            disabled={busy}
            onClick={() => setConfirmRemoval(true)}
          >
            Remove model
          </button>
        ) : status.phase !== "revisionMismatch" ? (
          <button
            className="primary-action"
            type="button"
            disabled={busy || authenticationNeeded}
            onClick={() => void perform(startModelDownload)}
          >
            {status.canResume ? "Resume download" : "Download model"}
          </button>
        ) : null}
      </div>

      {confirmRemoval ? (
        <div className="removal-confirmation" role="group" aria-label="Confirm model removal">
          <p>Più will remove only files verified as owned by Più. Other files stay untouched.</p>
          <div>
            <button
              className="danger-action"
              type="button"
              disabled={busy}
              onClick={() =>
                void perform(async () => {
                  const next = await removeModelAssets();
                  setStatus(next);
                  setConfirmRemoval(false);
                })
              }
            >
              Confirm removal
            </button>
            <button
              className="secondary-action"
              type="button"
              onClick={() => setConfirmRemoval(false)}
            >
              Keep model
            </button>
          </div>
        </div>
      ) : null}
    </section>
  );
}
