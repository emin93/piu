const INBOX_SCOPE_STORAGE_KEY = "piu.inbox-scope.v1";
const ALL_PROJECTS_SCOPE = "all";

export function readRememberedProjectScope(): number | null {
  let value: string | null;
  try {
    value = window.localStorage.getItem(INBOX_SCOPE_STORAGE_KEY);
  } catch {
    return null;
  }
  if (!value || value === ALL_PROJECTS_SCOPE) return null;
  const projectId = Number(value);
  return Number.isSafeInteger(projectId) && projectId > 0 ? projectId : null;
}

export function rememberProjectScope(projectId: number | null) {
  try {
    window.localStorage.setItem(
      INBOX_SCOPE_STORAGE_KEY,
      projectId === null ? ALL_PROJECTS_SCOPE : String(projectId),
    );
  } catch {
    // Scope persistence is a convenience; an unavailable webview store must not block the inbox.
  }
}
