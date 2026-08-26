import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";

import type { ModelControlsSnapshot } from "@/generated/ModelControlsSnapshot";
import type { ModelControlsAdapter } from "@/platform/model-controls";
import type { ProjectSummary } from "@/platform/project-inbox";

import { ChatComposer } from "./ChatComposer";
import { ProjectDraftController } from "./draft-controller";

const project: ProjectSummary = {
  id: 7,
  name: "Atlas",
  availability: "available",
  unmergedChatCount: 0,
};

const qwenRoute = { modelId: "qwen3.8-27b", provider: "piu-local" };
const codexRoute = { modelId: "gpt-5.6-sol", provider: "openai-codex" };
const modelControls: ModelControlsSnapshot = {
  appliesAfterCurrentStep: false,
  efforts: ["low", "medium", "xhigh"],
  routes: [
    { acceptsImages: true, id: qwenRoute, name: "Qwen 3.8 27B" },
    { acceptsImages: true, id: codexRoute, name: "GPT-5.6 Sol" },
  ],
  selectedEffort: "medium",
  selectedRoute: qwenRoute,
};

function deferred<T>() {
  let resolve: (value: T) => void = () => undefined;
  const promise = new Promise<T>((resolvePromise) => {
    resolve = resolvePromise;
  });
  return { promise, resolve };
}

function modelAdapter(
  overrides: Partial<ModelControlsAdapter<number>> = {},
): ModelControlsAdapter<number> {
  return {
    get: vi.fn().mockResolvedValue(modelControls),
    selectEffort: vi.fn().mockResolvedValue(modelControls),
    selectRoute: vi.fn().mockResolvedValue(modelControls),
    ...overrides,
  };
}

test("loads project inference controls without blocking the draft input", async () => {
  const pending = deferred<ModelControlsSnapshot>();
  const adapter = modelAdapter({ get: vi.fn().mockReturnValue(pending.promise) });
  const drafts = new ProjectDraftController(vi.fn().mockResolvedValue(undefined));
  const user = userEvent.setup();
  render(
    <ChatComposer
      drafts={drafts}
      modelControlsAdapter={adapter}
      onSubmit={vi.fn().mockResolvedValue(undefined)}
      project={project}
    />,
  );

  const textarea = screen.getByRole("textbox", { name: "Draft for Atlas" });
  expect(screen.getByRole("button", { name: "Loading model controls" })).toBeDisabled();
  await user.type(textarea, "Keep typing while Pi is inspected");
  expect(textarea).toHaveValue("Keep typing while Pi is inspected");

  await act(() => {
    pending.resolve(modelControls);
    return pending.promise;
  });

  expect(adapter.get).toHaveBeenCalledWith(7);
  expect(screen.getByRole("button", { name: "Model: Qwen 3.8 27B" })).toBeVisible();
  expect(screen.getByRole("button", { name: "Reasoning effort: Medium" })).toBeVisible();
});

test("persists a keyboard-selected project route before creating the chat", async () => {
  const pendingRoute = deferred<ModelControlsSnapshot>();
  const events: string[] = [];
  const changedControls: ModelControlsSnapshot = {
    ...modelControls,
    efforts: ["high", "max"],
    selectedEffort: "max",
    selectedRoute: codexRoute,
  };
  const adapter = modelAdapter({
    selectRoute: vi.fn(async () => {
      events.push("select route");
      const controls = await pendingRoute.promise;
      events.push("route persisted");
      return controls;
    }),
  });
  const onSubmit = vi.fn(() => {
    events.push("create chat");
    return Promise.resolve(undefined);
  });
  const drafts = new ProjectDraftController(vi.fn().mockResolvedValue(undefined));
  const user = userEvent.setup();
  render(
    <ChatComposer
      drafts={drafts}
      modelControlsAdapter={adapter}
      onSubmit={onSubmit}
      project={project}
    />,
  );

  const modelTrigger = await screen.findByRole("button", { name: "Model: Qwen 3.8 27B" });
  await user.type(screen.getByRole("textbox", { name: "Draft for Atlas" }), "Create with Codex");
  modelTrigger.focus();
  await user.keyboard("{Enter}");
  const codexOption = await screen.findByRole("menuitemradio", { name: "GPT-5.6 Sol" });
  codexOption.focus();
  await user.keyboard("{Enter}");

  expect(adapter.selectRoute).toHaveBeenCalledWith(7, codexRoute);
  expect(screen.getByRole("button", { name: "Send message" })).toBeDisabled();
  fireEvent.submit(screen.getByRole("textbox", { name: "Draft for Atlas" }).closest("form")!);
  expect(onSubmit).not.toHaveBeenCalled();

  await act(() => {
    pendingRoute.resolve(changedControls);
    return pendingRoute.promise;
  });

  const effortTrigger = await screen.findByRole("button", { name: "Reasoning effort: Maximum" });
  await user.click(effortTrigger);
  expect(await screen.findByRole("menuitemradio", { name: "High" })).toBeVisible();
  expect(screen.getByRole("menuitemradio", { name: "Maximum" })).toBeVisible();
  expect(screen.queryByRole("menuitemradio", { name: "Medium" })).not.toBeInTheDocument();
  await user.keyboard("{Escape}");

  await user.click(screen.getByRole("button", { name: "Send message" }));

  expect(onSubmit).toHaveBeenCalledWith(7, "Create with Codex", [], codexRoute, "max");
  expect(events).toEqual(["select route", "route persisted", "create chat"]);
});

test("keeps the draft usable when project controls fail and retries inline", async () => {
  const get = vi
    .fn<ModelControlsAdapter<number>["get"]>()
    .mockRejectedValueOnce(new Error("inspection failed"))
    .mockResolvedValueOnce(modelControls);
  const drafts = new ProjectDraftController(vi.fn().mockResolvedValue(undefined));
  const requestCodexSignIn = vi.fn();
  const user = userEvent.setup();
  render(
    <ChatComposer
      drafts={drafts}
      modelControlsAdapter={modelAdapter({ get })}
      onRequestCodexSignIn={requestCodexSignIn}
      onSubmit={vi.fn().mockResolvedValue(undefined)}
      project={project}
    />,
  );

  expect(await screen.findByRole("alert")).toHaveTextContent(
    "Model controls are unavailable. Try again.",
  );
  const textarea = screen.getByRole("textbox", { name: "Draft for Atlas" });
  await user.type(textarea, "The draft remains editable");
  expect(textarea).toHaveValue("The draft remains editable");
  await user.click(screen.getByRole("button", { name: "Sign in to Codex" }));
  expect(requestCodexSignIn).toHaveBeenCalledOnce();

  await user.click(screen.getByRole("button", { name: "Try again" }));

  expect(await screen.findByRole("button", { name: "Model: Qwen 3.8 27B" })).toBeVisible();
  expect(get).toHaveBeenCalledTimes(2);
  expect(screen.queryByText("Model controls are unavailable. Try again.")).not.toBeInTheDocument();
});

test("reloads project controls after Codex sign-in without losing the draft", async () => {
  const get = vi
    .fn<ModelControlsAdapter<number>["get"]>()
    .mockRejectedValueOnce(new Error("no model routes"))
    .mockResolvedValueOnce(modelControls);
  const adapter = modelAdapter({ get });
  const drafts = new ProjectDraftController(vi.fn().mockResolvedValue(undefined));
  const user = userEvent.setup();
  const { rerender } = render(
    <ChatComposer
      drafts={drafts}
      modelControlsAdapter={adapter}
      onRequestCodexSignIn={vi.fn()}
      onSubmit={vi.fn().mockResolvedValue(undefined)}
      project={project}
      revision={0}
    />,
  );

  expect(await screen.findByRole("button", { name: "Sign in to Codex" })).toBeVisible();
  const textarea = screen.getByRole("textbox", { name: "Draft for Atlas" });
  await user.type(textarea, "Keep this while authenticating");

  rerender(
    <ChatComposer
      drafts={drafts}
      modelControlsAdapter={adapter}
      onRequestCodexSignIn={vi.fn()}
      onSubmit={vi.fn().mockResolvedValue(undefined)}
      project={project}
      revision={1}
    />,
  );

  expect(await screen.findByRole("button", { name: "Model: Qwen 3.8 27B" })).toBeVisible();
  expect(textarea).toHaveValue("Keep this while authenticating");
  expect(screen.getByRole("button", { name: "Send message" })).toBeEnabled();
  expect(get).toHaveBeenCalledTimes(2);
});

test("moves the same focused composer from centered to docked without losing its draft", async () => {
  const drafts = new ProjectDraftController(vi.fn().mockResolvedValue(undefined));
  const adapter = modelAdapter();
  const onSubmit = vi.fn().mockResolvedValue(undefined);
  const user = userEvent.setup();
  const { rerender } = render(
    <ChatComposer
      drafts={drafts}
      layout="centered"
      modelControlsAdapter={adapter}
      onSubmit={onSubmit}
      project={project}
    />,
  );

  const textarea = screen.getByRole("textbox", { name: "Draft for Atlas" });
  await waitFor(() => expect(textarea).toHaveFocus());
  await user.type(textarea, "Preserve this exact draft and focus");
  const stage = textarea.closest("section");
  const composer = textarea.closest("form");
  expect(stage).toHaveAttribute("data-composer-layout", "centered");
  expect(composer).toHaveAttribute("data-composer-layout", "centered");
  await user.keyboard("{Meta>}{Enter}{/Meta}");
  expect(onSubmit).not.toHaveBeenCalled();
  expect(textarea).toHaveValue("Preserve this exact draft and focus\n");

  rerender(
    <ChatComposer
      drafts={drafts}
      layout="docked"
      modelControlsAdapter={adapter}
      onSubmit={onSubmit}
      project={project}
    />,
  );

  const dockedTextarea = screen.getByRole("textbox", { name: "Draft for Atlas" });
  expect(dockedTextarea).toBe(textarea);
  expect(dockedTextarea).toHaveValue("Preserve this exact draft and focus\n");
  expect(dockedTextarea).toHaveFocus();
  expect(stage).toHaveAttribute("data-composer-layout", "docked");
  expect(composer).toHaveAttribute("data-composer-layout", "docked");
});

test("locks the accepted draft while chat creation is pending", async () => {
  let finishSubmission: (() => void) | undefined;
  const pendingSubmission = new Promise<string | undefined>((resolve) => {
    finishSubmission = () => resolve(undefined);
  });
  const drafts = new ProjectDraftController(vi.fn().mockResolvedValue(undefined));
  const user = userEvent.setup();
  render(
    <ChatComposer
      drafts={drafts}
      modelControlsAdapter={modelAdapter()}
      onSubmit={vi.fn().mockReturnValue(pendingSubmission)}
      project={project}
    />,
  );
  const textarea = screen.getByRole("textbox", { name: "Draft for Atlas" });
  await user.type(textarea, "Create the accepted chat");
  await user.click(screen.getByRole("button", { name: "Send message" }));

  expect(textarea).toHaveAttribute("readonly");
  expect(screen.getByRole("button", { name: "Attach files" })).toBeDisabled();
  await user.type(textarea, " discarded edit");
  expect(textarea).toHaveValue("Create the accepted chat");

  await act(() => {
    finishSubmission?.();
    return pendingSubmission;
  });
  expect(textarea).not.toHaveAttribute("readonly");
});

test("draft typing does not rerender the memoized inference controls", async () => {
  let routeReads = 0;
  const trackedControls: ModelControlsSnapshot = Object.defineProperty(
    { ...modelControls },
    "routes",
    {
      enumerable: true,
      get() {
        routeReads += 1;
        return modelControls.routes;
      },
    },
  );
  const drafts = new ProjectDraftController(vi.fn().mockResolvedValue(undefined));
  const user = userEvent.setup();
  render(
    <ChatComposer
      drafts={drafts}
      modelControlsAdapter={modelAdapter({
        get: vi.fn().mockResolvedValue(trackedControls),
      })}
      onSubmit={vi.fn().mockResolvedValue(undefined)}
      project={project}
    />,
  );

  await screen.findByRole("button", { name: "Model: Qwen 3.8 27B" });
  const readsAfterLoad = routeReads;
  await user.type(screen.getByRole("textbox", { name: "Draft for Atlas" }), "No broad renders");

  expect(routeReads).toBe(readsAfterLoad);
});
