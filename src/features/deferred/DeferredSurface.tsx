import { lazy, Suspense } from "react";

import type { ProjectSummary } from "@/platform/project-inbox";

export type DeferredSurfaceName = "conversation" | "diff" | "files" | "terminal" | "settings";

const ConversationSurface = lazy(() => import("./surfaces/ConversationSurface"));
const DiffSurface = lazy(() => import("./surfaces/DiffSurface"));
const FilesSurface = lazy(() => import("./surfaces/FilesSurface"));
const TerminalSurface = lazy(() => import("./surfaces/TerminalSurface"));
const SettingsSurface = lazy(() => import("./surfaces/SettingsSurface"));

const surfaces = {
  conversation: ConversationSurface,
  diff: DiffSurface,
  files: FilesSurface,
  terminal: TerminalSurface,
} satisfies Record<
  Exclude<DeferredSurfaceName, "settings">,
  React.LazyExoticComponent<() => React.ReactNode>
>;

export function DeferredSurface({
  onClose,
  project,
  surface,
}: {
  onClose?: () => void;
  project?: ProjectSummary;
  surface: DeferredSurfaceName;
}) {
  if (surface === "settings") {
    return (
      <Suspense fallback={<div className="surface-loading">Loading Settings</div>}>
        <SettingsSurface key={project?.id ?? "no-project"} onClose={onClose} project={project} />
      </Suspense>
    );
  }
  const Surface = surfaces[surface];
  return (
    <Suspense fallback={<div className="surface-loading">Loading view</div>}>
      <Surface />
    </Suspense>
  );
}
