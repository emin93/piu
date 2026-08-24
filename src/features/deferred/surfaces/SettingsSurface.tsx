import { ModelResourceQaSurface, modelResourceQaEnabled } from "#model-resource-qa";
import { ModelResourcePanel } from "../../model-resources/ModelResourcePanel";

export default function SettingsSurface() {
  return (
    <section className="settings-surface" aria-label="Settings">
      <div className="settings-surface__heading">
        <p className="settings-eyebrow">Più preferences</p>
        <h1>Settings</h1>
        <p>Manage the resources Più uses on this Mac.</p>
      </div>
      {modelResourceQaEnabled ? (
        <ModelResourceQaSurface />
      ) : (
        <ModelResourcePanel context="settings" />
      )}
    </section>
  );
}
