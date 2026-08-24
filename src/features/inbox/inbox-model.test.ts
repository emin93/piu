import { expect, test } from "vitest";

import type { ChatSummary, InboxSnapshot } from "../../platform/project-inbox";
import { projectDraft, selectInbox } from "./inbox-model";

const chats: ChatSummary[] = [
  {
    id: "older",
    projectId: 1,
    projectName: "Atlas",
    title: "Document the importer",
    branchName: "docs/importer",
    pullRequestNumber: null,
    createdAtMs: 100,
    mergeState: "unmerged",
  },
  {
    id: "newer",
    projectId: 2,
    projectName: "Beacon",
    title: "Repair repository indexing",
    branchName: "fix/indexing",
    pullRequestNumber: 73,
    createdAtMs: 300,
    mergeState: "unmerged",
  },
  {
    id: "same-time-b",
    projectId: 1,
    projectName: "Atlas",
    title: "Second stable row",
    branchName: "feature/b",
    pullRequestNumber: null,
    createdAtMs: 200,
    mergeState: "merged",
  },
  {
    id: "same-time-a",
    projectId: 1,
    projectName: "Atlas",
    title: "First stable row",
    branchName: "feature/a",
    pullRequestNumber: null,
    createdAtMs: 200,
    mergeState: "unmerged",
  },
];

const snapshot: InboxSnapshot = {
  projects: [
    { id: 1, name: "Atlas", availability: "available", unmergedChatCount: 2 },
    { id: 2, name: "Beacon", availability: "available", unmergedChatCount: 1 },
  ],
  drafts: [
    { projectId: 1, prompt: "Explain the parser", updatedAtMs: 500 },
    { projectId: 2, prompt: "Polish the index", updatedAtMs: 600 },
  ],
  chats,
};

test("chat ordering is newest-created-first with a stable id tie-breaker", () => {
  const selected = selectInbox(snapshot, { projectId: null, query: "" });

  expect(selected.unmergedChats.map(({ id }) => id)).toEqual(["newer", "same-time-a", "older"]);
  expect(selected.mergedChats.map(({ id }) => id)).toEqual(["same-time-b"]);

  const withTransientPresentationChanges = {
    ...snapshot,
    chats: snapshot.chats.map((chat) => ({
      ...chat,
      transientPresentation: chat.id === "older" ? "working" : "idle",
    })),
  };
  expect(
    selectInbox(withTransientPresentationChanges, {
      projectId: null,
      query: "",
    }).unmergedChats.map(({ id }) => id),
  ).toEqual(["newer", "same-time-a", "older"]);
});

test("project filters and metadata-only search compose", () => {
  expect(
    selectInbox(snapshot, { projectId: 1, query: "feature/a" }).unmergedChats.map(({ id }) => id),
  ).toEqual(["same-time-a"]);
  expect(
    selectInbox(snapshot, { projectId: null, query: "#73" }).unmergedChats.map(({ id }) => id),
  ).toEqual(["newer"]);
  expect(
    selectInbox(snapshot, { projectId: null, query: "beacon" }).unmergedChats.map(({ id }) => id),
  ).toEqual(["newer"]);

  const withTranscript = {
    ...snapshot,
    chats: snapshot.chats.map((chat) => ({
      ...chat,
      transcript: chat.id === "older" ? "secret search needle" : "",
    })),
  };
  expect(
    selectInbox(withTranscript, {
      projectId: null,
      query: "secret search needle",
    }).unmergedChats,
  ).toEqual([]);
});

test("drafts stay scoped to their project and do not match search", () => {
  expect(projectDraft(snapshot, 1)?.prompt).toBe("Explain the parser");
  expect(selectInbox(snapshot, { projectId: 1, query: "" }).drafts).toEqual([snapshot.drafts[0]]);
  expect(selectInbox(snapshot, { projectId: 1, query: "parser" }).drafts).toEqual([]);
  expect(selectInbox(snapshot, { projectId: null, query: "" }).drafts).toEqual(snapshot.drafts);
});
