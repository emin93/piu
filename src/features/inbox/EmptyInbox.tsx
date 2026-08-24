interface EmptyInboxProps {
  actionError?: string;
  onOpenRepository: () => void;
}

export function EmptyInbox({ actionError, onOpenRepository }: EmptyInboxProps) {
  return (
    <section className="empty-inbox" aria-labelledby="empty-inbox-title">
      <div className="branch-mark" aria-hidden="true">
        <svg viewBox="0 0 184 116" role="presentation">
          <path d="M25 22v72M25 42c0 25 31 12 31 38M56 80h44c23 0 24-29 47-29h14" />
          <circle cx="25" cy="22" r="7" />
          <circle cx="25" cy="94" r="7" />
          <circle cx="56" cy="80" r="7" />
          <circle cx="161" cy="51" r="7" />
        </svg>
      </div>
      <p className="empty-inbox__eyebrow">Inbox is clear</p>
      <h2 id="empty-inbox-title">Your work starts here</h2>
      <p className="empty-inbox__copy">
        Open a Git repository to begin a focused conversation in its own worktree.
      </p>
      <button className="primary-action" type="button" onClick={onOpenRepository}>
        <svg viewBox="0 0 20 20" aria-hidden="true">
          <path d="M2.75 5.75h5l1.5 1.75h8v7.75h-14.5z" />
          <path d="M2.75 7.5V4.75h5l1.5 1" />
        </svg>
        Open Repository
      </button>
      {actionError ? (
        <p className="empty-inbox__error" role="alert">
          {actionError}
        </p>
      ) : null}
    </section>
  );
}
