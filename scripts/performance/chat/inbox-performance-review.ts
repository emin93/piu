interface InboxPerformanceRenderTarget {
  id?: string;
  kind: "chat-row" | "scope-control";
}

const counts = new Map<string, number>();

function key({ id, kind }: InboxPerformanceRenderTarget) {
  return id ? `${kind}:${id}` : kind;
}

export function recordInboxRender(target: InboxPerformanceRenderTarget) {
  const targetKey = key(target);
  counts.set(targetKey, (counts.get(targetKey) ?? 0) + 1);
}

export function inboxRenderCount(target: InboxPerformanceRenderTarget) {
  return counts.get(key(target)) ?? 0;
}

export function resetInboxRenderCounts() {
  counts.clear();
}
