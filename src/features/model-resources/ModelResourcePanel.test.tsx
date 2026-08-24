import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, test, vi } from "vitest";

import type { ModelAssetStatus } from "../../generated/ModelAssetStatus";
import { ModelResourcePanel } from "./ModelResourcePanel";
import { ModelResourceQaGallery } from "./ModelResourceQaGallery";

const assets = vi.hoisted(() => ({
  status: vi.fn(),
  subscribe: vi.fn(),
  start: vi.fn(),
  cancel: vi.fn(),
  authorize: vi.fn(),
  remove: vi.fn(),
  listener: undefined as ((status: ModelAssetStatus) => void) | undefined,
}));

vi.mock("../../platform/model-assets", () => ({
  getModelAssetStatus: assets.status,
  subscribeToModelAssetStatus: assets.subscribe,
  startModelDownload: assets.start,
  cancelModelDownload: assets.cancel,
  authorizeHuggingFace: assets.authorize,
  removeModelAssets: assets.remove,
}));

const missing: ModelAssetStatus = {
  phase: "missing",
  repository: "orcarouter/Qwen3.8-27B-Uncensored-MLX",
  revision: "0f88c40e9eff87740295f27654558fcb77e21ae5",
  manifestId: "fixture",
  totalBytes: 16_950_451_879,
  transferredBytes: 0,
  remainingBytes: 16_950_451_879,
  currentFreeBytes: 100_000_000_000,
  requiredFreeBytes: 18_024_193_703,
  currentAsset: null,
  currentFile: null,
  operationId: null,
  authenticationConfigured: false,
  canCancel: false,
  canResume: false,
  errorCode: null,
  message: null,
};

beforeEach(() => {
  for (const mock of [
    assets.status,
    assets.subscribe,
    assets.start,
    assets.cancel,
    assets.authorize,
    assets.remove,
  ])
    mock.mockReset();
  assets.listener = undefined;
  assets.status.mockResolvedValue(missing);
  assets.subscribe.mockImplementation((listener: (status: ModelAssetStatus) => void) => {
    assets.listener = listener;
    return Promise.resolve(() => undefined);
  });
  assets.start.mockResolvedValue(1);
  assets.cancel.mockResolvedValue(true);
  assets.authorize.mockResolvedValue(undefined);
  assets.remove.mockResolvedValue({ ...missing, phase: "missing" });
});

test("settings explains the pinned target, disk requirement, and manual download", async () => {
  render(<ModelResourcePanel context="settings" />);

  expect(await screen.findByRole("heading", { name: "Local model" })).toBeVisible();
  expect(screen.getByText("Qwen 3.8 27B · 4-bit")).toBeVisible();
  expect(screen.getByText("MTP drafter")).toBeVisible();
  expect(screen.queryByText(/block 3/i)).not.toBeInTheDocument();
  expect(screen.getByText("Download size").parentElement).toHaveTextContent("17 GB");
  expect(screen.getByText("Space needed").parentElement).toHaveTextContent("18 GB");
  await userEvent.click(screen.getByRole("button", { name: "Download model" }));
  expect(assets.start).toHaveBeenCalledOnce();
});

test("authentication is graphical and the token disappears after Keychain handoff", async () => {
  assets.status.mockResolvedValue({ ...missing, phase: "authenticationRequired" });
  const user = userEvent.setup();
  render(<ModelResourcePanel context="onboarding" />);
  const token = await screen.findByLabelText("Hugging Face access token");

  await user.type(token, "hf_secret");
  await user.click(screen.getByRole("button", { name: "Connect Hugging Face" }));

  expect(assets.authorize).toHaveBeenCalledWith("hf_secret");
  await waitFor(() => expect(token).toHaveValue(""));
});

test("ready resources require confirmation before ownership-safe removal", async () => {
  assets.status.mockResolvedValue({
    ...missing,
    phase: "ready",
    transferredBytes: missing.totalBytes,
    remainingBytes: 0,
  });
  const user = userEvent.setup();
  render(<ModelResourcePanel context="settings" />);

  const remove = await screen.findByRole("button", { name: "Remove model" });
  await user.click(remove);
  const dialog = screen.getByRole("alertdialog", { name: "Remove local model?" });
  expect(dialog).toBeVisible();
  expect(assets.remove).not.toHaveBeenCalled();
  const keep = screen.getByRole("button", { name: "Keep model" });
  await waitFor(() => expect(keep).toHaveFocus());
  await user.keyboard("{Escape}");
  expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
  await waitFor(() => expect(remove).toHaveFocus());

  await user.click(remove);
  await user.click(screen.getByRole("button", { name: "Confirm removal" }));
  expect(assets.remove).toHaveBeenCalledOnce();
});

test("an active removal closes confirmation and remains cancellable", async () => {
  assets.status.mockResolvedValue({
    ...missing,
    phase: "ready",
    transferredBytes: missing.totalBytes,
    remainingBytes: 0,
  });
  assets.remove.mockImplementation(() => new Promise(() => undefined));
  const user = userEvent.setup();
  render(<ModelResourcePanel context="settings" />);

  await user.click(await screen.findByRole("button", { name: "Remove model" }));
  await user.click(screen.getByRole("button", { name: "Confirm removal" }));
  expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
  assets.listener?.({
    ...missing,
    phase: "removing",
    transferredBytes: missing.totalBytes,
    remainingBytes: 0,
    operationId: 2,
    canCancel: true,
  });
  const cancel = await screen.findByRole("button", { name: "Cancel removal" });
  expect(cancel).toBeEnabled();
  await user.click(cancel);
  expect(assets.cancel).toHaveBeenCalledOnce();

  assets.listener?.({
    ...missing,
    phase: "removing",
    transferredBytes: missing.totalBytes,
    remainingBytes: 0,
    operationId: 2,
    canCancel: false,
    message: "Finalizing removal. Più will finish this safely.",
  });
  await waitFor(() =>
    expect(screen.queryByRole("button", { name: "Cancel removal" })).not.toBeInTheDocument(),
  );
  expect(screen.getByText("Finalizing removal. Più will finish this safely.")).toBeVisible();
});

test("a rejected stale cancel refreshes the committed removal state", async () => {
  const removing = {
    ...missing,
    phase: "removing" as const,
    operationId: 2,
    canCancel: true,
  };
  assets.status.mockResolvedValueOnce(removing).mockResolvedValueOnce({
    ...removing,
    canCancel: false,
    message: "Finalizing removal. Più will finish this safely.",
  });
  assets.cancel.mockResolvedValue(false);
  const user = userEvent.setup();
  render(<ModelResourcePanel context="settings" />);

  await user.click(await screen.findByRole("button", { name: "Cancel removal" }));

  await waitFor(() =>
    expect(screen.queryByRole("button", { name: "Cancel removal" })).not.toBeInTheDocument(),
  );
  expect(assets.status).toHaveBeenCalledTimes(2);
});

test("the removal dialog traps keyboard focus with the safe action first", async () => {
  assets.status.mockResolvedValue({ ...missing, phase: "ready" });
  const user = userEvent.setup();
  render(<ModelResourcePanel context="settings" />);
  await user.click(await screen.findByRole("button", { name: "Remove model" }));
  const keep = screen.getByRole("button", { name: "Keep model" });
  const confirm = screen.getByRole("button", { name: "Confirm removal" });

  await waitFor(() => expect(keep).toHaveFocus());
  await user.tab({ shift: true });
  await waitFor(() => expect(confirm).toHaveFocus());
  await user.tab();
  await waitFor(() => expect(keep).toHaveFocus());
});

test("initialization failures stop loading and offer a finite retry", async () => {
  assets.status.mockRejectedValueOnce(new Error("resource boundary unavailable"));
  const user = userEvent.setup();
  render(<ModelResourcePanel context="settings" />);

  expect(await screen.findByRole("heading", { name: "Model resources unavailable" })).toBeVisible();
  expect(screen.getByRole("alert")).toHaveTextContent("resource boundary unavailable");
  const retry = screen.getByRole("button", { name: "Retry" });
  await user.click(retry);

  expect(await screen.findByRole("heading", { name: "Local model" })).toBeVisible();
  expect(assets.status).toHaveBeenCalledTimes(2);
});

test("subscription failures are finite and retryable", async () => {
  assets.subscribe.mockRejectedValueOnce(new Error("events unavailable"));
  render(<ModelResourcePanel context="settings" />);

  expect(await screen.findByRole("alert")).toHaveTextContent("events unavailable");
  expect(screen.getByRole("button", { name: "Retry" })).toBeVisible();
  expect(screen.queryByText("Checking model resources…")).not.toBeInTheDocument();
});

test("background phase changes are announced without remounting settings", async () => {
  render(<ModelResourcePanel context="settings" />);
  await screen.findByRole("heading", { name: "Local model" });

  assets.listener?.({
    ...missing,
    phase: "downloading",
    transferredBytes: 4_000_000_000,
    remainingBytes: 12_950_451_879,
    currentAsset: "target",
  });

  expect(await screen.findByRole("status")).toHaveTextContent("Downloading");
  expect(screen.getByLabelText("Model download progress")).toBeVisible();
});

test("revision mismatch offers ownership-safe graphical reset and then the pinned download", async () => {
  assets.status.mockResolvedValue({
    ...missing,
    phase: "revisionMismatch",
    errorCode: "revisionMismatch",
    message: "An older Più model revision is installed.",
  });
  const user = userEvent.setup();
  render(<ModelResourcePanel context="settings" />);

  expect(
    await screen.findByText(
      "An older Più model revision is installed. Remove it here, then download the pinned revision.",
    ),
  ).toBeVisible();
  expect(screen.queryByText(/manually/i)).not.toBeInTheDocument();

  await user.click(screen.getByRole("button", { name: "Remove old model" }));
  await user.click(screen.getByRole("button", { name: "Confirm removal" }));

  expect(assets.remove).toHaveBeenCalledOnce();
  expect(await screen.findByRole("button", { name: "Download model" })).toBeVisible();
});

test("unsupported ownership fails closed without offering an operation", async () => {
  assets.status.mockResolvedValue({
    ...missing,
    phase: "failed",
    errorCode: "ownership",
    message: "Existing model assets are not owned by this Più manifest and were left untouched.",
  });
  render(<ModelResourcePanel context="settings" />);

  expect(await screen.findByRole("alert")).toHaveTextContent("were left untouched");
  expect(screen.queryByRole("button", { name: /download model/i })).not.toBeInTheDocument();
  expect(screen.queryByRole("button", { name: /remove/i })).not.toBeInTheDocument();
});

test("the build-time QA gallery cannot invoke production model IPC", async () => {
  const user = userEvent.setup();
  render(<ModelResourceQaGallery />);

  expect(assets.status).not.toHaveBeenCalled();
  expect(assets.subscribe).not.toHaveBeenCalled();
  for (const label of ["Cancel download", "Download model", "Resume download"]) {
    for (const button of screen.queryAllByRole("button", { name: label })) {
      expect(button).toBeDisabled();
    }
  }
  expect(screen.getByLabelText("Hugging Face access token")).toBeDisabled();

  await user.click(screen.getByRole("button", { name: "Onboarding context" }));
  expect(screen.getAllByLabelText("Local model onboarding")).toHaveLength(7);
  expect(assets.status).not.toHaveBeenCalled();
  await user.click(screen.getByRole("button", { name: "Settings context" }));

  await user.click(screen.getByRole("button", { name: "Remove old model" }));
  expect(screen.getByRole("button", { name: "Confirm removal" })).toBeDisabled();
  await user.keyboard("{Escape}");
  await user.click(screen.getByRole("button", { name: "Remove model" }));
  expect(screen.getByRole("button", { name: "Confirm removal" })).toBeDisabled();

  expect(assets.start).not.toHaveBeenCalled();
  expect(assets.cancel).not.toHaveBeenCalled();
  expect(assets.authorize).not.toHaveBeenCalled();
  expect(assets.remove).not.toHaveBeenCalled();
});
