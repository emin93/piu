import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, test, vi } from "vitest";

import { App } from "./App";
import { installMatchMedia } from "./test/match-media";

const boundary = vi.hoisted(() => ({ verify: vi.fn() }));
const repositoryPicker = vi.hoisted(() => ({ open: vi.fn() }));

vi.mock("./platform/host-boundary", () => ({ verifyHostBoundary: boundary.verify }));
vi.mock("./platform/repository-picker", () => ({
  selectRepositoryDirectory: repositoryPicker.open,
}));

beforeEach(() => {
  boundary.verify.mockReset();
  boundary.verify.mockResolvedValue({
    correlationId: "test-boundary",
    latencyMs: 2,
    schemaVersion: 1,
  });
  repositoryPicker.open.mockReset();
  repositoryPicker.open.mockResolvedValue(null);
});

test("the shell follows system appearance changes live", () => {
  const systemAppearance = installMatchMedia("dark");

  render(<App />);
  expect(document.documentElement).toHaveAttribute("data-appearance", "dark");

  act(() => systemAppearance.setAppearance("light"));
  expect(document.documentElement).toHaveAttribute("data-appearance", "light");
});

test("the empty inbox action is keyboard reachable", async () => {
  installMatchMedia("light");
  const openRepository = vi.fn();
  const user = userEvent.setup();

  render(<App onOpenRepository={openRepository} />);

  expect(screen.getByRole("heading", { name: "Your work starts here" })).toBeVisible();
  const action = screen.getByRole("button", { name: "Open Repository" });
  await user.tab();
  expect(action).toHaveFocus();
  await user.keyboard("{Enter}");
  expect(openRepository).toHaveBeenCalledOnce();
});

test("the production empty inbox action opens the native repository picker", async () => {
  installMatchMedia("light");
  const user = userEvent.setup();

  render(<App />);
  await user.click(screen.getByRole("button", { name: "Open Repository" }));

  expect(repositoryPicker.open).toHaveBeenCalledOnce();
});

test("a repository picker failure is explained in product language", async () => {
  installMatchMedia("light");
  repositoryPicker.open.mockRejectedValueOnce(new Error("dialog unavailable"));
  const user = userEvent.setup();

  render(<App />);
  await user.click(screen.getByRole("button", { name: "Open Repository" }));

  expect(await screen.findByRole("alert")).toHaveTextContent("Couldn't open the repository picker");
});

test("the shell verifies the typed host boundary without exposing internals", async () => {
  installMatchMedia("light");

  render(<App />);

  await waitFor(() => expect(boundary.verify).toHaveBeenCalledOnce());
  expect(screen.queryByText(/core ready|schema 1/i)).not.toBeInTheDocument();
});

test("a host startup failure offers a retry in product language", async () => {
  installMatchMedia("light");
  boundary.verify.mockRejectedValueOnce(new Error("IPC unavailable"));
  const user = userEvent.setup();

  render(<App />);

  expect(await screen.findByRole("heading", { name: "Più couldn't start" })).toBeVisible();
  await user.click(screen.getByRole("button", { name: "Retry" }));
  await waitFor(() => expect(boundary.verify).toHaveBeenCalledTimes(2));
  await waitFor(() =>
    expect(screen.queryByRole("heading", { name: "Più couldn't start" })).not.toBeInTheDocument(),
  );
});
