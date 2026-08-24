import type { ModelAssetStatus } from "../../generated/ModelAssetStatus";
import { ModelResourcePanel } from "./ModelResourcePanel";

export function OnboardingModelResourceStep({ status }: { status: ModelAssetStatus }) {
  return (
    <section className="onboarding-model-step" aria-label="Local model onboarding">
      <header>
        <p className="settings-eyebrow">Mac setup</p>
        <h2>Prepare the local model</h2>
        <p>Più verifies this required resource before local conversations become available.</p>
      </header>
      <ModelResourcePanel context="onboarding" statusOverride={status} />
    </section>
  );
}
