import { ArrowLeftIcon } from "lucide-react";
import { useEffect, useRef } from "react";

import { Button } from "@/components/ui/button";
import { ModelResourceQaSurface, modelResourceQaEnabled } from "#model-resource-qa";
import { ModelResourcePanel } from "../../model-resources/ModelResourcePanel";

export default function SettingsSurface({ onClose }: { onClose?: () => void }) {
  const backButtonRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    backButtonRef.current?.focus();
  }, []);

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
            <p className="settings-eyebrow">Più on this Mac</p>
            <h1>Settings</h1>
            <p>Manage the resources used for local conversations.</p>
          </div>
          {modelResourceQaEnabled ? (
            <ModelResourceQaSurface />
          ) : (
            <ModelResourcePanel context="settings" />
          )}
        </div>
      </div>
    </section>
  );
}
