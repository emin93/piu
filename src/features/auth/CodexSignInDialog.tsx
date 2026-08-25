import { Dialog, DialogContent, DialogDescription, DialogTitle } from "@/components/ui/dialog";
import { codexAuthAdapter } from "@/platform/codex-auth";

import { CodexSignIn } from "./CodexSignIn";

interface CodexSignInDialogProps {
  onComplete: () => void;
  onOpenChange: (open: boolean) => void;
  open: boolean;
}

export default function CodexSignInDialog({
  onComplete,
  onOpenChange,
  open,
}: CodexSignInDialogProps) {
  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      {open ? (
        <DialogContent className="max-h-[calc(100dvh-2rem)] overflow-y-auto overscroll-contain sm:max-w-[480px]">
          <DialogTitle className="sr-only">Sign in to Codex</DialogTitle>
          <DialogDescription className="sr-only">
            Connect Più to the Codex account on this Mac.
          </DialogDescription>
          <CodexSignIn adapter={codexAuthAdapter} onComplete={onComplete} />
        </DialogContent>
      ) : null}
    </Dialog>
  );
}
