import { memo, useId } from "react";

import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { Switch } from "@/components/ui/switch";
import type { AgentEnvironmentDiagnostic } from "@/generated/AgentEnvironmentDiagnostic";
import type { AgentEnvironmentDiagnosticResource } from "@/generated/AgentEnvironmentDiagnosticResource";
import type { AgentEnvironmentSnapshot } from "@/generated/AgentEnvironmentSnapshot";
import type { AgentPackageSummary } from "@/generated/AgentPackageSummary";
import type { AgentResourceId } from "@/generated/AgentResourceId";
import type { AgentResourcePreferenceScope } from "@/generated/AgentResourcePreferenceScope";
import type { AgentResourceSource } from "@/generated/AgentResourceSource";
import type { AgentResourceSummary } from "@/generated/AgentResourceSummary";

import { ModelResourcePanel } from "../model-resources/ModelResourcePanel";
import "./models-resources-panel.css";

interface ModelsResourcesPanelProps {
  error?: string | null;
  loading?: boolean;
  onResourceEnabledChange: (
    resource: AgentResourceId,
    enabled: boolean,
    scope: AgentResourcePreferenceScope,
  ) => Promise<void> | void;
  onRetry: () => Promise<void> | void;
  pendingResourceId?: AgentResourceId | null;
  snapshot: AgentEnvironmentSnapshot | null;
  status?: string | null;
}

interface ResourceRowProps {
  enabled: boolean;
  name: string;
  onResourceEnabledChange: ModelsResourcesPanelProps["onResourceEnabledChange"];
  pending: boolean;
  resource: AgentResourceId;
  scope: AgentResourcePreferenceScope;
  source: AgentResourceSource;
  states?: readonly string[];
}

const SOURCE_LABELS: Record<AgentResourceSource, string> = {
  piu: "Più",
  project: "Project",
};

function resourceKey(resource: AgentResourceId): string {
  switch (resource.kind) {
    case "modelRoute":
      return `model:${resource.route.provider}:${resource.route.modelId}`;
    case "skill":
    case "extension":
    case "package":
      return `${resource.kind}:${resource.id}`;
  }
}

function scopeForSource(source: AgentResourceSource): AgentResourcePreferenceScope {
  return source === "project" ? "project" : "global";
}

const ResourceRow = memo(function ResourceRow({
  enabled,
  name,
  onResourceEnabledChange,
  pending,
  resource,
  scope,
  source,
  states = [],
}: ResourceRowProps) {
  return (
    <li className="models-resources-row">
      <div className="models-resources-row__identity">
        <span className="models-resources-row__name" title={name}>
          {name}
        </span>
        <span className="models-resources-row__metadata">
          <span>{SOURCE_LABELS[source]}</span>
          {states.map((state) => (
            <span className="models-resources-row__state" key={state}>
              {state}
            </span>
          ))}
        </span>
      </div>
      <Switch
        aria-busy={pending || undefined}
        aria-label={name}
        checked={enabled}
        disabled={pending}
        onCheckedChange={(checked) => void onResourceEnabledChange(resource, checked, scope)}
      />
    </li>
  );
});

function ExceptionalDiagnostics({
  diagnostics,
  resource,
}: {
  diagnostics: readonly AgentEnvironmentDiagnostic[];
  resource: AgentEnvironmentDiagnosticResource;
}) {
  const visible = diagnostics.filter(
    (diagnostic) => diagnostic.resource === resource && diagnostic.kind !== "info",
  );
  if (visible.length === 0) return null;
  return (
    <div className="models-resources-diagnostics" aria-label={`${resource} diagnostics`}>
      {visible.map((diagnostic, index) => (
        <p data-kind={diagnostic.kind} key={`${diagnostic.kind}:${diagnostic.message}:${index}`}>
          {diagnostic.message}
        </p>
      ))}
    </div>
  );
}

function ResourceGroup({
  children,
  diagnostics,
  diagnosticResource,
  emptyMessage,
  heading,
}: {
  children: React.ReactNode;
  diagnosticResource: AgentEnvironmentDiagnosticResource;
  diagnostics: readonly AgentEnvironmentDiagnostic[];
  emptyMessage?: string;
  heading: string;
}) {
  const headingId = useId();
  return (
    <section className="models-resources-group" aria-labelledby={headingId}>
      <header className="models-resources-group__header">
        <h2 id={headingId}>{heading}</h2>
      </header>
      {emptyMessage ? <p className="models-resources-empty-row">{emptyMessage}</p> : children}
      <ExceptionalDiagnostics diagnostics={diagnostics} resource={diagnosticResource} />
    </section>
  );
}

function resourceRows(
  resources: readonly AgentResourceSummary[],
  kind: "skill" | "extension",
  pendingResourceId: AgentResourceId | null | undefined,
  onResourceEnabledChange: ModelsResourcesPanelProps["onResourceEnabledChange"],
) {
  const pendingKey = pendingResourceId ? resourceKey(pendingResourceId) : null;
  return resources.map((summary) => {
    const resource = { kind, id: summary.id } satisfies AgentResourceId;
    return (
      <ResourceRow
        enabled={summary.enabled}
        key={resourceKey(resource)}
        name={summary.name}
        onResourceEnabledChange={onResourceEnabledChange}
        pending={pendingKey === resourceKey(resource)}
        resource={resource}
        scope={scopeForSource(summary.source)}
        source={summary.source}
      />
    );
  });
}

function packageStates(summary: AgentPackageSummary): string[] {
  const states: string[] = [];
  if (!summary.installed) states.push("Couldn’t load");
  if (summary.filtered) states.push("Project version in use");
  return states;
}

function LoadingInventory() {
  return (
    <div
      aria-label="Loading models and resources"
      className="models-resources-loading"
      role="status"
    >
      {["Model routes", "Skills", "Extensions", "Packages"].map((heading) => (
        <section className="models-resources-group" key={heading}>
          <header className="models-resources-group__header">
            <h2>{heading}</h2>
          </header>
          <div className="models-resources-loading__row">
            <div>
              <Skeleton className="h-3 w-36" />
              <Skeleton className="mt-2 h-2.5 w-16" />
            </div>
            <Skeleton className="h-5 w-9 rounded-full" />
          </div>
        </section>
      ))}
      <span className="sr-only">Inspecting the Pi environment</span>
    </div>
  );
}

export const ModelsResourcesPanel = memo(function ModelsResourcesPanel({
  error,
  loading = false,
  onResourceEnabledChange,
  onRetry,
  pendingResourceId,
  snapshot,
  status,
}: ModelsResourcesPanelProps) {
  const pendingKey = pendingResourceId ? resourceKey(pendingResourceId) : null;
  const generalDiagnostics = snapshot?.diagnostics.filter(
    (diagnostic) =>
      diagnostic.kind !== "info" &&
      (diagnostic.resource === "runtime" || diagnostic.resource === "settings"),
  );

  return (
    <div className="models-resources-panel">
      {status ? (
        <p className="models-resources-notice" role="status">
          {status}
        </p>
      ) : null}
      {error ? (
        <div className="models-resources-error" role="alert">
          <p>{error}</p>
          <Button onClick={() => void onRetry()} type="button" variant="outline">
            Retry
          </Button>
        </div>
      ) : null}
      {generalDiagnostics?.length ? (
        <div className="models-resources-diagnostics models-resources-diagnostics--general">
          {generalDiagnostics.map((diagnostic, index) => (
            <p
              data-kind={diagnostic.kind}
              key={`${diagnostic.kind}:${diagnostic.message}:${index}`}
            >
              {diagnostic.message}
            </p>
          ))}
        </div>
      ) : null}

      {loading ? (
        <LoadingInventory />
      ) : snapshot ? (
        <div className="models-resources-inventory">
          <ResourceGroup
            diagnosticResource="model"
            diagnostics={snapshot.diagnostics}
            emptyMessage={snapshot.modelRoutes.length === 0 ? "No model routes found" : undefined}
            heading="Model routes"
          >
            <ul className="models-resources-list">
              {snapshot.modelRoutes.map((route) => {
                const resource = { kind: "modelRoute", route: route.id } satisfies AgentResourceId;
                return (
                  <ResourceRow
                    enabled={route.enabled}
                    key={resourceKey(resource)}
                    name={route.name}
                    onResourceEnabledChange={onResourceEnabledChange}
                    pending={pendingKey === resourceKey(resource)}
                    resource={resource}
                    scope="global"
                    source="piu"
                  />
                );
              })}
            </ul>
          </ResourceGroup>

          <ResourceGroup
            diagnosticResource="skill"
            diagnostics={snapshot.diagnostics}
            emptyMessage={snapshot.resources.skills.length === 0 ? "No skills found" : undefined}
            heading="Skills"
          >
            <ul className="models-resources-list">
              {resourceRows(
                snapshot.resources.skills,
                "skill",
                pendingResourceId,
                onResourceEnabledChange,
              )}
            </ul>
          </ResourceGroup>

          <ResourceGroup
            diagnosticResource="extension"
            diagnostics={snapshot.diagnostics}
            emptyMessage={
              snapshot.resources.extensions.length === 0 ? "No extensions found" : undefined
            }
            heading="Extensions"
          >
            <ul className="models-resources-list">
              {resourceRows(
                snapshot.resources.extensions,
                "extension",
                pendingResourceId,
                onResourceEnabledChange,
              )}
            </ul>
          </ResourceGroup>

          <ResourceGroup
            diagnosticResource="package"
            diagnostics={snapshot.diagnostics}
            emptyMessage={
              snapshot.resources.packages.length === 0 ? "No packages found" : undefined
            }
            heading="Packages"
          >
            <ul className="models-resources-list">
              {snapshot.resources.packages.map((summary) => {
                const resource = { kind: "package", id: summary.id } satisfies AgentResourceId;
                return (
                  <ResourceRow
                    enabled={summary.enabled}
                    key={resourceKey(resource)}
                    name={summary.name}
                    onResourceEnabledChange={onResourceEnabledChange}
                    pending={pendingKey === resourceKey(resource)}
                    resource={resource}
                    scope={scopeForSource(summary.source)}
                    source={summary.source}
                    states={packageStates(summary)}
                  />
                );
              })}
            </ul>
          </ResourceGroup>
        </div>
      ) : null}

      <section className="models-resources-group models-resources-managed-model">
        <header className="models-resources-group__header">
          <h2>Managed local model</h2>
          <span className="models-resources-required">Required</span>
        </header>
        <ModelResourcePanel context="settings" />
      </section>
    </div>
  );
});

export type { ModelsResourcesPanelProps };
