import { ArrowLeftIcon } from "lucide-react";
import { useEffect, useMemo, useRef, useSyncExternalStore } from "react";

import { Button } from "@/components/ui/button";
import { AgentEnvironmentController } from "@/features/agent-environment/agent-environment-controller";
import { ModelsResourcesPanel } from "@/features/agent-environment/ModelsResourcesPanel";
import {
  type AgentEnvironmentAdapter,
  tauriAgentEnvironmentAdapter,
} from "@/platform/agent-environment";
import type { ProjectSummary } from "@/platform/project-inbox";
import { ModelResourceQaSurface, modelResourceQaEnabled } from "#model-resource-qa";
import "./settings-surface.css";

interface SettingsSurfaceProps {
  agentEnvironmentAdapter?: AgentEnvironmentAdapter;
  onClose?: () => void;
  project?: ProjectSummary;
}

export default function SettingsSurface({
  agentEnvironmentAdapter = tauriAgentEnvironmentAdapter,
  onClose,
  project,
}: SettingsSurfaceProps) {
  const projectId = project?.id;
  const projectName = project?.name;
  const backButtonRef = useRef<HTMLButtonElement>(null);
  const environment = useMemo(
    () => new AgentEnvironmentController(projectId ?? null, agentEnvironmentAdapter),
    [agentEnvironmentAdapter, projectId],
  );
  const environmentSnapshot = useSyncExternalStore(
    environment.subscribe,
    environment.getSnapshot,
    environment.getSnapshot,
  );

  useEffect(() => {
    backButtonRef.current?.focus();
  }, []);

  useEffect(() => {
    void environment.load();
    return () => environment.dispose();
  }, [environment]);

  return (
    <section className="settings-surface" aria-label="Settings">
      <header className="settings-surface__toolbar">
        {onClose ? (
          <Button onClick={onClose} ref={backButtonRef} type="button" variant="ghost">
            <ArrowLeftIcon aria-hidden="true" data-icon="inline-start" />
            Back to Inbox
          </Button>
        ) : null}
      </header>
      <div className="settings-surface__scroll">
        <div className="settings-surface__content">
          <div className="settings-surface__heading">
            <h1>Models &amp; Resources</h1>
            <p>Choose which models and resources Più can use.</p>
          </div>
          {modelResourceQaEnabled ? (
            <ModelResourceQaSurface />
          ) : (
            <ModelsResourcesPanel
              error={environmentSnapshot.error}
              loading={environmentSnapshot.phase === "loading"}
              onResourceEnabledChange={environment.setResourceEnabled}
              onRetry={projectId === undefined ? undefined : environment.retry}
              pendingResourceId={environmentSnapshot.pendingResource}
              projectName={projectName}
              snapshot={environmentSnapshot.environment}
              status={environmentSnapshot.status}
            />
          )}
        </div>
      </div>
    </section>
  );
}
