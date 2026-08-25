import { render, screen } from "@testing-library/react";
import { expect, test, vi } from "vitest";

import CodexSignInDialog from "./CodexSignInDialog";

vi.mock("@/platform/codex-auth", () => ({ codexAuthAdapter: {} }));
vi.mock("./CodexSignIn", () => ({
  CodexSignIn: () => <div>Provider-owned sign-in flow</div>,
}));

test("keeps the complete sign-in flow reachable at the minimum window height", async () => {
  render(<CodexSignInDialog onComplete={vi.fn()} onOpenChange={vi.fn()} open />);

  const dialog = await screen.findByRole("dialog");
  expect(dialog).toHaveClass("max-h-[calc(100dvh-2rem)]", "overflow-y-auto", "overscroll-contain");
  expect(screen.getByText("Provider-owned sign-in flow")).toBeVisible();
});
