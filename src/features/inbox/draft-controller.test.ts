import { expect, test, vi } from "vitest";

import type { InboxSnapshot } from "@/platform/project-inbox";

import { ProjectDraftController } from "./draft-controller";

const snapshot = (prompt: string): InboxSnapshot => ({
  projects: [{ id: 1, name: "Atlas", availability: "available", unmergedChatCount: 0 }],
  drafts: prompt ? [{ projectId: 1, prompt, updatedAtMs: 1 }] : [],
  chats: [],
});

test("a stale inbox snapshot cannot replace a dirty or acknowledged draft", async () => {
  let finishSave: (() => void) | undefined;
  const save = vi.fn(
    () =>
      new Promise<void>((resolve) => {
        finishSave = resolve;
      }),
  );
  const drafts = new ProjectDraftController(save);
  drafts.reconcile(snapshot("Older prompt"));

  drafts.change(1, "Current prompt");
  expect(drafts.overlay(snapshot("Older prompt")).drafts[0]?.prompt).toBe("Current prompt");

  const flushed = drafts.flush(1);
  await vi.waitFor(() => expect(finishSave).toBeTypeOf("function"));
  finishSave?.();
  await flushed;

  expect(drafts.get(1).status).toEqual({ state: "saved" });
  expect(drafts.overlay(snapshot("Older prompt")).drafts[0]?.prompt).toBe("Current prompt");
});

test("cancelling a failed draft clears stale queue bookkeeping before shutdown", async () => {
  const drafts = new ProjectDraftController(() => Promise.reject(new Error("database offline")));
  drafts.reconcile(snapshot(""));
  drafts.change(1, "Keep this draft");

  await drafts.flush(1);
  expect(drafts.get(1).status).toMatchObject({ state: "failed" });

  drafts.cancel(1);
  drafts.forget(1);
  await expect(drafts.flushAll()).resolves.toBeUndefined();
  expect(drafts.get(1).prompt).toBe("");
});

test("a failed draft can be retried without losing its prompt", async () => {
  const save = vi
    .fn<() => Promise<void>>()
    .mockRejectedValueOnce(new Error("database offline"))
    .mockResolvedValueOnce();
  const drafts = new ProjectDraftController(save);
  drafts.reconcile(snapshot(""));
  drafts.change(1, "Keep this draft");

  await drafts.flush(1);
  await expect(drafts.flushAll()).rejects.toThrow("could not be saved");
  await drafts.retry(1);

  expect(save).toHaveBeenCalledTimes(2);
  expect(drafts.get(1)).toMatchObject({
    prompt: "Keep this draft",
    status: { state: "saved" },
  });
});

test("an older save completion never labels a newer generation saved", async () => {
  const completions: Array<() => void> = [];
  const drafts = new ProjectDraftController(
    () =>
      new Promise<void>((resolve) => {
        completions.push(resolve);
      }),
  );
  drafts.reconcile(snapshot(""));
  drafts.change(1, "First");
  const first = drafts.flush(1);
  drafts.change(1, "Second");
  const second = drafts.flush(1);

  await vi.waitFor(() => expect(completions).toHaveLength(1));
  completions[0]?.();
  await first;
  expect(drafts.get(1).status).toEqual({ state: "saving" });

  await vi.waitFor(() => expect(completions).toHaveLength(2));
  completions[1]?.();
  await second;
  expect(drafts.get(1)).toMatchObject({ prompt: "Second", status: { state: "saved" } });
});
