import { useState } from "react";
import { Button } from "@/components/ui/button";
import type { ModelAssetStatus } from "../../generated/ModelAssetStatus";
import { ModelResourcePanel } from "./ModelResourcePanel";
import { OnboardingModelResourceStep } from "./OnboardingModelResourceStep";

const totalBytes = 16_950_451_879;
const base: ModelAssetStatus = {
  phase: "missing",
  repository: "orcarouter/Qwen3.8-27B-Uncensored-MLX",
  revision: "0f88c40e9eff87740295f27654558fcb77e21ae5",
  manifestId: "qa-gallery",
  totalBytes,
  transferredBytes: 0,
  remainingBytes: totalBytes,
  currentFreeBytes: 445_500_000_000,
  requiredFreeBytes: 18_024_193_703,
  currentAsset: null,
  currentFile: null,
  operationId: null,
  authenticationConfigured: false,
  canResume: false,
  availableActions: ["download"],
  errorCode: null,
  message: null,
};

const states: Array<{ label: string; status: ModelAssetStatus }> = [
  {
    label: "Download progress",
    status: {
      ...base,
      phase: "downloading",
      transferredBytes: 6_200_000_000,
      remainingBytes: totalBytes - 6_200_000_000,
      requiredFreeBytes: totalBytes - 6_200_000_000 + 1_073_741_824,
      currentAsset: "target",
      currentFile: "target/model-00002-of-00003.safetensors",
      operationId: 7,
      canResume: true,
      availableActions: ["cancel"],
    },
  },
  {
    label: "Integrity verification",
    status: {
      ...base,
      phase: "verifying",
      transferredBytes: 16_100_000_000,
      remainingBytes: totalBytes - 16_100_000_000,
      requiredFreeBytes: totalBytes - 16_100_000_000 + 1_073_741_824,
      currentAsset: "drafter",
      currentFile: "drafter/model.safetensors",
      operationId: 7,
      canResume: true,
      availableActions: ["cancel"],
    },
  },
  {
    label: "Authentication required",
    status: {
      ...base,
      phase: "authenticationRequired",
      errorCode: "authentication",
      message: "Hugging Face access expired. Connect again, then resume the download.",
      availableActions: ["authorize"],
    },
  },
  {
    label: "Disk failure",
    status: {
      ...base,
      phase: "failed",
      currentFreeBytes: 8_000_000_000,
      errorCode: "insufficientSpace",
      message: "Not enough disk space. Free 10 GB, then retry the download.",
      availableActions: ["download"],
    },
  },
  {
    label: "Cancelled with resume",
    status: {
      ...base,
      phase: "cancelled",
      transferredBytes: 4_500_000_000,
      remainingBytes: totalBytes - 4_500_000_000,
      requiredFreeBytes: totalBytes - 4_500_000_000 + 1_073_741_824,
      canResume: true,
      message: "Download paused. The verified partial is ready to resume.",
      availableActions: ["download"],
    },
  },
  {
    label: "Ready and removal entry",
    status: {
      ...base,
      phase: "ready",
      transferredBytes: totalBytes,
      remainingBytes: 0,
      requiredFreeBytes: 0,
      message: "The local model and MTP drafter are ready.",
      availableActions: ["remove"],
    },
  },
  {
    label: "Old revision recovery",
    status: {
      ...base,
      phase: "revisionMismatch",
      errorCode: "revisionMismatch",
      message: null,
      availableActions: ["remove"],
    },
  },
];

export function ModelResourceQaGallery() {
  const [context, setContext] = useState<"settings" | "onboarding">("settings");
  return (
    <section className="model-resource-qa" aria-label="Model resource QA states">
      <div className="model-resource-qa__notice">
        Deterministic build-time QA gallery · production IPC disabled
      </div>
      <div className="model-resource-qa__context" aria-label="QA context">
        <Button
          type="button"
          aria-pressed={context === "settings"}
          onClick={() => setContext("settings")}
          size="sm"
          variant={context === "settings" ? "secondary" : "ghost"}
        >
          Settings context
        </Button>
        <Button
          type="button"
          aria-pressed={context === "onboarding"}
          onClick={() => setContext("onboarding")}
          size="sm"
          variant={context === "onboarding" ? "secondary" : "ghost"}
        >
          Onboarding context
        </Button>
      </div>
      {states.map(({ label, status }) => (
        <article key={label} aria-label={label}>
          <h2>{label}</h2>
          {context === "settings" ? (
            <ModelResourcePanel context="settings" statusOverride={status} />
          ) : (
            <OnboardingModelResourceStep status={status} />
          )}
        </article>
      ))}
    </section>
  );
}
