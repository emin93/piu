import type {
  ChatSummary,
  DraftSummary,
  InboxSnapshot,
  ProjectSummary,
} from "../../platform/project-inbox";

export interface InboxSelection {
  drafts: DraftSummary[];
  unmergedChats: ChatSummary[];
  mergedChats: ChatSummary[];
}

interface InboxFilter {
  projectId: number | null;
  query: string;
}

const byNewestCreation = (left: ChatSummary, right: ChatSummary) =>
  right.createdAtMs - left.createdAtMs || left.id.localeCompare(right.id);

function matchesQuery(chat: ChatSummary, query: string) {
  if (!query) return true;
  const metadata = [
    chat.title,
    chat.projectName,
    chat.branchName,
    chat.pullRequestNumber === null ? "" : `#${chat.pullRequestNumber}`,
  ];
  return metadata.some((value) => value.toLocaleLowerCase().includes(query));
}

export function selectInbox(snapshot: InboxSnapshot, filter: InboxFilter): InboxSelection {
  const query = filter.query.trim().toLocaleLowerCase();
  const chats = snapshot.chats
    .filter((chat) => filter.projectId === null || chat.projectId === filter.projectId)
    .filter((chat) => matchesQuery(chat, query));
  chats.sort(byNewestCreation);
  const drafts = query
    ? []
    : snapshot.drafts.filter(
        (draft) => filter.projectId === null || draft.projectId === filter.projectId,
      );

  return {
    drafts,
    unmergedChats: chats.filter((chat) => chat.mergeState === "unmerged"),
    mergedChats: chats.filter((chat) => chat.mergeState === "merged"),
  };
}

export function projectDraft(snapshot: InboxSnapshot, projectId: number) {
  return snapshot.drafts.find((draft) => draft.projectId === projectId);
}

export function composerProject(
  snapshot: InboxSnapshot,
  selectedProjectId: number | null,
): ProjectSummary | undefined {
  if (selectedProjectId !== null) {
    const selected = snapshot.projects.find((project) => project.id === selectedProjectId);
    if (selected) return selected;
  }

  return (
    snapshot.projects.find((project) => project.availability === "available") ??
    snapshot.projects[0]
  );
}
