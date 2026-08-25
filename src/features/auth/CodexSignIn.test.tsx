import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";

import type { CodexAuthAdapter, CodexAuthRecord, CodexAuthSession } from "@/platform/codex-auth";

import { CodexSignIn } from "./CodexSignIn";

function createAuthFixture() {
  let receive: ((record: CodexAuthRecord) => void) | undefined;
  const session: CodexAuthSession = {
    answer: vi.fn().mockResolvedValue(undefined),
    cancel: vi.fn().mockResolvedValue(undefined),
    copyText: vi.fn().mockResolvedValue(undefined),
    disconnect: vi.fn(),
    openExternal: vi.fn().mockResolvedValue(undefined),
  };
  const adapter: CodexAuthAdapter = {
    connect: vi.fn().mockImplementation((listener: (record: CodexAuthRecord) => void) => {
      receive = listener;
      return Promise.resolve(session);
    }),
  };

  return {
    adapter,
    emit(record: CodexAuthRecord) {
      if (!receive) throw new Error("authentication listener is not connected");
      receive(record);
    },
    session,
  };
}

test("offers the provider browser and device choices and returns the selected option", async () => {
  const auth = createAuthFixture();
  const user = userEvent.setup();
  render(<CodexSignIn adapter={auth.adapter} />);
  await waitFor(() => expect(auth.adapter.connect).toHaveBeenCalledOnce());

  act(() => {
    auth.emit({
      type: "auth_prompt",
      id: "auth-1",
      prompt: {
        type: "select",
        message: "Select OpenAI Codex login method:",
        options: [
          { id: "browser", label: "Browser login", description: "Recommended" },
          { id: "device_code", label: "Device code login" },
        ],
      },
    });
  });

  expect(screen.getByRole("heading", { name: "Sign in to Codex" })).toBeVisible();
  expect(screen.getByText("Select OpenAI Codex login method:")).toBeVisible();
  const browserChoice = screen.getByRole("button", { name: /Browser login/ });
  expect(browserChoice).toHaveFocus();
  await user.click(browserChoice);

  expect(auth.session.answer).toHaveBeenCalledWith("auth-1", "browser");
});

test("presents provider notifications with browser handoff and device-code copy", async () => {
  const auth = createAuthFixture();
  const user = userEvent.setup();
  render(<CodexSignIn adapter={auth.adapter} />);
  await waitFor(() => expect(auth.adapter.connect).toHaveBeenCalledOnce());

  act(() => {
    auth.emit({
      type: "auth_event",
      event: {
        type: "info",
        message: "Your organization may require single sign-on.",
        links: [{ label: "Open sign-in help", url: "https://example.test/help" }],
      },
    });
    auth.emit({
      type: "auth_event",
      event: {
        type: "auth_url",
        url: "https://example.test/sign-in",
        instructions: "Complete sign-in in your browser.",
      },
    });
    auth.emit({
      type: "auth_event",
      event: {
        type: "device_code",
        userCode: "ABCD-EFGH",
        verificationUri: "https://example.test/device",
        expiresInSeconds: 900,
        intervalSeconds: 5,
      },
    });
    auth.emit({
      type: "auth_event",
      event: { type: "progress", message: "Waiting for authorization" },
    });
  });

  expect(screen.getByText("Your organization may require single sign-on.")).toBeVisible();
  expect(screen.getByText("Complete sign-in in your browser.")).toBeVisible();
  expect(screen.getByText("ABCD-EFGH")).toBeVisible();
  expect(screen.getByText("Expires in 15 minutes")).toBeVisible();
  const progress = screen.getByRole("status");
  expect(progress).toHaveTextContent("Waiting for authorization");
  expect(progress.querySelector(".codex-auth__spinner")).toBeInTheDocument();

  await user.click(screen.getByRole("button", { name: "Open sign-in help" }));
  await user.click(screen.getByRole("button", { name: "Open sign-in page" }));
  await user.click(screen.getByRole("button", { name: "Open verification page" }));
  await user.click(screen.getByRole("button", { name: "Copy code" }));

  expect(auth.session.openExternal).toHaveBeenNthCalledWith(1, "https://example.test/help");
  expect(auth.session.openExternal).toHaveBeenNthCalledWith(2, "https://example.test/sign-in");
  expect(auth.session.openExternal).toHaveBeenNthCalledWith(3, "https://example.test/device");
  expect(auth.session.copyText).toHaveBeenCalledWith("ABCD-EFGH");
  expect(screen.getByRole("button", { name: "Copied" })).toBeVisible();
});

test("answers text, secret, and manual-code prompts through labeled controls", async () => {
  const auth = createAuthFixture();
  const user = userEvent.setup();
  render(<CodexSignIn adapter={auth.adapter} />);
  await waitFor(() => expect(auth.adapter.connect).toHaveBeenCalledOnce());

  act(() => {
    auth.emit({
      type: "auth_prompt",
      id: "auth-1",
      prompt: { type: "text", message: "Organization", placeholder: "Your organization" },
    });
  });
  await user.type(screen.getByLabelText("Organization"), "OpenAI");
  await user.click(screen.getByRole("button", { name: "Continue" }));
  await waitFor(() => expect(auth.session.answer).toHaveBeenCalledWith("auth-1", "OpenAI"));

  act(() => {
    auth.emit({
      type: "auth_prompt",
      id: "auth-2",
      prompt: { type: "secret", message: "One-time secret" },
    });
  });
  const secret = screen.getByLabelText("One-time secret");
  expect(secret).toHaveAttribute("type", "password");
  expect(secret).toHaveAttribute("autocomplete", "off");
  await user.type(secret, "temporary-value");
  await user.click(screen.getByRole("button", { name: "Continue" }));
  await waitFor(() =>
    expect(auth.session.answer).toHaveBeenCalledWith("auth-2", "temporary-value"),
  );

  act(() => {
    auth.emit({
      type: "auth_prompt",
      id: "auth-3",
      prompt: { type: "manual_code", message: "Paste the browser code" },
    });
  });
  await user.type(screen.getByLabelText("Paste the browser code"), "returned-code");
  await user.click(screen.getByRole("button", { name: "Submit code" }));

  await waitFor(() => expect(auth.session.answer).toHaveBeenCalledWith("auth-3", "returned-code"));
});

test("retires a provider-cancelled prompt and ignores its raced response failure", async () => {
  const auth = createAuthFixture();
  let rejectAnswer: ((reason: Error) => void) | undefined;
  vi.mocked(auth.session.answer).mockImplementation(
    () =>
      new Promise<void>((_resolve, reject) => {
        rejectAnswer = reject;
      }),
  );
  const user = userEvent.setup();
  render(<CodexSignIn adapter={auth.adapter} />);
  await waitFor(() => expect(auth.adapter.connect).toHaveBeenCalledOnce());

  act(() => {
    auth.emit({
      type: "auth_prompt",
      id: "auth-1",
      prompt: { type: "manual_code", message: "Paste the browser code" },
    });
  });
  await user.type(screen.getByLabelText("Paste the browser code"), "late-code");
  await user.click(screen.getByRole("button", { name: "Submit code" }));

  act(() => {
    auth.emit({ type: "auth_prompt_cancelled", id: "auth-1" });
    auth.emit({ type: "auth_event", event: { type: "progress", message: "Browser approved" } });
    rejectAnswer?.(new Error("prompt already resolved"));
  });

  await waitFor(() =>
    expect(screen.queryByLabelText("Paste the browser code")).not.toBeInTheDocument(),
  );
  expect(screen.getByRole("status")).toHaveTextContent("Browser approved");
  expect(screen.queryByRole("alert")).not.toBeInTheDocument();
});

test("cancels, retries terminal failures, and reports completion", async () => {
  const auth = createAuthFixture();
  const onComplete = vi.fn();
  const user = userEvent.setup();
  render(<CodexSignIn adapter={auth.adapter} onComplete={onComplete} />);
  await waitFor(() => expect(auth.adapter.connect).toHaveBeenCalledOnce());

  await user.click(screen.getByRole("button", { name: "Cancel sign-in" }));
  expect(auth.session.cancel).toHaveBeenCalledOnce();
  act(() => auth.emit({ type: "auth_cancelled" }));
  expect(screen.getByRole("status")).toHaveTextContent("Sign-in cancelled");

  await user.click(screen.getByRole("button", { name: "Try again" }));
  await waitFor(() => expect(auth.adapter.connect).toHaveBeenCalledTimes(2));
  act(() => {
    auth.emit({
      type: "auth_failed",
      code: "sign_in_failed",
      message: "Sign-in failed. Try again.",
    });
  });
  expect(screen.getByRole("alert")).toHaveTextContent("Sign-in failed. Try again.");

  await user.click(screen.getByRole("button", { name: "Try again" }));
  await waitFor(() => expect(auth.adapter.connect).toHaveBeenCalledTimes(3));
  act(() => auth.emit({ type: "auth_complete" }));

  expect(screen.getByRole("status")).toHaveTextContent("Codex is connected");
  expect(onComplete).toHaveBeenCalledOnce();
});

test("cancels an active helper and disconnects when the sign-in view closes", async () => {
  const auth = createAuthFixture();
  const { unmount } = render(<CodexSignIn adapter={auth.adapter} />);
  await screen.findByRole("button", { name: "Cancel sign-in" });

  unmount();

  expect(auth.session.cancel).toHaveBeenCalledOnce();
  expect(auth.session.disconnect).toHaveBeenCalledOnce();
});
