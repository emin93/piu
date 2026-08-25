export interface InboxPerformanceRenderTarget {
  id?: string;
  kind: "chat-row" | "scope-control";
}

export const recordInboxRender: ((target: InboxPerformanceRenderTarget) => void) | undefined =
  undefined;

export function inboxRenderCount(_target: InboxPerformanceRenderTarget) {
  void _target;
  return 0;
}

export function resetInboxRenderCounts() {
  return undefined;
}
