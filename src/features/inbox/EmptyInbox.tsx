import { FolderOpenIcon } from "lucide-react";

import { Button } from "@/components/ui/button";
import {
  Empty,
  EmptyContent,
  EmptyDescription,
  EmptyHeader,
  EmptyTitle,
} from "@/components/ui/empty";

interface EmptyInboxProps {
  actionError?: string;
  onOpenRepository: () => void;
}

export function EmptyInbox({ actionError, onOpenRepository }: EmptyInboxProps) {
  return (
    <Empty aria-labelledby="empty-inbox-title" className="empty-inbox">
      <EmptyHeader>
        <EmptyTitle id="empty-inbox-title">Open a repository to start</EmptyTitle>
        <EmptyDescription>
          Più keeps each chat isolated in its own worktree when you send the first message.
        </EmptyDescription>
      </EmptyHeader>
      <EmptyContent>
        <Button onClick={onOpenRepository} size="lg" type="button">
          <FolderOpenIcon aria-hidden="true" data-icon="inline-start" />
          Open Repository
        </Button>
        {actionError ? (
          <p className="text-destructive" role="alert">
            {actionError}
          </p>
        ) : null}
      </EmptyContent>
    </Empty>
  );
}
