import { ChevronDownIcon, LoaderCircleIcon } from "lucide-react";
import { memo } from "react";

import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import type { ModelRouteId } from "@/generated/ModelRouteId";
import type { ModelRouteSummary } from "@/generated/ModelRouteSummary";
import type { ReasoningEffort } from "@/generated/ReasoningEffort";

import { type ModelControlsStoreSnapshot, reasoningEffortLabel } from "./model-controls-controller";
import "./model-controls.css";

interface ComposerInferenceControlsProps {
  disabled?: boolean;
  onSelectEffort: (effort: ReasoningEffort) => Promise<void> | void;
  onSelectRoute: (route: ModelRouteId) => Promise<void> | void;
  snapshot: ModelControlsStoreSnapshot;
}

function sameRoute(left: ModelRouteId, right: ModelRouteId) {
  return left.modelId === right.modelId && left.provider === right.provider;
}

function routeKey(route: ModelRouteId) {
  return JSON.stringify([route.provider, route.modelId]);
}

function routeDisplayName(route: ModelRouteSummary, routes: readonly ModelRouteSummary[]) {
  const duplicateName = routes.some(
    (candidate) => candidate !== route && candidate.name === route.name,
  );
  return duplicateName ? `${route.name} · ${route.id.provider}` : route.name;
}

export const ComposerInferenceControls = memo(function ComposerInferenceControls({
  disabled = false,
  onSelectEffort,
  onSelectRoute,
  snapshot,
}: ComposerInferenceControlsProps) {
  const controls = snapshot.controls;
  if (!controls) {
    const loading = snapshot.phase === "loading";
    return (
      <Button
        aria-label={loading ? "Loading model controls" : "Model controls unavailable"}
        className="composer-inference-trigger composer-inference-trigger--model"
        disabled
        size="sm"
        type="button"
        variant="ghost"
      >
        {loading ? <LoaderCircleIcon aria-hidden="true" className="conversation-spin" /> : null}
        <span>{loading ? "Loading models" : "Models unavailable"}</span>
      </Button>
    );
  }

  const pendingRouteId = snapshot.pending?.kind === "route" ? snapshot.pending.route : null;
  const pendingRoute = pendingRouteId
    ? controls.routes.find((route) => sameRoute(route.id, pendingRouteId))
    : undefined;
  const selectedRoute =
    pendingRoute ??
    controls.routes.find((route) => sameRoute(route.id, controls.selectedRoute)) ??
    ({
      acceptsImages: false,
      id: controls.selectedRoute,
      name: controls.selectedRoute.modelId,
    } satisfies ModelRouteSummary);
  const selectedRouteName = routeDisplayName(selectedRoute, controls.routes);
  const selectedEffort =
    snapshot.pending?.kind === "effort" ? snapshot.pending.effort : controls.selectedEffort;
  const changing = snapshot.phase === "changing";
  const controlsDisabled = disabled || changing;

  return (
    <div className="composer-inference-controls">
      <DropdownMenu>
        <DropdownMenuTrigger
          render={
            <Button
              aria-label={`Model: ${selectedRouteName}${changing && pendingRoute ? ", switching" : ""}`}
              className="composer-inference-trigger composer-inference-trigger--model"
              disabled={controlsDisabled}
              size="sm"
              title={selectedRouteName}
              type="button"
              variant="ghost"
            />
          }
        >
          {changing && pendingRoute ? (
            <LoaderCircleIcon aria-hidden="true" className="conversation-spin" />
          ) : null}
          <span className="composer-inference-trigger__label">{selectedRouteName}</span>
          <ChevronDownIcon aria-hidden="true" />
        </DropdownMenuTrigger>
        <DropdownMenuContent
          align="start"
          className="composer-inference-menu"
          side="top"
          sideOffset={6}
        >
          <DropdownMenuGroup>
            <DropdownMenuLabel>Model</DropdownMenuLabel>
            <DropdownMenuRadioGroup
              onValueChange={(value) => {
                const route = controls.routes.find((candidate) => routeKey(candidate.id) === value);
                if (route) void onSelectRoute(route.id);
              }}
              value={routeKey(selectedRoute.id)}
            >
              {controls.routes.map((route) => {
                const name = routeDisplayName(route, controls.routes);
                return (
                  <DropdownMenuRadioItem
                    closeOnClick
                    key={routeKey(route.id)}
                    value={routeKey(route.id)}
                  >
                    <span className="composer-inference-menu__label" title={name}>
                      {name}
                    </span>
                  </DropdownMenuRadioItem>
                );
              })}
            </DropdownMenuRadioGroup>
          </DropdownMenuGroup>
        </DropdownMenuContent>
      </DropdownMenu>

      {controls.efforts.length > 1 ? (
        <DropdownMenu>
          <DropdownMenuTrigger
            render={
              <Button
                aria-label={`Reasoning effort: ${reasoningEffortLabel(selectedEffort)}`}
                className="composer-inference-trigger composer-inference-trigger--effort"
                disabled={controlsDisabled}
                size="sm"
                type="button"
                variant="ghost"
              />
            }
          >
            {changing && snapshot.pending?.kind === "effort" ? (
              <LoaderCircleIcon aria-hidden="true" className="conversation-spin" />
            ) : null}
            <span className="composer-inference-trigger__label">
              {reasoningEffortLabel(selectedEffort)}
            </span>
            <ChevronDownIcon aria-hidden="true" />
          </DropdownMenuTrigger>
          <DropdownMenuContent
            align="start"
            className="composer-inference-menu composer-inference-menu--effort"
            side="top"
            sideOffset={6}
          >
            <DropdownMenuGroup>
              <DropdownMenuLabel>Reasoning</DropdownMenuLabel>
              <DropdownMenuRadioGroup
                onValueChange={(value) => void onSelectEffort(value as ReasoningEffort)}
                value={selectedEffort}
              >
                {controls.efforts.map((effort) => (
                  <DropdownMenuRadioItem closeOnClick key={effort} value={effort}>
                    {reasoningEffortLabel(effort)}
                  </DropdownMenuRadioItem>
                ))}
              </DropdownMenuRadioGroup>
            </DropdownMenuGroup>
          </DropdownMenuContent>
        </DropdownMenu>
      ) : null}
    </div>
  );
});
