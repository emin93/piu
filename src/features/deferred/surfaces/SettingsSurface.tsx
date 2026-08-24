import { ModelResourcePanel } from "../../model-resources/ModelResourcePanel";
import { ModelResourceQaGallery } from "../../model-resources/ModelResourceQaGallery";

export default function SettingsSurface() {
  const qaGallery = import.meta.env.VITE_PIU_MODEL_QA_GALLERY === "1";
  return (
    <section className="settings-surface" aria-label="Settings">
      <div className="settings-surface__heading">
        <p className="settings-eyebrow">Più preferences</p>
        <h1>Settings</h1>
        <p>Manage the resources Più uses on this Mac.</p>
      </div>
      {qaGallery ? <ModelResourceQaGallery /> : <ModelResourcePanel context="settings" />}
    </section>
  );
}
