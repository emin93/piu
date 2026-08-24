import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, test, vi } from "vitest";

import type { ModelAssetStatus } from "../../generated/ModelAssetStatus";
import { ModelResourcePanel } from "./ModelResourcePanel";

const assets = vi.hoisted(() => ({
  status: vi.fn(),
  subscribe: vi.fn(),
  start: vi.fn(),
  cancel: vi.fn(),
  authorize: vi.fn(),
  remove: vi.fn(),
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
  canResume: false,
  errorCode: null,
  message: null,
};

beforeEach(() => {
  for (const mock of Object.values(assets)) mock.mockReset();
  assets.status.mockResolvedValue(missing);
  assets.subscribe.mockResolvedValue(() => undefined);
  assets.start.mockResolvedValue(1);
  assets.cancel.mockResolvedValue(true);
  assets.authorize.mockResolvedValue(undefined);
  assets.remove.mockResolvedValue({ ...missing, phase: "missing" });
});

test("settings explains the pinned target, disk requirement, and manual download", async () => {
  render(<ModelResourcePanel context="settings" />);

  expect(await screen.findByRole("heading", { name: "Local model" })).toBeVisible();
  expect(screen.getByText("Qwen 3.8 27B · 4-bit")).toBeVisible();
  expect(screen.getByText(/MTP drafter · block 4/i)).toBeVisible();
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

  await user.click(await screen.findByRole("button", { name: "Remove model" }));
  expect(screen.getByText(/only files verified as owned by Più/i)).toBeVisible();
  expect(assets.remove).not.toHaveBeenCalled();
  await user.click(screen.getByRole("button", { name: "Confirm removal" }));
  expect(assets.remove).toHaveBeenCalledOnce();
});
