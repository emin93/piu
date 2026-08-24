import { useCallback, useEffect, useId, useRef, useState } from "react";

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
import {
  Empty,
  EmptyContent,
  EmptyDescription,
  EmptyHeader,
  EmptyTitle,
} from "@/components/ui/empty";
import { Input } from "@/components/ui/input";
import { Skeleton } from "@/components/ui/skeleton";
import type { ModelAssetStatus } from "@/generated/ModelAssetStatus";
import {
  authorizeHuggingFace,
  cancelModelDownload,
  getModelAssetStatus,
  removeModelAssets,
  startModelDownload,
  subscribeToModelAssetStatus,
} from "@/platform/model-assets";

const phaseLabels: Record<ModelAssetStatus["phase"], string> = {
  initializing: "Checking",
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
export const REVISION_MISMATCH_MESSAGE =
  "An older Più model revision is installed. Remove it here, then download the pinned revision.";
const GIGABYTE_FORMATTER = new Intl.NumberFormat(undefined, { maximumFractionDigits: 1 });

function formatBytes(bytes: number): string {
  if (bytes === 0) return "0 GB";
  return `${GIGABYTE_FORMATTER.format(bytes / 1_000_000_000)} GB`;
}

function progress(status: ModelAssetStatus): number {
  return status.totalBytes === 0
    ? 0
    : Math.min(100, Math.round((status.transferredBytes / status.totalBytes) * 100));
}

function statusVariant(phase: ModelAssetStatus["phase"]): "destructive" | "outline" | "secondary" {
  if (phase === "failed" || phase === "revisionMismatch" || phase === "authenticationRequired") {
    return "destructive";
  }
  return phase === "ready" ? "secondary" : "outline";
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
      <section aria-label="Models & resources" className="model-resource-panel">
        <Empty className="model-resource-empty">
          <EmptyHeader>
            <EmptyTitle>Model resources unavailable</EmptyTitle>
            <EmptyDescription>
              Più couldn&apos;t reach the local model resource service.
            </EmptyDescription>
          </EmptyHeader>
          <EmptyContent>
            <p className="resource-error" role="alert">
              {error ?? "Try again to reconnect."}
            </p>
            <Button
              onClick={() => {
                setInitialization("loading");
                setStatus(undefined);
                setError(undefined);
                setInitializationAttempt((attempt) => attempt + 1);
              }}
              type="button"
              variant="outline"
            >
              Retry
            </Button>
          </EmptyContent>
        </Empty>
      </section>
    );
  }

  const currentStatus = statusOverride ?? status;
  if (initialization === "loading" || !currentStatus) {
    return (
      <section
        aria-busy="true"
        aria-label="Models & resources"
        className="model-resource-panel model-resource-panel--loading"
        role="status"
      >
        <div className="model-resource-loading-copy">
          <Skeleton className="h-3 w-28" />
          <Skeleton className="h-5 w-44" />
        </div>
        <Skeleton className="h-7 w-24" />
        <span className="sr-only">Checking model resources…</span>
      </section>
    );
  }

  const downloading = currentStatus.phase === "downloading" || currentStatus.phase === "verifying";
  const removing = currentStatus.phase === "removing";
  const initializing = currentStatus.phase === "initializing";
  const authenticationNeeded = currentStatus.phase === "authenticationRequired";
  const mismatch = currentStatus.phase === "revisionMismatch";
  const ownershipBlocked = currentStatus.errorCode === "ownership";
  const removable = currentStatus.phase === "ready" || mismatch;
  const transferRelevant =
    downloading || currentStatus.canResume || currentStatus.phase === "cancelled";
  const qaMode = statusOverride !== undefined;
  const progressValue = progress(currentStatus);
  const statusMessage = mismatch ? REVISION_MISMATCH_MESSAGE : currentStatus.message;

  return (
    <section aria-labelledby={headingId} className="model-resource-panel">
      <header className="model-resource-header">
        <div>
          <p className="settings-eyebrow">
            {context === "onboarding" ? "Required resource" : "Models & resources"}
          </p>
          <h2 id={headingId}>Local model</h2>
        </div>
        <Badge
          aria-atomic="true"
          aria-live="polite"
          className={`resource-status resource-status--${currentStatus.phase}`}
          role="status"
          variant={statusVariant(currentStatus.phase)}
        >
          {phaseLabels[currentStatus.phase]}
        </Badge>
      </header>

      <div className="model-resource-identity">
        <strong>Qwen 3.8 27B · 4-bit</strong>
        <span>MTP drafter</span>
      </div>
      <p className="model-resource-copy">
        Più installs and verifies this exact model in managed application storage. It loads only
        when a local conversation needs it.
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
            <span className="font-mono">{progressValue}%</span>
          </div>
          <progress aria-label="Model download progress" max="100" value={progressValue} />
          <div className="resource-progress__detail">
            <span className="font-mono" title={currentStatus.currentFile ?? undefined}>
              {currentStatus.currentFile ?? "Ready to resume"}
            </span>
            <span>
              {formatBytes(currentStatus.transferredBytes)} of{" "}
              {formatBytes(currentStatus.totalBytes)}
              {currentStatus.remainingBytes > 0
                ? ` · ${formatBytes(currentStatus.remainingBytes)} remaining`
                : ""}
            </span>
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
          <div className="resource-auth__controls">
            <Input
              autoComplete="off"
              disabled={qaMode}
              id={tokenId}
              onChange={(event) => setToken(event.currentTarget.value)}
              type="password"
              value={token}
            />
            <Button disabled={busy || !token.trim() || qaMode} type="submit" variant="outline">
              Connect Hugging Face
            </Button>
          </div>
          <p>The token is validated, then stored only in macOS Keychain.</p>
        </form>
      ) : null}

      {statusMessage ? (
        <p
          className={currentStatus.errorCode ? "resource-error" : "resource-message"}
          role={currentStatus.errorCode ? "alert" : undefined}
        >
          {statusMessage}
        </p>
      ) : null}
      {error ? (
        <p className="resource-error" role="alert">
          {error}
        </p>
      ) : null}

      <div className="resource-actions">
        {(downloading || removing) && currentStatus.canCancel ? (
          <Button
            disabled={(busy && !removing) || qaMode}
            onClick={() => {
              if (removing) {
                void cancelModelDownload()
                  .then(async (accepted) => {
                    if (!accepted) setStatus(await getModelAssetStatus());
                  })
                  .catch((cause: unknown) => {
                    setError(cause instanceof Error ? cause.message : String(cause));
                  });
              } else {
                void perform(cancelModelDownload);
              }
            }}
            type="button"
            variant="outline"
          >
            {removing ? "Cancel removal" : "Cancel download"}
          </Button>
        ) : removing || initializing || ownershipBlocked ? null : removable ? (
          <Button
            disabled={busy}
            onClick={() => setConfirmRemoval(true)}
            ref={removeButtonRef}
            type="button"
            variant="destructive"
          >
            {mismatch ? "Remove old model" : "Remove model"}
          </Button>
        ) : (
          <Button
            disabled={busy || authenticationNeeded || qaMode}
            onClick={() => void perform(startModelDownload)}
            type="button"
          >
            {currentStatus.canResume ? "Resume download" : "Download model"}
          </Button>
        )}
      </div>

      <AlertDialog onOpenChange={setConfirmRemoval} open={confirmRemoval}>
        <AlertDialogContent finalFocus={removeButtonRef} initialFocus={cancelRemovalRef}>
          <AlertDialogHeader>
            <AlertDialogTitle>
              {mismatch ? "Remove old model?" : "Remove local model?"}
            </AlertDialogTitle>
            <AlertDialogDescription>
              Più verifies every file in its ownership record before removal. Unknown or changed
              files stay untouched.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel ref={cancelRemovalRef}>Keep model</AlertDialogCancel>
            <AlertDialogAction
              disabled={busy || qaMode}
              onClick={() => {
                setConfirmRemoval(false);
                void perform(async () => {
                  const next = await removeModelAssets();
                  setStatus(next);
                });
              }}
              variant="destructive"
            >
              Confirm removal
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </section>
  );
}
